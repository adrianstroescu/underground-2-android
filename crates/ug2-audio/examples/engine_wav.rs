//! Renders a car's engine through the mixer to a WAV file, for listening to and
//! checking the granular engine without a device:
//!   cargo run --release -p ug2-audio --example engine_wav -- <game-dir> <CAR> <out.wav>
//! Plays idle, three gears at full throttle, then lifts off.

use ug2_audio::sound::{Effects, EngineSound, Mixer, SoundState, RATE};
use ug2_formats::fs::GameDir;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, car, out) = match args.as_slice() {
        [d, c, o] => (d, c, o),
        _ => anyhow::bail!("usage: engine_wav <game-dir> <CAR> <out.wav>"),
    };
    let game = GameDir::new(dir);
    let engine = EngineSound::load(&game, car).ok_or_else(|| anyhow::anyhow!("no engine sound for {car}"))?;
    println!(
        "{car}: accel GIN {:.0}-{:.0} rpm @ {} Hz, {} grains; decel {}",
        engine.accel.min_rpm,
        engine.accel.max_rpm,
        engine.accel.sample_rate,
        engine.accel.grains.len(),
        engine.decel.as_ref().map_or("none".into(), |d| format!("{:.0}-{:.0} rpm", d.min_rpm, d.max_rpm))
    );
    let mut mixer = Mixer::default();
    mixer.set_engine(&engine);
    mixer.set_effects(Effects::load(&game));

    // (seconds, rpm from, rpm to, throttle)
    let script = [(1.0, 0.18, 0.18, 0.0), (2.5, 0.25, 1.0, 1.0), (2.5, 0.45, 1.0, 1.0), (2.5, 0.5, 1.0, 1.0), (2.0, 1.0, 0.3, 0.0)];
    let chunk = RATE as usize / 100; // 10 ms of frames between state updates
    let mut pcm: Vec<i16> = Vec::new();
    let mut buf = vec![0.0f32; chunk * 2];
    for (i, &(secs, from, to, throttle)) in script.iter().enumerate() {
        let steps = (secs * 100.0) as usize;
        if i >= 2 && i <= 3 {
            mixer.shift();
        }
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            mixer.set_state(SoundState {
                rpm: from + (to - from) * t,
                throttle,
                speed: 10.0 + 30.0 * (i as f32 + t) / 5.0,
                active: true,
                ..Default::default()
            });
            mixer.render(&mut buf);
            pcm.extend(buf.iter().map(|&x| (x * 32767.0) as i16));
        }
    }
    write_wav(out, &pcm, RATE, 2)?;
    println!("wrote {out}: {:.1} s", pcm.len() as f32 / 2.0 / RATE as f32);
    Ok(())
}

fn write_wav(path: &str, pcm: &[i16], rate: u32, channels: u16) -> std::io::Result<()> {
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    let data = (pcm.len() * 2) as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    w.extend_from_slice(&(channels * 2).to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, w)
}
