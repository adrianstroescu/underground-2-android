//! Career: a ladder of the original Bayview races, cash prizes, cars to buy and
//! performance upgrades, saved between sessions in `career.cfg`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::cars::tier;
use crate::roads::RaceDef;
use crate::vehicle::Perf;

/// Performance upgrade categories, 3 levels each.
pub const UPGRADES: [&str; 4] = ["ENGINE", "TURBO", "NITROUS", "TIRES"];
const START_CASH: u32 = 6000;

pub struct Career {
    path: PathBuf,
    pub cash: u32,
    pub owned: Vec<String>,
    pub current: String,
    /// Keys of races won (see [`race_key`]).
    pub won: HashSet<String>,
    pub upgrades: HashMap<String, [u8; 4]>,
}

/// Stable key for a race: track number and direction.
pub fn race_key(r: &RaceDef) -> String {
    format!("{}{}", r.track, if r.reverse { "R" } else { "F" })
}

impl Career {
    /// Loads the saved career, or starts one with `starter` (a class-1 car).
    pub fn load(path: &Path, starter: &str) -> Self {
        let mut c = Self {
            path: path.to_path_buf(),
            cash: START_CASH,
            owned: Vec::new(),
            current: String::new(),
            won: HashSet::new(),
            upgrades: HashMap::new(),
        };
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                let (k, v) = (k.trim(), v.trim());
                let list = || v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
                match k {
                    "cash" => c.cash = v.parse().unwrap_or(START_CASH),
                    "car" => c.current = v.to_string(),
                    "owned" => c.owned = list().collect(),
                    "won" => c.won = list().collect(),
                    _ => {
                        if let Some(car) = k.strip_prefix("upgrades.") {
                            let mut lv = [0u8; 4];
                            for (i, x) in v.split(',').take(4).enumerate() {
                                lv[i] = x.trim().parse::<u8>().unwrap_or(0).min(3);
                            }
                            c.upgrades.insert(car.to_string(), lv);
                        }
                    }
                }
            }
        }
        if c.owned.is_empty() {
            let car = if tier(starter) == 1 { starter } else { "PEUGOT" };
            c.owned.push(car.to_string());
        }
        if !c.owned.contains(&c.current) {
            c.current = c.owned[0].clone();
        }
        c
    }

    pub fn save(&self) {
        let mut s = format!(
            "# Underground 2 Android career\ncash = {}\ncar = {}\nowned = {}\nwon = {}\n",
            self.cash,
            self.current,
            self.owned.join(", "),
            self.won.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        for (car, lv) in &self.upgrades {
            s.push_str(&format!("upgrades.{car} = {}, {}, {}, {}\n", lv[0], lv[1], lv[2], lv[3]));
        }
        if let Err(e) = std::fs::write(&self.path, s) {
            log::warn!("cannot save career: {e}");
        }
    }

    pub fn car_price(car: &str) -> u32 {
        match (tier(car), car) {
            (_, "SKYLINE" | "SUPRA" | "RX7" | "LANCEREVO8" | "IMPREZAWRX") => 62_000,
            (1, _) => 8_000,
            (2, _) => 20_000,
            _ => 42_000,
        }
    }

    pub fn levels(&self, car: &str) -> [u8; 4] {
        self.upgrades.get(car).copied().unwrap_or_default()
    }

    /// Price of the next level of upgrade `kind` for `car`, if any is left.
    pub fn upgrade_price(&self, car: &str, kind: usize) -> Option<u32> {
        let level = self.levels(car)[kind];
        if level >= 3 {
            return None;
        }
        let base = [1_500, 4_000, 9_000][level as usize];
        Some(base * (3 + tier(car)) / 4)
    }

    pub fn buy_upgrade(&mut self, car: &str, kind: usize) -> bool {
        let Some(price) = self.upgrade_price(car, kind) else { return false };
        if self.cash < price {
            return false;
        }
        self.cash -= price;
        self.upgrades.entry(car.to_string()).or_default()[kind] += 1;
        self.save();
        true
    }

    pub fn buy_car(&mut self, car: &str) -> bool {
        let price = Self::car_price(car);
        if self.owned.iter().any(|c| c == car) || self.cash < price {
            return false;
        }
        self.cash -= price;
        self.owned.push(car.to_string());
        self.current = car.to_string();
        self.save();
        true
    }

    /// Stock performance `base` plus the player's upgrades.
    pub fn perf(&self, car: &str, base: Perf) -> Perf {
        let mut p = base;
        let lv = self.levels(car).map(|l| l as f32);
        p.power *= 1.0 + 0.07 * lv[0] + 0.05 * lv[1];
        p.top_speed += 1.5 * lv[0] + 2.0 * lv[1];
        p.nitro *= 1.0 + 0.3 * lv[2];
        p.grip *= 1.0 + 0.05 * lv[3];
        p
    }

    /// Engine sound level (0 = stock .. 3) from the engine upgrades.
    pub fn engine_level(&self, car: &str) -> usize {
        self.levels(car)[0] as usize
    }

    /// Career races in ladder order (cheapest prize first).
    pub fn ladder(races: &[RaceDef]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..races.len()).collect();
        order.sort_by_key(|&i| (races[i].reward, races[i].track, races[i].reverse));
        order
    }

    /// A race is open once every race two or more steps earlier on the ladder is won.
    pub fn unlocked(&self, races: &[RaceDef], ladder: &[usize], step: usize) -> bool {
        ladder.iter().take(step.saturating_sub(2)).all(|&i| self.won.contains(&race_key(&races[i])))
    }

    /// Prize for finishing `place` (1-based).
    pub fn prize(reward: u32, place: usize) -> u32 {
        match place {
            1 => reward,
            2 => reward / 2,
            3 => reward / 4,
            _ => 0,
        }
    }

    /// Opponent class and upgrade level for ladder step `step` of `n`.
    pub fn opponent_strength(step: usize, n: usize) -> (u32, f32) {
        let f = step as f32 / n.max(1) as f32;
        let class = if f < 0.33 { 1 } else if f < 0.66 { 2 } else { 3 };
        (class, f)
    }
}

/// Performance for an AI car at career progress `f` (0..1): stock plus a share of
/// the upgrades the player could have bought by then.
pub fn ai_perf(base: Perf, f: f32) -> Perf {
    let mut p = base;
    let lv = (f * 3.0).min(3.0);
    p.power *= 1.0 + 0.1 * lv;
    p.top_speed += 3.0 * lv;
    p.grip *= 1.0 + 0.04 * lv;
    p
}
