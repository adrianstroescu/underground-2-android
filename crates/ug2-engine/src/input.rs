//! Input: platform layers feed [`RawInput`]; bindings from `controls.cfg` turn keys and
//! gamepad buttons/axes into driving [`Action`]s. Touch goes through the HUD instead.

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadButton {
    A,
    B,
    X,
    Y,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    Start,
    Select,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    /// Analog triggers, 0..1.
    L2,
    R2,
    HatX,
    HatY,
}

#[derive(Clone, Debug)]
pub enum RawInput {
    /// Pointer position in physical pixels.
    Touch { id: u64, phase: TouchPhase, x: f32, y: f32 },
    /// Key names follow the W3C `KeyboardEvent.code` set ("KeyW", "ArrowUp", "Space").
    Key { code: String, pressed: bool },
    PadButton { button: PadButton, pressed: bool },
    PadAxis { axis: PadAxis, value: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Throttle,
    Brake,
    SteerLeft,
    SteerRight,
    Handbrake,
    Nitro,
    Camera,
    LookBack,
    Reset,
    Pause,
}

impl Action {
    pub const ALL: [Action; 10] = [
        Action::Throttle,
        Action::Brake,
        Action::SteerLeft,
        Action::SteerRight,
        Action::Handbrake,
        Action::Nitro,
        Action::Camera,
        Action::LookBack,
        Action::Reset,
        Action::Pause,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Action::Throttle => "throttle",
            Action::Brake => "brake",
            Action::SteerLeft => "steer_left",
            Action::SteerRight => "steer_right",
            Action::Handbrake => "handbrake",
            Action::Nitro => "nitro",
            Action::Camera => "camera",
            Action::LookBack => "look_back",
            Action::Reset => "reset",
            Action::Pause => "pause",
        }
    }

    fn from_name(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == s)
    }

    /// Edge-triggered actions fire once per press.
    pub fn is_trigger(self) -> bool {
        matches!(self, Action::Camera | Action::Reset | Action::Pause)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    Key(String),
    Pad(PadButton),
    /// Axis value multiplied by `sign`, positive part used.
    Axis(PadAxis, f32),
}

fn pad_button(s: &str) -> Option<PadButton> {
    use PadButton::*;
    Some(match s {
        "A" => A,
        "B" => B,
        "X" => X,
        "Y" => Y,
        "L1" => L1,
        "R1" => R1,
        "L2" => L2,
        "R2" => R2,
        "L3" => L3,
        "R3" => R3,
        "Start" => Start,
        "Select" => Select,
        "DpadUp" => DpadUp,
        "DpadDown" => DpadDown,
        "DpadLeft" => DpadLeft,
        "DpadRight" => DpadRight,
        _ => return None,
    })
}

fn pad_axis(s: &str) -> Option<PadAxis> {
    use PadAxis::*;
    Some(match s {
        "LeftX" => LeftX,
        "LeftY" => LeftY,
        "RightX" => RightX,
        "RightY" => RightY,
        "L2" => L2,
        "R2" => R2,
        "HatX" => HatX,
        "HatY" => HatY,
        _ => return None,
    })
}

impl Binding {
    fn parse(s: &str) -> Option<Binding> {
        let (kind, rest) = s.split_once(':')?;
        match kind.trim() {
            "key" => Some(Binding::Key(rest.trim().to_string())),
            "pad" => pad_button(rest.trim()).map(Binding::Pad),
            "axis" => {
                let rest = rest.trim();
                let (sign, name) = match rest.as_bytes().first() {
                    Some(b'-') => (-1.0, &rest[1..]),
                    Some(b'+') => (1.0, &rest[1..]),
                    _ => (1.0, rest),
                };
                pad_axis(name).map(|a| Binding::Axis(a, sign))
            }
            _ => None,
        }
    }

    fn to_text(&self) -> String {
        match self {
            Binding::Key(k) => format!("key:{k}"),
            Binding::Pad(b) => format!("pad:{b:?}"),
            Binding::Axis(a, s) => format!("axis:{}{a:?}", if *s < 0.0 { "-" } else { "+" }),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InputMap {
    pub bindings: Vec<(Action, Vec<Binding>)>,
}

impl Default for InputMap {
    fn default() -> Self {
        let b = |s: &[&str]| s.iter().filter_map(|x| Binding::parse(x)).collect::<Vec<_>>();
        Self {
            bindings: vec![
                (Action::Throttle, b(&["key:ArrowUp", "key:KeyW", "axis:+R2", "pad:R1"])),
                (Action::Brake, b(&["key:ArrowDown", "key:KeyS", "axis:+L2", "pad:L1"])),
                (Action::SteerLeft, b(&["key:ArrowLeft", "key:KeyA", "axis:-LeftX", "axis:-HatX", "pad:DpadLeft"])),
                (Action::SteerRight, b(&["key:ArrowRight", "key:KeyD", "axis:+LeftX", "axis:+HatX", "pad:DpadRight"])),
                (Action::Handbrake, b(&["key:Space", "pad:X"])),
                (Action::Nitro, b(&["key:ShiftLeft", "key:KeyN", "pad:A"])),
                (Action::Camera, b(&["key:KeyC", "pad:Y"])),
                (Action::LookBack, b(&["key:KeyB", "pad:B"])),
                (Action::Reset, b(&["key:KeyR", "pad:Select"])),
                (Action::Pause, b(&["key:Escape", "key:KeyP", "pad:Start"])),
            ],
        }
    }
}

impl InputMap {
    pub fn load_or_create(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut map = Self::default();
                for line in text.lines() {
                    let line = line.split('#').next().unwrap_or("").trim();
                    let Some((k, v)) = line.split_once('=') else { continue };
                    let Some(action) = Action::from_name(k.trim()) else {
                        log::warn!("controls: unknown action {k}");
                        continue;
                    };
                    let binds: Vec<Binding> = v.split(',').filter_map(|s| Binding::parse(s.trim())).collect();
                    if let Some(slot) = map.bindings.iter_mut().find(|(a, _)| *a == action) {
                        slot.1 = binds;
                    }
                }
                map
            }
            Err(_) => {
                let map = Self::default();
                if let Err(e) = std::fs::write(path, map.to_text()) {
                    log::warn!("cannot write {}: {e}", path.display());
                }
                map
            }
        }
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from(
            "# Controls: action = binding, binding, ...\n\
             # key:<KeyboardEvent.code>  pad:<A|B|X|Y|L1|R1|L2|R2|L3|R3|Start|Select|DpadUp|...>\n\
             # axis:[+|-]<LeftX|LeftY|RightX|RightY|L2|R2|HatX|HatY>\n",
        );
        for (a, binds) in &self.bindings {
            let list: Vec<String> = binds.iter().map(Binding::to_text).collect();
            s += &format!("{} = {}\n", a.name(), list.join(", "));
        }
        s
    }
}

/// Analog driving controls for one frame, merged from every source.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// -1 (left) .. 1 (right)
    pub steer: f32,
    pub throttle: f32,
    pub brake: f32,
    pub handbrake: bool,
    pub nitro: bool,
    pub look_back: bool,
}

/// Tracks keyboard and gamepad state and resolves it through the [`InputMap`].
pub struct InputState {
    map: InputMap,
    keys: HashMap<String, bool>,
    buttons: HashMap<PadButton, bool>,
    axes: HashMap<PadAxis, f32>,
    triggered: Vec<Action>,
    pub last_device_was_pad: bool,
}

const DEADZONE: f32 = 0.12;

impl InputState {
    pub fn new(map: InputMap) -> Self {
        Self {
            map,
            keys: HashMap::new(),
            buttons: HashMap::new(),
            axes: HashMap::new(),
            triggered: Vec::new(),
            last_device_was_pad: false,
        }
    }

    pub fn handle(&mut self, ev: &RawInput) {
        let before: Vec<bool> = Action::ALL.iter().map(|&a| self.value(a) > 0.5).collect();
        match ev {
            RawInput::Key { code, pressed } => {
                self.keys.insert(code.clone(), *pressed);
                self.last_device_was_pad = false;
            }
            RawInput::PadButton { button, pressed } => {
                self.buttons.insert(*button, *pressed);
                self.last_device_was_pad = true;
            }
            RawInput::PadAxis { axis, value } => {
                self.axes.insert(*axis, *value);
                if value.abs() > 0.3 {
                    self.last_device_was_pad = true;
                }
            }
            RawInput::Touch { .. } => {}
        }
        for (i, &a) in Action::ALL.iter().enumerate() {
            if a.is_trigger() && !before[i] && self.value(a) > 0.5 {
                self.triggered.push(a);
            }
        }
    }

    /// 0..1 strength of an action from keyboard/gamepad.
    pub fn value(&self, action: Action) -> f32 {
        let Some((_, binds)) = self.map.bindings.iter().find(|(a, _)| *a == action) else { return 0.0 };
        let mut v: f32 = 0.0;
        for b in binds {
            let x = match b {
                Binding::Key(k) => self.keys.get(k).copied().unwrap_or(false) as u8 as f32,
                Binding::Pad(p) => self.buttons.get(p).copied().unwrap_or(false) as u8 as f32,
                Binding::Axis(a, sign) => {
                    let raw = self.axes.get(a).copied().unwrap_or(0.0) * sign;
                    if raw > DEADZONE {
                        ((raw - DEADZONE) / (1.0 - DEADZONE)).min(1.0)
                    } else {
                        0.0
                    }
                }
            };
            v = v.max(x);
        }
        v
    }

    pub fn take_triggers(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.triggered)
    }

    pub fn controls(&self) -> Controls {
        Controls {
            steer: self.value(Action::SteerRight) - self.value(Action::SteerLeft),
            throttle: self.value(Action::Throttle),
            brake: self.value(Action::Brake),
            handbrake: self.value(Action::Handbrake) > 0.5,
            nitro: self.value(Action::Nitro) > 0.5,
            look_back: self.value(Action::LookBack) > 0.5,
        }
    }
}
