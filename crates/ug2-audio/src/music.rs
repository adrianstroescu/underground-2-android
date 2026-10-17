//! Music player: streams the EA Trax songs of `pfdata/MusicSFx.mus` on a background
//! thread, as the jukebox settings say (see [`Playlist`]), into a ring buffer of 48 kHz stereo samples that the mixer drains.
//! EA-XA songs are decoded directly; EALayer3 songs are rebuilt as standard MP3 frames
//! (see `ug2_formats::audio::EaLayer3`) and decoded with symphonia.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use ug2_formats::audio::{decode_ea_xa_block, scan_streams, Codec, EaLayer3, SchlReader, StreamHeader};

use crate::sound::RATE;

pub type Ring = Arc<Mutex<VecDeque<f32>>>;

/// Keep about a second of audio queued.
const TARGET: usize = RATE as usize * 2;

pub struct Music {
    pub ring: Ring,
    stop: Arc<AtomicBool>,
    /// What to play: the EA Trax settings and whether a race is on.
    pub playlist: Arc<Mutex<Playlist>>,
    /// Set to end the current song (the menu / race set changed).
    skip: Arc<AtomicBool>,
}

/// The licensed songs of the EA Trax list: song id N is SCHl stream SONG_STREAMS[N]
/// of MusicSFx.mus (PathFinder map MusicSFx.mpf: event N → node 3N → 3N+2 plays its
/// sample, played whole). The other streams are the background score.
pub const SONG_STREAMS: [usize; 27] = [70, 45, 66, 49, 71, 60, 43, 56, 57, 69, 64, 65, 61, 50, 46, 47, 44, 62, 68, 67, 48, 51, 72, 55, 59, 58, 63];

/// EA Trax playback (speed2.exe 0x479220 masks, 0x46b6f0 picks): songs enabled for
/// the menus (mode Menu or All) or races (Racing or All); no song repeats until every
/// enabled one has played; Sequential follows the list, Random picks among the rest.
#[derive(Clone, Debug)]
pub struct Playlist {
    /// Mode per song id: 0 Off, 1 Menu, 2 Racing, 3 All.
    pub modes: [u8; 27],
    pub random: bool,
    pub in_race: bool,
}

impl Default for Playlist {
    fn default() -> Self {
        // Songs 0..5 are menu songs, the rest race songs (0x515aa0).
        let mut modes = [2u8; 27];
        for m in modes.iter_mut().take(6) {
            *m = 1;
        }
        Self { modes, random: false, in_race: false }
    }
}

impl Playlist {
    fn mask(&self) -> u32 {
        let want = if self.in_race { 2 } else { 1 };
        (0..27).filter(|&i| self.modes[i] == want || self.modes[i] == 3).fold(0, |m, i| m | 1 << i)
    }
}

impl Music {
    /// Changes the playlist; a change of set (menu ↔ race) ends the current song.
    pub fn set_playlist(&self, p: Playlist) {
        if let Ok(mut cur) = self.playlist.lock() {
            if cur.in_race != p.in_race || cur.mask() != p.mask() {
                self.skip.store(true, Ordering::Relaxed);
            }
            *cur = p;
        }
    }
}

impl Drop for Music {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Music {
    pub fn start(path: PathBuf) -> Self {
        let ring: Ring = Arc::new(Mutex::new(VecDeque::with_capacity(TARGET * 2)));
        let stop = Arc::new(AtomicBool::new(false));
        let playlist = Arc::new(Mutex::new(Playlist::default()));
        let skip = Arc::new(AtomicBool::new(false));
        let (r, s, p, k) = (ring.clone(), stop.clone(), playlist.clone(), skip.clone());
        let _ = std::thread::Builder::new().name("ug2-music".into()).spawn(move || {
            if let Err(e) = run(&path, &r, &s, &p, &k) {
                log::warn!("music stopped: {e:#}");
            }
        });
        Self { ring, stop, playlist, skip }
    }
}

/// Converts decoded interleaved i16 at `rate` to 48 kHz stereo and queues it,
/// waiting while the queue is full.
pub struct Sink<'a> {
    ring: &'a Ring,
    stop: &'a AtomicBool,
    /// Ends the song early (and drops what is queued).
    skip: Option<&'a AtomicBool>,
    pos: f64,
    last: [f32; 2],
}

impl<'a> Sink<'a> {
    pub fn new(ring: &'a Ring, stop: &'a AtomicBool) -> Self {
        Self { ring, stop, skip: None, pos: 0.0, last: [0.0; 2] }
    }

    /// Queues PCM; false once `stop` is set.
    pub fn push(&mut self, pcm: &[i16], channels: usize, rate: u32) -> bool {
        let channels = channels.max(1);
        let frames = pcm.len() / channels;
        let step = rate as f64 / RATE as f64;
        let frame = |i: usize| -> [f32; 2] {
            let l = pcm[i * channels] as f32 / 32768.0;
            let r = if channels > 1 { pcm[i * channels + 1] as f32 / 32768.0 } else { l };
            [l, r]
        };
        let mut out = Vec::with_capacity((frames as f64 / step) as usize * 2 + 4);
        while self.pos < frames as f64 {
            let i = self.pos.floor() as isize;
            let f = (self.pos - i as f64) as f32;
            let a = if i < 0 { self.last } else { frame(i as usize) };
            let b = if (i + 1) as usize >= frames { a } else { frame((i + 1) as usize) };
            out.push(a[0] + (b[0] - a[0]) * f);
            out.push(a[1] + (b[1] - a[1]) * f);
            self.pos += step;
        }
        self.pos -= frames as f64;
        if frames > 0 {
            self.last = frame(frames - 1);
        }
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return false;
            }
            if self.skip.is_some_and(|s| s.load(Ordering::Relaxed)) {
                self.ring.lock().unwrap().clear();
                return false;
            }
            {
                let mut q = self.ring.lock().unwrap();
                if q.len() < TARGET {
                    q.extend(out.iter().copied());
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

fn run(path: &PathBuf, ring: &Ring, stop: &AtomicBool, playlist: &Mutex<Playlist>, skip: &AtomicBool) -> anyhow::Result<()> {
    let mut f = BufReader::new(File::open(path)?);
    let streams: Vec<(u64, StreamHeader)> = scan_streams(&mut f)?;
    log::info!("music: {} streams", streams.len());
    // The game's random numbers (0x43c1c0).
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u32);
    let mut rand = move |n: u32| {
        let r = if n == 0 { 0 } else { seed % n };
        let s = seed ^ 0x1d87_2b41;
        let t = s ^ (s >> 5);
        seed = t ^ (t << 27) ^ s;
        r
    };
    let (mut remaining, mut last, mut set) = (0u32, None::<usize>, None::<bool>);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let p = playlist.lock().map(|p| p.clone()).unwrap_or_default();
        let full = p.mask();
        if set != Some(p.in_race) {
            set = Some(p.in_race);
            remaining = full;
        }
        if full == 0 {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        remaining &= full;
        if remaining == 0 {
            remaining = full;
        }
        let song = if p.random {
            // The r-th remaining song in id order.
            let r = rand(remaining.count_ones());
            (0..27).filter(|i| remaining & (1 << i) != 0).nth(r as usize).unwrap_or(0)
        } else {
            // The next list position after the last one played.
            let start = last.map_or(0, |l| l + 1);
            (0..27).map(|k| (start + k) % 27).find(|i| remaining & (1 << i) != 0).unwrap_or(0)
        };
        remaining &= !(1 << song);
        last = Some(song);
        skip.store(false, Ordering::Relaxed);
        let Some((offset, h)) = streams.get(SONG_STREAMS[song]) else { continue };
        log::info!("music: song {song} ({}), stream {}", if p.in_race { "race" } else { "menu" }, SONG_STREAMS[song]);
        let mut sink = Sink { ring, stop, skip: Some(skip), pos: 0.0, last: [0.0; 2] };
        let result = match h.codec {
            Codec::EaXa => play_ea_xa(path, *offset, h, &mut sink),
            Codec::EaLayer3 => play_layer3(path, *offset, h, &mut sink),
            _ => Ok(true),
        };
        match result {
            Ok(_) if stop.load(Ordering::Relaxed) => return Ok(()),
            Ok(_) => {}
            Err(e) => log::warn!("music: song {song}: {e:#}"),
        }
    }
}

fn play_ea_xa(path: &PathBuf, offset: u64, h: &StreamHeader, sink: &mut Sink) -> anyhow::Result<bool> {
    let mut r = SchlReader::open(BufReader::new(File::open(path)?), offset)?;
    let (mut block, mut pcm) = (Vec::new(), Vec::new());
    while r.next_block(&mut block)? {
        pcm.clear();
        decode_ea_xa_block(&block, h.channels, &mut pcm)?;
        if !sink.push(&pcm, h.channels, h.sample_rate) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn play_layer3(path: &PathBuf, offset: u64, h: &StreamHeader, sink: &mut Sink) -> anyhow::Result<bool> {
    // Rebuild the whole song as MP3 (a few MB), then decode it frame by frame.
    let mut r = SchlReader::open(BufReader::new(File::open(path)?), offset)?;
    let (mut block, mut mp3) = (Vec::new(), Vec::new());
    let mut conv = EaLayer3::new();
    while r.next_block(&mut block)? {
        conv.push_block(&block, h.channels, &mut mp3)?;
    }
    let mss = MediaSourceStream::new(Box::new(Cursor::new(mp3)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow::anyhow!("no track"))?.clone();
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut buf: Option<SampleBuffer<i16>> = None;
    while let Ok(packet) = format.next_packet() {
        let Ok(decoded) = dec.decode(&packet) else { continue };
        let spec = *decoded.spec();
        let sb = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if sb.capacity() < decoded.capacity() * spec.channels.count() {
            *sb = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        sb.copy_interleaved_ref(decoded);
        if !sink.push(sb.samples(), spec.channels.count(), spec.rate) {
            return Ok(false);
        }
    }
    Ok(true)
}
