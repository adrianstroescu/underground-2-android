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
}

#[derive(Default)]
pub struct Frontend {
    pub data: Option<Box<FrontendData>>,
    pub screen: Option<FeScreen>,
    /// The PC help bar (UI_PC_Help_Bar, in GLOBALB) under every screen.
    help: Option<HelpBar>,
    /// A dialog over the screen (GenericDialog_*, speed2.exe 0x558790).
    dialog: Option<Dialog>,
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
        FeKind::StartCareer => 0x405,
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
        d.highlight(true);
        self.dialog = Some(d);
        true
    }

    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
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
        let mut s = FeScreen { scene, kind, menu, list: Some(list), list_kind: Some(list_kind), car: None };
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
        let mut s = FeScreen { scene, kind: FeKind::QuickRaceCar, menu, list: None, list_kind: None, car: Some(car) };
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
        let mut s = FeScreen { scene, kind, menu, list: None, list_kind: None, car: None };
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

    /// Writes the controller state into the scene, as the game code does every frame.
    fn apply(&mut self, strings: &ug2_formats::lang::Strings) {
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
    /// The highlighted button plays Highlight; its text turns the highlight green.
    fn highlight(&mut self, on: bool) {
        let Some(&(g, t, _)) = self.buttons.get(self.selected) else { return };
        self.scene.set_script(g, h(if on { "Highlight" } else { "UNHIGHLIGHT" }));
        self.scene.states[t].tint = None;
        let _ = t;
    }
}
