//! Arcade vehicle model, tuned to feel like UG2's street handling: strong grip,
//! handbrake slides, nitro boost, automatic gearbox. Car space is X forward, Y left, Z up.

use glam::{Mat3, Mat4, Quat, Vec3};

use crate::collision::CollisionWorld;
use crate::input::Controls;

pub struct Vehicle {
    pub pos: Vec3,
    pub yaw: f32,
    pub vel: Vec3,
    pub yaw_rate: f32,
    /// Smoothed ground normal used for pitch/roll.
    pub up: Vec3,
    pub grounded: bool,
    pub steer: f32,
    pub gear: i32,
    pub rpm: f32,
    pub nitro: f32,
    pub wheel_spin: f32,
    /// Height of the car origin above the ground (from wheel hub height and radius).
    pub ride_height: f32,
    pub wheelbase: f32,
    last_safe: (Vec3, f32),
    safe_timer: f32,
    /// Wall contacts in the last step (debug telemetry).
    pub wall_hits: u32,
    pub last_wall: u32,
}

const GRAVITY: f32 = 9.81;
const MAX_SPEED: f32 = 72.0; // ~260 km/h
const MAX_REVERSE: f32 = 14.0;
const GEAR_TOP: [f32; 6] = [14.0, 24.0, 35.0, 47.0, 60.0, MAX_SPEED];
/// Cornering grip (m/s², ~2 g, arcade): caps the yaw rate at speed to v·ω ≤ this, so full
/// lock from on/off touch buttons gives a fast but drivable turn instead of a spin.
const MAX_LAT_ACCEL: f32 = 20.0;
/// Floor for that cap (rad/s, ~40°/s) so the car still answers the buttons at 150+ km/h.
const MIN_YAW_CAP: f32 = 0.7;

impl Vehicle {
    pub fn new(pos: Vec3, yaw: f32, ride_height: f32, wheelbase: f32) -> Self {
        Self {
            pos,
            yaw,
            vel: Vec3::ZERO,
            yaw_rate: 0.0,
            up: Vec3::Z,
            grounded: false,
            steer: 0.0,
            gear: 1,
            rpm: 0.0,
            nitro: 1.0,
            wheel_spin: 0.0,
            ride_height,
            wheelbase,
            last_safe: (pos, yaw),
            safe_timer: 0.0,
            wall_hits: 0,
            last_wall: 0,
        }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), self.yaw.sin(), 0.0)
    }

    pub fn left(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin(), self.yaw.cos(), 0.0)
    }

    pub fn speed(&self) -> f32 {
        self.vel.dot(self.forward())
    }

    pub fn reset(&mut self, pos: Vec3, yaw: f32) {
        self.pos = pos;
        self.yaw = yaw;
        self.vel = Vec3::ZERO;
        self.yaw_rate = 0.0;
        self.up = Vec3::Z;
        self.last_safe = (pos, yaw);
        self.safe_timer = 0.0;
    }

    /// Back to the last place the car was driving normally.
    pub fn reset_to_safe(&mut self) {
        let (p, y) = self.last_safe;
        self.reset(p + Vec3::Z * 0.5, y);
    }

    pub fn step(&mut self, c: &Controls, world: Option<&CollisionWorld>, dt: f32) {
        let fwd = self.forward();
        let left = self.left();
        let mut v_long = self.vel.dot(fwd);
        let mut v_lat = self.vel.dot(left);
        let mut v_up = self.vel.z;

        // Steering: rate-limited, with less lock at speed.
        let speed_factor = 1.0 / (1.0 + v_long.abs() / 22.0);
        let target_steer = (-c.steer).clamp(-1.0, 1.0) * speed_factor.max(0.22);
        let rate = if target_steer.abs() < self.steer.abs() { 6.0 } else { 3.2 };
        self.steer += (target_steer - self.steer).clamp(-rate * dt, rate * dt);

        if self.grounded {
            // Engine / brakes along the car.
            let nitro_on = c.nitro && self.nitro > 0.0 && c.throttle > 0.1;
            if nitro_on {
                self.nitro = (self.nitro - dt / 6.0).max(0.0);
            } else {
                self.nitro = (self.nitro + dt / 40.0).min(1.0);
            }
            let top = if nitro_on { MAX_SPEED * 1.15 } else { MAX_SPEED };
            let power = 14.0 * (1.0 - (v_long / top).clamp(0.0, 1.0).powf(1.6)) + 2.0;
            let mut accel = c.throttle * power * if nitro_on { 1.6 } else { 1.0 };
            if c.brake > 0.05 {
                if v_long > 0.5 {
                    accel -= c.brake * 22.0;
                } else if v_long > -MAX_REVERSE {
                    accel -= c.brake * 7.0;
                }
            }
            if c.handbrake {
                accel -= v_long.signum() * 6.0 * (v_long.abs() > 0.3) as u8 as f32;
            }
            // Aerodynamic drag and rolling resistance.
            accel -= v_long * v_long.abs() * 0.0012 + v_long * 0.12;
            if c.throttle < 0.05 && c.brake < 0.05 && v_long.abs() < 0.3 {
                v_long = 0.0;
            } else {
                v_long += accel * dt;
            }

            // Lateral grip: kill sideways velocity, much less with the handbrake.
            let grip = if c.handbrake { 1.6 } else { 9.0 };
            v_lat *= (-grip * dt).exp();

            // Yaw from a bicycle model; handbrake adds rotation for drifts.
            let steer_angle = self.steer * 0.6;
            let mut target_rate = v_long / self.wheelbase * steer_angle.tan();
            let max_rate = (MAX_LAT_ACCEL / v_long.abs().max(1.0)).max(MIN_YAW_CAP);
            target_rate = target_rate.clamp(-max_rate, max_rate);
            if c.handbrake {
                target_rate *= 1.6;
            }
            let yaw_resp = if c.handbrake { 4.0 } else { 9.0 };
            self.yaw_rate += (target_rate - self.yaw_rate) * (1.0 - (-yaw_resp * dt).exp());
        } else {
            self.yaw_rate *= (-1.0 * dt).exp();
        }
        self.yaw += self.yaw_rate * dt;
        v_up -= GRAVITY * dt;

        let fwd = self.forward();
        let left = self.left();
        self.vel = fwd * v_long + left * v_lat + Vec3::Z * v_up;
        self.pos += self.vel * dt;

        // Ground contact.
        self.grounded = false;
        if let Some(world) = world {
            let probe = self.pos;
            if let Some((gz, n)) = world.ground(probe, 1.2, 3.0) {
                let target = gz + self.ride_height;
                if self.pos.z <= target + 0.05 {
                    self.pos.z = target;
                    if self.vel.z < 0.0 {
                        self.vel.z = 0.0;
                    }
                    self.grounded = true;
                    self.up = self.up.lerp(n, 1.0 - (-12.0 * dt).exp()).normalize();
                }
            }
            if !self.grounded {
                self.up = self.up.lerp(Vec3::Z, 1.0 - (-1.5 * dt).exp()).normalize();
            }
            // Walls: test points along the body.
            let mut push = Vec3::ZERO;
            let ground_z = self.pos.z - self.ride_height;
            self.wall_hits = 0;
            for off in [1.5, 0.0, -1.5] {
                let p = self.pos + fwd * off + Vec3::Z * 0.6;
                world.walls(p, 0.95, ground_z + 0.45, |n, depth, t| {
                    push += n * depth;
                    self.wall_hits += 1;
                    self.last_wall = t.owner;
                });
            }
            if push != Vec3::ZERO {
                let n = push.normalize();
                self.pos += push.clamp_length_max(1.0);
                let into = self.vel.dot(n);
                if into < 0.0 {
                    // Remove the inward component, bounce a little, scrub speed.
                    self.vel -= n * into * 1.3;
                    self.vel *= 0.92;
                    self.yaw_rate *= 0.5;
                }
            }
            // Fell through the world: put the car back.
            if world.ground(self.pos, 50.0, 400.0).is_none() && self.pos.z < -200.0 {
                self.reset_to_safe();
            }
        } else if self.pos.z <= 0.0 {
            self.pos.z = 0.0;
            self.vel.z = 0.0;
            self.grounded = true;
        }

        if self.grounded && self.up.z > 0.9 && self.vel.length() > 2.0 {
            self.safe_timer += dt;
            if self.safe_timer > 1.0 {
                self.last_safe = (self.pos, self.yaw);
                self.safe_timer = 0.0;
            }
        }

        // Automatic gearbox, for the HUD and engine sound later.
        let v = self.speed();
        if v < -0.5 {
            self.gear = -1;
            self.rpm = (v.abs() / MAX_REVERSE).min(1.0);
        } else {
            let mut g = 1;
            while g < 6 && v > GEAR_TOP[g as usize - 1] * 0.97 {
                g += 1;
            }
            self.gear = g;
            let lo = if g == 1 { 0.0 } else { GEAR_TOP[g as usize - 2] * 0.6 };
            let hi = GEAR_TOP[g as usize - 1];
            self.rpm = (0.18 + 0.82 * ((v - lo) / (hi - lo)).clamp(0.0, 1.0)).min(1.0);
            if c.throttle > 0.5 && v < 2.0 {
                self.rpm = 0.6;
            }
        }
    }

    pub fn advance_wheels(&mut self, radius: f32, dt: f32) {
        self.wheel_spin = (self.wheel_spin + self.speed() / radius.max(0.1) * dt) % std::f32::consts::TAU;
    }

    /// Car-to-world transform including pitch/roll from the ground normal.
    pub fn transform(&self) -> Mat4 {
        let up = self.up;
        let f = (self.forward() - up * self.forward().dot(up)).normalize();
        let l = up.cross(f);
        Mat4::from_cols(f.extend(0.0), l.extend(0.0), up.extend(0.0), self.pos.extend(1.0))
    }

    pub fn rotation(&self) -> Quat {
        let up = self.up;
        let f = (self.forward() - up * self.forward().dot(up)).normalize();
        Quat::from_mat3(&Mat3::from_cols(f, up.cross(f), up))
    }
}
