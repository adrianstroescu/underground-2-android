//! Menus and screen flow: title, quick race setup, pause and race results.

use crate::career::{ai_perf, race_key, Career, UPGRADES};
use crate::cars::{car_name, tier};

use crate::cars::display_name;
use crate::drift::{format_points, rival_scores};
use crate::race::DriftEvent;
use crate::config::SteerMode;
use crate::hud::ordinal;
use crate::menu::{MenuHit, MenuItem};
use crate::race::format_time;
use crate::roads::{Mode, RaceInfo};
use crate::{Game, PAINTS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Title,
    Options,
    Career,
    Garage,
    QuickRace,
    Playing,
    Results,
}

/// Quick race choices, kept between races.
pub struct QuickRace {
    pub mode: Mode,
    /// Index among the races of `mode`.
    pub race: usize,
    /// Selected step on the career ladder, and car on offer in the garage.
    pub career_step: usize,
    pub shop_car: usize,
    pub laps: u32,
    pub opponents: usize,
    /// The last race started, for "race again".
    pub last: Option<PendingRace>,
}

impl Default for QuickRace {
    fn default() -> Self {
        Self { mode: Mode::Circuit, race: 0, career_step: 0, shop_car: 0, laps: 2, opponents: 3, last: None }
    }
}

/// A race waiting for its cars to load.
#[derive(Clone, Debug)]
pub struct PendingRace {
    pub race: RaceInfo,
    pub laps: u32,
    pub opponents: Vec<(String, [f32; 3])>,
    /// Career race: (race key, prize, progress 0..1 for opponent strength).
    pub career: Option<(String, u32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Options,
    Reflections,
    Bloom,
    Msaa,
    ShowFps,
    Career,
    CareerRace,
    StartCareerRace,
    Garage,
    OwnedCar,
    Kit,
    Widebody,
    Hood,
    Lights,
    Rims,
    Spoiler,
    Neon,
    Tint,
    BuyCar,
    BuyNow,
    Upgrade(usize),
    BackToCareer,
    FreeRoam,
    QuickRace,
    RaceMode,
    Car,
    Color,
    Steering,
    Traffic,
    Track,
    Laps,
    Opponents,
    StartRace,
    Back,
    Resume,
    Restart,
    ResetCar,
    QuitToMenu,
    Standing(usize),
    RaceAgain,
    Continue,
}

impl Game {
    pub(crate) fn menu_visible(&self) -> bool {
        // Hidden while another region loads (the loading screen covers everything).
        self.spawned && !self.switching && (self.screen != Screen::Playing || self.paused)
    }

    fn rows(&self) -> Vec<Row> {
        match self.screen {
            Screen::Title => vec![Row::Career, Row::QuickRace, Row::FreeRoam, Row::Car, Row::Garage, Row::Options],
            Screen::Options => vec![
                Row::Reflections,
                Row::Bloom,
                Row::Msaa,
                Row::ShowFps,
                Row::Traffic,
                Row::Steering,
                Row::Back,
            ],
            Screen::Career => vec![Row::CareerRace, Row::StartCareerRace, Row::Garage, Row::Back],
            Screen::Garage => {
                let mut v = Vec::new();
                if self.in_career {
                    v.extend([Row::OwnedCar, Row::BuyCar, Row::BuyNow]);
                    v.extend((0..4).map(Row::Upgrade));
                } else {
                    v.push(Row::Car);
                }
                v.extend([Row::Kit, Row::Widebody, Row::Hood, Row::Lights, Row::Rims, Row::Spoiler, Row::Neon, Row::Tint, Row::Color]);
                v.push(if self.in_career { Row::BackToCareer } else { Row::Back });
                v
            }
            Screen::QuickRace => {
                let mut v = vec![Row::RaceMode, Row::Track];
                if self.quick.mode.laps() {
                    v.push(Row::Laps);
                }
                // Drift rivals are scores, not cars on the track.
                if self.quick.mode != Mode::Drift {
                    v.push(Row::Opponents);
                }
                v.extend([Row::Car, Row::StartRace, Row::Back]);
                v
            }
            Screen::Playing => {
                let mut v = vec![Row::Resume];
                if self.session.race().is_some() {
                    v.push(Row::Restart);
                } else {
                    v.push(Row::Car);
                }
                v.extend([Row::Color, Row::Steering, Row::Traffic, Row::ResetCar, Row::QuitToMenu]);
                v
            }
            Screen::Results => {
                let n = self.session.race().map_or(0, |r| r.entrants());
                let mut v: Vec<Row> = (0..n).map(Row::Standing).collect();
                v.extend([Row::RaceAgain, Row::Continue]);
                v
            }
        }
    }

    fn selected_race(&self) -> Option<&RaceInfo> {
        self.catalog.iter().filter(|r| r.mode == self.quick.mode).nth(self.quick.race)
    }

    /// Race modes with at least one race installed.
    fn modes(&self) -> Vec<Mode> {
        Mode::ALL.into_iter().filter(|m| self.catalog.iter().any(|r| r.mode == *m)).collect()
    }

    fn car_label(&self) -> String {
        self.pending_car.as_ref().or(self.player_car.as_ref()).map(|n| display_name(car_name(n)).to_string()).unwrap_or_default()
    }

    /// Name of the car the player drives (or is about to).
    fn current_car(&self) -> String {
        self.pending_car.as_ref().or(self.player_car.as_ref()).map(|k| car_name(k).to_string()).unwrap_or_default()
    }

    /// Changes one visual part of the current car and reloads it.
    fn change_part(&mut self, f: impl FnOnce(&mut ug2_formats::car::PartSet)) {
        let car = self.current_car();
        if car.is_empty() {
            return;
        }
        let mut p = self.cfg.parts_for(&car);
        f(&mut p);
        self.cfg.parts.insert(car.clone(), p);
        self.cfg.save(&self.config_path);
        self.select_car(car);
    }

    pub(crate) fn build_menu(&mut self) {
        let rows = self.rows();
        let paint = PAINTS.iter().find(|(_, c)| *c == self.cfg.paint).map_or("CUSTOM", |(n, _)| n);
        let steer = match self.cfg.steer_mode {
            SteerMode::Buttons => "BUTTONS",
            SteerMode::Joystick => "JOYSTICK",
        };
        let on_off = |b: bool| if b { "ON" } else { "OFF" };
        let ladder = Career::ladder(&self.catalog);
        let shop = self.shop_cars();
        let items = rows
            .iter()
            .map(|row| match *row {
                Row::Career => MenuItem::button(if self.catalog.is_empty() { "CAREER (LOADING)" } else { "CAREER" }),
                Row::CareerRace => {
                    let text = match (self.career.as_ref(), ladder.get(self.quick.career_step)) {
                        (Some(c), Some(&i)) => {
                            let r = &self.catalog[i];
                            let kind = r.mode.label();
                            let state = if c.won.contains(&race_key(r)) {
                                "WON"
                            } else if c.unlocked(&self.catalog, &ladder, self.quick.career_step) {
                                "OPEN"
                            } else {
                                "LOCKED"
                            };
                            format!("{}. {} {} ${} {state}", self.quick.career_step + 1, kind, r.name, r.reward)
                        }
                        _ => "NO RACES".to_string(),
                    };
                    MenuItem::choice("RACE", text)
                }
                Row::StartCareerRace => MenuItem::button("START RACE"),
                Row::Garage => MenuItem::button("GARAGE"),
                Row::OwnedCar => MenuItem::choice("MY CAR", self.car_label()),
                Row::Rims => {
                    let p = self.cfg.parts_for(&self.current_car());
                    let value = match p.rim {
                        0 => "STOCK".to_string(),
                        i => self.rim_names.get(i as usize - 1).cloned().unwrap_or_else(|| format!("RIM {i}")),
                    };
                    MenuItem::choice("RIMS", value)
                }
                Row::Neon => {
                    let p = self.cfg.parts_for(&self.current_car());
                    let v = crate::NEONS.get((p.neon as usize).wrapping_sub(1)).map_or("NONE", |n| n.0);
                    MenuItem::choice("NEON", v)
                }
                Row::Tint => {
                    let p = self.cfg.parts_for(&self.current_car());
                    let v = crate::TINTS.get((p.tint as usize).wrapping_sub(1)).map_or("STOCK", |n| n.0);
                    MenuItem::choice("WINDOW TINT", v)
                }
                Row::Spoiler => {
                    let p = self.cfg.parts_for(&self.current_car());
                    let value = match (p.spoiler, p.spoiler_cf) {
                        (0, _) => "STOCK".to_string(),
                        (s, false) => format!("STYLE {s:02}"),
                        (s, true) => format!("STYLE {s:02} CARBON"),
                    };
                    MenuItem::choice("SPOILER", value)
                }
                Row::Kit | Row::Widebody | Row::Hood | Row::Lights => {
                    let p = self.cfg.parts_for(&self.current_car());
                    let (label, v, name) = match *row {
                        Row::Kit => ("BODY KIT", p.kit, "KIT"),
                        Row::Widebody => ("WIDEBODY", p.widebody, "WIDE"),
                        Row::Hood => ("HOOD", p.hood, "STYLE"),
                        _ => ("LIGHTS", p.lights, "STYLE"),
                    };
                    let value = if v == 0 { "STOCK".to_string() } else { format!("{name} {v:02}") };
                    MenuItem::choice(label, value)
                }
                Row::BuyCar => {
                    let text = match shop.get(self.quick.shop_car) {
                        Some(car) => format!("{} ${}", display_name(car), Career::car_price(car)),
                        None => "ALL CARS OWNED".to_string(),
                    };
                    MenuItem::choice("BUY", text)
                }
                Row::BuyNow => MenuItem::button("BUY THIS CAR"),
                Row::Upgrade(k) => {
                    let car = self.current_car();
                    let text = match &self.career {
                        Some(c) => {
                            let lv = c.levels(&car)[k];
                            match c.upgrade_price(&car, k) {
                                Some(p) => format!("{}  LV {lv}/3  BUY ${p}", UPGRADES[k]),
                                None => format!("{}  LV 3/3  MAX", UPGRADES[k]),
                            }
                        }
                        None => UPGRADES[k].to_string(),
                    };
                    MenuItem::button(text)
                }
                Row::BackToCareer => MenuItem::button("BACK"),
                Row::FreeRoam => MenuItem::button("FREE ROAM"),
                Row::Options => MenuItem::button("OPTIONS"),
                Row::Reflections => {
                    let r = self.cfg.reflections;
                    MenuItem::choice("ROAD REFLECTIONS", if r <= 0.01 { "OFF" } else if r < 0.45 { "LOW" } else { "HIGH" })
                }
                Row::Bloom => MenuItem::choice("BLOOM", on_off(self.cfg.bloom)),
                Row::Msaa => MenuItem::choice("ANTI-ALIASING", if self.cfg.msaa > 1 { "4X (RESTART)" } else { "OFF (RESTART)" }),
                Row::ShowFps => MenuItem::choice("SHOW FPS", on_off(self.cfg.show_fps)),
                Row::QuickRace => {
                    MenuItem::button(if self.catalog.is_empty() { "QUICK RACE (LOADING)" } else { "QUICK RACE" })
                }
                Row::RaceMode => MenuItem::choice("MODE", self.quick.mode.label()),
                Row::Car => MenuItem::choice("CAR", self.car_label()),
                Row::Color => MenuItem::choice("COLOR", paint),
                Row::Steering => MenuItem::choice("STEERING", steer),
                Row::Traffic => MenuItem::choice("TRAFFIC", on_off(self.session.traffic_on)),
                Row::Track => {
                    let name = self.selected_race().map_or("NO RACES".to_string(), |r| {
                        let rev = if r.reverse { " REV" } else { "" };
                        format!("{}{rev}", r.name)
                    });
                    MenuItem::choice("TRACK", name)
                }
                Row::Laps => MenuItem::choice("LAPS", self.quick.laps.to_string()),
                Row::Opponents => MenuItem::choice("OPPONENTS", self.quick.opponents.to_string()),
                Row::StartRace => MenuItem::button("START RACE"),
                Row::Back => MenuItem::button("BACK"),
                Row::Resume => MenuItem::button("RESUME"),
                Row::Restart => MenuItem::button("RESTART RACE"),
                Row::ResetCar => MenuItem::button("RESET CAR"),
                Row::QuitToMenu => MenuItem::button("QUIT TO MENU"),
                Row::Standing(i) => {
                    let text = self.session.race().map_or(String::new(), |r| {
                        if r.drift.is_some() {
                            let rows = r.drift_standings();
                            return rows.get(i).map_or(String::new(), |(name, score, _)| {
                                format!("{}  {name}  {}", i + 1, format_points(*score))
                            });
                        }
                        let car = r.standings()[i];
                        let name = if car == 0 {
                            "YOU".to_string()
                        } else {
                            display_name(&self.session.opponents[car - 1].model).to_string()
                        };
                        let time = match r.cars[car].finish_time {
                            Some(t) => format_time(t),
                            None if r.cars[car].retired => "DNF".to_string(),
                            None => "--:--.--".to_string(),
                        };
                        format!("{}  {name}  {time}", i + 1)
                    });
                    MenuItem::button(text)
                }
                Row::RaceAgain => MenuItem::button("RACE AGAIN"),
                Row::Continue => MenuItem::button("CONTINUE"),
            })
            .collect();
        let cash = self.career.as_ref().map_or(0, |c| c.cash);
        let (title, subtitle) = match self.screen {
            Screen::Title => ("UNDERGROUND 2".to_string(), "BAYVIEW".to_string()),
            Screen::Options => ("OPTIONS".to_string(), "GRAPHICS AND CONTROLS".to_string()),
            Screen::Career => {
                let won = self.career.as_ref().map_or(0, |c| c.won.len());
                ("CAREER".to_string(), format!("CASH ${cash}  |  RACES WON {won}/{}", ladder.len()))
            }
            Screen::Garage if self.in_career => ("GARAGE".to_string(), format!("CASH ${cash}")),
            Screen::Garage => ("GARAGE".to_string(), self.car_label()),
            Screen::QuickRace => {
                let sub = self.selected_race().map_or(String::new(), |r| {
                    format!("{}  |  {:.1} KM  |  REWARD ${}", r.mode.label(), r.length as f32 / 1000.0, r.reward)
                });
                ("QUICK RACE".to_string(), sub)
            }
            Screen::Playing => ("PAUSED".to_string(), String::new()),
            Screen::Results => {
                let place = self.session.race().map_or(1, |r| r.place(0));
                let best = self.session.race().and_then(|r| r.cars[0].best_lap).map(format_time);
                let score = self.session.race().and_then(|r| r.drift.as_ref()).map(|d| format_points(d.score.total));
                let retired = self.session.race().is_some_and(|r| r.cars[0].retired);
                let mut sub = match (score, best) {
                    _ if retired => "ENGINE BLOWN  |  DID NOT FINISH".to_string(),
                    (Some(s), _) => format!("YOU FINISHED {}  |  SCORE {s}", ordinal(place)),
                    (None, Some(b)) => format!("YOU FINISHED {}  |  BEST LAP {b}", ordinal(place)),
                    (None, None) => format!("YOU FINISHED {}", ordinal(place)),
                };
                if let Some(p) = self.prize_paid {
                    sub.push_str(&format!("  |  +${p}"));
                }
                ("RACE RESULTS".to_string(), sub)
            }
        };
        self.menu.title = title;
        self.menu.subtitle = subtitle;
        self.menu.anchor_left = self.screen == Screen::Garage;
        self.menu.items = items;
        if self.menu.selected >= rows.len() {
            self.menu.selected = 0;
        }
    }

    pub(crate) fn menu_action(&mut self, hit: MenuHit) {
        let (MenuHit::Activate(i) | MenuHit::Prev(i) | MenuHit::Next(i)) = hit;
        let step: i32 = if matches!(hit, MenuHit::Prev(_)) { -1 } else { 1 };
        let Some(&row) = self.rows().get(i) else { return };
        let cycle = |at: usize, len: usize| (at as i32 + step).rem_euclid(len.max(1) as i32) as usize;
        match row {
            Row::Options => {
                self.screen = Screen::Options;
                self.menu.selected = 0;
            }
            Row::Reflections => {
                let levels = [0.0, 0.35, 0.55];
                let at = levels.iter().position(|l| (l - self.cfg.reflections).abs() < 0.1).unwrap_or(2);
                self.cfg.reflections = levels[cycle(at, levels.len())];
                self.cfg.save(&self.config_path);
            }
            Row::Bloom => {
                self.cfg.bloom = !self.cfg.bloom;
                self.cfg.save(&self.config_path);
            }
            Row::Msaa => {
                self.cfg.msaa = if self.cfg.msaa > 1 { 1 } else { 4 };
                self.cfg.save(&self.config_path);
            }
            Row::ShowFps => {
                self.cfg.show_fps = !self.cfg.show_fps;
                self.cfg.save(&self.config_path);
            }
            Row::Career if !self.catalog.is_empty() => {
                self.in_career = true;
                let starter = self.player_car.clone().unwrap_or_else(|| "PEUGOT".into());
                if self.career.is_none() {
                    self.career = Some(Career::load(&self.career_path, &starter));
                }
                let car = self.career.as_ref().map(|c| c.current.clone()).unwrap_or_default();
                self.select_car(car);
                self.screen = Screen::Career;
                self.menu.selected = 0;
            }
            Row::CareerRace => {
                let n = self.catalog.len();
                self.quick.career_step = cycle(self.quick.career_step, n);
            }
            Row::StartCareerRace => self.prepare_career_race(),
            Row::Garage => {
                self.screen = Screen::Garage;
                self.menu.selected = 0;
            }
            Row::Kit => self.change_part(|p| p.kit = (p.kit as i32 + step).rem_euclid(30) as u8),
            Row::Widebody => self.change_part(|p| p.widebody = (p.widebody as i32 + step).rem_euclid(5) as u8),
            Row::Hood => self.change_part(|p| p.hood = (p.hood as i32 + step).rem_euclid(29) as u8),
            Row::Lights => self.change_part(|p| p.lights = (p.lights as i32 + step).rem_euclid(15) as u8),
            // Neon and tint are drawn per frame: no model reload needed.
            Row::Neon | Row::Tint => {
                let car = self.current_car();
                let mut p = self.cfg.parts_for(&car);
                if row == Row::Neon {
                    p.neon = (p.neon as i32 + step).rem_euclid(crate::NEONS.len() as i32 + 1) as u8;
                } else {
                    p.tint = (p.tint as i32 + step).rem_euclid(crate::TINTS.len() as i32 + 1) as u8;
                }
                self.cfg.parts.insert(car, p);
                self.cfg.save(&self.config_path);
            }
            Row::Rims => {
                let n = self.rim_names.len() as i32 + 1;
                self.change_part(|p| p.rim = (p.rim as i32 + step).rem_euclid(n) as u16);
            }
            Row::Spoiler => self.change_part(|p| {
                // 0 stock, 1..40 painted, 41..80 carbon.
                let n = ug2_formats::parts::SPOILER_STYLES as i32;
                let cur = if p.spoiler == 0 { 0 } else { p.spoiler as i32 + if p.spoiler_cf { n } else { 0 } };
                let next = (cur + step).rem_euclid(2 * n + 1);
                p.spoiler_cf = next > n;
                p.spoiler = if next == 0 { 0 } else { ((next - 1) % n + 1) as u8 };
            }),
            Row::BackToCareer => {
                self.screen = Screen::Career;
                self.menu.selected = 0;
            }
            Row::OwnedCar => {
                let Some(owned) = self.career.as_ref().map(|c| c.owned.clone()) else { return };
                let current = self.current_car();
                let at = owned.iter().position(|o| *o == current).unwrap_or(0);
                let next = owned[cycle(at, owned.len())].clone();
                if let Some(c) = &mut self.career {
                    c.current = next.clone();
                    c.save();
                }
                self.select_car(next);
            }
            Row::BuyCar => {
                let n = self.shop_cars().len();
                self.quick.shop_car = cycle(self.quick.shop_car, n);
            }
            Row::BuyNow => self.buy_shop_car(),
            Row::Upgrade(k) => {
                let car = self.current_car();
                if let Some(c) = &mut self.career {
                    if c.buy_upgrade(&car, k) {
                        self.vehicle.perf = self.perf_for(&car);
                        self.apply_engine_sound();
                    }
                }
            }
            Row::FreeRoam => {
                // Free roam is in Bayview: come back from a race area first.
                if self.region != self.cfg.location {
                    self.pending_free_roam = true;
                    let home = self.cfg.location.clone();
                    self.load_region(&home);
                } else {
                    self.start_free_roam();
                }
            }
            Row::RaceMode => {
                let modes = self.modes();
                let at = modes.iter().position(|m| *m == self.quick.mode).unwrap_or(0);
                if let Some(&m) = modes.get(cycle(at, modes.len())) {
                    self.quick.mode = m;
                    self.quick.race = 0;
                    self.quick.laps = self.selected_race().map_or(2, |r| r.laps.max(1));
                }
            }
            Row::QuickRace if !self.catalog.is_empty() => {
                self.screen = Screen::QuickRace;
                self.menu.selected = 0;
            }
            Row::Car if !self.racers.is_empty() => {
                let current = self.current_car();
                let at = self.racers.iter().position(|r| *r == current).unwrap_or(0);
                let next = self.racers[cycle(at, self.racers.len())].clone();
                self.cfg.car = next.clone();
                self.cfg.save(&self.config_path);
                self.select_car(next);
            }
            Row::Color => {
                let at = PAINTS.iter().position(|(_, c)| *c == self.cfg.paint).unwrap_or(0);
                self.cfg.paint = PAINTS[cycle(at, PAINTS.len())].1;
                self.cfg.save(&self.config_path);
            }
            Row::Steering => {
                self.cfg.steer_mode = match self.cfg.steer_mode {
                    SteerMode::Buttons => SteerMode::Joystick,
                    SteerMode::Joystick => SteerMode::Buttons,
                };
                self.hud.steer_mode = self.cfg.steer_mode;
                self.hud.layout(self.size.0 as f32, self.size.1 as f32, self.dp);
                self.cfg.save(&self.config_path);
            }
            Row::Traffic => {
                self.session.traffic_on = !self.session.traffic_on;
                self.cfg.traffic = self.session.traffic_on;
                self.cfg.save(&self.config_path);
            }
            Row::Track => {
                let n = self.catalog.iter().filter(|r| r.mode == self.quick.mode).count();
                self.quick.race = cycle(self.quick.race, n);
                if let Some(r) = self.selected_race() {
                    self.quick.laps = r.laps.max(1);
                }
            }
            Row::Laps => self.quick.laps = (self.quick.laps as i32 + step).clamp(1, 5) as u32,
            Row::Opponents => self.quick.opponents = (self.quick.opponents as i32 + step).clamp(1, 5) as usize,
            Row::StartRace => self.prepare_race(),
            Row::Back => {
                if self.screen == Screen::Career {
                    self.in_career = false;
                }
                self.screen = Screen::Title;
                self.menu.selected = 0;
            }
            Row::Resume => self.set_paused(false),
            Row::Restart | Row::RaceAgain => {
                if let Some(p) = self.quick.last.clone() {
                    self.pending_race = Some(p);
                }
            }
            Row::ResetCar => {
                self.reset_player();
                self.set_paused(false);
            }
            Row::QuitToMenu | Row::Continue => {
                self.session.free_roam();
                self.screen = if self.race_prize.is_some() { Screen::Career } else { Screen::Title };
                self.race_prize = None;
                self.set_paused(false);
                self.menu.selected = 0;
            }
            _ => {}
        }
    }

    /// Escape / Android Back / the pause button.
    pub(crate) fn back_pressed(&mut self) {
        match self.screen {
            Screen::Playing => self.set_paused(!self.paused),
            Screen::QuickRace | Screen::Results | Screen::Career | Screen::Options => {
                if self.screen == Screen::Results {
                    self.session.free_roam();
                }
                self.screen = if self.screen == Screen::Results && self.race_prize.take().is_some() {
                    Screen::Career
                } else {
                    Screen::Title
                };
                self.menu.selected = 0;
            }
            Screen::Garage => {
                self.screen = if self.in_career { Screen::Career } else { Screen::Title };
                self.menu.selected = 0;
            }
            Screen::Title => {}
        }
    }

    /// Cars the career shop still sells.
    fn shop_cars(&self) -> Vec<String> {
        match &self.career {
            Some(c) => self.racers.iter().filter(|r| !c.owned.contains(r)).cloned().collect(),
            None => Vec::new(),
        }
    }

    /// Buys the car on offer in the garage.
    pub(crate) fn buy_shop_car(&mut self) {
        let shop = self.shop_cars();
        let Some(car) = shop.get(self.quick.shop_car).cloned() else { return };
        if self.career.as_mut().is_some_and(|c| c.buy_car(&car)) {
            self.select_car(car);
            self.quick.shop_car = 0;
        }
    }

    /// Starts the selected career race if it is unlocked.
    fn prepare_career_race(&mut self) {
        let Some(career) = &self.career else { return };
        let ladder = Career::ladder(&self.catalog);
        let step = self.quick.career_step.min(ladder.len().saturating_sub(1));
        let Some(&race) = ladder.get(step) else { return };
        if !career.unlocked(&self.catalog, &ladder, step) {
            return;
        }
        let def = self.catalog[race].clone();
        let (class, progress) = Career::opponent_strength(step, ladder.len());
        let player = self.current_car();
        let mut pool: Vec<String> = self.racers.iter().filter(|r| **r != player && tier(r) == class).cloned().collect();
        if pool.len() < 3 {
            pool = self.racers.iter().filter(|r| **r != player).cloned().collect();
        }
        let mut seed = (self.title_time * 1000.0) as u32 | 1;
        let mut opponents = Vec::new();
        for _ in 0..3 {
            if pool.is_empty() {
                break;
            }
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let car = pool.remove(seed as usize % pool.len());
            opponents.push((car, PAINTS[seed as usize % PAINTS.len()].1));
        }
        let p = PendingRace {
            laps: def.laps,
            opponents,
            career: Some((race_key(&def), def.reward, progress)),
            race: def,
        };
        self.quick.last = Some(p.clone());
        self.pending_race = Some(p);
    }

    /// Picks opponents and starts loading their cars; the race begins once they are in.
    fn prepare_race(&mut self) {
        let Some(race) = self.selected_race().cloned() else { return };
        let player = self.current_car();
        let mut pool: Vec<String> = self.racers.iter().filter(|r| **r != player).cloned().collect();
        let mut seed = (self.title_time * 1000.0) as u32 | 1;
        let mut rand = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut opponents = Vec::new();
        let count = if race.mode == Mode::Drift { 3 } else { self.quick.opponents };
        for _ in 0..count {
            if pool.is_empty() {
                break;
            }
            let car = pool.remove(rand() as usize % pool.len());
            let paint = PAINTS[rand() as usize % PAINTS.len()].1;
            opponents.push((car, paint));
        }
        let p = PendingRace { race, laps: self.quick.laps, opponents, career: None };
        self.quick.last = Some(p.clone());
        self.pending_race = Some(p);
    }

    pub(crate) fn try_start_pending_race(&mut self) {
        let Some(p) = &self.pending_race else { return };
        // The race's region comes first (the call waits for a load in progress).
        if self.region != p.race.region {
            let region = p.race.region.clone();
            self.load_region(&region);
            return;
        }
        if self.world_loading || self.roads.is_none() {
            return;
        }
        // Drift rivals only lend their names: nothing to load.
        let missing: Vec<String> = p
            .opponents
            .iter()
            .map(|o| o.0.clone())
            .filter(|n| p.race.mode != Mode::Drift && !self.models.contains_key(n))
            .collect();
        if !missing.is_empty() {
            for m in &missing {
                self.request_model(m);
            }
            return;
        }
        let drift = p.race.mode == Mode::Drift;
        let p = self.pending_race.take().unwrap();
        let Some(def) = self.roads.as_ref().and_then(|r| r.race(&p.race)).cloned() else {
            log::warn!("race {} ({}) is not in the loaded region", p.race.track, p.race.name);
            return;
        };
        let strength = p.career.as_ref().map(|c| c.2);
        let opps: Vec<(String, [f32; 3], crate::play::ModelPhysics, crate::vehicle::Perf)> = p
            .opponents
            .iter()
            .filter(|_| !drift)
            .take(if def.info.mode == Mode::Drag { 3 } else { 5 })
            .filter_map(|(n, c)| {
                let base = self.base_perf(n);
                let perf = match strength {
                    Some(f) => ai_perf(base, f),
                    None => base,
                };
                self.physics.get(n).map(|ph| (n.clone(), *c, *ph, perf))
            })
            .collect();
        self.race_prize = p.career.as_ref().map(|c| (c.0.clone(), c.1));
        self.prize_paid = None;
        let car = self.current_car();
        if !car.is_empty() {
            self.vehicle.perf = self.perf_for(&car);
        }
        log::info!("race {} ({} {:?}): {} laps, {} opponents", def.info.track, def.info.name, def.info.mode, p.laps, opps.len());
        let length = def.line.length;
        self.session.start_race(def, p.laps, &mut self.vehicle, &opps);
        // With this race's barriers switched on, steer its line clear of the walls.
        self.update_groups();
        if let (Some(w), Some(r)) = (&mut self.world, self.session.race_mut()) {
            // Every section's walls count, not just those near the grid.
            w.set_sections_on(Vec::new());
            self.vis_cars.clear();
            let t = std::time::Instant::now();
            let moved = r.def.line.avoid_walls(w);
            log::info!("race line: {moved} points moved off walls in {:.1?}", t.elapsed());
        }
        if drift {
            let scores = rival_scores(length, p.laps, strength.unwrap_or(0.4));
            let rivals = p.opponents.iter().zip(scores).map(|((n, _), s)| (display_name(n).to_string(), s)).collect();
            if let Some(r) = self.session.race_mut() {
                r.drift = Some(DriftEvent { score: Default::default(), rivals });
            }
        }
        self.camera.snap(&self.vehicle);
        self.screen = Screen::Playing;
        self.finish_timer = 0.0;
        self.set_paused(false);
    }
}
