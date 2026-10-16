//! A race in progress: countdown, lap counting and standings from each car's distance
//! along the race line, wrong-way detection and the finish. Drift events rank the
//! player's score against rival scores instead.

use glam::Vec3;

use crate::drift::DriftScore;

use crate::roads::RaceDef;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RacePhase {
    /// Seconds until the start.
    Countdown(f32),
    Running,
    /// The player has finished; the race keeps running for the others.
    Finished,
}

#[derive(Clone, Debug)]
pub struct Progress {
    /// Nearest race line point.
    pub index: usize,
    /// Completed laps (circuits); -1 on the grid behind the start line.
    pub lap: i32,
    /// Metres covered since the start line.
    pub total: f32,
    pub finish_time: Option<f32>,
    last_dist: f32,
    best_total: f32,
    /// Seconds spent going backwards.
    pub wrong_way: f32,
    pub lap_start: f32,
    pub best_lap: Option<f32>,
    /// Out of the race (blown engine).
    pub retired: bool,
}

/// The scoring side of a drift event.
#[derive(Clone, Debug)]
pub struct DriftEvent {
    pub score: DriftScore,
    /// Rival names and the scores they set.
    pub rivals: Vec<(String, u32)>,
}

pub struct Race {
    pub def: RaceDef,
    pub laps: u32,
    pub phase: RacePhase,
    /// Seconds since the green light.
    pub time: f32,
    /// One entry per car: index 0 is the player, then the opponents.
    pub cars: Vec<Progress>,
    pub drift: Option<DriftEvent>,
}

impl Race {
    pub fn new(def: RaceDef, laps: u32, starts: &[Vec3]) -> Self {
        let laps = if def.info.mode.laps() { laps.max(1) } else { 1 };
        let cars = starts
            .iter()
            .map(|&p| {
                let index = def.line.locate(p, None);
                let d = def.line.dist[index];
                let behind = def.line.circuit && d > def.line.length * 0.5;
                Progress {
                    index,
                    lap: if behind { -1 } else { 0 },
                    total: if behind { d - def.line.length } else { d },
                    finish_time: None,
                    last_dist: d,
                    best_total: f32::MIN,
                    wrong_way: 0.0,
                    lap_start: 0.0,
                    best_lap: None,
                    retired: false,
                }
            })
            .collect();
        Self { def, laps, phase: RacePhase::Countdown(3.5), time: 0.0, cars, drift: None }
    }

    pub fn racing(&self) -> bool {
        !matches!(self.phase, RacePhase::Countdown(_))
    }

    /// Full race distance in metres.
    pub fn distance(&self) -> f32 {
        self.def.line.length * self.laps as f32
    }

    pub fn update(&mut self, positions: &[Vec3], dt: f32) {
        match &mut self.phase {
            RacePhase::Countdown(t) => {
                *t -= dt;
                if *t <= 0.0 {
                    self.phase = RacePhase::Running;
                }
                return;
            }
            _ => self.time += dt,
        }
        let line = &self.def.line;
        let full = self.distance();
        for (p, &pos) in self.cars.iter_mut().zip(positions) {
            if p.finish_time.is_some() || p.retired {
                continue;
            }
            p.index = line.locate(pos, Some(p.index));
            let d = line.dist[p.index];
            if line.circuit {
                if p.last_dist > line.length * 0.75 && d < line.length * 0.25 {
                    p.lap += 1;
                    if p.lap >= 1 {
                        let lap_time = self.time - p.lap_start;
                        p.best_lap = Some(p.best_lap.map_or(lap_time, |b: f32| b.min(lap_time)));
                    }
                    p.lap_start = self.time;
                } else if p.last_dist < line.length * 0.25 && d > line.length * 0.75 {
                    p.lap -= 1;
                }
            }
            p.last_dist = d;
            let total = p.lap.max(0) as f32 * line.length + if p.lap < 0 { d - line.length } else { d };
            if total < p.total - 2.0 {
                p.wrong_way += dt;
            } else if total > p.total + 0.5 {
                p.wrong_way = 0.0;
            }
            p.total = total;
            p.best_total = p.best_total.max(total);
            let done = if line.circuit { p.lap >= self.laps as i32 } else { d >= line.length - 6.0 };
            if done {
                p.finish_time = Some(self.time);
                p.total = full;
            }
        }
        if self.phase == RacePhase::Running && (self.cars[0].finish_time.is_some() || self.cars[0].retired) {
            self.phase = RacePhase::Finished;
            if let Some(d) = &mut self.drift {
                d.score.finish();
            }
        }
    }

    /// Takes car `i` out of the race.
    pub fn retire(&mut self, i: usize) {
        if let Some(p) = self.cars.get_mut(i) {
            p.retired = true;
        }
    }

    /// Drift events: (name, score, is the player) best first. Rivals' scores stand
    /// from the start; the player's counts what is banked.
    pub fn drift_standings(&self) -> Vec<(String, u32, bool)> {
        let Some(d) = &self.drift else { return Vec::new() };
        let mut rows: Vec<(String, u32, bool)> = d.rivals.iter().map(|(n, s)| (n.clone(), *s, false)).collect();
        rows.push(("YOU".into(), d.score.total, true));
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        rows
    }

    /// Car indices from first to last.
    pub fn standings(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.cars.len()).collect();
        order.sort_by(|&a, &b| {
            let (pa, pb) = (&self.cars[a], &self.cars[b]);
            if pa.retired != pb.retired {
                return pa.retired.cmp(&pb.retired);
            }
            match (pa.finish_time, pb.finish_time) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => pb.total.total_cmp(&pa.total),
            }
        });
        order
    }

    /// 1-based place of car `i` (drift events: the player among the rivals).
    pub fn place(&self, i: usize) -> usize {
        if self.drift.is_some() && i == 0 {
            return self.drift_standings().iter().position(|r| r.2).map_or(1, |p| p + 1);
        }
        self.standings().iter().position(|&c| c == i).map_or(self.cars.len(), |p| p + 1)
    }

    /// Entrants, rivals of a drift event included.
    pub fn entrants(&self) -> usize {
        self.drift.as_ref().map_or(self.cars.len(), |d| d.rivals.len() + 1)
    }

    /// Lap the player is on, 1-based, for the HUD.
    pub fn player_lap(&self) -> u32 {
        (self.cars[0].lap.max(0) as u32 + 1).min(self.laps)
    }

    /// Where to put a car back on the line at its current progress: position and yaw.
    pub fn reset_point(&self, car: usize) -> (Vec3, f32) {
        let line = &self.def.line;
        let n = line.points.len();
        let i = self.cars[car].index;
        let a = line.points[i];
        let b = line.points[(i + 1) % n];
        let d = (b - a).truncate();
        (a, d.y.atan2(d.x))
    }
}

/// "1:23.45"
pub fn format_time(t: f32) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0) as u32;
    let s = t - m as f32 * 60.0;
    format!("{m}:{s:05.2}")
}
