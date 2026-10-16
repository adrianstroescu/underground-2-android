//! The PC game's per-frame visual treatment: speed blur, colour treatment, overbright
//! bloom and light trails, computed as speed2.exe does (addresses in CLAUDE.md,
//! "Post-processing chain"). The original updates these once per frame at 60 Hz; the
//! per-frame steps here are scaled to the frame's length.

use glam::{Mat4, Vec3};
use ug2_formats::envmap::{EnvMaps, Tracked};

use crate::post::{PostFrame, SpeedBlur};

/// The original's trail decay per 60 Hz frame (BLENDFACTOR 0xFF785AAA): R, G, B.
const TRAIL_DECAY: [f32; 3] = [120.0 / 255.0, 90.0 / 255.0, 170.0 / 255.0];

#[derive(Default)]
pub struct VisualTreatment {
    tint: Tracked,
    /// Smoothed bloom amount (0x876848).
    bloom_s: f32,
    /// Extra blur while the boost flag is on (0x870900).
    boost: f32,
    last_eye: Option<Vec3>,
}

pub struct FrameInput<'a> {
    pub maps: Option<&'a EnvMaps>,
    /// Racing or free roaming (the game's mode 6); false in the menus.
    pub in_game: bool,
    pub eye: Vec3,
    pub view: Mat4,
    pub dt: f32,
    /// The flag that raises the speed blur; taken to be the player's nitrous.
    pub boost: bool,
    pub bloom_enabled: bool,
}

impl VisualTreatment {
    /// A new region: the tint eases from scratch.
    pub fn reset(&mut self) {
        self.tint = Tracked::default();
        self.last_eye = None;
    }

    pub fn frame(&mut self, i: &FrameInput) -> PostFrame {
        let dt = i.dt.clamp(1e-4, 0.25);
        let steps = dt * 60.0;
        let mut out = PostFrame::default();

        // Speed blur (0x5cc460).
        let velocity = self.last_eye.map_or(Vec3::ZERO, |l| (i.eye - l) / dt);
        self.last_eye = Some(i.eye);
        let target = if i.boost { 1.0 } else { 0.0 };
        if (self.boost - target).abs() > 1.0 / 255.0 {
            self.boost = if self.boost < target {
                (self.boost + 0.2 * steps).min(target)
            } else {
                (self.boost - 0.1 * steps).max(target)
            };
        }
        let speed = velocity.length();
        let t = (((speed - 25.0) / 100.0).clamp(0.0, 1.0) + 1.1 * self.boost).min(0.65);
        if t > 1.0 / 255.0 {
            // Velocity in D3D view space: x right, y up, z forward (ours looks down −z).
            let v = i.view.transform_vector3(velocity);
            let d = Vec3::new(v.x, v.y, -v.z);
            let dir = if speed > 1e-5 { d.normalize_or_zero().to_array() } else { [0.0; 3] };
            out.speed_blur = Some(SpeedBlur {
                amount: (t * 255.0).floor() / 255.0,
                k: 0.00625 * 1.2 * t * (1.0 + 0.5 * self.boost),
                dir,
            });
        }

        // Overbright bloom (0x5bc100, 0x5bc180, 0x5cca00).
        let x = if i.in_game { 0.0 } else { 0.45 };
        let target = 0.2 + 0.8 * x;
        self.bloom_s += (target - self.bloom_s) * (1.0 - 0.8f32.powf(steps));
        out.overbright_offset = (216.0 - 154.0 * self.bloom_s).floor() / 510.0;
        let bloom = if i.in_game { (10.0 + 62.0 * self.bloom_s).floor() } else { 80.0 };
        out.bloom = if i.bloom_enabled { bloom / 255.0 } else { 0.0 };
        out.half_add = if i.in_game { 176.0 } else { 160.0 } / 255.0;

        // Colour treatment (0x5cb860, 0x5c68d0, 0x5d77f0): the screen tint record under
        // the camera, eased by distance; the Sunrise map at full weight, as captured.
        let (intensity, color, mix) = if i.in_game {
            let rec = match i.maps.and_then(|m| m.get("Sunrise Screen Tint")) {
                Some(map) => self.tint.update(map, i.eye.x, i.eye.y).to_vec(),
                None => vec![0.0; 7],
            };
            let g = |k: usize| rec.get(k).copied().unwrap_or(0.0);
            (g(3), [g(0) / 255.0, g(1) / 255.0, g(2) / 255.0], g(6))
        } else {
            (0.8, [0.5; 3], 0.2)
        };
        let s = 0.2 + 0.8 * intensity;
        out.gain = [0.5 - 0.3 * s; 4];
        let scale = 0.25 * s;
        out.tint = [color[0] * scale, color[1] * scale, color[2] * scale, mix];

        // Light trails: the 60 Hz decay over this frame's length, with the new light
        // weighted so the steady state matches the original.
        for c in 0..3 {
            let k = TRAIL_DECAY[c].powf(steps);
            out.trail_decay[c] = k;
            out.trail_weight[c] = (1.0 - k) / (1.0 - TRAIL_DECAY[c]);
        }
        out
    }
}
