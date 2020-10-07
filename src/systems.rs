//! The S in ECS.
//!
//! Not using legion's #[system] macro because:
//! - Legion wants to own resources and state (cvars, map, RNG, ...).
//!   Both #[resource] and #[state] require the data to be passed by value (into Resources or the *_system() functions).
//!   There's no way to have them stored somewhere else and pass them as reference into the systems.
//!   This means I'd have to move everything into the ECS, which in turn would make even resources and state duck-typed
//!   when accessing them outside systems. Cvars are even worse because those have to be owned by JS.
//! - WASM currently only uses 1 thread anyway so no perf benefit from parallelism.
//! - https://github.com/amethyst/legion/issues/199 - I'd have to to split Pos
//!   into separate components for vehicles and projectiles to be able to do collision detection.
//! - Simple functions like these can return data to be passed to other systems.

use std::f64::consts::PI;

use legion::{query::IntoQuery, systems::CommandBuffer, Entity, World};
use rand::Rng;
use rand_distr::StandardNormal;
use vek::Clamp;

use crate::{
    components::{
        Ammo, Angle, Bfg, Cb, GuidedMissile, Hitbox, Mg, Owner, Pos, Time, TurnRate, Vehicle, Vel,
        Weapon, WEAPS_CNT,
    },
    cvars::Cvars,
    cvars::Hardpoint,
    cvars::MovementStats,
    game_state::{Explosion, GameState, Input, EMPTY_INPUT},
    map::F64Ext,
    map::Map,
    map::Vec2f,
    map::VecExt,
};

pub(crate) fn self_destruct(cvars: &Cvars, world: &mut World, gs: &mut GameState) {
    let mut query = <(&mut Vehicle, &mut Pos)>::query();
    for (vehicle, veh_pos) in query.iter_mut(world) {
        if gs.input.self_destruct && !vehicle.destroyed {
            vehicle.destroyed = true;
            gs.explosions.push(Explosion::new(
                veh_pos.0,
                cvars.g_self_destruct_explosion1_scale,
                gs.frame_time,
                false,
            ));
            gs.explosions.push(Explosion::new(
                veh_pos.0,
                cvars.g_self_destruct_explosion2_scale,
                gs.frame_time,
                false,
            ));
        }
    }
}

pub(crate) fn vehicle_movement(cvars: &Cvars, world: &mut World, gs: &GameState, map: &Map) {
    let mut query = <(
        &Vehicle,
        &mut Pos,
        &mut Vel,
        &mut Angle,
        &mut TurnRate,
        &Hitbox,
        &Input,
    )>::query();
    for (vehicle, pos, vel, angle, turn_rate, hitbox, input) in query.iter_mut(world) {
        let stats = cvars.g_vehicle_movement_stats(vehicle.veh_type);

        let new_angle = turning(&stats, vel, angle, turn_rate, input, gs.dt);

        if hitbox
            .corners(pos.0, new_angle)
            .iter()
            .any(|&corner| map.collision(corner))
        {
            turn_rate.0 *= -0.5;
        } else {
            angle.0 = new_angle;
        }

        accel_decel(&stats, vel, angle, input, gs.dt);

        // Moving
        let new_pos = pos.0 + vel.0 * gs.dt;
        if hitbox
            .corners(new_pos, angle.0)
            .iter()
            .any(|&corner| map.collision(corner))
        {
            vel.0 *= -0.5;
        } else {
            pos.0 = new_pos;
        }
    }
}

fn turning(
    stats: &MovementStats,
    vel: &mut Vel,
    angle: &Angle,
    turn_rate: &mut TurnRate,
    input: &Input,
    dt: f64,
) -> f64 {
    let tr_change = input.right_left() * stats.turn_rate_increase * dt;
    turn_rate.0 += tr_change;

    // Friction's constant component - always the same no matter the speed
    let tr_fric_const = stats.turn_rate_friction_const * dt;
    if turn_rate.0 >= 0.0 {
        turn_rate.0 = (turn_rate.0 - tr_fric_const).max(0.0);
    } else {
        turn_rate.0 = (turn_rate.0 + tr_fric_const).min(0.0);
    }

    // Friction's linear component - increases with speed
    let tr_new = turn_rate.0 * (1.0 - stats.turn_rate_friction_linear).powf(dt);
    turn_rate.0 = tr_new.clamped(-stats.turn_rate_max, stats.turn_rate_max);

    // Turning - part of vel gets rotated to simulate steering
    let turn = turn_rate.0 * dt;
    let vel_rotation = turn * stats.turn_effectiveness;
    vel.0.rotate_z(vel_rotation);

    // Normalize to 0..=360 deg
    (angle.0 + turn).rem_euclid(2.0 * PI)
}

fn accel_decel(stats: &MovementStats, vel: &mut Vel, angle: &mut Angle, input: &Input, dt: f64) {
    // TODO lateral friction
    let vel_change = input.up_down() * stats.accel_forward * dt;
    vel.0 += angle.0.to_vec2f() * vel_change;

    // Friction's constant component - always the same no matter the speed
    let vel_fric_const = stats.friction_const * dt;
    let vel_norm = vel.0.try_normalized().unwrap_or_default();
    vel.0 -= (vel_fric_const).min(vel.0.magnitude()) * vel_norm;

    // Friction's linear component - increases with speed
    vel.0 *= (1.0 - stats.friction_linear).powf(dt);
    if vel.0.magnitude_squared() > stats.speed_max.powi(2) {
        vel.0 = vel_norm * stats.speed_max;
    }
}

pub(crate) fn vehicle_logic(
    cvars: &Cvars,
    world: &mut World,
    gs: &mut GameState,
    gs_prev: &GameState,
) {
    let mut query = <(&mut Vehicle, &Input)>::query();
    for (vehicle, input) in query.iter_mut(world) {
        // Change weapon
        if input.prev_weapon && !gs_prev.input.prev_weapon {
            let prev = (vehicle.cur_weapon as u8 + WEAPS_CNT - 1) % WEAPS_CNT;
            vehicle.cur_weapon = Weapon::n(prev).unwrap();
        }
        if input.next_weapon && !gs_prev.input.next_weapon {
            let next = (vehicle.cur_weapon as u8 + 1) % WEAPS_CNT;
            vehicle.cur_weapon = Weapon::n(next).unwrap();
        }

        // Turret turning
        if gs.input.turret_left {
            vehicle.turret_angle -= cvars.g_turret_turn_speed * gs.dt;
        }
        if gs.input.turret_right {
            vehicle.turret_angle += cvars.g_turret_turn_speed * gs.dt;
        }

        // Reloading
        let ammo = &mut vehicle.ammos[vehicle.cur_weapon as usize];
        if let Ammo::Reloading(_, end) = ammo {
            if gs.frame_time >= *end {
                *ammo = Ammo::Loaded(
                    gs.frame_time,
                    cvars.g_weapon_reload_ammo(vehicle.cur_weapon),
                );
            }
        }
    }
}

pub(crate) fn shooting(cvars: &Cvars, world: &mut World, gs: &mut GameState, map: &Map) {
    let mut cmds = CommandBuffer::new(world);
    let mut query = <(Entity, &mut Vehicle, &Pos, &Vel, &Angle)>::query();
    for (&veh_id, vehicle, veh_pos, veh_vel, veh_angle) in query.iter_mut(world) {
        if vehicle.destroyed || !gs.input.fire {
            continue;
        }
        let ammo = &mut vehicle.ammos[vehicle.cur_weapon as usize];
        if let Ammo::Loaded(ready_time, count) = ammo {
            if gs.frame_time < *ready_time {
                continue;
            }

            *ready_time = gs.frame_time + cvars.g_weapon_refire(vehicle.cur_weapon);
            *count -= 1;
            if *count == 0 {
                let reload_time = cvars.g_weapon_reload_time(vehicle.cur_weapon);
                *ammo = Ammo::Reloading(gs.frame_time, gs.frame_time + reload_time);
            }

            let (hardpoint, weapon_offset) =
                cvars.g_hardpoint(vehicle.veh_type, vehicle.cur_weapon);
            let (shot_angle, shot_origin);
            match hardpoint {
                Hardpoint::Chassis => {
                    shot_angle = veh_angle.0;
                    shot_origin = veh_pos.0 + weapon_offset.rotated_z(shot_angle);
                }
                Hardpoint::Turret => {
                    shot_angle = veh_angle.0 + vehicle.turret_angle;
                    let turret_offset = cvars.g_vehicle_turret_offset_chassis(vehicle.veh_type);
                    shot_origin = veh_pos.0
                        + turret_offset.rotated_z(veh_angle.0)
                        + weapon_offset.rotated_z(shot_angle);
                }
            }
            let pos = Pos(shot_origin);
            let owner = Owner(veh_id);
            match vehicle.cur_weapon {
                Weapon::Mg => {
                    let r: f64 = gs.rng.sample(StandardNormal);
                    let spread = cvars.g_machine_gun_angle_spread * r;
                    // Using spread as y would mean the resulting spread depends on speed
                    // so it's better to use spread on angle.
                    let shot_vel = Vec2f::new(cvars.g_machine_gun_speed, 0.0)
                        .rotated_z(shot_angle + spread)
                        + cvars.g_machine_gun_vehicle_velocity_factor * veh_vel.0;
                    let vel = Vel(shot_vel);
                    cmds.push((Weapon::Mg, Mg, pos, vel, owner));
                }
                Weapon::Rail => {
                    let dir = shot_angle.to_vec2f();
                    let end = shot_origin + dir * 100_000.0;
                    let hit = map.collision_between(shot_origin, end);
                    if let Some(hit) = hit {
                        gs.railguns.push((shot_origin, hit));
                    }
                }
                Weapon::Cb => {
                    for _ in 0..cvars.g_cluster_bomb_count {
                        let speed = cvars.g_cluster_bomb_speed;
                        let spread_forward;
                        let spread_sideways;
                        if cvars.g_cluster_bomb_speed_spread_gaussian {
                            // Broken type inference (works with rand crate but distributions are deprecated).
                            let r: f64 = gs.rng.sample(StandardNormal);
                            spread_forward = cvars.g_cluster_bomb_speed_spread_forward * r;
                            let r: f64 = gs.rng.sample(StandardNormal);
                            spread_sideways = cvars.g_cluster_bomb_speed_spread_sideways * r;
                        } else {
                            let r = gs.rng.gen_range(-1.5, 1.5);
                            spread_forward = cvars.g_cluster_bomb_speed_spread_forward * r;
                            let r = gs.rng.gen_range(-1.5, 1.5);
                            spread_sideways = cvars.g_cluster_bomb_speed_spread_sideways * r;
                        }
                        let shot_vel = Vec2f::new(speed + spread_forward, spread_sideways)
                            .rotated_z(shot_angle)
                            + cvars.g_cluster_bomb_vehicle_velocity_factor * veh_vel.0;
                        let vel = Vel(shot_vel);
                        let time = gs.frame_time
                            + cvars.g_cluster_bomb_time
                            + gs.rng.gen_range(-1.0, 1.0) * cvars.g_cluster_bomb_time_spread;
                        let time = Time(time);
                        cmds.push((Weapon::Cb, Cb, pos, vel, time, owner));
                    }
                }
                Weapon::Rockets => {
                    let shot_vel = Vec2f::new(cvars.g_rockets_speed, 0.0).rotated_z(shot_angle)
                        + cvars.g_rockets_vehicle_velocity_factor * veh_vel.0;
                    let vel = Vel(shot_vel);
                    cmds.push((Weapon::Rockets, pos, vel, owner));
                }
                Weapon::Hm => {
                    let shot_vel = Vec2f::new(cvars.g_homing_missile_speed_initial, 0.0)
                        .rotated_z(shot_angle)
                        + cvars.g_homing_missile_vehicle_velocity_factor * veh_vel.0;
                    let vel = Vel(shot_vel);
                    cmds.push((Weapon::Hm, pos, vel, owner));
                }
                Weapon::Gm => {
                    if veh_id != gs.player_entity {
                        // TODO let everyone shoot GMs
                        continue;
                    }
                    let gm = GuidedMissile;
                    let shot_vel = Vec2f::new(cvars.g_guided_missile_speed_initial, 0.0)
                        .rotated_z(shot_angle)
                        + cvars.g_guided_missile_vehicle_velocity_factor * veh_vel.0;
                    let vel = Vel(shot_vel);
                    let angle = Angle(vel.0.to_angle());
                    let tr = TurnRate(0.0);
                    let gm_entity =
                        cmds.push((Weapon::Gm, gm, pos, vel, angle, tr, owner, EMPTY_INPUT));
                    gs.guided_missile = Some(gm_entity);
                }
                Weapon::Bfg => {
                    let shot_vel = Vec2f::new(cvars.g_bfg_speed, 0.0).rotated_z(shot_angle)
                        + cvars.g_bfg_vehicle_velocity_factor * veh_vel.0;
                    let vel = Vel(shot_vel);
                    cmds.push((Weapon::Bfg, Bfg, pos, vel, owner));
                }
            }
        }
    }
    cmds.flush(world);
}

pub(crate) fn gm_turning(cvars: &Cvars, world: &mut World, gs: &GameState) {
    let mut query = <(&GuidedMissile, &mut Vel, &mut Angle, &mut TurnRate, &Input)>::query();
    for (_, vel, angle, turn_rate, input) in query.iter_mut(world) {
        let stats = cvars.g_weapon_movement_stats();

        angle.0 = turning(&stats, vel, angle, turn_rate, input, gs.dt);

        accel_decel(&stats, vel, angle, input, gs.dt);
    }
}

pub(crate) fn projectiles(cvars: &Cvars, world: &mut World, gs: &mut GameState, map: &Map) {
    let mut query_vehicles = <(Entity, &Vehicle, &Pos, &Angle, &Hitbox)>::query();
    let vehicles: Vec<(Entity, _, _, _)> = query_vehicles
        .iter(world)
        .filter_map(|(&entity, vehicle, &pos, &angle, &hitbox)| {
            if !vehicle.destroyed {
                Some((entity, pos, angle, hitbox))
            } else {
                None
            }
        })
        .collect();

    let mut to_remove = Vec::new();
    let mut to_kill = Vec::new();

    let mut query = <(Entity, &Weapon, &mut Pos, &Vel, &Owner)>::query();
    for (&proj_id, &proj_weap, proj_pos, proj_vel, proj_owner) in query.iter_mut(world) {
        let new_pos = proj_pos.0 + proj_vel.0 * gs.dt;

        if proj_weap == Weapon::Cb {
            proj_pos.0 = new_pos;
            continue;
        }

        let collision = map.collision_between(proj_pos.0, new_pos);
        if let Some(col_pos) = collision {
            remove_projectile(cvars, gs, &mut to_remove, proj_id, proj_weap, col_pos);
            continue;
        }

        proj_pos.0 = new_pos;

        for (veh_id, veh_pos, _veh_angle, _veh_hitbox) in &vehicles {
            if *veh_id != proj_owner.0 {
                let dist2 = (proj_pos.0 - veh_pos.0).magnitude_squared();
                if dist2 <= 24.0 * 24.0 {
                    // Vehicle explosion first to it's below projectile explosion because it looks better.
                    gs.explosions
                        .push(Explosion::new(veh_pos.0, 1.0, gs.frame_time, false));
                    to_kill.push(*veh_id);
                    remove_projectile(cvars, gs, &mut to_remove, proj_id, proj_weap, proj_pos.0);
                    break;
                } else if proj_weap == Weapon::Bfg
                    && dist2 <= cvars.g_bfg_beam_range * cvars.g_bfg_beam_range
                    && map.collision_between(proj_pos.0, veh_pos.0).is_none()
                {
                    gs.explosions
                        .push(Explosion::new(veh_pos.0, 1.0, gs.frame_time, false));
                    to_kill.push(*veh_id);
                    gs.bfg_beams.push((proj_pos.0, veh_pos.0));
                }
            }
        }
    }

    for entity in to_remove {
        world.remove(entity);
    }

    for veh_id in to_kill {
        let mut entry = world.entry(veh_id).unwrap();
        let vehicle = entry.get_component_mut::<Vehicle>().unwrap();
        vehicle.destroyed = true;
    }
}

/// Right now, CBs are the only timed projectiles, long term, might wanna add timeouts to more
/// to avoid too many entities on huge maps..
pub(crate) fn projectiles_timeout(cvars: &Cvars, world: &mut World, gs: &mut GameState) {
    let mut to_remove = Vec::new();

    let mut query = <(Entity, &Weapon, &Pos, &Time)>::query();
    for (&entity, &weap, pos, time) in query.iter(world) {
        if gs.frame_time > time.0 {
            remove_projectile(cvars, gs, &mut to_remove, entity, weap, pos.0);
        }
    }

    for entity in to_remove {
        world.remove(entity);
    }
}

fn remove_projectile(
    cvars: &Cvars,
    gs: &mut GameState,
    to_remove: &mut Vec<Entity>,
    entity: Entity,
    weap: Weapon,
    pos: Vec2f,
) {
    if let Some(expl_scale) = cvars.g_weapon_explosion_scale(weap) {
        gs.explosions.push(Explosion::new(
            pos,
            expl_scale,
            gs.frame_time,
            weap == Weapon::Bfg,
        ));
    }
    if weap == Weapon::Gm {
        gs.guided_missile = None;
    }
    to_remove.push(entity);
}
