//! Need for Speed: Underground 2, reimplemented natively. Platform layers (desktop,
//! Android) create a window/surface, forward raw input and call [`Game::frame`].

pub mod ai;
pub mod assets;
pub mod camera;
pub mod career;
pub mod cars;
pub mod collision;
pub mod config;
pub mod font;
mod frontend;
pub mod hud;
pub mod input;
pub mod menu;
pub mod play;
pub mod post;
pub mod race;
pub mod render;
pub mod roads;
pub mod traffic;
pub mod vehicle;

pub use ug2_audio::{music, sound};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{vec2, Mat4, Vec3};
use ug2_formats::bin_hash;
use ug2_formats::carinfo::WheelInfo;
use ug2_formats::fs::GameDir;

use crate::assets::{spawn_loader, GpuVertex, LoadEvent, LoadRequest, Loader, LoaderConfig};
use crate::camera::ChaseCamera;
use crate::career::Career;
use crate::cars::{car_name, display_name, model_key, CarModel};
use crate::collision::CollisionWorld;
use crate::config::GameConfig;
use crate::frontend::{PendingRace, QuickRace, Screen};
use crate::hud::{Canvas, Hud, RaceHud, StandingRow, Telemetry};
use crate::input::{Action, Controls, InputMap, InputState, RawInput, TouchPhase};
use crate::menu::Menu;
use crate::music::Music;
use crate::play::{wheel_transforms, ModelPhysics, Session};
use crate::race::RacePhase;
use crate::render::{CarDraw, FrameData, Gpu, Renderer};
use crate::roads::RoadData;
use crate::sound::{EngineSound, Sound, SoundState};
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
    skins: Vec<u32>,
    physics: ModelPhysics,
}

impl ModelInfo {
    fn new(id: usize, m: &CarModel) -> Self {
        Self {
            id,
            wheels: m.wheels,
            headlights: m.headlights.clone(),
            brakelights: m.brakelights.clone(),
            front: m.bbox_max.x,
            skins: m.skins.clone(),
            physics: ModelPhysics {
                ride_height: m.ride_height(),
                wheelbase: m.wheelbase(),
                wheel_radius: m.wheels[0].radius,
                skins: m.skins.len(),
            },
        }
    }
}

/// Paint colours offered in the menus.
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
    roads: Option<RoadData>,
    lights: Vec<ug2_formats::light::WorldLight>,
    vehicle: Vehicle,
    models: HashMap<String, ModelInfo>,
    physics: HashMap<String, ModelPhysics>,
    racers: Vec<String>,
    traffic_names: Vec<String>,
    /// Stock performance per car from its real data.
    base_perfs: HashMap<String, vehicle::Perf>,
    requested: Vec<String>,
    /// Model the player drives (key into `models`) and the one being loaded.
    player_car: Option<String>,
    pending_car: Option<String>,
    spawned: bool,
    session: Session,
    screen: Screen,
    menu: Menu,
    quick: QuickRace,
    pending_race: Option<PendingRace>,
    career: Option<Career>,
    /// The menus are in the career branch (garage shows the shop and upgrades).
    in_career: bool,
    career_path: PathBuf,
    /// Prize for the race in progress (career only) and whether it has been paid.
    race_prize: Option<(String, u32)>,
    prize_paid: Option<u32>,
    /// Seconds since the race result came in (results screen delay).
    finish_timer: f32,
    camera: ChaseCamera,
    title_time: f32,
    input: InputState,
    hud: Hud,
    canvas: Canvas,
    paused: bool,
    glows: Vec<GpuVertex>,
    brake_light: f32,
    sound: Sound,
    _music: Option<Music>,
    engine_sounds: HashMap<String, Arc<EngineSound>>,
    last_gear: i32,
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
            let lc = LoaderConfig {
                car: cfg.car.clone(),
                parts: cfg.parts_for(&cfg.car),
                location: cfg.location.clone(),
                bc_textures: gpu.bc_textures,
            };
            (Status::Loading("STARTING".into()), Some(spawn_loader(game_dir, lc)))
        } else {
            (Status::MissingData(data_dir.to_path_buf()), None)
        };
        let hud = Hud::new(cfg.steer_mode, cfg.hud_opacity);
        let sound = Sound::default();
        let music = GameDir::new(data_dir).resolve("pfdata/MusicSFx.mus").map(Music::start);
        if let Some(m) = &music {
            let ring = m.ring.clone();
            sound.with(|mx| mx.set_music(ring));
        }
        let mut session = Session::new(14);
        session.traffic_on = cfg.traffic;
        Self {
            gpu,
            surface: None,
            renderer,
            config_path,
            world_loading: loader.is_some(),
            loader,
            status,
            world: None,
            roads: None,
            lights: Vec::new(),
            vehicle: Vehicle::new(Vec3::ZERO, 0.0, 0.16, 2.6),
            models: HashMap::new(),
            physics: HashMap::new(),
            racers: Vec::new(),
            traffic_names: Vec::new(),
            base_perfs: HashMap::new(),
            requested: Vec::new(),
            player_car: None,
            pending_car: Some(model_key(&cfg.car, &cfg.parts_for(&cfg.car))),
            spawned: false,
            session,
            screen: Screen::Title,
            menu: Menu::default(),
            quick: QuickRace::default(),
            pending_race: None,
            career: None,
            in_career: false,
            career_path: config_dir.join("career.cfg"),
            race_prize: None,
            prize_paid: None,
            finish_timer: 0.0,
            camera: ChaseCamera::new(),
            title_time: 0.0,
            input,
            hud,
            canvas: Canvas::default(),
            paused: false,
            glows: Vec::new(),
            brake_light: 0.0,
            sound,
            _music: music,
            engine_sounds: HashMap::new(),
            last_gear: 1,
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
            if self.menu_visible() {
                if phase == TouchPhase::Down {
                    if let Some(hit) = self.menu.hit(vec2(x, y)) {
                        self.menu_action(hit);
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
            self.menu.selected = 0;
        }
    }

    /// Switches the player's car (with its saved visual parts), loading it if needed.
    fn select_car(&mut self, name: String) {
        let parts = self.cfg.parts_for(&name);
        let key = model_key(&name, &parts);
        if self.models.contains_key(&key) {
            self.pending_car = None;
            self.use_car(&key);
        } else {
            self.request_parts(&name, parts);
            self.pending_car = Some(key);
        }
    }

    /// Loads a stock model (opponents, traffic).
    fn request_model(&mut self, name: &str) {
        self.request_parts(name, Default::default());
    }

    fn request_parts(&mut self, name: &str, parts: ug2_formats::car::PartSet) {
        let key = model_key(name, &parts);
        if self.models.contains_key(&key) || self.requested.iter().any(|r| *r == key) {
            return;
        }
        if let Some(l) = &self.loader {
            l.request(LoadRequest::Car { name: name.to_string(), parts });
            self.requested.push(key);
        }
    }

    /// Makes the loaded model `key` the player's car.
    fn use_car(&mut self, key: &str) {
        let name = car_name(key).to_string();
        let name = name.as_str();
        let Some(m) = self.models.get(key) else { return };
        self.vehicle.ride_height = m.physics.ride_height;
        self.vehicle.wheelbase = m.physics.wheelbase;
        self.vehicle.perf = self.perf_for(name);
        if self.spawned {
            let (pos, yaw) = (self.vehicle.pos, self.vehicle.yaw);
            self.vehicle.reset(pos + Vec3::Z * 0.3, yaw);
        }
        self.player_car = Some(key.to_string());
        if let Some(e) = self.engine_sounds.get(name).cloned() {
            self.sound.with(|m| m.set_engine(&e));
        }
    }

    /// Stock performance of `car`.
    fn base_perf(&self, car: &str) -> vehicle::Perf {
        self.base_perfs.get(car).copied().unwrap_or_else(|| cars::base_perf(car))
    }

    /// Performance of the player's `car`: upgraded in the career, stock otherwise.
    fn perf_for(&self, car: &str) -> vehicle::Perf {
        let base = self.base_perf(car);
        match &self.career {
            Some(c) if c.owned.iter().any(|o| o == car) => c.perf(car, base),
            _ => base,
        }
    }

    /// The mixer the platform's audio callback pulls from.
    pub fn sound(&self) -> Sound {
        self.sound.clone()
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
                LoadEvent::CarList { racers, traffic, perfs } => {
                    log::info!("{} racers, {} traffic cars, {} with performance data", racers.len(), traffic.len(), perfs.len());
                    self.base_perfs = perfs.into_iter().collect();
                    self.racers = racers;
                    self.traffic_names = traffic.clone();
                    for t in &traffic {
                        self.request_model(t);
                    }
                }
                LoadEvent::Car(car) => {
                    let key = model_key(&car.name, &car.parts);
                    if let Some(id) = self.renderer.add_car_model(&self.gpu, &car) {
                        let info = ModelInfo::new(id, &car);
                        self.physics.insert(key.clone(), info.physics);
                        self.models.insert(key.clone(), info);
                    }
                    self.requested.retain(|r| *r != key);
                    if self.pending_car.as_deref() == Some(key.as_str()) {
                        self.pending_car = None;
                        self.use_car(&key);
                    }
                }
                LoadEvent::CarFailed(name) => {
                    self.requested.retain(|r| *r != name);
                    if self.pending_car.as_deref() == Some(name.as_str()) {
                        self.pending_car = None;
                    }
                }
                LoadEvent::Effects(fx) => {
                    self.sound.with(|m| m.set_effects(*fx));
                }
                LoadEvent::Engine { car, sound } => {
                    let sound = Arc::new(*sound);
                    if self.player_car.as_deref() == Some(car.as_str()) {
                        self.sound.with(|m| m.set_engine(&sound));
                    }
                    self.engine_sounds.insert(car, sound);
                }
                LoadEvent::Roads(r) => self.roads = Some(*r),
                LoadEvent::Collision(c) => self.world = Some(c),
                LoadEvent::Lights(l) => self.lights = l,
                LoadEvent::Spawn { pos, yaw } => {
                    self.vehicle.reset(pos, yaw);
                    self.camera.snap(&self.vehicle);
                    self.spawned = true;
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
        self.try_start_pending_race();

        let mut triggers = self.input.take_triggers();
        triggers.extend(self.hud.take_triggers());
        for t in triggers {
            match t {
                Action::Camera => self.camera.next_mode(),
                Action::Reset => self.reset_player(),
                Action::Pause => self.back_pressed(),
                _ => {}
            }
        }

        let playing = self.screen == Screen::Playing && !self.paused && self.spawned;
        let mut controls = if playing { self.controls() } else { Controls::default() };
        let countdown = matches!(self.session.race().map(|r| r.phase), Some(RacePhase::Countdown(_)));
        let finished = matches!(self.session.race().map(|r| r.phase), Some(RacePhase::Finished));
        if countdown {
            // Rev on the line, but stay put.
            controls = Controls { throttle: controls.throttle, brake: 1.0, ..Controls::default() };
        } else if finished {
            controls = Controls { brake: 0.4, ..Controls::default() };
        }
        let simulate = self.spawned && !self.paused && self.screen != Screen::Title;
        if simulate {
            let traffic_models: Vec<(String, f32, f32, usize, vehicle::Perf)> = self
                .traffic_names
                .iter()
                .filter_map(|n| self.physics.get(n).map(|p| (n.clone(), p.ride_height, p.wheelbase, p.skins, self.base_perf(n))))
                .collect();
            self.session.plan(dt, &self.vehicle, self.roads.as_ref(), &traffic_models);
            self.acc += dt;
            let mut steps = 0;
            while self.acc >= STEP && steps < 12 {
                self.vehicle.step(&controls, self.world.as_ref(), STEP);
                self.session.step(&mut self.vehicle, self.world.as_ref(), STEP);
                self.acc -= STEP;
                steps += 1;
            }
            if steps == 12 {
                self.acc = 0.0;
            }
            self.session.animate(&self.physics, dt);
        }
        if self.spawned {
            let radius = self.player_model().map_or(0.33, |m| m.physics.wheel_radius);
            self.vehicle.advance_wheels(radius, dt);
            self.camera.update(&self.vehicle, controls.look_back, self.world.as_ref(), dt);
        }
        self.title_time += dt;
        self.update_sound(&controls, simulate, dt);
        if finished {
            self.pay_prize();
            self.finish_timer += dt;
            if self.finish_timer > 2.5 && self.screen == Screen::Playing {
                self.screen = Screen::Results;
                self.menu.selected = 0;
            }
        }
        if simulate {
            self.debug_timer += dt;
            if self.debug_timer > 1.0 {
                self.debug_timer = 0.0;
                let v = &self.vehicle;
                let peak = self.sound.with(|m| std::mem::take(&mut m.peak)).unwrap_or(0.0);
                let race = self
                    .session
                    .race()
                    .map(|r| format!(" | race {:?} t {:.1} place {}/{} lap {}", r.phase, r.time, r.place(0), r.cars.len(), r.player_lap()))
                    .unwrap_or_default();
                log::info!(
                    "car pos {:.1?} speed {:.1} | thr {:.1} brk {:.1} steer {:.2} | traffic {} | audio {peak:.2}{race}",
                    v.pos,
                    v.speed() * 3.6,
                    controls.throttle,
                    controls.brake,
                    controls.steer,
                    self.session.traffic.cars.len()
                );
            }
        }
        self.build_hud();
    }

    /// Career prize, once per race, when the player crosses the line.
    fn pay_prize(&mut self) {
        if self.prize_paid.is_some() {
            return;
        }
        let place = self.session.race().map_or(9, |r| r.place(0));
        let Some((key, reward)) = self.race_prize.clone() else { return };
        let Some(c) = &mut self.career else { return };
        let prize = Career::prize(reward, place);
        c.cash += prize;
        if place == 1 {
            c.won.insert(key);
        }
        c.save();
        self.prize_paid = Some(prize);
        log::info!("career: finished {place}, prize ${prize}, cash ${}", c.cash);
    }

    fn reset_player(&mut self) {
        if let Some((p, yaw)) = self.session.race().map(|r| r.reset_point(0)) {
            self.vehicle.reset(p + Vec3::Z * (self.vehicle.ride_height + 0.5), yaw);
        } else {
            self.vehicle.reset_to_safe();
        }
        self.camera.snap(&self.vehicle);
    }

    fn update_sound(&mut self, controls: &Controls, active: bool, dt: f32) {
        let v = &mut self.vehicle;
        let shifted_up = v.gear > self.last_gear && self.last_gear > 0;
        self.last_gear = v.gear;
        let impact = std::mem::take(&mut v.impact);
        let skid = ((v.slip - 2.5) / 6.0).clamp(0.0, 1.0)
            + if controls.handbrake { (v.speed().abs() / 20.0).min(1.0) * 0.6 } else { 0.0 };
        let state = SoundState {
            rpm: v.rpm,
            throttle: controls.throttle,
            speed: v.speed().abs(),
            nitro: controls.nitro && v.nitro > 0.0 && controls.throttle > 0.1,
            skid: skid.min(1.0),
            active,
        };
        self.sound.with(|m| {
            m.set_state(state);
            if active && shifted_up {
                m.shift();
            }
            if active && impact > 3.0 {
                m.impact(((impact - 3.0) / 15.0).min(1.0));
            }
        });
        let braking = controls.brake > 0.05 && self.vehicle.speed() > 0.5;
        let target = if braking { 1.0 } else { 0.0 };
        self.brake_light += (target - self.brake_light) * (1.0 - (-25.0 * dt).exp());
    }

    fn player_model(&self) -> Option<&ModelInfo> {
        self.player_car.as_ref().and_then(|n| self.models.get(n))
    }

    fn race_hud(&self) -> Option<RaceHud> {
        let r = self.session.race()?;
        let p = &r.cars[0];
        let countdown = match r.phase {
            RacePhase::Countdown(t) => Some(t),
            _ => None,
        };
        let speed = self.vehicle.speed().abs().max(10.0);
        let standings = r
            .standings()
            .into_iter()
            .map(|car| {
                let prog = &r.cars[car];
                let name = if car == 0 {
                    "YOU".to_string()
                } else {
                    self.session.opponents[car - 1].model.clone()
                };
                StandingRow {
                    name,
                    gap: (p.total - prog.total) / speed,
                    player: car == 0,
                    finished: prog.finish_time.is_some(),
                }
            })
            .collect();
        Some(RaceHud {
            standings,
            place: r.place(0),
            cars: r.cars.len(),
            lap: r.player_lap(),
            laps: r.laps,
            circuit: r.def.line.circuit,
            remaining: r.distance() - p.total,
            time: r.time,
            countdown,
            go_flash: if countdown.is_none() { 1.0 - r.time } else { 0.0 },
            wrong_way: p.wrong_way > 1.0,
            final_lap: r.def.line.circuit && r.laps > 1 && p.lap + 1 == r.laps as i32 && p.lap_start + 3.0 > r.time,
        })
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
        if self.screen == Screen::Playing && !self.paused {
            let t = Telemetry {
                speed_kmh: self.vehicle.speed().abs() * 3.6,
                gear: self.vehicle.gear,
                rpm: self.vehicle.rpm,
                nitro: self.vehicle.nitro,
                fps: self.fps,
                draw_calls: self.renderer.stats_draws,
            };
            self.hud.draw(&mut self.canvas, &t, show_touch, self.cfg.show_fps);
            if let Some(r) = self.race_hud() {
                self.hud.race_overlay(&mut self.canvas, &r);
            }
            self.draw_minimap();
        }
        if let Status::Loading(s) = &self.status {
            let s = s.clone();
            let size = 1.6 * self.dp;
            self.canvas.text(self.size.0 as f32 * 0.5, 70.0 * self.dp, size, 0.5, &s, [1.0, 1.0, 1.0, 0.6]);
        }
        let loading = self.pending_car.as_ref().map(|n| display_name(car_name(n)).to_string()).or_else(|| {
            self.pending_race.as_ref().map(|_| "RACE".to_string())
        });
        if let Some(name) = loading {
            let text = format!("LOADING {name}");
            self.canvas.text(self.size.0 as f32 * 0.5, 90.0 * self.dp, 1.8 * self.dp, 0.5, &text, [1.0, 1.0, 1.0, 0.8]);
        }
        if self.menu_visible() {
            self.build_menu();
            let size = vec2(self.size.0 as f32, self.size.1 as f32);
            self.menu.draw(&mut self.canvas, size, self.dp);
        }
    }

    fn draw_minimap(&mut self) {
        const RANGE: f32 = 220.0;
        let Some(roads) = &self.roads else { return };
        let me = self.vehicle.pos.truncate();
        let near = |b: &[f32; 4]| {
            me.x > b[0] - RANGE && me.x < b[2] + RANGE && me.y > b[1] - RANGE && me.y < b[3] + RANGE
        };
        let polylines: Vec<Vec<glam::Vec2>> = roads
            .graph
            .routes
            .iter()
            .filter(|r| near(&r.bbox))
            .map(|r| r.points.iter().map(|p| glam::Vec2::new(p.pos[0], p.pos[1])).collect())
            .collect();
        let mut route = Vec::new();
        let mut cars = Vec::new();
        if let Some(race) = self.session.race() {
            let line = &race.def.line;
            let n = line.points.len();
            let at = race.cars[0].index;
            // The next stretch of the race, and a little behind.
            for k in 0..140 {
                let i = (at + n - 15 + k) % n;
                if !line.circuit && (at + k < 15 || at + k - 15 >= n) {
                    continue;
                }
                route.push(line.points[i].truncate());
            }
            cars.extend(self.session.opponents.iter().map(|o| (o.vehicle.pos.truncate(), [1.0, 0.25, 0.2, 1.0])));
        }
        let map = hud::Minimap {
            center: me,
            heading: self.vehicle.yaw,
            range: RANGE,
            roads: &polylines,
            route: &route,
            cars: &cars,
        };
        self.hud.minimap(&mut self.canvas, &map);
    }

    /// Renders into `target`. Borrows the HUD canvas and the renderer as disjoint fields.
    fn render_to(&mut self, target: &wgpu::TextureView, width: u32, height: u32) {
        let (view, eye) = if self.screen == Screen::Title && self.spawned {
            // Slow orbit around the parked car behind the title menu.
            let t = self.title_time * 0.15;
            let c = self.vehicle.pos + Vec3::Z * 0.6;
            let eye = c + Vec3::new(t.cos() * 7.5, t.sin() * 7.5, 1.8);
            (Mat4::look_at_rh(eye, c, Vec3::Z), eye)
        } else {
            (self.camera.view(), self.camera.eye)
        };
        let (right, up) = (view.row(0).truncate(), view.row(1).truncate());
        let mut cars = Vec::new();
        let mut glows = std::mem::take(&mut self.glows);
        glows.clear();
        let near = |p: Vec3| p.distance(eye) < 160.0;
        if let (true, Some(m)) = (self.spawned, self.player_model()) {
            let t = self.vehicle.transform();
            let wheels = wheel_transforms(&self.vehicle, t, &m.wheels);
            cars.push(CarDraw { model: m.id, transform: t, wheels, paint: self.cfg.paint, skin: None, brake: self.brake_light });
            push_car_glows(&mut glows, m, t, self.brake_light, right, up);
        }
        for o in &self.session.opponents {
            let Some(m) = self.models.get(&o.model) else { continue };
            let t = o.vehicle.transform();
            let wheels = wheel_transforms(&o.vehicle, t, &m.wheels);
            cars.push(CarDraw { model: m.id, transform: t, wheels, paint: o.paint, skin: None, brake: o.brake_light });
            if near(o.vehicle.pos) {
                push_car_glows(&mut glows, m, t, o.brake_light, right, up);
            }
        }
        for c in &self.session.traffic.cars {
            let Some(m) = self.models.get(&c.model) else { continue };
            let t = c.vehicle.transform();
            let wheels = wheel_transforms(&c.vehicle, t, &m.wheels);
            let skin = m.skins.get(c.skin).copied();
            cars.push(CarDraw { model: m.id, transform: t, wheels, paint: [0.5; 3], skin, brake: c.brake_light });
            if near(c.vehicle.pos) {
                push_car_glows(&mut glows, m, t, c.brake_light, right, up);
            }
        }
        push_world_glows(&mut glows, &self.lights, eye, view.row(2).truncate() * -1.0, right, up);
        let data = FrameData {
            view,
            fov_y: self.camera.fov_y,
            cam_pos: eye,
            cars: &cars,
            glows: &glows,
            glow_texture: bin_hash("HEADLIGHTGLOW"),
            draw_distance: self.cfg.draw_distance,
            vertex_light_scale: self.cfg.vertex_light_scale,
            hud: &self.canvas.vertices,
        };
        self.renderer.render(&self.gpu, target, width, height, &data);
        self.glows = glows;
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

/// Street-lamp glows within reach of the camera, fading in the distance.
fn push_world_glows(out: &mut Vec<GpuVertex>, lights: &[ug2_formats::light::WorldLight], eye: Vec3, forward: Vec3, right: Vec3, up: Vec3) {
    for l in lights {
        let p = Vec3::from(l.position);
        let to = p - eye;
        let d = to.length();
        if !(4.0..260.0).contains(&d) || to.dot(forward) < 0.0 {
            continue;
        }
        let fade = ((260.0 - d) / 80.0).clamp(0.0, 1.0) * ((d - 4.0) / 10.0).clamp(0.0, 1.0);
        let size = (l.size * 0.06).clamp(0.9, 3.0) * (1.0 + d / 200.0);
        let a = (150.0 * fade) as u8;
        if a == 0 {
            continue;
        }
        // Pull the sprite towards the camera so the lamp's own mesh does not clip it.
        let c = p - to / d * 0.6;
        glow_quad(out, c, size, [l.color[0], l.color[1], l.color[2], a], right, up);
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
    let z = -m.physics.ride_height + 0.07;
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
