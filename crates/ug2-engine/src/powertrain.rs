//! The player car's engine, clutch and gearbox, re-implemented from speed2.exe's
//! PhysicsMover (vtable 0x7a25e0, update 0x5b72d0). Units are the game's: kN·m, t·m²,
//! rad/s; rpm = w · 9.549304. Data from `ug2_formats::carphysics::CarPhysics`.
//!
//! * Engine (vtable 0x7a235c): torque 0x594300, dw/dt = (T + ext) / I 0x5a1290,
//!   post-step 0x5aa760 (clamp idle..max, rev limiter, drag heat 0x5a11e0).
//! * DriveTrain (vtable 0x7a2508): clutch rate 0x5949b0, clutch state machine 0x5a1400,
//!   slipping clutch torque 0x594cb0, torque to the wheels 0x594a30, wheel-side engine
//!   speed 0x5945f0, locked clutch 0x594da0.
//! * Gearbox 0x5aa960 with shift points 0x5ac7d0 / 0x594670 and offsets 0x594710; the
//!   shift rpm (car+0x440) is smoothed once a frame by 0x5a5540.
//! * Input stage 0x5abbf0: throttle ≤ 0.15 is none; in gear under 3000 rpm at least 0.5.

use ug2_formats::carphysics::CarPhysics;

pub const RPM_K: f32 = 9.549_304;
pub const RAD_K: f32 = 0.104_719_67;
/// Longest physics substep (0x7fc070): n = trunc(dt / it) + 1 substeps a frame.
pub const MAX_SUBSTEP: f32 = 0.0044;
/// Wheel inertia, t·m² (0x7fc6f0, set by 0x5ab990).
pub const WHEEL_INERTIA: f32 = 0.01;

/// Clutch rate per state (0x7fc298).
const CLUTCH_RATE: [f32; 4] = [-3.0, 0.0, 1.5, 0.0];
/// Slipping clutch speed difference clamp, rad/s (0x7fbf34).
const SLIP_CLAMP: f32 = 300.0;
/// Clutch locks when engine and wheel side are this close, rad/s (0x7fc028).
const LOCK_DIFF: f32 = 50.0;
/// Shift rpm fall limit, torque units / I (0x7a1a2c): normal, while upshifting.
const RPM_DROP: [f32; 2] = [2.5, 15.0];
/// Nitrous torque curve idle..redline (0x7fbf38) and the level factors (0x7a1798).
const NOS_CURVE: [f32; 9] = [0.3, 0.4, 0.45, 0.5, 0.55, 0.6, 0.7, 1.0, 0.7];
pub const NOS_LEVEL: [f32; 3] = [1.0, 1.4, 1.7];

/// Gear index as the game numbers them.
pub const REVERSE: usize = 0;
pub const NEUTRAL: usize = 1;
pub const FIRST: usize = 2;

/// A piecewise-linear curve over evenly spaced points (0x5764d0).
fn curve(y: &[f32], x0: f32, x1: f32, x: f32) -> f32 {
    let n = y.len();
    let t = (x - x0) * (n - 1) as f32 / (x1 - x0);
    if t < 0.0 {
        return y[0];
    }
    let i = t as usize;
    if i >= n - 1 {
        return y[n - 1];
    }
    y[i] + (y[i + 1] - y[i]) * (t - i as f32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clutch {
    Disengaging = 0,
    Disengaged = 1,
    Engaging = 2,
    Engaged = 3,
}

#[derive(Clone, Debug)]
pub struct Powertrain {
    pub data: CarPhysics,
    /// Engine speed, rad/s (+0x14).
    pub w: f32,
    /// Throttle after the input stage (+0x78) and the clutch's scale on it (+0x80).
    pub throttle: f32,
    scale: f32,
    /// External torque on the engine this substep (+0x7c), reset after each.
    pub ext: f32,
    /// Nitrous level 0..3 (+0x84).
    pub nos: usize,
    pub limiter: bool,
    /// Drag-race engine heat (+0x8c); None outside drag.
    pub heat: Option<f32>,
    pub blown: bool,
    /// Clutch position 0..1 (+0x14) and state (+0x44).
    pub clutch: f32,
    pub state: Clutch,
    /// Gear (+0x3c) and the gear the box is going to (+0x38).
    pub gear: usize,
    pub target: usize,
    /// A shift is in progress (+0x48).
    pub shifting: bool,
    /// Wheel-side engine speed (+0x4c).
    pub wheel_side: f32,
    /// Shift rpm (car+0x440).
    pub shift_rpm: f32,
    up: Vec<f32>,
    down: Vec<f32>,
    /// Random shift offsets, rpm: upshift (+0x78), upshift from 2nd (+0x80), downshift.
    offsets: [f32; 3],
    last_diff: f32,
    rng: u32,
    /// Drag race gearbox (+0x60 = 3): the driver shifts, the clutch never slips.
    pub drag: bool,
    /// Shift window row (+0xf0, 3 by default).
    pub shift_skill: usize,
    /// Physics time, s, and the missed-shift (+0x5c) and shift bonus (+0x64 / +0x68) timers.
    time: f32,
    missed_time: f32,
    /// Quality of the last upshift (see [`Powertrain::shift_quality`]).
    pub bonus: f32,
    bonus_time: f32,
    /// Engine rpm when the last shift began (+0x6c).
    shift_start_rpm: f32,
}

/// Upshift timing windows, rpm, by skill row and gear index (0x7fbf60).
const SHIFT_WINDOWS: [[f32; 9]; 4] = [
    [0.0, 560.0, 360.0, 175.0, 160.0, 140.0, 140.0, 0.0, 0.0],
    [0.0, 600.0, 400.0, 230.0, 220.0, 160.0, 160.0, 0.0, 0.0],
    [0.0, 850.0, 550.0, 300.0, 250.0, 200.0, 180.0, 0.0, 0.0],
    [0.0, 1100.0, 750.0, 400.0, 325.0, 290.0, 230.0, 0.0, 0.0],
];

impl Powertrain {
    pub fn new(data: CarPhysics) -> Self {
        let ng = data.gear_count as usize;
        let (idle, red, max) = (data.idle_rpm, data.redline_rpm, data.max_rpm);
        // Shift points (0x5ac7d0).
        let mut up = vec![0.0; ng + 2];
        let mut down = vec![0.0; ng + 2];
        // The neutral -> first point scales by the drive type (0x7fc2a8 + 0x144*type, +0x10).
        let drive_type = if data.rear_bias < 0.2 { 1.0 } else if data.rear_bias > 0.8 { 1.1 } else { 1.2 };
        up[NEUTRAL] = (idle + (red - idle) * data.first_shift) * drive_type;
        for u in up.iter_mut().take(ng + 1).skip(FIRST) {
            *u = red - 100.0;
        }
        up[ng + 1] = max + 500.0;
        for g in FIRST..=ng {
            down[g + 1] = data.gears[g + 1] / data.gears[g] * up[g];
        }
        let mut p = Powertrain {
            w: idle * RAD_K,
            throttle: 0.0,
            scale: 1.0,
            ext: 0.0,
            nos: 0,
            limiter: false,
            heat: None,
            blown: false,
            clutch: 0.0,
            state: Clutch::Engaging,
            gear: NEUTRAL,
            target: NEUTRAL,
            shifting: false,
            wheel_side: 0.0,
            shift_rpm: idle,
            up,
            down,
            offsets: [0.0; 3],
            last_diff: 0.0,
            rng: 0x1d87_2b41,
            drag: false,
            shift_skill: 3,
            time: 0.0,
            missed_time: 0.0,
            bonus: 0.0,
            bonus_time: 0.0,
            shift_start_rpm: 0.0,
            data,
        };
        p.roll_offsets();
        p
    }

    pub fn top_gear(&self) -> usize {
        self.data.gear_count as usize + 1
    }

    pub fn rpm(&self) -> f32 {
        self.w * RPM_K
    }

    fn rand(&mut self, n: u32) -> u32 {
        let r = if n == 0 { 0 } else { self.rng % n };
        let s = self.rng ^ 0x1d87_2b41;
        let t = s ^ (s >> 5);
        self.rng = t ^ (t << 27) ^ s;
        r
    }

    /// New random shift offsets, −100..0 rpm each (0x594710).
    fn roll_offsets(&mut self) {
        for k in 0..3 {
            self.offsets[k] = -(self.rand(101) as f32);
        }
    }

    /// Upshift rpm of gear `g` (0x594670); `low` takes the lowest offset.
    fn upshift_rpm(&self, g: usize, low: bool) -> f32 {
        let o = if low {
            -100.0
        } else if g == FIRST + 1 {
            self.offsets[1]
        } else {
            self.offsets[0]
        };
        (self.up[g] + o).min(self.data.redline_rpm)
    }

    /// The input stage (0x5abbf0) and the gearbox (0x5aa960), once a frame.
    /// `request`: a gear the driver asks for (manual / reverse), None = automatic.
    /// `wheels_ok`: all wheels on the ground with slip under 4 m/s.
    pub fn frame_input(&mut self, throttle: f32, request: Option<usize>, wheels_ok: bool) {
        let mut th = if throttle <= 0.15 { 0.0 } else { throttle };
        if th > 0.0 && th < 0.5 && self.gear >= FIRST && self.shift_rpm < 3000.0 {
            th = 0.5;
        }
        self.throttle = if self.blown { 0.0 } else { th };
        if let Some(req) = request {
            let req = req.min(self.top_gear());
            if self.drag {
                if req != self.gear {
                    if self.state != Clutch::Disengaged {
                        // Shifting with the clutch in: a missed shift.
                        self.shifting = false;
                        self.missed_time = self.time;
                    } else if !self.shifting {
                        self.shifting = true;
                        self.target = req;
                        if req > self.gear {
                            self.on_upshift(self.shift_start_rpm);
                        }
                    }
                }
            } else if req != self.gear && req != self.target {
                self.target = req;
                if req > self.gear {
                    self.on_upshift(self.shift_rpm);
                }
            }
            return;
        }
        let g = self.gear;
        if g <= NEUTRAL {
            self.target = FIRST;
            return;
        }
        if self.state != Clutch::Engaged {
            return;
        }
        let rpm = self.shift_rpm;
        if rpm >= self.upshift_rpm(g, false) && g < self.top_gear() {
            if wheels_ok {
                self.target = g + 1;
            }
        } else if g > FIRST {
            let low = self.upshift_rpm(g - 1, true) * self.data.gears[g] / self.data.gears[g - 1] - 200.0;
            if rpm <= self.down[g] + self.offsets[2] && rpm <= low {
                self.target = g - 1;
            }
        }
        if self.state == Clutch::Engaged && self.target > g {
            self.on_upshift(rpm);
        }
    }

    /// Engine torque (0x594300), kN·m.
    pub fn engine_torque(&self) -> f32 {
        let d = &self.data;
        let rpm = self.rpm();
        let th = if self.limiter { 0.0 } else { self.scale * self.throttle };
        let mut t = curve(&d.torque, d.idle_rpm, d.max_rpm, rpm) * th;
        if self.nos > 0 {
            t += curve(&NOS_CURVE, d.idle_rpm, d.redline_rpm, rpm) * d.nitrous * NOS_LEVEL[self.nos - 1] * th;
        }
        if let Some(h) = self.heat {
            if h > 0.6 {
                t *= 1.0 - 0.75 * (h.min(1.0) - 0.6) / 0.4;
            }
        }
        t -= (1.0 - th) * curve(&d.engine_brake, d.idle_rpm, d.max_rpm, rpm) * if self.limiter { 2.0 } else { 1.0 };
        if t < 0.0 && !self.limiter {
            t *= if self.gear == NEUTRAL { 0.65 } else { 0.85 };
        }
        t
    }

    /// Engine angular acceleration with the torques known so far (0x5a1290).
    pub fn engine_accel(&self) -> f32 {
        (self.engine_torque() + self.ext) / self.data.engine_inertia
    }

    /// Gear × final drive for the front and rear axle.
    pub fn axle_ratios(&self) -> [f32; 2] {
        let g = self.data.gears.get(self.gear).copied().unwrap_or(0.0);
        [g * self.data.final_drive[0], g * self.data.final_drive[1]]
    }

    pub fn front_driven(&self) -> bool {
        self.data.rear_bias <= 0.999
    }

    pub fn rear_driven(&self) -> bool {
        self.data.rear_bias >= 0.001
    }

    /// Engine speed seen from the wheels (0x5945f0): axle wheel speeds (average of the
    /// two wheels, rad/s) through the final drives and the gear.
    pub fn wheel_side_speed(&self, front: f32, rear: f32) -> f32 {
        let d = &self.data;
        let mut acc = 0.0;
        if self.rear_driven() {
            acc = rear * d.final_drive[1];
        }
        if self.front_driven() {
            acc += front * d.final_drive[0];
        }
        if self.rear_driven() && self.front_driven() {
            acc *= 0.5;
        }
        acc * d.gears.get(self.gear).copied().unwrap_or(0.0)
    }

    /// The torque the clutch passes (0x594cb0): slipping in states 0 / 2, the engine's
    /// own torque when engaged, none when out.
    pub fn clutch_torque(&mut self) -> f32 {
        match self.state {
            Clutch::Disengaged => 0.0,
            Clutch::Engaged => self.engine_torque(),
            _ => {
                let diff = (self.wheel_side - self.w) * RPM_K;
                let mut k = self.data.clutch_stiffness;
                if self.last_diff != 0.0 && diff * self.last_diff < 0.0 && diff.abs() > 2000.0 && self.last_diff.abs() > 2000.0 {
                    k *= 0.5;
                }
                self.last_diff = diff;
                k * self.clutch * (self.w - self.wheel_side).clamp(-SLIP_CLAMP, SLIP_CLAMP)
            }
        }
    }

    /// Splits clutch torque `t` to the wheels FL FR RL RR (0x594a30) and takes the
    /// reaction off the engine. `wheel_w`: wheel speeds; `grounded_wheels`: on the ground.
    pub fn apply(&mut self, t: f32, wheel_w: [f32; 4], grounded_wheels: [bool; 4]) -> [f32; 4] {
        if self.gear == NEUTRAL {
            return [0.0; 4];
        }
        let grounded = grounded_wheels.iter().filter(|g| **g).count();
        self.ext -= (grounded as f32 * 0.25).max(0.25) * t;
        let [rf, rr] = self.axle_ratios();
        let bias = self.data.rear_bias;
        let axle = [(1.0 - bias) * rf * t, bias * rr * t];
        let mut out = [0.0; 4];
        for (a, &ta) in axle.iter().enumerate() {
            let (l, r) = (wheel_w[2 * a], wheel_w[2 * a + 1]);
            // Right wheel's share: by speed between s and 1 − s, the slower wheel gets
            // 82.5 % beyond (limited slip); s = 0.35 with both wheels down, else 0.5.
            let s = if grounded_wheels[2 * a] && grounded_wheels[2 * a + 1] { 0.35 } else { 0.5 };
            let sum = l + r;
            let right = if sum.abs() <= 1e-5 {
                0.5
            } else {
                let q = r / sum;
                if q < s {
                    1.0 - 0.35 * 0.5
                } else if q > 1.0 - s {
                    0.35 * 0.5
                } else {
                    q
                }
            };
            out[2 * a] = ta * (1.0 - right);
            out[2 * a + 1] = ta * right;
        }
        out
    }

    /// End of a substep: clutch, engine clamp and limiter, heat, clutch state machine.
    pub fn post_step(&mut self, h: f32) {
        // Clutch position (0x5949b0).
        let rate = if self.drag { 0.0 } else { self.data.clutch_rate * CLUTCH_RATE[self.state as usize] };
        self.clutch += rate * h;
        // Engine (0x5aa760).
        let d = &self.data;
        self.w = self.w.clamp(d.idle_rpm * RAD_K, d.max_rpm * RAD_K);
        self.ext = 0.0;
        let rpm = self.rpm();
        if self.limiter && rpm < d.max_rpm * (1.0 - 0.0125) {
            self.limiter = false;
        } else if !self.limiter && rpm > d.max_rpm - 25.0 {
            self.limiter = true;
        }
        if let Some(heat) = self.heat.as_mut() {
            *heat = (*heat + if rpm > d.redline_rpm { 0.45 } else { -0.1 } * h).max(0.0);
            if *heat >= 1.2 {
                self.blown = true;
            }
        }
        self.time += h;
        self.clutch_states();
    }

    /// Engine and wheel side within 50 rad/s (0x594ed0).
    fn can_lock(&self) -> bool {
        (self.w - self.wheel_side).abs() <= LOCK_DIFF
    }

    /// Off the throttle with the wheels slower than idle: the clutch comes out (0x594e90).
    fn declutch(&self) -> bool {
        self.throttle < 0.3 && self.data.idle_rpm * RAD_K + 0.0001 >= self.wheel_side
    }

    /// The clutch state machine (0x5a1400); each state sets the throttle scale first.
    fn clutch_states(&mut self) {
        if self.drag {
            // Drag: the clutch stays in; a requested shift happens in state 1.
            self.scale = 1.0;
            if self.state == Clutch::Engaged {
                self.shift_start_rpm = self.rpm();
                if self.bonus > 0.0 && self.bonus_time == 0.0 {
                    self.bonus_time = self.time;
                }
            } else if self.state == Clutch::Disengaged && self.shifting {
                self.shifting = false;
                self.gear = self.target;
            }
        } else {
            match self.state {
                Clutch::Engaged => {
                    if self.bonus > 0.0 && self.bonus_time == 0.0 {
                        self.bonus_time = self.time;
                    }
                    self.scale = 1.0;
                    if self.gear != NEUTRAL && self.declutch() {
                        self.state = Clutch::Disengaging;
                    }
                    if self.gear != self.target {
                        self.shifting = true;
                        self.state = Clutch::Disengaging;
                        self.shift_start_rpm = self.rpm();
                    } else {
                        self.shifting = false;
                    }
                }
                Clutch::Engaging => {
                    self.scale = 0.7;
                    if self.clutch >= 1.0 {
                        self.clutch = 1.0;
                        if self.can_lock() || self.gear == NEUTRAL {
                            self.state = Clutch::Engaged;
                        }
                    }
                    if self.declutch() && self.gear != NEUTRAL {
                        self.state = Clutch::Disengaging;
                    }
                }
                Clutch::Disengaging => {
                    self.scale = 0.0;
                    if self.clutch <= 0.0 {
                        self.clutch = 0.0;
                        self.state = Clutch::Disengaged;
                    }
                }
                Clutch::Disengaged => {
                    self.scale = 0.0;
                    self.gear = self.target;
                    if self.shifting || !self.declutch() {
                        self.state = Clutch::Engaging;
                        self.roll_offsets();
                    }
                }
            }
        }
        // A missed drag shift: no throttle for 0.45 s. A good shift: the throttle scale
        // rises by up to 2 x the shift quality, fading over 0.5 s.
        if self.missed_time != 0.0 {
            self.scale = 0.0;
            if self.time - self.missed_time >= 0.45 {
                self.missed_time = 0.0;
            }
        } else if self.state == Clutch::Engaged && self.bonus_time > 0.0 {
            let k = (0.5 - (self.time - self.bonus_time)) / 0.5 * self.bonus * 2.0;
            if k > 0.0 {
                self.scale = 1.0 + k;
            }
            if self.time - self.bonus_time >= 0.5 {
                self.bonus_time = 0.0;
                self.bonus = 0.0;
            }
        }
    }

    /// How well an upshift at `rpm` from gear `g` is timed (0x594f20): 1 perfect (within
    /// 5 % of the window past the shift point), 0.85 / 0.5 good / early, 0 too early,
    /// -1 late. The window is per gear and drag skill row (0x7fbf60).
    pub fn shift_quality(&self, rpm: f32, g: usize) -> f32 {
        let win = SHIFT_WINDOWS[self.shift_skill.min(3)][g.min(8)];
        if win <= 0.0 {
            return 0.0;
        }
        let diff = self.up[g] - rpm;
        if diff >= 0.0 {
            let q = 1.0 - diff / (0.95 * win);
            if q <= 0.0 {
                0.0
            } else if q < 0.5 {
                0.5
            } else {
                0.85
            }
        } else if (1.0 - 0.95) * win >= diff.abs() {
            1.0
        } else {
            -1.0
        }
    }

    /// An upshift starts (0x5a1700): in a drag race its quality becomes the bonus.
    fn on_upshift(&mut self, rpm: f32) {
        if self.drag && self.target >= self.gear {
            self.bonus = self.shift_quality(rpm, self.gear);
        }
    }

    /// The shift rpm, once a frame after the substeps (0x5a5540).
    pub fn frame_end(&mut self, dt: f32) {
        let mut raw = self.rpm();
        let k = if self.shifting && self.target > self.gear { RPM_DROP[1] } else { RPM_DROP[0] };
        let drop = -k / self.data.engine_inertia;
        if (raw - self.shift_rpm) / dt < drop {
            raw = raw.max(self.shift_rpm + drop * dt);
        }
        self.shift_rpm = 0.45 * self.shift_rpm + 0.55 * raw;
    }
}
