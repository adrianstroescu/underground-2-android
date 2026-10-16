//! Every racer's engine loads at every upgrade level (needs the game files; skips
//! when they are absent).

use ug2_audio::sound::EngineSound;
use ug2_formats::audio::CAR_SOUNDS;
use ug2_formats::fs::GameDir;

#[test]
fn all_engines_load() {
    let game = GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
    if game.read("SOUND/SHIFTING/GEAR_MED_Base.abk").is_err() {
        return;
    }
    for (car, _) in CAR_SOUNDS.iter() {
        for level in 0..=3 {
            let e = EngineSound::load(&game, car, level).unwrap_or_else(|| panic!("{car} level {level}"));
            assert!(!e.accel.pcm.is_empty() && e.accel.grains.len() > 2, "{car} level {level}");
            assert!(!e.shifts.is_empty(), "{car} level {level}: no gear shift sounds");
        }
    }
}
