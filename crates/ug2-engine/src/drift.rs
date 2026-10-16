//! Drift events: points for sliding at speed, banked when the slide ends cleanly and
//! lost on a hard hit; long slides raise the multiplier for the next one. As in the
//! original, the rivals are scores to beat rather than cars on the track.

/// Slip angle (degrees) and speed (m/s) from which a slide counts.
const MIN_ANGLE: f32 = 12.0;
const MIN_SPEED: f32 = 8.0;
/// Seconds of driving straight that end a slide and bank its points.
const SETTLE: f32 = 0.7;
/// A multiplier lapses this long after the last banked slide.
const CHAIN: f32 = 3.0;
/// Wall or car hit (m/s into it) that loses the slide in progress.
const CRASH: f32 = 4.0;
const MAX_MULTIPLIER: u32 = 5;

#[derive(Clone, Debug)]
pub struct DriftScore {
    /// Banked points.
    pub total: u32,
    /// Points of the slide in progress, before the multiplier.
    pub current: f32,
    pub multiplier: u32,
    settle: f32,
    since_bank: f32,
    /// Last event for the HUD ("+1250", "CRASH") and seconds left to show it.
    pub flash: Option<(String, f32)>,
}

impl Default for DriftScore {
    fn default() -> Self {
        Self { total: 0, current: 0.0, multiplier: 1, settle: 0.0, since_bank: 0.0, flash: None }
    }
}

impl DriftScore {
    /// `angle`: slip angle in radians, `speed` in m/s, `impact`: hardest hit this frame.
    pub fn update(&mut self, angle: f32, speed: f32, impact: f32, dt: f32) {
        if let Some((_, t)) = &mut self.flash {
            *t -= dt;
            if *t <= 0.0 {
                self.flash = None;
            }
        }
        if impact > CRASH && self.current > 0.0 {
            self.current = 0.0;
            self.settle = 0.0;
            self.multiplier = 1;
            self.flash = Some(("CRASH".into(), 1.5));
            return;
        }
        let deg = angle.to_degrees();
        if speed > MIN_SPEED && deg > MIN_ANGLE && deg < 110.0 {
            self.settle = 0.0;
            // 80 km/h at 30 degrees: 100 points a second.
            self.current += speed * 3.6 * (deg.min(60.0) / MIN_ANGLE) * 0.5 * dt;
        } else if self.current > 0.0 {
            self.settle += dt;
            if self.settle > SETTLE {
                self.bank();
            }
        } else {
            self.since_bank += dt;
            if self.since_bank > CHAIN {
                self.multiplier = 1;
            }
        }
    }

    fn bank(&mut self) {
        let points = (self.current * self.multiplier as f32) as u32;
        self.total += points;
        self.flash = Some((format!("+{points}"), 1.5));
        if self.current > 250.0 {
            self.multiplier = (self.multiplier + 1).min(MAX_MULTIPLIER);
        }
        self.current = 0.0;
        self.settle = 0.0;
        self.since_bank = 0.0;
    }

    /// At the finish: a slide still in progress counts.
    pub fn finish(&mut self) {
        if self.current > 0.0 {
            self.bank();
        }
    }

    /// Points the slide in progress is worth with the multiplier.
    pub fn pending(&self) -> u32 {
        (self.current * self.multiplier as f32) as u32
    }
}

/// Scores of three rivals for a drift event of `laps` x `length` metres; `strength`
/// 0..1 scales them with career progress.
pub fn rival_scores(length: f32, laps: u32, strength: f32) -> [u32; 3] {
    let per_metre = 5.0 * (0.8 + 0.5 * strength.clamp(0.0, 1.0));
    let base = length * laps.max(1) as f32 * per_metre;
    let round = |x: f32| ((x / 10.0).round() * 10.0) as u32;
    [round(base * 1.25), round(base * 0.95), round(base * 0.7)]
}

/// "12,345"
pub fn format_points(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slides_bank_and_chain() {
        let mut d = DriftScore::default();
        let dt = 1.0 / 60.0;
        // Five seconds at 30 degrees and 80 km/h, then straight.
        for _ in 0..300 {
            d.update(30f32.to_radians(), 22.2, 0.0, dt);
        }
        assert!(d.current > 400.0 && d.total == 0, "{d:?}");
        for _ in 0..60 {
            d.update(0.0, 22.2, 0.0, dt);
        }
        assert!(d.total > 400 && d.current == 0.0 && d.multiplier == 2, "{d:?}");
        // A crash during the next slide loses it and the multiplier.
        for _ in 0..60 {
            d.update(30f32.to_radians(), 22.2, 0.0, dt);
        }
        let banked = d.total;
        d.update(30f32.to_radians(), 22.2, 8.0, dt);
        assert_eq!(d.total, banked);
        assert_eq!(d.multiplier, 1);
        assert_eq!(format_points(1234567), "1,234,567");
    }
}
