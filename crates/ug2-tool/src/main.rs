//! Developer tool: dump chunk trees, validate parsers against a game install,
//! export meshes (OBJ) and textures (PNG).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ug2_formats::chunk::walk;
use ug2_formats::fs::GameDir;
use ug2_formats::solid::{find_solid_lists, Solid};
use ug2_formats::tpk::find_tpks;
use ug2_formats::track::{parse_region, parse_streaming_sections};

const USAGE: &str = "usage:
  ug2-tool chunks <file> [max-depth]
  ug2-tool car <game-dir> <CAR> [out-dir]     parse a car, optionally export OBJ+PNG
  ug2-tool track <game-dir> [L4RA]            parse every streaming section, print stats
  ug2-tool textures <file> <out-dir>          export every texture in a file to PNG";

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("chunks") if args.len() >= 2 => chunks(&args[1], args.get(2).and_then(|d| d.parse().ok()).unwrap_or(2)),
        Some("car") if args.len() >= 3 => car(&args[1], &args[2], args.get(3).map(PathBuf::from)),
        Some("track") if args.len() >= 2 => track(&args[1], args.get(2).map(String::as_str).unwrap_or("L4RA")),
        Some("textures") if args.len() >= 3 => textures(&args[1], Path::new(&args[2])),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn read_any(path: &str) -> Result<Vec<u8>> {
    let data = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    Ok(ug2_formats::compression::decompress(&data)?.into_owned())
}

fn chunks(path: &str, max_depth: usize) -> Result<()> {
    let data = read_any(path)?;
    let mut hist: BTreeMap<(usize, u32), usize> = BTreeMap::new();
    walk(&data, &mut |depth, c| {
        *hist.entry((depth, c.id)).or_default() += 1;
    });
    let mut shown = 0;
    walk(&data, &mut |depth, c| {
        if depth <= max_depth && shown < 200 {
            println!("{}{:08X} size={} @{:#x}", "  ".repeat(depth), c.id, c.data.len(), c.offset);
            shown += 1;
        }
    });
    println!("--- histogram (depth, id): count");
    for ((d, id), n) in hist {
        println!("  d{d} {id:08X}: {n}");
    }
    Ok(())
}

fn car(game: &str, name: &str, out: Option<PathBuf>) -> Result<()> {
    let game = GameDir::new(game);
    let geo = game.read(&format!("CARS/{name}/GEOMETRY.BIN"))?;
    let tex = game.read(&format!("CARS/{name}/TEXTURES.BIN"))?;
    let lists = find_solid_lists(&geo);
    let packs = find_tpks(&tex);
    let solids: Vec<&Solid> = lists.iter().flat_map(|l| &l.solids).collect();
    println!("{name}: {} solid lists, {} solids", lists.len(), solids.len());
    for p in &packs {
        println!("  tpk {} ({}) {} textures", p.name, p.path, p.textures.len());
        let mut fmts: BTreeMap<String, usize> = BTreeMap::new();
        for t in &p.textures {
            *fmts.entry(format!("{:?}", t.format)).or_default() += 1;
        }
        println!("    formats {fmts:?}");
    }
    let wanted = ug2_formats::car::stock_parts(name);
    let mut stock: Vec<&Solid> = Vec::new();
    for (full, kind) in &wanted {
        let h = ug2_formats::bin_hash(full);
        match solids.iter().find(|s| s.hash == h) {
            Some(s) => stock.push(s),
            None => println!("    (missing {full} {kind:?})"),
        }
    }
    println!("  stock parts: {}", stock.len());
    for s in &stock {
        let m = s.mesh.as_ref();
        println!(
            "    {:28} verts={:5} tris={:5} groups={:2} stride={:2} tex={} bbox={:?}..{:?}",
            s.name,
            m.map_or(0, |m| m.vertices.len()),
            m.map_or(0, |m| m.indices.len() / 3),
            m.map_or(0, |m| m.groups.len()),
            m.map_or(0, |m| m.vertex_stride),
            s.textures.len(),
            s.bbox_min,
            s.bbox_max
        );
    }
    if let Some(out) = out {
        std::fs::create_dir_all(&out)?;
        export_obj(&out.join(format!("{name}.obj")), &stock)?;
        for p in &packs {
            for t in &p.textures {
                if let Some((w, h, rgba)) = t.level_rgba8(0) {
                    write_png(&out.join(format!("{:08X}_{}.png", t.hash, sanitize(&t.name))), w, h, &rgba)?;
                }
            }
        }
        println!("exported to {}", out.display());
    }
    Ok(())
}

fn export_obj(path: &Path, solids: &[&Solid]) -> Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut base = 1usize;
    for s in solids {
        let Some(m) = &s.mesh else { continue };
        writeln!(f, "o {}", s.name)?;
        for v in &m.vertices {
            writeln!(f, "v {} {} {}", v.pos[0], v.pos[2], -v.pos[1])?;
            writeln!(f, "vt {} {}", v.uv[0], 1.0 - v.uv[1])?;
        }
        for g in &m.groups {
            let tex = s.textures.get(g.texture as usize).copied().unwrap_or(0);
            writeln!(f, "usemtl tex_{tex:08X}")?;
            let idx = &m.indices[g.first_index as usize..(g.first_index + g.index_count) as usize];
            for t in idx.chunks_exact(3) {
                let (a, b, c) = (t[0] as usize + base, t[1] as usize + base, t[2] as usize + base);
                writeln!(f, "f {a}/{a} {b}/{b} {c}/{c}")?;
            }
        }
        base += m.vertices.len();
    }
    Ok(())
}

fn track(game: &str, loc: &str) -> Result<()> {
    let game = GameDir::new(game);
    let index = game.read(&format!("TRACKS/{loc}.BUN"))?;
    let stream = game.read(&format!("TRACKS/STREAM{loc}.BUN"))?;
    let sections = parse_streaming_sections(&index)?;
    println!("{} sections, stream {} MB", sections.len(), stream.len() >> 20);
    let mut strides: BTreeMap<usize, usize> = BTreeMap::new();
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut alpha: BTreeMap<String, usize> = BTreeMap::new();
    let (mut n_solids, mut n_tex, mut n_inst, mut n_tris) = (0usize, 0usize, 0usize, 0usize);
    let mut solid_hashes = std::collections::HashSet::new();
    let mut missing_lod = 0usize;
    let mut insts = Vec::new();
    let t0 = std::time::Instant::now();
    for s in &sections {
        if s.offset + s.size > stream.len() {
            println!("  section {} ({}) out of range", s.name, s.number);
            continue;
        }
        let d = parse_region(&stream, s.offset, s.size);
        for l in &d.solids {
            for so in &l.solids {
                n_solids += 1;
                solid_hashes.insert(so.hash);
                if let Some(m) = &so.mesh {
                    *strides.entry(m.vertex_stride).or_default() += 1;
                    n_tris += m.indices.len() / 3;
                }
            }
        }
        for p in &d.textures {
            for t in &p.textures {
                n_tex += 1;
                *formats.entry(format!("{:?}", t.format)).or_default() += 1;
                *alpha.entry(format!("{:?}/blend{}", t.alpha, t.alpha_blend)).or_default() += 1;
            }
        }
        for sc in d.scenery {
            n_inst += sc.instances.len();
            for i in &sc.instances {
                insts.push(sc.infos[i.info as usize].clone());
            }
        }
    }
    let mut unresolved: BTreeMap<String, usize> = BTreeMap::new();
    for info in &insts {
        if !info.lods.iter().any(|h| solid_hashes.contains(h)) {
            missing_lod += 1;
            *unresolved.entry(info.name.clone()).or_default() += 1;
        }
    }
    let mut top: Vec<_> = unresolved.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, n) in top.iter().take(15) {
        let full = insts.iter().find(|i| &i.name == name).unwrap();
        println!("  unresolved {name} x{n} lods={:08X?} hash(name)={:08X}", full.lods, ug2_formats::bin_hash(name));
    }
    // Solids drawn with textures that no texture pack provides.
    let mut tex_hashes = std::collections::HashSet::new();
    let mut solid_tex: BTreeMap<String, (Vec<u32>, usize)> = BTreeMap::new();
    for s in &sections {
        if s.offset + s.size > stream.len() {
            continue;
        }
        let d = parse_region(&stream, s.offset, s.size);
        for p in &d.textures {
            tex_hashes.extend(p.textures.iter().map(|t| t.hash));
        }
        for l in &d.solids {
            for so in &l.solids {
                let n = so.mesh.as_ref().map_or(0, |m| m.indices.len() / 3);
                solid_tex.insert(so.name.clone(), (so.textures.clone(), n));
            }
        }
    }
    let mut missing: Vec<(String, usize, usize)> = solid_tex
        .iter()
        .filter(|(_, (t, n))| *n > 0 && !t.is_empty())
        .map(|(name, (t, n))| (name.clone(), t.iter().filter(|h| !tex_hashes.contains(h)).count(), *n))
        .filter(|(_, m, _)| *m > 0)
        .collect();
    missing.sort_by(|a, b| b.2.cmp(&a.2));
    println!("solids with missing textures: {}", missing.len());
    for (name, m, n) in missing.iter().take(25) {
        println!("  {name:30} missing {m} tris {n}");
    }
    println!("parsed in {:.2?}", t0.elapsed());
    println!("solids {n_solids} (tris {n_tris}), textures {n_tex}, instances {n_inst}, unresolved LOD-A {missing_lod}");
    println!("vertex strides {strides:?}");
    println!("texture formats {formats:?}");
    println!("alpha usage {alpha:?}");
    for s in sections.iter().take(12) {
        println!("  {:4} #{:5} @{:#010x} {:9} bytes", s.name, s.number, s.offset, s.size);
    }
    Ok(())
}

fn textures(path: &str, out: &Path) -> Result<()> {
    let data = read_any(path)?;
    let packs = find_tpks(&data);
    if packs.is_empty() {
        bail!("no texture packs in {path}");
    }
    std::fs::create_dir_all(out)?;
    for p in &packs {
        for t in &p.textures {
            match t.level_rgba8(0) {
                Some((w, h, rgba)) => write_png(&out.join(format!("{}.png", sanitize(&t.name))), w, h, &rgba)?,
                None => println!("skip {} ({:?})", t.name, t.format),
            }
        }
        println!("{}: {} textures", p.name, p.textures.len());
    }
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(f, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}
