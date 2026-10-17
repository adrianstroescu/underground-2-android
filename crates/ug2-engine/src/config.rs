//! User settings, stored as `key = value` lines next to the game data.

use std::collections::HashMap;
use std::path::Path;

use ug2_formats::car::PartSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SteerMode {
    /// Floating joystick on the left half of the screen.
    Joystick,
    /// Two arrow buttons.
    Buttons,
}

#[derive(Clone, Debug)]
pub struct GameConfig {
    pub car: String,
    pub location: String,
    pub paint: [f32; 3],
    pub steer_mode: SteerMode,
    pub hud_scale: f32,
    pub hud_opacity: f32,
    pub show_fps: bool,
    /// Testing aid: the computer drives the player's car in races (`autopilot = 1`).
    pub autopilot: bool,
    pub msaa: u32,
    /// Fraction of native resolution to render at (S23 Ultra native is 3088x1440).
    pub render_scale: f32,
    pub draw_distance: f32,
    /// Multiplier for baked track vertex lighting.
    pub vertex_light_scale: f32,
    pub traffic: bool,
    /// Road reflection strength 0..1 (0 turns the extra pass off).
    pub reflections: f32,
    /// Glow around bright lights (post-processing).
    pub bloom: bool,
    /// Visual parts per car.
    pub parts: HashMap<String, PartSet>,
    /// The original game's options (Options screens), `opt.<key> = value`.
    pub options: std::collections::BTreeMap<String, f32>,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            car: "350Z".into(),
            location: "L4RA".into(),
            paint: [0.85, 0.32, 0.05],
            steer_mode: SteerMode::Buttons,
            hud_scale: 1.0,
            hud_opacity: 0.55,
            show_fps: true,
            autopilot: false,
            msaa: 4,
            render_scale: 1.0,
            draw_distance: 650.0,
            vertex_light_scale: 2.0,
            traffic: true,
            reflections: 1.0,
            bloom: true,
            parts: HashMap::new(),
            options: Default::default(),
        }
    }
}

impl GameConfig {
    pub fn load_or_create(path: &Path) -> Self {
        let mut cfg = Self::default();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                cfg.apply(&text);
                // Files written before version 2 defaulted to the joystick; the touch
                // buttons are the default now.
                let version = text
                    .lines()
                    .find_map(|l| l.trim_start().strip_prefix("version"))
                    .and_then(|v| v.trim_start_matches([' ', '=']).split_whitespace().next()?.parse::<u32>().ok());
                if version.is_none() {
                    cfg.steer_mode = SteerMode::Buttons;
                }
                // Before version 3 reflections were a 0..1 fresnel blend (default 0.55); now
                // 1 is the original game's road reflection.
                if version.unwrap_or(0) < 3 {
                    if cfg.reflections > 0.01 {
                        cfg.reflections = 1.0;
                    }
                    cfg.save(path);
                }
            }
            Err(_) => {
                if let Err(e) = std::fs::write(path, cfg.to_text()) {
                    log::warn!("cannot write {}: {e}", path.display());
                }
            }
        }
        cfg
    }

    pub fn save(&self, path: &Path) {
        if let Err(e) = std::fs::write(path, self.to_text()) {
            log::warn!("cannot write {}: {e}", path.display());
        }
    }

    pub fn apply(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let f = || v.parse::<f32>().ok();
            match k {
                "car" => self.car = v.to_ascii_uppercase(),
                "location" => self.location = v.to_ascii_uppercase(),
                "paint" => {
                    let c: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if c.len() == 3 {
                        self.paint = [c[0], c[1], c[2]];
                    }
                }
                "steer_mode" => {
                    self.steer_mode = if v.eq_ignore_ascii_case("joystick") { SteerMode::Joystick } else { SteerMode::Buttons }
                }
                "hud_scale" => self.hud_scale = f().unwrap_or(self.hud_scale).clamp(0.5, 2.0),
                "hud_opacity" => self.hud_opacity = f().unwrap_or(self.hud_opacity).clamp(0.1, 1.0),
                "show_fps" => self.show_fps = v == "1" || v.eq_ignore_ascii_case("true"),
                "autopilot" => self.autopilot = v == "1" || v.eq_ignore_ascii_case("true"),
                "msaa" => self.msaa = if v.trim() == "1" { 1 } else { 4 },
                "render_scale" => self.render_scale = f().unwrap_or(self.render_scale).clamp(0.3, 1.0),
                "draw_distance" => self.draw_distance = f().unwrap_or(self.draw_distance).clamp(100.0, 3000.0),
                "vertex_light_scale" => self.vertex_light_scale = f().unwrap_or(self.vertex_light_scale),
                "version" => {}
                "reflections" => self.reflections = f().unwrap_or(self.reflections).clamp(0.0, 1.0),
                "bloom" => self.bloom = v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("on"),
                _ if k.starts_with("parts.") => {
                    let n: Vec<u16> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if n.len() >= 4 {
                        let at = |i: usize| n.get(i).copied().unwrap_or(0);
                        let p = PartSet {
                            kit: at(0) as u8,
                            widebody: at(1) as u8,
                            hood: at(2) as u8,
                            lights: at(3) as u8,
                            rim: at(4),
                            spoiler: at(5) as u8,
                            spoiler_cf: at(6) != 0,
                            neon: at(7) as u8,
                            tint: at(8) as u8,
                        };
                        self.parts.insert(k["parts.".len()..].to_string(), p);
                    }
                }
                _ if k.starts_with("opt.") => {
                    if let Some(x) = f() {
                        self.options.insert(k["opt.".len()..].to_string(), x);
                    }
                }
                "traffic" => self.traffic = v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("on"),
                _ => log::warn!("unknown setting {k}"),
            }
        }
    }

    pub fn parts_for(&self, car: &str) -> PartSet {
        self.parts.get(car).copied().unwrap_or_default()
    }

    pub fn to_text(&self) -> String {
        let mut parts = String::new();
        for (k, v) in &self.options {
            parts.push_str(&format!("opt.{k} = {v}
"));
        }
        for (car, p) in &self.parts {
            if !p.is_stock() {
                parts.push_str(&format!(
                    "parts.{car} = {}, {}, {}, {}, {}, {}, {}, {}, {}\n",
                    p.kit, p.widebody, p.hood, p.lights, p.rim, p.spoiler, p.spoiler_cf as u8, p.neon, p.tint
                ));
            }
        }
        format!(
            "# Underground 2 Android settings\n\
             version = 3\n\
             car = {}\n\
             location = {}\n\
             paint = {}, {}, {}\n\
             steer_mode = {}   # buttons | joystick\n\
             hud_scale = {}\n\
             hud_opacity = {}\n\
             show_fps = {}\n\
             msaa = {}   # 1 or 4\n\
             render_scale = {}\n\
             draw_distance = {}\n\
             vertex_light_scale = {}\n\
             traffic = {}\n\
             reflections = {}   # road reflections 0..1: 1 = as the original, 0 = off\n\
             bloom = {}\n{}{}",
            self.car,
            self.location,
            self.paint[0],
            self.paint[1],
            self.paint[2],
            if self.steer_mode == SteerMode::Buttons { "buttons" } else { "joystick" },
            self.hud_scale,
            self.hud_opacity,
            self.show_fps as u8,
            self.msaa,
            self.render_scale,
            self.draw_distance,
            self.vertex_light_scale,
            self.traffic as u8,
            self.reflections,
            self.bloom as u8,
            if self.autopilot { "autopilot = 1\n" } else { "" },
            parts,
        )
    }
}
