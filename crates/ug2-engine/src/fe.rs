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
use crate::fe_list::{Label, OptionList, Row, Tap};
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
    /// An Options category was chosen (speed2.exe [0x83aa04]).
    OptionsCategory(u8),
    /// Accept on an Options row (its key).
    ListButton(&'static str),
    /// The help bar's Back button.
    Back,
    /// The help bar's Quit Game button.
    Quit,
    /// The help bar's Customize button (car select).
    CustomizeCar,
    /// Pause menu (speed2.exe 0x4c2bd2 race, 0x4c2902 free run).
    /// Career menu (UI_StartCareer).
    ResumeCareer,
    NewCareer,
    LoadGame,
    /// Post race: Continue on the results, the menu's items, and confirmations.
    ResultsNext,
    PostRestart,
    ConfirmRestart,
    RaceStats,
    /// Car select accepted, transmission chosen.
    CarChosen,
    TransmissionManual,
    TransmissionAuto,
    /// A dialog's button that only closes it.
    CloseDialog,
    /// "Quit to Main Menu?" answered OK.
    ConfirmQuit,
    /// Online / LAN play (no network game here).
    Online,
    ProfileManager,
    Resume,
    RestartRace,
    Replay,
    QuitToMenu,
    PauseOptions,
}

/// Which original screen is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeKind {
    MainMenu,
    QuickRaceMode,
    QuickRaceTrack,
    QuickRaceOptions,
    QuickRaceCar,
    OptionsMain,
    Options,
    Pause,
    PauseOptionsMain,
    PauseOptions,
    StartCareer,
    PostRaceResults,
    PostRace,
    PostRaceStats,
    Credits,
}

/// Credits.fng (speed2.exe 0x4b87b0): every line is a copy of CreditsArea running
/// ROLLCREDIT (5.2 s up the screen); each starts 0.3 s after the previous one (the
/// script's event at 300 ms).
struct Credits {
    slots: Vec<usize>,
    time: f32,
}

const CREDIT_SLOTS: usize = 18;
const CREDIT_STEP: f32 = 0.3;
const CREDIT_LEN: f32 = 5.2;

/// One row of the results table.
#[derive(Clone, Debug)]
pub struct ResultRow {
    pub name: String,
    /// Car type, shown through its CAR_NAME_<type> label.
    pub car: String,
    /// Finish time (s); None when the car did not finish.
    pub time: Option<f32>,
    pub retired: bool,
    /// Knocked out of a Lap KO race.
    pub knocked: bool,
    /// Drift score.
    pub score: Option<u32>,
    pub player: bool,
    /// Race stats: lap times and places, laps led, best lap, top and average speed (m/s).
    pub lap_times: Vec<f32>,
    pub lap_places: Vec<usize>,
    pub laps_led: u32,
    pub laps: u32,
    pub best_lap: Option<f32>,
    pub top_speed: f32,
    pub avg_speed: f32,
}

/// UI_PostRaceStats (fill speed2.exe 0x4e9c00): one racer at a time (left / right).
struct Stats {
    rows: Vec<ResultRow>,
    racer: usize,
    circuit: bool,
    mph: bool,
    from_menu: bool,
    /// (name, data) objects of the stat rows and (time, position) of the lap list.
    stat_objs: Vec<(Option<usize>, Option<usize>)>,
    lap_objs: Vec<(Option<usize>, Option<usize>)>,
    title: [f32; 4],
    data: [f32; 4],
    lap_time: [f32; 4],
    lap_pos: [f32; 4],
}

/// UI_PostRaceResults (speed2.exe 0x4bf290): rank, name, car and time per row; the
/// time column alternates Finish Time (4.5 s) and Time Behind (1.75 s), or shows the
/// score in drift.
struct Results {
    rows: Vec<ResultRow>,
    drift: bool,
    time: f32,
}

/// Race time as the game prints it (0x576d30, English): "M:SS.hh" with a space for a
/// single-digit minute (`strip` drops it), "SS.hh" under a minute.
pub fn race_time(t: f32, strip: bool) -> String {
    let hs = (t.max(0.0) * 100.0) as u32;
    let (m, s, h) = (hs / 6000, hs / 100 % 60, hs % 100);
    if m == 0 {
        return format!("{s:02}.{h:02}");
    }
    let lead = if m < 10 && !strip { " " } else { "" };
    format!("{lead}{m}:{s:02}.{h:02}")
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

/// Which list a list screen shows.
#[derive(Clone, Debug)]
pub enum ListKind {
    /// Quick race game options for a mode (None: Free Run) and the chosen race.
    QuickRace { mode: Option<crate::roads::Mode>, race: Option<crate::roads::RaceInfo> },
    /// An Options category (speed2.exe [0x83aa04]: 0 Audio, 2 Gameplay, 3 Player).
    Options(u8),
}

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
    list_kind: Option<ListKind>,
    car: Option<CarSelect>,
    results: Option<Results>,
    stats: Option<Stats>,
    credits: Option<Credits>,
}

#[derive(Default)]
pub struct Frontend {
    pub data: Option<Box<FrontendData>>,
    pub screen: Option<FeScreen>,
    /// The PC help bar (UI_PC_Help_Bar, in GLOBALB) under every screen.
    help: Option<HelpBar>,
    /// A dialog over the screen (GenericDialog_*, speed2.exe 0x558790).
    dialog: Option<Dialog>,
    /// The original race HUD while racing.
    pub race_hud: Option<crate::fe_hud::RaceHud>,
}

/// A generic dialog: the message in `message_blurb`, one to three buttons (two: left
/// and right; one: the middle one).
struct Dialog {
    scene: Scene,
    /// (button group, button text, action).
    buttons: Vec<(usize, usize, FeAction)>,
    selected: usize,
}

/// The PC help bar: up to six buttons, filled in the order of the screen's flag bits
/// (the 4th word of its factory entry, speed2.exe 0x7f7de4..; labels from 0x52d330).
struct HelpBar {
    scene: Scene,
    /// (slot icon, slot text, flag bit) of the buttons shown.
    buttons: Vec<(usize, usize, u32)>,
    flags: u32,
}

/// Help bar slots as the PC game fills them: bottom right, top right, then leftwards.
const HELP_SLOTS: [(&str, &str); 6] =
    [("icon_1", "button1"), ("icon_2", "button2"), ("icon_3", "button3"), ("icon_4", "button4"), ("icon_5", "button5"), ("icon_6", "Button6")];

/// Help bar flag bits and their labels (speed2.exe 0x52d330).
const HELP_LABELS: [(u32, u32); 4] = [(0x1, 0x1c10_1778), (0x4, 0x9f1b_2c7a), (0x400, 0x1c07_87e6), (0x1000, 0x8d5a_bf58)];

/// Help bar flags per screen (the factory table).
fn help_flags(kind: FeKind) -> u32 {
    match kind {
        FeKind::MainMenu => 0x5,
        FeKind::QuickRaceMode | FeKind::QuickRaceTrack | FeKind::QuickRaceOptions | FeKind::OptionsMain => 0x405,
        FeKind::QuickRaceCar => 0x1405,
        FeKind::Options => 0x401,
        FeKind::StartCareer | FeKind::PostRace => 0x405,
        FeKind::PostRaceResults => 0x5,
        FeKind::PostRaceStats => 0x404,
        FeKind::Credits => 0x401,
        FeKind::Pause | FeKind::PauseOptionsMain => 0x30404,
        FeKind::PauseOptions => 0x10400,
    }
}

fn h(name: &str) -> u32 {
    bin_hash(&name.to_ascii_uppercase())
}

impl Frontend {
    pub fn ready(&self) -> bool {
        self.data.is_some()
    }

    /// The main menu with the options of speed2.exe 0x4aee40 (icon, label): Career,
    /// Quick Race, Online, LAN, Customize, Profile Manager, Options.
    pub fn open_main_menu(&mut self) -> bool {
        let options = vec![
            opt(0x0370_4f3d, 0x5e7e_1192, FeAction::Career),
            opt(0x4e6f_bb02, 0x2490_9fd7, FeAction::QuickRace),
            opt(0x7718_49a5, 0x7b63_ff85, FeAction::Online),
            opt(0xd13a_2e66, 0x7294_434c, FeAction::Online),
            opt(0xb0c4_6023, 0xa530_39a3, FeAction::Customize),
            opt(0x6b30_3856, 0xbcb1_8f38, FeAction::ProfileManager),
            opt(0x3058_fe37, 0xed1f_0b2c, FeAction::Options),
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

    /// Quick race mode options (UI_QRModeOptions), rows as speed2.exe builds them
    /// (0x4ccf60 and the builders after it). `mode` None is Free Run.
    pub fn open_mode_options(&mut self, mode: Option<crate::roads::Mode>, race: Option<crate::roads::RaceInfo>, settings: ModeSettings) -> bool {
        use crate::roads::Mode;
        let reversible = race.as_ref().is_some_and(|r| r.reversible);
        let track = race.as_ref().map_or(0, |r| r.track);
        let drift_kind = race.as_ref().map_or(0, |r| r.drift_kind);
        let s = settings;
        let direction = || Row::choice("reverse", 0xa88f_feb4, vec![Label::Hash(0x9e4c_4648, "Forward"), Label::Hash(0x22f2_54af, "Reverse")], s.reverse as u8 as f32);
        let lap_ko = || Row::on_off("lap_ko", 0x7fd7_3849, s.lap_ko);
        let laps = || Row::number("laps", 0x4849_4e83, 1, 10, s.laps as f32);
        let traffic = || {
            let v = vec![
                Label::Hash(0x896b_cb4d, "None"),
                Label::Hash(0xb6bd_9c95, "Minimum"),
                Label::Hash(0x907c_555e, "Moderate"),
                Label::Hash(0xb672_f15a, "Maximum"),
            ];
            Row::choice("traffic", 0xeb9d_fc09, v, s.traffic as f32)
        };
        // Opponents: 0 (quick race) to 3, or 5 in URL, one human; 2 on drag track 0x1132.
        let max_opp = if mode == Some(Mode::Drag) && track == 0x1132 {
            2
        } else if mode == Some(Mode::Url) {
            5
        } else {
            3
        };
        let opponents = || Row::number("opponents", 0x3384_a679, 0, max_opp, s.opponents.min(max_opp as u8) as f32);
        let difficulty = || {
            let v = vec![Label::Hash(0xdbbb_4e8f, "Easy"), Label::Hash(0xcab2_62de, "Medium"), Label::Hash(0xdbbc_f37c, "Hard")];
            Row::choice("difficulty", 0x4d15_6786, v, s.difficulty as f32)
        };
        let catch_up = || Row::on_off("catch_up", 0x8b8e_913a, s.catch_up);
        let mut rows = Vec::new();
        if reversible {
            rows.push(direction());
        }
        match mode {
            Some(Mode::Circuit) => rows.extend([lap_ko(), laps(), traffic(), opponents(), difficulty(), catch_up()]),
            Some(Mode::Sprint) => rows.extend([traffic(), opponents(), difficulty(), catch_up()]),
            Some(Mode::Drag) => {
                if !(0x1073..=0x107f).contains(&track) {
                    rows.push(traffic());
                }
                rows.extend([opponents(), difficulty(), catch_up()]);
            }
            Some(Mode::Drift) => {
                if drift_kind == 0 {
                    rows.extend([laps(), opponents()]);
                }
                rows.push(difficulty());
                if drift_kind == 1 {
                    rows.push(traffic());
                }
            }
            Some(Mode::Url | Mode::StreetX) => rows.extend([laps(), opponents(), difficulty(), catch_up()]),
            None => rows.push(traffic()),
        }
        // "Quick Race Game Options".
        let ok = self.open_list("UI_QRModeOptions.fng", FeKind::QuickRaceOptions, 0x4759_7606, rows, ListKind::QuickRace { mode, race });
        if let Some(l) = self.screen.as_mut().and_then(|s| s.list.as_mut()) {
            qr_constrain(l);
        }
        ok
    }

    /// Opens a dialog with `message` (label hash) and buttons (label hash, action).
    pub fn open_dialog(&mut self, message: u32, buttons: Vec<(u32, FeAction)>) -> bool {
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case("GenericDialog_SMALL.fng")) else { return false };
        let mut scene = Scene::new(pkg.clone());
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let find = |s: &Scene, n: &str| s.index_of(h(n));
        if let Some(i) = find(&scene, "message_blurb") {
            scene.states[i].text = data.strings.by_hash(message).map(str::to_string);
        }
        // The memory card title and icon belong to the save dialogs.
        for n in ["Header_text_02", "U2_Save_Icon", "_EVENT_HANDLER_"] {
            if let Some(i) = find(&scene, n) {
                scene.states[i].hidden = Some(true);
            }
        }
        // (group, text) slots: left, right, middle.
        let slots = [(find(&scene, "button1"), scene.index_of(0xda47_3d2f)), (find(&scene, "button2"), scene.index_of(0xda47_3d30)), (find(&scene, "button3"), scene.index_of(0xda47_3d31))];
        let used: Vec<usize> = if buttons.len() == 1 { vec![2] } else { vec![0, 1, 2].into_iter().take(buttons.len()).collect() };
        let mut list = Vec::new();
        for (k, &(g, t)) in slots.iter().enumerate() {
            let on = used.contains(&k);
            for i in [g, t].into_iter().flatten() {
                scene.states[i].hidden = Some(!on);
            }
        }
        for (&slot, &(label, action)) in used.iter().zip(&buttons) {
            if let (Some(g), Some(t)) = slots[slot] {
                scene.states[t].text = data.strings.by_hash(label).map(str::to_string);
                list.push((g, t, action));
            }
        }
        let mut d = Dialog { scene, buttons: list, selected: 0 };
        for k in 0..d.buttons.len() {
            d.selected = k;
            d.highlight(false);
        }
        d.selected = 0;
        d.highlight(true);
        self.dialog = Some(d);
        true
    }

    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// The results table after a race.
    pub fn open_results(&mut self, rows: Vec<ResultRow>, drift: bool) -> bool {
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case("UI_PostRaceResults.fng")) else { return false };
        let mut scene = Scene::new(pkg.clone());
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let mut menu = IconMenu::default();
        // Online and console-only objects.
        for n in ["_EVENT_HANDLER_", "OL_Countdown_Timer", "Online_DATA_01"] {
            menu.hidden.extend(scene.index_of(h(n)));
        }
        for k in rows.len() + 1..=6 {
            for p in ["Player_Data", "Ranking_Data", "Car_Data", "Time_Data", "High_Light"] {
                menu.hidden.extend(scene.index_of(h(&format!("{p}_{k}"))));
            }
        }
        // The player's rank runs "Highlight".
        if let Some(k) = rows.iter().position(|r| r.player) {
            if let Some(i) = scene.index_of(h(&format!("Ranking_Data_{}", k + 1))) {
                scene.set_script(i, h("Highlight"));
            }
        }
        let results = Results { rows, drift, time: 0.0 };
        let mut s = FeScreen { scene, kind: FeKind::PostRaceResults, menu, list: None, list_kind: None, car: None, results: Some(results), stats: None, credits: None };
        s.apply(&data.strings);
        self.screen = Some(s);
        true
    }

    /// The post race menu (UI_PostRace, speed2.exe 0x4bee10) for a quick race: Restart,
    /// Race Stats, Quit to Main Menu (Replay only with a replay recorded).
    pub fn open_post_race(&mut self) -> bool {
        let options = vec![
            opt(0xb295_a6b6, 0x8083_3fc2, FeAction::PostRestart),
            opt(bin_hash("INGAME_ICON_STATS"), 0xf82d_f37e, FeAction::RaceStats),
            opt(0x4c9e_34e6, 0xc7e1_420c, FeAction::QuitToMenu),
        ];
        self.open_icon_menu("UI_PostRace.fng", FeKind::PostRace, options)
    }

    pub fn in_post_race(&self) -> bool {
        matches!(self.kind(), Some(FeKind::PostRaceResults | FeKind::PostRace | FeKind::PostRaceStats))
    }

    /// The credits roll; true when it has ended.
    pub fn credits_done(&self) -> bool {
        let (Some(s), Some(d)) = (&self.screen, &self.data) else { return false };
        s.credits.as_ref().is_some_and(|c| c.time > d.credits.len() as f32 * CREDIT_STEP + CREDIT_LEN)
    }

    pub fn open_credits(&mut self) -> bool {
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case("Credits.fng")) else { return false };
        let mut pkg = pkg.clone();
        let area = h("CreditsArea");
        let Some(src) = pkg.objects.iter().position(|o| o.hash == area) else { return false };
        // Copies of the line object, one per line on screen at a time.
        let mut slots = vec![src];
        for k in 1..CREDIT_SLOTS {
            let mut o = pkg.objects[src].clone();
            o.hash = area.wrapping_add(k as u32);
            o.guid = o.guid.wrapping_add(0x1000_0000 + k as u32);
            pkg.objects.push(o);
            slots.push(pkg.objects.len() - 1);
        }
        let mut scene = Scene::new(pkg);
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let mut menu = IconMenu::default();
        menu.hidden.extend(scene.index_of(h("_EVENT_HANDLER_")));
        let credits = Credits { slots, time: 0.0 };
        let mut s = FeScreen { scene, kind: FeKind::Credits, menu, list: None, list_kind: None, car: None, results: None, stats: None, credits: Some(credits) };
        s.apply(&data.strings);
        s.apply_credits(&data.credits);
        self.screen = Some(s);
        true
    }

    /// Whether the stats screen was opened from the post race menu (back goes there).
    pub fn stats_from_menu(&self) -> bool {
        self.screen.as_ref().and_then(|s| s.stats.as_ref()).is_some_and(|s| s.from_menu)
    }

    /// The race stats screen, starting on the player.
    pub fn open_race_stats(&mut self, rows: Vec<ResultRow>, circuit: bool, mph: bool, from_menu: bool) -> bool {
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case("UI_PostRaceStats.fng")) else { return false };
        let mut scene = Scene::new(pkg.clone());
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        let find = |s: &Scene, n: &str| s.index_of(h(n));
        let rect = |s: &Scene, n: &str, def: [f32; 4]| {
            find(s, n).map_or(def, |i| {
                let d = &s.package.objects[i].data;
                [d[7] - d[14] * 0.5, d[8] - d[15] * 0.5, d[14], d[15]]
            })
        };
        let title = rect(&scene, "TITLE_MASTER", [-46.0, -113.0, 188.0, 36.0]);
        let data_r = rect(&scene, "DATA_MASTER", [183.0, -113.0, 92.0, 32.0]);
        let lap_time = rect(&scene, "TIME_MASTER", [-283.0, -89.0, 82.0, 26.0]);
        let lap_pos = rect(&scene, "POSITION_MASTER", [-142.0, -89.0, 82.0, 26.0]);
        let stat_objs = (1..=10).map(|n| (find(&scene, &format!("option_name_{n}")), find(&scene, &format!("option_data_{n}")))).collect();
        let lap_objs = (1..=10).map(|n| (find(&scene, &format!("Time_Data_{n}")), find(&scene, &format!("position_data_{n}")))).collect();
        if let Some(i) = find(&scene, "HEADER_TEXT") {
            scene.states[i].text = data.strings.by_hash(0xc7da_078e).map(str::to_string);
        }
        let mut menu = IconMenu::default();
        menu.hidden.extend(["_EVENT_HANDLER_", "EVENT", "Back_button", "Joy_Event"].into_iter().filter_map(|n| find(&scene, n)));
        if !circuit {
            menu.hidden.extend(["Head_text_2", "Head_text_3"].into_iter().filter_map(|n| find(&scene, n)));
        }
        menu.arrows = [find(&scene, "ScrollBar_Arrow_1"), find(&scene, "ScrollBar_Arrow_2")];
        let racer = rows.iter().position(|r| r.player).unwrap_or(0);
        let stats = Stats { rows, racer, circuit, mph, from_menu, stat_objs, lap_objs, title, data: data_r, lap_time, lap_pos };
        let mut s = FeScreen { scene, kind: FeKind::PostRaceStats, menu, list: None, list_kind: None, car: None, results: None, stats: Some(stats), credits: None };
        s.apply(&data.strings);
        self.screen = Some(s);
        true
    }

    /// The career menu (UI_StartCareer, speed2.exe 0x4af110): Resume Career when a
    /// career is under way, else Start New Career; then Load Game.
    pub fn open_start_career(&mut self, in_progress: bool) -> bool {
        let mut options = Vec::new();
        if in_progress {
            options.push(opt(0xa230_cd88, 0xe072_db21, FeAction::ResumeCareer));
        }
        options.push(opt(0x3cb9_2e26, 0x97b1_dcda, FeAction::NewCareer));
        options.push(opt(0x72c8_7f3c, 0xaa3f_09d8, FeAction::LoadGame));
        let ok = self.open_icon_menu("UI_StartCareer.fng", FeKind::StartCareer, options);
        // The header reads "Career".
        if let (Some(s), Some(d)) = (self.screen.as_mut(), self.data.as_ref()) {
            if let Some(i) = s.scene.index_of(0x42ad_b44c) {
                s.scene.states[i].text = d.strings.by_hash(0x5e7e_1192).map(str::to_string);
            }
        }
        ok
    }

    /// The pause menu (UI_Pause). In a race (speed2.exe 0x4c2bd2): Resume Race,
    /// Restart, Options, Replay, Quit to Main Menu; in free run (0x4c2902): Return to
    /// Free Run, Options, Replay, Quit to Main Menu.
    pub fn open_pause(&mut self, race: bool) -> bool {
        let quit = opt(bin_hash("INGAME_ICON_QUIT_TO_FE"), 0xc7e1_420c, FeAction::QuitToMenu);
        let options_ = opt(bin_hash("INGAME_ICON_OPTIONS"), 0xb349_93c9, FeAction::PauseOptions);
        let replay = opt(bin_hash("INGAME_ICON_REPLAY"), 0x7840_428a, FeAction::Replay);
        let options = if race {
            vec![
                opt(bin_hash("INGAME_ICON_RESUME_RACE"), 0xf660_1088, FeAction::Resume),
                opt(0xb295_a6b6, 0x8083_3fc2, FeAction::RestartRace),
                options_,
                replay,
                quit,
            ]
        } else {
            vec![opt(bin_hash("INGAME_ICON_EXPLORE"), 0xc44a_6515, FeAction::Resume), options_, replay, quit]
        };
        self.open_icon_menu("UI_Pause.fng", FeKind::Pause, options)
    }

    /// The pause menu's options (UI_PauseOptionsMain, speed2.exe 0x4c30b1): Audio,
    /// Gameplay, Player.
    pub fn open_pause_options_main(&mut self) -> bool {
        let options = vec![
            opt(0xf37a_f144, 0xe76c_d783, FeAction::OptionsCategory(0)),
            opt(0x4df9_8fb2, 0xd0cf_6ee1, FeAction::OptionsCategory(2)),
            opt(0xd708_efef, 0xf760_eabe, FeAction::OptionsCategory(3)),
        ];
        self.open_icon_menu("UI_PauseOptionsMain.fng", FeKind::PauseOptionsMain, options)
    }

    /// True on the pause screens.
    pub fn in_pause(&self) -> bool {
        matches!(self.kind(), Some(FeKind::Pause | FeKind::PauseOptionsMain | FeKind::PauseOptions))
    }

    /// The Options categories (UI_OptionsMain, speed2.exe 0x4b6230). Display (PC
    /// resolution and detail) and the PC key bindings have no counterpart here yet.
    pub fn open_options_main(&mut self) -> bool {
        let options = vec![
            opt(0xf37a_f144, 0xe76c_d783, FeAction::OptionsCategory(0)),
            opt(0x4df9_8fb2, 0xd0cf_6ee1, FeAction::OptionsCategory(2)),
            opt(0xd708_efef, 0xf760_eabe, FeAction::OptionsCategory(3)),
            opt(0xc52c_cbf6, 0xdcfb_6b36, FeAction::OptionsCategory(5)),
            opt(0x5100_9e20, 0x0905_101f, FeAction::OptionsCategory(8)),
        ];
        self.open_icon_menu("UI_OptionsMain.fng", FeKind::OptionsMain, options)
    }

    /// An Options category (UI_Options, rows from speed2.exe 0x4b8470) with the saved
    /// values (`get(key)`, default when missing).
    pub fn open_options(&mut self, category: u8, get: impl Fn(&str) -> Option<f32>) -> bool {
        let pause = self.in_pause();
        let v = |k: &str, d: f32| get(k).unwrap_or(d);
        let off_on = |k: &'static str, label: u32, d: bool| Row::on_off(k, label, v(k, d as u8 as f32) != 0.0);
        let rows = match category {
            // Audio (0x4b6e60): volumes 0..1 in steps of 0.05.
            0 => vec![
                Row::slider("master_volume", 0x2678_2c3e, v("master_volume", 1.0)),
                Row::slider("menu_music_volume", 0x418e_681d, v("menu_music_volume", 0.7)),
                Row::slider("game_music_volume", 0xdf21_eac2, v("game_music_volume", 1.0)),
                Row::slider("speech_volume", 0x9e5f_b82a, v("speech_volume", 1.0)),
                Row::slider("sfx_volume", 0xfd48_7543, v("sfx_volume", 1.0)),
                Row::slider("tire_volume", 0x69b7_45c6, v("tire_volume", 1.0)),
                Row::slider("engine_volume", 0xa2b1_f888, v("engine_volume", 1.0)),
                Row::choice("audio_mode", 0x2881_ab87, vec![Label::Hash(0xc50f_a35f, "Mono"), Label::Hash(0x55da_8bf8, "Stereo")], v("audio_mode", 1.0)),
            ],
            // Gameplay (0x4b7310).
            2 => {
                let map = |k: &'static str, label: u32| {
                    let vals = vec![Label::Hash(0xccfa, "Off"), Label::Hash(0xf755_95f2, "Static"), Label::Hash(0xf4b0_0e99, "Rotate")];
                    Row::choice(k, label, vals, v(k, 2.0))
                };
                vec![
                    off_on("jump_camera", 0xf26a_5cbf, true),
                    off_on("rearview_mirror", 0x85a6_ce05, true),
                    Row::choice("speedo_units", 0xf86a_6272, vec![Label::Hash(0x8569_ab44, "MPH"), Label::Hash(0x8569_a25f, "KM/H")], v("speedo_units", 0.0)),
                    Row::choice(
                        "career_difficulty",
                        0xbc0e_6c55,
                        vec![Label::Hash(0xdbbb_4e8f, "Easy"), Label::Hash(0xcab2_62de, "Medium"), Label::Hash(0xdbbc_f37c, "Hard")],
                        v("career_difficulty", 1.0),
                    ),
                    map("explore_map", 0xc626_9082),
                    map("race_map", 0x9fa5_ec9e),
                    Row::choice(
                        "save",
                        0x9964_63e6,
                        vec![Label::Hash(0x1b7a_bf90, "Off"), Label::Hash(0x1b7a_bf8f, "Prompt"), Label::Hash(0x1b7a_bf8e, "Autosave")],
                        v("save", 1.0),
                    ),
                ]
            }
            // Player (0x4b7cd0); the wheel rows only show with a wheel.
            3 => vec![
                Row::choice("transmission", 0xd314_07e7, vec![Label::Hash(0x61cb_0720, "Auto"), Label::Hash(0x1b3a_2485, "Manual")], v("transmission", 0.0)),
                // Stored inverted: 0 = On.
                Row::choice("stability_control", 0xdf06_ed22, vec![Label::Hash(0x063c, "On"), Label::Hash(0xccfa, "Off")], v("stability_control", 1.0)),
                Row::choice(
                    "camera",
                    0xf6cc_dc5f,
                    vec![Label::Hash(0xc3e9_ae58, "Bumper"), Label::Hash(0x414f_19d7, "Hood"), Label::Hash(0x5ae3_441f, "Close"), Label::Hash(0x1ea4_cec2, "Far")],
                    v("camera", 3.0),
                ),
                off_on("gauges", 0xac14_8579, true),
                off_on("race_position", 0x82cd_8f92, true),
                off_on("lap_info", 0x969f_96a5, true),
                Row::choice(
                    "split_time",
                    0x084b_c378,
                    vec![
                        Label::Hash(0xd93f_8f55, "Leader"),
                        Label::Hash(0xc44d_3943, "Lap Record"),
                        Label::Hash(0x17fa_fc32, "Best Lap"),
                        Label::Hash(0x1ea4_59f8, "Last Lap"),
                        Label::Hash(0xccfa, "Off"),
                    ],
                    v("split_time", 0.0),
                ),
                off_on("score", 0xc03f_9f19, true),
                off_on("leaderboard", 0xc93f_bfb5, true),
                off_on("ask_transmission", 0xbfb3_4120, true),
            ],
            _ => return false,
        };
        // The header is the category's name.
        let header = match category {
            0 => 0xe76c_d783,
            2 => 0xd0cf_6ee1,
            _ => 0xf760_eabe,
        };
        if pause {
            return self.open_list("UI_PauseOptions.fng", FeKind::PauseOptions, header, rows, ListKind::Options(category));
        }
        self.open_list("UI_Options.fng", FeKind::Options, header, rows, ListKind::Options(category))
    }

    fn open_list(&mut self, package: &str, kind: FeKind, header: u32, rows: Vec<Row>, list_kind: ListKind) -> bool {
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case(package)) else { return false };
        let mut scene = Scene::new(pkg.clone());
        scene.advance(1.0);
        scene.send_named("CALL PACKAGE");
        scene.send_named("CALL FRAMES");
        if let Some(i) = scene.index_of(h("HEADER_TEXT")) {
            if let Some(t) = data.strings.by_hash(header) {
                scene.states[i].text = Some(t.to_string());
            }
        }
        let visible = if kind == FeKind::QuickRaceOptions { 7 } else { 8 };
        let (list, hidden) = OptionList::new(&mut scene, rows, visible);
        let mut menu = IconMenu::default();
        menu.hidden = hidden;
        let mut s = FeScreen { scene, kind, menu, list: Some(list), list_kind: Some(list_kind), car: None, results: None, stats: None, credits: None };
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
        let mut s = FeScreen { scene, kind: FeKind::QuickRaceCar, menu, list: None, list_kind: None, car: Some(car), results: None, stats: None, credits: None };
        s.apply(&data.strings);
        s.apply_car(data);
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

    /// The settings on the quick race options screen.
    pub fn mode_settings(&self) -> Option<ModeSettings> {
        let s = self.screen.as_ref()?;
        let l = s.list.as_ref()?;
        let Some(ListKind::QuickRace { .. }) = s.list_kind else { return None };
        let d = ModeSettings::default();
        let g = |k: &str, d: f32| l.get(k).unwrap_or(d);
        Some(ModeSettings {
            reverse: g("reverse", d.reverse as u8 as f32) != 0.0,
            lap_ko: g("lap_ko", d.lap_ko as u8 as f32) != 0.0,
            laps: g("laps", d.laps as f32) as u32,
            traffic: g("traffic", d.traffic as f32) as u8,
            opponents: g("opponents", d.opponents as f32) as u8,
            difficulty: g("difficulty", d.difficulty as f32) as u8,
            catch_up: g("catch_up", d.catch_up as u8 as f32) != 0.0,
        })
    }

    /// The values on an Options category screen: (category, [(key, value)]).
    pub fn options_values(&self) -> Option<(u8, Vec<(&'static str, f32)>)> {
        let s = self.screen.as_ref()?;
        let Some(ListKind::Options(c)) = s.list_kind else { return None };
        Some((c, s.list.as_ref()?.rows.iter().map(|r| (r.key, r.value)).collect()))
    }

    /// Up / down on a list screen.
    pub fn step_row(&mut self, delta: i32) -> bool {
        let Some(l) = self.screen.as_mut().and_then(|s| s.list.as_mut()) else { return false };
        l.step_row(delta);
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
        // As built (the object's own data): scripts such as UI_Pause's "Hidden" move it.
        let master = master_i.map_or([0.0, -134.0], |i| {
            let d = &scene.package.objects[i].data;
            [d[7], d[8]]
        });
        let width = master_i.map_or(118.0, |i| scene.package.objects[i].data[14]);
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
        let mut s = FeScreen { scene, kind, menu, list: None, list_kind: None, car: None, results: None, stats: None, credits: None };
        s.apply(&data.strings);
        self.screen = Some(s);
        true
    }

    /// Back closes a dialog (as its cancel button).
    pub fn close_dialog(&mut self) -> bool {
        self.dialog.take().is_some()
    }

    pub fn close(&mut self) {
        self.dialog = None;
        self.screen = None;
    }

    pub fn update(&mut self, dt: f32) {
        let (Some(s), Some(data)) = (&mut self.screen, &self.data) else { return };
        s.scene.advance(dt * 1000.0);
        if let Some(c) = &mut s.car {
            c.settled += dt;
        }
        if let Some(r) = &mut s.results {
            r.time += dt;
        }
        if let Some(c) = &mut s.credits {
            c.time += dt;
        }
        s.apply_credits(&data.credits);
        let kind = s.kind;
        if self.help.is_none() {
            self.help = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case("UI_PC_Help_Bar.fng")).map(|p| {
                let mut scene = Scene::new(p.clone());
                scene.advance(1.0);
                scene.send_named("CALL PACKAGE");
                HelpBar { scene, buttons: Vec::new(), flags: u32::MAX }
            });
        }
        if let Some(d) = &mut self.dialog {
            d.scene.advance(dt * 1000.0);
        }
        if let Some(hb) = &mut self.help {
            hb.scene.advance(dt * 1000.0);
            let flags = help_flags(kind);
            if hb.flags != flags {
                hb.set(flags, &data.strings);
            }
        }
        let target = s.menu.selected as f32;
        let step = dt / SCROLL_TIME;
        let d = target - s.menu.scroll;
        s.menu.scroll = if d.abs() <= step { target } else { s.menu.scroll + d.signum() * step.max(d.abs() * dt * 8.0) };
        s.apply(&data.strings);
        s.apply_car(data);
    }

    /// Steps the selection (left / right); on a list screen, changes the highlighted
    /// row's value. True when something moved.
    pub fn step(&mut self, delta: i32) -> bool {
        if let Some(d) = &mut self.dialog {
            let n = d.buttons.len() as i32;
            let next = (d.selected as i32 + delta).clamp(0, n - 1) as usize;
            if next != d.selected {
                d.highlight(false);
                d.selected = next;
                d.highlight(true);
            }
            return true;
        }
        let Some(s) = &mut self.screen else { return false };
        if let Some(st) = &mut s.stats {
            let next = (st.racer as i32 + delta).clamp(0, st.rows.len() as i32 - 1) as usize;
            if next == st.racer {
                return false;
            }
            st.racer = next;
            if let Some(a) = if delta < 0 { s.menu.arrows[0] } else { s.menu.arrows[1] } {
                s.scene.set_script(a, h("Blink"));
            }
            return true;
        }
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
            let changed = l.change(delta, &mut s.scene);
            if changed && matches!(s.list_kind, Some(ListKind::QuickRace { .. })) {
                qr_constrain(l);
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

    pub fn activate(&mut self) -> Option<FeAction> {
        if let Some(d) = self.dialog.take() {
            return d.buttons.get(d.selected).map(|b| b.2).filter(|a| *a != FeAction::CloseDialog);
        }
        let s = self.screen.as_ref()?;
        if let Some(l) = &s.list {
            return match s.list_kind {
                Some(ListKind::QuickRace { .. }) => Some(FeAction::OptionsDone),
                _ => l.selected_key().map(FeAction::ListButton),
            };
        }
        if s.car.is_some() {
            return Some(FeAction::CarDone);
        }
        if s.results.is_some() {
            return Some(FeAction::ResultsNext);
        }
        if s.stats.is_some() {
            return Some(FeAction::Back);
        }
        Some(s.menu.options.get(s.menu.selected)?.action)
    }

    /// A tap at screen pixel (x, y): arrows scroll, an icon is selected (or, when it is
    /// already selected, activated).
    pub fn tap(&mut self, x: f32, y: f32, size: (u32, u32)) -> Option<FeAction> {
        let (vx, vy) = to_virtual(x, y, size);
        if let Some(d) = &mut self.dialog {
            let hit = d.buttons.iter().position(|&(g, _, _)| {
                let v = d.scene.values(g);
                // The buttons' boxes are 214 × 42.
                (vx - 320.0 - v[7]).abs() <= 107.0 && (vy - 240.0 - v[8]).abs() <= 21.0
            });
            let a = hit.map(|k| d.buttons[k].2);
            if a.is_some() {
                self.dialog = None;
            }
            return a.filter(|a| *a != FeAction::CloseDialog);
        }
        if let Some(bit) = self.help.as_ref().and_then(|hb| hb.hit(vx, vy)) {
            return match bit {
                0x1 => Some(FeAction::Quit),
                0x4 => self.activate(),
                0x400 => Some(FeAction::Back),
                0x1000 => Some(FeAction::CustomizeCar),
                _ => None,
            };
        }
        if self.screen.as_ref()?.list.is_some() {
            let s = self.screen.as_mut()?;
            let l = s.list.as_mut()?;
            return match l.tap(vx, vy, &mut s.scene) {
                Tap::Accept => match s.list_kind {
                    Some(ListKind::QuickRace { .. }) => Some(FeAction::OptionsDone),
                    _ => l.selected_key().map(FeAction::ListButton),
                },
                Tap::Changed => {
                    if matches!(s.list_kind, Some(ListKind::QuickRace { .. })) {
                        qr_constrain(l);
                    }
                    None
                }
                Tap::Nothing => None,
            };
        }
        if self.screen.as_ref()?.stats.is_some() {
            let s = self.screen.as_ref()?;
            let near = |i: usize| {
                let d = s.scene.values(i);
                (vx - d[7] - 320.0).abs() < 40.0 && (vy - d[8] - 240.0).abs() < 40.0
            };
            if s.menu.arrows[0].is_some_and(near) {
                self.step(-1);
            } else if s.menu.arrows[1].is_some_and(near) {
                self.step(1);
            }
            return None;
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

    /// Starts the race HUD.
    pub fn open_race_hud(&mut self, kind: crate::fe_hud::HudKind) -> bool {
        let Some(data) = &self.data else { return false };
        let Some(pkg) = data.packages.iter().find(|p| p.name.eq_ignore_ascii_case(kind.package())) else { return false };
        self.race_hud = Some(crate::fe_hud::RaceHud::new(Scene::new(pkg.clone()), kind));
        true
    }

    pub fn update_race_hud(&mut self, input: &crate::fe_hud::HudInput, size: (u32, u32), dt: f32) {
        let (Some(hud), Some(data)) = (&mut self.race_hud, &self.data) else { return };
        let (scale, ox) = mapping(size);
        hud.wide = (ox / scale).max(0.0);
        hud.update(input, &data.strings, &data.textures, dt);
    }

    /// The race HUD's quads (drawn under the touch controls).
    pub fn draw_race_hud(&self, size: (u32, u32), verts: &mut Vec<FeVertex>, batches: &mut Vec<FeBatch>) {
        let (Some(hud), Some(data)) = (&self.race_hud, &self.data) else { return };
        let mut quads = hud.scene.layout(&data.strings, &data.fonts);
        // The map shows as a disc (the game masks it round).
        let disc = hud.map_object;
        let (maps, rest): (Vec<_>, Vec<_>) = quads.drain(..).partition(|q| Some(q.object) == disc);
        for q in &maps {
            push_disc(q, size, verts, batches);
        }
        push_quads(&rest, size, verts, batches);
    }

    /// The screen's quads in screen pixels, batched by texture.
    pub fn draw(&self, size: (u32, u32), verts: &mut Vec<FeVertex>, batches: &mut Vec<FeBatch>) {
        let (Some(s), Some(data)) = (&self.screen, &self.data) else { return };
        let mut quads = s.scene.layout(&data.strings, &data.fonts);
        if let Some(hb) = &self.help {
            quads.extend(hb.scene.layout(&data.strings, &data.fonts));
        }
        if let Some(d) = &self.dialog {
            quads.extend(d.scene.layout(&data.strings, &data.fonts));
        }
        push_quads(&quads, size, verts, batches);
    }
}

/// A quad drawn as the disc inscribed in it (32 segments).
fn push_disc(q: &ug2_formats::feng_scene::Quad, size: (u32, u32), verts: &mut Vec<FeVertex>, batches: &mut Vec<FeBatch>) {
    let (scale, ox) = mapping(size);
    let texture = match &q.source {
        QuadSource::Texture(n) | QuadSource::Font(n) => bin_hash(n),
        QuadSource::TextureHash(h) => *h,
        QuadSource::Solid => 0,
    };
    let lerp = |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let at = |s: f32, t: f32| {
        let p = lerp(lerp(q.corners[0], q.corners[1], s), lerp(q.corners[3], q.corners[2], s), t);
        let uv = lerp(lerp(q.uv[0], q.uv[1], s), lerp(q.uv[3], q.uv[2], s), t);
        let c = q.colors[0];
        FeVertex { pos: [ox + p[0] * scale, p[1] * scale], uv, color: [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0] }
    };
    let first = verts.len() as u32;
    const N: usize = 32;
    for k in 0..N {
        let (a0, a1) = (k as f32 / N as f32 * std::f32::consts::TAU, (k + 1) as f32 / N as f32 * std::f32::consts::TAU);
        verts.push(at(0.5, 0.5));
        verts.push(at(0.5 + 0.5 * a0.cos(), 0.5 + 0.5 * a0.sin()));
        verts.push(at(0.5 + 0.5 * a1.cos(), 0.5 + 0.5 * a1.sin()));
    }
    batches.push(FeBatch { texture, first, count: (N * 3) as u32 });
}

fn push_quads(quads: &[ug2_formats::feng_scene::Quad], size: (u32, u32), verts: &mut Vec<FeVertex>, batches: &mut Vec<FeBatch>) {
    {
        let (scale, ox) = mapping(size);
        for q in quads {
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
    /// Car select (speed2.exe 0x511e60 / 0x4b2310): the logo is
    /// CARSELECT_MANUFACTURER_<make>, the name badge SECONDARY_LOGO_<car type>
    /// (GENERICPLACEHOLDER when missing); the end arrows hide at the ends of the list.
    fn apply_car(&mut self, data: &FrontendData) {
        let (strings, textures) = (&data.strings, &data.textures);
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
        // Stat bars (speed2.exe 0x497cd0, sliders 0x497bf0 / 0x540e90): the track is the
        // backing less a 46-unit margin each side; the bar is value × track × 0.95 and the
        // handle sits 7 past its end. The game's axis runs the other way; on screen the
        // bars grow to the right.
        let rating = data.ratings.get(name).copied().unwrap_or([0.03; 3]);
        for (k, part) in ["accel", "top_speed", "handling"].into_iter().enumerate() {
            let find = |s: &str| self.scene.index_of(h(&format!("{part}_{s}")));
            let (Some(back), Some(cur), Some(handle)) = (find("backing"), find("cur"), find("handle")) else { continue };
            let b = self.scene.package.objects[back].data;
            let track = b[14] - 92.0;
            let start = b[7] - track * 0.5;
            let w = rating[k] * track * 0.95;
            let hx = start + w + 7.0;
            let cd = self.scene.package.objects[cur].data;
            let hd = self.scene.package.objects[handle].data;
            let st = &mut self.scene.states;
            st[handle].position = Some([hx, hd[8]]);
            st[cur].position = Some([start + w * 0.5, cd[8]]);
            st[cur].size = Some([w, cd[15]]);
        }
        let n = c.cars.len();
        for (k, hidden) in [(0, c.selected == 0), (1, c.selected + 1 >= n)] {
            if let Some(a) = self.menu.arrows[k] {
                self.scene.states[a].hidden = Some(hidden);
            }
        }
    }

    fn apply_results(&mut self, strings: &ug2_formats::lang::Strings) {
        let Some(r) = &self.results else { return };
        let label = |hash: u32| strings.by_hash(hash).unwrap_or("").to_string();
        let behind = !r.drift && r.time % 6.25 >= 4.5;
        let header = if r.drift {
            0xc03f_9f19
        } else if behind {
            0xe2f9_334d
        } else {
            0xc97a_2d22
        };
        let leader = r.rows.first().and_then(|x| x.time);
        let mut set = |n: String, t: String| {
            if let Some(i) = self.scene.index_of(h(&n)) {
                self.scene.states[i].text = Some(t);
            }
        };
        set("Header_text_3".into(), label(header));
        for (k, row) in r.rows.iter().enumerate() {
            let n = k + 1;
            set(format!("Ranking_Data_{n}"), format!("{:2}", n));
            set(format!("Player_Data_{n}"), row.name.clone());
            let car = strings.by_hash(bin_hash(&format!("CAR_NAME_{}", row.car))).map_or(row.car.clone(), str::to_string);
            set(format!("Car_Data_{n}"), car);
            let time = match (row.score, row.time) {
                (Some(s), _) if r.drift => s.to_string(),
                _ if row.knocked => label(0x5d82_dba2),
                _ if row.retired => label(0x01e6_6364),
                (_, None) => label(0x61c9_b7a2),
                (_, Some(t)) if behind => match leader {
                    Some(l) if k > 0 => race_time(t - l, true),
                    _ => label(0x61c9_b7a2),
                },
                (_, Some(t)) => race_time(t, false),
            };
            set(format!("Time_Data_{n}"), time);
        }
    }

    /// Places each credits line copy at its point in ROLLCREDIT.
    fn apply_credits(&mut self, lines: &[String]) {
        let Some(c) = &self.credits else { return };
        let roll = h("RollCredit");
        for (s, &i) in c.slots.iter().enumerate() {
            // The latest line k of this slot that has started.
            let started = (c.time / CREDIT_STEP).floor() as i64;
            let mut k = started - (started - s as i64).rem_euclid(CREDIT_SLOTS as i64);
            let t = c.time - k as f32 * CREDIT_STEP;
            if k < 0 || t > CREDIT_LEN || k as usize >= lines.len() {
                k = -1;
            }
            let st = &mut self.scene.states[i];
            if k < 0 {
                st.hidden = Some(true);
                continue;
            }
            st.hidden = Some(false);
            st.text = Some(lines[k as usize].clone());
            st.script = roll;
            st.time = t * 1000.0;
            st.running = true;
        }
    }

    fn apply_stats(&mut self, strings: &ug2_formats::lang::Strings) {
        let Some(st) = &self.stats else { return };
        let Some(r) = st.rows.get(st.racer) else { return };
        let label = |hash: u32| strings.by_hash(hash).unwrap_or("").to_string();
        let car = strings.by_hash(bin_hash(&format!("CAR_NAME_{}", r.car))).map_or(r.car.clone(), str::to_string);
        if let Some(i) = self.scene.index_of(h("RACER_DATA")) {
            self.scene.states[i].text = Some(format!("{:2}   {} - {}", st.racer + 1, r.name, car));
        }
        // "%$3.1f %s": m/s × 2.237 mph or × 3.6 km/h.
        let speed = |v: f32| if st.mph { format!("{:.1} {}", v * 2.237, label(0xe74b_86f8)) } else { format!("{:.1} {}", v * 3.6, label(0xe74b_7e13)) };
        let leader = st.rows.first().and_then(|x| x.time);
        let mut stats: Vec<(String, String)> = vec![
            (label(0xf9d7_f4c2), r.time.map_or_else(|| label(0x61c9_b7a2), |t| race_time(t, false))),
            (
                label(0xab44_ed8b),
                if r.knocked {
                    label(0x5d82_dba2)
                } else if r.retired {
                    label(0x01e6_6364)
                } else if st.racer == 0 {
                    label(0x61c9_b7a2)
                } else {
                    match (r.time, leader) {
                        (Some(t), Some(l)) => race_time(t - l, true),
                        _ => label(0xd429_3449),
                    }
                },
            ),
            (label(0x5d30_4669), speed(r.top_speed)),
            (label(0x5f38_e63e), speed(r.avg_speed)),
        ];
        if st.circuit {
            stats.push((label(0xae28_0d3d), r.best_lap.map_or_else(|| label(0x61c9_b7a2), |t| race_time(t, false))));
            stats.push((label(0xadbf_d917), format!("{}/{}", r.laps_led, r.laps)));
        }
        let [tl, tt, _, th] = st.title;
        let [dl, _, dw, _] = st.data;
        for (k, &(n, d)) in st.stat_objs.iter().enumerate() {
            let row = stats.get(k);
            let y = tt + k as f32 * th;
            if let Some(i) = n {
                let s = &mut self.scene.states[i];
                s.hidden = Some(row.is_none());
                s.position = Some([tl, y]);
                s.text = row.map(|x| x.0.clone());
            }
            if let Some(i) = d {
                let s = &mut self.scene.states[i];
                s.hidden = Some(row.is_none());
                s.position = Some([dl + dw, y]);
                s.text = row.map(|x| x.1.clone());
            }
        }
        // Lap list (circuits): "%d   %s" and the place, 10 a page.
        for (k, &(t, p)) in st.lap_objs.iter().enumerate() {
            let lap = st.circuit.then(|| r.lap_times.get(k)).flatten();
            if let Some(i) = t {
                let s = &mut self.scene.states[i];
                s.hidden = Some(lap.is_none());
                s.position = Some([st.lap_time[0], st.lap_time[1] + k as f32 * st.lap_time[3]]);
                s.text = lap.map(|x| format!("{}   {}", k + 1, race_time(*x, false)));
            }
            if let Some(i) = p {
                let s = &mut self.scene.states[i];
                s.hidden = Some(lap.is_none());
                s.position = Some([st.lap_pos[0] + st.lap_pos[2] * 0.5, st.lap_pos[1] + k as f32 * st.lap_pos[3]]);
                s.text = r.lap_places.get(k).map(|x| x.to_string());
            }
        }
        let n = st.rows.len();
        for (k, hidden) in [(0, st.racer == 0), (1, st.racer + 1 >= n)] {
            if let Some(a) = self.menu.arrows[k] {
                self.scene.states[a].hidden = Some(hidden);
            }
        }
    }

    /// Writes the controller state into the scene, as the game code does every frame.
    fn apply(&mut self, strings: &ug2_formats::lang::Strings) {
        self.apply_results(strings);
        self.apply_stats(strings);
        if let Some(l) = &mut self.list {
            l.apply(&mut self.scene, strings);
        }
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

/// Lap KO forces 3 laps and 3 opponents and greys both rows out.
fn qr_constrain(l: &mut OptionList) {
    let ko = l.get("lap_ko").is_some_and(|v| v != 0.0);
    if ko {
        l.set("laps", 3.0);
        l.set("opponents", 3.0);
    }
    l.set_enabled("laps", !ko);
    l.set_enabled("opponents", !ko);
}

impl HelpBar {
    /// Shows the buttons of `flags` in slot order; the hover glows, the EA Messenger
    /// status and the cash stay hidden.
    fn set(&mut self, flags: u32, strings: &ug2_formats::lang::Strings) {
        self.flags = flags;
        self.buttons.clear();
        let find = |s: &Scene, n: &str| s.index_of(h(n));
        let slots: Vec<(Option<usize>, Option<usize>)> = HELP_SLOTS.iter().map(|(i, t)| (find(&self.scene, i), find(&self.scene, t))).collect();
        let mut next = 0;
        for &(bit, label) in &HELP_LABELS {
            if flags & bit == 0 {
                continue;
            }
            if let Some(&(Some(icon), Some(text))) = slots.get(next) {
                self.scene.states[text].text = strings.by_hash(label).map(str::to_string);
                self.buttons.push((icon, text, bit));
            }
            next += 1;
        }
        for (k, &(icon, text)) in slots.iter().enumerate() {
            for i in [icon, text].into_iter().flatten() {
                self.scene.states[i].hidden = Some(k >= next);
            }
        }
        let mut hide: Vec<usize> = (1..=5).filter_map(|n| find(&self.scene, &format!("UI_PC_Generic_Button_Glow_{n}"))).collect();
        hide.extend(self.scene.index_of(0xe0ae_8220));
        hide.extend(["_EVENT_HANDLER_", "Player_Cash", "PLAYER_NAME"].into_iter().filter_map(|n| find(&self.scene, n)));
        for n in 1..=4 {
            hide.extend(find(&self.scene, &format!("PC_EAM_Status_{n}")));
        }
        for i in hide {
            self.scene.states[i].hidden = Some(true);
        }
    }

    /// The button (flag bit) at virtual (vx, vy).
    fn hit(&self, vx: f32, vy: f32) -> Option<u32> {
        self.buttons.iter().find_map(|&(icon, _, bit)| {
            let d = self.scene.values(icon);
            let inside = (vx - 320.0 - d[7]).abs() <= d[14] * 0.5 && (vy - 240.0 - d[8]).abs() <= d[15] * 0.5 + 2.0;
            inside.then_some(bit)
        })
    }
}

impl Dialog {
    /// The highlighted button and its text play Highlight, the others UNHIGHLIGHT (the
    /// Button_Highlight / Button_UnHighlight responses).
    fn highlight(&mut self, on: bool) {
        let Some(&(g, t, _)) = self.buttons.get(self.selected) else { return };
        let script = h(if on { "Highlight" } else { "UNHIGHLIGHT" });
        self.scene.set_script(g, script);
        self.scene.set_script(t, script);
    }
}
