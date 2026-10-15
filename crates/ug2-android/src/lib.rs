//! Android entry point: a NativeActivity driven from Rust. The APK's only Java code is
//! `MainActivity` (android/java), which hides the system bars.
//!
//! Game files are read from the app's external files directory:
//! `/sdcard/Android/data/com.adrianstroescu.ug2/files/NFSU2/` (CARS, GLOBAL, TRACKS, ...).

#![cfg(target_os = "android")]

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::time::Duration;

use android_activity::input::{Axis, InputEvent, KeyAction, Keycode, MotionAction, MotionEvent};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent, WindowManagerFlags};
use raw_window_handle::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};
use ug2_engine::input::{PadAxis, PadButton, RawInput, TouchPhase};
use ndk::audio::{
    AudioCallbackResult, AudioDirection, AudioFormat, AudioPerformanceMode, AudioSharingMode, AudioStream,
    AudioStreamBuilder, AudioUsage,
};
use ug2_engine::render::Gpu;
use ug2_engine::sound::{Sound, RATE};
use ug2_engine::{wgpu, Game};

const SOURCE_TOUCHSCREEN: u32 = 0x0000_1002;
const SOURCE_JOYSTICK: u32 = 0x0100_0010;

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("ug2"),
    );
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    app.set_window_flags(WindowManagerFlags::KEEP_SCREEN_ON | WindowManagerFlags::FULLSCREEN, WindowManagerFlags::empty());

    let files = app
        .external_data_path()
        .unwrap_or_else(|| PathBuf::from("/sdcard/Android/data/com.adrianstroescu.ug2/files"));
    let data_dir = files.join("NFSU2");
    let _ = std::fs::create_dir_all(&data_dir);
    log::info!("game data directory: {}", data_dir.display());

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN, ..Default::default() });
    let mut game: Option<Game> = None;
    // Keeps the ANativeWindow referenced for as long as the surface uses it.
    let mut window = None;
    let mut quit = false;
    let mut focused = true;
    let mut audio: Option<AudioStream> = None;

    while !quit {
        let active = focused && game.as_ref().is_some_and(|g| g.has_surface());
        let timeout = if active { Some(Duration::ZERO) } else { Some(Duration::from_millis(100)) };
        app.poll_events(timeout, |event| {
            let PollEvent::Main(main) = event else { return };
            match main {
                MainEvent::InitWindow { .. } => {
                    let Some(w) = app.native_window() else { return };
                    let (width, height) = (w.width().max(1) as u32, w.height().max(1) as u32);
                    let handle = AndroidNdkWindowHandle::new(NonNull::new(w.ptr().as_ptr() as *mut c_void).unwrap());
                    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_display_handle: RawDisplayHandle::Android(AndroidDisplayHandle::new()),
                        raw_window_handle: RawWindowHandle::AndroidNdk(handle),
                    };
                    let surface = match unsafe { instance.create_surface_unsafe(target) } {
                        Ok(s) => s,
                        Err(e) => {
                            log::error!("create surface: {e}");
                            return;
                        }
                    };
                    window = Some(w);
                    if game.is_none() {
                        let gpu = match Gpu::new(instance.clone(), Some(&surface)) {
                            Ok(g) => g,
                            Err(e) => {
                                log::error!("no usable GPU: {e:#}");
                                quit = true;
                                return;
                            }
                        };
                        let format = gpu.surface_config(&surface, width, height).format;
                        let dp = app.config().density().unwrap_or(480) as f32 / 160.0;
                        let mut g = Game::new(gpu, format, &data_dir, &files, dp);
                        g.touch_ui = true;
                        audio = open_audio(g.sound());
                        game = Some(g);
                    }
                    if let Some(g) = game.as_mut() {
                        g.attach_surface(surface, width, height);
                    }
                }
                MainEvent::TerminateWindow { .. } => {
                    if let Some(g) = game.as_mut() {
                        g.detach_surface();
                    }
                    window = None;
                }
                MainEvent::WindowResized { .. } | MainEvent::ContentRectChanged { .. } => {
                    if let (Some(g), Some(w)) = (game.as_mut(), app.native_window()) {
                        g.resize(w.width().max(1) as u32, w.height().max(1) as u32);
                    }
                }
                MainEvent::ConfigChanged { .. } => {
                    if let Some(g) = game.as_mut() {
                        g.set_dp(app.config().density().unwrap_or(480) as f32 / 160.0);
                    }
                }
                MainEvent::GainedFocus => {
                    focused = true;
                    if let Some(a) = &audio {
                        let _ = a.request_start();
                    }
                }
                MainEvent::LostFocus => {
                    focused = false;
                    if let Some(a) = &audio {
                        let _ = a.request_pause();
                    }
                }
                MainEvent::Destroy => quit = true,
                _ => {}
            }
        });

        if let Ok(mut iter) = app.input_events_iter() {
            loop {
                let read = iter.next(|event| match game.as_mut() {
                    Some(g) => handle_input(g, event),
                    None => InputStatus::Unhandled,
                });
                if !read {
                    break;
                }
            }
        }

        if let Some(g) = game.as_mut() {
            if focused && g.has_surface() {
                g.frame();
            }
        }
    }
    drop(audio);
    drop(window);
    log::info!("exiting");
}

/// Low-latency AAudio output; the callback pulls stereo f32 frames from the mixer.
fn open_audio(sound: Sound) -> Option<AudioStream> {
    let mixer = sound.mixer();
    let stream = AudioStreamBuilder::new()
        .and_then(|b| {
            b.direction(AudioDirection::Output)
                .channel_count(2)
                .sample_rate(RATE as i32)
                .format(AudioFormat::PCM_Float)
                .performance_mode(AudioPerformanceMode::LowLatency)
                .sharing_mode(AudioSharingMode::Shared)
                .usage(AudioUsage::Game)
                .data_callback(Box::new(move |_stream, data, frames| {
                    // SAFETY: AAudio hands us `frames` stereo f32 frames to fill.
                    let out = unsafe { std::slice::from_raw_parts_mut(data as *mut f32, frames.max(0) as usize * 2) };
                    // A panic must not unwind into AAudio (that aborts the app): play silence.
                    let ok = mixer.lock().ok().and_then(|mut m| {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.render(out))).ok()
                    });
                    if ok.is_none() {
                        out.fill(0.0);
                    }
                    AudioCallbackResult::Continue
                }))
                .error_callback(Box::new(|_stream, e| log::warn!("audio stream error: {e:?}")))
                .open_stream()
        })
        .map_err(|e| log::error!("audio: {e:?}"))
        .ok()?;
    if let Err(e) = stream.request_start() {
        log::error!("audio start: {e:?}");
    }
    log::info!("audio: {} Hz, {} frames per burst", stream.sample_rate(), stream.frames_per_burst());
    Some(stream)
}

fn handle_input(game: &mut Game, event: &InputEvent) -> InputStatus {
    match event {
        InputEvent::MotionEvent(m) => {
            let source: u32 = m.source().into();
            if source & SOURCE_TOUCHSCREEN == SOURCE_TOUCHSCREEN {
                touch(game, m);
                InputStatus::Handled
            } else if source & SOURCE_JOYSTICK == SOURCE_JOYSTICK && m.action() == MotionAction::Move {
                let p = m.pointer_at_index(0);
                let axes = [
                    (Axis::X, PadAxis::LeftX),
                    (Axis::Y, PadAxis::LeftY),
                    (Axis::Z, PadAxis::RightX),
                    (Axis::Rz, PadAxis::RightY),
                    (Axis::HatX, PadAxis::HatX),
                    (Axis::HatY, PadAxis::HatY),
                ];
                for (a, pa) in axes {
                    game.handle_input(RawInput::PadAxis { axis: pa, value: p.axis_value(a) });
                }
                // Controllers report triggers on either the trigger or the pedal axes.
                let l2 = p.axis_value(Axis::Ltrigger).max(p.axis_value(Axis::Brake));
                let r2 = p.axis_value(Axis::Rtrigger).max(p.axis_value(Axis::Gas));
                game.handle_input(RawInput::PadAxis { axis: PadAxis::L2, value: l2 });
                game.handle_input(RawInput::PadAxis { axis: PadAxis::R2, value: r2 });
                InputStatus::Handled
            } else {
                InputStatus::Unhandled
            }
        }
        InputEvent::KeyEvent(k) => {
            let pressed = match k.action() {
                KeyAction::Down => true,
                KeyAction::Up => false,
                _ => return InputStatus::Unhandled,
            };
            if pressed && k.repeat_count() > 0 {
                return InputStatus::Handled;
            }
            let code = k.key_code();
            if let Some(button) = pad_button(code) {
                game.handle_input(RawInput::PadButton { button, pressed });
                InputStatus::Handled
            } else if let Some(name) = key_name(code) {
                game.handle_input(RawInput::Key { code: name.to_string(), pressed });
                InputStatus::Handled
            } else {
                // Volume keys etc. keep their system behaviour.
                InputStatus::Unhandled
            }
        }
        _ => InputStatus::Unhandled,
    }
}

fn touch(game: &mut Game, m: &MotionEvent) {
    let one = |game: &mut Game, idx: usize, phase: TouchPhase| {
        let p = m.pointer_at_index(idx);
        game.handle_input(RawInput::Touch { id: p.pointer_id() as u64, phase, x: p.x(), y: p.y() });
    };
    match m.action() {
        MotionAction::Down | MotionAction::PointerDown => one(game, m.pointer_index(), TouchPhase::Down),
        MotionAction::Up | MotionAction::PointerUp => one(game, m.pointer_index(), TouchPhase::Up),
        MotionAction::Move => {
            for i in 0..m.pointer_count() {
                one(game, i, TouchPhase::Move);
            }
        }
        MotionAction::Cancel => {
            for i in 0..m.pointer_count() {
                one(game, i, TouchPhase::Cancel);
            }
        }
        _ => {}
    }
}

fn pad_button(k: Keycode) -> Option<PadButton> {
    Some(match k {
        Keycode::ButtonA => PadButton::A,
        Keycode::ButtonB => PadButton::B,
        Keycode::ButtonX => PadButton::X,
        Keycode::ButtonY => PadButton::Y,
        Keycode::ButtonL1 => PadButton::L1,
        Keycode::ButtonR1 => PadButton::R1,
        Keycode::ButtonL2 => PadButton::L2,
        Keycode::ButtonR2 => PadButton::R2,
        Keycode::ButtonThumbl => PadButton::L3,
        Keycode::ButtonThumbr => PadButton::R3,
        Keycode::ButtonStart => PadButton::Start,
        Keycode::ButtonSelect => PadButton::Select,
        Keycode::DpadUp => PadButton::DpadUp,
        Keycode::DpadDown => PadButton::DpadDown,
        Keycode::DpadLeft => PadButton::DpadLeft,
        Keycode::DpadRight => PadButton::DpadRight,
        _ => return None,
    })
}

/// Android keycodes to the W3C `KeyboardEvent.code` names used by controls.cfg.
fn key_name(k: Keycode) -> Option<&'static str> {
    Some(match k {
        Keycode::Back | Keycode::Escape => "Escape",
        Keycode::W => "KeyW",
        Keycode::A => "KeyA",
        Keycode::S => "KeyS",
        Keycode::D => "KeyD",
        Keycode::C => "KeyC",
        Keycode::B => "KeyB",
        Keycode::R => "KeyR",
        Keycode::N => "KeyN",
        Keycode::P => "KeyP",
        Keycode::Space => "Space",
        Keycode::ShiftLeft => "ShiftLeft",
        Keycode::ShiftRight => "ShiftRight",
        Keycode::Enter => "Enter",
        _ => return None,
    })
}
