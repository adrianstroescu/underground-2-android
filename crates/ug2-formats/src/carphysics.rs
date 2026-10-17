//! The physics half of a car's `CarTypeInfo` record (GLOBALB chunk 0x34600, 0x890
//! bytes), in the game's own units: tonnes, kN, kN·m, t·m², m, s, rad/s, rpm.
//! speed2.exe copies the record from +0x110 into the PhysicsMover's data block, so a
//! physics-struct offset X is record offset X + 0x110 (`ps+0x1f0` = record 0x300).

use crate::bytes::{f32_at, u32_at};
use crate::Result;

/// Per-axle tyre data (record 0x1E0 front, 0x200 rear).
#[derive(Clone, Copy, Debug, Default)]
pub struct TyreData {
    /// Largest slip angle that still grips (rad).
    pub max_static_slip: f32,
    /// Brake torque scale (× 4 × brake input).
    pub brake: f32,
    /// Handbrake torque scale (× 10 × handbrake input).
    pub handbrake: f32,
    /// Static and sliding friction.
    pub static_grip: f32,
    pub dynamic_grip: f32,
}

#[derive(Clone, Debug, Default)]
pub struct CarPhysics {
    /// The whole record, for the fields not named here yet.
    pub record: Vec<u8>,
    /// Tonnes (0x220).
    pub mass: f32,
    /// Wheel radius per wheel FL FR RL RR (0x120 + 0x30·i + 0x10).
    pub wheel_radius: [f32; 4],
    /// Front and rear tyres.
    pub tyres: [TyreData; 2],
    /// First-gear upshift point as a fraction of idle..redline (0x2C0).
    pub first_shift: f32,
    /// Final drive, front and rear axle (0x2C8, 0x2CC).
    pub final_drive: [f32; 2],
    /// Share of drive torque to the rear axle (0x2D0).
    pub rear_bias: f32,
    /// Clutch stiffness, kN·m per rad/s (0x2D4).
    pub clutch_stiffness: f32,
    /// Forward gears (0x2D8).
    pub gear_count: u32,
    /// Clutch speed (0x2DC): out in 1/(3·k) s, in over 1/(1.5·k) s.
    pub clutch_rate: f32,
    /// Gear ratios: [R, N, 1st, ...] (0x2E0, gear_count + 2 entries).
    pub gears: Vec<f32>,
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    pub max_rpm: f32,
    /// Engine inertia, t·m² (0x30C).
    pub engine_inertia: f32,
    /// Torque curve, kN·m at 9 points idle..max (0x310), stock turbo already added
    /// (0x334, as speed2.exe 0x5a6e30 does).
    pub torque: [f32; 9],
    /// Engine braking, kN·m at 3 points idle..max (0x358).
    pub engine_brake: [f32; 3],
    /// Aero drag coefficient (0x370) and the two downforce values (0x374, 0x378).
    pub drag: f32,
    pub downforce: [f32; 2],
    /// Top-speed limiter, mph (0x3A0); 0 = none.
    pub speed_limit_mph: f32,
    /// Nitrous torque scale (0x3BC).
    pub nitrous: f32,
}

pub const RECORD_LEN: usize = 0x890;

impl CarPhysics {
    pub fn parse(rec: &[u8]) -> Result<Self> {
        let f = |o: usize| f32_at(rec, o);
        let tyre = |o: usize| -> Result<TyreData> {
            Ok(TyreData { max_static_slip: f(o)?, brake: f(o + 4)?, handbrake: f(o + 8)?, static_grip: f(o + 0xC)?, dynamic_grip: f(o + 0x10)? })
        };
        let gear_count = u32_at(rec, 0x2D8)?.min(6);
        let mut torque = [0.0; 9];
        for (k, t) in torque.iter_mut().enumerate() {
            *t = f(0x310 + 4 * k)? + f(0x334 + 4 * k)?;
        }
        Ok(CarPhysics {
            record: rec.to_vec(),
            mass: f(0x220)?,
            wheel_radius: [f(0x130)?, f(0x160)?, f(0x190)?, f(0x1C0)?],
            tyres: [tyre(0x1E0)?, tyre(0x200)?],
            first_shift: f(0x2C0)?,
            final_drive: [f(0x2C8)?, f(0x2CC)?],
            rear_bias: f(0x2D0)?,
            clutch_stiffness: f(0x2D4)?,
            gear_count,
            clutch_rate: f(0x2DC)?,
            gears: (0..gear_count as usize + 2).map(|i| f(0x2E0 + 4 * i)).collect::<Result<_>>()?,
            idle_rpm: f(0x300)?,
            redline_rpm: f(0x304)?,
            max_rpm: f(0x308)?,
            engine_inertia: f(0x30C)?,
            torque,
            engine_brake: [f(0x358)?, f(0x35C)?, f(0x360)?],
            drag: f(0x370)?,
            downforce: [f(0x374)?, f(0x378)?],
            speed_limit_mph: f(0x3A0)?,
            nitrous: f(0x3BC)?,
        })
    }

    /// Record field as f32 (for fields not named yet).
    pub fn f(&self, o: usize) -> f32 {
        f32_at(&self.record, o).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn skyline_physics() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(g) = game.read("GLOBAL/GLOBALB.BUN") else { return };
        let cars = crate::carinfo::parse_car_types(&g).unwrap();
        let p = &cars.iter().find(|c| c.name == "SKYLINE").unwrap().physics;
        assert_eq!(p.gear_count, 6);
        assert!((p.mass - 1.5).abs() < 1e-3);
        assert_eq!((p.idle_rpm, p.redline_rpm, p.max_rpm), (850.0, 8000.0, 8500.0));
        assert!((p.engine_inertia - 0.00026).abs() < 1e-6);
        assert!((p.rear_bias - 0.5).abs() < 1e-3);
        assert_eq!(p.clutch_rate, 4.0);
    }
}
