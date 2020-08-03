// Load the webassembly and some assets,
// do a couple more things that are easier on the JS side like reading input.

// Use ES module import syntax to import functionality from the module
// that we have compiled.
//
// Note that the `default` import is an initialization function which
// will "boot" the module and make it ready to use. Currently browsers
// don't support natively imported WebAssembly as an ES module, but
// eventually the manual initialization won't be required!
import init, { World, Cvars } from '../pkg/rec_wars.js';

async function run() {
    // First up we need to actually load the wasm file, so we use the
    // default export to inform it where the wasm file is located on the
    // server, and then we wait on the returned promise to wait for the
    // wasm to be loaded.
    //
    // It may look like this: `await init('./pkg/without_a_bundler_bg.wasm');`,
    // but there is also a handy default inside `init` function, which uses
    // `import.meta` to locate the wasm file relatively to js file.
    //
    // Note that instead of a string you can also pass in any of the
    // following things:
    //
    // * `WebAssembly.Module`
    //
    // * `ArrayBuffer`
    //
    // * `Response`
    //
    // * `Promise` which returns any of the above, e.g. `fetch("./path/to/wasm")`
    //
    // This gives you complete control over how the module is loaded
    // and compiled.
    //
    // Also note that the promise, when resolved, yields the wasm module's
    // exports which is the same as importing the `*_bg` module in other
    // modes
    await init();

    // And afterwards we can use all the functionality defined in wasm.

    let imgs_textures = [
        "../assets/tiles/g1.bmp",
        "../assets/tiles/g2.bmp",
        "../assets/tiles/g3.bmp",
        "../assets/tiles/g_stripes.bmp",
        "../assets/tiles/bunker1.bmp",
        "../assets/tiles/ice1.bmp",
        "../assets/tiles/ice.bmp",
        "../assets/tiles/ice_side.bmp",
        "../assets/tiles/ice_corner.bmp",
        "../assets/tiles/g_spawn.bmp",
        "../assets/tiles/road.bmp",
        "../assets/tiles/water.bmp",
        "../assets/tiles/snow.bmp",
        "../assets/tiles/snow2.bmp",
        "../assets/tiles/bunker2.bmp",
        "../assets/tiles/base.bmp",
        "../assets/tiles/water_side.bmp",
        "../assets/tiles/water_corner.bmp",
        "../assets/tiles/desert.bmp",
        "../assets/tiles/d_rock.bmp",
        "../assets/tiles/g2d.bmp",
        "../assets/tiles/water_middle.bmp",
    ].map((tile) => {
        let img = new Image();
        img.src = tile;
        return img;
    });

    let img_explosion = new Image();
    img_explosion.src = "../assets/explosion.png";
    let img_guided_missile = new Image();
    img_guided_missile.src = "../assets/weapons/guided_missile.png";
    // https://stackoverflow.com/questions/46399223/async-await-in-image-loading
    // https://developer.mozilla.org/en-US/docs/Web/API/WindowOrWorkerGlobalScope/createImageBitmap
    // or better yet, figure out how webpack works

    const canvas = document.getElementById("canvas");
    const ctx = canvas.getContext("2d", { alpha: false });

    let left = 0, right = 0, up = 0, down = 0;

    document.addEventListener('keydown', event => {
        if (event.key === "ArrowLeft" || event.key === "a") {
            left = 1;
        } else if (event.key === "ArrowRight" || event.key === "d") {
            right = 1;
        } else if (event.key === "ArrowUp" || event.key === "w") {
            up = 1;
        } else if (event.key === "ArrowDown" || event.key === "s") {
            down = 1;
        }
    });

    document.addEventListener('keyup', event => {
        if (event.key === "ArrowLeft" || event.key === "a") {
            left = 0;
        } else if (event.key === "ArrowRight" || event.key === "d") {
            right = 0;
        } else if (event.key === "ArrowUp" || event.key === "w") {
            up = 0;
        } else if (event.key === "ArrowDown" || event.key === "s") {
            down = 0;
        }
    });

    let load_tex_list = () => {
        let request = new XMLHttpRequest();
        request.open("GET", "../assets/texture_list.txt");
        request.onloadend = () => {
            if (request.status !== 200) {
                console.log("Failed to load texture_list: ", request);
                return;
            }

            load_map(request.responseText);
        }
        request.send();
    }

    let load_map = (tex_list_text) => {
        let request = new XMLHttpRequest();
        request.open("GET", "../maps/Atrium.map");
        request.onloadend = () => {
            if (request.status !== 200) {
                console.log("Failed to load map: ", request);
                return;
            }

            play(tex_list_text, request.responseText);
        }
        request.send();
    }

    let play = (tex_list_text, map_text) => {
        
        // For now, cvars need to live on the JS heap and be passed into each function that needs them.
        // I couldn't find a better way to make them mutable in JS and readable in Rust:
        // - can't return references from Rust into JS
        // - can't have a reference in World
        // - owned pub cvars in World need to be copy -> can't be changed from JS (changing will have no effect)
        // - TODO try returning Rc/Arc
        const cvars = new Cvars();
        const world = new World(cvars, ctx, canvas.width, canvas.height, imgs_textures, img_explosion, tex_list_text, map_text);

        // Make some game objects available on window for easier debugging.
        window.cvars = cvars;
        window.world = world;

        const frame = (t) => {
            // Apparently it's best practice to call requestAnimationFrame at the start of the frame.
            // However if something throws an exception, it'll likely happen every frame and
            // spam the console, making firefox painfully slow. In that case, cancel the next frame.
            // Note that rust panics seem to throw exceptions but ALSO abort the program after the catch runs.
            const handle = window.requestAnimationFrame(frame);

            try {
                world.input(cvars, left, right, up, down);
                world.update_pre(cvars, t);
                world.draw(cvars, img_guided_missile, true);
                world.update_post();
            } catch (e) {
                console.log("exception - aborting next frame");
                window.cancelAnimationFrame(handle);
                throw e;
            }
        };
        window.requestAnimationFrame(frame);
    }

    // TODO there's gotta be a way to avoid this retarded chain
    load_tex_list();
}

run();
