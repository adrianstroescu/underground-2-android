//! Runs a FEng package: plays each object's scripts and lays the visible objects out as
//! quads on the 640×480 virtual screen (origin top-left here, +y down). The renderer
//! (GPU in the engine, CPU in ug2-tool) only draws the quads.
//!
//! Conventions (see CLAUDE.md, "FEng"): an object's local transform is
//! Scale(Size) · [Translate(−Pivot) · Rotate · Translate(Pivot)] · Translate(Position)
//! (row vectors), then its parent's. Images are unit quads centred on the origin; text is
//! laid out in font pixels from the origin by its justification. Colours multiply down
//! the hierarchy with `(a·b + 128) >> 8`. Larger z is drawn first.

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::feng::{field, flags, Object, ObjectKind, Package, ResourceKind, Script, INIT};
use crate::font::Font;
use crate::lang::Strings;

/// Script state of one object.
#[derive(Clone, Debug)]
pub struct ObjectState {
    pub script: u32,
    /// Milliseconds into the script.
    pub time: f32,
    pub running: bool,
    /// Overrides set by game code (text, visibility, texture, position).
    pub text: Option<String>,
    pub hidden: Option<bool>,
    /// Texture by name hash, for code-supplied images (menu icons).
    pub texture: Option<u32>,
    /// Position (x, y) replacing the scripted one.
    pub position: Option<[f32; 2]>,
    /// Multiplies the object's colour (RGBA 0..255).
    pub tint: Option<[u8; 4]>,
    /// Size (x, y) replacing the scripted one.
    pub size: Option<[f32; 2]>,
}

pub struct Scene {
    pub package: Package,
    pub states: Vec<ObjectState>,
    /// Messages the package posted to the game (PostMessageToGame), oldest first.
    pub to_game: Vec<u32>,
    /// The highlighted button (object index), as set by SetActiveButton.
    pub active_button: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum QuadSource {
    /// A texture by name (upper-case stem, e.g. "U2_MENU_BOX").
    Texture(String),
    /// A texture by name hash.
    TextureHash(u32),
    /// Untextured (white).
    Solid,
    /// A glyph from a font's texture.
    Font(String),
}

#[derive(Clone, Debug)]
pub struct Quad {
    pub source: QuadSource,
    /// Corners TL, TR, BR, BL in virtual-screen pixels (0..640, 0..480).
    pub corners: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    /// RGBA per corner, 0..255.
    pub colors: [[u8; 4]; 4],
    pub z: f32,
    /// Index of the object (for hit testing).
    pub object: usize,
}

impl Scene {
    pub fn new(package: Package) -> Self {
        let states = package
            .objects
            .iter()
            .map(|_| ObjectState { script: INIT, time: 0.0, running: true, text: None, hidden: None, texture: None, position: None, tint: None, size: None })
            .collect();
        Self { package, states, to_game: Vec::new(), active_button: None }
    }

    /// Sends a message (name hash) to the package and every object that responds to it,
    /// running their response commands.
    pub fn send(&mut self, message: u32) {
        let mut queue = vec![(message, None::<usize>)];
        let mut guard = 0;
        while let Some((msg, only)) = queue.pop() {
            guard += 1;
            if guard > 256 {
                break;
            }
            let mut work: Vec<(Option<usize>, Vec<crate::feng::Command>)> = Vec::new();
            if only.is_none() {
                for r in self.package.responses.iter().filter(|r| r.message == msg) {
                    work.push((None, r.commands.clone()));
                }
            }
            for (i, o) in self.package.objects.iter().enumerate() {
                if only.is_some_and(|k| k != i) {
                    continue;
                }
                for r in o.responses.iter().filter(|r| r.message == msg) {
                    work.push((Some(i), r.commands.clone()));
                }
            }
            for (owner, cmds) in work {
                let mut skipping = false;
                for c in cmds {
                    match c.id {
                        0x300 | 0x301 => {
                            let who = if c.target != 0 { self.package.objects.iter().position(|o| o.guid == c.target) } else { owner };
                            let cur = who.map(|w| self.states[w].script);
                            let eq = cur == Some(c.param);
                            skipping = if c.id == 0x300 { !eq } else { eq };
                            continue;
                        }
                        0x500 => {
                            skipping = !skipping;
                            continue;
                        }
                        0x501 => {
                            skipping = false;
                            continue;
                        }
                        _ if skipping => continue,
                        0x000 => {
                            let who = if c.target != 0 { self.package.objects.iter().position(|o| o.guid == c.target) } else { owner };
                            match who {
                                Some(w) => self.set_script(w, c.param),
                                None => self.set_script_all(c.param),
                            }
                        }
                        0x001 => {
                            let to = self.package.objects.iter().position(|o| o.guid == c.target);
                            queue.push((c.param, to));
                        }
                        0x002 => self.to_game.push(c.param),
                        0x100 | 0x103 => self.active_button = self.package.objects.iter().position(|o| o.guid == c.param),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Sends a message by name (upper-cased before hashing).
    pub fn send_named(&mut self, name: &str) {
        self.send(crate::bin_hash(&name.to_ascii_uppercase()));
    }

    /// Starts script `id` (a name hash) on object `i`, if it has one.
    pub fn set_script(&mut self, i: usize, id: u32) {
        if self.package.objects[i].script(id).is_some() {
            self.states[i] = ObjectState { script: id, time: 0.0, running: true, ..self.states[i].clone() };
        }
    }

    /// Starts script `id` on every object that has it.
    pub fn set_script_all(&mut self, id: u32) {
        for i in 0..self.states.len() {
            self.set_script(i, id);
        }
    }

    pub fn index_of(&self, hash: u32) -> Option<usize> {
        self.package.objects.iter().position(|o| o.hash == hash)
    }

    /// Advances every script by `dt_ms`, following chains and loops.
    pub fn advance(&mut self, dt_ms: f32) {
        for (o, st) in self.package.objects.iter().zip(self.states.iter_mut()) {
            if !st.running {
                continue;
            }
            st.time += dt_ms;
            for _ in 0..8 {
                let Some(s) = o.script(st.script) else {
                    st.running = false;
                    break;
                };
                if st.time < s.length as f32 {
                    break;
                }
                if let Some(next) = s.chain.filter(|n| o.script(*n).is_some()) {
                    st.time -= s.length as f32;
                    st.script = next;
                } else if s.looping() {
                    st.time = if s.length > 0 { st.time % s.length as f32 } else { 0.0 };
                    break;
                } else {
                    st.time = s.length as f32;
                    st.running = false;
                    break;
                }
            }
        }
    }

    /// The object's data block at its current script time.
    pub fn values(&self, i: usize) -> [f32; field::WORDS] {
        let o = &self.package.objects[i];
        let mut d = o.data;
        if let Some(s) = o.script(self.states[i].script) {
            apply_script(&mut d, s, self.states[i].time);
        }
        if let Some([x, y]) = self.states[i].position {
            d[field::POSITION] = x;
            d[field::POSITION + 1] = y;
        }
        if let Some([x, y]) = self.states[i].size {
            d[field::SIZE] = x;
            d[field::SIZE + 1] = y;
        }
        if let Some(t) = self.states[i].tint {
            // B, G, R, A words.
            for (w, k) in [(2, 0), (1, 1), (0, 2), (3, 3)] {
                d[w] = d[w] * t[k] as f32 / 255.0;
            }
        }
        d
    }

    /// Lays out every visible object. `text_of` gives the shown string for a text object
    /// (its label looked up, or an override).
    pub fn layout(&self, strings: &Strings, fonts: &[Font]) -> Vec<Quad> {
        let objs = &self.package.objects;
        let mut world: Vec<Option<(Mat4, [i32; 4], bool)>> = vec![None; objs.len()];
        let mut quads = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            let d = self.values(i);
            let (parent_m, parent_c, parent_hidden) = match o.parent.and_then(|p| objs.iter().position(|q| q.guid == p)) {
                Some(p) => world[p].unwrap_or((Mat4::IDENTITY, [255; 4], false)),
                None => (Mat4::IDENTITY, [255; 4], false),
            };
            let hidden_flag = o.flags & (flags::INVISIBLE | flags::HIDE_IN_EDIT | flags::CONSOLE_ONLY) != 0;
            let hidden = parent_hidden || self.states[i].hidden.unwrap_or(hidden_flag);
            let m = parent_m * local_matrix(&d);
            // B, G, R, A → R, G, B, A, blended with the parent.
            let own = [d[2] as i32, d[1] as i32, d[0] as i32, d[3] as i32].map(|c| c.clamp(0, 255));
            let c = [0, 1, 2, 3].map(|k| (own[k] * parent_c[k] + 128) >> 8);
            world[i] = Some((m, c, hidden));
            if hidden || c[3] == 0 {
                continue;
            }
            let z = m.w_axis.z;
            let rgba = c.map(|x| x.clamp(0, 255) as u8);
            match o.kind {
                ObjectKind::Image | ObjectKind::ColoredImage | ObjectKind::MultiImage | ObjectKind::SimpleImage => {
                    let source = match (self.states[i].texture, o.resource.and_then(|r| self.package.resources.get(r))) {
                        (Some(h), _) => QuadSource::TextureHash(h),
                        (None, Some(r)) if r.kind == ResourceKind::Image => QuadSource::Texture(r.texture_name()),
                        _ => QuadSource::Solid,
                    };
                    let p = |x: f32, y: f32| {
                        let v = m * Vec4::new(x, y, 0.0, 1.0);
                        [v.x + 320.0, v.y + 240.0]
                    };
                    let (ul, lr) = ([d[field::UV_TOP_LEFT], d[field::UV_TOP_LEFT + 1]], [d[field::UV_BOTTOM_RIGHT], d[field::UV_BOTTOM_RIGHT + 1]]);
                    let colors = if o.kind == ObjectKind::ColoredImage {
                        [0, 1, 2, 3].map(|k| {
                            let w = field::CORNER_COLORS + k * 4;
                            [d[w + 2], d[w + 1], d[w], d[w + 3]].map(|x| x.clamp(0.0, 255.0) as u8)
                        })
                    } else {
                        [rgba; 4]
                    };
                    quads.push(Quad {
                        source,
                        corners: [p(-0.5, -0.5), p(0.5, -0.5), p(0.5, 0.5), p(-0.5, 0.5)],
                        uv: [ul, [lr[0], ul[1]], lr, [ul[0], lr[1]]],
                        colors,
                        z,
                        object: i,
                    });
                }
                ObjectKind::Text => {
                    let Some(t) = &o.text else { continue };
                    let text = match &self.states[i].text {
                        Some(s) => s.clone(),
                        None => strings.by_hash(t.label).map(str::to_string).unwrap_or_else(|| t.placeholder.clone()),
                    };
                    let font_name = o
                        .resource
                        .and_then(|r| self.package.resources.get(r))
                        .map(|r| {
                            let n = r.name.rsplit(['\\', '/']).next().unwrap_or(&r.name);
                            n.rsplit_once('.').map_or(n, |(s, _)| s).to_ascii_lowercase()
                        })
                        .unwrap_or_default();
                    let Some(font) = fonts.iter().find(|f| f.name == font_name).or_else(|| fonts.first()) else { continue };
                    layout_text(&mut quads, font, &text, t.justify, t.max_width, t.leading, m, rgba, z, i);
                }
                _ => {}
            }
        }
        // Back to front: larger z first; stable for equal z (file order).
        quads.sort_by(|a, b| b.z.total_cmp(&a.z));
        quads
    }
}

fn local_matrix(d: &[f32; field::WORDS]) -> Mat4 {
    let v = |w: usize| Vec3::new(d[w], d[w + 1], d[w + 2]);
    let size = v(field::SIZE);
    let pivot = v(field::PIVOT);
    let pos = v(field::POSITION);
    let q = Quat::from_xyzw(d[field::ROTATION], d[field::ROTATION + 1], d[field::ROTATION + 2], d[field::ROTATION + 3]);
    let rot = if q.length_squared() > 1e-6 && !q.normalize().abs_diff_eq(Quat::IDENTITY, 1e-6) {
        Mat4::from_translation(pivot) * Mat4::from_quat(q.normalize()) * Mat4::from_translation(-pivot)
    } else {
        Mat4::IDENTITY
    };
    // Column vectors: applied right to left = the row-vector order of the original.
    Mat4::from_translation(pos) * rot * Mat4::from_scale(Vec3::new(size.x, size.y, if size.z == 0.0 { 1.0 } else { size.z }))
}

fn apply_script(d: &mut [f32; field::WORDS], s: &Script, t: f32) {
    for tr in &s.tracks {
        if tr.base.is_empty() {
            continue;
        }
        let v = tr.eval(t);
        for (k, x) in v.iter().enumerate() {
            if let Some(slot) = d.get_mut(tr.offset + k) {
                *slot = *x;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_text(out: &mut Vec<Quad>, font: &Font, text: &str, justify: u32, max_width: u32, leading: i32, m: Mat4, color: [u8; 4], z: f32, object: usize) {
    // Lines: explicit breaks, then word wrap at max_width (font pixels).
    let mut lines: Vec<String> = Vec::new();
    // '^' is the strings' line break ("Restart race?^All unsaved ...").
    let text = text.replace('^', "\n");
    for para in text.split('\n') {
        if max_width == 0 {
            lines.push(para.to_string());
            continue;
        }
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if font.width(&candidate) > max_width as i32 && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    let line_h = font.line_height as f32 + leading as f32;
    let total_h = line_h * lines.len() as f32;
    let y0 = if justify & 8 != 0 {
        -total_h
    } else if justify & 4 != 0 {
        -total_h * 0.5
    } else {
        0.0
    };
    let (tw, th) = font.texture_size;
    for (li, line) in lines.iter().enumerate() {
        let w = font.width(line) as f32;
        let mut x = if justify & 2 != 0 {
            -w
        } else if justify & 1 != 0 {
            -w * 0.5
        } else {
            0.0
        };
        let y = y0 + li as f32 * line_h;
        let mut prev = None;
        for c in line.chars() {
            if let Some(p) = prev {
                x += font.kern(p, c) as f32;
            }
            prev = Some(c);
            let Some(g) = font.glyph(c) else { continue };
            if g.width > 0 && g.height > 0 {
                let (gx, gy) = (x + g.x_offset as f32, y + g.y_offset as f32);
                let (gw, gh) = (g.width as f32, g.height as f32);
                let p = |px: f32, py: f32| {
                    let v = m * Vec4::new(px, py, 0.0, 1.0);
                    [v.x + 320.0, v.y + 240.0]
                };
                let (u0, v0) = (g.u as f32 / tw, g.v as f32 / th);
                let (u1, v1) = ((g.u as f32 + gw) / tw, (g.v as f32 + gh) / th);
                out.push(Quad {
                    source: QuadSource::Font(font.texture_name()),
                    corners: [p(gx, gy), p(gx + gw, gy), p(gx + gw, gy + gh), p(gx, gy + gh)],
                    uv: [[u0, v0], [u1, v0], [u1, v1], [u0, v1]],
                    colors: [color; 4],
                    z,
                    object,
                });
            }
            x += g.advance as f32;
        }
    }
}

/// Convenience for tools: the object at `i` is a button.
pub fn is_button(o: &Object) -> bool {
    o.flags & flags::IS_BUTTON != 0
}
