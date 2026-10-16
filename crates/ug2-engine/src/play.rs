//! Gameplay session: the cars besides the player (race opponents and traffic), the
//! race itself and car-to-car contact.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec2, Vec3};
use ug2_formats::carinfo::WheelInfo;

use crate::ai::AiDriver;
use crate::collision::CollisionWorld;
use crate::input::Controls;
use crate::race::{Race, RacePhase};
use crate::roads::{Mode as RaceMode, RaceDef, RaceLine, RoadData};
use crate::traffic::Traffic;
use crate::vehicle::{Perf, Vehicle};

/// What physics needs from a loaded car model.
#[derive(Clone, Copy, Debug)]
pub struct ModelPhysics {
    pub ride_height: f32,
    pub wheelbase: f32,
    pub wheel_radius: f32,
    pub skins: usize,
}

pub struct Opponent {
    pub vehicle: Vehicle,
    pub model: String,
    pub paint: [f32; 3],
    pub driver: AiDriver,
    /// Sideways offset from the race line, metres (spreads the pack).
    pub lane: f32,
    pub controls: Controls,
    pub brake_light: f32,
    base_pace: f32,
}

pub enum Mode {
    FreeRoam,
    Race(Race),
}

pub struct Session {
    pub mode: Mode,
    pub opponents: Vec<Opponent>,
    pub traffic: Traffic,
    pub traffic_on: bool,
    /// Drag races: the player's lane, metres left of the race line.
    pub player_lane: f32,
    /// Street races have traffic; drags, drifts and the closed tracks do not.
    traffic_allowed: bool,
    traffic_controls: Vec<Controls>,
    rng: u32,
}

impl Session {
    pub fn new(traffic_density: usize) -> Self {
        Self {
            mode: Mode::FreeRoam,
            opponents: Vec::new(),
            traffic: Traffic::new(traffic_density),
            traffic_on: true,
            player_lane: 0.0,
            traffic_allowed: true,
            traffic_controls: Vec::new(),
            rng: 0x9E37_79B9,
        }
    }

    pub fn race(&self) -> Option<&Race> {
        match &self.mode {
            Mode::Race(r) => Some(r),
            Mode::FreeRoam => None,
        }
    }

    pub fn race_mut(&mut self) -> Option<&mut Race> {
        match &mut self.mode {
            Mode::Race(r) => Some(r),
            Mode::FreeRoam => None,
        }
    }

    /// Ends any race and drives freely.
    pub fn free_roam(&mut self) {
        self.mode = Mode::FreeRoam;
        self.opponents.clear();
        self.traffic_allowed = true;
    }

    /// Puts the player and opponents on the grid and starts the countdown.
    /// `opponents`: (model, paint, physics) for each AI racer.
    pub fn start_race(
        &mut self,
        def: RaceDef,
        laps: u32,
        player: &mut Vehicle,
        opponents: &[(String, [f32; 3], ModelPhysics, Perf)],
    ) {
        let n = opponents.len() + 1;
        let slots: Vec<Vec3> = (0..n).map(|i| def.grid[i.min(def.grid.len() - 1)]).collect();
        // Every grid slot faces along the race line. The slots stand side by side, up to
        // 7 m to either side of the start marker, so aiming at the marker turned the outer
        // ones towards the barriers.
        let face = |p: Vec3| {
            let i = def.line.locate(p, None);
            let n = def.line.points.len();
            let t = (def.line.points[(i + 1) % n] - def.line.points[i]).truncate();
            t.y.atan2(t.x)
        };
        // Sideways offset of a grid slot from the race line (drag lanes).
        let lane_of = |p: Vec3| {
            let i = def.line.locate(p, None);
            let m = def.line.points.len();
            let d = (def.line.points[(i + 1) % m] - def.line.points[i]).truncate().normalize_or_zero();
            (p - def.line.points[i]).truncate().dot(glam::Vec2::new(-d.y, d.x))
        };
        let drag = def.info.mode == RaceMode::Drag;
        self.traffic_allowed = matches!(def.info.mode, RaceMode::Circuit | RaceMode::Sprint);
        // Player starts at the back of the grid, as in the original.
        let player_slot = n - 1;
        let p = slots[player_slot];
        player.reset(p + Vec3::Z * (player.ride_height + 0.3), face(p));
        player.reset_engine();
        self.player_lane = lane_of(p);
        self.opponents.clear();
        for (i, (model, paint, phys, perf)) in opponents.iter().enumerate() {
            let p = slots[i];
            let mut v = Vehicle::new(p + Vec3::Z * (phys.ride_height + 0.3), face(p), phys.ride_height, phys.wheelbase);
            v.reset(p + Vec3::Z * (phys.ride_height + 0.3), face(p));
            v.perf = *perf;
            let pace = 0.92 + 0.06 * (self.rand() % 100) as f32 / 100.0;
            let lane = if drag { lane_of(p) } else { ((i as f32 % 3.0) - 1.0) * 2.2 };
            self.opponents.push(Opponent {
                vehicle: v,
                model: model.clone(),
                paint: *paint,
                driver: AiDriver::new(pace, perf.top_speed * 1.05),
                lane,
                controls: Controls::default(),
                brake_light: 0.0,
                base_pace: pace,
            });
        }
        let mut starts = vec![player.pos];
        starts.extend(self.opponents.iter().map(|o| o.vehicle.pos));
        // Traffic near the grid would block the start.
        let grid_center = def.start_line;
        self.traffic.cars.retain(|c| c.vehicle.pos.distance(grid_center) > 120.0);
        self.mode = Mode::Race(Race::new(def, laps, &starts));
    }

    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// Once per frame: AI decisions, traffic upkeep, race progress.
    pub fn plan(
        &mut self,
        dt: f32,
        player: &Vehicle,
        roads: Option<&RoadData>,
        traffic_models: &[(String, f32, f32, usize, Perf)],
    ) {
        // Race opponents follow the race line.
        let countdown = matches!(self.race().map(|r| r.phase), Some(RacePhase::Countdown(_)));
        if let Mode::Race(race) = &mut self.mode {
            let mut positions = vec![player.pos];
            positions.extend(self.opponents.iter().map(|o| o.vehicle.pos));
            race.update(&positions, dt);
            let line = &race.def.line;
            let player_total = race.cars[0].total;
            for (k, o) in self.opponents.iter_mut().enumerate() {
                let prog = &race.cars[k + 1];
                if countdown || prog.finish_time.is_some() {
                    // Stop past the line; held at a standstill the brake would reverse.
                    let rolling = o.vehicle.vel.dot(o.vehicle.forward()) > 0.5;
                    o.controls = Controls { brake: if !countdown && rolling { 1.0 } else { 0.0 }, ..Controls::default() };
                    continue;
                }
                let path = line_path(line, prog.index, o.lane, 70);
                // Rubber band: a little faster when behind the player, slower far ahead.
                let gap = (player_total - prog.total) / 300.0;
                o.driver.pace = o.base_pace * (1.0 + gap.clamp(-0.12, 0.12));
                let mut c = o.driver.drive(&o.vehicle, &path, dt);
                c.nitro = gap > 0.15 && c.steer.abs() < 0.15 && c.throttle > 0.9;
                o.controls = c;
                if o.driver.wants_reset || prog.wrong_way > 3.0 {
                    o.driver.wants_reset = false;
                    let (p, yaw) = race.reset_point(k + 1);
                    o.vehicle.reset(p + Vec3::Z * (o.vehicle.ride_height + 0.5), yaw);
                }
            }
        }

        // Traffic follows its lanes and keeps its distance from whatever is ahead.
        let traffic = self.traffic_on && self.traffic_allowed;
        if let (Some(roads), true) = (roads, traffic) {
            self.traffic.maintain(&roads.graph, player.pos, traffic_models);
        } else if !traffic {
            self.traffic.cars.clear();
        }
        let others: Vec<(Vec3, Vec3)> = std::iter::once((player.pos, player.vel))
            .chain(self.opponents.iter().map(|o| (o.vehicle.pos, o.vehicle.vel)))
            .chain(self.traffic.cars.iter().map(|t| (t.vehicle.pos, t.vehicle.vel)))
            .collect();
        self.traffic_controls.clear();
        let mut rng = self.rng;
        for (ti, t) in self.traffic.cars.iter_mut().enumerate() {
            if let Some(roads) = roads {
                t.lane.advance(t.vehicle.pos);
                t.lane.refill(&roads.graph, &mut rng);
            }
            let fwd = t.vehicle.forward();
            let mut limit = f32::MAX;
            for (oi, &(p, vel)) in others.iter().enumerate() {
                if oi == ti + 1 + self.opponents.len() {
                    continue;
                }
                let d = p - t.vehicle.pos;
                let ahead = d.dot(fwd);
                let side = (d - fwd * ahead).truncate().length();
                if ahead > 1.0 && ahead < 30.0 && side < 2.6 && d.z.abs() < 4.0 {
                    let their = vel.dot(fwd).max(0.0);
                    limit = limit.min(their + (ahead - 8.0).max(0.0) * 0.6);
                }
            }
            let saved = t.driver.top_speed;
            t.driver.top_speed = saved.min(limit.max(0.0));
            let c = t.driver.drive(&t.vehicle, &t.lane.ahead, dt);
            t.driver.top_speed = saved;
            if t.driver.wants_reset {
                t.driver.wants_reset = false;
                t.vehicle.pos.z = -1000.0; // removed by the next upkeep
            }
            self.traffic_controls.push(c);
        }
        self.rng = rng;
    }

    /// One physics step for every AI car, then car-to-car contact (player included).
    pub fn step(&mut self, player: &mut Vehicle, world: Option<&CollisionWorld>, dt: f32) {
        for o in &mut self.opponents {
            o.vehicle.step(&o.controls, world, dt);
        }
        for (t, c) in self.traffic.cars.iter_mut().zip(&self.traffic_controls) {
            t.vehicle.step(c, world, dt);
        }
        let mut bodies: Vec<&mut Vehicle> = std::iter::once(player)
            .chain(self.opponents.iter_mut().map(|o| &mut o.vehicle))
            .chain(self.traffic.cars.iter_mut().map(|t| &mut t.vehicle))
            .collect();
        collide(&mut bodies);
    }

    /// Wheel spin and brake lights, once per frame.
    pub fn animate(&mut self, models: &HashMap<String, ModelPhysics>, dt: f32) {
        let k = 1.0 - (-25.0 * dt).exp();
        for o in &mut self.opponents {
            let r = models.get(&o.model).map_or(0.33, |m| m.wheel_radius);
            o.vehicle.advance_wheels(r, dt);
            let on = (o.controls.brake > 0.05 && o.vehicle.speed() > 0.5) as u8 as f32;
            o.brake_light += (on - o.brake_light) * k;
        }
        for (t, c) in self.traffic.cars.iter_mut().zip(&self.traffic_controls) {
            let r = models.get(&t.model).map_or(0.33, |m| m.wheel_radius);
            t.vehicle.advance_wheels(r, dt);
            let on = (c.brake > 0.05 || t.vehicle.speed() < 0.5) as u8 as f32;
            t.brake_light += (on - t.brake_light) * k;
        }
    }
}

/// `count` points of the race line from point `from` on, `lane` metres to the left
/// (kept inside the road): the path an AI driver follows.
pub fn line_path(line: &RaceLine, from: usize, lane: f32, count: usize) -> Vec<Vec3> {
    let n = line.points.len();
    let mut path = Vec::with_capacity(count);
    for j in 0..count {
        let i = from + j;
        if !line.circuit && i >= n {
            break;
        }
        let (a, b) = (line.points[i % n], line.points[(i + 1) % n]);
        let d = (b - a).truncate().normalize_or_zero();
        let left = Vec3::new(-d.y, d.x, 0.0);
        let room = (line.half_width[i % n] - 1.5).max(0.0);
        path.push(a + left * lane.clamp(-room, room));
    }
    path
}

/// Two circles per car (front and rear axle area); overlapping cars are pushed apart and
/// exchange momentum along the contact normal.
fn collide(bodies: &mut [&mut Vehicle]) {
    const R: f32 = 1.0;
    const AXLE: f32 = 1.25;
    for i in 0..bodies.len() {
        let (head, tail) = bodies.split_at_mut(i + 1);
        let a = &mut head[i];
        for b in tail.iter_mut() {
            if a.pos.distance_squared(b.pos) > 36.0 || (a.pos.z - b.pos.z).abs() > 2.5 {
                continue;
            }
            let (fa, fb) = (a.forward().truncate(), b.forward().truncate());
            let mut push = Vec2::ZERO;
            let mut normal = Vec2::ZERO;
            for sa in [-AXLE, AXLE] {
                for sb in [-AXLE, AXLE] {
                    let ca = a.pos.truncate() + fa * sa;
                    let cb = b.pos.truncate() + fb * sb;
                    let d = ca - cb;
                    let dist = d.length();
                    if dist < 2.0 * R && dist > 1e-4 {
                        let n = d / dist;
                        push += n * (2.0 * R - dist);
                        normal += n;
                    }
                }
            }
            if normal == Vec2::ZERO {
                continue;
            }
            let n = normal.normalize();
            // Heavier cars move less: split corrections and impulses by inverse mass.
            let (ia, ib) = (1.0 / a.perf.mass.max(0.3), 1.0 / b.perf.mass.max(0.3));
            let (wa, wb) = (ia / (ia + ib), ib / (ia + ib));
            let corr = push.length().min(1.0);
            a.pos += (n * corr * wa).extend(0.0);
            b.pos -= (n * corr * wb).extend(0.0);
            let rel = (a.vel - b.vel).truncate().dot(n);
            if rel < 0.0 {
                let j = -(1.0 + 0.25) * rel / (ia + ib);
                a.vel += (n * j * ia).extend(0.0);
                b.vel -= (n * j * ib).extend(0.0);
                a.impact = a.impact.max(-rel);
                b.impact = b.impact.max(-rel);
                // A glancing hit also turns the cars a little.
                let side_a = fa.perp_dot(n);
                let side_b = fb.perp_dot(n);
                a.yaw_rate += side_a * j * ia * 0.08;
                b.yaw_rate -= side_b * j * ib * 0.08;
            }
        }
    }
}

/// Wheel transforms for drawing a car.
pub fn wheel_transforms(v: &Vehicle, car: Mat4, wheels: &[WheelInfo; 4]) -> [Mat4; 4] {
    let mut out = [Mat4::IDENTITY; 4];
    for (i, w) in wheels.iter().enumerate() {
        let p = Vec3::from(w.position);
        let steer = if p.x > 0.0 { v.steer * 0.6 } else { 0.0 };
        // The wheel model faces the right side (-Y); mirror it for the left wheels.
        let mirror = if p.y > 0.0 { Vec3::new(1.0, -1.0, 1.0) } else { Vec3::ONE };
        out[i] = car
            * Mat4::from_translation(p)
            * Mat4::from_quat(Quat::from_rotation_z(steer))
            * Mat4::from_scale(mirror)
            * Mat4::from_quat(Quat::from_rotation_y(v.wheel_spin));
    }
    out
}
