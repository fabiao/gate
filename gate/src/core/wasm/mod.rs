// Copyright 2017-2018 Matthew D. Michelotti
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

pub mod wasm_imports;
pub mod wasm_exports;

use std::collections::HashSet;
use std::cell::{self, RefCell};
use std::mem;
use std::io::Cursor;
use std::os::raw::c_int;

use ::{App, AppContext};
use asset_id::{AppAssetId, IdU16};
use renderer::Renderer;
use app_info::AppInfo;
use input::KeyCode;
use renderer::atlas::Atlas;
use renderer::render_buffer::RenderBuffer;
use renderer::core_renderer::CoreRenderer;
use self::wasm_imports::*;
use super::mark_app_created_flag;

pub struct CoreAudio;

impl CoreAudio {
    pub fn play_sound(&mut self, id: u16) {
        unsafe {
            gateWasmPlaySound(id as c_int);
        }
    }
    pub fn loop_music(&mut self, id: u16) {
        unsafe {
            gateWasmLoopMusic(id as c_int);
        }
    }
    pub fn stop_music(&mut self) {
        unsafe {
            gateWasmStopMusic();
        }
    }
}

trait TraitAppRunner {
    fn init(&mut self);
    fn resize(&mut self, dims: (u32, u32));
    fn update_and_draw(&mut self, time_sec: f64) -> bool;
    fn update_cursor(&mut self, cursor_x: i32, cursor_y: i32);
    fn input(&mut self, key: KeyCode, down: bool) -> bool;
    fn music_count(&self) -> u16;
    fn sound_count(&self) -> u16;
    fn on_restart(&mut self);
}

struct StaticAppRunner { r: RefCell<Option<Box<TraitAppRunner>>> }

// NOTE: StaticAppRunner is not really safe to access concurrently, this was just the easiest way
//       I could find to make it a static variable without artifically requiring `App` to
//       implement `Send`. Do not access concurrently.
unsafe impl Sync for StaticAppRunner {}

static APP_RUNNER: StaticAppRunner = StaticAppRunner { r: RefCell::new(None) };

fn app_runner_is_defined() -> bool {
    APP_RUNNER.r.borrow().is_some()
}

fn app_runner_borrow_mut() -> cell::RefMut<'static, TraitAppRunner> {
    cell::RefMut::map(APP_RUNNER.r.borrow_mut(), |x| &mut **x.as_mut().unwrap())
}

fn app_runner_borrow() -> cell::Ref<'static, TraitAppRunner> {
    cell::Ref::map(APP_RUNNER.r.borrow(), |x| &**x.as_ref().unwrap())
}

struct AppRunner<AS: AppAssetId, AP: App<AS>> {
    app: AP,
    info: AppInfo,
    renderer: Option<Renderer<AS>>,
    ctx: AppContext<AS>,
    last_time_sec: Option<f64>,
    held_keys: HashSet<KeyCode>,
}

impl<AS: AppAssetId, AP: App<AS>> TraitAppRunner for AppRunner<AS, AP> {
    fn init(&mut self) {
        assert!(self.renderer.is_none());

        let mut atlas_buf: Vec<u8>;
        unsafe {
            atlas_buf = vec![0; gateWasmSpriteAtlasBinSize()];
            gateWasmSpriteAtlasBinFill(mem::transmute(&mut atlas_buf[0]));
        }
        let sprite_atlas = Atlas::new(Cursor::new(atlas_buf)).unwrap();

        let render_buffer = RenderBuffer::new(&self.info, self.info.window_pixels, sprite_atlas);
        let core_renderer = CoreRenderer::new();
        self.renderer = Some(Renderer::<AS>::new(render_buffer, core_renderer));

        let renderer = self.renderer.as_ref().unwrap();
        self.ctx.set_dims(renderer.app_dims(), renderer.native_px());
        self.app.start(&mut self.ctx);
        assert!(!self.ctx.take_close_request(), "unexpected close immediately upon start");
    }

    fn resize(&mut self, dims: (u32, u32)) {
        let renderer = self.renderer.as_mut().unwrap();
        renderer.set_screen_dims(dims);
        self.ctx.set_dims(renderer.app_dims(), renderer.native_px());
    }

    fn update_and_draw(&mut self, time_sec: f64) -> bool {
        let elapsed = self.last_time_sec.map(|x| time_sec - x).unwrap_or(0.0).max(0.0).min(0.1);
        if elapsed > 0.0 {
            self.app.advance(elapsed.min(::MAX_TIMESTEP), &mut self.ctx);
        }
        self.last_time_sec = Some(time_sec);

        let close_requested = self.ctx.take_close_request();
        if !close_requested {
            self.app.render(self.renderer.as_mut().unwrap(), &self.ctx);
            self.renderer.as_mut().unwrap().flush();
        }
        !close_requested
    }

    fn update_cursor(&mut self, cursor_x: i32, cursor_y: i32) {
        self.ctx.set_cursor(self.renderer.as_ref().unwrap().to_app_pos(cursor_x, cursor_y));
    }

    fn input(&mut self, key: KeyCode, down: bool) -> bool {
        if down {
            if self.held_keys.insert(key) {
                self.app.key_down(key, &mut self.ctx);
            }
        } else {
            if self.held_keys.remove(&key) {
                self.app.key_up(key, &mut self.ctx);
            }
        }
        !self.ctx.take_close_request()
    }

    fn music_count(&self) -> u16 { AS::Music::count() }
    fn sound_count(&self) -> u16 { AS::Sound::count() }

    fn on_restart(&mut self) {
        for key in self.held_keys.drain() {
            self.app.key_up(key, &mut self.ctx);
        }
        assert!(!self.ctx.take_close_request(), "unexpected close immediately upon restart");
    }
}

pub fn run<AS: 'static + AppAssetId, AP: 'static + App<AS>>(info: AppInfo, app: AP) {
    mark_app_created_flag();
    *APP_RUNNER.r.borrow_mut() = Some(Box::new(AppRunner {
        app, info,
        ctx: AppContext::new(CoreAudio { }, (0., 0.), 1.),
        renderer: None,
        last_time_sec: None,
        held_keys: HashSet::new(),
    }));
}
