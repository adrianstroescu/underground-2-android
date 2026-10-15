//! Background loading: parses the original game files on a worker thread and turns
//! them into GPU-ready meshes and textures, streamed to the main thread as events.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use ug2_formats::car::{stock_part_hashes, PartKind};
use ug2_formats::carinfo::{parse_car_types, WheelInfo};
use ug2_formats::fs::GameDir;
use ug2_formats::scenery::ScenerySection;
use ug2_formats::solid::{find_solid_lists, Solid};
use ug2_formats::tpk::{find_tpks, AlphaUsage, TexFormat, Texture};
use ug2_formats::track::{parse_region, parse_streaming_sections};

use crate::collision::CollisionWorld;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct GpuVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [u8; 4],
    pub uv: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Blend {
    Opaque,
    Cutout,
    Blend,
    /// Neon, glows and light beams.
    Additive,
}

/// Texture hash used for car paint (groups whose texture is supplied by a shader).
pub const PAINT: u32 = 0xFFFF_FFFF;

#[derive(Clone, Debug)]
pub struct DrawRange {
    pub texture: u32,
    pub blend: Blend,
    pub first_index: u32,
    pub index_count: u32,
}

#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<GpuVertex>,
    pub indices: Vec<u32>,
    pub ranges: Vec<DrawRange>,
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
}

pub struct SectionAsset {
    pub number: u32,
    pub sky: bool,
    pub mesh: MeshData,
}

pub struct PreparedTexture {
    pub hash: u32,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub mips: u32,
    /// All mip levels back to back.
    pub data: Vec<u8>,
}

pub struct CarAsset {
    pub name: String,
    pub body: MeshData,
    pub wheel: Option<MeshData>,
    pub wheels: [WheelInfo; 4],
}

pub enum LoadEvent {
    Progress(String),
    Textures(Vec<PreparedTexture>),
    Car(CarAsset),
    Collision(CollisionWorld),
    Spawn { pos: Vec3, yaw: f32 },
    Section(SectionAsset),
    Done,
    Failed(String),
}

#[derive(Clone)]
pub struct LoaderConfig {
    pub car: String,
    pub location: String,
    pub bc_textures: bool,
}

pub fn spawn_loader(game: GameDir, cfg: LoaderConfig) -> Receiver<LoadEvent> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("ug2-loader".into())
        .spawn(move || {
            if let Err(e) = load_all(&game, &cfg, &tx) {
                log::error!("load failed: {e:#}");
                let _ = tx.send(LoadEvent::Failed(format!("{e:#}")));
            }
        })
        .expect("spawn loader thread");
    rx
}

fn load_all(game: &GameDir, cfg: &LoaderConfig, tx: &Sender<LoadEvent>) -> anyhow::Result<()> {
    let t0 = std::time::Instant::now();
    let send = |e: LoadEvent| {
        let _ = tx.send(e);
    };

    send(LoadEvent::Progress(format!("LOADING CAR {}", cfg.car)));
    let (car, car_textures) = load_car(game, &cfg.car)?;
    send(LoadEvent::Textures(car_textures.iter().filter_map(|t| prepare_texture(t, cfg.bc_textures)).collect()));
    send(LoadEvent::Car(car));

    send(LoadEvent::Progress("READING TRACK".into()));
    let index = game.read(&format!("TRACKS/{}.BUN", cfg.location))?;
    let stream = game.read(&format!("TRACKS/STREAM{}.BUN", cfg.location))?;
    let sections = parse_streaming_sections(&index)?;

    send(LoadEvent::Progress("PARSING TRACK".into()));
    let mut solids: HashMap<u32, Arc<Solid>> = HashMap::new();
    let mut textures: Vec<Texture> = Vec::new();
    let mut scenery: Vec<ScenerySection> = Vec::new();
    for s in &sections {
        if s.offset + s.size > stream.len() {
            continue;
        }
        let d = parse_region(&stream, s.offset, s.size);
        for l in d.solids {
            for so in l.solids {
                solids.entry(so.hash).or_insert_with(|| Arc::new(so));
            }
        }
        for p in d.textures {
            textures.extend(p.textures);
        }
        scenery.extend(d.scenery);
    }
    drop(stream);
    // Shared world textures (street furniture, cranes, ships...) live outside the stream.
    for extra in ["GLOBAL/GLOBALB.BUN", "TRACKS/LOC4DYNTEX.BIN", "GLOBAL/DYNTEX.BIN"] {
        let Ok(data) = game.read_unpacked(extra) else { continue };
        for p in find_tpks(&data) {
            textures.extend(p.textures);
        }
    }
    log::info!(
        "track parsed in {:.2?}: {} solids, {} textures, {} scenery sections",
        t0.elapsed(),
        solids.len(),
        textures.len(),
        scenery.len()
    );

    let mut blend_of: HashMap<u32, Blend> = HashMap::new();
    for t in &textures {
        blend_of.insert(t.hash, blend_for(t));
    }

    send(LoadEvent::Progress("BUILDING COLLISION".into()));
    let (collision, spawn) = build_collision(&scenery, &solids);
    send(LoadEvent::Collision(collision));
    let (spawn_pos, spawn_yaw) = spawn.unwrap_or((Vec3::new(0.0, 0.0, 20.0), 0.0));
    send(LoadEvent::Spawn { pos: spawn_pos, yaw: spawn_yaw });

    // Textures: upload in batches so the main thread can keep rendering.
    let total = textures.len();
    let mut seen = HashSet::new();
    for (i, chunk) in textures.chunks(64).enumerate() {
        send(LoadEvent::Progress(format!("TEXTURES {}%", (i * 64 * 100) / total.max(1))));
        let prepared: Vec<PreparedTexture> = chunk
            .iter()
            .filter(|t| seen.insert(t.hash))
            .filter_map(|t| prepare_texture(t, cfg.bc_textures))
            .collect();
        send(LoadEvent::Textures(prepared));
    }
    drop(textures);

    // Sections nearest to the spawn point first.
    let mut order: Vec<usize> = (0..scenery.len()).collect();
    let centers: Vec<Vec3> = scenery.iter().map(section_center).collect();
    order.sort_by(|&a, &b| {
        centers[a].distance_squared(spawn_pos).total_cmp(&centers[b].distance_squared(spawn_pos))
    });
    for (n, &i) in order.iter().enumerate() {
        if n % 16 == 0 {
            send(LoadEvent::Progress(format!("CITY {}%", n * 100 / order.len().max(1))));
        }
        let sc = &scenery[i];
        let sky = sc.number / 100 == 26;
        let mesh = if sky { build_sky_mesh(sc, &solids, &blend_of) } else { build_section_mesh(sc, &solids, &blend_of) };
        if mesh.indices.is_empty() {
            continue;
        }
        send(LoadEvent::Section(SectionAsset { number: sc.number, sky, mesh }));
    }
    log::info!("world loaded in {:.2?}", t0.elapsed());
    send(LoadEvent::Done);
    Ok(())
}

fn blend_for(t: &Texture) -> Blend {
    match t.alpha {
        AlphaUsage::Opaque => Blend::Opaque,
        AlphaUsage::Cutout => Blend::Cutout,
        AlphaUsage::Blend if t.alpha_blend == 1 => Blend::Additive,
        AlphaUsage::Blend => Blend::Blend,
    }
}

fn section_center(s: &ScenerySection) -> Vec3 {
    if s.instances.is_empty() {
        return Vec3::splat(1e9);
    }
    let sum: Vec3 = s.instances.iter().map(|i| Vec3::from(i.position)).sum();
    sum / s.instances.len() as f32
}

fn instance_solid<'a>(
    sc: &ScenerySection,
    inst: &ug2_formats::scenery::SceneryInstance,
    solids: &'a HashMap<u32, Arc<Solid>>,
) -> Option<&'a Arc<Solid>> {
    let info = sc.infos.get(inst.info as usize)?;
    // 0x200: race-route barriers, 0x4/0x8: start/finish graphics (forward/backward).
    // The game only shows these during events.
    if inst.flags & (0x200 | 0x4 | 0x8) != 0 || info.name.starts_with("ZPM_") {
        return None;
    }
    if info.name.starts_with("PAN_") {
        return None; // distant panoramas (skyline, hills, ocean): need their own far pass
    }
    info.lods.iter().find_map(|h| solids.get(h))
}

fn d3d_color(c: u32) -> [u8; 4] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8]
}

/// Appends a solid's mesh transformed by `m`, bucketing indices by texture.
fn append_solid(
    s: &Solid,
    m: &Mat4,
    vertices: &mut Vec<GpuVertex>,
    buckets: &mut HashMap<(Blend, u32), Vec<u32>>,
    texture_of: &dyn Fn(u32) -> (u32, Blend),
    bbox: &mut (Vec3, Vec3),
) {
    let Some(mesh) = &s.mesh else { return };
    if mesh.vertices.is_empty() {
        return;
    }
    // The solid's own matrix holds its authoring pivot; vertices are already relative
    // to the placement, so only the instance transform applies (verified on all
    // 43,877 Bayview instances against their stored world bounding boxes).
    let m = *m;
    let base = vertices.len() as u32;
    for v in &mesh.vertices {
        let p = m.transform_point3(Vec3::from(v.pos));
        let n = m.transform_vector3(Vec3::from(v.normal)).normalize_or_zero();
        bbox.0 = bbox.0.min(p);
        bbox.1 = bbox.1.max(p);
        vertices.push(GpuVertex { pos: p.into(), normal: n.into(), color: d3d_color(v.color), uv: v.uv });
    }
    for g in &mesh.groups {
        let hash = s.textures.get(g.texture as usize).copied().unwrap_or(0);
        let (tex, blend) = texture_of(hash);
        let out = buckets.entry((blend, tex)).or_default();
        let idx = &mesh.indices[g.first_index as usize..(g.first_index + g.index_count) as usize];
        out.extend(idx.iter().map(|&i| base + i as u32));
    }
}

fn finish_mesh(vertices: Vec<GpuVertex>, buckets: HashMap<(Blend, u32), Vec<u32>>, bbox: (Vec3, Vec3)) -> MeshData {
    let mut keys: Vec<(Blend, u32)> = buckets.keys().copied().collect();
    keys.sort();
    let mut indices = Vec::new();
    let mut ranges = Vec::new();
    let mut buckets = buckets;
    for k in keys {
        let idx = buckets.remove(&k).unwrap();
        ranges.push(DrawRange { texture: k.1, blend: k.0, first_index: indices.len() as u32, index_count: idx.len() as u32 });
        indices.extend(idx);
    }
    MeshData { vertices, indices, ranges, bbox_min: bbox.0, bbox_max: bbox.1 }
}

fn build_section_mesh(
    sc: &ScenerySection,
    solids: &HashMap<u32, Arc<Solid>>,
    blend_of: &HashMap<u32, Blend>,
) -> MeshData {
    let mut vertices = Vec::new();
    let mut buckets = HashMap::new();
    let mut bbox = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let texture_of = |h: u32| (h, blend_of.get(&h).copied().unwrap_or(Blend::Opaque));
    for inst in &sc.instances {
        let Some(solid) = instance_solid(sc, inst, solids) else { continue };
        let m = Mat4::from_cols_array(&inst.matrix());
        append_solid(solid, &m, &mut vertices, &mut buckets, &texture_of, &mut bbox);
    }
    finish_mesh(vertices, buckets, bbox)
}

/// The sky section also holds animated aircraft; only the dome is kept, re-centred on
/// the origin so the renderer can attach it to the camera.
fn build_sky_mesh(sc: &ScenerySection, solids: &HashMap<u32, Arc<Solid>>, blend_of: &HashMap<u32, Blend>) -> MeshData {
    let mut vertices = Vec::new();
    let mut buckets = HashMap::new();
    let mut bbox = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let texture_of = |h: u32| (h, blend_of.get(&h).copied().unwrap_or(Blend::Opaque));
    for inst in &sc.instances {
        if !sc.infos[inst.info as usize].name.starts_with("SKYDOME") {
            continue;
        }
        let Some(solid) = instance_solid(sc, inst, solids) else { continue };
        let pos = Vec3::from(inst.position);
        let m = Mat4::from_translation(-pos) * Mat4::from_cols_array(&inst.matrix());
        append_solid(solid, &m, &mut vertices, &mut buckets, &texture_of, &mut bbox);
    }
    finish_mesh(vertices, buckets, bbox)
}

fn build_collision(
    scenery: &[ScenerySection],
    solids: &HashMap<u32, Arc<Solid>>,
) -> (CollisionWorld, Option<(Vec3, f32)>) {
    let mut world = CollisionWorld::new(8.0);
    // Spawn on an open-air road piece near the middle of the map.
    let target = Vec3::new(-500.0, 300.0, 0.0);
    let mut candidates: Vec<(f32, Vec3, f32)> = Vec::new();
    for sc in scenery {
        if sc.number / 100 == 26 {
            continue;
        }
        for inst in &sc.instances {
            let Some(solid) = instance_solid(sc, inst, solids) else { continue };
            let name = &sc.infos[inst.info as usize].name;
            let terrain = name.starts_with("TRN");
            let wall_only = name.starts_with("XB") || name.starts_with("XW");
            if !terrain && !wall_only {
                continue;
            }
            let Some(mesh) = &solid.mesh else { continue };
            let m = Mat4::from_cols_array(&inst.matrix());
            let pts: Vec<Vec3> = mesh.vertices.iter().map(|v| m.transform_point3(Vec3::from(v.pos))).collect();
            world.begin_owner(&solid.name);
            for t in mesh.indices.chunks_exact(3) {
                world.add(pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize], terrain);
            }
            let is_road = name.contains("ROAD") || solid.name.contains("ROAD");
            if terrain && is_road && !pts.is_empty() {
                let min = pts.iter().fold(Vec3::splat(f32::MAX), |a, &p| a.min(p));
                let max = pts.iter().fold(Vec3::splat(f32::MIN), |a, &p| a.max(p));
                let center = (min + max) * 0.5;
                let ext = max - min;
                if ext.x.max(ext.y) < 8.0 || (max.z - min.z) > 2.0 {
                    continue;
                }
                let yaw = if ext.x >= ext.y { 0.0 } else { std::f32::consts::FRAC_PI_2 };
                candidates.push((center.distance_squared(target), center, yaw));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let spawn = candidates.iter().find_map(|&(_, c, yaw)| {
        let (z, n) = world.ground(c + Vec3::Z * 3.0, 3.5, 10.0)?;
        if n.z < 0.97 {
            return None;
        }
        // Reject tunnels and underpasses: nothing solid in the 40 m above the road.
        let p = Vec3::new(c.x, c.y, z);
        if world.ground(p + Vec3::Z * 40.0, 0.0, 38.5).is_some() {
            return None;
        }
        let mut blocked = false;
        world.walls(p + Vec3::Z * 0.8, 4.0, z + 0.45, |_, _, _| blocked = true);
        (!blocked).then_some((Vec3::new(c.x, c.y, z + 0.5), yaw))
    });
    log::info!("collision: {} triangles, spawn {:?}", world.tris.len(), spawn);
    (world, spawn)
}

fn load_car(game: &GameDir, name: &str) -> anyhow::Result<(CarAsset, Vec<Texture>)> {
    let geo = game.read(&format!("CARS/{name}/GEOMETRY.BIN"))?;
    let mut textures: Vec<Texture> = find_tpks(&game.read(&format!("CARS/{name}/TEXTURES.BIN"))?)
        .into_iter()
        .flat_map(|p| p.textures)
        .collect();
    if let Ok(shared) = game.read("CARS/TEXTURES.BIN") {
        textures.extend(find_tpks(&shared).into_iter().flat_map(|p| p.textures));
    }
    let solids: Vec<Solid> = find_solid_lists(&geo).into_iter().flat_map(|l| l.solids).collect();

    let have: HashMap<u32, Blend> = textures.iter().map(|t| (t.hash, blend_for(t))).collect();
    let texture_of = |h: u32| match have.get(&h) {
        Some(&b) => (h, b),
        None => (PAINT, Blend::Opaque),
    };

    let mut body_v = Vec::new();
    let mut body_b = HashMap::new();
    let mut body_box = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut wheel = None;
    let mut used: HashSet<u32> = HashSet::new();
    for (hash, kind) in stock_part_hashes(name) {
        let Some(s) = solids.iter().find(|s| s.hash == hash) else { continue };
        used.extend(s.textures.iter().copied());
        match kind {
            PartKind::Body => append_solid(s, &Mat4::IDENTITY, &mut body_v, &mut body_b, &texture_of, &mut body_box),
            PartKind::FrontWheel => {
                let (mut v, mut b, mut bb) = (Vec::new(), HashMap::new(), (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)));
                append_solid(s, &Mat4::IDENTITY, &mut v, &mut b, &texture_of, &mut bb);
                wheel = Some(finish_mesh(v, b, bb));
            }
            PartKind::FrontBrake | PartKind::RearBrake => {}
        }
    }
    if body_v.is_empty() {
        anyhow::bail!("car {name}: no stock parts found");
    }

    let global_b = game.read("GLOBAL/GLOBALB.BUN").or_else(|_| game.read_unpacked("GLOBAL/GlobalB.lzc"));
    let info = global_b
        .ok()
        .and_then(|g| parse_car_types(&g).ok())
        .and_then(|cars| cars.into_iter().find(|c| c.name.eq_ignore_ascii_case(name)));
    let wheels = match info {
        Some(i) => i.wheels,
        None => {
            log::warn!("no CarTypeInfo for {name}, guessing wheel positions");
            let w = |x: f32, y: f32| WheelInfo { position: [x, y, 0.17], radius: 0.33, width: 0.22 };
            [w(1.35, 0.75), w(1.35, -0.75), w(-1.3, -0.75), w(-1.3, 0.75)]
        }
    };
    let textures: Vec<Texture> = textures.into_iter().filter(|t| used.contains(&t.hash)).collect();
    let car = CarAsset { name: name.to_string(), body: finish_mesh(body_v, body_b, body_box), wheel, wheels };
    Ok((car, textures))
}

/// Converts a parsed texture into an upload-ready mip chain. With BC support the
/// DXT data is passed straight through; otherwise it is decoded to RGBA8.
pub fn prepare_texture(t: &Texture, bc: bool) -> Option<PreparedTexture> {
    let levels = t.mip_levels();
    if levels.is_empty() || t.width == 0 || t.height == 0 {
        return None;
    }
    if let TexFormat::Bc(kind) = t.format {
        if bc && t.width % 4 == 0 && t.height % 4 == 0 {
            let format = match kind {
                ug2_formats::dxt::Bc::Dxt1 => wgpu::TextureFormat::Bc1RgbaUnorm,
                ug2_formats::dxt::Bc::Dxt3 => wgpu::TextureFormat::Bc2RgbaUnorm,
                ug2_formats::dxt::Bc::Dxt5 => wgpu::TextureFormat::Bc3RgbaUnorm,
            };
            let data: Vec<u8> = levels.iter().flat_map(|l| l.2.iter().copied()).collect();
            return Some(PreparedTexture {
                hash: t.hash,
                width: t.width,
                height: t.height,
                format,
                mips: levels.len() as u32,
                data,
            });
        }
    }
    let mut data = Vec::new();
    let mut mips = 0;
    for i in 0..levels.len() {
        let (_, _, rgba) = t.level_rgba8(i)?;
        data.extend_from_slice(&rgba);
        mips += 1;
    }
    Some(PreparedTexture { hash: t.hash, width: t.width, height: t.height, format: wgpu::TextureFormat::Rgba8Unorm, mips, data })
}
