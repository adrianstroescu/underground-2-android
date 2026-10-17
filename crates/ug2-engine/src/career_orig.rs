//! The original career's progress rules (speed2.exe), on the career data of
//! `ug2_formats::career`:
//!
//! * open races (0x527d30): per category, while fewer races are open than the stage's
//!   cap, a random not-won race of the stage joins (0x527bb0); hidden races up to the
//!   hidden cap; URL races once their win thresholds are met (0x513510);
//! * a win (0x541420) pays the race's prize (no money for losing or for places), counts
//!   as a regular, sponsor or URL win of the stage (0x513390) and may advance the stage
//!   (0x532c60): the stage final won (stage 1: all its races) → next stage.
//!
//! Sponsors, DVD covers and the story's drive-to events (stage 0) are not done yet: no
//! sponsor races open, covers count as done, and a career starts at stage 1.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ug2_formats::career::{CareerData, Race};

/// S5_CIRCUIT_36, the last race; once its prerequisite is won it is the only one shown.
const LAST_RACE: u32 = 0x04cc_e3e1;

#[derive(Clone, Debug)]
pub struct Progress {
    pub stage: u8,
    pub won: BTreeSet<u32>,
    /// Wins per stage: regular, sponsor, URL.
    pub wins: BTreeMap<u8, [u16; 3]>,
    /// Races open now, in the order they opened.
    pub open: Vec<u32>,
    rng: u32,
    path: PathBuf,
}

/// What a win changed.
#[derive(Default, Debug)]
pub struct WinOutcome {
    pub prize: u32,
    /// The stage advanced to this one.
    pub new_stage: Option<u8>,
}

impl Progress {
    pub fn load(path: &Path) -> Self {
        let mut p = Progress { stage: 1, won: BTreeSet::new(), wins: BTreeMap::new(), open: Vec::new(), rng: 0x1d87_2b41, path: path.to_path_buf() };
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                let (k, v) = (k.trim(), v.trim());
                let nums = || v.split(',').filter_map(|x| u32::from_str_radix(x.trim().trim_start_matches("0x"), 16).ok());
                match k {
                    "stage" => p.stage = v.parse().unwrap_or(1),
                    "won" => p.won = nums().collect(),
                    "open" => p.open = nums().collect(),
                    "rng" => p.rng = v.parse().unwrap_or(p.rng),
                    _ if k.starts_with("wins.") => {
                        if let Ok(s) = k["wins.".len()..].parse::<u8>() {
                            let w: Vec<u16> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                            if w.len() == 3 {
                                p.wins.insert(s, [w[0], w[1], w[2]]);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        p
    }

    pub fn save(&self) {
        let hex = |s: &mut dyn Iterator<Item = &u32>| s.map(|h| format!("{h:08x}")).collect::<Vec<_>>().join(",");
        let mut text = format!(
            "# Underground 2 career progress (original rules)\nstage = {}\nwon = {}\nopen = {}\nrng = {}\n",
            self.stage,
            hex(&mut self.won.iter()),
            hex(&mut self.open.iter()),
            self.rng
        );
        for (s, w) in &self.wins {
            text.push_str(&format!("wins.{s} = {}, {}, {}\n", w[0], w[1], w[2]));
        }
        if let Err(e) = std::fs::write(&self.path, text) {
            log::warn!("cannot write {}: {e}", self.path.display());
        }
    }

    /// The game's random numbers (0x43c1c0): r = seed % n, then the seed scrambles.
    fn rand(&mut self, n: u32) -> u32 {
        let r = if n == 0 { 0 } else { self.rng % n };
        let s = self.rng ^ 0x1d87_2b41;
        let t = s ^ (s >> 5);
        self.rng = t ^ (t << 27) ^ s;
        r
    }

    pub fn wins(&self, stage: u8) -> [u16; 3] {
        self.wins.get(&stage).copied().unwrap_or_default()
    }

    /// Stages unlocked so far (1-based, as the world map's scroller shows them).
    pub fn stages(&self) -> Vec<u8> {
        (1..=self.stage.max(1)).collect()
    }

    /// Category of a pool race (0x513600): 0..4 regular circuit/drag/Street X/drift/sprint,
    /// 7 SUV; None for the others.
    fn category(r: &Race) -> Option<usize> {
        if r.kind != 3 {
            return None;
        }
        if r.suv {
            return Some(7);
        }
        Some(match r.race_type {
            0 => 0,
            4 => 1,
            2 => 2,
            5 => 3,
            1 => 4,
            _ => return None,
        })
    }

    /// Rebuilds the open races (0x527d30). `suv`: the player drives an SUV.
    pub fn rebuild(&mut self, data: &CareerData, suv: bool) {
        let stage = self.stage;
        // The last race alone once its prerequisite is won.
        if let Some(last) = data.race(LAST_RACE) {
            if self.won.contains(&last.prerequisite) && !self.won.contains(&LAST_RACE) {
                self.open = vec![LAST_RACE];
                return;
            }
            if self.won.contains(&LAST_RACE) {
                self.open.clear();
                return;
            }
        }
        let Some(cfg) = data.stage(stage) else { return };
        let won = self.won.clone();
        // Keep the open races of this stage that are not won.
        self.open.retain(|h| data.race(*h).is_some_and(|r| r.stage == stage) && !won.contains(h));
        for cat in [0usize, 1, 2, 3, 4, 7] {
            if cat == 7 && !suv {
                continue;
            }
            let cap = cfg.caps[cat] as usize;
            loop {
                let open_in_cat = self.open.iter().filter(|h| data.race(**h).and_then(Self::category) == Some(cat)).count();
                if open_in_cat >= cap {
                    break;
                }
                let pool: Vec<u32> = data
                    .races
                    .iter()
                    .filter(|r| r.stage == stage && !r.hidden && Self::category(r) == Some(cat) && !won.contains(&r.hash) && !self.open.contains(&r.hash))
                    .map(|r| r.hash)
                    .collect();
                if pool.is_empty() {
                    break;
                }
                let k = self.rand(pool.len() as u32) as usize;
                self.open.push(pool[k]);
            }
        }
        // Hidden races, up to the stage's cap.
        if cfg.hidden_cap > 0 {
            for r in data.races.iter().filter(|r| r.stage == stage && r.hidden && !won.contains(&r.hash)) {
                let n = self.open.iter().filter(|h| data.race(**h).is_some_and(|x| x.hidden)).count();
                if n >= cfg.hidden_cap as usize {
                    break;
                }
                if !self.open.contains(&r.hash) {
                    self.open.push(r.hash);
                }
            }
        }
        // URL races once their thresholds are met (0x513510).
        let [regular, sponsor, url] = self.wins(stage);
        for r in data.races.iter().filter(|r| r.stage == stage && r.kind == 2 && !won.contains(&r.hash)) {
            let ok = match r.unlock {
                3 => regular >= r.url_regular_wins,
                4 => regular >= r.thresholds[0] as u16 && sponsor >= r.thresholds[1] as u16 && url >= r.thresholds[2] as u16,
                _ => true,
            };
            if ok && !self.open.contains(&r.hash) {
                self.open.push(r.hash);
            }
        }
    }

    /// A win: prize, counters, stage advance (0x541420, 0x513390, 0x532c60).
    pub fn on_win(&mut self, data: &CareerData, hash: u32, suv: bool) -> WinOutcome {
        let Some(r) = data.race(hash).cloned() else { return WinOutcome::default() };
        let mut out = WinOutcome { prize: r.prize, new_stage: None };
        if self.won.insert(hash) {
            let w = self.wins.entry(r.stage).or_default();
            match r.kind {
                3 if !r.suv => w[0] += 1,
                1 => w[1] += 1,
                2 => w[2] += 1,
                _ => {}
            }
        }
        // Stage advance.
        if let Some(cfg) = data.stage(self.stage) {
            let done = if cfg.final_race != 0 {
                self.won.contains(&cfg.final_race)
            } else {
                // Stage 1 has no final: every race of the stage (0x5130a0).
                data.races.iter().filter(|x| x.stage == self.stage).all(|x| self.won.contains(&x.hash))
            };
            if done && data.stage(self.stage + 1).is_some() {
                self.stage += 1;
                self.open.clear();
                out.new_stage = Some(self.stage);
            }
        }
        self.rebuild(data, suv);
        self.save();
        out
    }
}
