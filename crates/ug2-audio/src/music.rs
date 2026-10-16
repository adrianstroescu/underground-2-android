//! Music player: streams the songs in `pfdata/MusicSFx.mus` on a background thread,
//! shuffled, into a ring buffer of 48 kHz stereo samples that the mixer drains.
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
        let (r, s) = (ring.clone(), stop.clone());
        let _ = std::thread::Builder::new().name("ug2-music".into()).spawn(move || {
            if let Err(e) = run(&path, &r, &s) {
                log::warn!("music stopped: {e:#}");
            }
        });
        Self { ring, stop }
    }
}

/// Converts decoded interleaved i16 at `rate` to 48 kHz stereo and queues it,
/// waiting while the queue is full.
pub struct Sink<'a> {
    ring: &'a Ring,
    stop: &'a AtomicBool,
    pos: f64,
    last: [f32; 2],
}

impl<'a> Sink<'a> {
    pub fn new(ring: &'a Ring, stop: &'a AtomicBool) -> Self {
        Self { ring, stop, pos: 0.0, last: [0.0; 2] }
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

fn run(path: &PathBuf, ring: &Ring, stop: &AtomicBool) -> anyhow::Result<()> {
    let mut f = BufReader::new(File::open(path)?);
    let songs: Vec<(u64, StreamHeader)> = scan_streams(&mut f)?;
    log::info!("music: {} songs", songs.len());
    if songs.is_empty() {
        return Ok(());
    }
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64);
    let mut order: Vec<usize> = (0..songs.len()).collect();
    loop {
        for i in (1..order.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            order.swap(i, (seed % (i as u64 + 1)) as usize);
        }
        for &i in &order {
            let (offset, h) = &songs[i];
            let mut sink = Sink { ring, stop, pos: 0.0, last: [0.0; 2] };
            let result = match h.codec {
                Codec::EaXa => play_ea_xa(path, *offset, h, &mut sink),
                Codec::EaLayer3 => play_layer3(path, *offset, h, &mut sink),
                _ => Ok(true),
            };
            match result {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(e) => log::warn!("music: song {i}: {e:#}"),
            }
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
