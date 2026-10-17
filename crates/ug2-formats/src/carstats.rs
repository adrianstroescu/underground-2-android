//! The car select stat bars (Acceleration, Top Speed, Handling) of a stock car, as
//! speed2.exe computes them: accel 0x5b53a0, top speed 0x5b6a20, handling 0x5b5490.
//!
//! * An acceleration-vs-speed curve (0x5b5ab0) is built from the torque curve (stock
//!   turbo included), gears and final drive, stepping 3 mph and shifting at
//!   redline − 100, then resampled to 64 points.
//! * A (0x59a350) integrates acceleration minus drag up to 100 mph; V (0x5b5930) is
//!   where drag meets the curve, capped by the limiter.
//! * Each value is normalised by a pair of the 30-float range table (GLOBALB chunk
//!   0x34a32, whose lower bounds are the stock extremes) and clamped to 0.03..1.
//!
//! Record offsets are into the 0x890-byte car type record (the game's physics info is
//! a copy of it from +0x110).

use crate::bytes::{f32_at, u32_at};
use crate::chunk::walk;

const RECORD_LEN: usize = 0x890;
/// 3 mph in m/s (0x7a1a80).
const DV: f32 = 1.341_081_9;
/// 60 / 2π (0x7a1d28).
const RPM_K: f32 = 9.549_304;
/// 100 mph.
const ACCEL_END: f32 = 44.702_728;
const MPH: f32 = 0.447_027_27;
/// Handling term weights (0x7a1a94).
const HANDLING_W: [f32; 9] = [0.14, 0.10, 0.15, 0.15, 0.10, 0.15, 0.15, 0.08, 0.08];

/// Piecewise linear curve (0x5764d0): index truncated, clamped at both ends.
struct Curve {
    y: Vec<f32>,
    x0: f32,
    inv: f32,
}

impl Curve {
    fn at(&self, x: f32) -> f32 {
        let t = (x - self.x0) * self.inv;
        let i = t as i32;
        if i < 0 || t < 0.0 {
            return self.y[0];
        }
        let i = i as usize;
        if i >= self.y.len() - 1 {
            return *self.y.last().unwrap();
        }
        let f = t - i as f32;
        (1.0 - f) * self.y[i] + f * self.y[i + 1]
    }
}

/// (name, [acceleration, top speed, handling]) of every car type, in record order.
pub fn car_ratings(global_b: &[u8]) -> Vec<(String, [f32; 3])> {
    let (mut types, mut range) = (None, None);
    walk(global_b, &mut |_, c| match c.id {
        0x0003_4600 if types.is_none() => types = Some(c.data),
        0x0003_4a32 if range.is_none() => range = Some(c.data),
        _ => {}
    });
    let (Some(types), Some(range)) = (types, range) else { return Vec::new() };
    let Ok(r) = (0..30).map(|i| f32_at(range, i * 4)).collect::<crate::Result<Vec<f32>>>() else { return Vec::new() };
    let start = types.len() % RECORD_LEN;
    types[start..]
        .chunks_exact(RECORD_LEN)
        .filter_map(|rec| {
            let name = crate::bytes::cstr_at(rec, 0, 32).ok()?;
            Some((name, ratings(rec, &r)?))
        })
        .collect()
}

fn ratings(rec: &[u8], r: &[f32]) -> Option<[f32; 3]> {
    let f = |o: usize| f32_at(rec, o).unwrap_or(0.0);
    let n = |x: f32, i: usize| (x - r[i]) / (r[i + 1] - r[i]);
    let clamp = |x: f32| if x <= 0.03 { 0.03 } else { x.min(1.0) };
    let mass = f(0x220);
    if mass <= 0.0 {
        return None;
    }
    let (curve, vend) = accel_curve(rec)?;
    let drag = f(0x370);
    // Nitrous (0 on stock cars) and the upgrade level at p+0x2e4 (0 stock).
    let nos = f(0x3bc) * f(0x3b0);
    let accel = 0.82 * n(accel_integral(&curve, mass, drag), 2) + 0.1 * n(nos, 8);
    let top = 0.915 * n(top_speed(&curve, vend, mass, drag, f(0x3a0)), 0) + 0.085 * n(nos, 8);
    let im = 1.0 / mass;
    let bias = f(0x2d0);
    let t = [
        n((f(0x210) + f(0x20c) + f(0x1f0) + f(0x1ec)) * 0.5 * im * 0.5, 14),
        n((f(0x388) * 0.35 + f(0x384) * 0.65) * im, 16),
        n(((f(0x2a4) * f(0x2a0) + f(0x2b0)) + (f(0x284) * f(0x280) + f(0x290))) * 0.5 * im * 0.5, 12),
        1.0 - n(mass, 18),
        if bias > 0.6 { (1.0 - bias) * 2.5 } else { (bias + 0.2) * 1.25 },
        1.0 - n(f(0x118), 20),
        1.0 - n(f(0x270), 22),
        n(f(0x374), 24),
        n(f(0x378), 26),
    ];
    let handling: f32 = t.iter().zip(HANDLING_W).map(|(x, w)| x * w).sum();
    Some([clamp(accel), clamp(top), clamp(handling)])
}

/// 0x5b5ab0: the most acceleration (m/s²) at each speed, resampled to 64 points, and
/// the speed the curve ends at.
fn accel_curve(rec: &[u8]) -> Option<(Curve, f32)> {
    let f = |o: usize| f32_at(rec, o).unwrap_or(0.0);
    let mass = f(0x220);
    let bias = f(0x2d0);
    let radius = bias * f(0x190) + (1.0 - bias) * f(0x130);
    let fd = (1.0 - bias) * f(0x2c8) + bias * f(0x2cc);
    let ng = u32_at(rec, 0x2d8).ok()? as usize;
    if ng == 0 || ng > 8 || radius <= 0.0 {
        return None;
    }
    // [reverse, neutral, 1st..].
    let gears: Vec<f32> = (0..ng + 2).map(|i| f(0x2e0 + 4 * i)).collect();
    let (idle, red, max) = (f(0x300), f(0x304), f(0x308));
    // Torque: 9 points from idle to max rpm, stock turbo added (0x5a6e30).
    let torque = Curve { y: (0..9).map(|k| f(0x310 + 4 * k) + f(0x334 + 4 * k)).collect(), x0: idle, inv: 8.0 / (max - idle) };
    // Shift points (0x5ac7d0): redline − 100, the top gear runs to max + 500.
    let shift = |gi: usize| if gi >= ng + 1 { max + 500.0 } else { red - 100.0 };
    let inv_r = 1.0 / radius;
    let (mut buf, mut v, mut gi) = (Vec::new(), 0.0f32, 2usize);
    loop {
        let rpm = inv_r * v * gears[gi] * fd * RPM_K;
        if rpm >= shift(gi) {
            gi += 1;
            if gi >= ng + 2 {
                break;
            }
            // The rpm is not recomputed for the new gear (as in the game).
        }
        buf.push(torque.at(rpm) * gears[gi] * fd * inv_r / mass);
        v += DV;
        if buf.len() >= 92 {
            break;
        }
    }
    let cnt = buf.len();
    if cnt < 2 {
        return None;
    }
    let vend = v;
    buf[cnt - 1] = 0.0;
    let tmp = Curve { y: buf, x0: 0.0, inv: (cnt - 1) as f32 / vend };
    let y = (0..64).map(|i| tmp.at(i as f32 * vend / 63.0)).collect();
    Some((Curve { y, x0: 0.0, inv: 63.0 / vend }, vend))
}

/// 0x59a350: acceleration minus drag integrated up to 100 mph. When drag wins first the
/// game returns the speed reached instead (kept as it is).
fn accel_integral(c: &Curve, mass: f32, drag: f32) -> f32 {
    let (mut v, mut s) = (0.0f32, 0.0f32);
    loop {
        let a = c.at(v);
        let d = v * drag * v / mass;
        if a < d {
            return v;
        }
        s += (a - d) * DV;
        v += DV;
        if v >= ACCEL_END {
            return s;
        }
    }
}

/// 0x5b5930: where drag meets the curve, capped by the limiter (mph).
fn top_speed(c: &Curve, vend: f32, mass: f32, drag: f32, limit_mph: f32) -> f32 {
    let mut v = 0.0f32;
    while v <= vend {
        if v * drag * v / mass > c.at(v) {
            break;
        }
        v += DV;
    }
    if limit_mph > 0.0 {
        v = v.min(limit_mph * MPH);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_ratings() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(gb) = game.read("GLOBAL/GLOBALB.BUN") else { return };
        let all = car_ratings(&gb);
        let get = |n: &str| all.iter().find(|(m, _)| m == n).map(|x| x.1).unwrap();
        // Values of the reference implementation of speed2.exe's code.
        for (car, want) in [("PEUGOT", [0.127, 0.030, 0.318]), ("350Z", [0.333, 0.275, 0.309]), ("3000GT", [0.303, 0.311, 0.327])] {
            let got = get(car);
            for k in 0..3 {
                assert!((got[k] - want[k]).abs() < 0.002, "{car} {got:?} {want:?}");
            }
        }
    }
}
