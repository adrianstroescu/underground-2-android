//! The EA Trax jukebox (EA_Trax_Jukebox.fng, speed2.exe ctor 0x553c70, handler
//! 0x56b1c0): the 27 songs of the exe's table (0x7ecd80, plain ASCII, not language
//! labels), four rows at a time, each song Off / Menu / Racing / All (defaults from its
//! FE / IG category, 0x515aa0), and Sequential / Random play.

use ug2_formats::bin_hash;
use ug2_formats::feng_scene::Scene;
use ug2_formats::lang::Strings;

fn h(name: &str) -> u32 {
    bin_hash(&name.to_ascii_uppercase())
}

/// (title, artist, album, menu song).
pub const SONGS: [(&str, &str, &str, bool); 27] = [
    ("\"Riders on the Storm (Fredwreck Remix)\"", "Snoop Dogg feat. The Doors", "Need for Speed Underground 2 Exclusive", true),
    ("\"I Need Speed\"", "Capone", "NFSU2 Exclusive", true),
    ("\"I Do\"", "Chingy", "PowerBallin'", true),
    ("\"That'z My Name\"", "Sly Boogy", "Mistaken Identity", true),
    ("\"LAX\"", "Xzibit", "Weapons of Mass Destruction", true),
    ("\"Lean Back\"", "Terror Squad", "True Story", true),
    ("\"Switch/Twitch\"", "Fluke", "Puppy", false),
    ("\"Rush Hour\"", "Christopher Lawrence", "All or Nothing", false),
    ("\"Rocket Ride (Soulwax Remix)\"", "Felix Da Housecat", "Devin Dazzle & the Neon Fever", false),
    ("\"Hard EBM\"", "Sin", "Errare Digital EST", false),
    ("\"Mind Killer (Jagz Kooner Remix)\"", "FREELAND", "Now & Them", false),
    ("\"Nothing But You (Cirrus Remix)\"", "Paul Van Dyk", "Reflections", false),
    ("\"E-Ville\"", "Sonic Animation", "Reality By Deception", false),
    ("\"The Death & Resurrection Show\"", "Killing Joke", "Killing Joke", false),
    ("\"Give it All\"", "Rise Against", "Siren Song of the Counter Culture", false),
    ("\"Scavenger\"", "Killradio", "Raised on Whipped Cream", false),
    ("\"Notice of Eviction\"", "The Bronx", "The Bronx", false),
    ("\"No W\"", "Ministry", "Houses of the Mol\u{e9}", false),
    ("\"In My Head\"", "Queens of the Stone Age", "Lullabies to Paralyze", false),
    ("\"Determined\"", "Mudvayne", "", false),
    ("\"I Am Weightless\"", "Septembre", "Rule 3: Conceal Your Intentions", false),
    ("\"Crashing Foreign Cars\"", "Helmet", "Size Matters", false),
    ("\"Back on a Mission\"", "Cirrus", "Back on a Mission", false),
    ("\"Black Betty\"", "SpiderBait", "Tonight Alright", false),
    ("\"Nobody\"", "Skindred", "Babylon", false),
    ("\"Skeptic\"", "Snapcase", "Bright Flashes", false),
    ("\"The Celebration Song\"", "Unwritten Law", "The Way", false),
];

/// Song modes: Off, Menu, Racing, All.
const MODES: [u32; 4] = [0x9cce_9f86, 0x5278_c50b, 0x5c1b_351c, 0x9cce_64c4];

const ROWS: usize = 4;

pub struct Trax {
    /// Mode per song (0..3).
    pub modes: [u8; 27],
    /// 0 Sequential, 1 Random.
    pub random: bool,
    pub selected: usize,
    top: usize,
}

impl Trax {
    pub fn new(get: impl Fn(&str) -> Option<f32>) -> Self {
        let mut modes = [0u8; 27];
        for (i, m) in modes.iter_mut().enumerate() {
            let default = if SONGS[i].3 { 1.0 } else { 2.0 };
            *m = get(&format!("trax{i}")).unwrap_or(default) as u8 % 4;
        }
        Trax { modes, random: get("trax_random").unwrap_or(0.0) != 0.0, selected: 0, top: 0 }
    }

    /// The settings to save (`opt.trax<i>`, `opt.trax_random`).
    pub fn values(&self) -> Vec<(String, f32)> {
        let mut v: Vec<(String, f32)> = self.modes.iter().enumerate().map(|(i, m)| (format!("trax{i}"), *m as f32)).collect();
        v.push(("trax_random".into(), self.random as u8 as f32));
        v
    }

    pub fn step_row(&mut self, d: i32) -> bool {
        let n = (self.selected as i32 + d).clamp(0, SONGS.len() as i32 - 1) as usize;
        if n == self.selected {
            return false;
        }
        self.selected = n;
        if n < self.top {
            self.top = n;
        } else if n >= self.top + ROWS {
            self.top = n + 1 - ROWS;
        }
        true
    }

    /// Left / right on the selected song: its mode, wrapping.
    pub fn change(&mut self, d: i32) {
        let m = &mut self.modes[self.selected];
        *m = (*m as i32 + d).rem_euclid(4) as u8;
    }

    /// A tap at virtual (x, y) from the screen centre.
    pub fn tap(&mut self, x: f32, y: f32) {
        // The play order arrows by TrackMode_Type (y −193).
        if (y + 193.0).abs() < 20.0 {
            self.random = !self.random;
            return;
        }
        // Rows 80 apart from y −120.
        let k = ((y + 160.0) / 80.0).floor();
        if !(0.0..ROWS as f32).contains(&k) {
            return;
        }
        let row = self.top + k as usize;
        if row >= SONGS.len() {
            return;
        }
        if row != self.selected {
            self.selected = row;
            return;
        }
        if x > 80.0 {
            self.change(if x < 172.0 { -1 } else { 1 });
        }
    }

    pub fn apply(&self, scene: &mut Scene, strings: &Strings) {
        let set = |scene: &mut Scene, n: &str, t: String| {
            if let Some(i) = scene.index_of(h(n)) {
                scene.states[i].text = Some(t);
            }
        };
        set(scene, "TrackMode_Type", strings.by_hash(if self.random { 0xa9c9_c8f7 } else { 0x4ca3_6b89 }).unwrap_or("").to_string());
        for k in 0..ROWS {
            let i = self.top + k;
            let n = k + 1;
            let Some(song) = SONGS.get(i) else { continue };
            set(scene, &format!("TRAX_INDEX_{n}"), if i + 1 < 10 { format!("0{}", i + 1) } else { (i + 1).to_string() });
            set(scene, &format!("TRAX_ARTIST_{n}"), song.1.to_string());
            set(scene, &format!("TRAX_TITLE_{n}"), song.0.to_string());
            set(scene, &format!("TRAX_ALBUM_{n}"), song.2.to_string());
            set(scene, &format!("TRAX_STATE_{n}"), strings.by_hash(MODES[self.modes[i] as usize]).unwrap_or("").to_string());
            // The arrows show on the selected row.
            for a in [format!("Left_arrow_{n}"), format!("right_arrow_{n}")] {
                if let Some(j) = scene.index_of(h(&a)) {
                    scene.states[j].hidden = Some(i != self.selected);
                }
            }
        }
    }
}
