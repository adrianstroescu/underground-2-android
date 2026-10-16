//! Car type table (`CarTypeInfo`, chunk 0x00034600 in GLOBAL/GLOBALB): one 0x890-byte
//! record per car with names, file paths, wheel placement and handling data.

use crate::bytes::{cstr_at, f32_at, u32_at, vec3_at};
use crate::chunk::ChunkIter;
use crate::{bail, Result};

const CAR_TYPE_INFO: u32 = 0x0003_4600;
const RECORD_LEN: usize = 0x890;
const WHEELS_AT: usize = 0x120;
const WHEEL_LEN: usize = 0x30;
/// 0 for the 31 cars players and racers drive, 2 for traffic.
const USAGE_AT: usize = 0x844;

#[derive(Clone, Copy, Debug, Default)]
pub struct WheelInfo {
    /// Hub position in car space (X forward, Y left, Z up).
    pub position: [f32; 3],
    pub radius: f32,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarUsage {
    Racer,
    Traffic,
    Other(u32),
}

/// Engine, gearbox and chassis data (stock). Values match the real cars, e.g. the
/// 350Z: 1450 kg, 288 hp, rear drive, 155 mph limiter.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drivetrain {
    /// Kilograms.
    pub mass: f32,
    /// Length, width, height in metres.
    pub size: [f32; 3],
    /// Peak tyre friction, front and rear.
    pub grip: [f32; 2],
    pub final_drive: f32,
    /// Share of drive to the rear wheels: 0 FWD, 1 RWD, ~0.5 AWD.
    pub rear_bias: f32,
    /// Gears 1-6 (0 = absent).
    pub gears: [f32; 6],
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    pub max_rpm: f32,
    /// Torque in N·m at `max_rpm * k / 9` for k = 0..9.
    pub torque: [f32; 10],
    /// Stock turbo torque added at points 1..9.
    pub turbo: [f32; 9],
    /// Top-speed limiter, m/s.
    pub speed_limit: f32,
}

impl Drivetrain {
    /// Peak engine power in kW over the torque curve (turbo included).
    pub fn peak_kw(&self) -> f32 {
        (0..10)
            .map(|k| {
                let t = self.torque[k] + if k > 0 { self.turbo[k - 1] } else { 0.0 };
                let rpm = self.max_rpm * k as f32 / 9.0;
                t * rpm * std::f32::consts::TAU / 60.0 / 1000.0
            })
            .fold(0.0, f32::max)
    }
}

#[derive(Clone, Debug)]
pub struct CarTypeInfo {
    pub name: String,
    pub wheels: [WheelInfo; 4],
    pub usage: CarUsage,
    pub drivetrain: Drivetrain,
}

pub fn parse_car_types(global_b: &[u8]) -> Result<Vec<CarTypeInfo>> {
    let Some(chunk) = ChunkIter::new(global_b).find(|c| c.id == CAR_TYPE_INFO) else {
        bail!("no CarTypeInfo chunk");
    };
    // Records are aligned to the end of the chunk; the start holds alignment padding.
    let data = chunk.data;
    let start = data.len() % RECORD_LEN;
    let mut out = Vec::new();
    for rec in data[start..].chunks_exact(RECORD_LEN) {
        let mut wheels = [WheelInfo::default(); 4];
        for (i, w) in wheels.iter_mut().enumerate() {
            let o = WHEELS_AT + i * WHEEL_LEN;
            *w = WheelInfo { position: vec3_at(rec, o)?, radius: f32_at(rec, o + 0x10)?, width: f32_at(rec, o + 0x14)? };
        }
        let usage = match u32_at(rec, USAGE_AT)? {
            0 => CarUsage::Racer,
            2 => CarUsage::Traffic,
            n => CarUsage::Other(n),
        };
        let f = |o: usize| f32_at(rec, o);
        let mut d = Drivetrain {
            mass: f(0x220)? * 1000.0,
            size: [f(0x224)?, f(0x228)?, f(0x22C)?],
            grip: [f(0x1EC)?, f(0x20C)?],
            final_drive: f(0x2C8)?,
            rear_bias: f(0x2D0)?,
            idle_rpm: f(0x300)?,
            redline_rpm: f(0x304)?,
            max_rpm: f(0x308)?,
            speed_limit: f(0x3A0)? * 0.44704,
            ..Default::default()
        };
        for i in 0..6 {
            d.gears[i] = f(0x2E8 + 4 * i)?;
        }
        for i in 0..10 {
            d.torque[i] = f(0x30C + 4 * i)? * 1000.0;
        }
        for i in 0..9 {
            d.turbo[i] = f(0x334 + 4 * i)? * 1000.0;
        }
        out.push(CarTypeInfo { name: cstr_at(rec, 0, 32)?, wheels, usage, drivetrain: d });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_car_data() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(g) = game.read("GLOBAL/GLOBALB.BUN") else { return };
        let cars = parse_car_types(&g).unwrap();
        let z = cars.iter().find(|c| c.name == "350Z").unwrap();
        assert_eq!(z.usage, CarUsage::Racer);
        assert!((z.drivetrain.mass - 1450.0).abs() < 1.0, "mass {}", z.drivetrain.mass);
        let hp = z.drivetrain.peak_kw() * 1.341;
        assert!((270.0..310.0).contains(&hp), "350Z peak {hp} hp");
        assert!(z.drivetrain.rear_bias > 0.9);
        let h2 = cars.iter().find(|c| c.name == "HUMMER").unwrap();
        assert!(h2.drivetrain.mass > 2500.0);
    }
}
