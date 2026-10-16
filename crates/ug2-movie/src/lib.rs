//! Plays the game's movies (`MOVIES/*.vp6`): a background thread reads the EA movie
//! container, decodes VP6 frames with NihAV's decoder and the EA-XA sound track, queues a
//! few frames ahead and feeds the sound to a ring the mixer drains. Frames are timed
//! from the sound actually played, so picture and sound stay together.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nihav_core::codecs::{NABufferType, NADecoderSupport};
use nihav_core::formats::YUV420_FORMAT;
use nihav_core::frame::NAVideoInfo;
use nihav_duck::codecs::vp6::{VP56Decoder, VP6BR};
use ug2_audio::music::{Ring, Sink};
use ug2_formats::audio::{decode_ea_xa_block_be, Codec, StreamHeader};
use ug2_formats::movie::{MovieChunk, MovieHeader, MovieReader};

/// One decoded picture, YUV 4:2:0 with full-range BT.601 colours.
pub struct Frame {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

pub struct Movie {
    pub header: MovieHeader,
    pub ring: Ring,
    frames: Receiver<Frame>,
    stop: Arc<AtomicBool>,
    /// 48 kHz stereo frames queued so far (for the sound clock).
    queued: Arc<AtomicU64>,
    has_sound: Arc<AtomicBool>,
    started: Option<Instant>,
    pending: Option<Frame>,
    current: Option<Frame>,
    ended: Arc<AtomicBool>,
}

impl Drop for Movie {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Movie {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let reader = MovieReader::open(BufReader::new(File::open(path)?))?;
        let header = reader.header;
        let ring: Ring = Arc::new(Mutex::new(Default::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicU64::new(0));
        let has_sound = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let (tx, rx) = sync_channel(6);
        let t = Thread { ring: ring.clone(), stop: stop.clone(), queued: queued.clone(), has_sound: has_sound.clone(), ended: ended.clone() };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        std::thread::Builder::new().name("ug2-movie".into()).spawn(move || {
            if let Err(e) = t.run(reader, tx) {
                log::warn!("movie {name}: {e:#}");
            }
            t.ended.store(true, Ordering::Relaxed);
        })?;
        Ok(Self { header, ring, frames: rx, stop, queued, has_sound, started: None, pending: None, current: None, ended })
    }

    /// Seconds of the movie shown so far: the sound played (queued minus still in the
    /// ring), or the wall clock for a silent movie.
    fn clock(&mut self) -> f64 {
        let start = *self.started.get_or_insert_with(Instant::now);
        if self.has_sound.load(Ordering::Relaxed) {
            let waiting = self.ring.lock().map_or(0, |r| r.len() / 2) as u64;
            let played = self.queued.load(Ordering::Relaxed).saturating_sub(waiting);
            played as f64 / ug2_audio::sound::RATE as f64
        } else {
            start.elapsed().as_secs_f64()
        }
    }

    /// The picture to show now; `None` until the first one is decoded.
    pub fn frame(&mut self) -> Option<&Frame> {
        let t = self.clock();
        loop {
            if self.pending.is_none() {
                match self.frames.try_recv() {
                    Ok(f) => self.pending = Some(f),
                    Err(_) => break,
                }
            }
            let due = self.pending.as_ref().is_some_and(|f| f.index as f64 * self.header.frame_time <= t || self.current.is_none());
            if !due {
                break;
            }
            self.current = self.pending.take();
        }
        self.current.as_ref()
    }

    /// True once every frame has been decoded and shown.
    pub fn finished(&self) -> bool {
        self.ended.load(Ordering::Relaxed) && self.pending.is_none() && self.frames.try_recv().is_err()
    }
}

struct Thread {
    ring: Ring,
    stop: Arc<AtomicBool>,
    queued: Arc<AtomicU64>,
    has_sound: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
}

impl Thread {
    fn run(&self, mut reader: MovieReader<BufReader<File>>, tx: SyncSender<Frame>) -> anyhow::Result<()> {
        let h = reader.header;
        let mut dec = VP56Decoder::new(6, false, false);
        let mut parser = VP6BR::new();
        let mut supp = NADecoderSupport::new();
        dec.init(&mut supp, NAVideoInfo::new(h.width as usize, h.height as usize, false, YUV420_FORMAT))
            .map_err(|e| anyhow::anyhow!("VP6 init: {e:?}"))?;
        let mut sound: Option<StreamHeader> = None;
        let mut sink = Sink::new(&self.ring, &self.stop);
        let mut pcm = Vec::new();
        let mut index = 0u32;
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            match reader.next()? {
                MovieChunk::Video { data, .. } => {
                    let buf = match dec.decode_frame(&mut supp, &data, &mut parser) {
                        Ok((buf, _)) => buf,
                        Err(e) => {
                            log::warn!("VP6 frame {index}: {e:?}");
                            index += 1;
                            continue;
                        }
                    };
                    if let NABufferType::Video(vb) = buf {
                        let plane = |i: usize| {
                            let (w, hh) = vb.get_dimensions(i);
                            let (off, stride) = (vb.get_offset(i), vb.get_stride(i));
                            let data = vb.get_data();
                            let mut out = Vec::with_capacity(w * hh);
                            for row in 0..hh {
                                out.extend_from_slice(&data[off + row * stride..off + row * stride + w]);
                            }
                            (w, hh, out)
                        };
                        let (w, hh, y) = plane(0);
                        let (_, _, u) = plane(1);
                        let (_, _, v) = plane(2);
                        let f = Frame { index, width: w as u32, height: hh as u32, y, u, v };
                        if tx.send(f).is_err() {
                            return Ok(());
                        }
                    }
                    index += 1;
                }
                MovieChunk::AudioHeader(s) => {
                    if s.codec == Codec::EaXa {
                        self.has_sound.store(true, Ordering::Relaxed);
                        sound = Some(s);
                    } else {
                        log::warn!("movie sound codec {:?} not supported", s.codec);
                    }
                }
                MovieChunk::Audio(block) => {
                    let Some(s) = &sound else { continue };
                    pcm.clear();
                    decode_ea_xa_block_be(&block, s.channels, &mut pcm)?;
                    if !sink.push(&pcm, s.channels, s.sample_rate) {
                        return Ok(());
                    }
                    // 48 kHz frames this block added to the ring.
                    let added = (pcm.len() / s.channels.max(1)) as f64 * ug2_audio::sound::RATE as f64 / s.sample_rate as f64;
                    self.queued.fetch_add(added.round() as u64, Ordering::Relaxed);
                }
                MovieChunk::End => return Ok(()),
            }
        }
    }
}
