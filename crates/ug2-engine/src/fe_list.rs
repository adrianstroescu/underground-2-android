//! The scrolling option list of the original frontend (speed2.exe 0x552780), used by
//! `UI_QRModeOptions` and `UI_Options`: one row per option with its name, value, left /
//! right arrows and backing, or a slider.
//!
//! Rows are laid out from the TITLE_MASTER and DATA_MASTER rectangles (their own data,
//! before scripts), one title height apart (0x51ff80, 0x54ef30); 7 show at a time and
//! the list wraps. The name sits at the title rectangle's top-left, the value centred in
//! the data rectangle, the arrows just outside it (0x52d800 / 0x52dac0), the backing
//! centred on the row.

use ug2_formats::bin_hash;
use ug2_formats::feng_scene::Scene;
use ug2_formats::lang::Strings;

fn h(name: &str) -> u32 {
    bin_hash(&name.to_ascii_uppercase())
}

/// A value's text: a language label (with an English fallback) or literal text.
#[derive(Clone, Debug)]
pub enum Label {
    Hash(u32, &'static str),
    Text(String),
}

impl Label {
    pub fn text(&self, strings: &Strings) -> String {
        match self {
            Label::Hash(h, fallback) => strings.by_hash(*h).unwrap_or(fallback).to_string(),
            Label::Text(t) => t.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Kind {
    /// One of the labels; the value is the index.
    Choice(Vec<Label>),
    /// Whole numbers shown as "%d".
    Number { min: i32, max: i32 },
    /// A slider (min, max, step).
    Slider { min: f32, max: f32, step: f32 },
    /// Opens something when accepted.
    Button,
}

#[derive(Clone, Debug)]
pub struct Row {
    /// Setting name, e.g. "laps" or "master_volume".
    pub key: &'static str,
    pub label: u32,
    pub kind: Kind,
    pub value: f32,
    pub enabled: bool,
}

impl Row {
    pub fn choice(key: &'static str, label: u32, values: Vec<Label>, value: f32) -> Self {
        Row { key, label, kind: Kind::Choice(values), value, enabled: true }
    }

    pub fn on_off(key: &'static str, label: u32, on: bool) -> Self {
        Self::choice(key, label, vec![Label::Hash(0xccfa, "Off"), Label::Hash(0x063c, "On")], on as u8 as f32)
    }

    pub fn number(key: &'static str, label: u32, min: i32, max: i32, value: f32) -> Self {
        Row { key, label, kind: Kind::Number { min, max }, value, enabled: true }
    }

    pub fn slider(key: &'static str, label: u32, value: f32) -> Self {
        Row { key, label, kind: Kind::Slider { min: 0.0, max: 1.0, step: 0.05 }, value, enabled: true }
    }

    pub fn button(key: &'static str, label: u32) -> Self {
        Row { key, label, kind: Kind::Button, value: 0.0, enabled: true }
    }

    fn text(&self, strings: &Strings) -> String {
        match &self.kind {
            Kind::Choice(v) => v.get(self.value as usize).map(|l| l.text(strings)).unwrap_or_default(),
            Kind::Number { .. } => (self.value as i32).to_string(),
            Kind::Slider { .. } | Kind::Button => String::new(),
        }
    }

    /// Left / right; lists and numbers wrap, sliders stop at their ends.
    fn change(&mut self, delta: i32) -> bool {
        let wrap = |v: i32, lo: i32, hi: i32| lo + (v - lo).rem_euclid(hi - lo + 1);
        let old = self.value;
        match &self.kind {
            Kind::Choice(v) if !v.is_empty() => self.value = wrap(self.value as i32 + delta, 0, v.len() as i32 - 1) as f32,
            Kind::Number { min, max } => self.value = wrap(self.value as i32 + delta, *min, *max) as f32,
            Kind::Slider { min, max, step } => {
                let n = ((self.value - min) / step).round() + delta as f32;
                self.value = (min + n * step).clamp(*min, *max);
            }
            _ => {}
        }
        self.value != old
    }
}

/// One row's objects.
#[derive(Clone, Copy, Debug, Default)]
struct RowObjects {
    name: Option<usize>,
    data: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    backing: Option<usize>,
    /// SLIDER_BASE / HANDLE (the fill) / KNOB / BEG / END.
    slider: [Option<usize>; 5],
}

/// What a tap on the list did.
pub enum Tap {
    Nothing,
    /// The highlighted row's name: accept (the PC mouse click).
    Accept,
    Changed,
}

pub struct OptionList {
    pub rows: Vec<Row>,
    objs: Vec<RowObjects>,
    pub selected: usize,
    top: usize,
    shown: Option<usize>,
    /// Left, top, width, height.
    title: [f32; 4],
    data: [f32; 4],
    /// Rows shown at a time: 7 on the quick race options (list +0x134), 8 on
    /// UI_Options (PC capture).
    visible: usize,
}

impl OptionList {
    /// Sets up the package's rows for `rows`; returns the list and the objects the game
    /// code hides (unused rows, sliders of non-slider rows).
    pub fn new(scene: &mut Scene, rows: Vec<Row>, visible: usize) -> (Self, Vec<usize>) {
        let find = |scene: &Scene, n: &str| scene.index_of(h(n));
        let rect = |scene: &Scene, i: Option<usize>, def: [f32; 4]| {
            i.map_or(def, |i| {
                let d = &scene.package.objects[i].data;
                [d[7] - d[14] * 0.5, d[8] - d[15] * 0.5, d[14], d[15]]
            })
        };
        let title = rect(scene, find(scene, "TITLE_MASTER"), [-272.0, -140.0, 240.0, 36.0]);
        let data = rect(scene, find(scene, "DATA_MASTER"), [26.0, -140.0, 200.0, 32.0]);
        let mut objs = Vec::new();
        for n in 1..=16 {
            let s = |p: &str| find(scene, &format!("SLIDER_{p}_{n}"));
            objs.push(RowObjects {
                name: find(scene, &format!("OPTION_NAME_{n}")),
                data: find(scene, &format!("OPTION_DATA_{n}")),
                left: find(scene, &format!("LEFT_ARROW_{n}")),
                right: find(scene, &format!("RIGHT_ARROW_{n}")),
                backing: find(scene, &format!("OPTION_BACKING_{n}")),
                slider: [s("BASE"), s("HANDLE"), s("KNOB"), s("BEG"), s("END")],
            });
        }
        let mut hidden = Vec::new();
        for (k, o) in objs.iter().enumerate() {
            let used = k < rows.len();
            if !used {
                hidden.extend([o.name, o.data, o.left, o.right, o.backing].into_iter().flatten());
            }
            let slider = rows.get(k).is_some_and(|r| matches!(r.kind, Kind::Slider { .. }));
            if !slider {
                hidden.extend(o.slider.into_iter().flatten());
            }
        }
        // The slider's end caps stay hidden (0x540e90).
        for o in &objs {
            hidden.extend([o.slider[3], o.slider[4]].into_iter().flatten());
        }
        hidden.extend(["_EVENT_HANDLER_", "DEMO_CAR_SELECT"].into_iter().filter_map(|n| find(scene, n)));
        objs.truncate(rows.len());
        for o in &objs {
            for i in [o.name, o.data, o.left, o.right, o.backing].into_iter().flatten() {
                scene.states[i].hidden = Some(false);
                scene.set_script(i, h("UNHIGHLIGHT"));
            }
        }
        let mut list = OptionList { rows, objs, selected: 0, top: 0, shown: None, title, data, visible };
        // The first enabled row is highlighted (0x539650).
        list.selected = list.rows.iter().position(|r| r.enabled).unwrap_or(0);
        (list, hidden)
    }

    pub fn get(&self, key: &str) -> Option<f32> {
        self.rows.iter().find(|r| r.key == key).map(|r| r.value)
    }

    pub fn set(&mut self, key: &str, value: f32) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.key == key) {
            r.value = value;
        }
    }

    pub fn set_enabled(&mut self, key: &str, enabled: bool) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.key == key) {
            r.enabled = enabled;
        }
    }

    pub fn selected_key(&self) -> Option<&'static str> {
        self.rows.get(self.selected).map(|r| r.key)
    }

    /// Up / down (wraps).
    pub fn step_row(&mut self, delta: i32) {
        let n = self.rows.len() as i32;
        if n == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).rem_euclid(n) as usize;
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + self.visible {
            self.top = self.selected + 1 - self.visible;
        }
    }

    /// Left / right on the highlighted row; true when the value changed.
    pub fn change(&mut self, delta: i32, scene: &mut Scene) -> bool {
        let Some(r) = self.rows.get_mut(self.selected) else { return false };
        if !r.enabled || !r.change(delta) {
            return false;
        }
        let o = self.objs[self.selected];
        if let Some(a) = if delta < 0 { o.left } else { o.right } {
            scene.set_script(a, h("Blink"));
        }
        true
    }

    /// A tap at virtual (vx, vy): another row is highlighted; on the highlighted row the
    /// arrows (or the value) change it and the name accepts.
    pub fn tap(&mut self, vx: f32, vy: f32, scene: &mut Scene) -> Tap {
        let (cx, cy) = (vx - 320.0, vy - 240.0);
        let row_h = self.title[3];
        let (left, data_left, data_w) = (self.title[0], self.data[0], self.data[2]);
        if cx < left - 30.0 || cx > data_left + data_w + 30.0 || cy < self.title[1] {
            return Tap::Nothing;
        }
        let slot = ((cy - self.title[1]) / row_h) as usize;
        let k = self.top + slot;
        if slot >= self.visible || k >= self.rows.len() {
            return Tap::Nothing;
        }
        if k != self.selected {
            if self.rows[k].enabled {
                self.selected = k;
            }
            return Tap::Nothing;
        }
        if cx < data_left - 30.0 || matches!(self.rows[k].kind, Kind::Button) {
            return Tap::Accept;
        }
        if let Kind::Slider { min, max, .. } = self.rows[k].kind {
            // A tap on the slider sets its value.
            let (start, track) = self.slider_track();
            let f = ((cx - start) / (track * 0.95)).clamp(0.0, 1.0);
            let r = &mut self.rows[k];
            let steps = ((f * (max - min)) / 0.05).round();
            r.value = (min + steps * 0.05).clamp(min, max);
            return Tap::Changed;
        }
        let delta = if cx < data_left + data_w * 0.35 { -1 } else { 1 };
        if self.change(delta, scene) {
            Tap::Changed
        } else {
            Tap::Nothing
        }
    }

    /// Left end and length of a slider's track: its base less a 46-unit margin each side
    /// (the stat bar sliders, 0x497bf0).
    fn slider_track(&self) -> (f32, f32) {
        let base_w = 234.9;
        let track = base_w - 92.0;
        (self.data[0] + self.data[2] * 0.5 - track * 0.5, track)
    }

    /// Writes the rows into the scene.
    pub fn apply(&mut self, scene: &mut Scene, strings: &Strings) {
        let [tl, tt, _, th] = self.title;
        let [dl, _, dw, _] = self.data;
        let row_cx = (tl + dl + dw) * 0.5;
        if self.shown != Some(self.selected) {
            for (k, o) in self.objs.iter().enumerate() {
                if self.shown == Some(k) || k == self.selected || self.shown.is_none() {
                    let script = if k == self.selected { h("Highlight") } else { h("UNHIGHLIGHT") };
                    for i in [o.name, o.data, o.left, o.right, o.backing].into_iter().flatten() {
                        scene.set_script(i, script);
                    }
                }
            }
            self.shown = Some(self.selected);
        }
        let (track_start, track) = self.slider_track();
        for (k, (r, o)) in self.rows.iter().zip(&self.objs).enumerate() {
            let visible = k >= self.top && k < self.top + self.visible;
            let y = tt + (k as f32 - self.top as f32) * th;
            let slider = matches!(r.kind, Kind::Slider { .. });
            let arrows = !matches!(r.kind, Kind::Button);
            let fill_h = o.slider[1].map_or(5.8, |i| scene.package.objects[i].data[15]);
            let st = &mut scene.states;
            let dim = (!r.enabled).then_some([255, 255, 255, 110]);
            if let Some(i) = o.name {
                st[i].hidden = Some(!visible);
                st[i].position = Some([tl, y]);
                st[i].text = strings.by_hash(r.label).map(str::to_string);
                st[i].tint = dim;
            }
            if let Some(i) = o.data {
                st[i].hidden = Some(!visible || slider);
                st[i].position = Some([dl + dw * 0.5, y]);
                st[i].text = Some(r.text(strings));
                st[i].tint = dim;
            }
            for (a, x) in [(o.left, dl - 12.0), (o.right, dl + dw + 12.0)] {
                if let Some(i) = a {
                    st[i].hidden = Some(!visible || !r.enabled || !arrows);
                    st[i].position = Some([x, y + 12.0]);
                    // U2_MENU_ARROW (speed2.exe 0x54f059).
                    st[i].texture = Some(0x367b_3e04);
                }
            }
            if let Some(i) = o.backing {
                st[i].hidden = Some(!visible);
                // The group's box sits at (−17, 1) inside it.
                st[i].position = Some([row_cx + 17.0, y + th * 0.5 - 1.0]);
            }
            if let (Kind::Slider { min, max, .. }, [Some(base), fill, knob, _, _]) = (&r.kind, o.slider) {
                let f = ((r.value - min) / (max - min)).clamp(0.0, 1.0);
                let w = f * track * 0.95;
                let cy = y + th * 0.5;
                st[base].hidden = Some(!visible);
                st[base].position = Some([dl + dw * 0.5, cy]);
                if let Some(i) = fill {
                    st[i].hidden = Some(!visible || w <= 0.0);
                    st[i].position = Some([track_start + w * 0.5, cy]);
                    st[i].size = Some([w, fill_h]);
                }
                if let Some(i) = knob {
                    st[i].hidden = Some(!visible);
                    st[i].position = Some([track_start + w + if f > 0.0 { 7.0 } else { 0.0 }, cy]);
                }
            }
        }
    }
}
