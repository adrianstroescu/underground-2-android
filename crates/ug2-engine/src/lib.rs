//! Need for Speed: Underground 2, reimplemented natively. Platform layers (desktop,
//! Android) create a window/surface, forward raw input and call [`Game::frame`].

pub mod assets;
pub mod camera;
pub mod cars;
pub mod collision;
pub mod config;
pub mod font;
pub mod hud;
pub mod input;
pub mod menu;
pub mod render;
pub mod vehicle;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use glam::{vec2, Mat4, Quat, Vec3};
use ug2_formats::bin_hash;
use ug2_formats::carinfo::WheelInfo;
use ug2_formats::fs::GameDir;

use crate::assets::{spawn_loader, GpuVertex, LoadEvent, LoadRequest, Loader, LoaderConfig};
use crate::camera::ChaseCamera;
use crate::cars::{display_name, CarModel};
use crate::collision::CollisionWorld;
use crate::config::{GameConfig, SteerMode};
use crate::hud::{Canvas, Hud, Telemetry};
use crate::input::{Action, Controls, InputMap, InputState, RawInput, TouchPhase};
use crate::menu::{Menu, MenuHit, MenuItem};
use crate::render::{CarDraw, FrameData, Gpu, Renderer};
use crate::vehicle::Vehicle;

pub use wgpu;

enum Status {
    Loading(String),
    Ready,
    Failed(String),
    MissingData(PathBuf),
}

/// A car model on the GPU and what physics and effects need from it.
struct ModelInfo {
    id: usize,
    wheels: [WheelInfo; 4],
    headlights: Vec<Vec3>,
    brakelights: Vec<Vec3>,
    /// Front of the body (car-space X), for the headlight pool on the road.
    front: f32,
    ride_height: f32,
    wheelbase: f32,
}

impl ModelInfo {
    fn new(id: usize, m: &CarModel) -> Self {
        Self {
            id,
            wheels: m.wheels,
            headlights: m.headlights.clone(),
            brakelights: m.brakelights.clone(),
            front: m.bbox_max.x,
            ride_height: m.ride_height(),
            wheelbase: m.wheelbase(),
        }
    }
}

/// Paint colours offered in the pause menu.
const PAINTS: &[(&str, [f32; 3])] = &[
    ("ORANGE", [0.85, 0.32, 0.05]),
    ("RED", [0.72, 0.04, 0.04]),
    ("YELLOW", [0.92, 0.72, 0.05]),
    ("LIME", [0.45, 0.85, 0.08]),
    ("GREEN", [0.05, 0.45, 0.15]),
    ("CYAN", [0.05, 0.65, 0.80]),
    ("BLUE", [0.05, 0.18, 0.70]),
    ("PURPLE", [0.38, 0.08, 0.62]),
    ("PINK", [0.90, 0.25, 0.55]),
    ("WHITE", [0.85, 0.86, 0.88]),
    ("SILVER", [0.55, 0.57, 0.60]),
    ("GUNMETAL", [0.22, 0.24, 0.27]),
    ("BLACK", [0.03, 0.03, 0.035]),
];

pub struct Game {
    pub gpu: Gpu,
    surface: Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration)>,
    renderer: Renderer,
    pub cfg: GameConfig,
    config_path: PathBuf,
    loader: Option<Loader>,
    world_loading: bool,
    status: Status,
    world: Option<CollisionWorld>,
    vehicle: Vehicle,
    models: HashMap<String, ModelInfo>,
    racers: Vec<String>,
    /// Model the player drives (key into `models`) and the one being loaded.
    player_car: Option<String>,
    pending_car: Option<String>,
    spawned: bool,
    camera: ChaseCamera,
    input: InputState,
    hud: Hud,
    canvas: Canvas,
    paused: bool,
    pause_menu: Menu,
    glows: Vec<GpuVertex>,
    brake_light: f32,
    last: Instant,
    acc: f32,
    fps: f32,
    debug_timer: f32,
    size: (u32, u32),
    dp: f32,
    /// Show touch controls even before the first touch (true on phones).
    pub touch_ui: bool,
}

const STEP: f32 = 1.0 / 120.0;

impl Game {
    /// `data_dir` holds the original game files; settings live in `config_dir`.
    pub fn new(gpu: Gpu, format: wgpu::TextureFormat, data_dir: &Path, config_dir: &Path, dp: f32) -> Self {
        let _ = std::fs::create_dir_all(config_dir);
        let config_path = config_dir.join("ug2.cfg");
        let cfg = GameConfig::load_or_create(&config_path);
        let input = InputState::new(InputMap::load_or_create(&config_dir.join("controls.cfg")));
        let renderer = Renderer::new(&gpu, format, cfg.msaa);
        log::info!("renderer: {:?}, MSAA x{}", format, renderer.sample_count);

        let game_dir = GameDir::new(data_dir);
        let (status, loader) = if game_dir.is_valid() {
            let lc = LoaderConfig { car: cfg.car.clone(), location: cfg.location.clone(), bc_textures: gpu.bc_textures };
            (Status::Loading("STARTING".into()), Some(spawn_loader(game_dir, lc)))
        } else {
            (Status::MissingData(data_dir.to_path_buf()), None)
        };
        let hud = Hud::new(cfg.steer_mode, cfg.hud_opacity);
        Self {
            gpu,
            surface: None,
            renderer,
            config_path,
            world_loading: loader.is_some(),
            loader,
            status,
            world: None,
            vehicle: Vehicle::new(Vec3::ZERO, 0.0, 0.16, 2.6),
            models: HashMap::new(),
            racers: Vec::new(),
            player_car: None,
            pending_car: Some(cfg.car.clone()),
            spawned: false,
            camera: ChaseCamera::new(),
            input,
            hud,
            canvas: Canvas::default(),
            paused: false,
            pause_menu: Menu::new("PAUSED", Vec::new()),
            glows: Vec::new(),
            brake_light: 0.0,
            last: Instant::now(),
            acc: 0.0,
            fps: 60.0,
            debug_timer: 0.0,
            size: (1, 1),
            dp: dp * cfg.hud_scale,
            touch_ui: false,
            cfg,
        }
    }

    pub fn attach_surface(&mut self, surface: wgpu::Surface<'static>, width: u32, height: u32) {
        let mut config = self.gpu.surface_config(&surface, width, height);
        if config.format != self.renderer.format {
            log::warn!("surface format changed to {:?}; keeping {:?}", config.format, self.renderer.format);
            config.format = self.renderer.format;
        }
        surface.configure(&self.gpu.device, &config);
        self.surface = Some((surface, config));
        self.resize(width, height);
    }

    pub fn detach_surface(&mut self) {
        self.surface = None;
    }

    pub fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        self.size = (w, h);
        if let Some((surface, config)) = &mut self.surface {
            config.width = w;
            config.height = h;
            surface.configure(&self.gpu.device, config);
        }
        self.hud.layout(w as f32, h as f32, self.dp);
    }

    pub fn set_dp(&mut self, dp: f32) {
        self.dp = dp * self.cfg.hud_scale;
        self.hud.layout(self.size.0 as f32, self.size.1 as f32, self.dp);
    }

    pub fn handle_input(&mut self, ev: RawInput) {
        if let RawInput::Touch { id, phase, x, y } = ev {
            if self.paused {
                if phase == TouchPhase::Down {
                    if let Some(hit) = self.pause_menu.hit(vec2(x, y)) {
                        self.pause_action(hit);
                    }
                }
                return;
            }
            self.hud.touch(id, phase, x, y);
        } else {
            self.input.handle(&ev);
        }
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if paused {
            self.pause_menu.selected = 0;
        }
    }

    fn pause_action(&mut self, hit: MenuHit) {
        let step = |h: MenuHit| match h {
            MenuHit::Prev(_) => -1,
            _ => 1,
        };
        let (MenuHit::Activate(i) | MenuHit::Prev(i) | MenuHit::Next(i)) = hit;
        match i {
            0 => self.set_paused(false),
            1 if !self.racers.is_empty() => {
                let current = self.pending_car.as_ref().or(self.player_car.as_ref()).cloned().unwrap_or_default();
                let at = self.racers.iter().position(|r| *r == current).unwrap_or(0) as i32;
                let next = self.racers[(at + step(hit)).rem_euclid(self.racers.len() as i32) as usize].clone();
                self.cfg.car = next.clone();
                self.cfg.save(&self.config_path);
                self.select_car(next);
            }
            2 => {
                let at = PAINTS.iter().position(|(_, c)| *c == self.cfg.paint).unwrap_or(0) as i32;
                self.cfg.paint = PAINTS[(at + step(hit)).rem_euclid(PAINTS.len() as i32) as usize].1;
                self.cfg.save(&self.config_path);
            }
            3 => {
                self.cfg.steer_mode = match self.cfg.steer_mode {
                    SteerMode::Buttons => SteerMode::Joystick,
                    SteerMode::Joystick => SteerMode::Buttons,
                };
                self.hud.steer_mode = self.cfg.steer_mode;
                self.hud.layout(self.size.0 as f32, self.size.1 as f32, self.dp);
                self.cfg.save(&self.config_path);
            }
            4 => {
                self.vehicle.reset_to_safe();
                self.camera.snap(&self.vehicle);
                self.set_paused(false);
            }
            _ => {}
        }
    }

    /// Switches the player's car, loading the model first if needed.
    fn select_car(&mut self, name: String) {
        if self.models.contains_key(&name) {
            self.pending_car = None;
            self.use_car(&name);
        } else {
            if let Some(l) = &self.loader {
                l.request(LoadRequest::Car(name.clone()));
            }
            self.pending_car = Some(name);
        }
    }

    fn use_car(&mut self, name: &str) {
        let Some(m) = self.models.get(name) else { return };
        self.vehicle.ride_height = m.ride_height;
        self.vehicle.wheelbase = m.wheelbase;
        if self.spawned {
            let (pos, yaw) = (self.vehicle.pos, self.vehicle.yaw);
            self.vehicle.reset(pos + Vec3::Z * 0.3, yaw);
        }
        self.player_car = Some(name.to_string());
    }

    fn controls(&self) -> Controls {
        let a = self.input.controls();
        let b = self.hud.controls();
        Controls {
            steer: (a.steer + b.steer).clamp(-1.0, 1.0),
            throttle: a.throttle.max(b.throttle),
            brake: a.brake.max(b.brake),
            handbrake: a.handbrake || b.handbrake,
            nitro: a.nitro || b.nitro,
            look_back: a.look_back || b.look_back,
        }
    }

    fn pump_loader(&mut self, budget: Duration) {
        let start = Instant::now();
        let mut lost = false;
        while start.elapsed() < budget {
            let Some(loader) = &self.loader else { return };
            let ev = match loader.events.try_recv() {
                Ok(ev) => ev,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    lost = true;
                    break;
                }
            };
            match ev {
                LoadEvent::Progress(s) => {
                    if !matches!(self.status, Status::Ready) {
                        self.status = Status::Loading(s);
                    }
                }
                LoadEvent::Textures(ts) => {
                    for t in &ts {
                        self.renderer.upload_texture(&self.gpu, t);
                    }
                }
                LoadEvent::CarList { racers, traffic } => {
                    log::info!("{} racers, {} traffic cars", racers.len(), traffic.len());
                    self.racers = racers;
                }
                LoadEvent::Car(car) => {
                    let name = car.name.clone();
                    if let Some(id) = self.renderer.add_car_model(&self.gpu, &car) {
                        self.models.insert(name.clone(), ModelInfo::new(id, &car));
                    }
                    if self.pending_car.as_deref() == Some(name.as_str()) {
                        self.pending_car = None;
                        self.use_car(&name);
                    }
                }
                LoadEvent::CarFailed(name) => {
                    if self.pending_car.as_deref() == Some(name.as_str()) {
                        self.pending_car = None;
                    }
                }
                LoadEvent::Collision(c) => self.world = Some(c),
                LoadEvent::Spawn { pos, yaw } => {
                    self.vehicle.reset(pos, yaw);
                    self.camera.snap(&self.vehicle);
                    self.spawned = true;
                    // Playable as soon as the car and the road exist.
                    self.status = Status::Ready;
                }
                LoadEvent::Section(s) => self.renderer.add_section(&self.gpu, &s.mesh, s.sky),
                LoadEvent::Done => {
                    log::info!(
                        "loaded: {} sections, {} textures",
                        self.renderer.section_count(),
                        self.renderer.texture_count()
                    );
                    self.status = Status::Ready;
                    self.world_loading = false;
                }
                LoadEvent::Failed(e) => {
                    self.status = Status::Failed(e);
                    self.world_loading = false;
                }
            }
        }
        if lost {
            self.loader = None;
            self.world_loading = false;
        }
    }

    /// Advances the simulation by `dt` seconds (real time, clamped).
    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.1);
        if dt > 0.0 {
            self.fps = self.fps * 0.95 + (1.0 / dt) * 0.05;
        }
        self.pump_loader(Duration::from_millis(12));

        let mut triggers = self.input.take_triggers();
        triggers.extend(self.hud.take_triggers());
        for t in triggers {
            match t {
                Action::Camera => self.camera.next_mode(),
                Action::Reset => {
                    self.vehicle.reset_to_safe();
                    self.camera.snap(&self.vehicle);
                }
                Action::Pause => self.set_paused(!self.paused),
                _ => {}
            }
        }

        let controls = self.controls();
        if !self.paused && self.spawned {
            self.acc += dt;
            let mut steps = 0;
            while self.acc >= STEP && steps < 12 {
                self.vehicle.step(&controls, self.world.as_ref(), STEP);
                self.acc -= STEP;
                steps += 1;
            }
            if steps == 12 {
                self.acc = 0.0;
            }
            let radius = self.player_model().map_or(0.33, |m| m.wheels[0].radius);
            self.vehicle.advance_wheels(radius, dt);
            self.camera.update(&self.vehicle, controls.look_back, self.world.as_ref(), dt);
            let braking = controls.brake > 0.05 && self.vehicle.speed() > 0.5;
            let target = if braking { 1.0 } else { 0.0 };
            self.brake_light += (target - self.brake_light) * (1.0 - (-25.0 * dt).exp());
            self.debug_timer += dt;
            if self.debug_timer > 1.0 {
                self.debug_timer = 0.0;
                let v = &self.vehicle;
                let wall = self.world.as_ref().and_then(|w| w.owners.get(v.last_wall as usize)).cloned().unwrap_or_default();
                log::info!(
                    "car pos {:.1?} speed {:.1} grounded {} walls {} ({wall}) | thr {:.1} brk {:.1} steer {:.2}",
                    v.pos, v.speed() * 3.6, v.grounded, v.wall_hits, controls.throttle, controls.brake, controls.steer
                );
            }
        }
        self.build_hud();
    }

    fn player_model(&self) -> Option<&ModelInfo> {
        self.player_car.as_ref().and_then(|n| self.models.get(n))
    }

    fn build_hud(&mut self) {
        self.canvas.clear();
        let show_touch = self.touch_ui || self.hud.touch_seen;
        match &self.status {
            Status::MissingData(p) => {
                let path = p.display().to_string().to_ascii_uppercase();
                self.hud.message(&mut self.canvas, &["GAME FILES NOT FOUND", "COPY CARS, GLOBAL, TRACKS TO", &path]);
                return;
            }
            Status::Failed(e) => {
                let e = e.to_ascii_uppercase();
                let short: String = e.chars().take(60).collect();
                self.hud.message(&mut self.canvas, &["LOADING FAILED", &short]);
                return;
            }
            Status::Loading(s) if !self.spawned => {
                let s = s.clone();
                self.hud.message(&mut self.canvas, &["NEED FOR SPEED UNDERGROUND 2", &s]);
                return;
            }
            _ => {}
        }
        let t = Telemetry {
            speed_kmh: self.vehicle.speed().abs() * 3.6,
            gear: self.vehicle.gear,
            rpm: self.vehicle.rpm,
            nitro: self.vehicle.nitro,
            fps: self.fps,
            draw_calls: self.renderer.stats_draws,
        };
        if !self.paused {
            self.hud.draw(&mut self.canvas, &t, show_touch, self.cfg.show_fps);
        }
        if let Status::Loading(s) = &self.status {
            let s = s.clone();
            let size = 1.6 * self.dp;
            self.canvas.text(self.size.0 as f32 * 0.5, 70.0 * self.dp, size, 0.5, &s, [1.0, 1.0, 1.0, 0.6]);
        }
        if let Some(name) = &self.pending_car {
            let text = format!("LOADING {}", display_name(name));
            self.canvas.text(self.size.0 as f32 * 0.5, 90.0 * self.dp, 1.8 * self.dp, 0.5, &text, [1.0, 1.0, 1.0, 0.8]);
        }
        if self.paused {
            let car = self.pending_car.as_ref().or(self.player_car.as_ref()).map(|n| display_name(n).to_string());
            let paint = PAINTS.iter().find(|(_, c)| *c == self.cfg.paint).map_or("CUSTOM", |(n, _)| n);
            let steer = match self.cfg.steer_mode {
                SteerMode::Buttons => "BUTTONS",
                SteerMode::Joystick => "JOYSTICK",
            };
            self.pause_menu.items = vec![
                MenuItem::button("RESUME"),
                MenuItem::choice("CAR", car.unwrap_or_default()),
                MenuItem::choice("COLOR", paint),
                MenuItem::choice("STEERING", steer),
                MenuItem::button("RESET CAR"),
            ];
            let size = vec2(self.size.0 as f32, self.size.1 as f32);
            self.pause_menu.draw(&mut self.canvas, size, self.dp);
        }
    }

    fn wheel_transforms(&self, car: Mat4, wheels: &[WheelInfo; 4]) -> [Mat4; 4] {
        let mut out = [Mat4::IDENTITY; 4];
        for (i, w) in wheels.iter().enumerate() {
            let p = Vec3::from(w.position);
            let front = p.x > 0.0;
            let steer = if front { self.vehicle.steer * 0.6 } else { 0.0 };
            let mirror = if p.y < 0.0 { Vec3::new(1.0, -1.0, 1.0) } else { Vec3::ONE };
            out[i] = car
                * Mat4::from_translation(p)
                * Mat4::from_quat(Quat::from_rotation_z(steer))
                * Mat4::from_scale(mirror)
                * Mat4::from_quat(Quat::from_rotation_y(self.vehicle.wheel_spin));
        }
        out
    }

    /// Renders into `target`. Borrows the HUD canvas and the renderer as disjoint fields.
    fn render_to(&mut self, target: &wgpu::TextureView, width: u32, height: u32) {
        let view = self.camera.view();
        let mut cars = Vec::new();
        let mut glows = std::mem::take(&mut self.glows);
        glows.clear();
        if let (true, Some(m)) = (self.spawned, self.player_model()) {
            let t = self.vehicle.transform();
            let wheels = self.wheel_transforms(t, &m.wheels);
            cars.push(CarDraw { model: m.id, transform: t, wheels, paint: self.cfg.paint, skin: None, brake: self.brake_light });
            let (right, up) = (view.row(0).truncate(), view.row(1).truncate());
            push_car_glows(&mut glows, m, t, self.brake_light, right, up);
        }
        self.glows = glows;
        let data = FrameData {
            view,
            fov_y: self.camera.fov_y,
            cam_pos: self.camera.eye,
            cars: &cars,
            glows: &self.glows,
            glow_texture: bin_hash("HEADLIGHTGLOW"),
            draw_distance: self.cfg.draw_distance,
            vertex_light_scale: self.cfg.vertex_light_scale,
            hud: &self.canvas.vertices,
        };
        self.renderer.render(&self.gpu, target, width, height, &data);
    }

    /// One iteration of the main loop: simulate and present.
    pub fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32();
        self.last = now;
        self.update(dt);

        let Some((surface, config)) = &self.surface else { return };
        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                surface.configure(&self.gpu.device, config);
                return;
            }
            Err(e) => {
                log::warn!("surface: {e}");
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (w, h) = (config.width, config.height);
        self.render_to(&view, w, h);
        frame.present();
    }

    pub fn is_loading(&self) -> bool {
        self.world_loading
    }

    pub fn status_text(&self) -> String {
        match &self.status {
            Status::Loading(s) => format!("loading: {s}"),
            Status::Ready => "ready".into(),
            Status::Failed(e) => format!("failed: {e}"),
            Status::MissingData(p) => format!("missing data in {}", p.display()),
        }
    }

    pub fn draw_calls(&self) -> u32 {
        self.renderer.stats_draws
    }

    /// Renders one frame offscreen and returns tightly packed RGBA8 (tests, screenshots).
    pub fn screenshot(&mut self, width: u32, height: u32) -> Vec<u8> {
        self.hud.layout(width as f32, height as f32, self.dp);
        self.build_hud();
        let format = self.renderer.format;
        let tex = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.render_to(&view, width, height);

        let row = (width * 4).div_ceil(256) * 256;
        let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.gpu.queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.gpu.device.poll(wgpu::PollType::Wait);
        let mapped = slice.get_mapped_range();
        let bgra = matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let r = &mapped[(y * row) as usize..(y * row + width * 4) as usize];
            for p in r.chunks_exact(4) {
                if bgra {
                    out.extend_from_slice(&[p[2], p[1], p[0], 255]);
                } else {
                    out.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
            }
        }
        drop(mapped);
        buf.unmap();
        self.hud.layout(self.size.0 as f32, self.size.1 as f32, self.dp);
        out
    }

    /// Drive the car from a test harness without a window.
    pub fn debug_controls(&mut self, throttle: f32, steer: f32) {
        self.input.handle(&RawInput::PadAxis { axis: input::PadAxis::R2, value: throttle });
        self.input.handle(&RawInput::PadAxis { axis: input::PadAxis::LeftX, value: steer });
    }

    pub fn debug_teleport(&mut self, pos: Vec3, yaw: f32) {
        self.vehicle.reset(pos, yaw);
        self.camera.snap(&self.vehicle);
    }

    pub fn vehicle_state(&self) -> (Vec3, f32, f32) {
        (self.vehicle.pos, self.vehicle.yaw, self.vehicle.speed())
    }
}

/// Camera-facing additive quad.
fn glow_quad(out: &mut Vec<GpuVertex>, c: Vec3, size: f32, color: [u8; 4], right: Vec3, up: Vec3) {
    let (r, u) = (right * size, up * size);
    let v = |p: Vec3, uv: [f32; 2]| GpuVertex { pos: p.into(), normal: [0.0; 3], color, uv };
    let (a, b, cc, d) = (v(c - r - u, [0.0, 1.0]), v(c + r - u, [1.0, 1.0]), v(c + r + u, [1.0, 0.0]), v(c - r + u, [0.0, 0.0]));
    out.extend_from_slice(&[a, b, cc, a, cc, d]);
}

/// Headlight and brake-light glows plus the pool of light the headlights throw on the road.
fn push_car_glows(out: &mut Vec<GpuVertex>, m: &ModelInfo, t: Mat4, brake: f32, right: Vec3, up: Vec3) {
    for p in &m.headlights {
        glow_quad(out, t.transform_point3(*p + Vec3::X * 0.08), 0.75, [255, 244, 225, 210], right, up);
    }
    let b = brake.clamp(0.0, 1.0);
    for p in &m.brakelights {
        let alpha = (70.0 + 170.0 * b) as u8;
        glow_quad(out, t.transform_point3(*p - Vec3::X * 0.06), 0.28 + 0.32 * b, [255, 40, 25, alpha], right, up);
    }
    // Pool of light: a fan on the ground ahead of the car, bright at the centre. The
    // glow texture is sampled at its white centre so only the vertex alpha shapes it.
    let z = -m.ride_height + 0.07;
    let center = Vec3::new(m.front + 6.0, 0.0, z);
    let (rx, ry) = (6.5, 3.4);
    let uv = [0.5, 0.5];
    let mid = GpuVertex { pos: t.transform_point3(center).into(), normal: [0.0; 3], color: [255, 236, 205, 55], uv };
    let n = 18;
    let rim = |i: usize| {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let p = center + Vec3::new(a.cos() * rx, a.sin() * ry, 0.0);
        GpuVertex { pos: t.transform_point3(p).into(), normal: [0.0; 3], color: [255, 236, 205, 0], uv }
    };
    for i in 0..n {
        out.extend_from_slice(&[mid, rim(i), rim(i + 1)]);
    }
}
