//! The career World Map (UI_CareerWorldMap, speed2.exe ctor 0x4ee690, handler 0x4e0f60,
//! list 0x4b1360):
//!
//! * a top scroller of the unlocked stages ("Stage %d");
//! * a vertical list of EVENT_ICONS (MINI_MASTER copies, 36 units apart): row 0 is the
//!   stage overview, then one row per race of the stage (stage 1 also lists stage 0
//!   but not the DDAY events). The selected row always sits at MINI_MASTER; ±3 rows show;
//! * the map: TRACKMAP<track> zoomed to the track's view (track record +0x118), or the
//!   stage's overview map (table 0x7f4388): 0.3 s after a change the map loads, holds
//!   0.5 s whole, then zooms in over 0.5 s (0x498630);
//! * the right panel: event status, race type, distance, best time / lap / score.

use ug2_formats::bin_hash;
use ug2_formats::feng_scene::Scene;
use ug2_formats::lang::Strings;

fn h(name: &str) -> u32 {
    bin_hash(&name.to_ascii_uppercase())
}

/// One race on the world map.
#[derive(Clone, Debug)]
pub struct WorldRace {
    pub hash: u32,
    /// 0 circuit, 1 sprint, 2 Street X, 4 drag, 5 drift.
    pub kind: u8,
    pub track: u16,
    pub length: u32,
    /// TRACKMAP view: x, y, size in 512-pixel image space.
    pub view: [f32; 3],
    pub completed: bool,
    /// Best time (s) or drift score.
    pub best: Option<f32>,
}

/// Stage overview maps (speed2.exe table 0x7f4388): x, y, size, TRACKMAP number.
const STAGE_VIEWS: [([f32; 3], u16); 5] =
    [([163.0, 154.0, 350.0], 4500), ([126.0, 93.0, 387.0], 4501), ([17.0, 5.0, 480.0], 4502), ([0.0, 0.0, 512.0], 4503), ([0.0, 0.0, 512.0], 4504)];

const ROW_STEP: f32 = 36.0;

pub struct WorldMap {
    /// Unlocked stages (1-based) and the shown one.
    stages: Vec<u8>,
    stage: usize,
    /// All races by stage (from the career data); the list is rebuilt per stage.
    races: Vec<(u8, WorldRace)>,
    /// Row 0 = overview (None).
    rows: Vec<Option<WorldRace>>,
    pub selected: usize,
    icons: Vec<usize>,
    /// Seconds since the selection changed (map timing) and the pulse phase.
    since: f32,
    pulse: f32,
    mph: bool,
    /// The rows were rebuilt: their icons fade in (Init chains to "hide").
    fade: bool,
}

impl WorldMap {
    pub fn new(scene: &mut Scene, stages: Vec<u8>, races: Vec<(u8, WorldRace)>, mph: bool) -> Self {
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let icon = h("EVENT_ICONS");
        let icons: Vec<usize> = (0..scene.package.objects.len()).filter(|&i| scene.package.objects[i].hash == icon).collect();
        for n in ["_EVENT_HANDLER_", "MINI_MASTER"] {
            if let Some(i) = scene.index_of(h(n)) {
                scene.states[i].hidden = Some(true);
            }
        }
        let stage = stages.len().saturating_sub(1);
        let mut m = WorldMap { stages, stage, races, rows: Vec::new(), selected: 0, icons, since: 0.0, pulse: 0.0, mph, fade: true };
        m.build();
        m
    }

    fn build(&mut self) {
        let s = self.stages.get(self.stage).copied().unwrap_or(1);
        let mut rows = vec![None];
        for (st, r) in &self.races {
            if *st == s || (s == 1 && *st == 0) {
                if rows.len() > 71 {
                    break;
                }
                rows.push(Some(r.clone()));
            }
        }
        self.rows = rows;
        self.selected = 0;
        self.since = 0.0;
        self.fade = true;
    }

    /// Up / down (no wrap).
    pub fn step_row(&mut self, d: i32) -> bool {
        let n = (self.selected as i32 + d).clamp(0, self.rows.len() as i32 - 1) as usize;
        if n == self.selected {
            return false;
        }
        self.selected = n;
        self.since = 0.0;
        true
    }

    /// Left / right: the stage (no wrap); the list starts on the overview.
    pub fn step_stage(&mut self, d: i32) -> bool {
        let n = (self.stage as i32 + d).clamp(0, self.stages.len() as i32 - 1) as usize;
        if n == self.stage {
            return false;
        }
        self.stage = n;
        self.build();
        true
    }

    /// The race of the selected row (None on the overview).
    pub fn selected_race(&self) -> Option<&WorldRace> {
        self.rows.get(self.selected)?.as_ref()
    }

    /// The TRACKMAP number the map shows now.
    pub fn map_number(&self) -> u16 {
        match self.selected_race() {
            Some(r) => r.track,
            None => STAGE_VIEWS[(self.stages.get(self.stage).copied().unwrap_or(1) as usize).clamp(1, 5) - 1].1,
        }
    }

    /// A tap at virtual (vx, vy) with the origin at the screen centre: the list column
    /// moves rows, the top arrows change stage, the map or the panel accepts.
    pub fn tap(&mut self, x: f32, y: f32) -> TapAction {
        if y < -140.0 {
            self.step_stage(if x < 0.0 { -1 } else { 1 });
            return TapAction::None;
        }
        if x < -240.0 {
            let d = ((y - 24.0) / ROW_STEP).round() as i32;
            if d != 0 {
                self.step_row(d);
            }
            return TapAction::None;
        }
        if self.selected_race().is_some() { TapAction::Accept } else { TapAction::None }
    }

    pub fn tick(&mut self, dt: f32) {
        self.since += dt;
        self.pulse = (self.pulse + 0.03 * dt * 60.0) % 2.0;
    }

    pub fn apply(&mut self, scene: &mut Scene, strings: &Strings) {
        let set_text = |scene: &mut Scene, hash: u32, text: String| {
            if let Some(i) = scene.index_of(hash) {
                scene.states[i].text = Some(text);
            }
        };
        let label = |hash: u32| strings.by_hash(hash).unwrap_or("").to_string();
        // Stage scroller.
        let stage = self.stages.get(self.stage).copied().unwrap_or(1);
        set_text(scene, h("REGION_DATA"), label(0x0a96_b270).replace("%d", &stage.to_string()));
        for (n, hidden) in [("ScrollBar_Arrow_1", self.stage == 0), ("ScrollBar_Arrow_2", self.stage + 1 >= self.stages.len())] {
            if let Some(i) = scene.index_of(h(n)) {
                scene.states[i].hidden = Some(hidden);
            }
        }
        // Rows: the selected one at MINI_MASTER (-264, 24); others at 0.8, ±3 shown.
        let tri = if self.pulse <= 1.0 { self.pulse } else { 2.0 - self.pulse };
        let pulse = (tri * 255.0 * 1.1 + 127.5).min(255.0) / 8500.0 + 0.97;
        if self.fade {
            self.fade = false;
            for &i in self.icons.iter().take(self.rows.len()) {
                scene.set_script(i, h("FadeIn"));
            }
        }
        for (k, &i) in self.icons.iter().enumerate() {
            let st = &mut scene.states[i];
            if k >= self.rows.len() {
                st.hidden = Some(true);
                continue;
            }
            let y = 24.0 + (k as f32 - self.selected as f32) * ROW_STEP;
            st.hidden = Some((y - 24.0).abs() > 136.5);
            st.position = Some([-264.0, y]);
            st.texture = Some(bin_hash("U2_MINI_ICON"));
            let s = if k == self.selected { 16.0 * pulse } else { 12.8 };
            st.size = Some([s, s]);
        }
        // Side scroll bar: handle 290 / count long over the 290-unit backing.
        let count = self.rows.len().max(1) as f32;
        let len = 290.0 / count;
        let top = -123.0 + self.selected as f32 * (290.0 - len) / (count - 1.0).max(1.0);
        for n in ["SideScrollBar_Handle", "SideScrollBar_Backing", "SideScrollBar_arrow_1", "SideScrollBar_arrow_2"] {
            if let Some(i) = scene.index_of(h(n)) {
                scene.states[i].hidden = Some(self.rows.len() <= 1);
            }
        }
        if let Some(i) = scene.index_of(h("SideScrollBar_Handle")) {
            let d = scene.package.objects[i].data;
            scene.states[i].position = Some([d[7], top + len * 0.5]);
            scene.states[i].size = Some([d[14], len]);
        }
        // Right panel.
        let (t1, t2, t3, t4) = (0x4d2f_5c61, 0x4d2f_5c62, 0x4d2f_5c63, 0x4d2f_5c64);
        let (v1, v2, v3, v4) = (h("data_01"), 0x23ea_e8ba, 0x23ea_e8bb, 0x23ea_e8bc);
        match self.selected_race().cloned() {
            None => {
                for x in [t1, t2, t3, t4, v1, v2, v3, v4] {
                    set_text(scene, x, String::new());
                }
            }
            Some(r) => {
                set_text(scene, t1, label(0xed8c_a191));
                set_text(scene, v1, label(if r.completed { 0xb1ee_4ca9 } else { 0xd11e_cd79 }));
                set_text(scene, t2, label(0x7f90_4ae8));
                let kind = match r.kind {
                    0 => 0x268d_063c,
                    1 => 0xba61_3729,
                    2 => 0x0fda_6758,
                    4 => 0x40c9_1ba7,
                    5 => 0x59ec_b2c2,
                    _ => 0,
                };
                set_text(scene, v2, if kind != 0 { label(kind) } else { "Untitled Race".into() });
                set_text(scene, t3, label(0x7019_add7));
                let (dist, unit) = if self.mph { (r.length as f32 * 0.001 * 0.62137, 0xac3a_4f4c) } else { (r.length as f32 * 0.001, 0xb5d6_40b1) };
                set_text(scene, v3, format!("{dist:.2} {}", label(unit)));
                let (title, value) = match r.kind {
                    5 => (0x4e0a_36a0, r.best.map_or(String::new(), |b| (b as u32).to_string())),
                    1 | 4 => (0xdb0c_5efd, r.best.map_or_else(|| label(0x61c9_b7a2), |b| crate::fe::race_time(b, false))),
                    _ => (0x3b4a_1ecb, r.best.map_or_else(|| label(0x61c9_b7a2), |b| crate::fe::race_time(b, false))),
                };
                set_text(scene, t4, label(title));
                set_text(scene, v4, value);
            }
        }
        // The map: load 0.3 s after a change, hold 0.5 s whole, zoom in over 0.5 s.
        let (view, number) = match self.selected_race() {
            Some(r) => (r.view, r.track),
            None => {
                let v = STAGE_VIEWS[(stage as usize).clamp(1, 5) - 1];
                (v.0, v.1)
            }
        };
        if let Some(i) = scene.index_of(h("Track2000_map")) {
            let t = ((self.since - 0.8) / 0.5).clamp(0.0, 1.0);
            let k = if self.since < 0.3 { 0.0 } else { (t * std::f32::consts::FRAC_PI_2).sin() };
            let [x, y, size] = view;
            let k = if size <= 0.0 { 0.0 } else { k };
            let uv0 = [x * k / 512.0, y * k / 512.0];
            let uv1 = [(x * k + 512.0 - (512.0 - size) * k) / 512.0, (y * k + 512.0 - (512.0 - size) * k) / 512.0];
            let st = &mut scene.states[i];
            if self.since >= 0.3 {
                st.texture = Some(bin_hash(&format!("TRACKMAP{number}")));
            }
            st.uv = Some([uv0[0], uv0[1], uv1[0], uv1[1]]);
        }
    }
}

pub enum TapAction {
    None,
    Accept,
}
