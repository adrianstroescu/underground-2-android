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
    ShiftUp,
    ShiftDown,
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
            Button::ShiftUp => "+",
            Button::ShiftDown => "-",
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
            Button::ShiftUp => Some(Action::ShiftUp),
            Button::ShiftDown => Some(Action::ShiftDown),
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

/// What the minimap shows, in world XY metres.
pub struct Minimap<'a> {
    pub center: Vec2,
    /// Car yaw (radians, 0 = +X).
    pub heading: f32,
    /// World metres from the centre to the rim.
    pub range: f32,
    pub roads: &'a [Vec<Vec2>],
    pub route: &'a [Vec2],
    pub cars: &'a [(Vec2, Rgba)],
}

/// What the race overlay shows.
/// One line of the standings list.
#[derive(Clone, Debug, Default)]
pub struct StandingRow {
    pub name: String,
    /// Seconds relative to the player (negative = ahead).
    pub gap: f32,
    pub player: bool,
    pub finished: bool,
    /// Shown instead of the gap (drift scores, "DNF").
    pub value: Option<String>,
}

/// Drift event scoring for the HUD.
#[derive(Clone, Debug, Default)]
pub struct DriftHud {
    pub total: u32,
    /// The slide in progress, multiplier included.
    pub pending: u32,
    pub multiplier: u32,
    pub flash: Option<(String, f32)>,
}

pub fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, x) if x != 11 => "ST",
        (2, x) if x != 12 => "ND",
        (3, x) if x != 13 => "RD",
        _ => "TH",
    };
    format!("{n}{suffix}")
}

#[derive(Clone, Debug, Default)]
pub struct RaceHud {
    pub standings: Vec<StandingRow>,
    pub place: usize,
    pub cars: usize,
    pub lap: u32,
    pub laps: u32,
    pub circuit: bool,
    /// Metres to the finish (sprints).
    pub remaining: f32,
    pub time: f32,
    pub countdown: Option<f32>,
    /// Seconds left to show "GO!" after the start.
    pub go_flash: f32,
    pub wrong_way: bool,
    pub final_lap: bool,
    pub drift: Option<DriftHud>,
    /// Big centred message (shift timing, blown engine), its colour and time left.
    pub banner: Option<(String, Rgba, f32)>,
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
    /// Manual gearbox (drag): the gauge shows the shift light and engine heat.
    pub manual: bool,
    pub heat: f32,
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
    /// Drag race layout: shift buttons instead of brake and handbrake.
    drag: bool,
    /// Speedometer in miles per hour (Options > Gameplay > Speedometer Units).
    pub mph: bool,
    /// Manual gearbox outside drags: shift buttons beside the pedals.
    manual: bool,
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
            drag: false,
            mph: true,
            manual: false,
        }
    }

    /// Switches to the drag race controls (or back).
    pub fn set_manual(&mut self, manual: bool) {
        if self.manual != manual {
            self.manual = manual;
            self.layout(self.size.x, self.size.y, self.dp);
        }
    }

    pub fn set_drag(&mut self, drag: bool) {
        if self.drag != drag {
            self.drag = drag;
            self.layout(self.size.x, self.size.y, self.dp);
        }
    }

    /// `dp` = physical pixels per density-independent pixel (Android density / 160).
    pub fn layout(&mut self, width: f32, height: f32, dp: f32) {
        self.size = vec2(width, height);
        self.dp = dp;
        let s = dp;
        let (w, h) = (width, height);
        // Layout after the target design (docs/hud-target.png): pedals bottom right with
        // NOS above the brake pedal and the rev gauge above the gas; handbrake on the left
        // above the steering buttons; camera, look-back and reset bottom centre.
        let mut b = vec![
            ButtonDef::pedal(Button::Gas, vec2(w - 74.0 * s, h - 98.0 * s), vec2(40.0, 70.0) * s),
            ButtonDef::circle(Button::Nitro, vec2(w - 190.0 * s, h - 194.0 * s), 30.0 * s),
            ButtonDef::circle(Button::Pause, vec2(36.0 * s, 36.0 * s), 22.0 * s),
            ButtonDef::circle(Button::Camera, vec2(w * 0.5 - 64.0 * s, h - 30.0 * s), 22.0 * s),
            ButtonDef::circle(Button::LookBack, vec2(w * 0.5, h - 30.0 * s), 22.0 * s),
            ButtonDef::circle(Button::Reset, vec2(w * 0.5 + 64.0 * s, h - 30.0 * s), 22.0 * s),
        ];
        if self.drag {
            // Drag: shift up where the brake pedal sits, shift down beside it.
            b.push(ButtonDef::circle(Button::ShiftUp, vec2(w - 182.0 * s, h - 82.0 * s), 40.0 * s));
            b.push(ButtonDef::circle(Button::ShiftDown, vec2(w - 276.0 * s, h - 60.0 * s), 26.0 * s));
        } else {
            b.push(ButtonDef::pedal(Button::Brake, vec2(w - 176.0 * s, h - 74.0 * s), vec2(36.0, 46.0) * s));
            b.push(ButtonDef::circle(Button::Handbrake, vec2(64.0 * s, h - 192.0 * s), 28.0 * s));
            if self.manual {
                // Manual gearbox (Options > Player > Transmission): up above the gas, down
                // beside it.
                b.push(ButtonDef::circle(Button::ShiftUp, vec2(w - 74.0 * s, h - 236.0 * s), 28.0 * s));
                b.push(ButtonDef::circle(Button::ShiftDown, vec2(w - 120.0 * s, h - 296.0 * s), 22.0 * s));
            }
        }
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
                // NOS is blue and the handbrake red, as in the target design.
                let tint = match b.id {
                    Button::Nitro => Some([0.15, 0.55, 1.0]),
                    Button::Handbrake => Some([1.0, 0.2, 0.15]),
                    _ => None,
                };
                let fill = match (held, tint) {
                    (true, Some(t)) => [t[0], t[1], t[2], 0.6 * o.max(0.6)],
                    (true, None) => [ACCENT[0], ACCENT[1], ACCENT[2], 0.45 * o],
                    (false, Some(t)) => [t[0] * 0.3, t[1] * 0.3, t[2] * 0.3, 0.45 * o],
                    (false, None) => [0.0, 0.0, 0.0, 0.30 * o],
                };
                let edge = match tint {
                    Some(t) => [t[0], t[1], t[2], if held { 1.0 } else { 0.85 }],
                    None => [1.0, 1.0, 1.0, if held { 0.95 } else { 0.7 * o }],
                };
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
                    Button::ShiftUp | Button::ShiftDown => {
                        let size = r * 0.09;
                        c.text(b.center.x, b.center.y - font::HEIGHT * size * 0.62, size, 0.5, b.id.label(), icon);
                        if b.id == Button::ShiftUp {
                            c.text(b.center.x, b.center.y + r * 0.42, 1.2 * s, 0.5, "SHIFT", icon);
                        }
                    }
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

        self.gauge(c, t, vec2(w - 86.0 * s, h - 238.0 * s), 50.0 * s);

        if show_fps {
            let text = format!("{:.0} FPS  {} DC", t.fps, t.draw_calls);
            c.text(w * 0.5, 4.0 * s, 1.3 * s, 0.5, &text, [1.0, 1.0, 1.0, 0.55]);
        }
    }

    /// Analog rev counter with the speed and gear in the middle and a nitro arc inside.
    fn gauge(&self, c: &mut Canvas, t: &Telemetry, center: Vec2, r: f32) {
        let s = self.dp;
        let (a0, a1) = (std::f32::consts::PI * 0.75, std::f32::consts::PI * 2.25);
        let at = |f: f32| a0 + (a1 - a0) * f.clamp(0.0, 1.0);
        let dir = |a: f32| vec2(a.cos(), a.sin());
        c.circle(center, r, [0.0, 0.02, 0.05, 0.6]);
        c.arc(center, r - 3.0 * s, 3.0 * s, a0, at(0.85), [ACCENT[0], ACCENT[1], ACCENT[2], 0.9]);
        c.arc(center, r - 3.0 * s, 3.0 * s, at(0.85), a1, [1.0, 0.2, 0.15, 1.0]);
        for k in 0..=8 {
            let f = k as f32 / 8.0;
            let d = dir(at(f));
            c.line(center + d * (r - 10.0 * s), center + d * (r - 5.0 * s), 2.0 * s, WHITE);
            let label = k.to_string();
            let p = center + d * (r - 17.0 * s);
            c.text(p.x, p.y - 3.5 * s, 1.0 * s, 0.5, &label, [1.0, 1.0, 1.0, 0.75]);
        }
        // Nitro left: inner arc.
        c.arc(center, r - 24.0 * s, 2.5 * s, a0, a1, [1.0, 1.0, 1.0, 0.12]);
        if t.nitro > 0.0 {
            c.arc(center, r - 24.0 * s, 2.5 * s, a0, at(t.nitro), [0.2, 0.6, 1.0, 0.95]);
        }
        let rpm = t.rpm.clamp(0.0, 1.0);
        if t.manual {
            // Shift light: the perfect window on the dial, lit when the needle is in it.
            let win = crate::vehicle::PERFECT_SHIFT;
            let lit = win.contains(&rpm);
            let green = [0.2, 1.0, 0.35, if lit { 1.0 } else { 0.45 }];
            c.arc(center, r - 3.0 * s, 6.0 * s, at(win.start), at(win.end), green);
            if lit {
                c.text(center.x, center.y - r - 18.0 * s, 2.4 * s, 0.5, "SHIFT", green);
            }
            if t.heat > 0.02 {
                // Engine heat bar under the dial.
                let (bw, bh) = (r * 1.6, 5.0 * s);
                let (bx, by) = (center.x - bw * 0.5, center.y + r + 6.0 * s);
                c.rect(bx, by, bw, bh, [0.0, 0.0, 0.0, 0.5]);
                c.rect(bx, by, bw * t.heat.min(1.0), bh, [1.0, 0.35 - 0.25 * t.heat, 0.1, 0.95]);
            }
        }
        let d = dir(at(rpm));
        c.line(center - d * 6.0 * s, center + d * (r - 8.0 * s), 3.0 * s, [1.0, 0.35, 0.15, 1.0]);
        c.circle(center, 4.0 * s, [1.0, 0.35, 0.15, 1.0]);
        // Gear above the hub, speed below it, inside the dial.
        let gear = match t.gear {
            -1 => "R".to_string(),
            0 => "N".to_string(),
            g => g.to_string(),
        };
        c.text(center.x, center.y - 24.0 * s, 2.2 * s, 0.5, &gear, ACCENT);
        let (value, unit) = if self.mph { (t.speed_kmh / 1.609_344, "MPH") } else { (t.speed_kmh, "KM/H") };
        let speed = format!("{}", value.round().max(0.0) as i32);
        c.text(center.x, center.y + 9.0 * s, 2.6 * s, 0.5, &speed, WHITE);
        c.text(center.x, center.y + 30.0 * s, 1.1 * s, 0.5, unit, [1.0, 1.0, 1.0, 0.75]);
    }

    /// Circular heading-up minimap, top left: roads, the race route, other racers.
    pub fn minimap(&self, c: &mut Canvas, m: &Minimap) {
        let s = self.dp;
        let r = 48.0 * s;
        let center = vec2(16.0 * s + r, 66.0 * s + r);
        let scale = r / m.range;
        let (sin, cos) = (m.heading.sin(), m.heading.cos());
        // World XY -> minimap pixels, with the car's heading pointing up.
        let to_map = |p: Vec2| {
            let d = p - m.center;
            let fwd = d.x * cos + d.y * sin;
            let left = -d.x * sin + d.y * cos;
            center + vec2(-left, -fwd) * scale
        };
        let inside = |p: Vec2| p.distance(center) < r - 2.0 * s;
        c.circle(center, r, [0.02, 0.04, 0.08, 0.72]);
        let seg = |c: &mut Canvas, a: Vec2, b: Vec2, w: f32, col: Rgba| {
            let (pa, pb) = (to_map(a), to_map(b));
            if inside(pa) && inside(pb) {
                c.line(pa, pb, w, col);
            }
        };
        for road in m.roads {
            for w in road.windows(2) {
                seg(c, w[0], w[1], 3.0 * s, [0.55, 0.62, 0.72, 0.55]);
            }
        }
        if m.route.len() > 1 {
            for w in m.route.windows(2) {
                seg(c, w[0], w[1], 4.5 * s, [1.0, 0.7, 0.1, 0.95]);
            }
        }
        for &(p, col) in m.cars {
            let q = to_map(p);
            if inside(q) {
                c.circle(q, 4.0 * s, col);
            }
        }
        c.ring(center, r, 2.5 * s, [ACCENT[0], ACCENT[1], ACCENT[2], 0.8]);
        // North marker on the rim.
        let north = to_map(m.center + vec2(0.0, 1.0) * m.range * 10.0);
        let dir = (north - center).normalize_or_zero();
        c.text(center.x + dir.x * (r - 9.0 * s), center.y + dir.y * (r - 9.0 * s) - 5.0 * s, 1.4 * s, 0.5, "N", WHITE);
        // The player: an arrow pointing up.
        let a = 9.0 * s;
        c.triangle(center + vec2(0.0, -a), center + vec2(a * 0.7, a * 0.7), center + vec2(-a * 0.7, a * 0.7), [0.3, 1.0, 0.45, 1.0]);
    }

    /// Race information: place, lap and time at the top, countdown and warnings centred.
    pub fn race_overlay(&self, c: &mut Canvas, r: &RaceHud) {
        let s = self.dp;
        let w = self.size.x;
        // Lap (or distance left) and time, top left next to the pause button.
        let (x, y) = (66.0 * s, 12.0 * s);
        c.rect(x, y, 128.0 * s, 44.0 * s, [0.0, 0.0, 0.0, 0.5]);
        c.rect(x, y, 3.0 * s, 44.0 * s, ACCENT);
        let first = if r.circuit {
            format!("LAP  {}/{}", r.lap, r.laps)
        } else {
            format!("LEFT {:.1} KM", (r.remaining / 1000.0).max(0.0))
        };
        c.text(x + 10.0 * s, y + 6.0 * s, 1.9 * s, 0.0, &first, ACCENT);
        let second = match &r.drift {
            Some(d) => format!("SCORE {}", crate::drift::format_points(d.total)),
            None => format!("TIME {}", crate::race::format_time(r.time)),
        };
        c.text(x + 10.0 * s, y + 25.0 * s, 1.9 * s, 0.0, &second, WHITE);
        if let Some(d) = &r.drift {
            // The slide in progress, big at the top centre, and the last result under it.
            if d.pending > 0 {
                let text = crate::drift::format_points(d.pending);
                c.text(w * 0.5, 54.0 * s, 4.2 * s, 0.5, &text, WHITE);
                let mult = format!("DRIFT  X{}", d.multiplier);
                c.text(w * 0.5, 96.0 * s, 2.0 * s, 0.5, &mult, ACCENT);
            } else if d.multiplier > 1 {
                let mult = format!("X{}", d.multiplier);
                c.text(w * 0.5, 96.0 * s, 2.0 * s, 0.5, &mult, [ACCENT[0], ACCENT[1], ACCENT[2], 0.6]);
            }
            if let Some((text, t)) = &d.flash {
                let col = if text == "CRASH" { [1.0, 0.25, 0.2, t.min(1.0)] } else { [1.0, 0.85, 0.2, t.min(1.0)] };
                c.text(w * 0.5, 118.0 * s, 2.6 * s, 0.5, text, col);
            }
        }
        if let Some((text, col, t)) = &r.banner {
            c.text(w * 0.5, self.size.y * 0.24, 3.2 * s, 0.5, text, [col[0], col[1], col[2], t.min(1.0)]);
        }

        // Place and standings, top right.
        let right = w - 12.0 * s;
        let place = ordinal(r.place);
        let size = 4.0 * s;
        let total = format!("/{}", r.cars);
        let pw = Canvas::text_width(&place, size) + Canvas::text_width(&total, 2.2 * s) + 6.0 * s;
        c.text(right - pw, 8.0 * s, size, 0.0, &place, WHITE);
        c.text(right - Canvas::text_width(&total, 2.2 * s), 18.0 * s, 2.2 * s, 0.0, &total, [1.0, 1.0, 1.0, 0.8]);
        let row_h = 13.0 * s;
        let list_w = 150.0 * s;
        for (i, st) in r.standings.iter().enumerate().take(6) {
            let ry = 42.0 * s + i as f32 * row_h;
            let bg = if st.player { [0.1, 0.35, 0.9, 0.75] } else { [0.0, 0.0, 0.0, 0.45] };
            c.rect(right - list_w, ry, list_w, row_h - 2.0 * s, bg);
            let gap = if let Some(v) = &st.value {
                v.clone()
            } else if st.player {
                "---".to_string()
            } else if st.finished {
                "FIN".to_string()
            } else {
                format!("{:+.1}", st.gap)
            };
            c.text(right - 5.0 * s, ry + 2.0 * s, 1.3 * s, 1.0, &gap, [1.0, 1.0, 1.0, 0.85]);
            // The name gets what the value leaves, cut short if needed.
            let room = list_w - Canvas::text_width(&gap, 1.3 * s) - 14.0 * s;
            let mut name = format!("{} {}", i + 1, st.name);
            while name.len() > 3 && Canvas::text_width(&name, 1.3 * s) > room {
                name.pop();
            }
            c.text(right - list_w + 5.0 * s, ry + 2.0 * s, 1.3 * s, 0.0, name.trim_end(), WHITE);
        }
        if let Some(t) = r.countdown {
            let text = if t > 3.0 { "" } else if t > 2.0 { "3" } else if t > 1.0 { "2" } else if t > 0.0 { "1" } else { "GO!" };
            c.text(w * 0.5, self.size.y * 0.32, 12.0 * s, 0.5, text, ACCENT);
        } else if r.go_flash > 0.0 {
            c.text(w * 0.5, self.size.y * 0.32, 12.0 * s, 0.5, "GO!", [ACCENT[0], ACCENT[1], ACCENT[2], r.go_flash.min(1.0)]);
        }
        if r.wrong_way {
            c.text(w * 0.5, self.size.y * 0.42, 4.0 * s, 0.5, "WRONG WAY", [1.0, 0.25, 0.2, 1.0]);
        }
        if r.final_lap {
            c.text(w * 0.5, 60.0 * s, 2.2 * s, 0.5, "FINAL LAP", [1.0, 0.8, 0.2, 1.0]);
        }
    }

    /// Centered message box (loading, errors).
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
