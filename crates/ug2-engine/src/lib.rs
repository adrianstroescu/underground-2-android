//! Need for Speed: Underground 2, reimplemented natively. Platform layers (desktop,
//! Android) create a window/surface, forward raw input and call [`Game::frame`].

pub mod ai;
pub mod assets;
pub mod camera;
pub mod career;
pub mod cars;
pub mod collision;
pub mod config;
pub mod drift;
pub mod font;
pub mod fe;
pub mod fe_hud;
pub mod fe_list;
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
mod movie_view;
pub mod visual;

pub use ug2_audio::{music, sound};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{vec2, Mat4, Vec3};
use ug2_formats::bin_hash;
use ug2_formats::carinfo::WheelInfo;
use ug2_formats::fs::GameDir;
use ug2_formats::visible::VisibleSections;

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
use crate::roads::{Mode as RaceMode, RaceInfo, RoadData};
use crate::sound::{EngineSound, Sound, SoundState};
use crate::vehicle::{Shift, Vehicle};

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

/// Underglow colours (index 1..).
const NEONS: &[(&str, [u8; 3])] = &[
    ("BLUE", [40, 90, 255]),
    ("GREEN", [40, 255, 90]),
    ("RED", [255, 30, 30]),
    ("PURPLE", [170, 40, 255]),
    ("PINK", [255, 60, 190]),
    ("CYAN", [40, 230, 255]),
    ("YELLOW", [255, 220, 40]),
    ("WHITE", [235, 240, 255]),
];

/// Window tints (index 1..): darker glass multipliers.
const TINTS: &[(&str, [f32; 3])] = &[
    ("LIGHT SMOKE", [0.65, 0.65, 0.68]),
    ("DARK SMOKE", [0.4, 0.4, 0.42]),
    ("LIMO", [0.18, 0.18, 0.2]),
    ("GREEN", [0.45, 0.7, 0.5]),
    ("BLUE", [0.4, 0.5, 0.8]),
    ("GOLD", [0.8, 0.65, 0.35]),
];

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
    /// Every race of the installed regions.
    catalog: Vec<RaceInfo>,
    /// World region loaded (or loading), e.g. "L4RA".
    region: String,
    /// Another region is loading: the loading screen covers the old world.
    switching: bool,
    /// Go to free roam once Bayview is back.
    pending_free_roam: bool,
    /// Textures of the loaded region, and of cars (kept when the region changes).
    world_textures: HashSet<u32>,
    car_textures: HashSet<u32>,
    /// Scenery group sets of the loaded region (see [`LoadEvent::Groups`]) and the race
    /// track whose barriers are switched on (0: free roam).
    group_sets: Vec<Vec<String>>,
    groups_for: Option<u16>,
    /// Big centred race message (shift timing, blown engine): text, colour, time left.
    banner: Option<(String, [f32; 4], f32)>,
    /// The last manual shift's timing and a serial (the HUD's ShiftMessage).
    last_shift: Option<(Shift, u32)>,
    /// Driver names of the race's opponents (quick race).
    race_names: Vec<String>,
    /// Steering input last frame (drag lane changes fire on a new press).
    lane_input: f32,
    /// Driver for the player's car when the `autopilot` test option is on.
    autopilot: Option<crate::ai::AiDriver>,
    /// Seconds the autopilot has been stuck (it is put back on the line after 3).
    autopilot_stuck: f32,
    /// World lights with their streaming section, and those of the visible sections.
    all_lights: Vec<(u16, ug2_formats::light::WorldLight)>,
    lights: Vec<ug2_formats::light::WorldLight>,
    /// The region's visible section manager, the sections listed anywhere in it, and the
    /// drivable sections the current draw and collision sets were made for.
    visibility: Option<Box<VisibleSections>>,
    env_maps: Option<Box<ug2_formats::envmap::EnvMaps>>,
    /// The movie playing (FMV), drawn instead of the game, and the ones queued after it.
    movie: Option<ug2_movie::Movie>,
    movie_queue: std::collections::VecDeque<std::path::PathBuf>,
    movie_view: movie_view::MovieView,
    /// The original frontend screens (FEng).
    fe: fe::Frontend,
    fe_verts: Vec<render::FeVertex>,
    /// Whether the Options were last applied on the frontend (menu music) or not.
    options_title: Option<bool>,
    options_camera: Option<camera::CameraMode>,
    /// Frontend textures already asked of the loader.
    fe_requested: std::collections::HashSet<u32>,
    fe_batches: Vec<render::FeBatch>,
    /// The original game files.
    data_dir: std::path::PathBuf,
    visual: visual::VisualTreatment,
    /// Length of the last frame, seconds.
    frame_dt: f32,
    vis_listed: Vec<bool>,
    vis_view: Option<u16>,
    vis_cars: Vec<u16>,
    /// Regular paint light materials by their colour, and the traffic cars' material.
    paint_materials: Vec<([f32; 3], u16)>,
    traffic_material: u16,
    vehicle: Vehicle,
    models: HashMap<String, ModelInfo>,
    physics: HashMap<String, ModelPhysics>,
    racers: Vec<String>,
    traffic_names: Vec<String>,
    /// Rim styles offered in the garage.
    rim_names: Vec<String>,
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
        let movie_view = movie_view::MovieView::new(&gpu.device, format);
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
            catalog: Vec::new(),
            region: cfg.location.clone(),
            switching: false,
            pending_free_roam: false,
            world_textures: HashSet::new(),
            car_textures: HashSet::new(),
            group_sets: Vec::new(),
            groups_for: None,
            banner: None,
            last_shift: None,
            race_names: Vec::new(),
            lane_input: 0.0,
            autopilot: None,
            autopilot_stuck: 0.0,
            all_lights: Vec::new(),
            lights: Vec::new(),
            visibility: None,
            env_maps: None,
            movie: None,
            // As the PC game at start: the EA and THX logos, then the opening movie.
            movie_queue: ["MOVIES/ealogo.vp6", "MOVIES/THX_logo.vp6", "MOVIES/FMVOpening.vp6"]
                .iter()
                .filter_map(|m| GameDir::new(data_dir).resolve(m))
                .collect(),
            movie_view,
            fe: fe::Frontend::default(),
            fe_verts: Vec::new(),
            options_title: None,
            options_camera: None,
            fe_requested: Default::default(),
            fe_batches: Vec::new(),
            data_dir: data_dir.to_path_buf(),
            visual: visual::VisualTreatment::default(),
            frame_dt: 1.0 / 60.0,
            vis_listed: Vec::new(),
            vis_view: None,
            vis_cars: Vec::new(),
            paint_materials: Vec::new(),
            traffic_material: 0,
            vehicle: Vehicle::new(Vec3::ZERO, 0.0, 0.16, 2.6),
            models: HashMap::new(),
            physics: HashMap::new(),
            racers: Vec::new(),
            traffic_names: Vec::new(),
            base_perfs: HashMap::new(),
            rim_names: Vec::new(),
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
        if self.movie.is_some() {
            // Any tap or key skips the movie, as Enter does on the PC.
            let skip = match &ev {
                RawInput::Touch { phase, .. } => *phase == TouchPhase::Down,
                RawInput::Key { pressed, .. } | RawInput::PadButton { pressed, .. } => *pressed,
                RawInput::PadAxis { .. } => false,
            };
            if skip {
                self.end_movie();
            }
            return;
        }
        if self.fe_active() {
            let action = match &ev {
                RawInput::Touch { phase: TouchPhase::Down, x, y, .. } => self.fe.tap(*x, *y, self.size),
                RawInput::Key { code, pressed: true } if matches!(code.as_str(), "Escape" | "Backspace" | "BrowserBack") => {
                    self.fe_back();
                    None
                }
                RawInput::Key { code, pressed: true } => match code.as_str() {
                    "ArrowLeft" => {
                        self.fe.step(-1);
                        None
                    }
                    "ArrowUp" => {
                        self.fe.step_row(-1);
                        None
                    }
                    "ArrowDown" => {
                        self.fe.step_row(1);
                        None
                    }
                    "ArrowRight" => {
                        self.fe.step(1);
                        None
                    }
                    "Enter" | "Space" | "NumpadEnter" => self.fe.activate(),
                    _ => None,
                },
                _ => None,
            };
            if let Some(a) = action {
                self.fe_action(a);
            }
            if matches!(ev, RawInput::Touch { .. } | RawInput::Key { .. }) {
                return;
            }
        }
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
        self.apply_engine_sound();
    }

    /// Replaces the world with `region` (another race area). Waits for a load in
    /// progress to finish first: call again until [`Game::region`] matches.
    pub(crate) fn load_region(&mut self, region: &str) {
        if self.region == region || self.world_loading {
            return;
        }
        let Some(loader) = &self.loader else { return };
        log::info!("loading region {region}");
        loader.request(LoadRequest::World(region.to_string()));
        self.region = region.to_string();
        self.session.free_roam();
        self.session.traffic.cars.clear();
        let drop: Vec<u32> = self.world_textures.drain().filter(|h| !self.car_textures.contains(h)).collect();
        self.renderer.clear_world(&drop);
        self.world = None;
        self.roads = None;
        self.all_lights.clear();
        self.lights.clear();
        self.visibility = None;
        self.world_loading = true;
        self.switching = true;
        self.status = Status::Loading("LOADING".into());
    }

    pub(crate) fn start_free_roam(&mut self) {
        self.session.free_roam();
        self.screen = Screen::Playing;
        self.set_paused(false);
        self.camera.snap(&self.vehicle);
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

    /// Engine sound for the player's car at its engine upgrade level, loading it if needed.
    fn apply_engine_sound(&mut self) {
        let Some(key) = &self.player_car else { return };
        let car = car_name(key).to_string();
        let level = match &self.career {
            Some(c) if self.in_career => c.engine_level(&car),
            _ => 0,
        };
        let sound_key = format!("{car}#{level}");
        match self.engine_sounds.get(&sound_key).cloned() {
            Some(e) => {
                self.sound.with(|m| m.set_engine(&e));
            }
            None if level > 0 => {
                if let Some(l) = &self.loader {
                    l.request(LoadRequest::Engine { car, level });
                }
            }
            None => {}
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
                    if !matches!(self.status, Status::Ready) || self.switching {
                        self.status = Status::Loading(s);
                    }
                }
                LoadEvent::Textures(ts) => {
                    for t in &ts {
                        self.renderer.upload_texture(&self.gpu, t);
                        self.car_textures.insert(t.hash);
                    }
                }
                LoadEvent::WorldTextures(ts) => {
                    for t in &ts {
                        self.renderer.upload_texture(&self.gpu, t);
                        self.world_textures.insert(t.hash);
                    }
                }
                LoadEvent::Groups(g) => {
                    self.group_sets = g;
                    self.groups_for = None;
                }
                LoadEvent::Catalog(c) => {
                    log::info!("{} races in the installed regions", c.len());
                    self.catalog = c;
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
                    self.engine_sounds.insert(car, Arc::new(*sound));
                    self.apply_engine_sound();
                }
                LoadEvent::Roads(r) => self.roads = Some(*r),
                LoadEvent::Rims(r) => self.rim_names = r,
                LoadEvent::Collision(c) => {
                    self.world = Some(c);
                    self.groups_for = None;
                    self.vis_cars.clear();
                }
                LoadEvent::Lights(l) => {
                    self.lights = l.iter().map(|(_, l)| l.clone()).collect();
                    self.all_lights = l;
                    self.vis_view = None;
                }
                LoadEvent::Materials(list) => {
                    self.renderer.set_materials(&self.gpu, &list);
                    self.paint_materials = PAINT_MATERIALS
                        .iter()
                        .filter_map(|(name, rgb)| {
                            let id = list.iter().position(|m| m.name == *name)? as u16 + 1;
                            Some((*rgb, id))
                        })
                        .collect();
                    self.traffic_material = list.iter().position(|m| m.name == "TRAFFIC").map_or(0, |i| i as u16 + 1);
                }
                LoadEvent::Platform(mesh) => {
                    self.renderer.set_platform(&self.gpu, &mesh);
                }
                LoadEvent::FeTextures(list, blends) => {
                    let blend_of: HashMap<u32, assets::FeBlend> = blends.into_iter().collect();
                    for t in &list {
                        let b = blend_of.get(&t.hash).copied().unwrap_or(assets::FeBlend::Alpha);
                        self.renderer.upload_fe_texture(&self.gpu, t, b);
                    }
                }
                LoadEvent::Frontend(d) => {
                    log::info!("frontend ready: {:?}", fe::labels(&d.strings));
                    self.fe.data = Some(d);
                }
                LoadEvent::EnvMaps(m) => {
                    log::info!("environment maps: {}", m.maps.len());
                    self.env_maps = Some(m);
                    self.visual.reset();
                }
                LoadEvent::Visibility(v) => {
                    let mut listed = Vec::new();
                    for &s in v.iter().flat_map(|v| v.lists.values()).flatten() {
                        if listed.len() <= s as usize {
                            listed.resize(s as usize + 1, false);
                        }
                        listed[s as usize] = true;
                    }
                    log::info!("visible sections: {} drivable", v.as_ref().map_or(0, |v| v.lists.len()));
                    self.visibility = v;
                    self.vis_listed = listed;
                    self.vis_view = None;
                    self.vis_cars.clear();
                }
                LoadEvent::Spawn { pos, yaw } => {
                    self.vehicle.reset(pos, yaw);
                    self.camera.snap(&self.vehicle);
                    self.spawned = true;
                    if !self.switching {
                        self.status = Status::Ready;
                    }
                }
                LoadEvent::Section(s) => self.renderer.add_section(&self.gpu, &s.mesh, s.sky, s.group, s.number),
                LoadEvent::Done => {
                    log::info!(
                        "loaded: {} sections, {} textures",
                        self.renderer.section_count(),
                        self.renderer.texture_count()
                    );
                    self.status = Status::Ready;
                    self.world_loading = false;
                    self.switching = false;
                }
                LoadEvent::Failed(e) => {
                    self.status = Status::Failed(e);
                    self.world_loading = false;
                    self.switching = false;
                }
            }
        }
        if lost {
            self.loader = None;
            self.world_loading = false;
        }
    }

    /// Advances the simulation by `dt` seconds (real time, clamped).
    /// Starts the next queued movie when none is playing; ends a finished one.
    fn update_movies(&mut self) {
        if self.movie.as_ref().is_some_and(|m| m.finished()) {
            self.end_movie();
        }
        while self.movie.is_none() {
            let Some(path) = self.movie_queue.pop_front() else { break };
            match ug2_movie::Movie::open(&path) {
                Ok(m) => {
                    let ring = m.ring.clone();
                    self.sound.with(|mx| mx.set_movie(Some(ring)));
                    log::info!("movie {}", path.display());
                    self.movie = Some(m);
                }
                Err(e) => log::warn!("movie {}: {e:#}", path.display()),
            }
        }
    }

    fn end_movie(&mut self) {
        self.movie = None;
        self.sound.with(|mx| mx.set_movie(None));
    }

    pub(crate) fn movie_playing(&self) -> bool {
        self.movie.is_some() || !self.movie_queue.is_empty()
    }

    /// Queues a movie (`MOVIES/<name>.vp6`) to play now.
    pub fn play_movie(&mut self, name: &str) {
        if let Some(p) = GameDir::new(&self.data_dir).resolve(&format!("MOVIES/{name}.vp6")) {
            self.movie_queue.push_back(p);
        }
    }

    /// The original main menu replaces the title menu once the frontend data is in.
    fn fe_active(&self) -> bool {
        self.fe.screen.is_some()
            && self.menu_visible()
            && (self.screen == Screen::Title || (self.screen == Screen::Playing && self.paused) || self.original_results())
    }

    /// Races whose HUD is HUD_SingleRace (circuit, sprint, URL) use the original HUD;
    /// drag, drift and Street X have their own packages, not done yet.
    fn original_hud(&self) -> bool {
        self.hud_kind().is_some()
    }

    /// The HUD package for what is being driven; None keeps the port's HUD (drift is
    /// not done yet, nor free roam outside Bayview).
    fn hud_kind(&self) -> Option<fe_hud::HudKind> {
        if self.screen != Screen::Playing {
            return None;
        }
        match self.session.race().map(|r| r.def.info.mode) {
            Some(roads::Mode::Circuit | roads::Mode::Sprint | roads::Mode::Url) => Some(fe_hud::HudKind::Single),
            Some(roads::Mode::StreetX) => Some(fe_hud::HudKind::ShortTrack),
            Some(roads::Mode::Drag) => Some(fe_hud::HudKind::Drag),
            Some(roads::Mode::Drift) => Some(fe_hud::HudKind::Drift),
            None => (self.region == "L4RA").then_some(fe_hud::HudKind::Single),
        }
    }

    fn update_race_hud(&mut self, dt: f32) {
        let on = self.original_hud() && self.fe.ready();
        self.hud.original_gauges = on;
        if !on {
            self.fe.race_hud = None;
            return;
        }
        let kind = self.hud_kind().unwrap_or(fe_hud::HudKind::Single);
        if self.fe.race_hud.as_ref().is_some_and(|h| h.kind != kind) {
            self.fe.race_hud = None;
        }
        if self.fe.race_hud.is_none() {
            if !self.fe.open_race_hud(kind) {
                return;
            }
            // The race's map (TRACKMAP4000 in free roam).
            if let Some(l) = &self.loader {
                let t = self.session.race().map_or(4000, |r| r.def.info.track);
                if self.fe_requested.insert(bin_hash(&format!("TRACKMAP{t}"))) {
                    l.request(LoadRequest::TrackMap(t));
                }
            }
        }
        if let Some(input) = self.hud_input() {
            self.fe.update_race_hud(&input, self.size, dt);
        }
    }

    fn hud_input(&self) -> Option<fe_hud::HudInput> {
        let Some(r) = self.session.race() else { return self.explore_hud_input() };
        let rh = self.race_hud()?;
        let p = &r.cars[0];
        let car = self.current_car();
        let [idle, redline, max, turbo] = self.fe.data.as_ref().and_then(|d| d.engines.get(&car).copied()).unwrap_or([1000.0, 7000.0, 7500.0, 0.0]);
        // The port's engine speed is 0.18..1 over idle..max.
        let rpm = idle + (max - idle) * ((self.vehicle.rpm - 0.18) / 0.82).clamp(0.0, 1.0);
        let player_name = self.fe.data.as_ref().and_then(|d| d.strings.by_hash(0xd871_33bc)).unwrap_or("Player 1").to_string();
        let leaders = rh
            .standings
            .iter()
            .enumerate()
            .take(4)
            .map(|(k, s)| {
                // Short car names (CAR_NAME_<type>) stand in for the AI drivers' names.
                let short = self.fe.data.as_ref().and_then(|d| d.strings.by_hash(bin_hash(&format!("CAR_NAME_{}", car_name(&s.name)))));
                let name = if s.player {
                    player_name.clone()
                } else if self.race_names.contains(&s.name) {
                    s.name.clone()
                } else {
                    short.map_or_else(|| display_name(&s.name).to_string(), str::to_string)
                };
                (k + 1, name, s.gap, s.player, s.value.clone())
            })
            .collect();
        let map_mode = self.cfg.options.get("race_map").copied().unwrap_or(2.0);
        let info = &r.def.info;
        let map = (map_mode != 0.0).then(|| fe_hud::MapInput {
            texture: bin_hash(&format!("TRACKMAP{}", info.track)),
            calib: info.map,
            rotate: map_mode == 2.0,
            player: (self.vehicle.pos.x, self.vehicle.pos.y, self.vehicle.yaw),
            others: self.session.opponents.iter().map(|o| (o.vehicle.pos.x, o.vehicle.pos.y, o.vehicle.yaw)).collect(),
        });
        let racing = rh.countdown.is_none();
        let shift_message = self.last_shift.map(|(q, serial)| {
            let label = match q {
                Shift::Perfect => 0x0598_b065,
                Shift::Good => 0x27d2_dd45,
                Shift::Early => 0x2303_d76c,
                Shift::Late => 0x27d7_5cd8,
            };
            (label, serial)
        });
        let drift = r.drift.as_ref().map(|d| fe_hud::DriftInput {
            total: d.score.total,
            pending: d.score.pending(),
            multiplier: d.score.multiplier,
            best: d.score.best,
            banks: d.score.banks,
            last_bank: d.score.last_bank,
            angle: self.vehicle.drift_angle().to_degrees(),
        });
        Some(fe_hud::HudInput {
            race: true,
            drift,
            race_time: r.time,
            heat: self.vehicle.heat,
            shift_message,
            speed_ms: self.vehicle.speed().abs(),
            rpm,
            redline,
            max_rpm: max,
            turbo: turbo != 0.0,
            gear: self.vehicle.gear,
            mph: self.hud.mph,
            nitro: Some((self.vehicle.nitro, self.vehicle.nitro_on)),
            place: rh.place,
            racers: rh.cars,
            lap: rh.lap,
            laps: rh.laps,
            progress: (!rh.circuit).then(|| (1.0 - rh.remaining / r.distance().max(1.0)).clamp(0.0, 1.0)),
            lap_time: racing.then(|| r.time - p.lap_start),
            countdown: rh.countdown,
            go_flash: rh.go_flash,
            wrong_way: rh.wrong_way,
            leaders,
            map,
        })
    }

    /// Free roam: gauges and the explore map (Explore Map Mode).
    fn explore_hud_input(&self) -> Option<fe_hud::HudInput> {
        let car = self.current_car();
        let [idle, redline, max, turbo] = self.fe.data.as_ref().and_then(|d| d.engines.get(&car).copied()).unwrap_or([1000.0, 7000.0, 7500.0, 0.0]);
        let rpm = idle + (max - idle) * ((self.vehicle.rpm - 0.18) / 0.82).clamp(0.0, 1.0);
        let mode = self.cfg.options.get("explore_map").copied().unwrap_or(2.0);
        let calib = self.fe.data.as_ref().and_then(|d| d.explore_map);
        let map = calib.filter(|_| mode != 0.0).map(|calib| fe_hud::MapInput {
            texture: bin_hash("TRACKMAP4000"),
            calib,
            rotate: mode == 2.0,
            player: (self.vehicle.pos.x, self.vehicle.pos.y, self.vehicle.yaw),
            others: Vec::new(),
        });
        Some(fe_hud::HudInput {
            race: false,
            drift: None,
            race_time: 0.0,
            heat: 0.0,
            shift_message: None,
            speed_ms: self.vehicle.speed().abs(),
            rpm,
            redline,
            max_rpm: max,
            turbo: turbo != 0.0,
            gear: self.vehicle.gear,
            mph: self.hud.mph,
            nitro: Some((self.vehicle.nitro, self.vehicle.nitro_on)),
            place: 1,
            racers: 1,
            lap: 1,
            laps: 1,
            progress: None,
            lap_time: None,
            countdown: None,
            go_flash: 0.0,
            wrong_way: false,
            leaders: Vec::new(),
            map,
        })
    }

    /// Quick race results use the original screens (career results are still the port's).
    fn original_results(&self) -> bool {
        self.screen == Screen::Results && self.race_prize.is_none()
    }

    /// The results table rows, in finishing order.
    fn result_rows(&self) -> Vec<fe::ResultRow> {
        let Some(r) = self.session.race() else { return Vec::new() };
        let player_name = self.fe.data.as_ref().and_then(|d| d.strings.by_hash(0xd871_33bc)).unwrap_or("Player 1").to_string();
        if r.drift.is_some() {
            return r
                .drift_standings()
                .into_iter()
                .map(|(name, score, player)| fe::ResultRow {
                    name: if player { player_name.clone() } else { name },
                    car: if player { self.current_car() } else { String::new() },
                    time: None,
                    retired: false,
                    knocked: false,
                    score: Some(score),
                    player,
                    lap_times: Vec::new(),
                    lap_places: Vec::new(),
                    laps_led: 0,
                    laps: r.laps,
                    best_lap: None,
                    top_speed: 0.0,
                    avg_speed: 0.0,
                })
                .collect();
        }
        r.standings()
            .into_iter()
            .map(|car| {
                let p = &r.cars[car];
                let model = if car == 0 { self.current_car() } else { self.session.opponents[car - 1].model.clone() };
                fe::ResultRow {
                    name: if car == 0 {
                        player_name.clone()
                    } else {
                        self.race_names.get(car - 1).cloned().unwrap_or_else(|| cars::display_name(&model).to_string())
                    },
                    car: cars::car_name(&model).to_string(),
                    time: r.cars[car].finish_time,
                    retired: r.cars[car].retired,
                    knocked: r.cars[car].knocked_out,
                    score: None,
                    player: car == 0,
                    lap_times: p.lap_times.clone(),
                    lap_places: p.lap_places.clone(),
                    laps_led: p.laps_led,
                    laps: r.laps,
                    best_lap: p.best_lap,
                    top_speed: p.top_speed,
                    avg_speed: p.finish_time.map_or(0.0, |t| r.distance() / t.max(0.1)),
                }
            })
            .collect()
    }

    /// Runs one of the port's own menu rows on the current screen (pause actions).
    fn fe_row(&mut self, row: frontend::Row) {
        if let Some(i) = self.rows().iter().position(|r| *r == row) {
            self.menu_action(menu::MenuHit::Activate(i));
        }
    }

    fn update_fe(&mut self, dt: f32) {
        let ready = self.menu_visible() && self.fe.ready();
        let title = self.screen == Screen::Title && ready;
        let pause = self.screen == Screen::Playing && self.paused && ready;
        if title && (self.fe.screen.is_none() || self.fe.in_pause() || self.fe.in_post_race()) {
            self.fe.open_main_menu();
        } else if pause && !self.fe.in_pause() {
            let race = self.session.race().is_some();
            self.fe.open_pause(race);
        } else if self.original_results() && ready && !self.fe.in_post_race() {
            let rows = self.result_rows();
            let drift = self.session.race().is_some_and(|r| r.drift.is_some());
            self.fe.open_results(rows, drift);
        } else if !title && !pause && !(self.original_results() && ready) && self.fe.screen.is_some() {
            self.fe.close();
        }
        // At the end of the credits the game goes back to the options.
        if self.fe.kind() == Some(fe::FeKind::Credits) && self.fe.credits_done() {
            self.fe.open_options_main();
        }
        // The car select shows the browsed car on the turntable.
        if let Some(name) = self.fe.wanted_car().map(str::to_string) {
            if self.current_car() != name {
                self.select_car(name);
            }
        }
        // Track maps load when a track is first shown.
        if let Some(t) = self.fe.wanted_texture() {
            if self.fe_requested.insert(t) {
                if let Some(n) = self.catalog.iter().find(|r| bin_hash(&format!("TRACKMAP{}", r.track)) == t).map(|r| r.track) {
                    if let Some(l) = &self.loader {
                        l.request(LoadRequest::TrackMap(n));
                    }
                }
            }
        }
        self.fe.update(dt);
    }

    /// Back on an original screen: up one level; false at the main menu.
    fn fe_back(&mut self) -> bool {
        if self.fe.close_dialog() {
            return true;
        }
        match self.fe.kind() {
            Some(fe::FeKind::QuickRaceMode) => {
                self.fe.open_main_menu();
                true
            }
            Some(fe::FeKind::QuickRaceTrack) => {
                self.fe.open_quick_race_modes();
                true
            }
            Some(fe::FeKind::PostRace) => {
                let rows = self.result_rows();
                let drift = self.session.race().is_some_and(|r| r.drift.is_some());
                self.fe.open_results(rows, drift);
                true
            }
            Some(fe::FeKind::PostRaceResults) => true,
            Some(fe::FeKind::PostRaceStats) => {
                if self.fe.stats_from_menu() {
                    self.fe.open_post_race();
                } else {
                    let rows = self.result_rows();
                    let drift = self.session.race().is_some_and(|r| r.drift.is_some());
                    self.fe.open_results(rows, drift);
                }
                true
            }
            Some(fe::FeKind::Crib) => {
                // Back offers to quit to the main menu.
                if !self.fe.open_dialog(0x30ed_2368, vec![(0x0639, fe::FeAction::ConfirmCribQuit), (0x5413_89e5, fe::FeAction::CloseDialog)]) {
                    self.fe.open_main_menu();
                }
                true
            }
            Some(fe::FeKind::StartCareer) => {
                self.fe.open_main_menu();
                true
            }
            Some(fe::FeKind::Pause) => {
                self.set_paused(false);
                true
            }
            Some(fe::FeKind::PauseOptionsMain) => {
                let race = self.session.race().is_some();
                self.fe.open_pause(race);
                true
            }
            Some(fe::FeKind::PauseOptions) => {
                if let Some((_, values)) = self.fe.options_values() {
                    for (k, v) in values {
                        self.cfg.options.insert(k.to_string(), v);
                    }
                    self.cfg.save(&self.config_path);
                    self.apply_options();
                }
                self.fe.open_pause_options_main();
                true
            }
            Some(fe::FeKind::Credits) => {
                self.fe.open_options_main();
                true
            }
            Some(fe::FeKind::OptionsMain) => {
                self.fe.open_main_menu();
                true
            }
            Some(fe::FeKind::Options) => {
                // Leaving a category keeps its values (the profile is saved).
                if let Some((_, values)) = self.fe.options_values() {
                    for (k, v) in values {
                        self.cfg.options.insert(k.to_string(), v);
                    }
                    self.cfg.save(&self.config_path);
                    self.apply_options();
                }
                self.fe.open_options_main();
                true
            }
            Some(fe::FeKind::QuickRaceCar) => {
                let mode = self.quick.chosen.as_ref().map(|r| r.mode);
                let settings = self.quick.settings[frontend::QuickRace::slot(mode)];
                self.fe.open_mode_options(mode, self.quick.chosen.clone(), settings);
                true
            }
            Some(fe::FeKind::QuickRaceOptions) => {
                match self.quick.chosen.as_ref().map(|r| r.mode) {
                    Some(m) => self.open_qr_tracks(m),
                    None => {
                        self.fe.open_quick_race_modes();
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Puts the Options values into effect where the port has the feature.
    fn apply_options(&mut self) {
        let o = &self.cfg.options;
        let get = |k: &str, d: f32| o.get(k).copied().unwrap_or(d);
        // Menu music on the frontend, game music elsewhere.
        let music = if self.screen == Screen::Title { get("menu_music_volume", 0.7) } else { get("game_music_volume", 1.0) };
        let levels = ug2_audio::sound::Levels {
            master: get("master_volume", 1.0),
            music,
            speech: get("speech_volume", 1.0),
            effects: get("sfx_volume", 1.0),
            tires: get("tire_volume", 1.0),
            engine: get("engine_volume", 1.0),
            mono: get("audio_mode", 1.0) == 0.0,
        };
        self.sound.with(|m| m.levels = levels);
        self.hud.mph = get("speedo_units", 0.0) == 0.0;
        // The Camera option is the view a drive starts with (the camera button cycles).
        let camera = camera::CameraMode::from_option(get("camera", 3.0) as u8);
        if self.options_camera != Some(camera) {
            self.options_camera = Some(camera);
            self.camera.mode = camera;
        }
    }

    /// The original car select: the racer car types in GLOBALB order, without the ones
    /// the North American release leaves out (speed2.exe 0x5022c0), those with no model
    /// installed and those still locked (the starter set, plus the career's cars and the
    /// car in use).
    fn open_qr_cars(&mut self) {
        let Some(data) = &self.fe.data else { return };
        let current = self.current_car();
        let owned: Vec<String> = self.career.as_ref().map(|c| c.owned.clone()).unwrap_or_default();
        let stock: Vec<(String, String)> = data
            .cars
            .iter()
            .filter(|(n, _)| !matches!(n.as_str(), "CORSA" | "PEUGOT106"))
            .filter(|(n, _)| self.racers.iter().any(|r| r == n))
            .filter(|(n, _)| fe::STARTER_CARS.contains(&n.as_str()) || owned.contains(n) || *n == current)
            .cloned()
            .collect();
        let career: Vec<(String, String)> =
            owned.iter().filter_map(|o| data.cars.iter().find(|(n, _)| n == o)).cloned().collect();
        // All Cars (stock + My Cars; no customised cars yet), Stock Cars, My Cars, Career.
        let filters = vec![(0x6b68_362c, stock.clone()), (0xe798_a2f7, stock), (0xe7ab_43d3, Vec::new()), (0xb404_5365, career)];
        let mode_label = match self.quick.chosen.as_ref().map(|r| r.mode) {
            Some(roads::Mode::Circuit) => 0x93fd_ee74,
            Some(roads::Mode::Sprint) => 0xdcb9_f861,
            Some(roads::Mode::Drag) => 0x3510_2edf,
            Some(roads::Mode::Drift) => 0xd716_2cfa,
            Some(roads::Mode::StreetX) => 0x30da_61e1,
            Some(roads::Mode::Url) => 0xc786_c4e7,
            None => 0x6f24_21b7,
        };
        self.fe.open_car_select(filters, &current, mode_label);
    }

    /// The original track select for a mode: each track once, named by the mode's
    /// direction setting.
    fn open_qr_tracks(&mut self, m: roads::Mode) {
        self.quick.mode = m;
        let reverse = self.quick.settings[frontend::QuickRace::slot(Some(m))].reverse;
        let mut races: Vec<(usize, roads::RaceInfo)> = Vec::new();
        for (i, r) in self.catalog.iter().enumerate().filter(|(_, r)| r.mode == m) {
            if let Some(at) = races.iter().position(|(_, o)| o.track == r.track) {
                // Prefer the forward entry.
                if races[at].1.reverse && !r.reverse {
                    races[at] = (i, r.clone());
                }
                continue;
            }
            races.push((i, r.clone()));
        }
        for (_, r) in &mut races {
            if r.reversible {
                r.reverse = reverse;
            }
        }
        let refs: Vec<(usize, &roads::RaceInfo)> = races.iter().map(|(i, r)| (*i, r)).collect();
        self.fe.open_track_select(&refs);
    }

    fn fe_action(&mut self, a: fe::FeAction) {
        let row = match a {
            fe::FeAction::Career => {
                let in_progress = self.career.as_ref().is_some_and(|c| !c.won.is_empty() || c.owned.len() > 1);
                if !self.fe.open_start_career(in_progress) {
                    self.fe_go(frontend::Row::Career);
                }
                return;
            }
            fe::FeAction::ResumeCareer => {
                // Resume lands on the crib menu (speed2.exe 0x495a30).
                if !self.fe.open_crib(true) {
                    self.fe_go(frontend::Row::Career);
                }
                return;
            }
            fe::FeAction::ExploreMode => {
                // "Return to Explore Mode?" then free roam.
                if !self.fe.open_dialog(0x60b4_2afb, vec![(0x0639, fe::FeAction::ConfirmExplore), (0x5413_89e5, fe::FeAction::CloseDialog)]) {
                    self.fe_go(frontend::Row::FreeRoam);
                }
                return;
            }
            fe::FeAction::ConfirmExplore => frontend::Row::FreeRoam,
            fe::FeAction::WorldMap => frontend::Row::Career,
            fe::FeAction::CareerCarSelect | fe::FeAction::ChangeParts => {
                self.in_career = true;
                frontend::Row::Garage
            }
            fe::FeAction::PerformanceTuning | fe::FeAction::ViewRewards => {
                log::info!("{a:?} is not available yet");
                return;
            }
            fe::FeAction::SaveCareer => {
                if let Some(c) = &self.career {
                    c.save();
                }
                return;
            }
            fe::FeAction::CribQuit => {
                if !self.fe.open_dialog(0x30ed_2368, vec![(0x0639, fe::FeAction::ConfirmCribQuit), (0x5413_89e5, fe::FeAction::CloseDialog)]) {
                    self.fe.open_main_menu();
                }
                return;
            }
            fe::FeAction::ConfirmCribQuit => {
                self.fe.open_main_menu();
                return;
            }
            fe::FeAction::NewCareer => {
                if self.career.as_ref().is_some_and(|c| !c.won.is_empty() || c.owned.len() > 1) {
                    // Starting over would erase the saved career: needs the game's
                    // confirmation dialog (not done yet).
                    log::info!("new career: a career is in progress");
                    return;
                }
                // A new career opens with scene01 (speed2.exe 0x53d710).
                self.play_movie("Scene01");
                frontend::Row::Career
            }
            fe::FeAction::LoadGame => {
                log::info!("load game is not available");
                return;
            }
            fe::FeAction::QuickRace => {
                self.fe.open_quick_race_modes();
                return;
            }
            fe::FeAction::Customize => frontend::Row::Garage,
            fe::FeAction::Options => {
                self.fe.open_options_main();
                return;
            }
            fe::FeAction::OptionsCategory(c) => {
                if c == 8 {
                    self.fe.open_credits();
                    return;
                }
                let opts = self.cfg.options.clone();
                if !self.fe.open_options(c, |k| opts.get(k).copied()) {
                    // EA Trax and Credits are not done yet.
                    log::info!("options category {c} not available");
                }
                return;
            }
            fe::FeAction::Online | fe::FeAction::ProfileManager => {
                log::info!("{a:?} is not available");
                return;
            }
            fe::FeAction::Resume => {
                self.set_paused(false);
                return;
            }
            fe::FeAction::RestartRace => {
                self.fe_row(frontend::Row::Restart);
                return;
            }
            fe::FeAction::Replay => {
                // Replays are not recorded yet.
                return;
            }
            fe::FeAction::QuitToMenu => {
                // "Quit to Main Menu?" OK / Cancel (speed2.exe 0x4c28c5); after a race
                // the longer one (0xe15b558a).
                let message = if self.screen == Screen::Results { 0xe15b_558a } else { 0x30ed_2368 };
                if !self.fe.open_dialog(message, vec![(0x0639, fe::FeAction::ConfirmQuit), (0x5413_89e5, fe::FeAction::CloseDialog)]) {
                    self.fe_row(frontend::Row::QuitToMenu);
                }
                return;
            }
            fe::FeAction::ConfirmQuit => {
                let row = if self.screen == Screen::Results { frontend::Row::Continue } else { frontend::Row::QuitToMenu };
                self.fe_row(row);
                return;
            }
            fe::FeAction::ResultsNext => {
                self.fe.open_post_race();
                return;
            }
            fe::FeAction::PostRestart => {
                let buttons = vec![(0x0639, fe::FeAction::ConfirmRestart), (0x5413_89e5, fe::FeAction::CloseDialog)];
                if !self.fe.open_dialog(0xaa50_aa02, buttons) {
                    self.fe_row(frontend::Row::RaceAgain);
                }
                return;
            }
            fe::FeAction::ConfirmRestart => {
                self.fe_row(frontend::Row::RaceAgain);
                return;
            }
            fe::FeAction::RaceStats => {
                let rows = self.result_rows();
                let circuit = self.session.race().is_some_and(|r| r.def.line.circuit && r.drift.is_none());
                self.fe.open_race_stats(rows, circuit, self.hud.mph, true);
                return;
            }
            fe::FeAction::CloseDialog => return,
            fe::FeAction::PauseOptions => {
                self.fe.open_pause_options_main();
                return;
            }
            fe::FeAction::Back => {
                self.fe_back();
                return;
            }
            fe::FeAction::Quit => {
                log::info!("quit game");
                std::process::exit(0);
            }
            fe::FeAction::CustomizeCar => {
                // The garage is still the port's own.
                frontend::Row::Garage
            }
            fe::FeAction::ListButton(key) => {
                log::info!("options row {key}");
                return;
            }
            fe::FeAction::Mode(m) => {
                // Picking a mode resets the opponents to 3 (5 in URL), speed2.exe 0x4b2c00.
                self.quick.settings[frontend::QuickRace::slot(Some(m))].opponents = if m == roads::Mode::Url { 5 } else { 3 };
                self.open_qr_tracks(m);
                return;
            }
            fe::FeAction::Track(i) => {
                let Some(race) = self.catalog.get(i).cloned() else { return };
                self.quick.mode = race.mode;
                self.quick.race = self.catalog.iter().filter(|r| r.mode == race.mode).position(|r| r.track == race.track && r.reverse == race.reverse).unwrap_or(0);
                let settings = self.quick.settings[frontend::QuickRace::slot(Some(race.mode))];
                self.quick.chosen = Some(race.clone());
                self.fe.open_mode_options(Some(race.mode), Some(race), settings);
                return;
            }
            fe::FeAction::FreeRun => {
                self.quick.chosen = None;
                let settings = self.quick.settings[frontend::QuickRace::slot(None)];
                if !self.fe.open_mode_options(None, None, settings) {
                    self.fe_go(frontend::Row::FreeRoam);
                }
                return;
            }
            fe::FeAction::OptionsDone => {
                let Some(s) = self.fe.mode_settings() else { return };
                let mode = self.quick.chosen.as_ref().map(|r| r.mode);
                self.quick.settings[frontend::QuickRace::slot(mode)] = s;
                // Traffic: None turns it off (the density levels are not decoded yet).
                self.session.traffic_on = s.traffic > 0;
                let Some(mut race) = self.quick.chosen.clone() else {
                    self.open_qr_cars();
                    return;
                };
                if race.reversible {
                    race.reverse = s.reverse;
                }
                self.quick.laps = if race.mode.laps() { s.laps } else { 1 };
                self.quick.opponents = s.opponents as usize;
                log::info!("quick race {} {:?} reverse {} laps {} opponents {} {:?}", race.track, race.mode, race.reverse, self.quick.laps, s.opponents, s);
                self.quick.chosen = Some(race);
                self.open_qr_cars();
                return;
            }
            fe::FeAction::CarDone => {
                // "Transmission:" Manual / Auto when Ask for Transmission is on, except for
                // drags (speed2.exe 0x497d50).
                let ask = self.cfg.options.get("ask_transmission").copied().unwrap_or(1.0) != 0.0;
                let drag = self.quick.chosen.as_ref().is_some_and(|r| r.mode == roads::Mode::Drag);
                if ask && !drag && !self.fe.dialog_open() {
                    let manual = self.cfg.options.get("transmission").copied().unwrap_or(0.0) != 0.0;
                    let buttons = vec![(0x1b3a_2485, fe::FeAction::TransmissionManual), (0x61cb_0720, fe::FeAction::TransmissionAuto)];
                    if self.fe.open_dialog(0x3000_3d27, buttons) {
                        if !manual {
                            self.fe.step(1);
                        }
                        return;
                    }
                }
                self.fe_action(fe::FeAction::CarChosen);
                return;
            }
            fe::FeAction::TransmissionManual | fe::FeAction::TransmissionAuto => {
                let manual = a == fe::FeAction::TransmissionManual;
                self.cfg.options.insert("transmission".into(), manual as u8 as f32);
                self.fe_action(fe::FeAction::CarChosen);
                return;
            }
            fe::FeAction::CarChosen => {
                let Some(name) = self.fe.selected_car().map(str::to_string) else { return };
                self.cfg.car = name.clone();
                self.cfg.save(&self.config_path);
                if self.current_car() != name {
                    self.select_car(name);
                }
                if self.quick.chosen.is_some() {
                    self.prepare_race();
                    return;
                }
                frontend::Row::FreeRoam
            }
        };
        self.fe_go(row);
    }

    /// Opens one of the port's own screens from the title list.
    fn fe_go(&mut self, row: frontend::Row) {
        // Those rows live on the title screen's list.
        if self.fe.kind() != Some(fe::FeKind::MainMenu) {
            self.fe.open_main_menu();
        }
        if let Some(i) = self.rows().iter().position(|r| *r == row) {
            self.menu_action(menu::MenuHit::Activate(i));
        }
    }

    pub fn update(&mut self, dt: f32) {
        // Menu and game music levels differ: re-apply when the frontend comes or goes.
        let title = self.screen == Screen::Title;
        if self.options_title != Some(title) {
            self.options_title = Some(title);
            self.apply_options();
        }
        self.update_movies();
        self.update_fe(dt.clamp(0.0, 0.1));
        self.update_race_hud(dt.clamp(0.0, 0.1));
        let dt = dt.clamp(0.0, 0.1);
        if dt > 0.0 {
            self.fps = self.fps * 0.95 + (1.0 / dt) * 0.05;
        }
        self.pump_loader(Duration::from_millis(12));
        self.try_start_pending_race();
        if self.pending_free_roam && !self.world_loading && self.region == self.cfg.location {
            self.pending_free_roam = false;
            self.start_free_roam();
        }

        let mut triggers = self.input.take_triggers();
        triggers.extend(self.hud.take_triggers());
        for t in triggers {
            match t {
                Action::Camera => self.camera.next_mode(),
                Action::Reset => self.reset_player(),
                Action::Pause => self.back_pressed(),
                Action::ShiftUp if self.screen == Screen::Playing => {
                    if let Some(q) = self.vehicle.shift(true) {
                        let col = match q {
                            Shift::Perfect => [0.2, 1.0, 0.35, 1.0],
                            Shift::Good => [1.0, 1.0, 1.0, 1.0],
                            Shift::Early | Shift::Late => [1.0, 0.55, 0.2, 1.0],
                        };
                        self.banner = Some((q.label().to_string(), col, 1.2));
                        let serial = self.last_shift.map_or(1, |(_, s)| s + 1);
                        self.last_shift = Some((q, serial));
                    }
                }
                Action::ShiftDown => {
                    self.vehicle.shift(false);
                }
                _ => {}
            }
        }

        self.update_groups();
        // Event handling: drift events loosen the grip, drags have a manual gearbox.
        let race_mode = self.session.race().map(|r| r.def.info.mode);
        self.vehicle.drift_handling = race_mode == Some(RaceMode::Drift);
        // Drags always shift by hand; elsewhere the Transmission option decides.
        let drag = race_mode == Some(RaceMode::Drag);
        let manual_choice = self.cfg.options.get("transmission").copied().unwrap_or(0.0) != 0.0;
        self.vehicle.manual = drag || manual_choice;
        self.hud.set_drag(drag);
        self.hud.set_manual(manual_choice && !drag);
        if let Some((_, _, t)) = &mut self.banner {
            *t -= dt;
            if *t <= 0.0 {
                self.banner = None;
            }
        }

        let playing = self.screen == Screen::Playing && !self.paused && self.spawned;
        let mut controls = if playing { self.controls() } else { Controls::default() };
        let countdown = matches!(self.session.race().map(|r| r.phase), Some(RacePhase::Countdown(_)));
        let finished = matches!(self.session.race().map(|r| r.phase), Some(RacePhase::Finished));
        if countdown {
            // Rev on the line, but stay put (held below: the brake pedal would reverse).
            controls = Controls { throttle: controls.throttle, ..Controls::default() };
        } else if finished {
            // Coast to a stop past the line; held at a standstill the brake would reverse.
            let rolling = self.vehicle.vel.dot(self.vehicle.forward()) > 0.5;
            controls = Controls { brake: if rolling { 0.4 } else { 0.0 }, ..Controls::default() };
        } else if playing && race_mode == Some(RaceMode::Drag) {
            if self.cfg.autopilot {
                controls = Controls { throttle: 1.0, ..Controls::default() };
                if self.vehicle.rpm >= 0.9 {
                    self.input.push_trigger(Action::ShiftUp);
                }
            }
            controls.steer = self.drag_steer(controls.steer);
            controls.handbrake = false;
            controls.brake = 0.0;
        } else if playing && race_mode.is_some() && self.cfg.autopilot {
            if let Some(c) = self.autopilot_controls(dt) {
                controls = c;
            }
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
            if countdown {
                self.vehicle.vel.x = 0.0;
                self.vehicle.vel.y = 0.0;
                self.vehicle.yaw_rate = 0.0;
            }
            self.session.animate(&self.physics, dt);
        }
        if simulate {
            self.score_event(dt);
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
                let wall = match (&self.world, v.wall_hits) {
                    (Some(w), n) if n > 0 => format!(" | wall {}", w.owners.get(v.last_wall as usize).map_or("?", |s| s.as_str())),
                    _ => String::new(),
                };
                log::info!(
                    "car pos {:.1?} speed {:.1} | thr {:.1} brk {:.1} steer {:.2} | traffic {} | audio {peak:.2}{race}{wall}",
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
        let cash = c.cash;
        self.prize_paid = Some(prize);
        log::info!("career: finished {place}, prize ${prize}, cash ${cash}");
        // The story movie after winning (speed2.exe 0x513a50).
        if place == 1 {
            if let Some(m) = self.session.race().and_then(|r| r.def.info.movie_after.clone()) {
                self.play_movie(&m);
            }
        }
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

    /// Drag races: hold the lane; a new press left or right moves one lane over.
    fn drag_steer(&mut self, input: f32) -> f32 {
        const LANE: f32 = 3.6;
        let pressed = input.abs() > 0.5 && self.lane_input.abs() <= 0.5;
        self.lane_input = input;
        let mut lane = self.session.player_lane;
        let Some(r) = self.session.race() else { return input };
        let line = &r.def.line;
        let n = line.points.len();
        let i = r.cars[0].index.min(n - 1);
        if pressed {
            // Steering right moves to the lane on the right (offsets grow to the left).
            lane -= input.signum() * LANE;
        }
        let room = (line.half_width[i] - 1.5).max(0.0);
        lane = lane.clamp(-room, room);
        let a = line.points[i];
        let b = line.points[(i + 4).min(n - 1)];
        let d = (b - a).truncate().normalize_or_zero();
        if d == glam::Vec2::ZERO {
            return 0.0;
        }
        let left = glam::Vec2::new(-d.y, d.x);
        let lat = (self.vehicle.pos - a).truncate().dot(left);
        // Aim 12 m ahead at the lane centre.
        let desired = d.y.atan2(d.x) + ((lane - lat) / 12.0).atan();
        let mut e = desired - self.vehicle.yaw;
        e = (e + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        self.session.player_lane = lane;
        (-e * 2.5).clamp(-1.0, 1.0)
    }

    /// Switches on the scenery groups of the race in progress (its barriers), or the
    /// free roam ones (shop markers), when that changes.
    fn update_groups(&mut self) {
        let track = self.session.race().map_or(0, |r| r.def.info.track);
        if self.groups_for == Some(track) {
            return;
        }
        let wanted: Vec<String> = if track == 0 {
            vec!["FREE_ROAM".into()]
        } else {
            vec![format!("BARRIERS_{track}"), format!("PLAYER_BARRIERS_{track}")]
        };
        let on: Vec<bool> = self.group_sets.iter().map(|names| names.iter().any(|n| wanted.contains(n))).collect();
        log::info!("scenery groups {wanted:?}: {} sets on", on.iter().filter(|&&b| b).count());
        self.renderer.set_groups_on(on.clone());
        if let Some(w) = &mut self.world {
            w.set_groups_on(on);
        }
        self.groups_for = Some(track);
    }

    /// The regular paint material closest in colour to `rgb` (the game's paints each have
    /// their own light material).
    fn paint_material(&self, rgb: [f32; 3]) -> u16 {
        let d = |c: [f32; 3]| (0..3).map(|k| (c[k] - rgb[k]).powi(2)).sum::<f32>();
        self.paint_materials.iter().min_by(|a, b| d(a.0).total_cmp(&d(b.0))).map_or(0, |p| p.1)
    }

    /// Like the original's visible section manager: draws only the sections listed for
    /// the drivable section the camera is on, and keeps collision for the sections seen
    /// from wherever a car is (so a car out of view still has its road under it).
    fn update_visibility(&mut self, eye: Vec3) {
        let Some(vis) = &self.visibility else { return };
        let on_for = |listed: &[bool], lists: &[&[u16]]| {
            // Sections no list mentions (the sky) are always there.
            let mut on: Vec<bool> = listed.iter().map(|&l| !l).collect();
            for &s in lists.iter().copied().flatten() {
                if let Some(v) = on.get_mut(s as usize) {
                    *v = true;
                }
            }
            on
        };
        let view = vis.drivable_at(eye.x, eye.y);
        if view != self.vis_view {
            let list = view.and_then(|s| vis.visible_from(s)).unwrap_or(&[]);
            let on = on_for(&self.vis_listed, &[list]);
            self.renderer.set_sections_on(on.clone());
            self.lights = self
                .all_lights
                .iter()
                .filter(|(s, _)| on.get(*s as usize).copied().unwrap_or(true))
                .map(|(_, l)| l.clone())
                .collect();
            self.vis_view = view;
        }
        let mut cars: Vec<u16> = std::iter::once(self.vehicle.pos)
            .chain(self.session.opponents.iter().map(|o| o.vehicle.pos))
            .chain(self.session.traffic.cars.iter().map(|c| c.vehicle.pos))
            .filter_map(|p| vis.drivable_at(p.x, p.y))
            .collect();
        cars.sort_unstable();
        cars.dedup();
        if cars != self.vis_cars {
            if let Some(w) = &mut self.world {
                let lists: Vec<&[u16]> = cars.iter().filter_map(|&s| vis.visible_from(s)).collect();
                w.set_sections_on(on_for(&self.vis_listed, &lists));
                self.vis_cars = cars;
            }
        }
    }

    /// The computer's controls for the player's car (testing aid).
    fn autopilot_controls(&mut self, dt: f32) -> Option<Controls> {
        let r = self.session.race()?;
        let path = crate::play::line_path(&r.def.line, r.cars[0].index, 0.0, 70);
        let ai = self.autopilot.get_or_insert_with(|| crate::ai::AiDriver::new(1.0, 80.0));
        ai.top_speed = self.vehicle.perf.top_speed;
        // Drift grip is low: take the corners slower.
        ai.pace = if self.vehicle.drift_handling { 0.55 } else { 1.0 };
        let c = ai.drive(&self.vehicle, &path, dt);
        self.autopilot_stuck = if self.vehicle.speed().abs() < 1.0 { self.autopilot_stuck + dt } else { 0.0 };
        if self.autopilot_stuck > 3.0 {
            self.autopilot_stuck = 0.0;
            self.reset_player();
        }
        Some(c)
    }

    /// Drift points and blown engines, once per frame during a race.
    fn score_event(&mut self, dt: f32) {
        let angle = self.vehicle.drift_angle();
        let speed = self.vehicle.vel.truncate().length();
        let impact = self.vehicle.impact;
        let blown = self.vehicle.blown;
        let Some(r) = self.session.race_mut() else { return };
        if r.phase != RacePhase::Running {
            return;
        }
        if let Some(d) = &mut r.drift {
            d.score.update(angle, speed, impact, dt);
        }
        if blown && !r.cars[0].retired {
            r.retire(0);
            self.banner = Some(("ENGINE BLOWN".into(), [1.0, 0.25, 0.2, 1.0], 3.0));
        }
    }

    fn race_hud(&self) -> Option<RaceHud> {
        let r = self.session.race()?;
        let p = &r.cars[0];
        let countdown = match r.phase {
            RacePhase::Countdown(t) => Some(t),
            _ => None,
        };
        let banner = self.banner.clone();
        if let Some(d) = &r.drift {
            let standings = r
                .drift_standings()
                .into_iter()
                .map(|(name, score, player)| StandingRow {
                    name,
                    gap: 0.0,
                    player,
                    finished: false,
                    value: Some(crate::drift::format_points(score)),
                })
                .collect();
            return Some(RaceHud {
                standings,
                place: r.place(0),
                cars: r.entrants(),
                lap: r.player_lap(),
                laps: r.laps,
                circuit: true,
                remaining: 0.0,
                time: r.time,
                countdown,
                go_flash: if countdown.is_none() { 1.0 - r.time } else { 0.0 },
                wrong_way: p.wrong_way > 1.0,
                final_lap: r.laps > 1 && p.lap + 1 == r.laps as i32 && p.lap_start + 3.0 > r.time,
                drift: Some(crate::hud::DriftHud {
                    total: d.score.total,
                    pending: d.score.pending(),
                    multiplier: d.score.multiplier,
                    flash: d.score.flash.clone(),
                }),
                banner,
            });
        }
        let speed = self.vehicle.speed().abs().max(10.0);
        let standings = r
            .standings()
            .into_iter()
            .map(|car| {
                let prog = &r.cars[car];
                let name = if car == 0 {
                    "YOU".to_string()
                } else {
                    self.race_names.get(car - 1).cloned().unwrap_or_else(|| self.session.opponents[car - 1].model.clone())
                };
                StandingRow {
                    name,
                    gap: (p.total - prog.total) / speed,
                    player: car == 0,
                    finished: prog.finish_time.is_some(),
                    value: prog.retired.then(|| "DNF".to_string()),
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
            drift: None,
            banner,
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
            Status::Loading(s) if !self.spawned || self.switching => {
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
                manual: self.vehicle.manual,
                heat: self.vehicle.heat,
            };
            self.hud.draw(&mut self.canvas, &t, show_touch, self.cfg.show_fps);
            if !self.hud.original_gauges {
                if let Some(r) = self.race_hud() {
                    self.hud.race_overlay(&mut self.canvas, &r);
                }
                self.draw_minimap();
            }
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
        if self.menu_visible() && !self.fe_active() {
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
        let orbit = matches!(self.screen, Screen::Title | Screen::Garage);
        // The showroom only on the frontend; the pause menu sits over the race.
        let platform = self.fe_active() && self.screen == Screen::Title;
        let (view, eye) = if platform {
            fe_camera()
        } else if orbit && self.spawned {
            // Slow orbit around the parked car behind the title menu; in the garage the
            // car sits on the right of the screen, next to the menu.
            let garage = self.screen == Screen::Garage;
            let t = self.title_time * if garage { 0.25 } else { 0.15 };
            let c = self.vehicle.pos + Vec3::Z * 0.6;
            let dist = if garage { 7.0 } else { 7.5 };
            let eye = c + Vec3::new(t.cos() * dist, t.sin() * dist, if garage { 1.4 } else { 1.8 });
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
            let t = if platform { fe_car_transform(&self.vehicle) } else { self.vehicle.transform() };
            let wheels = wheel_transforms(&self.vehicle, t, &m.wheels);
            let parts = self.cfg.parts_for(car_name(self.player_car.as_deref().unwrap_or("")));
            let glass = TINTS.get((parts.tint as usize).wrapping_sub(1)).map_or([1.0; 3], |t| t.1);
            let paint_material = self.paint_material(self.cfg.paint);
            cars.push(CarDraw { model: m.id, transform: t, wheels, paint: self.cfg.paint, skin: None, brake: self.brake_light, glass, paint_material });
            // The frontend turntable shows the car without light glows (menu capture).
            if !platform {
                push_car_glows(&mut glows, m, t, self.brake_light, right, up);
            }
            if let Some(&(_, color)) = NEONS.get((parts.neon as usize).wrapping_sub(1)) {
                push_neon(&mut glows, m, t, color);
            }
        }
        for o in self.session.opponents.iter().filter(|_| !platform) {
            let Some(m) = self.models.get(&o.model) else { continue };
            let t = o.vehicle.transform();
            let wheels = wheel_transforms(&o.vehicle, t, &m.wheels);
            let paint_material = self.paint_material(o.paint);
            cars.push(CarDraw { model: m.id, transform: t, wheels, paint: o.paint, skin: None, brake: o.brake_light, glass: [1.0; 3], paint_material });
            if near(o.vehicle.pos) {
                push_car_glows(&mut glows, m, t, o.brake_light, right, up);
            }
        }
        for c in self.session.traffic.cars.iter().filter(|_| !platform) {
            let Some(m) = self.models.get(&c.model) else { continue };
            let t = c.vehicle.transform();
            let wheels = wheel_transforms(&c.vehicle, t, &m.wheels);
            let skin = m.skins.get(c.skin).copied();
            let paint_material = self.traffic_material;
            cars.push(CarDraw { model: m.id, transform: t, wheels, paint: [0.5; 3], skin, brake: c.brake_light, glass: [1.0; 3], paint_material });
            if near(c.vehicle.pos) {
                push_car_glows(&mut glows, m, t, c.brake_light, right, up);
            }
        }
        self.update_visibility(eye);
        if !platform {
            push_world_glows(&mut glows, &self.lights, eye, view.row(2).truncate() * -1.0, right, up);
        }
        let mut fe_verts = std::mem::take(&mut self.fe_verts);
        let mut fe_batches = std::mem::take(&mut self.fe_batches);
        fe_verts.clear();
        fe_batches.clear();
        if self.fe_active() {
            self.fe.draw((width, height), &mut fe_verts, &mut fe_batches);
        } else if self.hud.original_gauges && !self.paused {
            self.fe.draw_race_hud((width, height), &mut fe_verts, &mut fe_batches);
        }
        let post = self.visual.frame(&visual::FrameInput {
            maps: self.env_maps.as_deref(),
            in_game: !orbit && self.spawned,
            eye,
            view,
            dt: self.frame_dt,
            boost: self.vehicle.nitro_on,
            bloom_enabled: self.cfg.bloom,
        });
        let data = FrameData {
            view,
            // The frontend camera: 29.2° vertical (P11 = 3.843 in the capture).
            fov_y: if platform { 2.0 * (1.0f32 / 3.843).atan() } else { self.camera.fov_y },
            // The garage menu covers the left of the screen: shift the image so the car
            // sits in the free space on the right without looking at it from an angle.
            lens_shift: if orbit && self.screen == Screen::Garage { 0.62 } else { 0.0 },
            cam_pos: eye,
            cars: &cars,
            glows: &glows,
            glow_texture: bin_hash("HEADLIGHTGLOW"),
            draw_distance: self.cfg.draw_distance,
            vertex_light_scale: self.cfg.vertex_light_scale,
            // Mirror plane at the street under the player's car.
            platform,
            reflect_height: (self.cfg.reflections > 0.01 && self.spawned && !platform)
                .then(|| self.vehicle.pos.z - self.vehicle.ride_height),
            reflect_strength: self.cfg.reflections,
            headlight: self
                .spawned
                .then(|| self.player_model())
                .flatten()
                .map(|m| (headlight_projector(m, self.vehicle.transform()), HEADLIGHT_COLOR)),
            hud: &self.canvas.vertices,
            fe: &fe_verts,
            fe_batches: &fe_batches,
            post,
        };
        self.renderer.render(&self.gpu, target, width, height, &data);
        self.glows = glows;
        self.fe_verts = fe_verts;
        self.fe_batches = fe_batches;
    }

    /// One iteration of the main loop: simulate and present.
    pub fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32();
        self.last = now;
        self.frame_dt = dt;
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
        if let Some(m) = &mut self.movie {
            let f = m.frame();
            self.movie_view.draw(&self.gpu.device, &self.gpu.queue, &view, (w, h), f);
        } else {
            self.render_to(&view, w, h);
        }
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
/// The frontend camera, from an apitrace frame of the PC game's menu: eye at
/// (0, −4, 1.4), looking along +y pitched 0.71° down, vertical field of view 29.2°.
fn fe_camera() -> (Mat4, Vec3) {
    let eye = Vec3::new(0.0, -4.0, 1.4);
    let dir = Vec3::new(0.0, 0.999_922, -0.012_462);
    (Mat4::look_at_rh(eye, eye + dir, Vec3::Z), eye)
}

/// The player's car on the frontend turntable, as in the captured menu frame: 9.43 m
/// in front of the camera, on turntable disc 1 at (0, 5.44), yawed −54.2°.
fn fe_car_transform(v: &Vehicle) -> Mat4 {
    let ground = v.pos.z - v.ride_height;
    let to_origin = Mat4::from_translation(-Vec3::new(v.pos.x, v.pos.y, ground));
    let turn = Mat4::from_rotation_z((-54.2f32).to_radians() - v.yaw);
    // DISC01's top is 0.17 m above the floor.
    Mat4::from_translation(Vec3::new(0.0, 5.44, 0.1697)) * turn * to_origin * v.transform()
}

fn push_world_glows(out: &mut Vec<GpuVertex>, lights: &[ug2_formats::light::WorldLight], eye: Vec3, forward: Vec3, right: Vec3, up: Vec3) {
    // As the PC game draws them (apitrace): a HEADLIGHTGLOW sprite 6 m wide, moved 4 m
    // towards the camera, colour RGB (110, 100, 75) at alpha 220, added to the scene.
    for l in lights {
        let p = Vec3::from(l.position);
        let to = p - eye;
        let d = to.length();
        if d < 4.5 || to.dot(forward) < 0.0 {
            continue;
        }
        let c = p - to / d * 4.0;
        glow_quad(out, c, l.size * 0.5, [110, 100, 75, 220], right, up);
    }
}

/// Underglow: a coloured pool on the road under the car, brightest in the middle.
fn push_neon(out: &mut Vec<GpuVertex>, m: &ModelInfo, t: Mat4, color: [u8; 3]) {
    let z = -m.physics.ride_height + 0.05;
    let center = Vec3::new(m.front * 0.5 - 0.6, 0.0, z);
    let (rx, ry) = (m.front + 0.6, 1.7);
    let uv = [0.5, 0.5];
    let mid = GpuVertex { pos: t.transform_point3(center).into(), normal: [0.0; 3], color: [color[0], color[1], color[2], 150], uv };
    let n = 20;
    let rim = |i: usize| {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let p = center + Vec3::new(a.cos() * rx, a.sin() * ry, 0.0);
        GpuVertex { pos: t.transform_point3(p).into(), normal: [0.0; 3], color: [color[0], color[1], color[2], 0], uv }
    };
    for i in 0..n {
        out.extend_from_slice(&[mid, rim(i), rim(i + 1)]);
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
        // As the PC game (apitrace): about 1.6 m, from RGB (76, 14, 14) at alpha 143 as
        // tail lights to (121, 23, 23) at 227 when braking.
        let k = |lo: f32, hi: f32| (lo + (hi - lo) * b) as u8;
        let color = [k(76.0, 121.0), k(14.0, 23.0), k(14.0, 23.0), k(143.0, 227.0)];
        glow_quad(out, t.transform_point3(*p - Vec3::X * 0.06), 0.8, color, right, up);
    }
}

/// The player's headlight projector: from between the lamps, looking ahead and down,
/// twice as wide as tall like the HEADLIGHTS pattern. Light falls on the road from a
/// metre in front of the car to a hot spot about 12 m ahead.
fn headlight_projector(m: &ModelInfo, t: Mat4) -> Mat4 {
    let lamps = if m.headlights.is_empty() { vec![Vec3::new(m.front, 0.0, 0.0)] } else { m.headlights.clone() };
    let local = lamps.iter().copied().sum::<Vec3>() / lamps.len() as f32;
    let eye = t.transform_point3(local - Vec3::X * 0.3);
    let pitch = HEADLIGHT_PITCH.to_radians();
    let dir = t.transform_vector3(Vec3::new(pitch.cos(), 0.0, -pitch.sin())).normalize_or_zero();
    let proj = Mat4::perspective_rh(HEADLIGHT_FOV_Y.to_radians(), 2.0, 0.3, 120.0);
    proj * Mat4::look_to_rh(eye, dir, Vec3::Z)
}

/// The game's regular paints and the colour each stands for.
const PAINT_MATERIALS: [(&str, [f32; 3]); 9] = [
    ("REGPAINTBLACK", [0.03, 0.03, 0.03]),
    ("REGPAINTWHITE", [0.9, 0.9, 0.9]),
    ("REGPAINTGRAY", [0.45, 0.45, 0.45]),
    ("REGPAINTRED", [0.75, 0.05, 0.05]),
    ("REGPAINTBLUE", [0.05, 0.15, 0.75]),
    ("REGPAINTGREEN", [0.05, 0.55, 0.12]),
    ("REGPAINTYELLOW", [0.9, 0.8, 0.05]),
    ("REGPAINTORANGE", [0.95, 0.4, 0.05]),
    ("REGPAINTPINK", [0.9, 0.35, 0.6]),
];

/// Headlight projector set-up (the game sets these at run time; chosen so the beam
/// pattern covers the lane ahead).
const HEADLIGHT_PITCH: f32 = 12.0;
const HEADLIGHT_FOV_Y: f32 = 30.0;
const HEADLIGHT_COLOR: [f32; 3] = [0.85, 0.83, 0.76];
