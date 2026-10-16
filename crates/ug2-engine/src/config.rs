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
    pub msaa: u32,
    /// Fraction of native resolution to render at (S23 Ultra native is 3088x1440).
    pub render_scale: f32,
    pub draw_distance: f32,
    /// Multiplier for baked track vertex lighting.
    pub vertex_light_scale: f32,
    pub traffic: bool,
    /// Visual parts per car.
    pub parts: HashMap<String, PartSet>,
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
            msaa: 4,
            render_scale: 1.0,
            draw_distance: 650.0,
            vertex_light_scale: 2.0,
            traffic: true,
            parts: HashMap::new(),
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
                if !text.lines().any(|l| l.trim_start().starts_with("version")) {
                    cfg.steer_mode = SteerMode::Buttons;
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
                "msaa" => self.msaa = if v.trim() == "1" { 1 } else { 4 },
                "render_scale" => self.render_scale = f().unwrap_or(self.render_scale).clamp(0.3, 1.0),
                "draw_distance" => self.draw_distance = f().unwrap_or(self.draw_distance).clamp(100.0, 3000.0),
                "vertex_light_scale" => self.vertex_light_scale = f().unwrap_or(self.vertex_light_scale),
                "version" => {}
                _ if k.starts_with("parts.") => {
                    let n: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if n.len() == 4 {
                        let p = PartSet { kit: n[0], widebody: n[1], hood: n[2], lights: n[3] };
                        self.parts.insert(k["parts.".len()..].to_string(), p);
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
        for (car, p) in &self.parts {
            if !p.is_stock() {
                parts.push_str(&format!("parts.{car} = {}, {}, {}, {}\n", p.kit, p.widebody, p.hood, p.lights));
            }
        }
        format!(
            "# Underground 2 Android settings\n\
             version = 2\n\
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
             traffic = {}\n{}",
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
            parts,
        )
    }
}
