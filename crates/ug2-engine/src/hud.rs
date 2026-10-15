//! On-screen touch controls and HUD drawing.
//!
//! Layout is in density-independent units (dp) so it has the same physical size on a
//! 1440p S23 Ultra and on a desktop window. Every finger is tracked separately, so
//! steering, gas and nitro can be held at the same time.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::{vec2, Vec2};

use crate::config::SteerMode;
use crate::font;
use crate::input::{Action, Controls, TouchPhase};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct HudVertex {
    pub pos: [f32; 2],
    pub color: [f32; 4],
}

/// Immediate-mode triangle builder in pixel coordinates.
#[derive(Default)]
pub struct Canvas {
    pub vertices: Vec<HudVertex>,
}

pub type Rgba = [f32; 4];

impl Canvas {
    pub fn clear(&mut self) {
        self.vertices.clear();
    }

    fn tri(&mut self, a: Vec2, b: Vec2, c: Vec2, col: Rgba) {
        for p in [a, b, c] {
            self.vertices.push(HudVertex { pos: p.into(), color: col });
        }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, col: Rgba) {
        let (a, b, c, d) = (vec2(x, y), vec2(x + w, y), vec2(x + w, y + h), vec2(x, y + h));
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }

    pub fn circle(&mut self, c: Vec2, r: f32, col: Rgba) {
        self.sector(c, r, 0.0, std::f32::consts::TAU, col);
    }

    pub fn sector(&mut self, c: Vec2, r: f32, from: f32, to: f32, col: Rgba) {
        let n = ((to - from).abs() / std::f32::consts::TAU * 40.0).ceil().max(2.0) as usize;
        for i in 0..n {
            let a0 = from + (to - from) * i as f32 / n as f32;
            let a1 = from + (to - from) * (i + 1) as f32 / n as f32;
            self.tri(c, c + vec2(a0.cos(), a0.sin()) * r, c + vec2(a1.cos(), a1.sin()) * r, col);
        }
    }

    pub fn ring(&mut self, c: Vec2, r: f32, thickness: f32, col: Rgba) {
        self.arc(c, r, thickness, 0.0, std::f32::consts::TAU, col);
    }

    pub fn arc(&mut self, c: Vec2, r: f32, thickness: f32, from: f32, to: f32, col: Rgba) {
        let n = ((to - from).abs() / std::f32::consts::TAU * 48.0).ceil().max(2.0) as usize;
        let (ri, ro) = (r - thickness * 0.5, r + thickness * 0.5);
        for i in 0..n {
            let a0 = from + (to - from) * i as f32 / n as f32;
            let a1 = from + (to - from) * (i + 1) as f32 / n as f32;
            let (d0, d1) = (vec2(a0.cos(), a0.sin()), vec2(a1.cos(), a1.sin()));
            self.tri(c + d0 * ri, c + d0 * ro, c + d1 * ro, col);
            self.tri(c + d0 * ri, c + d1 * ro, c + d1 * ri, col);
        }
    }

    pub fn triangle(&mut self, a: Vec2, b: Vec2, c: Vec2, col: Rgba) {
        self.tri(a, b, c, col);
    }

    pub fn line(&mut self, a: Vec2, b: Vec2, thickness: f32, col: Rgba) {
        let n = (b - a).perp().normalize_or_zero() * thickness * 0.5;
        self.tri(a - n, b - n, b + n, col);
        self.tri(a - n, b + n, a + n, col);
    }

    /// `count` chevrons centred on `c` pointing along the unit vector `dir` (« », a stack
    /// of ^). Each chevron is two parallelograms sharing an edge, so translucent colours
    /// don't double up where the arms meet.
    pub fn chevrons(&mut self, c: Vec2, size: f32, dir: Vec2, count: usize, thickness: f32, col: Rgba) {
        let t = dir * thickness;
        let h = dir.perp() * 0.5 * size;
        for i in 0..count {
            let off = (i as f32 - (count - 1) as f32 * 0.5) * 0.44;
            let tip = c + dir * ((off + 0.25 - thickness / size * 0.5) * size);
            let back = tip - dir * 0.5 * size;
            self.tri(back - h, back - h + t, tip + t, col);
            self.tri(back - h, tip + t, tip, col);
            self.tri(tip, tip + t, back + h + t, col);
            self.tri(tip, back + h + t, back + h, col);
        }
    }

    /// Corner centres of a rounded rectangle and the angle each quarter arc starts at.
    fn corners(c: Vec2, half: Vec2, r: f32) -> [(Vec2, f32); 4] {
        let a = half - Vec2::splat(r);
        let q = std::f32::consts::FRAC_PI_2;
        [(c + vec2(-a.x, -a.y), 2.0 * q), (c + vec2(a.x, -a.y), 3.0 * q), (c + a, 0.0), (c + vec2(-a.x, a.y), q)]
    }

    /// Filled rounded rectangle from non-overlapping pieces (safe for translucent colours).
    pub fn rounded_rect(&mut self, c: Vec2, half: Vec2, r: f32, col: Rgba) {
        let (min, size) = (c - half, half * 2.0);
        self.rect(min.x + r, min.y, size.x - 2.0 * r, size.y, col);
        self.rect(min.x, min.y + r, r, size.y - 2.0 * r, col);
        self.rect(min.x + size.x - r, min.y + r, r, size.y - 2.0 * r, col);
        for (cc, a) in Self::corners(c, half, r) {
            self.sector(cc, r, a, a + std::f32::consts::FRAC_PI_2, col);
        }
    }

    /// Outline of a rounded rectangle, `thickness` centred on the edge (like `ring`).
    pub fn rounded_rect_outline(&mut self, c: Vec2, half: Vec2, r: f32, thickness: f32, col: Rgba) {
        let (min, max, t) = (c - half, c + half, thickness);
        let (w, h) = (max.x - min.x - 2.0 * r, max.y - min.y - 2.0 * r);
        self.rect(min.x + r, min.y - t * 0.5, w, t, col);
        self.rect(min.x + r, max.y - t * 0.5, w, t, col);
        self.rect(min.x - t * 0.5, min.y + r, t, h, col);
        self.rect(max.x - t * 0.5, min.y + r, t, h, col);
        for (cc, a) in Self::corners(c, half, r) {
            self.arc(cc, r, t, a, a + std::f32::consts::FRAC_PI_2, col);
        }
    }

    /// Side view of a hanging pedal: slanted pad, arm and pivot. `size` ≈ icon width.
    pub fn pedal_icon(&mut self, c: Vec2, size: f32, col: Rgba) {
        let s = size;
        let d = vec2(0.524, -0.852); // pad direction, toe up and forward
        let n = vec2(-0.852, -0.524); // from the pad's back towards the pivot
        let mid = c + vec2(0.14, 0.06) * s;
        let pad_t = 0.28 * s;
        self.line(mid - d * 0.44 * s, mid + d * 0.44 * s, pad_t, col);
        let arm0 = mid + n * pad_t * 0.5;
        let pivot = arm0 + n * 0.36 * s;
        self.line(arm0, pivot, 0.12 * s, col);
        self.circle(pivot, 0.11 * s, col);
    }

    pub fn text_width(text: &str, size: f32) -> f32 {
        let n = text.chars().count() as f32;
        (n * font::ADVANCE - 1.0) * size
    }

    /// `size` is the size of one font cell in pixels; `align` 0 = left, 0.5 = center, 1 = right.
    pub fn text(&mut self, x: f32, y: f32, size: f32, align: f32, text: &str, col: Rgba) {
        let x0 = x - Self::text_width(text, size) * align;
        for (i, ch) in text.chars().enumerate() {
            let Some(rows) = font::glyph(ch) else { continue };
            let gx = x0 + i as f32 * font::ADVANCE * size;
            for (ry, bits) in rows.iter().enumerate() {
                let mut run_start: Option<usize> = None;
                for cx in 0..=5 {
                    let on = cx < 5 && bits & (0x10 >> cx) != 0;
                    match (on, run_start) {
                        (true, None) => run_start = Some(cx),
                        (false, Some(s)) => {
                            self.rect(gx + s as f32 * size, y + ry as f32 * size, (cx - s) as f32 * size, size, col);
                            run_start = None;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Gas,
    Brake,
    Nitro,
    Handbrake,
    LookBack,
    SteerLeft,
    SteerRight,
    Camera,
    Reset,
    Pause,
}

impl Button {
    fn label(self) -> &'static str {
        match self {
            Button::Gas => "GAS",
            Button::Brake => "BRAKE",
            Button::Nitro => "NOS",
            Button::Handbrake => "E-BRAKE",
            Button::LookBack => "LOOK",
            Button::SteerLeft => "<",
            Button::SteerRight => ">",
            Button::Camera => "CAM",
            Button::Reset => "RESET",
            Button::Pause => "II",
        }
    }

    /// Buttons a finger can slide between without lifting (pedal cluster, steering).
    fn slide_group(self) -> u8 {
        match self {
            Button::Gas | Button::Brake | Button::Nitro | Button::Handbrake | Button::LookBack => 1,
            Button::SteerLeft | Button::SteerRight => 2,
            _ => 0,
        }
    }

    fn trigger(self) -> Option<Action> {
        match self {
            Button::Camera => Some(Action::Camera),
            Button::Reset => Some(Action::Reset),
            Button::Pause => Some(Action::Pause),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Circle(f32),
    /// Rounded rectangle (GAS/BRAKE pedals), half extents.
    Pedal(Vec2),
}

struct ButtonDef {
    id: Button,
    center: Vec2,
    shape: Shape,
}

impl ButtonDef {
    fn circle(id: Button, center: Vec2, radius: f32) -> Self {
        Self { id, center, shape: Shape::Circle(radius) }
    }

    fn pedal(id: Button, center: Vec2, half: Vec2) -> Self {
        Self { id, center, shape: Shape::Pedal(half) }
    }

    /// Distance from the centre, scaled so the edge of the (slightly enlarged) touch
    /// area is 1.0.
    fn reach(&self, p: Vec2) -> f32 {
        let d = p - self.center;
        match self.shape {
            Shape::Circle(r) => d.length() / (r * 1.2),
            Shape::Pedal(half) => (d.abs() / (half * 1.15)).max_element(),
        }
    }

    /// Radius of the largest circle that fits, for sizing labels and icons.
    fn inner_radius(&self) -> f32 {
        match self.shape {
            Shape::Circle(r) => r,
            Shape::Pedal(half) => half.min_element(),
        }
    }
}

enum Owner {
    Button(Button),
    Joystick,
    None,
}

struct Pointer {
    owner: Owner,
    pos: Vec2,
}

/// Snapshot of what the driver is doing, for drawing the speedometer.
#[derive(Clone, Copy, Default)]
pub struct Telemetry {
    pub speed_kmh: f32,
    pub gear: i32,
    pub rpm: f32,
    pub nitro: f32,
    pub fps: f32,
    pub draw_calls: u32,
}

pub struct Hud {
    size: Vec2,
    dp: f32,
    pub steer_mode: SteerMode,
    pub opacity: f32,
    buttons: Vec<ButtonDef>,
    pointers: HashMap<u64, Pointer>,
    joy_origin: Vec2,
    joy_knob: Vec2,
    joy_radius: f32,
    triggers: Vec<Action>,
    /// True once any touch happened; desktop players never see the touch overlay.
    pub touch_seen: bool,
}

const ACCENT: Rgba = [0.25, 0.85, 1.0, 1.0];
const WHITE: Rgba = [1.0, 1.0, 1.0, 1.0];

impl Hud {
    pub fn new(steer_mode: SteerMode, opacity: f32) -> Self {
        Self {
            size: Vec2::ONE,
            dp: 1.0,
            steer_mode,
            opacity,
            buttons: Vec::new(),
            pointers: HashMap::new(),
            joy_origin: Vec2::ZERO,
            joy_knob: Vec2::ZERO,
            joy_radius: 70.0,
            triggers: Vec::new(),
            touch_seen: false,
        }
    }

    /// `dp` = physical pixels per density-independent pixel (Android density / 160).
    pub fn layout(&mut self, width: f32, height: f32, dp: f32) {
        self.size = vec2(width, height);
        self.dp = dp;
        let s = dp;
        let (w, h) = (width, height);
        // Pedals bottom right with their bottoms aligned (tall GAS outside, BRAKE inside),
        // like most mobile racers; nitro and handbrake sit above them.
        let mut b = vec![
            ButtonDef::pedal(Button::Gas, vec2(w - 74.0 * s, h - 98.0 * s), vec2(40.0, 70.0) * s),
            ButtonDef::pedal(Button::Brake, vec2(w - 176.0 * s, h - 74.0 * s), vec2(36.0, 46.0) * s),
            ButtonDef::circle(Button::Nitro, vec2(w - 74.0 * s, h - 232.0 * s), 34.0 * s),
            ButtonDef::circle(Button::Handbrake, vec2(w - 176.0 * s, h - 178.0 * s), 34.0 * s),
            ButtonDef::circle(Button::LookBack, vec2(w - 272.0 * s, h - 150.0 * s), 28.0 * s),
            ButtonDef::circle(Button::Pause, vec2(44.0 * s, 44.0 * s), 24.0 * s),
            ButtonDef::circle(Button::Camera, vec2(w - 48.0 * s, 44.0 * s), 26.0 * s),
            ButtonDef::circle(Button::Reset, vec2(w - 116.0 * s, 44.0 * s), 26.0 * s),
        ];
        if self.steer_mode == SteerMode::Buttons {
            // Side by side, bottom left; the touch zones (1.2 x radius) meet in the middle
            // so a thumb can rock between them.
            b.push(ButtonDef::circle(Button::SteerLeft, vec2(78.0 * s, h - 84.0 * s), 50.0 * s));
            b.push(ButtonDef::circle(Button::SteerRight, vec2(198.0 * s, h - 84.0 * s), 50.0 * s));
        }
        self.buttons = b;
        self.joy_radius = 70.0 * s;
    }

    fn hit(&self, p: Vec2, group: Option<u8>) -> Option<Button> {
        self.buttons
            .iter()
            .filter(|b| group.is_none_or(|g| b.id.slide_group() == g))
            .map(|b| (b, b.reach(p)))
            .filter(|(_, d)| *d <= 1.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(b, _)| b.id)
    }

    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        self.touch_seen = true;
        let p = vec2(x, y);
        match phase {
            TouchPhase::Down => {
                let owner = if let Some(b) = self.hit(p, None) {
                    if let Some(a) = b.trigger() {
                        self.triggers.push(a);
                    }
                    Owner::Button(b)
                } else if self.steer_mode == SteerMode::Joystick
                    && p.x < self.size.x * 0.45
                    && p.y > 90.0 * self.dp
                    && !self.pointers.values().any(|q| matches!(q.owner, Owner::Joystick))
                {
                    self.joy_origin = p;
                    self.joy_knob = p;
                    Owner::Joystick
                } else {
                    Owner::None
                };
                self.pointers.insert(id, Pointer { owner, pos: p });
            }
            TouchPhase::Move => {
                let mut new_owner = None;
                if let Some(ptr) = self.pointers.get(&id) {
                    match ptr.owner {
                        Owner::Joystick => {
                            let d = p - self.joy_origin;
                            let max = self.joy_radius;
                            // Drag the base along when the finger goes far past the rim.
                            if d.length() > max * 1.6 {
                                self.joy_origin = p - d.normalize() * max * 1.6;
                            }
                            self.joy_knob = self.joy_origin + (p - self.joy_origin).clamp_length_max(max);
                        }
                        Owner::Button(b) if b.slide_group() != 0 => {
                            if let Some(nb) = self.hit(p, Some(b.slide_group())) {
                                new_owner = Some(Owner::Button(nb));
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(ptr) = self.pointers.get_mut(&id) {
                    ptr.pos = p;
                    if let Some(o) = new_owner {
                        ptr.owner = o;
                    }
                }
            }
            TouchPhase::Up | TouchPhase::Cancel => {
                self.pointers.remove(&id);
            }
        }
    }

    pub fn take_triggers(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.triggers)
    }

    fn held(&self, b: Button) -> bool {
        self.pointers.values().any(|p| matches!(p.owner, Owner::Button(x) if x == b))
    }

    fn joystick_active(&self) -> bool {
        self.pointers.values().any(|p| matches!(p.owner, Owner::Joystick))
    }

    pub fn controls(&self) -> Controls {
        let mut steer = 0.0;
        if self.joystick_active() {
            let dx = (self.joy_knob.x - self.joy_origin.x) / self.joy_radius;
            // Small dead zone, then a gentle curve for fine control at speed.
            let a = ((dx.abs() - 0.06).max(0.0) / 0.94).min(1.0);
            steer = a.powf(1.4) * dx.signum();
        }
        if self.held(Button::SteerLeft) {
            steer -= 1.0;
        }
        if self.held(Button::SteerRight) {
            steer += 1.0;
        }
        Controls {
            steer,
            throttle: self.held(Button::Gas) as u8 as f32,
            brake: self.held(Button::Brake) as u8 as f32,
            handbrake: self.held(Button::Handbrake),
            nitro: self.held(Button::Nitro),
            look_back: self.held(Button::LookBack),
        }
    }

    pub fn draw(&self, c: &mut Canvas, t: &Telemetry, show_touch: bool, show_fps: bool) {
        let s = self.dp;
        let o = self.opacity;
        let (w, h) = (self.size.x, self.size.y);

        if show_touch {
            for b in &self.buttons {
                let held = self.held(b.id);
                let fill = if held { [ACCENT[0], ACCENT[1], ACCENT[2], 0.45 * o] } else { [0.0, 0.0, 0.0, 0.30 * o] };
                let edge = [1.0, 1.0, 1.0, if held { 0.95 } else { 0.7 * o }];
                match b.shape {
                    Shape::Circle(r) => {
                        c.circle(b.center, r, fill);
                        c.ring(b.center, r, 2.5 * s, edge);
                    }
                    Shape::Pedal(half) => {
                        c.rounded_rect(b.center, half, 14.0 * s, fill);
                        c.rounded_rect_outline(b.center, half, 14.0 * s, 2.5 * s, edge);
                    }
                }
                let icon = [1.0, 1.0, 1.0, 0.9 * o.max(0.6)];
                let r = b.inner_radius();
                match b.id {
                    Button::SteerLeft => c.chevrons(b.center, r * 0.62, vec2(-1.0, 0.0), 2, 8.0 * s, icon),
                    Button::SteerRight => c.chevrons(b.center, r * 0.62, vec2(1.0, 0.0), 2, 8.0 * s, icon),
                    Button::Gas => c.chevrons(b.center, r * 0.9, vec2(0.0, -1.0), 3, 7.0 * s, icon),
                    Button::Brake => c.pedal_icon(b.center, r * 1.25, icon),
                    _ => {
                        let label = b.id.label();
                        let size = (r * 0.5 / (font::ADVANCE * label.len().max(3) as f32 * 0.5)).min(3.2 * s);
                        c.text(b.center.x, b.center.y - font::HEIGHT * size * 0.5, size, 0.5, label, icon);
                    }
                }
            }
            if self.steer_mode == SteerMode::Joystick {
                if self.joystick_active() {
                    c.circle(self.joy_origin, self.joy_radius, [0.0, 0.0, 0.0, 0.25 * o]);
                    c.ring(self.joy_origin, self.joy_radius, 2.5 * s, [1.0, 1.0, 1.0, 0.6 * o]);
                    c.circle(self.joy_knob, 30.0 * s, [ACCENT[0], ACCENT[1], ACCENT[2], 0.6]);
                } else {
                    let hint = vec2(140.0 * s, h - 120.0 * s);
                    c.ring(hint, self.joy_radius, 2.0 * s, [1.0, 1.0, 1.0, 0.25 * o]);
                    c.text(hint.x, hint.y - 7.0 * s, 2.0 * s, 0.5, "STEER", [1.0, 1.0, 1.0, 0.35 * o]);
                }
            }
        }

        // Speedometer: speed, gear, rev bar and nitro bar, bottom centre.
        let cx = w * 0.5;
        let base_y = h - 36.0 * s;
        let speed = format!("{:3}", t.speed_kmh.round().max(0.0) as i32);
        c.text(cx, base_y - 44.0 * s, 6.0 * s, 0.5, &speed, WHITE);
        c.text(cx + 62.0 * s, base_y - 14.0 * s, 1.8 * s, 0.0, "KM/H", [1.0, 1.0, 1.0, 0.8]);
        let gear = match t.gear {
            -1 => "R".to_string(),
            0 => "N".to_string(),
            g => g.to_string(),
        };
        c.text(cx - 76.0 * s, base_y - 34.0 * s, 4.0 * s, 0.5, &gear, ACCENT);
        let bar_w = 180.0 * s;
        let bx = cx - bar_w * 0.5;
        c.rect(bx, base_y + 2.0 * s, bar_w, 6.0 * s, [0.0, 0.0, 0.0, 0.5]);
        let rpm = t.rpm.clamp(0.0, 1.0);
        let rpm_col = if rpm > 0.9 { [1.0, 0.25, 0.2, 1.0] } else { ACCENT };
        c.rect(bx, base_y + 2.0 * s, bar_w * rpm, 6.0 * s, rpm_col);
        c.rect(bx, base_y + 11.0 * s, bar_w, 4.0 * s, [0.0, 0.0, 0.0, 0.5]);
        c.rect(bx, base_y + 11.0 * s, bar_w * t.nitro.clamp(0.0, 1.0), 4.0 * s, [0.3, 1.0, 0.4, 1.0]);

        if show_fps {
            let text = format!("{:.0} FPS  {} DC", t.fps, t.draw_calls);
            c.text(80.0 * s, 36.0 * s, 1.6 * s, 0.0, &text, [1.0, 1.0, 1.0, 0.7]);
        }
    }

    /// Centered message box (loading, errors, pause).
    pub fn message(&self, c: &mut Canvas, lines: &[&str]) {
        let s = self.dp;
        let longest = lines.iter().map(|l| Canvas::text_width(l, 1.0)).fold(1.0, f32::max);
        let size = (2.6 * s).min(self.size.x * 0.92 / longest);
        let line_h = 12.0 * size;
        let width = lines.iter().map(|l| Canvas::text_width(l, size)).fold(0.0, f32::max) + 40.0 * s;
        let height = lines.len() as f32 * line_h + 24.0 * s;
        let (x, y) = ((self.size.x - width) * 0.5, (self.size.y - height) * 0.5);
        c.rect(x, y, width, height, [0.0, 0.0, 0.0, 0.7]);
        c.rect(x, y, width, 3.0 * s, ACCENT);
        for (i, l) in lines.iter().enumerate() {
            c.text(self.size.x * 0.5, y + 16.0 * s + i as f32 * line_h, size, 0.5, l, WHITE);
        }
    }
}
