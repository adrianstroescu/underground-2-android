//! Game audio: a software mixer the platform pulls from its audio callback (AAudio on
//! Android), mixing to 48 kHz interleaved stereo f32. The game thread only swaps in
//! loaded sounds and per-frame state; the callback does the work.
//!
//! The player's engine is granular, like the original: a GIN file is one recording of
//! the engine sweeping from idle to redline, cut into short grains. Playing the grain
//! recorded at the current RPM over and over (crossfading between grains) gives the
//! engine note at that RPM; an off-throttle sweep is blended in when lifting.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ug2_formats::audio::{car_sounds, parse_abk, Gin};
use ug2_formats::fs::GameDir;

pub const RATE: u32 = 48_000;

/// Mono PCM at its own sample rate.
#[derive(Clone)]
pub struct Clip {
    pub rate: f32,
    pub pcm: Arc<Vec<i16>>,
    /// Loop range in samples (inclusive end).
    pub looping: Option<(usize, usize)>,
}

fn load_bank(game: &GameDir, rel: &str) -> Vec<Clip> {
    let Ok(b) = game.read(rel) else {
        log::warn!("missing {rel}");
        return Vec::new();
    };
    match parse_abk(&b) {
        Ok(sounds) => sounds
            .iter()
            .filter_map(|s| {
                let pcm = s.decode(&b).ok()?;
                let looping = s.looping.map(|(a, e)| (a as usize, e as usize));
                Some(Clip { rate: s.sample_rate as f32, pcm: Arc::new(pcm), looping })
            })
            .collect(),
        Err(e) => {
            log::warn!("{rel}: {e}");
            Vec::new()
        }
    }
}

/// First looping sound of a bank, or its first sound looped whole.
fn load_loop(game: &GameDir, rel: &str) -> Option<Clip> {
    let bank = load_bank(game, rel);
    bank.iter().find(|c| c.looping.is_some()).cloned().or_else(|| {
        let mut c = bank.into_iter().next()?;
        c.looping = Some((0, c.pcm.len().saturating_sub(1)));
        Some(c)
    })
}

/// A car's engine and drivetrain sounds.
pub struct EngineSound {
    pub accel: Arc<Gin>,
    pub decel: Option<Arc<Gin>>,
    pub idle_rpm: f32,
    pub max_rpm: f32,
    /// Quiet and loud levels, 0..1.
    pub volume: (f32, f32),
    pub shifts: Vec<Clip>,
}

impl EngineSound {
    /// Stock engine of a racer (`None` for traffic models).
    pub fn load(game: &GameDir, car: &str) -> Option<Self> {
        let (engine, gear, _turbo) = car_sounds(car, 0)?;
        let gin = |name: &str| {
            let b = game.read(&format!("SOUND/ENGINE/{name}")).ok()?;
            Gin::parse(&b).map_err(|e| log::warn!("{name}: {e}")).ok().map(Arc::new)
        };
        Some(Self {
            accel: gin(engine.gin)?,
            decel: gin(engine.decel_gin),
            idle_rpm: engine.idle_rpm,
            max_rpm: engine.max_rpm,
            volume: (engine.volume.0 as f32 / 32767.0, engine.volume.1 as f32 / 32767.0),
            shifts: load_bank(game, &format!("SOUND/SHIFTING/{gear}")),
        })
    }
}

/// Effects shared by every car.
#[derive(Default)]
pub struct Effects {
    pub nitro: Option<Clip>,
    pub skid: Option<Clip>,
    pub wind: Option<Clip>,
    pub road: Option<Clip>,
    pub impacts: Vec<Clip>,
}

impl Effects {
    pub fn load(game: &GameDir) -> Self {
        Self {
            nitro: load_loop(game, "SOUND/NOS/Nitrous_00_MB.abk"),
            skid: load_loop(game, "SOUND/SKIDS/SKID_PAV_MB.abk"),
            wind: load_loop(game, "SOUND/IG_GLOBAL/WIND_00_MB.abk"),
            road: load_loop(game, "SOUND/IG_GLOBAL/ROADNOISE_00_MB.abk"),
            impacts: load_bank(game, "SOUND/IG_GLOBAL/MB_Stich_Collision.abk"),
        }
    }
}

/// What the game tells the mixer every frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct SoundState {
    /// Engine speed from the vehicle model, 0..1 across the rev range.
    pub rpm: f32,
    pub throttle: f32,
    /// Road speed, m/s.
    pub speed: f32,
    pub nitro: bool,
    /// Tyre squeal, 0..1.
    pub skid: f32,
    /// False while paused or loading: the engine idles quietly and effects fade out.
    pub active: bool,
}

fn sample_at(pcm: &[i16], pos: f32) -> f32 {
    let i = pos as usize;
    let Some(&a) = pcm.get(i) else { return 0.0 };
    let b = pcm.get(i + 1).copied().unwrap_or(a);
    let f = pos - i as f32;
    (a as f32 + (b as f32 - a as f32) * f) * (1.0 / 32768.0)
}

struct Voice {
    clip: Clip,
    pos: f32,
    gain: f32,
}

impl Voice {
    fn next(&mut self) -> Option<f32> {
        if self.pos as usize + 1 >= self.clip.pcm.len() {
            return None;
        }
        let s = sample_at(&self.clip.pcm, self.pos) * self.gain;
        self.pos += self.clip.rate / RATE as f32;
        Some(s)
    }
}

#[derive(Default)]
struct LoopVoice {
    clip: Option<Clip>,
    pos: f32,
    gain: f32,
    target: f32,
}

impl LoopVoice {
    fn next(&mut self) -> f32 {
        self.gain += (self.target - self.gain) * 0.0006;
        let Some(c) = &self.clip else { return 0.0 };
        if self.gain < 1e-4 {
            return 0.0;
        }
        let (start, end) = c.looping.unwrap_or((0, c.pcm.len().saturating_sub(1)));
        let s = sample_at(&c.pcm, self.pos);
        self.pos += c.rate / RATE as f32;
        if self.pos >= end as f32 {
            self.pos -= (end - start).max(1) as f32;
        }
        s * self.gain
    }
}

/// Plays grains from one GIN sweep, crossfading each into the next.
struct GrainVoice {
    gin: Arc<Gin>,
    cur: (usize, f32),
    prev: Option<(usize, f32)>,
    fade: f32,
}

impl GrainVoice {
    fn new(gin: Arc<Gin>) -> Self {
        Self { gin, cur: (0, 0.0), prev: None, fade: 1.0 }
    }

    fn sample(&mut self, rpm: f32, rng: &mut u32) -> f32 {
        let step = self.gin.sample_rate as f32 / RATE as f32;
        let xfade = (0.005 * self.gin.sample_rate as f32).max(16.0);
        let len = self.gin.grain(self.cur.0).len() as f32;
        if self.cur.1 >= len - xfade && self.prev.is_none() {
            let mut g = self.gin.grain_at(self.gin.position(rpm));
            // A little jitter keeps a held RPM from sounding like a buzzing loop.
            *rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if *rng >> 30 == 0 {
                g = (g + 1).min(self.gin.grains.len().saturating_sub(2));
            }
            self.prev = Some(self.cur);
            self.cur = (g, 0.0);
            self.fade = 0.0;
        }
        let mut out = sample_at(self.gin.grain(self.cur.0), self.cur.1);
        self.cur.1 += step;
        if let Some(p) = &mut self.prev {
            let old = sample_at(self.gin.grain(p.0), p.1);
            p.1 += step;
            self.fade = (self.fade + step / xfade).min(1.0);
            out = out * self.fade + old * (1.0 - self.fade);
            if self.fade >= 1.0 {
                self.prev = None;
            }
        }
        out
    }
}

struct EngineVoice {
    accel: GrainVoice,
    decel: Option<GrainVoice>,
    idle_rpm: f32,
    max_rpm: f32,
    volume: (f32, f32),
    rpm: f32,
    load: f32,
}

pub struct Mixer {
    state: SoundState,
    engine: Option<EngineVoice>,
    shifts: Vec<Clip>,
    fx: Effects,
    nitro: LoopVoice,
    skid: LoopVoice,
    wind: LoopVoice,
    road: LoopVoice,
    shots: Vec<Voice>,
    music: Option<Arc<Mutex<VecDeque<f32>>>>,
    pub music_volume: f32,
    pub sfx_volume: f32,
    rng: u32,
    /// Loudest output sample since the game last read it (diagnostics).
    pub peak: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            state: SoundState::default(),
            engine: None,
            shifts: Vec::new(),
            fx: Effects::default(),
            nitro: LoopVoice::default(),
            skid: LoopVoice::default(),
            wind: LoopVoice::default(),
            road: LoopVoice::default(),
            shots: Vec::new(),
            music: None,
            music_volume: 0.55,
            sfx_volume: 0.9,
            rng: 0x1234_5678,
            peak: 0.0,
        }
    }
}

impl Mixer {
    pub fn set_engine(&mut self, e: &EngineSound) {
        self.engine = Some(EngineVoice {
            accel: GrainVoice::new(e.accel.clone()),
            decel: e.decel.clone().map(GrainVoice::new),
            idle_rpm: e.idle_rpm,
            max_rpm: e.max_rpm,
            volume: e.volume,
            rpm: e.idle_rpm,
            load: 0.0,
        });
        self.shifts = e.shifts.clone();
    }

    pub fn set_effects(&mut self, fx: Effects) {
        self.nitro.clip = fx.nitro.clone();
        self.skid.clip = fx.skid.clone();
        self.wind.clip = fx.wind.clone();
        self.road.clip = fx.road.clone();
        self.fx = fx;
    }

    pub fn set_music(&mut self, ring: Arc<Mutex<VecDeque<f32>>>) {
        self.music = Some(ring);
    }

    pub fn set_state(&mut self, s: SoundState) {
        self.state = s;
        let on = if s.active { 1.0 } else { 0.0 };
        self.nitro.target = if s.nitro { 0.7 * on } else { 0.0 };
        self.skid.target = s.skid.clamp(0.0, 1.0) * 0.55 * on;
        let v = (s.speed / 70.0).clamp(0.0, 1.0);
        self.wind.target = v * v * 0.45 * on;
        self.road.target = v.sqrt() * 0.3 * on;
    }

    fn rand(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.rng >> 8
    }

    pub fn shift(&mut self) {
        if self.shifts.is_empty() {
            return;
        }
        let i = self.rand() as usize % self.shifts.len();
        self.shots.push(Voice { clip: self.shifts[i].clone(), pos: 0.0, gain: 0.6 });
    }

    /// A hit, `strength` 0..1.
    pub fn impact(&mut self, strength: f32) {
        if self.fx.impacts.is_empty() || self.shots.len() > 8 {
            return;
        }
        let i = self.rand() as usize % self.fx.impacts.len();
        let clip = self.fx.impacts[i].clone();
        self.shots.push(Voice { clip, pos: 0.0, gain: 0.35 + 0.65 * strength.clamp(0.0, 1.0) });
    }

    /// Fills interleaved stereo frames.
    pub fn render(&mut self, out: &mut [f32]) {
        let s = self.state;
        let mut music = self.music.as_ref().and_then(|m| m.lock().ok());
        let mut rng = self.rng;
        for frame in out.chunks_exact_mut(2) {
            let mut fx = 0.0;
            if let Some(e) = &mut self.engine {
                let t = ((s.rpm - 0.18) / 0.82).clamp(0.0, 1.0);
                let target_rpm = if s.active { e.idle_rpm + (e.max_rpm - e.idle_rpm) * t } else { e.idle_rpm };
                e.rpm += (target_rpm - e.rpm) * 0.0015;
                let load = if s.active { s.throttle } else { 0.0 };
                e.load += (load - e.load) * 0.0008;
                let r = ((e.rpm - e.idle_rpm) / (e.max_rpm - e.idle_rpm).max(1.0)).clamp(0.0, 1.0);
                let vol = (e.volume.0 + (e.volume.1 - e.volume.0) * (0.5 * e.load + 0.5 * r)) * if s.active { 0.8 } else { 0.35 };
                let a = e.accel.sample(e.rpm, &mut rng);
                fx += match &mut e.decel {
                    Some(d) => {
                        let b = d.sample(e.rpm, &mut rng);
                        (a * (0.3 + 0.7 * e.load) + b * (1.0 - e.load) * 0.8) * vol
                    }
                    None => a * (0.45 + 0.55 * e.load) * vol,
                };
            }
            fx += self.nitro.next() + self.skid.next() + self.wind.next() + self.road.next();
            self.shots.retain_mut(|v| match v.next() {
                Some(x) => {
                    fx += x;
                    true
                }
                None => false,
            });
            fx *= self.sfx_volume;
            let (mut l, mut r) = (fx, fx);
            if let Some(m) = &mut music {
                if m.len() >= 2 {
                    l += m.pop_front().unwrap_or(0.0) * self.music_volume;
                    r += m.pop_front().unwrap_or(0.0) * self.music_volume;
                }
            }
            frame[0] = soft_clip(l);
            frame[1] = soft_clip(r);
            self.peak = self.peak.max(frame[0].abs()).max(frame[1].abs());
        }
        self.rng = rng;
    }
}

/// Gentle saturation instead of hard clipping.
fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

/// Handle the game keeps; the platform audio callback locks [`Sound::mixer`].
#[derive(Clone, Default)]
pub struct Sound {
    mixer: Arc<Mutex<Mixer>>,
}

impl Sound {
    pub fn mixer(&self) -> Arc<Mutex<Mixer>> {
        self.mixer.clone()
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut Mixer) -> R) -> Option<R> {
        self.mixer.lock().ok().map(|mut m| f(&mut m))
    }
}
