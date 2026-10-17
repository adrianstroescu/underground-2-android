//! The original race HUD (`HUD_SingleRace.fng` in GLOBAL/InGameRace.bun) driven like
//! speed2.exe does (HUD ctor 0x4f8bf0, widgets updated by 0x4ec5a0):
//!
//! * tachometer (0x4c68d0): needle at rpm / dial max × 235°, dial chosen by the engine
//!   class (0x60fe10), speed "%3d" in MPH or KM/H, gear letter, shift light; the dial
//!   textures come from `GLOBAL/HUD_CustomTextures_00.bin` (`*_CUSTOM_00`);
//! * race position (0x4ebb10), lap counter (0x4aa1b0), lap time;
//! * leaderboard (0x4da190): four rows, gaps for 10 s then names for 2 s;
//! * minimap (0x4ca840): TRACKMAP<n> windowed to 0.3 / zoom of the image round the
//!   car, rotated with the car in Rotate mode; racer icons inside radius 0.55;
//! * the 3-2-1-GO countdown (0x4ab5b0).
//!
//! Everything else in the package (free roam indicators, online, drift, dyno...)
//! stays hidden.

use ug2_formats::bin_hash;
use ug2_formats::feng_scene::Scene;
use ug2_formats::lang::Strings;

use crate::fe::race_time;

fn h(name: &str) -> u32 {
    bin_hash(&name.to_ascii_uppercase())
}

/// What the HUD shows this frame.
/// Which HUD package (speed2.exe 0x5f189a picks it by race type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudKind {
    /// HUD_SingleRace: circuit, sprint, URL, free roam.
    Single,
    /// HUD_Short_Track: Street X.
    ShortTrack,
    /// HUD_Drag.
    Drag,
    /// HUD_Drift.
    Drift,
}

/// Drift scoring shown by HUD_Drift (0x4da590, 0x4c85d0).
#[derive(Clone, Copy, Debug, Default)]
pub struct DriftInput {
    pub total: u32,
    /// Points of the drift in progress (0 when none).
    pub pending: u32,
    pub multiplier: u32,
    pub best: u32,
    /// Banked drifts so far and the last one's points.
    pub banks: u32,
    pub last_bank: u32,
    /// Slip angle in degrees.
    pub angle: f32,
}

/// "%$d": thousands separated by commas.
fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Comment_DATA after a drift: thresholds from GLOBALB chunk 0x34a07.
const DRIFT_COMMENTS: [(u32, u32); 7] = [
    (100_000, 0x670b_90d0),
    (50_000, 0xecd6_4585),
    (20_000, 0xb927_8dd5),
    (15_000, 0x77d4_05a2),
    (7_000, 0x0bcb_8df8),
    (3_000, 0x68df_0cfa),
    (1_000, 0x1d10_73f0),
];

impl HudKind {
    pub fn package(self) -> &'static str {
        match self {
            HudKind::Single => "HUD_SingleRace.fng",
            HudKind::ShortTrack => "HUD_Short_Track.fng",
            HudKind::Drag => "HUD_Drag.fng",
            HudKind::Drift => "HUD_Drift.fng",
        }
    }

    /// Top-level objects a one-player race shows; the rest belongs to other widgets
    /// (online, dyno, free roam indicators) the game keeps hidden.
    fn shown(self) -> &'static [&'static str] {
        match self {
            HudKind::Single => &SHOWN,
            HudKind::ShortTrack => &[
                "GaugeCluster",
                "POSITION",
                "PositionBacking",
                "BestTimeCounter",
                "LeaderBoardGroup",
                "LapCounterBacking_1",
                "NumberLaps",
                "LapNumber",
                "LapText",
                "Track2000_map",
                "Minimap_Mask",
                "TrackMapTargetRing",
                "321_Go",
                "Lap_Time_Text",
                "Lap_Time_Title",
                "WrongWayImage",
            ],
            HudKind::Drift => &[
                "GaugeCluster",
                "Drift_Angle_Backing3",
                "POSITION",
                "PositionBacking",
                "LapCounterBacking_1",
                "NumberLaps",
                "LapNumber",
                "LapText",
                "Track2000_map",
                "Minimap_Mask",
                "TrackMapTargetRing",
                "321_Go",
                "BestTimeCounter",
                "StylePointsGroup",
                "StylePointsGroup_1",
                "TotalPointsGroup",
                "BestDriftCounter",
                "SP_Drift_Point",
                "SP_Drift_Title",
                "Comment_DATA",
                "LeaderBoardGroup",
                "Leader_bottom",
            ],
            HudKind::Drag => &[
                "GaugeCluster",
                "Group",
                "Group(1)",
                "NOS",
                "Group(12)",
                "HEAT_Bar",
                "321_Go",
                "ShiftMessage",
                "RaceTime",
                "Shift_light",
                "PositionBacking",
                "Drag_Position_Backing_03",
            ],
        }
    }
}

pub struct HudInput {
    /// A race (position, laps, leaderboard); false in free roam.
    pub race: bool,
    pub drift: Option<DriftInput>,
    /// Race clock (s), drag heat 0..1, and the last shift message (label, serial).
    pub race_time: f32,
    pub heat: f32,
    pub shift_message: Option<(u32, u32)>,
    pub speed_ms: f32,
    /// Engine speed (rpm).
    pub rpm: f32,
    pub redline: f32,
    pub max_rpm: f32,
    pub turbo: bool,
    /// −1 reverse, 0 neutral, 1.. forward.
    pub gear: i32,
    pub mph: bool,
    /// Nitrous left 0..1, and whether it fires.
    pub nitro: Option<(f32, bool)>,
    pub place: usize,
    pub racers: usize,
    pub lap: u32,
    pub laps: u32,
    /// Sprint progress 0..1 instead of laps.
    pub progress: Option<f32>,
    pub lap_time: Option<f32>,
    /// Seconds left of the countdown.
    pub countdown: Option<f32>,
    pub go_flash: f32,
    pub wrong_way: bool,
    /// (position, name, gap to the player in s, is player, text instead of the gap).
    pub leaders: Vec<(usize, String, f32, bool, Option<String>)>,
    pub map: Option<MapInput>,
}

pub struct MapInput {
    /// TRACKMAP texture name hash.
    pub texture: u32,
    /// World (x, y) of the image's top-left, its width, zoom.
    pub calib: [f32; 4],
    pub rotate: bool,
    /// Player (x, y, heading radians), then the others.
    pub player: (f32, f32, f32),
    pub others: Vec<(f32, f32, f32)>,
}

pub struct RaceHud {
    pub scene: Scene,
    pub kind: HudKind,
    /// Drift HUD: banks seen, seconds since the last bank and since the drift ended.
    banks_seen: u32,
    comment_time: f32,
    drift_idle: f32,
    shown: Vec<u32>,
    shift_serial: u32,
    shift_time: f32,
    /// The map object, drawn as a disc.
    pub map_object: Option<usize>,
    /// Extra virtual units on each side of the 640-wide frame (wide screens); the HUD's
    /// side groups stick to the edges, as the PC widescreen HUD does.
    pub wide: f32,
    time: f32,
    /// Dial maximum (10500 / 9500 / 8500 / 7500) for the current engine.
    dial: Option<(f32, f32)>,
}

/// Objects shown in a single race; everything else at the top level is hidden.
const SHOWN: [&str; 21] = [
    "GaugeCluster",
    "POSITION",
    "PositionBacking",
    "BestTimeCounter",
    "LapLeftGroup",
    "LeaderBoardGroup",
    "Track2000_map",
    "Minimap_Mask",
    "TrackMapTargetRing",
    "Minimap_North_indicator",
    "PlayerCarIndicator_1",
    "321_Go",
    "Lap_Time_Text",
    "Lap_Time_Title",
    "WrongWayImage",
    "Minimap_North_indicator",
    "AIRacerIndicator_0",
    "AIRacerIndicator_1",
    "AIRacerIndicator_2",
    "AIRacerIndicator_3",
    "AIRacerIndicator_4",
];

impl RaceHud {
    pub fn new(mut scene: Scene, kind: HudKind) -> Self {
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let shown: Vec<u32> = kind.shown().iter().map(|n| h(n)).collect();
        for i in 0..scene.package.objects.len() {
            let o = &scene.package.objects[i];
            if o.parent.is_some() {
                continue;
            }
            // The tachometer's backing group sits right under the gauge cluster.
            let backing = kind == HudKind::Single && o.hash == h("Group") && o.data[7] > 100.0 && o.data[8] > 150.0;
            let on = shown.contains(&o.hash) || backing;
            scene.states[i].hidden = Some(!on);
        }
        for n in ["Lap_Time_Text", "Lap_Time_Title", "WrongWayImage", "321_Go", "Shift_light", "ShiftMessage", "Comment_DATA", "SP_Drift_Point", "SP_Drift_Title"] {
            if let Some(i) = scene.index_of(h(n)) {
                scene.states[i].hidden = Some(true);
            }
        }
        let map_object = scene.index_of(h("Track2000_map"));
        RaceHud { scene, kind, banks_seen: 0, comment_time: 9.0, drift_idle: 9.0, shown, shift_serial: 0, shift_time: 0.0, map_object, wide: 0.0, time: 0.0, dial: None }
    }

    /// Wide screens: side groups stick to the screen edges, as the PC widescreen HUD
    /// does. (The touch layout of docs/hud-target.png comes at the end, owner's call.)
    fn offset(&self, _name_hash: u32, x: f32) -> [f32; 2] {
        [if x > 0.0 { self.wide } else { -self.wide }, 0.0]
    }

    fn place_groups(&mut self) {
        for i in 0..self.scene.package.objects.len() {
            let o = &self.scene.package.objects[i];
            if o.parent.is_some() || self.scene.states[i].hidden == Some(true) && !self.shown.contains(&o.hash) {
                continue;
            }
            let [dx, dy] = self.offset(o.hash, o.data[7]);
            self.scene.states[i].position = Some([o.data[7] + dx, o.data[8] + dy]);
        }
    }

    fn set_text(&mut self, name: &str, text: String) {
        if let Some(i) = self.scene.index_of(h(name)) {
            self.scene.states[i].text = Some(text);
        }
    }

    fn show(&mut self, name: &str, on: bool) {
        if let Some(i) = self.scene.index_of(h(name)) {
            self.scene.states[i].hidden = Some(!on);
        }
    }

    pub fn update(&mut self, t: &HudInput, strings: &Strings, textures: &std::collections::HashSet<u32>, dt: f32) {
        self.time += dt;
        self.scene.advance(dt * 1000.0);
        self.place_groups();
        if self.kind == HudKind::Drag {
            self.drag(t, strings, textures, dt);
        } else {
            self.tachometer(t, strings, textures);
        }
        if let Some(d) = t.drift {
            self.drift(&d, strings, dt);
        }
        // Race position: "%d", the suffix, "/%d".
        let suffix = match t.place % 10 {
            _ if t.place == 10 => "th",
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        };
        self.set_text("RacePositionNumber", t.place.to_string());
        self.set_text("RacePositionNth", suffix.to_string());
        self.set_text("NumberRacers", format!("/{}", t.racers));
        // Laps, or the progress of a sprint.
        match t.progress {
            Some(p) => {
                self.set_text("LapNumber", format!("{:3}%", (p * 100.0) as i32));
                self.set_text("NumberLaps", String::new());
                if let Some(c) = strings.by_hash(bin_hash("Completed")) {
                    self.set_text("LapText", c.to_string());
                }
            }
            None => {
                self.set_text("LapNumber", format!("{:2}", t.lap.min(t.laps).max(1)));
                self.set_text("NumberLaps", format!("/{:2}", t.laps));
            }
        }
        let lap = t.lap_time.map_or_else(|| strings.by_hash(0x61c9_b7a2).unwrap_or("--:--.--").to_string(), |x| race_time(x, false));
        self.set_text("CurrentLapTime", lap);
        if self.kind != HudKind::Drag {
            for n in ["POSITION", "PositionBacking", "BestTimeCounter", "LapLeftGroup", "LeaderBoardGroup"] {
                self.show(n, t.race);
            }
        }
        self.leaderboard(t);
        self.countdown(t);
        self.show("WrongWayImage", t.wrong_way);
        if let Some(m) = &t.map {
            self.minimap(m);
        }
    }

    fn tachometer(&mut self, t: &HudInput, strings: &Strings, textures: &std::collections::HashSet<u32>) {
        // Engine class (0x60fe10) → dial maximum and redline mark.
        let (red, max) = (t.redline, t.max_rpm.max(t.redline));
        let class = if max >= 9000.0 && red >= 8500.0 {
            if red >= 9000.0 { 0 } else { 1 }
        } else if max >= 8000.0 && red >= 7500.0 {
            if red >= 8000.0 { 2 } else { 3 }
        } else if max >= 7000.0 && red >= 6500.0 {
            if red >= 7000.0 { 4 } else { 5 }
        } else if red >= 6000.0 {
            6
        } else {
            7
        };
        let dial = [10500.0, 10500.0, 9500.0, 9500.0, 8500.0, 8500.0, 7500.0, 7500.0][class];
        if self.dial != Some((dial, red)) {
            self.dial = Some((dial, red));
            let tex = |n: String| {
                let x = bin_hash(&n);
                textures.contains(&x).then_some(x)
            };
            let lines = tex(format!("3RDPERSON_{}LINES_CUSTOM_00", dial as u32));
            let redline = ["9000", "8500", "8000", "7500", "7000", "6500", "6000", "5500"][class];
            let sets = [
                ("tach_fill", tex("TACH_FILL_CUSTOM_00".into())),
                ("3rdPersonNeedle", tex("3RDPERSON_RPMDIAL_CUSTOM_00".into())),
                ("3rdPersonTachRPMNumbers", lines),
                ("Redline", tex(format!("3RDPERSON_REDLINE_{redline}"))),
                ("3rdperson_Turbolines", tex("3RDPERSON_TURBOLINES_CUSTOM_00".into())),
                ("3rdperson_Turbofill", tex("3RDPERSON_TURBOFILL_CUSTOM_00".into())),
                ("3rdperson_TurboDial", tex("3RDPERSON_TURBODIAL_CUSTOM_00".into())),
            ];
            for (n, x) in sets {
                if let (Some(i), Some(x)) = (self.scene.index_of(h(n)), x) {
                    self.scene.states[i].texture = Some(x);
                }
            }
        }
        // Needle: rpm / dial max × 235°.
        let angle = (t.rpm / max.max(1.0)).clamp(0.0, 1.0) * (max / dial * 235.0);
        if let Some(i) = self.scene.index_of(h("3rdPersonNeedle")) {
            self.scene.states[i].rotation = Some(angle % 360.0);
        }
        let (v, unit) = if t.mph { (t.speed_ms * 2.237, 0x8569_ab44) } else { (t.speed_ms * 3.6, 0x8569_a25f) };
        self.set_text("3rdPersonSpeed", format!("{:3}", v as i32));
        self.set_text("3rdPersonSpeedUnits", strings.by_hash(unit).unwrap_or("").to_string());
        let gear = match t.gear {
            g if g < 0 => 'R',
            0 => 'N',
            g if g <= 7 => (b'0' + g as u8) as char,
            _ => 'X',
        };
        self.set_text("3rdPersonGear", gear.to_string());
        self.show("Turbo_Group", t.turbo);
        // Shift light near the redline.
        self.show("Shift_light", t.gear > 0 && t.rpm > red * 0.95);
        // Nitrous (0x4c7b20): the mask turns 152° over the bottle.
        match t.nitro {
            Some((v, firing)) if v > 0.0 => {
                self.show("NOS_Gamebreaker", true);
                self.show("nos_line", firing);
                if let Some(i) = self.scene.index_of(h("NOS_Mask")) {
                    self.scene.states[i].rotation = Some(152.0 * v - 180.0);
                    self.scene.states[i].color = Some(if firing { [0xb4, 0x00, 0xfa, 0x96] } else { [0x00, 0x5a, 0x7d, 0x96] });
                }
            }
            _ => {
                self.show("NOS_Gamebreaker", false);
                self.show("nos_line", false);
            }
        }
    }

    /// The drag tachometer (0x4c7090): needle −43° + 86° × rpm / max (every car type
    /// has −43 / +43 at +0x7fc / +0x800), DRAG_RPM_<dial>_LINES and the redline mark,
    /// the shift light and message, the heat bar (UV crop 0x4c66f0) and the race clock.
    fn drag(&mut self, t: &HudInput, strings: &Strings, textures: &std::collections::HashSet<u32>, dt: f32) {
        let (red, max) = (t.redline, t.max_rpm.max(t.redline));
        let class = if max >= 9000.0 && red >= 8500.0 {
            if red >= 9000.0 { 0 } else { 1 }
        } else if max >= 8000.0 && red >= 7500.0 {
            if red >= 8000.0 { 2 } else { 3 }
        } else if max >= 7000.0 && red >= 6500.0 {
            if red >= 7000.0 { 4 } else { 5 }
        } else if red >= 6000.0 {
            6
        } else {
            7
        };
        let dial = [10000.0, 10000.0, 9000.0, 9000.0, 8000.0, 8000.0, 7000.0, 7000.0][class];
        if self.dial != Some((dial, red)) {
            self.dial = Some((dial, red));
            let redline = ["9000", "8500", "8000", "7500", "7000", "6500", "6000", "5500"][class];
            for (n, tex) in [("3rdPersonTachRPMNumbers", format!("DRAG_RPM_{}_LINES", dial as u32)), ("RPM_Redline", format!("DRAG_RPM_REDLINES_{redline}"))] {
                let x = bin_hash(&tex);
                if let (Some(i), true) = (self.scene.index_of(h(n)), textures.contains(&x)) {
                    self.scene.states[i].texture = Some(x);
                }
            }
        }
        let f = (t.rpm / max.max(1.0)).clamp(0.0, 1.0);
        if let Some(i) = self.scene.index_of(h("3rdPersonNeedle")) {
            self.scene.states[i].rotation = Some((-43.0 + 86.0 * f).rem_euclid(360.0));
        }
        let (v, unit) = if t.mph { (t.speed_ms * 2.237, 0x8569_ab44) } else { (t.speed_ms * 3.6, 0x8569_a25f) };
        self.set_text("3rdPersonSpeed", format!("{:3}", v as i32));
        self.set_text("3rdPersonSpeedUnits", strings.by_hash(unit).unwrap_or("").to_string());
        let gear = match t.gear {
            g if g < 0 => 'R',
            0 => 'N',
            g if g <= 7 => (b'0' + g as u8) as char,
            _ => 'X',
        };
        self.set_text("3rdPersonGear", gear.to_string());
        self.show("Group(12)", t.turbo);
        // Shift light: close to the shift point it pulses blue, green when right.
        let near = t.gear > 0 && t.rpm > red - 600.0;
        if let Some(i) = self.scene.index_of(h("Shift_light")) {
            self.scene.states[i].hidden = Some(!near);
            if near {
                let want = if t.rpm > red + 30.0 {
                    h("PulseRed")
                } else if t.rpm > red - 300.0 {
                    h("PulseGreen")
                } else {
                    h("PulseBlue")
                };
                let st = &self.scene.states[i];
                if st.script != want || !st.running {
                    self.scene.set_script(i, want);
                }
            }
        }
        // "Perfect Shift!" and friends: ZoomIn, hidden 0.6 s later.
        if let Some((label, serial)) = t.shift_message {
            if serial != self.shift_serial {
                self.shift_serial = serial;
                self.shift_time = 0.0;
                if let Some(i) = self.scene.index_of(h("ShiftMessage")) {
                    self.scene.states[i].text = strings.by_hash(label).map(str::to_string);
                    self.scene.states[i].hidden = Some(false);
                    self.scene.set_script(i, h("ZoomIn"));
                }
            }
        }
        self.shift_time += dt;
        if self.shift_time > 0.6 {
            self.show("ShiftMessage", false);
        }
        // Heat bar: the top of the bar's texture moves with the heat, its bottom stays.
        if let Some(i) = self.scene.index_of(h("HEAT_Bar")) {
            let d = self.scene.package.objects[i].data;
            let hgt = d[15].max(1.0);
            let v = t.heat.clamp(0.0, 1.0);
            let top_v = ((hgt - 105.0) / hgt) + (5.0 / hgt - (hgt - 105.0) / hgt) * v;
            let new_h = (1.0 - top_v) * hgt;
            let st = &mut self.scene.states[i];
            st.uv = Some([d[17], top_v, d[19], d[20]]);
            st.size = Some([d[14], new_h]);
            let [px, py] = st.position.unwrap_or([d[7], d[8]]);
            st.position = Some([px, py + (hgt - new_h) * 0.5]);
        }
        self.set_text("RaceTime", race_time(t.race_time, false));
        match t.nitro {
            Some((v, _)) if v > 0.0 => self.show("nos_line", true),
            _ => self.show("nos_line", false),
        }
    }

    /// Drift scoring (0x4da590 gauge, 0x4c85d0 scores).
    fn drift(&mut self, d: &DriftInput, strings: &Strings, dt: f32) {
        self.set_text("Angle_Indicator_DATA", format!("{}", d.angle.abs().min(90.0) as i32));
        // The drift in progress: "DRIFT" and its points, hidden 1 s after it ends.
        if d.pending > 0 {
            self.drift_idle = 0.0;
            self.set_text("SP_Drift_Point", thousands(d.pending));
            if let Some(t) = strings.by_hash(0xa46a_aea8) {
                self.set_text("SP_Drift_Title", t.to_string());
            }
        } else {
            self.drift_idle += dt;
        }
        let live = self.drift_idle < 1.0;
        self.show("SP_Drift_Point", live);
        self.show("SP_Drift_Title", live);
        self.set_text("SP_DRIFT_TOTAL", thousands(d.total));
        self.set_text("BestDrift", thousands(d.best));
        self.set_text("drift_multiplier_text", format!("X{}", d.multiplier));
        // Next multiplier: "Gain %dX with:" and the points it needs; at the top, "MAX".
        match crate::drift::MULTIPLIER_STEPS.get(d.multiplier as usize - 1) {
            Some(&need) => {
                if let Some(t) = strings.by_hash(0xe03e_f7a4) {
                    self.set_text("NextSpeeed_Trap_Indicator", t.replace("%d", &(d.multiplier + 1).to_string()));
                }
                self.set_text("SpeedTrap_Speed_DATA", thousands(need));
            }
            None => {
                self.set_text("NextSpeeed_Trap_Indicator", strings.by_hash(0xcc62_c8be).unwrap_or("MAX").to_string());
                self.set_text("SpeedTrap_Speed_DATA", "----".into());
            }
        }
        // A banked drift's comment, shown 2 s.
        if d.banks != self.banks_seen {
            self.banks_seen = d.banks;
            if let Some(&(_, label)) = DRIFT_COMMENTS.iter().find(|(t, _)| d.last_bank >= *t) {
                self.comment_time = 0.0;
                if let Some(c) = strings.by_hash(label) {
                    self.set_text("Comment_DATA", c.to_string());
                }
            }
        }
        self.comment_time += dt;
        self.show("Comment_DATA", self.comment_time < 2.0);
    }

    fn leaderboard(&mut self, t: &HudInput) {
        // Gaps for 10 s, then names for 2 s.
        let names = self.time % 12.0 >= 10.0;
        for k in 0..4 {
            let n = k + 1;
            let row = t.leaders.get(k);
            for p in ["LeaderText", "LBData", "LBTextBacking", "LBBacking"] {
                self.show(&format!("{p}_{n}"), row.is_some());
            }
            let Some((pos, name, gap, player, value)) = row else { continue };
            self.set_text(&format!("LeaderText_{n}"), format!("{pos} :"));
            let data = match value {
                Some(v) => v.clone(),
                None if names || *player => name.clone(),
                None => {
                    let sign = if *gap < 0.0 { "-" } else { "+" };
                    format!("{sign}{}", race_time(gap.abs(), true))
                }
            };
            self.set_text(&format!("LBData_{n}"), data);
            if let Some(i) = self.scene.index_of(h(&format!("LBData_{n}"))) {
                let want = if *player { h("SetPlayerColor") } else { h("Init") };
                if self.scene.states[i].script != want {
                    self.scene.set_script(i, want);
                }
            }
        }
    }

    fn countdown(&mut self, t: &HudInput) {
        let Some(i) = self.scene.index_of(h("321_Go")) else { return };
        let (text, f) = match t.countdown {
            Some(c) if c > 0.0 => ((c.ceil() as i32).to_string(), c.fract()),
            _ if t.go_flash > 0.0 => ("GO!".to_string(), t.go_flash.min(1.0)),
            _ => {
                self.scene.states[i].hidden = Some(true);
                return;
            }
        };
        let st = &mut self.scene.states[i];
        st.hidden = Some(false);
        st.text = Some(text);
        // Alpha f × 255, scale 2f + 1.
        let d = self.scene.package.objects[i].data;
        let st = &mut self.scene.states[i];
        st.tint = Some([255, 255, 255, (f * 255.0) as u8]);
        st.size = Some([d[14] * (2.0 * f + 1.0), d[15] * (2.0 * f + 1.0)]);
    }

    fn minimap(&mut self, m: &MapInput) {
        let [min_x, max_y, size, zoom] = m.calib;
        if size <= 0.0 {
            return;
        }
        let span = 0.3 / zoom.max(0.01);
        let (px, py, heading) = m.player;
        let (u, v) = ((px - min_x) / size, (max_y - py) / size);
        // Screen angles (degrees, clockwise): an icon pointing up is heading 90°. In
        // Rotate mode the map turns so the car points up.
        let icon = |hd: f32| 90.0 - hd.to_degrees();
        let map_rot = if m.rotate { -icon(heading) } else { 0.0 };
        let Some(map) = self.scene.index_of(h("Track2000_map")) else { return };
        let st = &mut self.scene.states[map];
        st.texture = Some(m.texture);
        st.uv = Some([u - span * 0.5, v - span * 0.5, u + span * 0.5, v + span * 0.5]);
        st.rotation = Some(map_rot);
        for n in ["TrackMapTargetRing", "Minimap_North_indicator"] {
            if let Some(i) = self.scene.index_of(h(n)) {
                self.scene.states[i].rotation = Some(map_rot);
            }
        }
        let d = self.scene.package.objects[map].data;
        let [ox, oy] = self.offset(h("Track2000_map"), d[7]);
        let (cx, cy, w) = (d[7] + ox, d[8] + oy, d[14]);
        if let Some(i) = self.scene.index_of(h("PlayerCarIndicator_1")) {
            self.scene.states[i].position = Some([cx, cy]);
            self.scene.states[i].rotation = Some(icon(heading) + map_rot);
        }
        let (sn, cs) = map_rot.to_radians().sin_cos();
        for k in 0..5 {
            let Some(i) = self.scene.index_of(h(&format!("AIRacerIndicator_{k}"))) else { continue };
            let Some(&(x, y, hd)) = m.others.get(k) else {
                self.scene.states[i].hidden = Some(true);
                continue;
            };
            // Offset in map widths, turned with the map; past radius 0.55 pinned to it.
            let (du, dv) = ((x - px) / size / span, (py - y) / size / span);
            let (ru, rv) = (du * cs - dv * sn, du * sn + dv * cs);
            let r = (ru * ru + rv * rv).sqrt();
            let k = if r > 0.55 { 0.55 / r } else { 1.0 };
            let st = &mut self.scene.states[i];
            st.hidden = Some(false);
            st.position = Some([cx + ru * k * w, cy + rv * k * w]);
            st.rotation = Some(icon(hd) + map_rot);
        }
    }
}
