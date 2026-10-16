//! The original frontend screens (FEng packages) driven by Rust controllers in place of
//! the game code: the package draws itself through its scripts; a controller fills in
//! what speed2.exe would (icons, labels) and turns input into actions.
//!
//! The main menu is the icon scroller of `UI_Main.fng` (speed2.exe 0x4aee40 builds the
//! options, 0x543a90/0x543c10 is the scroller): options sit 40 units apart plus their
//! width, centred on `OPTION_MASTER`; the selected one shows its label in `ICON_TITLE`.

use std::collections::HashMap;

use ug2_formats::bin_hash;
use ug2_formats::feng_scene::{QuadSource, Scene};

use crate::assets::FrontendData;
use crate::render::{FeBatch, FeVertex};

/// What a frontend option asks the game to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeAction {
    Career,
    QuickRace,
    Customize,
    Options,
    /// A quick race mode was chosen.
    Mode(crate::roads::Mode),
    FreeRun,
}

/// Which original screen is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeKind {
    MainMenu,
    QuickRaceMode,
}

#[derive(Clone, Copy, Debug)]
pub struct IconOption {
    /// Texture name hash of the icon (e.g. MAIN_ICON_CAREER).
    pub icon: u32,
    /// Label hash in the language table (e.g. "Career").
    pub label: u32,
    pub action: FeAction,
}

/// The icon scroller of a menu package.
struct IconMenu {
    options: Vec<IconOption>,
    selected: usize,
    /// Scroll position in options (eases towards `selected`).
    scroll: f32,
    master: [f32; 2],
    spacing: f32,
    slots: Vec<usize>,
    title: Option<usize>,
    hidden: Vec<usize>,
    arrows: [Option<usize>; 2],
}

/// Spacing between scroller options: 40 units plus the option width (speed2.exe 0x543c10).
const ICON_GAP: f32 = 40.0;
/// Seconds a scroll step takes (IconScroller +0xb4).
const SCROLL_TIME: f32 = 0.3;

pub struct FeScreen {
    pub scene: Scene,
    pub kind: FeKind,
    menu: IconMenu,
}

#[derive(Default)]
pub struct Frontend {
    pub data: Option<Box<FrontendData>>,
    pub screen: Option<FeScreen>,
}

fn h(name: &str) -> u32 {
    bin_hash(&name.to_ascii_uppercase())
}

impl Frontend {
    pub fn ready(&self) -> bool {
        self.data.is_some()
    }

    /// The main menu with the options of speed2.exe 0x4aee40 (Online and LAN left out:
    /// there is no network play here; Profile Manager is not done yet).
    pub fn open_main_menu(&mut self) -> bool {
        let options = vec![
            IconOption { icon: bin_hash("MAIN_ICON_CAREER"), label: 0x5e7e_1192, action: FeAction::Career },
            IconOption { icon: bin_hash("MAIN_ICON_QUICK_RACE"), label: 0x2490_9fd7, action: FeAction::QuickRace },
            IconOption { icon: bin_hash("MAIN_ICON_CUSTOMIZE_CAR"), label: 0xa530_39a3, action: FeAction::Customize },
            IconOption { icon: bin_hash("MAIN_ICON_OPTIONS"), label: 0xed1f_0b2c, action: FeAction::Options },
        ];
        self.open_icon_menu("UI_Main.fng", FeKind::MainMenu, options)
    }

    /// Quick race modes, as speed2.exe 0x4b2d80 lists them for one player (Outrun only
    /// appears in other game modes; Free Run otherwise).
    pub fn open_quick_race_modes(&mut self) -> bool {
        use crate::roads::Mode;
        let options = vec![
            IconOption { icon: bin_hash("MODE_ICON_CIRCUIT"), label: 0x93fd_ee74, action: FeAction::Mode(Mode::Circuit) },
            IconOption { icon: bin_hash("MODE_ICON_SPRINT"), label: 0xdcb9_f861, action: FeAction::Mode(Mode::Sprint) },
            IconOption { icon: bin_hash("MODE_ICON_DRAG"), label: 0x3510_2edf, action: FeAction::Mode(Mode::Drag) },
            IconOption { icon: bin_hash("MODE_ICON_DRIFT"), label: 0xd716_2cfa, action: FeAction::Mode(Mode::Drift) },
            IconOption { icon: bin_hash("MODE_ICON_SHORT_TRACK"), label: 0x30da_61e1, action: FeAction::Mode(Mode::StreetX) },
            IconOption { icon: bin_hash("MODE_ICON_U_RACING_LEAGUE"), label: 0xc786_c4e7, action: FeAction::Mode(Mode::Url) },
            IconOption { icon: bin_hash("MODE_ICON_FREE_RUN"), label: 0x6f24_21b7, action: FeAction::FreeRun },
        ];
        self.open_icon_menu("UI_QRModeSelect.fng", FeKind::QuickRaceMode, options)
    }

    pub fn kind(&self) -> Option<FeKind> {
        self.screen.as_ref().map(|s| s.kind)
    }

    fn open_icon_menu(&mut self, package: &str, kind: FeKind, options: Vec<IconOption>) -> bool {
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case(package)) else { return false };
        let mut scene = Scene::new(pkg.clone());
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let find = |n: &str| scene.index_of(h(n));
        let master_i = find("OPTION_MASTER");
        let master = master_i.map_or([0.0, -134.0], |i| {
            let d = scene.values(i);
            [d[7], d[8]]
        });
        let width = master_i.map_or(118.0, |i| scene.values(i)[14]);
        let slots: Vec<usize> = (1..=options.len()).filter_map(|k| find(&format!("option_{k}"))).collect();
        let mut hidden: Vec<usize> = master_i.into_iter().collect();
        // Templates and debug objects the game code hides.
        for k in options.len() + 1..=16 {
            hidden.extend(find(&format!("option_{k}")));
        }
        hidden.extend(find("_EVENT_HANDLER_"));
        let menu = IconMenu {
            options,
            selected: 0,
            scroll: 0.0,
            master,
            spacing: width + ICON_GAP,
            slots,
            title: find("ICON_TITLE"),
            hidden,
            arrows: [find("ScrollBar_Arrow_1"), find("ScrollBar_Arrow_2")],
        };
        for &i in &menu.slots {
            scene.set_script(i, h("FadeIn"));
        }
        for i in menu.arrows.into_iter().flatten() {
            scene.set_script(i, h("static"));
        }
        let mut s = FeScreen { scene, kind, menu };
        s.apply(&data.strings);
        self.screen = Some(s);
        true
    }

    pub fn close(&mut self) {
        self.screen = None;
    }

    pub fn update(&mut self, dt: f32) {
        let (Some(s), Some(data)) = (&mut self.screen, &self.data) else { return };
        s.scene.advance(dt * 1000.0);
        let target = s.menu.selected as f32;
        let step = dt / SCROLL_TIME;
        let d = target - s.menu.scroll;
        s.menu.scroll = if d.abs() <= step { target } else { s.menu.scroll + d.signum() * step.max(d.abs() * dt * 8.0) };
        s.apply(&data.strings);
    }

    /// Steps the selection; true when it moved.
    pub fn step(&mut self, delta: i32) -> bool {
        let Some(s) = &mut self.screen else { return false };
        let n = s.menu.options.len() as i32;
        let next = (s.menu.selected as i32 + delta).clamp(0, n - 1) as usize;
        if next == s.menu.selected {
            return false;
        }
        s.menu.selected = next;
        let arrow = if delta < 0 { s.menu.arrows[0] } else { s.menu.arrows[1] };
        if let Some(a) = arrow {
            s.scene.set_script(a, h("Blink"));
        }
        true
    }

    pub fn activate(&self) -> Option<FeAction> {
        let s = self.screen.as_ref()?;
        Some(s.menu.options.get(s.menu.selected)?.action)
    }

    /// A tap at screen pixel (x, y): arrows scroll, an icon is selected (or, when it is
    /// already selected, activated).
    pub fn tap(&mut self, x: f32, y: f32, size: (u32, u32)) -> Option<FeAction> {
        let (vx, vy) = to_virtual(x, y, size);
        let s = self.screen.as_ref()?;
        let near = |i: usize, r: f32| {
            let d = s.scene.values(i);
            let (cx, cy) = (d[7] + 320.0, d[8] + 240.0);
            (vx - cx).abs() < r.max(d[14] * 0.5) && (vy - cy).abs() < r.max(d[15] * 0.5)
        };
        if s.menu.arrows[0].is_some_and(|a| near(a, 30.0)) {
            self.step(-1);
            return None;
        }
        if s.menu.arrows[1].is_some_and(|a| near(a, 30.0)) {
            self.step(1);
            return None;
        }
        let hit = s.menu.slots.iter().position(|&i| near(i, 0.0));
        match hit {
            Some(k) if k == s.menu.selected => self.activate(),
            Some(k) => {
                let d = k as i32 - s.menu.selected as i32;
                self.step(d);
                None
            }
            None => None,
        }
    }

    /// The screen's quads in screen pixels, batched by texture.
    pub fn draw(&self, size: (u32, u32), verts: &mut Vec<FeVertex>, batches: &mut Vec<FeBatch>) {
        let (Some(s), Some(data)) = (&self.screen, &self.data) else { return };
        let quads = s.scene.layout(&data.strings, &data.fonts);
        let (scale, ox) = mapping(size);
        for q in &quads {
            let texture = match &q.source {
                QuadSource::Texture(n) | QuadSource::Font(n) => bin_hash(n),
                QuadSource::TextureHash(h) => *h,
                QuadSource::Solid => 0,
            };
            let first = verts.len() as u32;
            for k in [0usize, 1, 2, 0, 2, 3] {
                let [x, y] = q.corners[k];
                let c = q.colors[k];
                verts.push(FeVertex {
                    pos: [ox + x * scale, y * scale],
                    uv: q.uv[k],
                    color: [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0],
                });
            }
            match batches.last_mut() {
                Some(b) if b.texture == texture && b.first + b.count == first => b.count += 6,
                _ => batches.push(FeBatch { texture, first, count: 6 }),
            }
        }
    }
}

impl FeScreen {
    /// Writes the controller state into the scene, as the game code does every frame.
    fn apply(&mut self, strings: &ug2_formats::lang::Strings) {
        let m = &self.menu;
        for &i in &m.hidden {
            self.scene.states[i].hidden = Some(true);
        }
        for (k, &i) in m.slots.iter().enumerate() {
            let Some(o) = m.options.get(k) else { continue };
            let x = m.master[0] + (k as f32 - m.scroll) * m.spacing;
            let st = &mut self.scene.states[i];
            st.texture = Some(o.icon);
            st.position = Some([x, m.master[1]]);
            // Only what fits the backing shows.
            st.hidden = Some((x - m.master[0]).abs() > m.spacing * 1.5);
        }
        if let (Some(t), Some(o)) = (m.title, m.options.get(m.selected)) {
            self.scene.states[t].text = strings.by_hash(o.label).map(str::to_string);
        }
    }
}

/// Virtual 640×480 to screen: fill the height, centre horizontally (the PC game's 4:3
/// frontend on a wide screen).
fn mapping(size: (u32, u32)) -> (f32, f32) {
    let scale = size.1 as f32 / 480.0;
    (scale, (size.0 as f32 - 640.0 * scale) * 0.5)
}

fn to_virtual(x: f32, y: f32, size: (u32, u32)) -> (f32, f32) {
    let (scale, ox) = mapping(size);
    ((x - ox) / scale, y / scale)
}

/// Labels used by the controllers, resolved for logging and tests.
pub fn labels(strings: &ug2_formats::lang::Strings) -> HashMap<&'static str, String> {
    let mut m = HashMap::new();
    for (k, h) in [("career", 0x5e7e_1192u32), ("quick race", 0x2490_9fd7), ("customize", 0xa530_39a3), ("options", 0xed1f_0b2c)] {
        m.insert(k, strings.by_hash(h).unwrap_or("?").to_string());
    }
    m
}
