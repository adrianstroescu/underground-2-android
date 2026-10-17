//! `ug2-tool fng <game-dir> <package.fng> <out.png> [ms]`: draws a frontend screen as
//! the game lays it out (INIT scripts run for `ms`, default 5 s) on a 640×480 image.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ug2_formats::compression::decompress;
use ug2_formats::feng::packages;
use ug2_formats::feng_scene::{QuadSource, Scene};
use ug2_formats::font::{fonts, Font};
use ug2_formats::fs::GameDir;
use ug2_formats::lang::Strings;
use ug2_formats::tpk::find_tpks;

struct Image {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

/// Prefers the 2004 original over a modded file (`X.bak` next to `X`).
fn read_original(game: &GameDir, rel: &str) -> Result<Vec<u8>> {
    game.read(&format!("{rel}.bak")).or_else(|_| game.read(rel)).with_context(|| rel.to_string())
}

pub fn render(game_dir: &str, name: &str, out: &Path, ms: Option<f32>) -> Result<()> {
    let game = GameDir::new(game_dir);
    let front = decompress(&read_original(&game, "FRONTEND/FrontB.lzc")?)?.into_owned();
    let mut pkgs = packages(&front);
    // In-game screens (pause, HUD, dialogs) are in GLOBAL/InGame*.bun.
    let mut ingame = Vec::new();
    for f in ["InGameCommon", "InGameRace", "InGameDrag", "InGameDrift", "InGameFreeRoam", "InGamePerformanceTuning"] {
        if let Ok(d) = game.read(&format!("GLOBAL/{f}.bun")) {
            pkgs.extend(packages(&d));
            ingame.push(d);
        }
    }
    let pkg = pkgs.into_iter().find(|p| p.name.eq_ignore_ascii_case(name)).context("no such package")?;
    let lang = read_original(&game, "LANGUAGES/English.bin")?;
    let strings = Strings::parse(&lang);
    let globala = game.read("GLOBAL/GLOBALA.BUN")?;
    let mut all_fonts: Vec<Font> = fonts(&lang);
    all_fonts.extend(fonts(&globala));

    // Textures by name hash (TPK names are cut at 24 characters).
    let mut textures: HashMap<u32, Image> = HashMap::new();
    let langtex = game.read("LANGUAGES/LanguageTextures.bin").unwrap_or_default();
    let globalb = game.read("GLOBAL/GLOBALB.BUN").unwrap_or_default();
    for file in [&front, &globalb, &globala, &langtex].into_iter().chain(ingame.iter()) {
        for p in find_tpks(file) {
            for t in &p.textures {
                if let Some((w, h, rgba)) = t.level_rgba8(0) {
                    textures.entry(t.hash).or_insert(Image { w, h, rgba });
                }
            }
        }
    }

    if std::env::var("FNG_INFO").is_ok() {
        info(&pkg, &strings);
    }
    let mut scene = Scene::new(pkg);
    scene.advance(16.0);
    // What the game sends when a screen is shown (see the package's responses).
    for m in std::env::var("FNG_SEND").unwrap_or_else(|_| "CALL PACKAGE,CALL FRAMES".into()).split(',') {
        scene.send_named(m.trim());
    }
    let total = ms.unwrap_or(5000.0);
    let mut t = 0.0;
    while t < total {
        scene.advance(16.0);
        t += 16.0;
    }
    // FNG_POS=name,name...: print the objects' state after the scripts ran.
    if let Ok(names) = std::env::var("FNG_POS") {
        for n in names.split(',') {
            let hash = ug2_formats::bin_hash(&n.trim().to_ascii_uppercase());
            for i in (0..scene.package.objects.len()).filter(|&i| scene.package.objects[i].hash == hash) {
                let v = scene.values(i);
                println!("{n}: pos ({:.1}, {:.1}, {:.1}) size ({:.2}, {:.2}) pivot ({:.1}, {:.1}) hidden {:?}", v[7], v[8], v[9], v[14], v[15], v[4], v[5], scene.states[i].hidden);
            }
        }
    }
    // FNG_TRACKS=name: print the object's script tracks.
    if let Ok(n) = std::env::var("FNG_TRACKS") {
        let hash = ug2_formats::bin_hash(&n.trim().to_ascii_uppercase());
        for o in scene.package.objects.iter().filter(|o| o.hash == hash) {
            println!("data {:?}", &o.data[..20]);
            for sc in &o.scripts {
                println!("script {:08x} len {}", sc.id, sc.length);
                for t in &sc.tracks {
                    println!("  off {} base {:?} keys {:?}", t.offset, t.base, t.keys);
                }
            }
        }
    }
    let quads = scene.layout(&strings, &all_fonts);
    let (w, h) = (640usize, 480usize);
    let mut fb = vec![0f32; w * h * 3];
    for px in fb.chunks_exact_mut(3) {
        px.copy_from_slice(&[0.12, 0.12, 0.14]);
    }
    let mut missing = std::collections::BTreeSet::new();
    for q in &quads {
        let tex = match &q.source {
            QuadSource::Texture(n) | QuadSource::Font(n) => {
                let t = textures.get(&ug2_formats::bin_hash(n));
                if t.is_none() {
                    missing.insert(n.clone());
                }
                t
            }
            QuadSource::TextureHash(h) => textures.get(h),
            QuadSource::Solid => None,
        };
        // Two triangles TL-TR-BR and TL-BR-BL.
        for tri in [[0usize, 1, 2], [0, 2, 3]] {
            let p = tri.map(|k| q.corners[k]);
            let (minx, maxx) = (p.iter().map(|v| v[0]).fold(f32::MAX, f32::min), p.iter().map(|v| v[0]).fold(f32::MIN, f32::max));
            let (miny, maxy) = (p.iter().map(|v| v[1]).fold(f32::MAX, f32::min), p.iter().map(|v| v[1]).fold(f32::MIN, f32::max));
            let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            if area.abs() < 1e-6 {
                continue;
            }
            for y in (miny.max(0.0) as usize)..=(maxy.min(h as f32 - 1.0).max(0.0) as usize) {
                for x in (minx.max(0.0) as usize)..=(maxx.min(w as f32 - 1.0).max(0.0) as usize) {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p[1][0] - fx) * (p[2][1] - fy) - (p[2][0] - fx) * (p[1][1] - fy)) / area;
                    let w1 = ((p[2][0] - fx) * (p[0][1] - fy) - (p[0][0] - fx) * (p[2][1] - fy)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let lerp2 = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| [a[0] * w0 + b[0] * w1 + c[0] * w2, a[1] * w0 + b[1] * w1 + c[1] * w2];
                    let uv = lerp2(q.uv[tri[0]], q.uv[tri[1]], q.uv[tri[2]]);
                    let col: [f32; 4] = std::array::from_fn(|k| {
                        (q.colors[tri[0]][k] as f32 * w0 + q.colors[tri[1]][k] as f32 * w1 + q.colors[tri[2]][k] as f32 * w2) / 255.0
                    });
                    let texel = match tex {
                        Some(t) => {
                            let tx = ((uv[0].clamp(0.0, 1.0) * t.w as f32) as u32).min(t.w - 1);
                            let ty = ((uv[1].clamp(0.0, 1.0) * t.h as f32) as u32).min(t.h - 1);
                            let o = ((ty * t.w + tx) * 4) as usize;
                            std::array::from_fn::<f32, 4, _>(|k| t.rgba[o + k] as f32 / 255.0)
                        }
                        None => [1.0; 4],
                    };
                    let a = texel[3] * col[3];
                    let o = (y * w + x) * 3;
                    for k in 0..3 {
                        fb[o + k] = fb[o + k] * (1.0 - a) + texel[k] * col[k] * a;
                    }
                }
            }
        }
    }
    let rgba: Vec<u8> = fb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 1.0].map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)).collect();
    crate::write_png(out, w as u32, h as u32, &rgba)?;
    println!("{} quads, {} objects", quads.len(), scene.package.objects.len());
    if !missing.is_empty() {
        println!("missing textures: {missing:?}");
    }
    Ok(())
}

/// Prints the objects, their scripts and responses, with names from FNG_NAMES (a folder
/// with FngObjects.txt, FngScripts.txt, FngMessages.txt) when given.
fn info(pkg: &ug2_formats::feng::Package, strings: &Strings) {
    let mut names: HashMap<u32, String> = HashMap::new();
    if let Ok(dir) = std::env::var("FNG_NAMES") {
        for f in ["FngObjects.txt", "FngScripts.txt", "FngMessages.txt"] {
            if let Ok(t) = std::fs::read_to_string(Path::new(&dir).join(f)) {
                for l in t.lines() {
                    let n = l.trim();
                    if !n.is_empty() {
                        names.insert(ug2_formats::bin_hash(&n.to_ascii_uppercase()), n.to_string());
                    }
                }
            }
        }
    }
    let nm = |h: u32| names.get(&h).cloned().unwrap_or_else(|| format!("{h:08x}"));
    for r in pkg.resources.iter() {
        println!("res {:?} {}", r.kind, r.name);
    }
    for (i, o) in pkg.objects.iter().enumerate() {
        let d = &o.data;
        let text = o.text.as_ref().map(|t| format!(" text={:?} label={:?} j={}", t.placeholder, strings.by_hash(t.label), t.justify)).unwrap_or_default();
        let res = o.resource.and_then(|r| pkg.resources.get(r)).map(|r| r.name.clone()).unwrap_or_default();
        println!(
            "{i:3} {:?} {} flags={:08x} parent={:?} pos=({:.0},{:.0},{:.0}) size=({:.1},{:.1}) rgba=({},{},{},{}) {res}{text}",
            o.kind, nm(o.hash), o.flags, o.parent.and_then(|p| pkg.objects.iter().position(|q| q.guid == p)),
            d[7], d[8], d[9], d[14], d[15], d[2], d[1], d[0], d[3]
        );
        for s in &o.scripts {
            println!("      script {} len={} flags={} tracks={} chain={:?}", nm(s.id), s.length, s.flags, s.tracks.len(), s.chain.map(nm));
        }
        for r in &o.responses {
            let cmds: Vec<String> = r.commands.iter().map(|c| format!("{:x}:{}->{:x}", c.id, nm(c.param), c.target)).collect();
            println!("      on {} {}", nm(r.message), cmds.join(" "));
        }
    }
    for r in &pkg.responses {
        let cmds: Vec<String> = r.commands.iter().map(|c| format!("{:x}:{}->{:x}", c.id, nm(c.param), c.target)).collect();
        println!("pkg on {} {}", nm(r.message), cmds.join(" "));
    }
}
