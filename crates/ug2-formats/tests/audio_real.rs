//! Checks the Rust decoders against values produced by the Python reference decoder
//! (scratchpad/audio/ea_audio.py) on the real game files. Skips when the game is absent.
use std::fs::File;
use std::path::PathBuf;

use ug2_formats::audio::*;

fn game() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var("UG2_GAME").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../..").into()));
    p.join("SOUND").exists().then_some(p)
}

#[test]
fn bank_engine_loop() {
    let Some(g) = game() else { return };
    let b = std::fs::read(g.join("SOUND/ENGINE/CAR_27_ENG_MB_EE.abk")).unwrap();
    let sounds = parse_abk(&b).unwrap();
    assert_eq!(sounds.len(), 8);
    let s = &sounds[0];
    assert_eq!((s.index, s.sample_rate, s.samples, s.looping), (1, 22050, 36806, Some((0, 36805))));
    let pcm = s.decode(&b).unwrap();
    assert_eq!(pcm.len(), 36806);
    assert_eq!(pcm.iter().map(|&x| x as i64).sum::<i64>(), -4306088);
    assert_eq!(&pcm[..6], &[236, 986, 1547, 2181, 2926, 3606]);
    // every bank in the game decodes
    for e in walk(&g.join("SOUND")) {
        if e.extension().is_some_and(|x| x.eq_ignore_ascii_case("abk")) {
            let b = std::fs::read(&e).unwrap();
            for s in parse_abk(&b).unwrap() {
                assert_eq!(s.decode(&b).unwrap().len(), s.samples as usize, "{}", e.display());
            }
        }
    }
}

fn walk(d: &std::path::Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(d).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() { v.extend(walk(&p)) } else { v.push(p) }
    }
    v
}

#[test]
fn gin_sweep() {
    let Some(g) = game() else { return };
    let gin = Gin::parse(&std::fs::read(g.join("SOUND/ENGINE/GIN_350Z_Jason_shrt2.gin")).unwrap()).unwrap();
    assert_eq!((gin.sample_rate, gin.pcm.len(), gin.grains.len(), gin.rpm_curve.len()), (32000, 148741, 51, 152));
    assert_eq!(gin.pcm.iter().map(|&x| x as i64).sum::<i64>(), -321736291);
    assert_eq!(&gin.pcm[..6], &[-31056, -30960, -30910, -30770, -30687, -30780]);
    assert_eq!(gin.position(0.0), 0.0);
    assert_eq!(gin.position(1e9), 148740.0);
    let mid = gin.position((gin.min_rpm + gin.max_rpm) / 2.0);
    assert!(mid > 90000.0 && mid < 110000.0, "{mid}");
    let i = gin.grain_at(mid);
    assert!(gin.grains[i] as f32 <= mid && mid < gin.grains[i + 1] as f32);
    assert!(!gin.grain(i).is_empty());
    for e in walk(&g.join("SOUND/ENGINE")) {
        if e.extension().is_some_and(|x| x == "gin") {
            let gin = Gin::parse(&std::fs::read(&e).unwrap()).unwrap();
            assert!(gin.max_rpm > gin.min_rpm && *gin.rpm_curve.last().unwrap() as usize <= gin.pcm.len());
        }
    }
}

#[test]
fn music_streams() {
    let Some(g) = game() else { return };
    let path = g.join("pfdata/MusicSFx.mus");
    let mut f = File::open(&path).unwrap();
    let streams = scan_streams(&mut f).unwrap();
    assert_eq!(streams.len(), 73);
    assert_eq!(streams.iter().filter(|s| s.1.codec == Codec::EaLayer3).count(), 43);

    // EA-XA stream: first 5 blocks
    let mut r = SchlReader::open(File::open(&path).unwrap(), 0x780ad00).unwrap();
    assert_eq!((r.header.sample_rate, r.header.channels, r.header.codec), (44100, 2, Codec::EaXa));
    let (mut blk, mut pcm) = (Vec::new(), Vec::new());
    for _ in 0..5 {
        assert!(r.next_block(&mut blk).unwrap());
        decode_ea_xa_block(&blk, 2, &mut pcm).unwrap();
    }
    assert_eq!(pcm.len(), 29400);
    assert_eq!(pcm.iter().map(|&x| x as i64).sum::<i64>(), -5075246);
    assert_eq!(&pcm[2 * 2940 * 2..2 * 2940 * 2 + 6], &[5952, 2948, 5960, 2507, 5617, 1966]);

    // EALayer3 stream: first 20 blocks give the same MP3 bytes as the reference
    let mut r = SchlReader::open(File::open(&path).unwrap(), 0).unwrap();
    assert_eq!(r.header.codec, Codec::EaLayer3);
    let mut conv = EaLayer3::new();
    let mut mp3 = Vec::new();
    for _ in 0..20 {
        assert!(r.next_block(&mut blk).unwrap());
        conv.push_block(&blk, 2, &mut mp3).unwrap();
    }
    // Byte-identical to the Python reference converter when this was written (23,616 bytes).
    assert_eq!(mp3.len(), 23616);
    assert_eq!(&mp3[..2], &[0xFF, 0xFB], "MPEG-1 Layer III frame sync");
    assert_eq!(conv.pcm_blocks.len(), 1);
    assert_eq!(conv.pcm_blocks[0].0, 47);
    assert_eq!(conv.pcm_blocks[0].1.len(), 94);
    assert_eq!(conv.free_format_frames, 0);
}
