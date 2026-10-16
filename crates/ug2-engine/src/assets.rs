//! Background loading: parses the original game files on a worker thread and turns
//! them into GPU-ready meshes and textures, streamed to the main thread as events.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use ug2_formats::carinfo::CarUsage;
use ug2_formats::fs::GameDir;
use ug2_formats::scenery::ScenerySection;
use ug2_formats::solid::Solid;
use ug2_formats::tpk::{find_tpks, AlphaUsage, TexFormat, Texture};
use ug2_formats::track::{parse_region, parse_streaming_sections};

use crate::cars::{load_car, CarModel, SharedCarData};
use crate::sound::{Effects, EngineSound};
use crate::collision::CollisionWorld;
use ug2_formats::parts::PartLibrary;
use crate::roads::RoadData;

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

/// How a draw range is lit; cars use several, the world only `Normal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    Normal,
    /// Player paint: drawn untextured with the paint colour.
    Paint,
    /// Player paint with the car's detail atlas (door lines, badges, grille) blended in
    /// by the texture's alpha: what the game composites into `<CAR>_SKIN1`.
    PaintDetail,
    /// Traffic paint job; the texture can be swapped per car.
    Skin,
    Glass,
    Headlight,
    Brakelight,
    Shadow,
}

/// Texture hash used for car paint (groups whose texture is supplied by a shader).
pub const PAINT: u32 = 0xFFFF_FFFF;

#[derive(Clone, Debug)]
pub struct DrawRange {
    pub texture: u32,
    pub blend: Blend,
    pub role: Role,
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

pub enum LoadEvent {
    Progress(String),
    Textures(Vec<PreparedTexture>),
    /// Rim styles for the garage, in catalogue order (index + 1 = `PartSet::rim`).
    Rims(Vec<String>),
    /// Car lists from CarTypeInfo.
    CarList { racers: Vec<String>, traffic: Vec<String>, perfs: Vec<(String, crate::vehicle::Perf)> },
    Car(Box<CarModel>),
    /// Model key of a car that could not be loaded.
    CarFailed(String),
    Effects(Box<Effects>),
    Engine { car: String, sound: Box<EngineSound> },
    Roads(Box<RoadData>),
    Collision(CollisionWorld),
    Spawn { pos: Vec3, yaw: f32 },
    Section(SectionAsset),
    /// Street lamps and other world lights, drawn as glows at night.
    Lights(Vec<ug2_formats::light::WorldLight>),
    Done,
    Failed(String),
}

#[derive(Clone)]
pub struct LoaderConfig {
    pub car: String,
    pub parts: ug2_formats::car::PartSet,
    pub location: String,
    pub bc_textures: bool,
}

/// Requests the game sends to the loader thread.
pub enum LoadRequest {
    Car { name: String, parts: ug2_formats::car::PartSet },
}

/// Background loader: loads the player's car and the world, then serves car requests.
pub struct Loader {
    pub events: Receiver<LoadEvent>,
    requests: Sender<LoadRequest>,
}

impl Loader {
    pub fn request(&self, r: LoadRequest) {
        let _ = self.requests.send(r);
    }
}

pub fn spawn_loader(game: GameDir, cfg: LoaderConfig) -> Loader {
    let (tx, rx) = channel();
    let (req_tx, req_rx) = channel::<LoadRequest>();
    std::thread::Builder::new()
        .name("ug2-loader".into())
        .spawn(move || {
            let shared = SharedCarData::load(&game);
            let racers = shared.names(CarUsage::Racer);
            let traffic = shared.names(CarUsage::Traffic);
            let _ = tx.send(LoadEvent::CarList { racers, traffic, perfs: shared.perfs() });
            // The part catalogue costs a moment to build: only up front if the saved car uses it.
            let mut lib = None;
            send_car(&game, &shared, &mut lib, &cfg, &cfg.car, &cfg.parts, &tx);
            let _ = tx.send(LoadEvent::Effects(Box::new(Effects::load(&game))));
            if let Err(e) = load_world(&game, &cfg, &tx) {
                log::error!("load failed: {e:#}");
                let _ = tx.send(LoadEvent::Failed(format!("{e:#}")));
                return;
            }
            let rims = lib.get_or_insert_with(|| PartLibrary::scan(&game)).rims.iter().map(|r| r.label.clone()).collect();
            let _ = tx.send(LoadEvent::Rims(rims));
            for req in req_rx {
                match req {
                    LoadRequest::Car { name, parts } => send_car(&game, &shared, &mut lib, &cfg, &name, &parts, &tx),
                }
            }
        })
        .expect("spawn loader thread");
    Loader { events: rx, requests: req_tx }
}

fn send_car(
    game: &GameDir,
    shared: &SharedCarData,
    lib: &mut Option<PartLibrary>,
    cfg: &LoaderConfig,
    name: &str,
    parts: &ug2_formats::car::PartSet,
    tx: &Sender<LoadEvent>,
) {
    let t0 = std::time::Instant::now();
    let needs_lib = parts.rim > 0 || parts.spoiler > 0;
    let lib = if needs_lib { lib.get_or_insert_with(|| PartLibrary::scan(game)) } else { lib.get_or_insert_with(PartLibrary::default) };
    match load_car(game, shared, lib, name, parts) {
        Ok((car, textures)) => {
            let prepared = textures.iter().filter_map(|t| prepare_texture(t, cfg.bc_textures)).collect();
            let _ = tx.send(LoadEvent::Textures(prepared));
            log::info!("car {name} loaded in {:.0?}", t0.elapsed());
            let _ = tx.send(LoadEvent::Car(Box::new(car)));
            if let Some(sound) = EngineSound::load(game, name) {
                let _ = tx.send(LoadEvent::Engine { car: name.to_string(), sound: Box::new(sound) });
            }
        }
        Err(e) => {
            log::error!("car {name}: {e:#}");
            let _ = tx.send(LoadEvent::CarFailed(crate::cars::model_key(name, parts)));
        }
    }
}

fn load_world(game: &GameDir, cfg: &LoaderConfig, tx: &Sender<LoadEvent>) -> anyhow::Result<()> {
    let t0 = std::time::Instant::now();
    let send = |e: LoadEvent| {
        let _ = tx.send(e);
    };

    send(LoadEvent::Progress("READING TRACK".into()));
    let index = game.read(&format!("TRACKS/{}.BUN", cfg.location))?;
    let stream = game.read(&format!("TRACKS/STREAM{}.BUN", cfg.location))?;
    let sections = parse_streaming_sections(&index)?;

    send(LoadEvent::Progress("PARSING TRACK".into()));
    let mut solids: HashMap<u32, Arc<Solid>> = HashMap::new();
    let mut textures: Vec<Texture> = Vec::new();
    let mut scenery: Vec<ScenerySection> = Vec::new();
    let mut lights = Vec::new();
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
        lights.extend(d.lights);
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
    log::info!("{} world lights", lights.len());
    send(LoadEvent::Lights(lights));
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
    match RoadData::load(game, &cfg.location) {
        Ok(r) => send(LoadEvent::Roads(Box::new(r))),
        Err(e) => log::warn!("no road data: {e:#}"),
    }
    send(LoadEvent::Done);
    Ok(())
}

pub fn blend_for(t: &Texture) -> Blend {
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

pub type Buckets = HashMap<(Blend, u32, Role), Vec<u32>>;

/// Appends a solid's mesh transformed by `m`, bucketing indices by texture.
pub fn append_solid(
    s: &Solid,
    m: &Mat4,
    vertices: &mut Vec<GpuVertex>,
    buckets: &mut Buckets,
    texture_of: &dyn Fn(u32) -> (u32, Blend, Role),
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
        let (tex, blend, role) = texture_of(hash);
        let out = buckets.entry((blend, tex, role)).or_default();
        let idx = &mesh.indices[g.first_index as usize..(g.first_index + g.index_count) as usize];
        out.extend(idx.iter().map(|&i| base + i as u32));
    }
}

pub fn finish_mesh(vertices: Vec<GpuVertex>, buckets: Buckets, bbox: (Vec3, Vec3)) -> MeshData {
    let mut keys: Vec<(Blend, u32, Role)> = buckets.keys().copied().collect();
    keys.sort();
    let mut indices = Vec::new();
    let mut ranges = Vec::new();
    let mut buckets = buckets;
    for k in keys {
        let idx = buckets.remove(&k).unwrap();
        ranges.push(DrawRange {
            texture: k.1,
            blend: k.0,
            role: k.2,
            first_index: indices.len() as u32,
            index_count: idx.len() as u32,
        });
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
    let texture_of = |h: u32| (h, blend_of.get(&h).copied().unwrap_or(Blend::Opaque), Role::Normal);
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
    let texture_of = |h: u32| (h, blend_of.get(&h).copied().unwrap_or(Blend::Opaque), Role::Normal);
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
