//! City traffic: AI-driven cars in the right-hand lane of the free-roam road network,
//! spawned out of sight around the player and removed when far away.

use glam::{Vec2, Vec3};

use crate::ai::AiDriver;
use crate::roads::{RoadGraph, RouteEnd};
use crate::vehicle::Vehicle;

pub struct TrafficCar {
    pub vehicle: Vehicle,
    pub model: String,
    /// Index into the model's skins.
    pub skin: usize,
    pub driver: AiDriver,
    pub lane: LanePath,
    pub brake_light: f32,
}

/// The lane a traffic car follows: points ahead, refilled from the road graph.
pub struct LanePath {
    route: usize,
    forward: bool,
    /// Next route point to append, in travel order.
    next: usize,
    pub ahead: Vec<Vec3>,
}

fn lane_offset(g: &RoadGraph, route: usize, i: usize, forward: bool) -> f32 {
    let p = &g.routes[route].points[i];
    let w = if forward { p.right_width } else { p.left_width };
    (w * 0.4).clamp(2.0, 4.5)
}

impl LanePath {
    pub fn new(g: &RoadGraph, route: usize, point: usize, forward: bool) -> Self {
        let mut l = Self { route, forward, next: point, ahead: Vec::new() };
        l.refill(g, &mut 1);
        l
    }

    fn point(&self, g: &RoadGraph, i: usize) -> Vec3 {
        let pts = &g.routes[self.route].points;
        let p = Vec3::from(pts[i].pos);
        // Travel direction at this point, then step to its right.
        let (a, b) = if self.forward {
            (pts[i.saturating_sub(1).min(i)], pts[(i + 1).min(pts.len() - 1)])
        } else {
            (pts[(i + 1).min(pts.len() - 1)], pts[i.saturating_sub(1)])
        };
        let d = Vec2::new(b.pos[0] - a.pos[0], b.pos[1] - a.pos[1]).normalize_or_zero();
        let right = Vec2::new(d.y, -d.x);
        let off = right * lane_offset(g, self.route, i, self.forward);
        p + Vec3::new(off.x, off.y, 0.0)
    }

    /// Keeps about 300 m of lane ahead, continuing onto another road at junctions.
    pub fn refill(&mut self, g: &RoadGraph, rng: &mut u32) {
        let mut guard = 0;
        while self.length() < 300.0 && guard < 64 {
            guard += 1;
            let n = g.routes[self.route].points.len();
            if self.next < n {
                let p = self.point(g, self.next);
                // Long segments become several points so steering stays smooth.
                if let Some(&last) = self.ahead.last() {
                    let steps = (last.distance(p) / 8.0).ceil() as usize;
                    for s in 1..steps {
                        self.ahead.push(last.lerp(p, s as f32 / steps as f32));
                    }
                }
                self.ahead.push(p);
                if self.forward {
                    self.next += 1;
                } else if self.next == 0 {
                    self.next = n;
                } else {
                    self.next -= 1;
                }
                continue;
            }
            // End of this road: pick another road leaving the junction.
            let node = g.ends[self.route][if self.forward { 1 } else { 0 }];
            let options: Vec<RouteEnd> = g.nodes[node].iter().copied().filter(|e| e.route != self.route).collect();
            let choice = if options.is_empty() {
                RouteEnd { route: self.route, start: !self.forward } // dead end: turn around
            } else {
                *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                options[(*rng >> 16) as usize % options.len()]
            };
            self.route = choice.route;
            self.forward = choice.start;
            let len = g.routes[self.route].points.len();
            // Skip the shared junction point.
            self.next = if self.forward { 1.min(len - 1) } else { len.saturating_sub(2) };
        }
    }

    fn length(&self) -> f32 {
        self.ahead.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// Drops points the car has passed.
    pub fn advance(&mut self, pos: Vec3) {
        while self.ahead.len() > 3 {
            let (a, b) = (self.ahead[0], self.ahead[1]);
            if (pos - b).truncate().dot((b - a).truncate()) > 0.0 || pos.distance(a) > pos.distance(b) + 0.5 {
                self.ahead.remove(0);
            } else {
                break;
            }
        }
    }
}

pub struct Traffic {
    pub cars: Vec<TrafficCar>,
    pub density: usize,
    rng: u32,
}

impl Traffic {
    pub fn new(density: usize) -> Self {
        Self { cars: Vec::new(), density, rng: 0x2545_f491 }
    }

    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// Removes far cars and spawns new ones on roads 150-320 m from `center`.
    /// `models` lists loaded traffic models with (ride height, wheelbase, skin count).
    pub fn maintain(&mut self, g: &RoadGraph, center: Vec3, models: &[(String, f32, f32, usize, crate::vehicle::Perf)]) {
        self.cars.retain(|c| c.vehicle.pos.distance(center) < 420.0 && c.vehicle.pos.z > -150.0);
        if models.is_empty() || g.routes.is_empty() {
            return;
        }
        let mut tries = 0;
        while self.cars.len() < self.density && tries < 40 {
            tries += 1;
            let ri = self.rand() as usize % g.routes.len();
            let r = &g.routes[ri];
            if r.is_connector() || r.points.len() < 3 {
                continue;
            }
            let pi = 1 + self.rand() as usize % (r.points.len() - 2);
            let p = Vec3::from(r.points[pi].pos);
            let d = p.truncate().distance(center.truncate());
            if !(150.0..320.0).contains(&d) || (p.z - center.z).abs() > 40.0 {
                continue;
            }
            // Keep spacing from other traffic.
            if self.cars.iter().any(|c| c.vehicle.pos.distance(p) < 25.0) {
                continue;
            }
            let forward = self.rand() & 1 == 0;
            let (name, ride, wheelbase, skins, perf) = &models[self.rand() as usize % models.len()];
            let mut lane = LanePath::new(g, ri, pi, forward);
            if lane.ahead.len() < 2 {
                continue;
            }
            let start = lane.ahead[0];
            let dir = (lane.ahead[1] - start).truncate().normalize_or_zero();
            let yaw = dir.y.atan2(dir.x);
            let mut vehicle = Vehicle::new(start + Vec3::Z * (ride + 0.4), yaw, *ride, *wheelbase);
            vehicle.vel = Vec3::new(dir.x, dir.y, 0.0) * 11.0;
            vehicle.perf = *perf;
            lane.advance(start);
            let skin = if *skins > 0 { self.rand() as usize % skins } else { 0 };
            let top = 12.0 + (self.rand() % 50) as f32 / 10.0;
            self.cars.push(TrafficCar {
                vehicle,
                model: name.clone(),
                skin,
                driver: AiDriver::new(0.55, top),
                lane,
                brake_light: 0.0,
            });
        }
    }
}
