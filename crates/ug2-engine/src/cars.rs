//! Car models: a car's stock parts (or a traffic car's body) merged into one mesh plus
//! a wheel, with the materials the game fills in at runtime (paint, windows, lights)
//! resolved to textures and lighting roles, and light positions for glow sprites.

use std::collections::{HashMap, HashSet};

use glam::{Mat4, Vec3};
use ug2_formats::bin_hash;
use ug2_formats::car::{fitted_parts, runtime_materials, traffic_parts, Material, PartKind, PartSet};
use ug2_formats::carinfo::{parse_car_types, CarTypeInfo, CarUsage, WheelInfo};
use ug2_formats::fs::GameDir;
use ug2_formats::solid::{find_solid_lists, Solid};
use ug2_formats::tpk::{find_tpks, AlphaUsage, TexFormat, Texture};

use crate::assets::{append_solid, blend_for, finish_mesh, Blend, GpuVertex, MeshData, Role, PAINT};

/// Data shared by every car: the car table and textures any car may reference.
pub struct SharedCarData {
    pub types: Vec<CarTypeInfo>,
    /// CARS/TEXTURES.BIN (traffic skins, shadows) and GLOBALB (window, chrome, driver...).
    textures: HashMap<u32, Texture>,
}

impl SharedCarData {
    pub fn load(game: &GameDir) -> Self {
        let mut textures = HashMap::new();
        let global_b = game.read("GLOBAL/GLOBALB.BUN").or_else(|_| game.read_unpacked("GLOBAL/GlobalB.lzc"));
        let types = global_b.as_ref().ok().and_then(|g| parse_car_types(g).ok()).unwrap_or_default();
        if let Ok(g) = &global_b {
            for t in find_tpks(g).into_iter().flat_map(|p| p.textures) {
                textures.insert(t.hash, t);
            }
        }
        if let Ok(d) = game.read("CARS/TEXTURES.BIN") {
            for t in find_tpks(&d).into_iter().flat_map(|p| p.textures) {
                textures.insert(t.hash, t);
            }
        }
        Self { types, textures }
    }

    /// Stock performance of every car with data.
    pub fn perfs(&self) -> Vec<(String, crate::vehicle::Perf)> {
        self.types
            .iter()
            .filter_map(|t| perf_from(&t.drivetrain, t.wheels[0].radius).map(|p| (t.name.clone(), p)))
            .collect()
    }

    pub fn names(&self, usage: CarUsage) -> Vec<String> {
        self.types.iter().filter(|t| t.usage == usage).map(|t| t.name.clone()).collect()
    }

    fn info(&self, name: &str) -> Option<&CarTypeInfo> {
        self.types.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
}

pub struct CarModel {
    pub name: String,
    /// Visual parts fitted (stock for traffic and opponents).
    pub parts: PartSet,
    pub body: MeshData,
    pub wheel: Option<MeshData>,
    /// Soft shadow quad under the car (`<CAR>_SHADOWIG`).
    pub shadow: Option<MeshData>,
    pub wheels: [WheelInfo; 4],
    /// Glow sprite positions in car space.
    pub headlights: Vec<Vec3>,
    pub brakelights: Vec<Vec3>,
    pub traffic: bool,
    /// Traffic paint jobs (`<CAR>_SKIN1..8`); the body references the first.
    pub skins: Vec<u32>,
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
}

impl CarModel {
    pub fn ride_height(&self) -> f32 {
        let w = self.wheels[0];
        w.radius - w.position[2]
    }

    pub fn wheelbase(&self) -> f32 {
        let front = self.wheels.iter().map(|w| w.position[0]).fold(f32::MIN, f32::max);
        let rear = self.wheels.iter().map(|w| w.position[0]).fold(f32::MAX, f32::min);
        (front - rear).max(1.5)
    }
}

fn center(s: &Solid) -> Vec3 {
    (Vec3::from(s.bbox_min) + Vec3::from(s.bbox_max)) * 0.5
}

/// Loads a racer or traffic car. Returns the model and every texture it draws with.
pub fn load_car(game: &GameDir, shared: &SharedCarData, name: &str, fitted: &PartSet) -> anyhow::Result<(CarModel, Vec<Texture>)> {
    let geo = game.read(&format!("CARS/{name}/GEOMETRY.BIN"))?;
    let own: Vec<Texture> = game
        .read(&format!("CARS/{name}/TEXTURES.BIN"))
        .map(|d| find_tpks(&d).into_iter().flat_map(|p| p.textures).collect())
        .unwrap_or_default();
    let solids: Vec<Solid> = find_solid_lists(&geo).into_iter().flat_map(|l| l.solids).collect();
    let info = shared.info(name);
    let traffic = info.is_some_and(|i| i.usage == CarUsage::Traffic);

    let mut available: HashMap<u32, &Texture> = shared.textures.iter().map(|(h, t)| (*h, t)).collect();
    available.extend(own.iter().map(|t| (t.hash, t)));
    let runtime: HashMap<u32, (Vec<u32>, Material)> =
        runtime_materials(name, fitted.lights).into_iter().map(|(h, r, m)| (h, (r, m))).collect();
    let skin1 = bin_hash(&format!("{name}_SKIN1"));
    let skins: Vec<u32> =
        (1..=8).map(|i| bin_hash(&format!("{name}_SKIN{i}"))).filter(|h| available.contains_key(h)).collect();

    // Racers: the paint skin is composited at runtime from the paint colour and the
    // door-line detail atlas; build that atlas once here.
    let detail = if skins.is_empty() {
        let line = available.get(&bin_hash(&format!("{name}_DOORLINE")));
        let mask = available.get(&bin_hash(&format!("{name}_DOORLINE_MASK")));
        line.zip(mask).and_then(|(l, m)| skin_detail(skin1, l, m))
    } else {
        None
    };
    let texture_of = |h: u32| -> (u32, Blend, Role) {
        if h == skin1 && !skins.is_empty() {
            return (h, Blend::Opaque, Role::Skin);
        }
        if h == skin1 && detail.is_some() {
            return (h, Blend::Opaque, Role::PaintDetail);
        }
        if let Some((candidates, material)) = runtime.get(&h) {
            let role = match material {
                Material::Paint => return (PAINT, Blend::Opaque, Role::Paint),
                Material::Window | Material::HeadlightGlass => Role::Glass,
                Material::Headlight => Role::Headlight,
                Material::Brakelight | Material::BrakelightGlass => Role::Brakelight,
            };
            let tex = candidates.iter().copied().find(|c| available.contains_key(c)).unwrap_or(0);
            let blend = available.get(&tex).map(|t| blend_for(t)).unwrap_or(Blend::Opaque);
            return (tex, blend, role);
        }
        match available.get(&h) {
            Some(t) => (h, blend_for(t), Role::Normal),
            None => (PAINT, Blend::Opaque, Role::Paint),
        }
    };

    // Each part: the first candidate solid the car has (styles fall back to stock).
    let parts: Vec<(u32, PartKind)> = if traffic {
        traffic_parts(name).into_iter().map(|(n, k)| (bin_hash(&n), k)).collect()
    } else {
        fitted_parts(name, fitted)
            .into_iter()
            .filter_map(|(names, k)| {
                names.iter().map(|n| bin_hash(n)).find(|h| solids.iter().any(|s| s.hash == *h)).map(|h| (h, k))
            })
            .collect()
    };
    let mut body_v: Vec<GpuVertex> = Vec::new();
    let mut body_b = HashMap::new();
    let mut body_box = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut wheel = None;
    let mut used: HashSet<u32> = HashSet::new();
    let (mut headlights, mut brakelights) = (Vec::new(), Vec::new());
    for (hash, kind) in parts {
        let Some(s) = solids.iter().find(|s| s.hash == hash) else { continue };
        for &t in &s.textures {
            used.insert(texture_of(t).0);
        }
        match kind {
            PartKind::Body => {
                append_solid(s, &Mat4::IDENTITY, &mut body_v, &mut body_b, &texture_of, &mut body_box);
                if s.name.contains("_KIT00_HEADLIGHT_") {
                    headlights.push(center(s));
                } else if s.name.contains("_KIT00_BRAKELIGHT") {
                    // One part spans both lamps: put a glow towards each end.
                    let (c, half_y) = (center(s), (s.bbox_max[1] - s.bbox_min[1]) * 0.5);
                    brakelights.push(c + Vec3::Y * half_y * 0.75);
                    brakelights.push(c - Vec3::Y * half_y * 0.75);
                }
            }
            PartKind::FrontWheel => {
                let (mut v, mut b, mut bb) = (Vec::new(), HashMap::new(), (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)));
                append_solid(s, &Mat4::IDENTITY, &mut v, &mut b, &texture_of, &mut bb);
                wheel = Some(finish_mesh(v, b, bb));
            }
            PartKind::FrontBrake | PartKind::RearBrake => {}
        }
    }
    if body_v.is_empty() {
        anyhow::bail!("car {name}: no body parts found");
    }
    let (bmin, bmax) = body_box;
    if headlights.is_empty() {
        // Traffic cars have the lamps painted into the skin: estimate from the bounds.
        let (x, y, z) = (bmax.x - 0.15, (bmax.y - bmin.y) * 0.32, bmin.z + (bmax.z - bmin.z) * 0.38);
        headlights = vec![Vec3::new(x, y, z), Vec3::new(x, -y, z)];
    }
    if brakelights.is_empty() {
        let (x, y, z) = (bmin.x + 0.12, (bmax.y - bmin.y) * 0.36, bmin.z + (bmax.z - bmin.z) * 0.45);
        brakelights = vec![Vec3::new(x, y, z), Vec3::new(x, -y, z)];
    }

    let wheels = match info {
        Some(i) => i.wheels,
        None => {
            log::warn!("no CarTypeInfo for {name}, guessing wheel positions");
            let w = |x: f32, y: f32| WheelInfo { position: [x, y, 0.17], radius: 0.33, width: 0.22 };
            [w(1.35, 0.75), w(1.35, -0.75), w(-1.3, -0.75), w(-1.3, 0.75)]
        }
    };

    let shadow_tex = bin_hash(&format!("{name}_SHADOWIG"));
    let shadow = available.contains_key(&shadow_tex).then(|| {
        used.insert(shadow_tex);
        let ground = wheels[0].position[2] - wheels[0].radius + 0.03;
        shadow_quad(shadow_tex, bmin, bmax, ground)
    });

    used.extend(skins.iter().copied());
    let mut textures: Vec<Texture> = used.iter().filter_map(|h| available.get(h).map(|t| (*t).clone())).collect();
    textures.extend(detail);
    let car = CarModel {
        name: name.to_string(),
        parts: *fitted,
        body: finish_mesh(body_v, body_b, body_box),
        wheel,
        shadow,
        wheels,
        headlights,
        brakelights,
        traffic,
        skins,
        bbox_min: bmin,
        bbox_max: bmax,
    };
    Ok((car, textures))
}

/// Detail atlas for the paint shader: RGB from `<CAR>_DOORLINE`, alpha from its mask
/// (0 = paint shows through, 1 = detail replaces it), with a full mip chain.
fn skin_detail(hash: u32, line: &Texture, mask: &Texture) -> Option<Texture> {
    let (w, h, rgb) = line.level_rgba8(0)?;
    let (mw, mh, m) = mask.level_rgba8(0)?;
    if (w, h) != (mw, mh) || w == 0 || h == 0 {
        return None;
    }
    // The mask peaks at 155: that grey means "fully detail".
    let mut level: Vec<u8> = rgb
        .chunks_exact(4)
        .zip(m.chunks_exact(4))
        .flat_map(|(c, k)| [c[2], c[1], c[0], ((k[0] as u32 * 255) / 155).min(255) as u8])
        .collect();
    let (mut lw, mut lh) = (w, h);
    let mut data = level.clone();
    let mut mips = 1;
    while lw > 1 || lh > 1 {
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for ch in 0..4 {
                    let px = |dx: u32, dy: u32| {
                        let (sx, sy) = ((x * 2 + dx).min(lw - 1), (y * 2 + dy).min(lh - 1));
                        level[((sy * lw + sx) * 4 + ch) as usize] as u32
                    };
                    next[((y * nw + x) * 4 + ch) as usize] = ((px(0, 0) + px(1, 0) + px(0, 1) + px(1, 1)) / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        (lw, lh) = (nw, nh);
        mips += 1;
    }
    Some(Texture {
        hash,
        name: line.name.clone(),
        width: w,
        height: h,
        mip_count: mips,
        format: TexFormat::Bgra8 { opaque: false },
        data,
        palette: Vec::new(),
        alpha: AlphaUsage::Opaque,
        alpha_blend: 0,
        tileable: 0,
    })
}

/// Key a loaded model is known by: the car name, plus the fitted parts when not stock.
pub fn model_key(name: &str, parts: &PartSet) -> String {
    if parts.is_stock() {
        name.to_string()
    } else {
        format!("{name}#{}", parts.key())
    }
}

/// Car name of a model key.
pub fn car_name(key: &str) -> &str {
    key.split('#').next().unwrap_or(key)
}

/// Flat quad on the ground under the body; the texture's long side runs along the car.
fn shadow_quad(tex: u32, min: Vec3, max: Vec3, z: f32) -> MeshData {
    let (cx, cy) = ((min.x + max.x) * 0.5, (min.y + max.y) * 0.5);
    let (hx, hy) = ((max.x - min.x) * 0.58, (max.y - min.y) * 0.62);
    let v = |x: f32, y: f32, u: f32, w: f32| GpuVertex {
        pos: [cx + x * hx, cy + y * hy, z],
        normal: [0.0, 0.0, 1.0],
        color: [255; 4],
        uv: [u, w],
    };
    let vertices = vec![v(-1.0, 1.0, 0.0, 0.0), v(1.0, 1.0, 1.0, 0.0), v(1.0, -1.0, 1.0, 1.0), v(-1.0, -1.0, 0.0, 1.0)];
    let mut buckets = HashMap::new();
    buckets.insert((Blend::Blend, tex, Role::Shadow), vec![0, 1, 2, 0, 2, 3]);
    finish_mesh(vertices, buckets, (Vec3::new(cx - hx, cy - hy, z), Vec3::new(cx + hx, cy + hy, z)))
}

/// Class of a racer: 1 (starter) .. 3 (top), roughly as the game unlocks them.
pub fn tier(car: &str) -> u32 {
    match car {
        "PEUGOT" | "FOCUS" | "COROLLA" | "240SX" | "MIATA" | "CIVIC" | "PEUGOT106" | "CORSA" => 1,
        "HUMMER" | "NAVIGATOR" | "ESCALADE" | "TIBURON" | "SENTRA" | "CELICA" | "IS300" | "GOLF" | "A3" | "RSX"
        | "ECLIPSE" => 2,
        _ => 3,
    }
}

/// Stock performance from the car's real data: power-to-weight relative to the median
/// racer (108 W/kg, square-rooted so the field stays close), top speed from the
/// limiter or the gearing, grip from the tyres relative to the median (1.66).
pub fn perf_from(d: &ug2_formats::carinfo::Drivetrain, wheel_radius: f32) -> Option<crate::vehicle::Perf> {
    if d.mass < 300.0 || d.max_rpm < 1000.0 {
        return None;
    }
    let w_per_kg = d.peak_kw() * 1000.0 / d.mass;
    let top_gear = d.gears.iter().copied().filter(|g| *g > 0.0).fold(f32::MAX, f32::min);
    let gearing = if top_gear < f32::MAX && d.final_drive > 0.0 {
        d.redline_rpm / 60.0 * std::f32::consts::TAU * wheel_radius / (top_gear * d.final_drive)
    } else {
        f32::MAX
    };
    let limit = if d.speed_limit > 10.0 { d.speed_limit } else { f32::MAX };
    Some(crate::vehicle::Perf {
        power: (w_per_kg / 108.0).sqrt().clamp(0.75, 1.25),
        top_speed: limit.min(gearing).clamp(40.0, 72.0),
        grip: ((d.grip[0] + d.grip[1]) * 0.5 / 1.66).clamp(0.85, 1.15),
        nitro: 1.0,
        mass: d.mass / 1000.0,
    })
}

/// Fallback when a car's data is missing: by class; SUVs are slower and grip less.
pub fn base_perf(car: &str) -> crate::vehicle::Perf {
    let suv = matches!(car, "HUMMER" | "NAVIGATOR" | "ESCALADE");
    let (power, top) = match tier(car) {
        1 => (0.82, 58.0),
        2 => (0.92, 64.0),
        _ => (1.0, 70.0),
    };
    crate::vehicle::Perf {
        power: if suv { power * 1.05 } else { power },
        top_speed: if suv { top - 6.0 } else { top },
        grip: if suv { 0.85 } else { 0.95 + 0.03 * tier(car) as f32 },
        nitro: 1.0,
        mass: if suv { 2.7 } else { 1.3 },
    }
}

/// Human-readable names for the 31 racers.
pub fn display_name(car: &str) -> &str {
    match car {
        "PEUGOT" => "PEUGEOT 206",
        "FOCUS" => "FORD FOCUS ZX3",
        "COROLLA" => "TOYOTA COROLLA",
        "240SX" => "NISSAN 240SX",
        "MIATA" => "MAZDA MIATA MX-5",
        "CIVIC" => "HONDA CIVIC",
        "PEUGOT106" => "PEUGEOT 106",
        "CORSA" => "VAUXHALL CORSA",
        "HUMMER" => "HUMMER H2",
        "NAVIGATOR" => "LINCOLN NAVIGATOR",
        "ESCALADE" => "CADILLAC ESCALADE",
        "TIBURON" => "HYUNDAI TIBURON",
        "SENTRA" => "NISSAN SENTRA SE-R",
        "CELICA" => "TOYOTA CELICA",
        "IS300" => "LEXUS IS300",
        "SUPRA" => "TOYOTA SUPRA",
        "GOLF" => "VOLKSWAGEN GOLF GTI",
        "A3" => "AUDI A3 3.2",
        "RSX" => "ACURA RSX",
        "ECLIPSE" => "MITSUBISHI ECLIPSE",
        "TT" => "AUDI TT 3.2",
        "RX8" => "MAZDA RX-8",
        "350Z" => "NISSAN 350Z",
        "G35" => "INFINITI G35",
        "3000GT" => "MITSUBISHI 3000GT VR-4",
        "GTO" => "PONTIAC GTO",
        "MUSTANGGT" => "FORD MUSTANG GT",
        "SKYLINE" => "NISSAN SKYLINE GT-R",
        "LANCEREVO8" => "MITSUBISHI LANCER EVO VIII",
        "RX7" => "MAZDA RX-7",
        "IMPREZAWRX" => "SUBARU IMPREZA WRX STI",
        other => other,
    }
}
