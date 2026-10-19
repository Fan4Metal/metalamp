//! Playback on a thread of its own ("audio"): Media Foundation decodes
//! (`decoder.rs`), the samples wait in a ring of about a second, and an
//! [`output::Output`] (WASAPI, `output.rs`) plays them. The UI sends
//! [`Cmd`]s and takes [`Event`]s; the position, the duration and the gain
//! are shared atomics, read and set without waiting for the thread.
//!
//! Gapless: the UI names the track after the one playing (`Cmd::Preload`);
//! the thread opens it at once and, when the playing one's decoder ends,
//! goes on decoding it into the same ring: the device never runs dry
//! between them. Which track is heard is known from where in the written
//! frames its first one went (`Segment`), against the device's clock. A
//! next track of another sample rate or channel count waits for the ring
//! to drain, then the device is opened anew for it.
//!
//! A device that goes away (unplugged, disabled) or a change of Windows'
//! default device reopens the output and goes on from the position heard.
//!
//! Crossfade: a next track preloaded with a fade (`Cmd::Preload`'s last
//! field, the player's choice) of the same format starts that long before
//! the playing one ends: both are decoded and mixed into the ring with
//! equal-power curves ([`Fade`]); the next track's segment starts where the
//! fade does, so it is heard (and reported) from then on.

pub mod decoder;
pub mod dsp;
pub mod mf;
pub mod mp3gap;
pub mod output;
pub mod vis;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use decoder::{Decoder, SECOND};
use output::{Output, Wasapi};

/// What the UI asks of the audio thread.
#[derive(Debug)]
pub enum Cmd {
    /// Play `path` from its start; `id` names it in the events; `gain`
    /// is its loudness gain (1 when not known).
    Play {
        id: u64,
        path: PathBuf,
        gain: f32,
    },
    /// The track to play after the current one, without a gap (or none),
    /// with its loudness gain and the crossfade into it (ms; 0: none).
    Preload(Option<(u64, PathBuf, f32, u64)>),
    /// The loudness gain of the track `id`, measured meanwhile.
    Gain {
        id: u64,
        gain: f32,
    },
    /// New processing settings.
    Dsp(dsp::DspSettings),
    Pause,
    Resume,
    Stop,
    /// To this time (milliseconds) of the track heard.
    Seek(u64),
    /// Play on this device (its id; empty: the default) from now on.
    Device(String),
}

/// What the audio thread tells the UI.
#[derive(Debug)]
pub enum Event {
    /// The track `id` is now heard (asked for, or the preloaded one after
    /// the last one ended).
    Started { id: u64 },
    /// The last track ended and nothing was preloaded.
    Ended,
    /// The track `id` cannot be played (or preloaded).
    Failed { id: u64, message: String },
}

/// Read and written without waiting for the thread.
#[derive(Default)]
pub struct Shared {
    pub position_ms: AtomicU64,
    pub duration_ms: AtomicU64,
    /// The gain applied to the samples, as `f32` bits.
    gain: AtomicU32,
    quit: AtomicBool,
    /// The samples heard, for the spectrum analyzer.
    pub vis: Mutex<vis::Tap>,
}

pub struct Engine {
    tx: Sender<Cmd>,
    events: Receiver<Event>,
    pub shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start(ctx: egui::Context, device: String, dsp: dsp::DspSettings) -> Engine {
        let (tx, rx) = channel();
        let (etx, events) = channel();
        let shared = Arc::new(Shared::default());
        shared.gain.store(1.0f32.to_bits(), Relaxed);
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(&rx, &etx, &worker, &ctx, device, dsp))
            .map_err(|e| log::error!("cannot start the audio thread: {e}"))
            .ok();
        Engine {
            tx,
            events,
            shared,
            thread,
        }
    }

    pub fn send(&self, cmd: Cmd) {
        log::debug!("audio: {cmd:?}");
        let _ = self.tx.send(cmd);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub fn position_ms(&self) -> u64 {
        self.shared.position_ms.load(Relaxed)
    }

    pub fn duration_ms(&self) -> u64 {
        self.shared.duration_ms.load(Relaxed)
    }

    /// The `n` frames heard last into `out`, for the visualizations, their
    /// sample rate and where they end; `None` while nothing is heard.
    pub fn heard_samples(&self, n: usize, out: &mut Vec<f32>) -> Option<vis::Heard> {
        self.shared
            .vis
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .latest(n, out)
    }

    /// The gain of what is heard, taken at the audio thread's next turn
    /// (within 10 ms while playing).
    pub fn set_gain(&self, gain: f32) {
        self.shared.gain.store(gain.to_bits(), Relaxed);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.quit.store(true, Relaxed);
        let _ = self.tx.send(Cmd::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The volume setting (0 to 100) as a gain: squared, which the ear takes
/// as nearly even steps.
pub fn gain_of(volume: u8, mute: bool) -> f32 {
    if mute {
        0.0
    } else {
        let v = f32::from(volume.min(100)) / 100.0;
        v * v
    }
}

/// A track open for decoding.
struct Open {
    id: u64,
    path: PathBuf,
    decoder: Decoder,
    /// The loudness gain (`dsp::track_gain`), 1 when not known.
    gain: f32,
    /// Frames decoded so far (from where it was opened or sought).
    decoded: u64,
    /// The crossfade into it from the track before (ms; 0: none).
    fade_ms: u64,
}

impl Open {
    fn new(id: u64, path: PathBuf, decoder: Decoder, gain: f32, fade_ms: u64) -> Open {
        Open {
            id,
            path,
            decoder,
            gain,
            decoded: 0,
            fade_ms,
        }
    }

    /// Its length in frames, from the decoder's duration.
    fn total_frames(&self) -> u64 {
        (self.decoder.duration.max(0) as i128 * i128::from(self.decoder.rate) / i128::from(SECOND))
            as u64
    }
}

/// A crossfade under way: the track ending (`from`) and the next one
/// (`Player::decoding`) decoded side by side and mixed into the ring.
struct Fade {
    from: Open,
    /// Its length in frames, and the frames mixed so far.
    len: u64,
    done: u64,
    /// Decoded and not yet mixed, of each.
    a: VecDeque<f32>,
    b: VecDeque<f32>,
    a_ended: bool,
}

/// The gains of the ending and the next track at `t` (0 to 1) of a fade:
/// equal power, so the sum is as loud throughout.
pub fn fade_gains(t: f32) -> (f32, f32) {
    let x = t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    (x.cos(), x.sin())
}

/// Where a track's first frame went among the frames written since the
/// device was last reset, and the time it started from.
struct Segment {
    start: u64,
    id: u64,
    path: PathBuf,
    origin_ms: u64,
    duration_ms: u64,
    /// The track's loudness gain, for the chain at write time.
    gain: f32,
}

/// How far ahead of what is heard the device is kept written, in
/// seconds: the processing is applied at write time, so a change of its
/// settings is heard this soon. The device's buffer is larger
/// (`output::BUFFER`); the ring in memory, a second deep, covers a slow
/// disk, and this only has to cover the thread's own scheduling (it
/// polls every 10 ms at Pro Audio priority).
const WRITE_AHEAD: f32 = 0.15;

struct Player<'a> {
    events: &'a Sender<Event>,
    shared: &'a Shared,
    ctx: &'a egui::Context,
    out: Option<Box<dyn Output>>,
    format: (u32, u16),
    decoding: Option<Open>,
    next: Option<Open>,
    /// A crossfade into `decoding` under way.
    fade: Option<Fade>,
    ring: VecDeque<f32>,
    written: u64,
    segments: VecDeque<Segment>,
    heard_id: Option<u64>,
    paused: bool,
    started: bool,
    /// The decoding track ended with nothing after it.
    eos: bool,
    /// The next track needs another format: drain first.
    format_change: bool,
    drained_since: Option<Instant>,
    checked_device: Instant,
    /// The device wanted (its id; empty: the default).
    device: String,
    dsp: dsp::DspSettings,
    /// The processing for the output's format, made with it.
    chain: Option<dsp::Chain>,
}

fn run(
    rx: &Receiver<Cmd>,
    events: &Sender<Event>,
    shared: &Shared,
    ctx: &egui::Context,
    device: String,
    dsp: dsp::DspSettings,
) {
    let _com = mf::com_init_mta();
    // Before anything holding MF objects, so it is dropped after them.
    let _mf = match mf::Mf::start() {
        Ok(mf) => mf,
        Err(e) => {
            log::error!("Media Foundation: {}", mf::describe(&e));
            return;
        }
    };
    raise_priority();
    let mut p = Player {
        events,
        shared,
        ctx,
        out: None,
        format: (0, 0),
        decoding: None,
        next: None,
        fade: None,
        ring: VecDeque::new(),
        written: 0,
        segments: VecDeque::new(),
        heard_id: None,
        paused: false,
        started: false,
        eos: false,
        format_change: false,
        drained_since: None,
        checked_device: Instant::now(),
        device,
        dsp,
        chain: None,
    };
    loop {
        let busy = p.decoding.is_some() && !p.paused;
        let wait = if busy { 10 } else { 250 };
        match rx.recv_timeout(Duration::from_millis(wait)) {
            Ok(cmd) => {
                p.handle(cmd);
                while let Ok(cmd) = rx.try_recv() {
                    p.handle(cmd);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if shared.quit.load(Relaxed) {
            break;
        }
        if let Err(e) = p.step() {
            log::warn!("audio output: {}", mf::describe(&e));
            p.recover();
        }
    }
}

/// The thread as one of Windows' "Pro Audio" tasks (MMCSS): its turns come
/// on time while other threads read tags and covers.
fn raise_priority() {
    use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
    let mut index = 0u32;
    if let Err(e) =
        unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &mut index) }
    {
        log::debug!("MMCSS: {}", mf::describe(&e));
    }
}

impl Player<'_> {
    fn vis(&self) -> std::sync::MutexGuard<'_, vis::Tap> {
        self.shared.vis.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, event: Event) {
        log::debug!("audio: {event:?}");
        let _ = self.events.send(event);
        self.ctx.request_repaint();
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play { id, path, gain } => {
                // What was preloaded came after the track played so far;
                // the one after this is preloaded anew once it starts.
                self.next = None;
                match Decoder::open(&path) {
                    Ok(decoder) => self.begin(Open::new(id, path, decoder, gain, 0), 0),
                    Err(e) => {
                        // Not the track before going on meanwhile.
                        self.stop();
                        self.send(Event::Failed {
                            id,
                            message: mf::describe(&e),
                        });
                    }
                }
            }
            Cmd::Preload(None) => self.next = None,
            Cmd::Preload(Some((id, path, gain, fade_ms))) => {
                if self.next.as_ref().is_some_and(|n| n.id == id) {
                    return;
                }
                match Decoder::open(&path) {
                    Ok(decoder) => {
                        self.next = Some(Open::new(id, path, decoder, gain, fade_ms));
                        // The track decoded to its end before the next was
                        // known (a short one, one with nothing in it, an
                        // entry added at the end): it goes on now.
                        if self.eos {
                            self.eos = false;
                            self.drained_since = None;
                            self.track_decoded();
                        }
                    }
                    Err(e) => {
                        self.next = None;
                        self.send(Event::Failed {
                            id,
                            message: mf::describe(&e),
                        });
                    }
                }
            }
            Cmd::Pause => {
                self.paused = true;
                // The wait for the last frames to be heard starts again
                // after the pause, not counting it.
                self.drained_since = None;
                if let Some(out) = &mut self.out {
                    let _ = out.stop();
                }
            }
            Cmd::Resume => {
                self.paused = false;
                if self.started
                    && let Some(out) = &mut self.out
                {
                    let _ = out.start();
                }
            }
            Cmd::Stop => self.stop(),
            Cmd::Seek(ms) => self.seek(ms),
            Cmd::Gain { id, gain } => {
                for open in [self.decoding.as_mut(), self.next.as_mut()]
                    .into_iter()
                    .flatten()
                {
                    if open.id == id {
                        open.gain = gain;
                    }
                }
                if let Some(fade) = &mut self.fade
                    && fade.from.id == id
                {
                    fade.from.gain = gain;
                }
                for seg in self.segments.iter_mut().filter(|s| s.id == id) {
                    seg.gain = gain;
                }
            }
            Cmd::Dsp(settings) => {
                if let Some(chain) = &mut self.chain {
                    chain.configure(&settings);
                }
                self.dsp = settings;
            }
            Cmd::Device(device) => {
                if self.device != device {
                    self.device = device;
                    if self.out.is_some() {
                        self.recover();
                    }
                }
            }
        }
    }

    fn stop(&mut self) {
        self.decoding = None;
        self.next = None;
        self.fade = None;
        self.ring.clear();
        self.segments.clear();
        self.heard_id = None;
        self.eos = false;
        self.format_change = false;
        self.paused = false;
        self.started = false;
        self.written = 0;
        self.vis().clear();
        if let Some(out) = &mut self.out {
            let _ = out.reset();
        }
        self.shared.position_ms.store(0, Relaxed);
    }

    /// Start `open` from `from_ms`, dropping what was written.
    fn begin(&mut self, mut open: Open, from_ms: u64) {
        let format = (open.decoder.rate, open.decoder.channels);
        // A decoder already under way (the track heard begun again, a
        // seek to its start among them) is sought even to 0.
        if (from_ms > 0 || open.decoded > 0)
            && let Err(e) = open.decoder.seek(from_ms as i64 * SECOND / 1000)
        {
            log::warn!("cannot seek: {}", mf::describe(&e));
        }
        open.decoded = from_ms * u64::from(open.decoder.rate) / 1000;
        // A fade under way was of the sound before.
        self.fade = None;
        if self.out.is_none() || self.format != format {
            self.out = None;
            match Wasapi::open(format.0, format.1, &self.device) {
                Ok(out) => {
                    self.out = Some(Box::new(out));
                    self.format = format;
                    self.chain = Some(dsp::Chain::new(format.0, format.1, &self.dsp));
                }
                Err(e) => {
                    let id = open.id;
                    self.stop();
                    self.send(Event::Failed {
                        id,
                        message: format!("no sound output: {}", mf::describe(&e)),
                    });
                    return;
                }
            }
        } else {
            if let Some(out) = &mut self.out {
                let _ = out.reset();
            }
            // What the filters and the limiter's delay hold is the sound
            // before.
            if let Some(chain) = &mut self.chain {
                chain.reset();
            }
        }
        self.ring.clear();
        self.written = 0;
        self.vis().clear();
        self.segments.clear();
        self.segments.push_back(Segment {
            start: 0,
            id: open.id,
            path: open.path.clone(),
            origin_ms: from_ms,
            duration_ms: (open.decoder.duration / (SECOND / 1000)) as u64,
            gain: open.gain,
        });
        self.heard_id = None;
        self.decoding = Some(open);
        self.eos = false;
        self.format_change = false;
        self.started = false;
        // A track begun plays: a pause left on would keep the ring empty
        // and the output stopped, the UI saying "playing" (a track chosen
        // while paused was silent). `seek` and `recover` put their pause
        // back after this.
        self.paused = false;
        self.drained_since = None;
        self.shared.position_ms.store(from_ms, Relaxed);
    }

    /// The segment heard now, if any.
    fn heard(&self) -> Option<&Segment> {
        self.segments.front()
    }

    fn seek(&mut self, ms: u64) {
        self.restart_heard(ms);
    }

    /// The track heard begun again at `from_ms`, its pause kept (a seek,
    /// a recovery). It may be behind the one being decoded (the last
    /// second of one, the next already in the ring): it is opened again,
    /// and the one ahead waits as the next. A file that cannot be opened
    /// any more (deleted, its disk gone) fails, as a Play of it would.
    fn restart_heard(&mut self, from_ms: u64) {
        let Some(seg) = self.heard() else { return };
        let (id, path, gain) = (seg.id, seg.path.clone(), seg.gain);
        let paused = self.paused;
        let open = match self.decoding.take() {
            Some(d) if d.id == id => Ok(d),
            ahead => {
                if let Some(mut a) = ahead
                    && a.decoder.seek(0).is_ok()
                {
                    a.decoded = 0;
                    self.next = Some(a);
                }
                Decoder::open(&path).map(|decoder| Open::new(id, path, decoder, gain, 0))
            }
        };
        match open {
            Ok(open) => {
                self.begin(open, from_ms);
                self.paused = paused;
                // Started again for the same track: not news.
                self.heard_id = Some(id);
            }
            Err(e) => {
                self.stop();
                self.send(Event::Failed {
                    id,
                    message: mf::describe(&e),
                });
            }
        }
    }

    /// Decode, write, follow the clock; the next track at the end.
    fn step(&mut self) -> windows::core::Result<()> {
        if self.decoding.is_none() && self.ring.is_empty() {
            return Ok(());
        }
        let Some(out) = self.out.as_mut() else {
            return Ok(());
        };
        let channels = usize::from(self.format.1.max(1));
        if !self.paused {
            let target = out.buffer_frames() as usize * 2 * channels;
            while self.ring.len() < target && !self.eos && !self.format_change {
                if self.fade.is_some() {
                    self.fade_step(channels);
                    continue;
                }
                if self.start_fade(channels) {
                    continue;
                }
                let Some(open) = self.decoding.as_mut() else {
                    break;
                };
                match open.decoder.next() {
                    Ok(Some(block)) => {
                        open.decoded += (block.len() / channels) as u64;
                        self.ring.extend(block);
                    }
                    Ok(None) => self.track_decoded(),
                    Err(e) => {
                        // A damaged end of a file: as if it ended there.
                        log::warn!("decoding {}: {}", open.path.display(), mf::describe(&e));
                        self.track_decoded();
                    }
                }
            }
            let out = self.out.as_mut().expect("checked above");
            // The device applies the gain to what it plays, so a change is
            // heard at once, not after the buffer written ahead.
            out.set_gain(f32::from_bits(self.shared.gain.load(Relaxed)))?;
            // Only `WRITE_AHEAD` of the device's buffer is kept filled.
            let room = out.room()? as usize;
            let held = (out.buffer_frames() as usize).saturating_sub(room);
            let ahead = (self.format.0 as f32 * WRITE_AHEAD) as usize;
            let frames = room
                .min(ahead.saturating_sub(held))
                .min(self.ring.len() / channels);
            if frames > 0 {
                let mut block: Vec<f32> = self.ring.drain(..frames * channels).collect();
                self.process(&mut block, frames);
                let out = self.out.as_mut().expect("checked above");
                out.write(&block)?;
                self.shared
                    .vis
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(&block, channels, self.written);
                self.written += frames as u64;
            }
            if !self.started && self.written > 0 {
                self.out.as_mut().expect("checked above").start()?;
                self.started = true;
            }
        }

        let out = self.out.as_mut().expect("checked above");
        let heard = out.heard()?;
        self.vis().set_heard(heard, self.format.0);
        while self.segments.len() > 1 && self.segments[1].start <= heard {
            self.segments.pop_front();
        }
        if let Some(seg) = self.segments.front() {
            let rate = u64::from(self.format.0.max(1));
            let position = seg.origin_ms + heard.saturating_sub(seg.start) * 1000 / rate;
            self.shared.position_ms.store(position, Relaxed);
            self.shared.duration_ms.store(seg.duration_ms, Relaxed);
            if self.heard_id != Some(seg.id) {
                self.heard_id = Some(seg.id);
                let id = seg.id;
                self.send(Event::Started { id });
            }
        }

        // The end: everything written has been heard (or the clock stopped
        // short of it for a while).
        if (self.eos || self.format_change) && self.ring.is_empty() && !self.paused {
            let since = *self.drained_since.get_or_insert_with(Instant::now);
            if heard >= self.written || since.elapsed() > Duration::from_millis(500) {
                self.drained_since = None;
                if self.format_change
                    && let Some(next) = self.next.take()
                {
                    self.begin(next, 0);
                } else {
                    self.decoding = None;
                    self.next = None;
                    self.vis().clear();
                    self.segments.clear();
                    self.heard_id = None;
                    self.eos = false;
                    self.started = false;
                    if let Some(out) = &mut self.out {
                        let _ = out.reset();
                    }
                    self.send(Event::Ended);
                }
            }
        }

        if self.checked_device.elapsed() >= Duration::from_secs(1) {
            self.checked_device = Instant::now();
            if self.out.as_mut().is_some_and(|o| o.device_changed()) {
                log::info!("the sound device changed");
                self.recover();
            }
        }
        Ok(())
    }

    /// The chain over `block` (`frames` of them) about to be written at
    /// `self.written`, a piece per track it spans, each with its gain.
    fn process(&mut self, block: &mut [f32], frames: usize) {
        let Some(chain) = &mut self.chain else { return };
        let channels = usize::from(self.format.1.max(1));
        let mut at = 0;
        while at < frames {
            let abs = self.written + at as u64;
            // The segment holding this frame: the last one starting at or
            // before it; the piece ends where the next one starts.
            let idx = self
                .segments
                .iter()
                .rposition(|s| s.start <= abs)
                .unwrap_or(0);
            let gain = self.segments.get(idx).map_or(1.0, |s| s.gain);
            if self.segments.get(idx).is_some_and(|s| s.start == abs) {
                chain.begin_track(gain);
            }
            let end = self
                .segments
                .get(idx + 1)
                .map_or(frames, |s| {
                    (s.start.saturating_sub(self.written) as usize).min(frames)
                })
                .max(at + 1);
            chain.process(&mut block[at * channels..end * channels], gain);
            at = end;
        }
    }

    /// Starts a crossfade when the next track asks for one, has the same
    /// format, and the decoding one is that close to its end; true if it
    /// started. The next track's segment starts here: it is heard from
    /// the fade's start.
    fn start_fade(&mut self, channels: usize) -> bool {
        let (Some(cur), Some(next)) = (&self.decoding, &self.next) else {
            return false;
        };
        if next.fade_ms == 0 || (next.decoder.rate, next.decoder.channels) != self.format {
            return false;
        }
        let total = cur.total_frames();
        let left = total.saturating_sub(cur.decoded);
        // Not longer than half of either track.
        let want = (next.fade_ms * u64::from(self.format.0) / 1000)
            .min(total / 2)
            .min(next.total_frames() / 2);
        if want == 0 || left > want || left == 0 {
            return false;
        }
        let (Some(from), Some(next)) = (self.decoding.take(), self.next.take()) else {
            return false;
        };
        self.segments.push_back(Segment {
            start: self.written + (self.ring.len() / channels) as u64,
            id: next.id,
            path: next.path.clone(),
            origin_ms: 0,
            duration_ms: (next.decoder.duration / (SECOND / 1000)) as u64,
            gain: next.gain,
        });
        log::debug!("audio: crossfade of {left} frames into {}", next.id);
        self.fade = Some(Fade {
            from,
            len: left,
            done: 0,
            a: VecDeque::new(),
            b: VecDeque::new(),
            a_ended: false,
        });
        self.decoding = Some(next);
        true
    }

    /// A piece of the crossfade decoded and mixed into the ring; at its end
    /// the ending track is dropped and the next goes on alone.
    fn fade_step(&mut self, channels: usize) {
        const CHUNK: usize = 4096;
        let (Some(fade), Some(next)) = (self.fade.as_mut(), self.decoding.as_mut()) else {
            self.fade = None;
            return;
        };
        while fade.a.len() < CHUNK * channels && !fade.a_ended {
            match fade.from.decoder.next() {
                Ok(Some(block)) => fade.a.extend(block),
                // Its end (or a damaged end): silence for the rest.
                _ => fade.a_ended = true,
            }
        }
        let mut next_ended = false;
        while fade.b.len() < CHUNK * channels {
            match next.decoder.next() {
                Ok(Some(block)) => {
                    next.decoded += (block.len() / channels) as u64;
                    fade.b.extend(block);
                }
                _ => {
                    next_ended = true;
                    break;
                }
            }
        }
        // The chain applies the next track's gain to the mix: the ending
        // one is scaled by its own over it.
        let ratio = if self.dsp.active() && self.dsp.normalize && next.gain > 0.0 {
            fade.from.gain / next.gain
        } else {
            1.0
        };
        let n = (fade.b.len() / channels).min((fade.len - fade.done) as usize);
        for i in 0..n {
            let t = (fade.done + i as u64) as f32 / fade.len as f32;
            let (ga, gb) = fade_gains(t);
            for _ in 0..channels {
                let a = fade.a.pop_front().unwrap_or(0.0);
                let b = fade.b.pop_front().unwrap_or(0.0);
                self.ring.push_back(a * ga * ratio + b * gb);
            }
        }
        fade.done += n as u64;
        if fade.done >= fade.len || n == 0 {
            // The rest of the next track's samples as they are.
            let rest: Vec<f32> = fade.b.drain(..).collect();
            self.ring.extend(rest);
            self.fade = None;
            if next_ended {
                self.track_decoded();
            }
        }
    }

    /// The decoding track ended: the next one goes on in the same ring if
    /// it has the same format.
    fn track_decoded(&mut self) {
        let channels = u64::from(self.format.1.max(1));
        match self.next.take() {
            Some(next) if (next.decoder.rate, next.decoder.channels) == self.format => {
                self.segments.push_back(Segment {
                    start: self.written + self.ring.len() as u64 / channels,
                    id: next.id,
                    path: next.path.clone(),
                    origin_ms: 0,
                    duration_ms: (next.decoder.duration / (SECOND / 1000)) as u64,
                    gain: next.gain,
                });
                self.decoding = Some(next);
            }
            Some(next) => {
                self.next = Some(next);
                self.format_change = true;
            }
            None => self.eos = true,
        }
    }

    /// The output failed or the default device changed: open it again and
    /// go on from where the track heard was.
    fn recover(&mut self) {
        self.out = None;
        if self.heard().is_some() {
            self.restart_heard(self.shared.position_ms.load(Relaxed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_gains_keep_the_power() {
        for i in 0..=10 {
            let (a, b) = fade_gains(i as f32 / 10.0);
            assert!((a * a + b * b - 1.0).abs() < 1e-5);
        }
        assert_eq!(fade_gains(0.0), (1.0, 0.0));
        let (a, b) = fade_gains(1.0);
        assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
    }

    #[test]
    fn volume_to_gain() {
        assert_eq!(gain_of(100, false), 1.0);
        assert_eq!(gain_of(50, false), 0.25);
        assert_eq!(gain_of(0, false), 0.0);
        assert_eq!(gain_of(80, true), 0.0);
        assert_eq!(gain_of(200, false), 1.0);
    }

    /// Decodes a real file (`METALAMP_SAMPLE`) through Media Foundation.
    #[test]
    #[ignore = "needs METALAMP_SAMPLE, an audio file"]
    fn decoder_opens_sample() {
        let path = std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE");
        let _com = mf::com_init_mta();
        let _mf = mf::Mf::start().unwrap();
        let mut d = Decoder::open(std::path::Path::new(&path)).unwrap();
        println!(
            "{} Hz, {} channels, {} s",
            d.rate,
            d.channels,
            d.duration / SECOND
        );
        assert!(d.rate > 0 && d.channels > 0);
        let block = d.next().unwrap().unwrap();
        assert!(!block.is_empty());
        d.seek(10 * SECOND).unwrap();
        assert!(d.next().unwrap().is_some());
    }

    /// The engine on the default device, quietly: the end of one track
    /// goes on into the preloaded next one (`Started` for it, no `Ended`),
    /// and the end of the last one is `Ended`. `METALAMP_SAMPLE` names the
    /// first file; the next file of its folder is the second.
    #[test]
    #[ignore = "needs METALAMP_SAMPLE and a sound device; plays a few seconds"]
    fn engine_goes_on_without_a_gap() {
        use std::time::Instant;
        let first = PathBuf::from(std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE"));
        let mut files: Vec<PathBuf> = std::fs::read_dir(first.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| crate::filetypes::is_audio(p))
            .collect();
        files.sort();
        let second = files
            .iter()
            .skip_while(|p| **p != first)
            .nth(1)
            .cloned()
            .expect("a second file in the folder");
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.05);
        let wait = |what: &dyn Fn(&Event) -> bool, secs: u64| {
            let until = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < until {
                for e in engine.poll() {
                    println!("{e:?} at {} ms", engine.position_ms());
                    if what(&e) {
                        return true;
                    }
                    assert!(!matches!(e, Event::Failed { .. }), "{e:?}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        engine.send(Cmd::Play {
            id: 1,
            path: first,
            gain: 1.0,
        });
        assert!(wait(&|e| matches!(e, Event::Started { id: 1 }), 5));
        let duration = engine.duration_ms();
        assert!(duration > 5000, "{duration}");
        engine.send(Cmd::Preload(Some((2, second, 1.0, 0))));
        engine.send(Cmd::Seek(duration - 1500));
        // The second starts within the second and a half left, and the
        // first does not end on its own.
        assert!(wait(&|e| matches!(e, Event::Started { id: 2 }), 4));
        let position = engine.position_ms();
        assert!(position < 1500, "the second began at {position} ms");
        let duration = engine.duration_ms();
        engine.send(Cmd::Seek(duration - 1000));
        assert!(wait(&|e| matches!(e, Event::Ended), 4));
    }

    /// A crossfade of 2 s: with 4 s of the first track left, the second
    /// is heard (`Started`) about 2 s later, not 4, from its start; the
    /// first never ends on its own. `METALAMP_SAMPLE` as above.
    #[test]
    #[ignore = "needs METALAMP_SAMPLE and a sound device; plays a few seconds"]
    fn engine_crossfades() {
        use std::time::Instant;
        let first = PathBuf::from(std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE"));
        let mut files: Vec<PathBuf> = std::fs::read_dir(first.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| crate::filetypes::is_audio(p))
            .collect();
        files.sort();
        let second = files
            .iter()
            .skip_while(|p| **p != first)
            .nth(1)
            .cloned()
            .expect("a second file in the folder");
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.05);
        engine.send(Cmd::Play {
            id: 1,
            path: first,
            gain: 1.0,
        });
        let until = Instant::now() + Duration::from_secs(5);
        while engine.duration_ms() == 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        let duration = engine.duration_ms();
        assert!(duration > 10_000, "{duration}");
        engine.send(Cmd::Preload(Some((2, second, 1.0, 2000))));
        engine.send(Cmd::Seek(duration - 4000));
        // Let the seek take effect before the clock starts.
        std::thread::sleep(Duration::from_millis(200));
        let _ = engine.poll();
        let from = Instant::now();
        let mut started = None;
        while from.elapsed() < Duration::from_secs(6) && started.is_none() {
            for e in engine.poll() {
                assert!(!matches!(e, Event::Ended | Event::Failed { .. }), "{e:?}");
                if matches!(e, Event::Started { id: 2 }) {
                    started = Some((from.elapsed(), engine.position_ms()));
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let (after, position) = started.expect("the second track started");
        println!(
            "the second heard {} ms after, at {position} ms",
            after.as_millis()
        );
        assert!(
            (1300..2700).contains(&after.as_millis()),
            "{} ms",
            after.as_millis()
        );
        assert!(position < 1000, "{position}");
        engine.send(Cmd::Stop);
    }

    /// A track chosen while paused plays (the pause used to stay on in the
    /// engine: silence, the UI saying "playing"). `METALAMP_SAMPLE` as
    /// above.
    #[test]
    #[ignore = "needs METALAMP_SAMPLE and a sound device; plays a few seconds"]
    fn play_while_paused_plays() {
        use std::time::Instant;
        let file = PathBuf::from(std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE"));
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.05);
        let wait = |what: &dyn Fn(&Event) -> bool, secs: u64| {
            let until = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < until {
                for e in engine.poll() {
                    println!("{e:?} at {} ms", engine.position_ms());
                    if what(&e) {
                        return true;
                    }
                    assert!(!matches!(e, Event::Failed { .. }), "{e:?}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        };
        engine.send(Cmd::Play {
            id: 1,
            path: file.clone(),
            gain: 1.0,
        });
        assert!(wait(&|e| matches!(e, Event::Started { id: 1 }), 5));
        std::thread::sleep(Duration::from_millis(500));
        engine.send(Cmd::Pause);
        std::thread::sleep(Duration::from_millis(300));
        engine.send(Cmd::Play {
            id: 2,
            path: file,
            gain: 1.0,
        });
        assert!(wait(&|e| matches!(e, Event::Started { id: 2 }), 5));
        std::thread::sleep(Duration::from_millis(800));
        let position = engine.position_ms();
        assert!(
            position > 300,
            "the position stands at {position} ms: still paused"
        );
    }
}
