//! Chase / bumper cameras.

use glam::{Mat4, Vec3};

use crate::collision::CollisionWorld;
use crate::vehicle::Vehicle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    ChaseNear,
    ChaseFar,
    Bumper,
}

pub struct ChaseCamera {
    pub mode: CameraMode,
    pub eye: Vec3,
    pub target: Vec3,
    yaw: f32,
    pub fov_y: f32,
}

impl ChaseCamera {
    pub fn new() -> Self {
        Self { mode: CameraMode::ChaseNear, eye: Vec3::new(-6.0, 0.0, 3.0), target: Vec3::ZERO, yaw: 0.0, fov_y: 55f32.to_radians() }
    }

    pub fn next_mode(&mut self) {
        self.mode = match self.mode {
            CameraMode::ChaseNear => CameraMode::ChaseFar,
            CameraMode::ChaseFar => CameraMode::Bumper,
            CameraMode::Bumper => CameraMode::ChaseNear,
        };
    }

    pub fn snap(&mut self, car: &Vehicle) {
        self.yaw = car.yaw;
        self.update(car, false, None, 1.0);
    }

    pub fn update(&mut self, car: &Vehicle, look_back: bool, world: Option<&CollisionWorld>, dt: f32) {
        // Follow the direction of travel when sliding, the nose otherwise.
        let mut heading = car.yaw;
        let v = Vec3::new(car.vel.x, car.vel.y, 0.0);
        if v.length() > 4.0 && car.speed() > 0.0 {
            let travel = v.y.atan2(v.x);
            heading = car.yaw + wrap(travel - car.yaw) * 0.5;
        }
        let lag = if self.mode == CameraMode::Bumper { 30.0 } else { 5.0 };
        self.yaw += wrap(heading - self.yaw) * (1.0 - (-lag * dt).exp());
        let mut dir = Vec3::new(self.yaw.cos(), self.yaw.sin(), 0.0);
        if look_back {
            dir = -dir;
        }
        let speed = car.speed().abs();
        let (back, height, look_ahead, look_h) = match self.mode {
            CameraMode::ChaseNear => (5.6 + speed * 0.012, 1.75, 4.0, 0.9),
            CameraMode::ChaseFar => (8.5 + speed * 0.02, 2.6, 5.0, 1.0),
            CameraMode::Bumper => (-1.9, 0.75, 20.0, 0.7),
        };
        let mut eye = car.pos - dir * back + Vec3::Z * height;
        if let Some(w) = world {
            if let Some((gz, _)) = w.ground(eye, 3.0, 10.0) {
                eye.z = eye.z.max(gz + 0.6);
            }
        }
        self.eye = eye;
        self.target = car.pos + dir * look_ahead + Vec3::Z * look_h;
        // Widen the view with speed for a sense of velocity.
        let base = if self.mode == CameraMode::Bumper { 62.0 } else { 55.0 };
        self.fov_y = (base + (speed / 72.0).min(1.0) * 12.0).to_radians();
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye, self.target, Vec3::Z)
    }
}

impl Default for ChaseCamera {
    fn default() -> Self {
        Self::new()
    }
}

fn wrap(a: f32) -> f32 {
    let mut a = a % std::f32::consts::TAU;
    if a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    if a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}
