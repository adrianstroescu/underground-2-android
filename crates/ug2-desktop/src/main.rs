//! Desktop host. Keyboard drives, the left mouse button acts as a finger so the touch
//! HUD can be tested without a phone. `--shot` renders headless screenshots.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use glam::Vec3;
use ug2_engine::input::{RawInput, TouchPhase};
use ug2_engine::render::Gpu;
use ug2_engine::{wgpu, Game};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

struct Args {
    data: PathBuf,
    config: PathBuf,
    shot: Option<PathBuf>,
    drive: f32,
    size: (u32, u32),
    pos: Option<(Vec3, f32)>,
    touch: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        data: if Path::new("../TRACKS").exists() { PathBuf::from("..") } else { PathBuf::from(".") },
        config: PathBuf::from("ug2-config"),
        shot: None,
        drive: 0.0,
        size: (1544, 720),
        pos: None,
        touch: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_default();
        match k.as_str() {
            "--data" => a.data = v().into(),
            "--config" => a.config = v().into(),
            "--shot" => a.shot = Some(v().into()),
            "--drive" => a.drive = v().parse().unwrap_or(0.0),
            "--touch" => a.touch = true,
            "--size" => {
                let s = v();
                if let Some((w, h)) = s.split_once('x') {
                    a.size = (w.parse().unwrap_or(1544), h.parse().unwrap_or(720));
                }
            }
            "--pos" => {
                let p: Vec<f32> = v().split(',').filter_map(|x| x.parse().ok()).collect();
                if p.len() == 4 {
                    a.pos = Some((Vec3::new(p[0], p[1], p[2]), p[3].to_radians()));
                }
            }
            _ => eprintln!("unknown argument {k}"),
        }
    }
    a
}

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
        ..Default::default()
    })
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .init();
    let args = parse_args();
    log::info!("game data: {}", args.data.display());
    if args.shot.is_some() {
        return headless(&args);
    }
    let event_loop = EventLoop::new()?;
    let mut app = App { args, game: None, window: None, cursor: (0.0, 0.0), mouse_down: false };
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// Loads the world, optionally drives for a while, writes a PNG.
fn headless(args: &Args) -> Result<()> {
    let gpu = Gpu::new(instance(), None)?;
    let mut game = Game::new(gpu, wgpu::TextureFormat::Rgba8Unorm, &args.data, &args.config, 1.0);
    game.touch_ui = args.touch;
    game.resize(args.size.0, args.size.1);
    let t0 = std::time::Instant::now();
    while game.is_loading() && t0.elapsed().as_secs() < 300 {
        game.update(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    log::info!("{} after {:.1?}", game.status_text(), t0.elapsed());
    if let Some((p, yaw)) = args.pos {
        game.debug_teleport(p, yaw);
    }
    let frames = (args.drive * 60.0) as u32;
    for i in 0..frames.max(30) {
        if i < frames {
            game.debug_controls(1.0, if i > frames / 2 { 0.4 } else { 0.0 });
        } else {
            game.debug_controls(0.0, 0.0);
        }
        game.update(1.0 / 60.0);
    }
    let (pos, yaw, speed) = game.vehicle_state();
    log::info!("car at {pos:.1?} yaw {:.0} speed {:.1} km/h", yaw.to_degrees(), speed * 3.6);
    let rgba = game.screenshot(args.size.0, args.size.1);
    log::info!("draw calls: {}", game.draw_calls());
    let path = args.shot.as_ref().unwrap();
    let f = std::io::BufWriter::new(std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?);
    let mut enc = png::Encoder::new(f, args.size.0, args.size.1);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba)?;
    log::info!("wrote {}", path.display());
    Ok(())
}

struct App {
    args: Args,
    game: Option<Game>,
    window: Option<Arc<Window>>,
    cursor: (f32, f32),
    mouse_down: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Need for Speed Underground 2 - Rust")
            .with_inner_size(winit::dpi::PhysicalSize::new(self.args.size.0, self.args.size.1));
        let window = Arc::new(el.create_window(attrs).expect("create window"));
        let instance = instance();
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let gpu = Gpu::new(instance, Some(&surface)).expect("GPU init");
        let size = window.inner_size();
        let format = gpu.surface_config(&surface, size.width, size.height).format;
        let mut game = Game::new(gpu, format, &self.args.data, &self.args.config, window.scale_factor() as f32);
        game.touch_ui = self.args.touch;
        game.attach_surface(surface, size.width, size.height);
        self.game = Some(game);
        self.window = Some(window);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(game) = self.game.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => game.resize(s.width, s.height),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => game.set_dp(scale_factor as f32),
            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key: PhysicalKey::Code(code), state, repeat, .. }, ..
            } => {
                if code == KeyCode::F1 && state == ElementState::Pressed {
                    game.touch_ui = !game.touch_ui;
                }
                if !repeat {
                    game.handle_input(RawInput::Key { code: format!("{code:?}"), pressed: state == ElementState::Pressed });
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                if self.mouse_down {
                    game.handle_input(RawInput::Touch { id: 0, phase: TouchPhase::Move, x: self.cursor.0, y: self.cursor.1 });
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.mouse_down = state == ElementState::Pressed;
                let phase = if self.mouse_down { TouchPhase::Down } else { TouchPhase::Up };
                game.handle_input(RawInput::Touch { id: 0, phase, x: self.cursor.0, y: self.cursor.1 });
            }
            WindowEvent::Touch(t) => {
                let phase = match t.phase {
                    winit::event::TouchPhase::Started => TouchPhase::Down,
                    winit::event::TouchPhase::Moved => TouchPhase::Move,
                    winit::event::TouchPhase::Ended => TouchPhase::Up,
                    winit::event::TouchPhase::Cancelled => TouchPhase::Cancel,
                };
                game.handle_input(RawInput::Touch { id: t.id + 1, phase, x: t.location.x as f32, y: t.location.y as f32 });
            }
            WindowEvent::RedrawRequested => {
                game.frame();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
