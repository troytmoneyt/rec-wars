// TODO lints

use vek::ops::Clamp;
use vek::Vec2;

use js_sys::Array;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use web_sys::{CanvasRenderingContext2d, HtmlImageElement};

mod data;

type Vec2f = Vec2<f64>;
type Map = Vec<Vec<usize>>;

const TILE_SIZE: f64 = 64.0;

#[wasm_bindgen]
pub struct World {
    context: CanvasRenderingContext2d,
    canvas_size: Vec2f,
    tiles: Vec<HtmlImageElement>,
    map: Map,
    pos: Vec2f,
    vel: Vec2f,
    prev_update: f64,
}

#[wasm_bindgen]
impl World {
    #[wasm_bindgen(constructor)]
    pub fn new(
        context: CanvasRenderingContext2d,
        width: f64,
        height: f64,
        tiles: Array,
        map_text: &str,
    ) -> Self {
        let tiles = tiles.iter().map(|tile| tile.dyn_into().unwrap()).collect();
        let map = data::load_map(map_text);
        Self {
            context,
            canvas_size: Vec2f::new(width, height),
            tiles,
            map,
            pos: Vec2f::new(640.0, 640.0),
            vel: Vec2f::new(0.02, 0.01),
            prev_update: 0.0,
        }
    }

    pub fn input(&mut self, left: f64, right: f64, up: f64, down: f64) {
        self.vel.x -= left * 0.007;
        self.vel.x += right * 0.007;
        self.vel.y -= up * 0.007;
        self.vel.y += down * 0.007;
    }

    pub fn update(&mut self, t: f64) {
        let dt = t - self.prev_update;

        self.pos += self.vel * dt;
        if self.pos.x <= 0.0 {
            self.pos.x = 0.0;
            self.vel.x = 0.0;
        }
        if self.pos.y <= 0.0 {
            self.pos.y = 0.0;
            self.vel.y = 0.0;
        }
        let map_size = self.map_size();
        if self.pos.x >= map_size.x {
            self.pos.x = map_size.x;
            self.vel.x = 0.0;
        }
        if self.pos.y >= map_size.y {
            self.pos.y = map_size.y;
            self.vel.y = 0.0;
        }

        self.prev_update = t;
    }

    pub fn draw(
        &self,
        img_explosion: &HtmlImageElement,
        img_guided_missile: &HtmlImageElement,
    ) -> Result<(), JsValue> {
        // Don't put the camera so close to the edge that it would render area outside the map.
        // TODO handle maps smaller than canvas (currently crashes on unreachable)
        assert!(self.map.len() >= 20);
        assert!(self.map[0].len() >= 20);
        // TODO print trace on unreachable?
        let camera_min = self.canvas_size / 2.0;
        let map_size = self.map_size();
        let camera_max = map_size - camera_min;
        let camera_pos = Vec2f::new(
            self.pos.x.clamped(camera_min.x, camera_max.x),
            self.pos.y.clamped(camera_min.y, camera_max.y),
        );

        // TODO whole pixels
        // Draw background
        // This only works properly with positive numbers but it's ok since top left of the map is (0.0, 0.0).
        let top_left = camera_pos - camera_min;
        let top_left_tile = (top_left / TILE_SIZE).floor();
        let offset_in_tile = top_left % TILE_SIZE;

        let mut c = top_left_tile.x as usize;
        let mut x = -offset_in_tile.x;
        while x < self.canvas_size.x {
            let mut r = top_left_tile.y as usize;
            let mut y = -offset_in_tile.y;
            while y < self.canvas_size.y {
                let idx = self.map[r][c] / 4;
                let img = &self.tiles[idx];
                self.context.draw_image_with_html_image_element(img, x, y)?;
                r += 1;
                y += TILE_SIZE;
            }
            c += 1;
            x += TILE_SIZE;
        }

        // Draw player
        let player_scr_pos = self.pos - top_left;
        self.context.draw_image_with_html_image_element(
            img_guided_missile,
            player_scr_pos.x - 10.0,
            player_scr_pos.y - 2.0,
        )?;

        // Draw debug text
        // TODO generalize
        self.context.set_fill_style(&"red".into());
        // TODO make vek respect decimals formatting
        self.context.fill_text(
            &format!("top left: {:.2}, {:.2}", top_left.x, top_left.y),
            20.0,
            30.0,
        )?;
        self.context.fill_text(
            &format!("player pos: {:.2}, {:.2}", self.pos.x, self.pos.y),
            20.0,
            40.0,
        )?;
        self.context.fill_text(
            &format!("camera pos: {:.2}, {:.2}", camera_pos.x, camera_pos.y),
            20.0,
            50.0,
        )?;

        Ok(())
    }

    fn map_size(&self) -> Vec2f {
        Vec2f::new(
            self.map.len() as f64 * TILE_SIZE,
            self.map[0].len() as f64 * TILE_SIZE,
        )
    }
}
