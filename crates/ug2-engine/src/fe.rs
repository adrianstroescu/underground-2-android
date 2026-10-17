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
    /// A race of the catalogue (index) on the track select screen.
    Track(usize),
    /// Continue on the mode options screen.
    OptionsDone,
    /// A car was chosen on the car select screen.
    CarDone,
}

/// Which original screen is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeKind {
    MainMenu,
    QuickRaceMode,
    QuickRaceTrack,
    QuickRaceOptions,
    QuickRaceCar,
}

/// Cars a new profile has in quick race (speed2.exe table 0x7f7c08); the others unlock
/// through the career.
pub const STARTER_CARS: [&str; 8] = ["PEUGOT", "FOCUS", "COROLLA", "CIVIC", "ESCALADE", "GTO", "LANCEREVO8", "350Z"];

/// The quick race car select (UI_QRCarSelect, speed2.exe 0x4f9d90): one car at a time
/// with its manufacturer logo and name badge, the filter, and stat bars.
struct CarSelect {
    /// (label, cars) per filter that has cars, in the trigger's order (speed2.exe
    /// 0x4eed10: All Cars, Stock Cars, My Cars, Career, Sponsor Cars; empty ones are
    /// skipped).
    filters: Vec<(u32, Vec<(String, String)>)>,
    filter: usize,
    cars: Vec<(String, String)>,
    selected: usize,
    /// Seconds since the selection last changed (the car loads after 0.3 s).
    settled: f32,
    mode_label: u32,
}

/// Settings of one quick race mode. speed2.exe keeps one record per mode (0x83aac4 +
/// 20·mode, saved in the profile); the defaults come from 0x52a220.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeSettings {
    pub reverse: bool,
    pub lap_ko: bool,
    pub laps: u32,
    /// 0 None, 1 Minimum, 2 Moderate, 3 Maximum.
    pub traffic: u8,
    pub opponents: u8,
    /// 0 Easy, 1 Medium, 2 Hard.
    pub difficulty: u8,
    pub catch_up: bool,
}

impl Default for ModeSettings {
    fn default() -> Self {
        Self { reverse: false, lap_ko: false, laps: 3, traffic: 1, opponents: 3, difficulty: 1, catch_up: true }
    }
}

/// The rows of the mode options screen (speed2.exe 0x4ccf60 and the builders after it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OptKey {
    Direction,
    LapKo,
    Laps,
    Traffic,
    Opponents,
    Difficulty,
    CatchUp,
}

impl OptKey {
    fn label(self) -> u32 {
        match self {
            OptKey::Direction => 0xa88f_feb4,
            OptKey::LapKo => 0x7fd7_3849,
            OptKey::Laps => 0x4849_4e83,
            OptKey::Traffic => 0xeb9d_fc09,
            OptKey::Opponents => 0x3384_a679,
            OptKey::Difficulty => 0x4d15_6786,
            OptKey::CatchUp => 0x8b8e_913a,
        }
    }
}

/// One row's objects (OPTION_NAME_n, OPTION_DATA_n, LEFT_ARROW_n, RIGHT_ARROW_n,
/// OPTION_BACKING_n).
#[derive(Clone, Copy, Debug, Default)]
struct RowObjects {
    name: Option<usize>,
    data: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    backing: Option<usize>,
}

/// The scrolling option list of `UI_QRModeOptions.fng` (speed2.exe 0x552780). Rows are
/// laid out from the TITLE_MASTER and DATA_MASTER rectangles, one title height apart
/// (0x51ff80, 0x54ef30); 7 rows show at a time.
struct OptionList {
    keys: Vec<OptKey>,
    rows: Vec<RowObjects>,
    selected: usize,
    top: usize,
    /// Highlighted row whose scripts were last set.
    shown: Option<usize>,
    /// Left, top, width, height.
    title: [f32; 4],
    data: [f32; 4],
    settings: ModeSettings,
    mode: Option<crate::roads::Mode>,
    race: Option<crate::roads::RaceInfo>,
}

const VISIBLE_ROWS: usize = 7;

/// Something an option shows elsewhere on the screen while it is selected.
#[derive(Clone, Debug)]
pub enum Detail {
    Text(String),
    /// Texture (name hash) of an image object.
    Texture(u32),
}

#[derive(Clone, Debug)]
pub struct IconOption {
    /// Texture name hash of the icon (e.g. MAIN_ICON_CAREER).
    pub icon: u32,
    /// Label hash in the language table (e.g. "Career").
    pub label: u32,
    pub action: FeAction,
    /// (object name hash, what to show) while selected.
    pub details: Vec<(u32, Detail)>,
}

fn opt(icon: u32, label: u32, action: FeAction) -> IconOption {
    IconOption { icon, label, action, details: Vec::new() }
}

/// The icon scroller of a menu package.
#[derive(Default)]
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
    list: Option<OptionList>,
    car: Option<CarSelect>,
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
            opt(bin_hash("MAIN_ICON_CAREER"), 0x5e7e_1192, FeAction::Career),
            opt(bin_hash("MAIN_ICON_QUICK_RACE"), 0x2490_9fd7, FeAction::QuickRace),
            opt(bin_hash("MAIN_ICON_CUSTOMIZE_CAR"), 0xa530_39a3, FeAction::Customize),
            opt(bin_hash("MAIN_ICON_OPTIONS"), 0xed1f_0b2c, FeAction::Options),
        ];
        self.open_icon_menu("UI_Main.fng", FeKind::MainMenu, options)
    }

    /// Quick race modes, as speed2.exe 0x4b2d80 lists them for one player (Outrun only
    /// appears in other game modes; Free Run otherwise).
    pub fn open_quick_race_modes(&mut self) -> bool {
        use crate::roads::Mode;
        let options = vec![
            opt(bin_hash("MODE_ICON_CIRCUIT"), 0x93fd_ee74, FeAction::Mode(Mode::Circuit)),
            opt(bin_hash("MODE_ICON_SPRINT"), 0xdcb9_f861, FeAction::Mode(Mode::Sprint)),
            opt(bin_hash("MODE_ICON_DRAG"), 0x3510_2edf, FeAction::Mode(Mode::Drag)),
            opt(bin_hash("MODE_ICON_DRIFT"), 0xd716_2cfa, FeAction::Mode(Mode::Drift)),
            opt(bin_hash("MODE_ICON_SHORT_TRACK"), 0x30da_61e1, FeAction::Mode(Mode::StreetX)),
            opt(bin_hash("MODE_ICON_U_RACING_LEAGUE"), 0xc786_c4e7, FeAction::Mode(Mode::Url)),
            opt(bin_hash("MODE_ICON_FREE_RUN"), 0x6f24_21b7, FeAction::FreeRun),
        ];
        self.open_icon_menu("UI_QRModeSelect.fng", FeKind::QuickRaceMode, options)
    }

    /// Track select (UI_QRTrackSelect): one option per race of the mode, with the
    /// district icon (speed2.exe 0x4984b0), the track name (TRACK_NAME_%d /
    /// REVERSE_TRACK_NAME_%d), race type, distance and the race's map (TRACKMAP%d).
    pub fn open_track_select(&mut self, races: &[(usize, &crate::roads::RaceInfo)]) -> bool {
        use crate::roads::Mode;
        let mut options = Vec::new();
        for &(i, r) in races {
            let icon = match r.location {
                0 => bin_hash("LOCATION_ICON_UPPER_CLASS"),
                1 => bin_hash("LOCATION_ICON_CITY_CORE"),
                2 => bin_hash("LOCATION_ICON_SUBURBAN_HILLS"),
                3 => 0x4fde_6feb,
                4 => bin_hash("LOCATION_ICON_AIRPORT"),
                _ => h(match r.mode {
                    Mode::Drag => "Track_Icon_Drag",
                    Mode::Drift => "Track_Icon_Drift",
                    Mode::StreetX => "Track_Icon_Short_Track",
                    _ => "Track_Icon_U_Racing_League",
                }),
            };
            let label = bin_hash(&if r.reverse { format!("REVERSE_TRACK_NAME_{}", r.track) } else { format!("TRACK_NAME_{}", r.track) });
            // The race type label of the mode select screen.
            let mode_label = match r.mode {
                Mode::Circuit => 0x93fd_ee74,
                Mode::Sprint => 0xdcb9_f861,
                Mode::Drag => 0x3510_2edf,
                Mode::Drift => 0xd716_2cfa,
                Mode::StreetX => 0x30da_61e1,
                Mode::Url => 0xc786_c4e7,
            };
            let mode_text = self.data.as_ref().and_then(|d| d.strings.by_hash(mode_label)).unwrap_or("").to_string();
            let mut o = opt(icon, label, FeAction::Track(i));
            o.details = vec![
                (0x15e1_351b, Detail::Text(mode_text)),
                (h("Distance"), Detail::Text(format!("{:.2} km", r.length as f32 / 1000.0))),
                (0x0555_eeca, Detail::Text("--:--:--".into())),
                (h("Track2000_map"), Detail::Texture(bin_hash(&format!("TRACKMAP{}", r.track)))),
            ];
            options.push(o);
        }
        self.open_icon_menu("UI_QRTrackSelect.fng", FeKind::QuickRaceTrack, options)
    }

    /// Quick race mode options (UI_QRModeOptions). `mode` None is Free Run.
    pub fn open_mode_options(&mut self, mode: Option<crate::roads::Mode>, race: Option<crate::roads::RaceInfo>, settings: ModeSettings) -> bool {
        use crate::roads::Mode;
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case("UI_QRModeOptions.fng")) else { return false };
        let mut scene = Scene::new(pkg.clone());
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let reversible = race.as_ref().is_some_and(|r| r.reversible);
        let mut keys = Vec::new();
        if reversible {
            keys.push(OptKey::Direction);
        }
        let track = race.as_ref().map_or(0, |r| r.track);
        let drift_kind = race.as_ref().map_or(0, |r| r.drift_kind);
        match mode {
            Some(Mode::Circuit) => keys.extend([OptKey::LapKo, OptKey::Laps, OptKey::Traffic, OptKey::Opponents, OptKey::Difficulty, OptKey::CatchUp]),
            Some(Mode::Sprint) => keys.extend([OptKey::Traffic, OptKey::Opponents, OptKey::Difficulty, OptKey::CatchUp]),
            Some(Mode::Drag) => {
                if !(0x1073..=0x107f).contains(&track) {
                    keys.push(OptKey::Traffic);
                }
                keys.extend([OptKey::Opponents, OptKey::Difficulty, OptKey::CatchUp]);
            }
            Some(Mode::Drift) => {
                if drift_kind == 0 {
                    keys.extend([OptKey::Laps, OptKey::Opponents]);
                }
                keys.push(OptKey::Difficulty);
                if drift_kind == 1 {
                    keys.push(OptKey::Traffic);
                }
            }
            Some(Mode::Url | Mode::StreetX) => keys.extend([OptKey::Laps, OptKey::Opponents, OptKey::Difficulty, OptKey::CatchUp]),
            None => keys.push(OptKey::Traffic),
        }
        let find = |n: &str| scene.index_of(h(n));
        let rect = |i: Option<usize>, def: [f32; 4]| {
            i.map_or(def, |i| {
                // The rectangle as built (the object's own data, before its scripts).
                let d = &scene.package.objects[i].data;
                [d[7] - d[14] * 0.5, d[8] - d[15] * 0.5, d[14], d[15]]
            })
        };
        let title = rect(find("TITLE_MASTER"), [-272.0, -140.0, 240.0, 36.0]);
        let data_rect = rect(find("DATA_MASTER"), [26.0, -140.0, 200.0, 32.0]);
        let mut rows = Vec::new();
        for n in 1..=16 {
            rows.push(RowObjects {
                name: find(&format!("OPTION_NAME_{n}")),
                data: find(&format!("OPTION_DATA_{n}")),
                left: find(&format!("LEFT_ARROW_{n}")),
                right: find(&format!("RIGHT_ARROW_{n}")),
                backing: find(&format!("OPTION_BACKING_{n}")),
            });
        }
        // Unused rows, the sliders (not used here) and debug objects stay hidden.
        let mut hidden: Vec<usize> = Vec::new();
        for r in rows.iter().skip(keys.len()) {
            hidden.extend([r.name, r.data, r.left, r.right, r.backing].into_iter().flatten());
        }
        for n in 1..=16 {
            for part in ["KNOB", "HANDLE", "BEG", "END", "BASE"] {
                hidden.extend(find(&format!("SLIDER_{part}_{n}")));
            }
        }
        hidden.extend(["_EVENT_HANDLER_", "DEMO_CAR_SELECT"].into_iter().filter_map(find));
        rows.truncate(keys.len());
        if let Some(i) = find("HEADER_TEXT") {
            // "Quick Race Game Options".
            scene.states[i].text = data.strings.by_hash(0x4759_7606).map(str::to_string);
        }
        for r in &rows {
            for i in [r.name, r.data, r.left, r.right, r.backing].into_iter().flatten() {
                scene.states[i].hidden = Some(false);
                scene.set_script(i, h("UNHIGHLIGHT"));
            }
        }
        let mut menu = IconMenu::default();
        menu.hidden = hidden;
        let mut list = OptionList { keys, rows, selected: 0, top: 0, shown: None, title, data: data_rect, settings, mode, race };
        list.constrain();
        let mut s = FeScreen { scene, kind: FeKind::QuickRaceOptions, menu, list: Some(list), car: None };
        s.apply(&data.strings);
        self.screen = Some(s);
        true
    }

    /// Quick race car select. `cars` are (car type, manufacturer) in the list's order;
    /// `current` is selected first.
    pub fn open_car_select(&mut self, filters: Vec<(u32, Vec<(String, String)>)>, current: &str, mode_label: u32) -> bool {
        let filters: Vec<_> = filters.into_iter().filter(|(_, c)| !c.is_empty()).collect();
        let Some(cars) = filters.first().map(|f| f.1.clone()) else { return false };
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case("UI_QRCarSelect.fng")) else { return false };
        let mut scene = Scene::new(pkg.clone());
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let find = |n: &str| scene.index_of(h(n));
        let mut menu = IconMenu::default();
        menu.hidden.extend(["_EVENT_HANDLER_", "CarLevel_Debug"].into_iter().filter_map(find));
        menu.arrows = [find("ScrollBar_Arrow_1"), find("ScrollBar_Arrow_2")];
        let header = find("HEADER_TEXT");
        for i in menu.arrows.into_iter().flatten() {
            scene.set_script(i, h("static"));
        }
        if let Some(i) = header {
            // "Quick Race Car Select".
            scene.states[i].text = data.strings.by_hash(0x5ac1_fb59).map(str::to_string);
        }
        let selected = cars.iter().position(|(n, _)| n.eq_ignore_ascii_case(current)).unwrap_or(0);
        let car = CarSelect { filters, filter: 0, cars, selected, settled: 0.0, mode_label };
        let mut s = FeScreen { scene, kind: FeKind::QuickRaceCar, menu, list: None, car: Some(car) };
        s.apply(&data.strings);
        s.apply_car(&data.strings, &data.textures);
        self.screen = Some(s);
        true
    }

    /// Next / previous car filter; the car stays selected when the new list has it.
    pub fn step_filter(&mut self, delta: i32) -> bool {
        let Some(c) = self.screen.as_mut().and_then(|s| s.car.as_mut()) else { return false };
        let n = c.filters.len() as i32;
        if n < 2 {
            return false;
        }
        c.filter = (c.filter as i32 + delta).rem_euclid(n) as usize;
        let current = c.cars.get(c.selected).map(|x| x.0.clone()).unwrap_or_default();
        c.cars = c.filters[c.filter].1.clone();
        c.selected = c.cars.iter().position(|x| x.0 == current).unwrap_or(0);
        c.settled = 0.0;
        true
    }

    /// The car on the car select screen.
    pub fn selected_car(&self) -> Option<&str> {
        let c = self.screen.as_ref()?.car.as_ref()?;
        c.cars.get(c.selected).map(|(n, _)| n.as_str())
    }

    /// The car to show on the turntable: the selection once it has rested 0.3 s.
    pub fn wanted_car(&self) -> Option<&str> {
        let c = self.screen.as_ref()?.car.as_ref()?;
        if c.settled < 0.3 {
            return None;
        }
        c.cars.get(c.selected).map(|(n, _)| n.as_str())
    }

    /// The settings on the options screen.
    pub fn mode_settings(&self) -> Option<ModeSettings> {
        Some(self.screen.as_ref()?.list.as_ref()?.settings)
    }

    /// Up / down on a list screen.
    pub fn step_row(&mut self, delta: i32) -> bool {
        let Some(l) = self.screen.as_mut().and_then(|s| s.list.as_mut()) else { return false };
        let n = l.keys.len() as i32;
        if n == 0 {
            return false;
        }
        // The list wraps (+0x146).
        l.selected = (l.selected as i32 + delta).rem_euclid(n) as usize;
        if l.selected < l.top {
            l.top = l.selected;
        } else if l.selected >= l.top + VISIBLE_ROWS {
            l.top = l.selected + 1 - VISIBLE_ROWS;
        }
        true
    }

    /// The track map the selected option shows (to load), if any.
    pub fn wanted_texture(&self) -> Option<u32> {
        let s = self.screen.as_ref()?;
        s.menu.options.get(s.menu.selected)?.details.iter().find_map(|(_, d)| match d {
            Detail::Texture(t) => Some(*t),
            Detail::Text(_) => None,
        })
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
        let master_i = find("OPTION_MASTER").or_else(|| find("ICON_MASTER"));
        let master = master_i.map_or([0.0, -134.0], |i| {
            let d = scene.values(i);
            [d[7], d[8]]
        });
        let width = master_i.map_or(118.0, |i| scene.values(i)[14]);
        let mut slots: Vec<usize> = (1..=options.len()).filter_map(|k| find(&format!("option_{k}"))).collect();
        let mut hidden: Vec<usize> = master_i.into_iter().collect();
        // Templates and debug objects the game code hides.
        for k in options.len() + 1..=16 {
            hidden.extend(find(&format!("option_{k}")));
        }
        if slots.is_empty() {
            // Track select clones "TRACK_ICON" objects (40 copies in the package).
            let all: Vec<usize> = (0..scene.package.objects.len()).filter(|&i| scene.package.objects[i].hash == h("TRACK_ICON")).collect();
            slots = all.iter().copied().take(options.len()).collect();
            hidden.extend(all.iter().copied().skip(options.len()));
        }
        hidden.extend(find("_EVENT_HANDLER_"));
        // "Track Number DEBUG".
        hidden.extend(scene.index_of(0x53c9_56c4));
        let menu = IconMenu {
            options,
            selected: 0,
            scroll: 0.0,
            master,
            spacing: width + ICON_GAP,
            slots,
            title: find("ICON_TITLE").or_else(|| find("TRACK_NAME")),
            hidden,
            arrows: [find("ScrollBar_Arrow_1"), find("ScrollBar_Arrow_2")],
        };
        for &i in &menu.slots {
            scene.set_script(i, h("FadeIn"));
        }
        for i in menu.arrows.into_iter().flatten() {
            scene.set_script(i, h("static"));
        }
        let mut s = FeScreen { scene, kind, menu, list: None, car: None };
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
        if let Some(c) = &mut s.car {
            c.settled += dt;
        }
        let target = s.menu.selected as f32;
        let step = dt / SCROLL_TIME;
        let d = target - s.menu.scroll;
        s.menu.scroll = if d.abs() <= step { target } else { s.menu.scroll + d.signum() * step.max(d.abs() * dt * 8.0) };
        s.apply(&data.strings);
        s.apply_car(&data.strings, &data.textures);
    }

    /// Steps the selection (left / right); on a list screen, changes the highlighted
    /// row's value. True when something moved.
    pub fn step(&mut self, delta: i32) -> bool {
        let Some(s) = &mut self.screen else { return false };
        if let Some(c) = &mut s.car {
            // No wrap (speed2.exe 0x4e2590).
            let next = (c.selected as i32 + delta).clamp(0, c.cars.len() as i32 - 1) as usize;
            if next == c.selected {
                return false;
            }
            c.selected = next;
            c.settled = 0.0;
            if let Some(a) = if delta < 0 { s.menu.arrows[0] } else { s.menu.arrows[1] } {
                s.scene.set_script(a, h("Blink"));
            }
            return true;
        }
        if let Some(l) = &mut s.list {
            let changed = l.change(delta);
            if changed {
                let r = l.rows[l.selected];
                if let Some(a) = if delta < 0 { r.left } else { r.right } {
                    s.scene.set_script(a, h("Blink"));
                }
            }
            return changed;
        }
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
        if s.list.is_some() {
            return Some(FeAction::OptionsDone);
        }
        if s.car.is_some() {
            return Some(FeAction::CarDone);
        }
        Some(s.menu.options.get(s.menu.selected)?.action)
    }

    /// A tap at screen pixel (x, y): arrows scroll, an icon is selected (or, when it is
    /// already selected, activated).
    pub fn tap(&mut self, x: f32, y: f32, size: (u32, u32)) -> Option<FeAction> {
        let (vx, vy) = to_virtual(x, y, size);
        if self.screen.as_ref()?.list.is_some() {
            return self.tap_list(vx, vy);
        }
        if self.screen.as_ref()?.car.is_some() {
            // The filter arrows (PC_FilterType's) change the filter.
            for (obj, d) in [(0xcf6f_0439u32, -1), (0xaba9_3d3f, 1)] {
                let s = self.screen.as_ref()?;
                if let Some(i) = s.scene.index_of(obj) {
                    let v = s.scene.values(i);
                    if (vx - v[7] - 320.0).abs() < 30.0 && (vy - v[8] - 240.0).abs() < 30.0 {
                        self.step_filter(d);
                        return None;
                    }
                }
            }
            // The scroll arrows step; the car (badge, logo or the turntable) accepts.
            let s = self.screen.as_ref()?;
            let near = |i: usize, r: f32| {
                let d = s.scene.values(i);
                (vx - d[7] - 320.0).abs() < r && (vy - d[8] - 240.0).abs() < r
            };
            if s.menu.arrows[0].is_some_and(|a| near(a, 40.0)) {
                self.step(-1);
                return None;
            }
            if s.menu.arrows[1].is_some_and(|a| near(a, 40.0)) {
                self.step(1);
                return None;
            }
            if vy > 140.0 && vy < 380.0 && (vx - 320.0).abs() < 200.0 {
                return Some(FeAction::CarDone);
            }
            return None;
        }
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

    /// A tap on the option list, as the PC mouse: a row is highlighted; on the
    /// highlighted row the arrows (or the value) change it and its name accepts (the
    /// name objects post Button_Pressed). The console button bar is hidden on PC.
    fn tap_list(&mut self, vx: f32, vy: f32) -> Option<FeAction> {
        let l = self.screen.as_ref()?.list.as_ref()?;
        let (cx, cy) = (vx - 320.0, vy - 240.0);
        let row_h = l.title[3];
        let left = l.title[0];
        let right = l.data[0] + l.data[2];
        if cx < left - 30.0 || cx > right + 30.0 || cy < l.title[1] {
            return None;
        }
        let slot = ((cy - l.title[1]) / row_h) as usize;
        let k = l.top + slot;
        if slot >= VISIBLE_ROWS || k >= l.keys.len() {
            return None;
        }
        let selected = l.selected;
        let data_left = l.data[0];
        if k != selected {
            let d = k as i32 - selected as i32;
            self.step_row(d);
            return None;
        }
        if cx < data_left - 30.0 {
            return Some(FeAction::OptionsDone);
        }
        let delta = if cx < data_left + l.data[2] * 0.35 { -1 } else { 1 };
        self.step(delta);
        None
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

impl OptionList {
    fn value_text(&self, key: OptKey, strings: &ug2_formats::lang::Strings) -> String {
        let s = &self.settings;
        let label = |hash: u32, fallback: &str| strings.by_hash(hash).unwrap_or(fallback).to_string();
        let on_off = |on: bool| if on { label(bin_hash("On"), "On") } else { label(bin_hash("Off"), "Off") };
        match key {
            OptKey::Direction => {
                if s.reverse {
                    label(0x22f2_54af, "Reverse")
                } else {
                    label(0x9e4c_4648, "Forward")
                }
            }
            OptKey::LapKo => on_off(s.lap_ko),
            OptKey::Laps => s.laps.to_string(),
            OptKey::Traffic => match s.traffic {
                0 => label(0x896b_cb4d, "None"),
                1 => label(0xb6bd_9c95, "Minimum"),
                2 => label(0x907c_555e, "Moderate"),
                _ => label(0xb672_f15a, "Maximum"),
            },
            OptKey::Opponents => s.opponents.to_string(),
            OptKey::Difficulty => match s.difficulty {
                0 => label(0xdbbb_4e8f, "Easy"),
                1 => label(0xcab2_62de, "Medium"),
                _ => label(0xdbbc_f37c, "Hard"),
            },
            OptKey::CatchUp => on_off(s.catch_up),
        }
    }

    /// Opponents allowed: 0 (quick race) up to 3, or 5 in URL, one human; 2 on drag
    /// track 0x1132.
    fn max_opponents(&self) -> u8 {
        if self.race.as_ref().is_some_and(|r| r.mode == crate::roads::Mode::Drag && r.track == 0x1132) {
            return 2;
        }
        if self.mode == Some(crate::roads::Mode::Url) {
            5
        } else {
            3
        }
    }

    fn enabled(&self, key: OptKey) -> bool {
        !(self.settings.lap_ko && matches!(key, OptKey::Laps | OptKey::Opponents))
    }

    /// Lap KO forces 3 laps and 3 opponents.
    fn constrain(&mut self) {
        let max = self.max_opponents();
        let s = &mut self.settings;
        if s.lap_ko {
            s.laps = 3;
            s.opponents = 3;
        }
        s.opponents = s.opponents.min(max);
        s.laps = s.laps.clamp(1, 10);
    }

    /// Left / right on the highlighted row; every option wraps.
    fn change(&mut self, delta: i32) -> bool {
        let Some(&key) = self.keys.get(self.selected) else { return false };
        if !self.enabled(key) {
            return false;
        }
        let wrap = |v: i32, lo: i32, hi: i32| lo + (v - lo).rem_euclid(hi - lo + 1);
        let max = self.max_opponents() as i32;
        let s = &mut self.settings;
        match key {
            OptKey::Direction => s.reverse = !s.reverse,
            OptKey::LapKo => s.lap_ko = !s.lap_ko,
            OptKey::Laps => s.laps = wrap(s.laps as i32 + delta, 1, 10) as u32,
            OptKey::Traffic => s.traffic = wrap(s.traffic as i32 + delta, 0, 3) as u8,
            OptKey::Opponents => s.opponents = wrap(s.opponents as i32 + delta, 0, max) as u8,
            OptKey::Difficulty => s.difficulty = wrap(s.difficulty as i32 + delta, 0, 2) as u8,
            OptKey::CatchUp => s.catch_up = !s.catch_up,
        }
        self.constrain();
        true
    }
}

impl FeScreen {
    /// Places the option rows (speed2.exe 0x52d800 / 0x52dac0): the name at the title
    /// rectangle's top-left, the value centred in the data rectangle, the arrows just
    /// outside it, the backing centred on the row.
    fn apply_list(&mut self, strings: &ug2_formats::lang::Strings) {
        let Some(l) = &mut self.list else { return };
        let [tl, tt, _, th] = l.title;
        let [dl, _, dw, _] = l.data;
        let row_cx = (tl + dl + dw) * 0.5;
        if l.shown != Some(l.selected) {
            for (k, r) in l.rows.iter().enumerate() {
                let script = if k == l.selected { h("Highlight") } else { h("UNHIGHLIGHT") };
                if l.shown == Some(k) || k == l.selected || l.shown.is_none() {
                    for i in [r.name, r.data, r.left, r.right, r.backing].into_iter().flatten() {
                        self.scene.set_script(i, script);
                    }
                }
            }
            l.shown = Some(l.selected);
        }
        for (k, r) in l.rows.iter().enumerate() {
            let visible = k >= l.top && k < l.top + VISIBLE_ROWS;
            let y = tt + (k as f32 - l.top as f32) * th;
            let key = l.keys[k];
            let enabled = l.enabled(key);
            let st = &mut self.scene.states;
            if let Some(i) = r.name {
                st[i].hidden = Some(!visible);
                st[i].position = Some([tl, y]);
                st[i].text = strings.by_hash(key.label()).map(str::to_string);
                st[i].tint = (!enabled).then_some([255, 255, 255, 110]);
            }
            if let Some(i) = r.data {
                st[i].hidden = Some(!visible);
                st[i].position = Some([dl + dw * 0.5, y]);
                st[i].text = Some(l.value_text(key, strings));
                st[i].tint = (!enabled).then_some([255, 255, 255, 110]);
            }
            for (a, x) in [(r.left, dl - 12.0), (r.right, dl + dw + 12.0)] {
                if let Some(i) = a {
                    st[i].hidden = Some(!visible || !enabled);
                    st[i].position = Some([x, y + 12.0]);
                    // U2_MENU_ARROW (speed2.exe 0x54f059).
                    st[i].texture = Some(0x367b_3e04);
                }
            }
            if let Some(i) = r.backing {
                st[i].hidden = Some(!visible);
                // The group's box sits at (−17, 1) inside it.
                st[i].position = Some([row_cx + 17.0, y + th * 0.5 - 1.0]);
            }
        }
    }

    /// Car select (speed2.exe 0x511e60 / 0x4b2310): the logo is
    /// CARSELECT_MANUFACTURER_<make>, the name badge SECONDARY_LOGO_<car type>
    /// (GENERICPLACEHOLDER when missing); the end arrows hide at the ends of the list.
    fn apply_car(&mut self, strings: &ug2_formats::lang::Strings, textures: &std::collections::HashSet<u32>) {
        let Some(c) = &self.car else { return };
        let Some((name, make)) = c.cars.get(c.selected) else { return };
        let tex = |n: String| {
            let t = bin_hash(&n.to_ascii_uppercase());
            if textures.contains(&t) {
                t
            } else {
                bin_hash("GENERICPLACEHOLDER")
            }
        };
        let (logo, badge) = (tex(format!("CARSELECT_MANUFACTURER_{make}")), tex(format!("SECONDARY_LOGO_{name}")));
        let set_tex = |scene: &mut Scene, n: &str, t: u32| {
            if let Some(i) = scene.index_of(h(n)) {
                scene.states[i].texture = Some(t);
            }
        };
        set_tex(&mut self.scene, "ManufacturerLogo", logo);
        set_tex(&mut self.scene, "CarBadge", badge);
        if let Some(i) = self.scene.index_of(0x3e81_de59) {
            self.scene.states[i].text = strings.by_hash(c.filters[c.filter].0).map(str::to_string);
        }
        if let Some(i) = self.scene.index_of(h("RaceMode_DATA")) {
            self.scene.states[i].text = strings.by_hash(c.mode_label).map(str::to_string);
        }
        let n = c.cars.len();
        for (k, hidden) in [(0, c.selected == 0), (1, c.selected + 1 >= n)] {
            if let Some(a) = self.menu.arrows[k] {
                self.scene.states[a].hidden = Some(hidden);
            }
        }
    }

    /// Writes the controller state into the scene, as the game code does every frame.
    fn apply(&mut self, strings: &ug2_formats::lang::Strings) {
        self.apply_list(strings);
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
        if let Some(o) = m.options.get(m.selected) {
            for (obj, d) in &o.details {
                let Some(i) = self.scene.index_of(*obj) else { continue };
                match d {
                    Detail::Text(t) => self.scene.states[i].text = Some(t.clone()),
                    Detail::Texture(t) => self.scene.states[i].texture = Some(*t),
                }
            }
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
