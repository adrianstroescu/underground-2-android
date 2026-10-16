//! EA FEng frontend packages (`.fng`): the menu screens and HUD of the original game.
//!
//! Layout (see CLAUDE.md, "FEng"): a `FEn\xE7` chunk holding `PkHd`, `TypS`, `Res\xCC`
//! (`RsNm` names, `RsRq` requests), `Obj\xCC` (`Butn`, then one `FOb\xEA` per object with
//! `ObjD` tags, `Scrp` scripts and `MsgR` responses), `PkgR` and `Targ`. Chunks are
//! u32 id + u32 size; inside ObjD/Scrp/MsgR the records are u16 tag + u16 size. All
//! little-endian. In `FrontB.lzc` each package is a chunk 0x30210 = u32 name hash +
//! a JDLZ/HUFF block; chunk 0x30203 holds an uncompressed package.

use crate::bytes::{f32_at, u16_at, u32_at};
use crate::compression::decompress;

const PACKAGE: u32 = 0xE76E_4546;
const PKHD: u32 = 0x6448_6B50;
const RES: u32 = 0xCC73_6552;
const RSNM: u32 = 0x6D4E_7352;
const RSRQ: u32 = 0x7152_7352;
const OBJS: u32 = 0xCC6A_624F;
const OBJ: u32 = 0xEA62_4F46;
const OBJD: u32 = 0x446A_624F;
const SCRP: u32 = 0x7072_6353;
const MSGR: u32 = 0x5267_734D;
const PKGR: u32 = 0x5267_6B50;

/// Hash of "INIT", the script every object starts with.
pub const INIT: u32 = 0x0017_44B3;

/// Words of the per-object data block (`SA`) that scripts animate.
pub mod field {
    /// B, G, R, A as integers 0..255.
    pub const COLOR: usize = 0;
    pub const PIVOT: usize = 4;
    pub const POSITION: usize = 7;
    /// Quaternion x, y, z, w.
    pub const ROTATION: usize = 10;
    pub const SIZE: usize = 14;
    pub const UV_TOP_LEFT: usize = 17;
    pub const UV_BOTTOM_RIGHT: usize = 19;
    /// Corner colours of a coloured image (TL, TR, BR, BL).
    pub const CORNER_COLORS: usize = 21;
    pub const WORDS: usize = 37;
}

pub mod flags {
    pub const INVISIBLE: u32 = 0x1;
    pub const PC_ONLY: u32 = 0x8;
    pub const CONSOLE_ONLY: u32 = 0x40;
    pub const DONT_NAVIGATE: u32 = 0x8_0000;
    pub const CODE_SUPPLIED_RESOURCE: u32 = 0x20_0000;
    pub const IS_BUTTON: u32 = 0x1000_0000;
    pub const HIDE_IN_EDIT: u32 = 0x8000_0000;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    None,
    Image,
    Font,
    Movie,
    Other(u32),
}

#[derive(Clone, Debug)]
pub struct Resource {
    pub id: u32,
    pub name: String,
    pub kind: ResourceKind,
}

impl Resource {
    /// Texture name an image resource refers to: the file stem, upper-cased
    /// (`U2_Menu_Box.tga` → `U2_MENU_BOX`).
    pub fn texture_name(&self) -> String {
        let stem = self.name.rsplit(['\\', '/']).next().unwrap_or(&self.name);
        let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
        stem.to_ascii_uppercase()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Image,
    Text,
    Group,
    Movie,
    ColoredImage,
    SimpleImage,
    MultiImage,
    Other(u32),
}

#[derive(Clone, Debug, Default)]
pub struct Text {
    /// Hash of the localised string's label (`LANGUAGES/*.bin`).
    pub label: u32,
    /// Design-time text, shown when the label is not found.
    pub placeholder: String,
    /// Justification bits: 1 centre, 2 right, 4 vertical middle, 8 bottom.
    pub justify: u32,
    pub leading: i32,
    pub max_width: u32,
}

#[derive(Clone, Debug)]
pub struct Track {
    /// Word offset of the animated field (see [`field`]).
    pub offset: usize,
    /// 3 vec2, 4 vec3, 5 quaternion, 6 colour.
    pub param: u8,
    pub interp: u8,
    pub action: u8,
    pub length: u32,
    pub base: Vec<f32>,
    /// (time ms, delta) keys after the base key.
    pub keys: Vec<(i32, Vec<f32>)>,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub message: u32,
    pub target: u32,
    pub time: u32,
}

#[derive(Clone, Debug)]
pub struct Script {
    pub id: u32,
    pub length: u32,
    pub flags: u32,
    pub chain: Option<u32>,
    pub tracks: Vec<Track>,
    pub events: Vec<Event>,
}

impl Script {
    pub fn looping(&self) -> bool {
        self.flags & 1 != 0
    }
}

#[derive(Clone, Debug)]
pub struct Command {
    pub id: u32,
    pub param: u32,
    pub text: Option<String>,
    pub target: u32,
}

#[derive(Clone, Debug)]
pub struct Response {
    pub message: u32,
    pub commands: Vec<Command>,
}

#[derive(Clone, Debug)]
pub struct Object {
    pub guid: u32,
    pub hash: u32,
    pub flags: u32,
    pub kind: ObjectKind,
    /// Index into [`Package::resources`].
    pub resource: Option<usize>,
    pub parent: Option<u32>,
    pub data: [f32; field::WORDS],
    pub text: Option<Text>,
    pub scripts: Vec<Script>,
    pub responses: Vec<Response>,
}

impl Object {
    pub fn script(&self, id: u32) -> Option<&Script> {
        self.scripts.iter().find(|s| s.id == id)
    }
}

#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,
    pub resources: Vec<Resource>,
    pub objects: Vec<Object>,
    pub responses: Vec<Response>,
}

impl Package {
    /// Parses a `FEn\xE7` chunk (starting at its header).
    pub fn parse(b: &[u8]) -> Option<Package> {
        if u32_at(b, 0).ok()? != PACKAGE {
            return None;
        }
        let size = u32_at(b, 4).ok()? as usize;
        let body = b.get(8..8 + size.min(b.len() - 8))?;
        let mut pkg = Package { name: String::new(), resources: Vec::new(), objects: Vec::new(), responses: Vec::new() };
        let mut names: &[u8] = &[];
        for (id, p) in chunks(body) {
            match id {
                PKHD => {
                    let nl = u32_at(p, 0x10).ok()? as usize;
                    pkg.name = cstr(p.get(0x18..0x18 + nl)?);
                }
                RES => {
                    for (id, q) in chunks(p) {
                        match id {
                            RSNM => names = q,
                            RSRQ => {
                                let n = u32_at(q, 0).ok()? as usize;
                                for i in 0..n {
                                    let o = 4 + i * 24;
                                    let (Ok(rid), Ok(name), Ok(kind)) = (u32_at(q, o), u32_at(q, o + 4), u32_at(q, o + 8)) else { break };
                                    let name = names.get(name as usize..).map(cstr).unwrap_or_default();
                                    let kind = match kind {
                                        0 => ResourceKind::None,
                                        1 => ResourceKind::Image,
                                        2 => ResourceKind::Font,
                                        4 => ResourceKind::Movie,
                                        k => ResourceKind::Other(k),
                                    };
                                    pkg.resources.push(Resource { id: rid, name, kind });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                OBJS => {
                    for (id, q) in chunks(p) {
                        if id == OBJ {
                            if let Some(o) = parse_object(q) {
                                pkg.objects.push(o);
                            }
                        }
                    }
                }
                PKGR => pkg.responses = parse_responses(p),
                _ => {}
            }
        }
        Some(pkg)
    }

    pub fn object(&self, guid: u32) -> Option<&Object> {
        self.objects.iter().find(|o| o.guid == guid)
    }

    pub fn object_by_hash(&self, hash: u32) -> Option<&Object> {
        self.objects.iter().find(|o| o.hash == hash)
    }
}

/// Every package in a decompressed `FrontB` / `InGame*` file.
pub fn packages(file: &[u8]) -> Vec<Package> {
    let mut out = Vec::new();
    for (id, p) in chunks(file) {
        let pkg = match id {
            0x0003_0210 => p.get(4..).and_then(|c| decompress(c).ok()).and_then(|d| Package::parse(&d)),
            0x0003_0203 => Package::parse(p),
            _ => None,
        };
        out.extend(pkg);
    }
    out
}

fn chunks(b: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    let mut o = 0usize;
    std::iter::from_fn(move || {
        if o + 8 > b.len() {
            return None;
        }
        let id = u32_at(b, o).ok()?;
        let size = u32_at(b, o + 4).ok()? as usize;
        let p = b.get(o + 8..(o + 8 + size).min(b.len()))?;
        o += 8 + size;
        Some((id, p))
    })
}

fn tags(b: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut o = 0usize;
    std::iter::from_fn(move || {
        if o + 4 > b.len() {
            return None;
        }
        let id = u16_at(b, o).ok()?;
        let size = u16_at(b, o + 2).ok()? as usize;
        let p = b.get(o + 4..(o + 4 + size).min(b.len()))?;
        o += 4 + size;
        Some((id, p))
    })
}

fn tag(s: &[u8; 2]) -> u16 {
    u16::from_le_bytes(*s)
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end].iter().map(|&c| c as char).collect()
}

fn parse_object(b: &[u8]) -> Option<Object> {
    let mut o = Object {
        guid: 0,
        hash: 0,
        flags: 0,
        kind: ObjectKind::Other(0),
        resource: None,
        parent: None,
        data: [0.0; field::WORDS],
        text: None,
        scripts: Vec::new(),
        responses: Vec::new(),
    };
    for (id, p) in chunks(b) {
        match id {
            OBJD => {
                let mut text = Text::default();
                let mut is_text = false;
                for (t, q) in tags(p) {
                    let u = |off: usize| u32_at(q, off).unwrap_or(0);
                    match t {
                        x if x == tag(b"Ot") => {
                            o.kind = match u(0) {
                                1 => ObjectKind::Image,
                                2 => ObjectKind::Text,
                                5 => ObjectKind::Group,
                                7 => ObjectKind::Movie,
                                9 => ObjectKind::ColoredImage,
                                11 => ObjectKind::SimpleImage,
                                12 => ObjectKind::MultiImage,
                                k => ObjectKind::Other(k),
                            }
                        }
                        x if x == tag(b"Oh") => o.hash = u(0),
                        x if x == tag(b"OP") => {
                            o.guid = u(0);
                            o.hash = u(4);
                            o.flags = u(8);
                            let r = u(12) as i32;
                            o.resource = (r >= 0).then_some(r as usize);
                        }
                        x if x == tag(b"PA") => o.parent = Some(u(0)),
                        x if x == tag(b"St") => {
                            is_text = true;
                            text.placeholder = q
                                .chunks_exact(2)
                                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                .take_while(|&c| c != 0)
                                .map(|c| char::from_u32(c as u32).unwrap_or('?'))
                                .collect();
                        }
                        x if x == tag(b"Sj") => text.justify = u(0),
                        x if x == tag(b"Sl") => text.leading = u(0) as i32,
                        x if x == tag(b"Sw") => text.max_width = u(0),
                        x if x == tag(b"SH") => {
                            is_text = true;
                            text.label = u(0)
                        }
                        x if x == tag(b"SA") => {
                            for w in 0..(q.len() / 4).min(field::WORDS) {
                                // Colours are integers; everything else is f32.
                                let is_color = w < 4 || (21..37).contains(&w) && o.kind == ObjectKind::ColoredImage;
                                o.data[w] = if is_color { u32_at(q, w * 4).unwrap_or(0) as i32 as f32 } else { f32_at(q, w * 4).unwrap_or(0.0) };
                            }
                        }
                        _ => {}
                    }
                }
                if is_text || o.kind == ObjectKind::Text {
                    o.text = Some(text);
                }
            }
            SCRP => o.scripts.extend(parse_script(p)),
            MSGR => o.responses = parse_responses(p),
            _ => {}
        }
    }
    Some(o)
}

fn parse_script(b: &[u8]) -> Option<Script> {
    let mut s = Script { id: 0, length: 0, flags: 0, chain: None, tracks: Vec::new(), events: Vec::new() };
    for (t, q) in tags(b) {
        let u = |off: usize| u32_at(q, off).unwrap_or(0);
        match t {
            x if x == tag(b"Sh") => {
                s.id = u(0);
                s.length = u(4);
                s.flags = u(8);
            }
            x if x == tag(b"Sc") => s.chain = Some(u(0)),
            x if x == tag(b"FI") => {
                let (param, size) = (q.first().copied().unwrap_or(0), q.get(1).copied().unwrap_or(0));
                let _ = size;
                s.tracks.push(Track {
                    offset: 0,
                    param,
                    interp: q.get(2).copied().unwrap_or(0),
                    action: q.get(3).copied().unwrap_or(0),
                    length: u(4) & 0xFF_FFFF,
                    base: Vec::new(),
                    keys: Vec::new(),
                });
            }
            x if x == tag(b"To") => {
                if let Some(tr) = s.tracks.last_mut() {
                    tr.offset = u(0) as usize;
                }
            }
            x if x == tag(b"Kd") => {
                let Some(tr) = s.tracks.last_mut() else { continue };
                let words = match tr.param {
                    3 => 2,
                    4 => 3,
                    5 | 6 => 4,
                    _ => continue,
                };
                let stride = 4 + words * 4;
                for k in 0..q.len() / stride {
                    let o = k * stride;
                    let time = u32_at(q, o).unwrap_or(0) as i32;
                    let v: Vec<f32> = (0..words)
                        .map(|i| {
                            let off = o + 4 + i * 4;
                            if tr.param == 6 { u32_at(q, off).unwrap_or(0) as i32 as f32 } else { f32_at(q, off).unwrap_or(0.0) }
                        })
                        .collect();
                    if time < 0 && tr.base.is_empty() {
                        tr.base = v;
                    } else {
                        tr.keys.push((time, v));
                    }
                }
            }
            x if x == tag(b"EV") => {
                for k in 0..q.len() / 12 {
                    s.events.push(Event { message: u(k * 12), target: u(k * 12 + 4), time: u(k * 12 + 8) });
                }
            }
            _ => {}
        }
    }
    Some(s)
}

fn parse_responses(b: &[u8]) -> Vec<Response> {
    let mut out: Vec<Response> = Vec::new();
    for (t, q) in tags(b) {
        let u = u32_at(q, 0).unwrap_or(0);
        match t {
            x if x == tag(b"Mi") => out.push(Response { message: u, commands: Vec::new() }),
            x if x == tag(b"Ri") => {
                if let Some(r) = out.last_mut() {
                    r.commands.push(Command { id: u, param: 0, text: None, target: 0 });
                }
            }
            x if x == tag(b"Ru") => {
                if let Some(c) = out.last_mut().and_then(|r| r.commands.last_mut()) {
                    c.param = u;
                }
            }
            x if x == tag(b"Rs") => {
                if let Some(c) = out.last_mut().and_then(|r| r.commands.last_mut()) {
                    c.text = Some(cstr(q));
                }
            }
            x if x == tag(b"Rt") => {
                if let Some(c) = out.last_mut().and_then(|r| r.commands.last_mut()) {
                    c.target = u;
                }
            }
            _ => {}
        }
    }
    out
}

impl Track {
    /// Value at `t` ms: the base key plus the interpolated delta keys.
    pub fn eval(&self, t: f32) -> Vec<f32> {
        let base = &self.base;
        if self.keys.is_empty() {
            return base.clone();
        }
        let t = if self.action & 1 != 0 && self.length > 0 { t % self.length as f32 } else { t };
        let next = self.keys.iter().position(|k| k.0 as f32 >= t).unwrap_or(self.keys.len() - 1);
        let (f, a, b) = if next > 0 && t < self.keys[next].0 as f32 {
            let (p, n) = (&self.keys[next - 1], &self.keys[next]);
            let span = (n.0 - p.0) as f32;
            let f = if span > 0.0 { (t - p.0 as f32) / span } else { 1.0 };
            (f.clamp(0.0, 1.0), &p.1, &n.1)
        } else {
            (1.0, &self.keys[next].1, &self.keys[next].1)
        };
        match self.param {
            6 => (0..4)
                .map(|i| {
                    let (bv, av, nv) = (base.get(i).copied().unwrap_or(0.0), a[i], b[i]);
                    bv + av + ((nv - av) * f + 0.5).trunc()
                })
                .collect(),
            5 => {
                // Normalised lerp of the delta rotations, applied to the base.
                let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
                let s = if dot < 0.0 { -1.0 } else { 1.0 };
                let mut q: Vec<f32> = (0..4).map(|i| a[i] + (b[i] * s - a[i]) * f).collect();
                let len = q.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
                q.iter_mut().for_each(|x| *x /= len);
                quat_mul(&q, base)
            }
            _ => (0..a.len()).map(|i| base.get(i).copied().unwrap_or(0.0) + a[i] + (b[i] - a[i]) * f).collect(),
        }
    }
}

/// Quaternion product `a · b` (x, y, z, w), System.Numerics order.
pub fn quat_mul(a: &[f32], b: &[f32]) -> Vec<f32> {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b.first().copied().unwrap_or(0.0), b.get(1).copied().unwrap_or(0.0), b.get(2).copied().unwrap_or(0.0), b.get(3).copied().unwrap_or(1.0));
    // System.Numerics Quaternion.Multiply(value1, value2) = value2 rotation then value1.
    vec![
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_packages() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(raw) = game.read("FRONTEND/FrontB.lzc.bak").or_else(|_| game.read("FRONTEND/FrontB.lzc")) else { return };
        let file = decompress(&raw).unwrap();
        let pkgs = packages(&file);
        assert!(pkgs.len() >= 130, "{}", pkgs.len());
        let main = pkgs.iter().find(|p| p.name == "UI_Main.fng").expect("UI_Main");
        assert!(main.objects.len() > 10);
        assert!(main.objects.iter().all(|o| o.script(INIT).is_some()));
        assert!(main.objects.iter().any(|o| o.text.as_ref().is_some_and(|t| t.label != 0)));
        // Parents come before children.
        for (i, o) in main.objects.iter().enumerate() {
            if let Some(p) = o.parent {
                assert!(main.objects[..i].iter().any(|q| q.guid == p));
            }
        }
    }
}
