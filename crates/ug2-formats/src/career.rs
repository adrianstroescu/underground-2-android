//! Career data (GLOBALB gameplay container 0x80034A10), as speed2.exe reads it
//! (loader 0x512c00). Names are u16 offsets into the string pool 0x34A1D.
//!
//! * 0x34A11 races, 136 bytes: +0 name, +2 / +4 movies before / after a win, +8 hash,
//!   +0xC unlock (1 pool race, 3 URL after N regular wins, 4 stage final), +0xD SUV,
//!   +0xF type, +0x10 prerequisite hash, +0x11..+0x13 final's win thresholds, +0x18
//!   heats {u16 track, u8 flags (bit 0 reverse), u8 laps} × +0x7E, +0x30 prize, +0x34
//!   kind (0 story, 1 sponsor, 2 URL, 3 regular), +0x37 stage, +0x40 opponents × +0x7C
//!   (12 bytes: u16 driver name, u8 mode, u8 level, u32 car hash, u8 skill[3]),
//!   +0x7F hidden.
//! * 0x34A18 stage configs, 80 bytes: +2 outrun stake, +8 sponsors[5], +0x28 final race
//!   hash, +0x30.. category caps, +0x41 hidden cap, +0x42 / +0x44 / +0x48 AI ramp.

use crate::bytes::{f32_at, u16_at, u32_at};
use crate::chunk::ChunkIter;
use crate::{bail, Result};

const GAMEPLAY: u32 = 0x8003_4A10;
const RACES: u32 = 0x0003_4A11;
const STAGES: u32 = 0x0003_4A18;
const POOL: u32 = 0x0003_4A1D;

/// bin_hash("RANDOM"): an opponent in a random AI car.
pub const RANDOM_CAR: u32 = 0x7710_f0e0;

#[derive(Clone, Debug)]
pub struct Heat {
    pub track: u16,
    pub reverse: bool,
    pub laps: u8,
}

#[derive(Clone, Debug)]
pub struct Opponent {
    pub name: String,
    /// 0 fixed level, 1 matched to the player, 2 stock, 3 ramped with the stage's wins.
    pub mode: u8,
    pub level: u8,
    /// Car preset hash, or [`RANDOM_CAR`].
    pub car: u32,
    /// Driver skill per AI-skill setting (Easy / Medium / Hard).
    pub skill: [u8; 3],
}

#[derive(Clone, Debug)]
pub struct Race {
    pub name: String,
    pub hash: u32,
    pub movie_before: Option<String>,
    pub movie_after: Option<String>,
    /// +0xC: 0 prerequisite (+0x10), 1 pool race, 3 URL, 4 stage final.
    pub unlock: u8,
    pub suv: bool,
    /// 0 circuit, 1 sprint, 2 Street X, 3 drive-to, 4 drag, 5 drift.
    pub race_type: u8,
    pub prerequisite: u32,
    /// Stage final: regular, sponsor, URL wins needed. URL race (unlock 3): regular wins
    /// needed (u16 at +0x12).
    pub thresholds: [u8; 3],
    pub url_regular_wins: u16,
    pub heats: Vec<Heat>,
    pub prize: u32,
    /// 0 story, 1 sponsor, 2 URL, 3 regular.
    pub kind: u8,
    pub stage: u8,
    pub opponents: Vec<Opponent>,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Stage {
    pub stage: u8,
    pub outrun_stake: u16,
    pub sponsors: Vec<u32>,
    /// Final race hash (0: every race of the stage, stage 1).
    pub final_race: u32,
    /// Open races per category: circuit, drag, Street X, drift, sprint, _, _, SUV
    /// (+0x30..+0x37).
    pub caps: [u8; 8],
    /// Hidden races open at a time (−1: none).
    pub hidden_cap: i8,
    /// AI level ramp: wins for the full ramp, level at the start and end.
    pub ramp_wins: i16,
    pub ramp: [f32; 2],
}

#[derive(Clone, Debug, Default)]
pub struct CareerData {
    pub races: Vec<Race>,
    pub stages: Vec<Stage>,
}

impl CareerData {
    pub fn race(&self, hash: u32) -> Option<&Race> {
        self.races.iter().find(|r| r.hash == hash)
    }

    pub fn stage(&self, s: u8) -> Option<&Stage> {
        self.stages.iter().find(|x| x.stage == s)
    }
}

pub fn parse(global_b: &[u8]) -> Result<CareerData> {
    let Some(gameplay) = ChunkIter::new(global_b).find(|c| c.id == GAMEPLAY) else { bail!("no gameplay data") };
    let pool = gameplay.child(POOL).map(|c| c.data).unwrap_or(&[]);
    let text = |off: u16| -> Option<String> {
        let rest = pool.get(off as usize..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        (end > 0).then(|| String::from_utf8_lossy(&rest[..end]).into_owned())
    };
    let mut out = CareerData::default();
    if let Some(c) = gameplay.child(RACES) {
        for r in c.data.chunks_exact(136) {
            let off = |o: usize| u16::from_le_bytes([r[o], r[o + 1]]);
            let heats = (0..(r[0x7E] as usize).min(4))
                .map(|k| {
                    let o = 0x18 + k * 4;
                    Heat { track: off(o), reverse: r[o + 2] & 1 != 0, laps: r[o + 3] }
                })
                .collect();
            let opponents = (0..(r[0x7C] as usize).min(5))
                .filter_map(|k| {
                    let o = 0x40 + k * 12;
                    Some(Opponent {
                        name: text(off(o)).unwrap_or_default(),
                        mode: r[o + 2],
                        level: r[o + 3],
                        car: u32_at(r, o + 4).ok()?,
                        skill: [r[o + 8], r[o + 9], r[o + 10]],
                    })
                })
                .collect();
            out.races.push(Race {
                name: text(off(0)).unwrap_or_default(),
                hash: u32_at(r, 8)?,
                movie_before: if off(2) != 0 { text(off(2)) } else { None },
                movie_after: if off(4) != 0 { text(off(4)) } else { None },
                unlock: r[0xC],
                suv: r[0xD] != 0,
                race_type: r[0xF],
                prerequisite: u32_at(r, 0x10)?,
                thresholds: [r[0x11], r[0x12], r[0x13]],
                url_regular_wins: u16_at(r, 0x12)?,
                heats,
                prize: u32_at(r, 0x30)?,
                kind: r[0x34],
                stage: r[0x37],
                opponents,
                hidden: r[0x7F] == 1,
            });
        }
    }
    if let Some(c) = gameplay.child(STAGES) {
        for s in c.data.chunks_exact(80) {
            out.stages.push(Stage {
                stage: s[0],
                outrun_stake: u16_at(s, 2)?,
                sponsors: (0..(s[1] as usize).min(5)).filter_map(|k| u32_at(s, 8 + k * 4).ok()).collect(),
                final_race: u32_at(s, 0x28)?,
                caps: [s[0x30], s[0x31], s[0x32], s[0x33], s[0x34], s[0x35], s[0x36], s[0x37]],
                hidden_cap: s[0x41] as i8,
                ramp_wins: u16_at(s, 0x42)? as i16,
                ramp: [f32_at(s, 0x44)?, f32_at(s, 0x48)?],
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn career_data() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(gb) = game.read("GLOBAL/GLOBALB.BUN") else { return };
        let c = super::parse(&gb).unwrap();
        assert_eq!(c.races.len(), 224);
        assert_eq!(c.stages.len(), 6);
        let s2 = c.stage(2).unwrap();
        assert_eq!(c.race(s2.final_race).map(|r| r.name.as_str()), Some("S2_URL_3"));
        let f = c.races.iter().find(|r| r.name == "S2_URL_3").unwrap();
        assert_eq!(f.thresholds, [10, 3, 2]);
        for s in &c.stages {
            println!("stage {} caps {:?} hidden {} ramp {:?} {:?} final {:?}", s.stage, s.caps, s.hidden_cap, s.ramp_wins, s.ramp, c.race(s.final_race).map(|r| &r.name));
        }
        let r = c.races.iter().find(|r| r.name == "STAGE_1_DRAG_1").unwrap();
        println!("{r:?}");
    }
}
