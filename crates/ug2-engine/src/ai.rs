//! Computer drivers for racers and traffic: steer at a point ahead on the path (pure
//! pursuit) and pick the highest speed from which every corner ahead can still be made.

use glam::{Vec2, Vec3};

use crate::input::Controls;
use crate::vehicle::Vehicle;

/// Lateral grip the planner assumes (m/s²); the car model allows a bit more.
const CORNER_GRIP: f32 = 13.0;
/// Braking the planner assumes (m/s²); the car brakes at ~22.
const BRAKING: f32 = 14.0;

pub struct AiDriver {
    /// Multiplier on corner speeds (skill / caution).
    pub pace: f32,
    /// Speed cap, m/s.
    pub top_speed: f32,
    /// Seconds spent not moving while trying to.
    stuck: f32,
    /// Seconds of reversing left to get unstuck.
    reversing: f32,
    /// Set when the driver wants to be put back on the road.
    pub wants_reset: bool,
}

impl AiDriver {
    pub fn new(pace: f32, top_speed: f32) -> Self {
        Self { pace, top_speed, stuck: 0.0, reversing: 0.0, wants_reset: false }
    }

    /// `path`: points ahead of the car in driving order, starting near it.
    pub fn drive(&mut self, v: &Vehicle, path: &[Vec3], dt: f32) -> Controls {
        let mut c = Controls::default();
        if path.len() < 2 {
            c.brake = 1.0;
            return c;
        }
        let pos = v.pos;
        let speed = v.speed();
        let fwd = v.forward();
        let left = v.left();

        // Steering: pure pursuit on a look-ahead point.
        let look = 5.0 + speed.abs() * 0.45;
        let target = point_along(path, pos, look);
        let d = target - pos;
        let (x, y) = (d.dot(fwd), d.dot(left));
        let k = 2.0 * y / (x * x + y * y).max(1.0);
        let angle = (k * v.wheelbase).atan();
        let speed_factor = (1.0 / (1.0 + speed.abs() / 22.0)).max(0.22);
        c.steer = (-(angle / 0.6) / speed_factor).clamp(-1.0, 1.0);

        // Speed: every point ahead caps the speed by its corner and the braking room.
        let mut allowed = self.top_speed;
        let mut dist = 0.0;
        for i in 1..path.len() - 1 {
            dist += path[i].distance(path[i - 1]);
            if dist > 250.0 {
                break;
            }
            let a = (path[i] - path[i - 1]).truncate();
            let b = (path[i + 1] - path[i]).truncate();
            let chord = (path[i + 1] - path[i - 1]).truncate().length().max(0.5);
            let turn = a.angle_to(b).abs();
            let kappa = 2.0 * (turn * 0.5).sin() / (chord * 0.5);
            let corner = if kappa > 1e-4 { (CORNER_GRIP * self.pace / kappa).sqrt() } else { f32::MAX };
            let from_dist = (dist - (look * 0.3)).max(0.0);
            allowed = allowed.min((corner * corner + 2.0 * BRAKING * from_dist).sqrt());
        }
        // Off the line: slow down to get back.
        let off = Vec2::new(path[0].x - pos.x, path[0].y - pos.y).length();
        if off > 8.0 {
            allowed = allowed.min(15.0);
        }
        if speed < allowed - 0.5 {
            c.throttle = ((allowed - speed) / 3.0).clamp(0.3, 1.0);
        } else if speed > allowed + 1.0 {
            c.brake = ((speed - allowed) / 6.0).clamp(0.2, 1.0);
        }

        // Unstick: back up with the wheels turned the other way, then try again.
        if self.reversing > 0.0 {
            self.reversing -= dt;
            return Controls { steer: -c.steer, brake: 1.0, ..Controls::default() };
        }
        if c.throttle > 0.3 && speed.abs() < 1.0 {
            self.stuck += dt;
            if self.stuck > 2.0 {
                self.stuck = 0.0;
                self.reversing = 1.3;
                if off > 20.0 {
                    self.wants_reset = true;
                }
            }
        } else {
            self.stuck = 0.0;
        }
        c
    }
}

/// The point `dist` metres along `path`, measured from the path point nearest `from`.
pub fn point_along(path: &[Vec3], from: Vec3, dist: f32) -> Vec3 {
    let start = path
        .iter()
        .take(12)
        .enumerate()
        .min_by(|a, b| a.1.distance_squared(from).total_cmp(&b.1.distance_squared(from)))
        .map_or(0, |(i, _)| i);
    let mut left = dist;
    for w in path[start..].windows(2) {
        let seg = w[0].distance(w[1]);
        if seg >= left {
            return w[0] + (w[1] - w[0]) * (left / seg.max(1e-3));
        }
        left -= seg;
    }
    *path.last().unwrap_or(&from)
}
