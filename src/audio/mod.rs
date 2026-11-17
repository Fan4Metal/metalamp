//! Playback on a thread of its own ("audio"): Media Foundation decodes
//! (`decoder.rs`), the samples wait in a ring of about a second, and an
//! [`output::Output`] (WASAPI, `output.rs`) plays them. The UI sends
//! [`Cmd`]s and takes [`Event`]s; the position, the duration and the gain
//! are shared atomics, read and set without waiting for the thread.
//!
//! Files are opened, and opened again at a place sought, on an "open"
//! thread (`opener`), which hands the decoders over and reads the files
//! through into Windows' cache: the audio thread keeps the device only
//! `WRITE_AHEAD` written, and a disk or Media Foundation's first read
//! after a seek (up to 90 ms) would leave it dry.
//!
//! Gapless: the UI names the track after the one playing (`Cmd::Preload`);
//! the thread has it opened at once and, when the playing one's decoder ends,
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
    /// is its loudness gain (1 when not known); `fade_ms`, a crossfade
    /// into it from the track heard (0: none, the sound before cut).
    Play {
        id: u64,
        path: PathBuf,
        gain: f32,
        fade_ms: u64,
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
    /// The crossfade of a seek from now on (ms; 0: none, the place left
    /// fades out and the one sought fades in).
    SeekFade(u64),
    /// Play on this device (its id; empty: the default) from now on.
    Device(String),
    /// The files at these paths closed where they are being decoded (the
    /// track heard, the next one, the one a fade leaves), their tags to be
    /// written; `done` is told whether any was. What is in the ring goes
    /// on being heard.
    Release {
        paths: Vec<PathBuf>,
        done: Sender<bool>,
    },
    /// The files released opened again, each from where it was.
    Reacquire,
    /// From the "open" thread: the file asked for with `ticket` opened.
    Opened { ticket: u64, decoder: Opened },
}

/// A decoder opened on the "open" thread for the audio thread: both are in
/// COM's multithreaded apartment, where a Media Foundation object may be
/// used from any thread (by one at a time: it is handed over).
pub struct Opened(windows::core::Result<Decoder>);

// SAFETY: the source reader inside lives in the MTA (see above) and is
// only ever used by the thread holding this.
unsafe impl Send for Opened {}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_ok() { "Ok" } else { "Err" })
    }
}

/// A track asked for (`Cmd::Play`, `Cmd::Preload`) while its file is
/// opened on the "open" thread.
struct Asked {
    ticket: u64,
    id: u64,
    path: PathBuf,
    gain: f32,
    fade_ms: u64,
}

/// A seek in the track heard waiting for its file to be opened again at
/// the place sought (`Player::seek`).
struct SeekAsked {
    ticket: u64,
    id: u64,
    ms: u64,
}

/// What a file is opened for on the "open" thread.
enum Ask {
    /// To play: it goes before the files still being read through.
    Play,
    /// The next track.
    Next,
    /// The track heard, at this place (ms), its first block read: only
    /// the last of the seeks waiting is opened.
    Seek(u64),
}

/// The most of a file read through into Windows' cache (`opener`).
const WARM_MOST: u64 = 512 << 20;

/// The "open" thread: the files to play opened away from the audio
/// thread, whose turns must come on time (a file on a hard disk took up
/// to 50 ms to open, measured; the first read after a seek up to 90 ms),
/// each handed over (`Cmd::Opened`), then read through so that Windows
/// keeps it in its cache, and decoding it reads nothing from the disk. A
/// track to play goes before the files still being read: those are of
/// the tracks it replaces. Ends when the audio thread does.
fn opener(asks: &Receiver<(u64, PathBuf, Ask)>, tx: &Sender<Cmd>) {
    use std::io::Read;
    use std::sync::mpsc::TryRecvError;
    let _com = mf::com_init_mta();
    let _mf = match mf::Mf::start() {
        Ok(mf) => mf,
        Err(e) => {
            log::error!("Media Foundation: {}", mf::describe(&e));
            return;
        }
    };
    let mut queue: VecDeque<(u64, PathBuf, Ask)> = VecDeque::new();
    // Each file opened when its turn comes, so the ones waiting are not
    // held open (a tagger may want one to itself).
    let mut warming: VecDeque<(PathBuf, Option<std::fs::File>, u64)> = VecDeque::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if queue.is_empty() && warming.is_empty() {
            match asks.recv() {
                Ok(ask) => queue.push_back(ask),
                Err(_) => return,
            }
        }
        loop {
            match asks.try_recv() {
                Ok(ask) => queue.push_back(ask),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        // A held key seeks every few tens of milliseconds: only the last
        // place is wanted.
        if let Some(last) = queue.iter().rposition(|a| matches!(a.2, Ask::Seek(_))) {
            let mut at = 0;
            queue.retain(|a| {
                at += 1;
                !matches!(a.2, Ask::Seek(_)) || at - 1 == last
            });
        }
        if let Some((ticket, path, ask)) = queue.pop_front() {
            let decoder = Decoder::open(&path).and_then(|mut d| {
                if let Ask::Seek(ms) = ask {
                    d.seek(ms as i64 * SECOND / 1000)?;
                    d.prime()?;
                }
                Ok(d)
            });
            if tx
                .send(Cmd::Opened {
                    ticket,
                    decoder: Opened(decoder),
                })
                .is_err()
            {
                return;
            }
            match ask {
                Ask::Play => {
                    warming.clear();
                    warming.push_back((path, None, 0));
                }
                Ask::Next => warming.push_back((path, None, 0)),
                Ask::Seek(_) => {}
            }
            continue;
        }
        let Some((path, file, read)) = warming.front_mut() else {
            continue;
        };
        if file.is_none() {
            *file = std::fs::File::open(&*path).ok();
        }
        match file.as_mut().map(|f| f.read(&mut buf)) {
            Some(Ok(n)) if n > 0 && *read < WARM_MOST => *read += n as u64,
            _ => {
                warming.pop_front();
            }
        }
    }
}

/// What the audio thread tells the UI.
#[derive(Debug)]
pub enum Event {
    /// The track `id` is now heard (asked for, or the preloaded one after
    /// the last one ended).
    Started { id: u64 },
    /// The last track, `id`, ended and nothing was preloaded.
    Ended { id: u64 },
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
    /// The least the device held while playing (ms; `u64::MAX` none
    /// yet): how near the thread's turns came to a gap. For the tests.
    pub least_held_ms: AtomicU64,
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
        shared.least_held_ms.store(u64::MAX, Relaxed);
        let worker = Arc::clone(&shared);
        let back = tx.clone();
        let thread = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(&rx, back, &etx, &worker, &ctx, device, dsp))
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

    /// As `heard_samples`, with the side of the same frames into `side`.
    pub fn heard_stereo(
        &self,
        n: usize,
        out: &mut Vec<f32>,
        side: &mut Vec<f32>,
    ) -> Option<vis::Heard> {
        self.shared
            .vis
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .latest_stereo(n, out, side)
    }

    /// The gain of what is heard, taken at the audio thread's next turn
    /// (within 10 ms while playing).
    pub fn set_gain(&self, gain: f32) {
        self.shared.gain.store(gain.to_bits(), Relaxed);
    }

    /// For another thread to have files closed while it writes them.
    pub fn releaser(&self) -> Releaser {
        Releaser(self.tx.clone())
    }
}

/// Has the audio thread close the files being decoded while their tags
/// are written: the decoder holds them open for reading only, and Windows
/// refuses the write. The ring, about a second deep, covers the time.
#[derive(Clone)]
pub struct Releaser(Sender<Cmd>);

impl Releaser {
    /// The files at `paths` closed while the guard lives, opened again
    /// when it is dropped; none when none of them was open. Without an
    /// answer within a second the guard is given anyway: the thread may
    /// release them later, and the commands come in order.
    pub fn hold(&self, paths: Vec<PathBuf>) -> Option<Held> {
        let (done, answer) = channel();
        self.0.send(Cmd::Release { paths, done }).ok()?;
        match answer.recv_timeout(Duration::from_secs(1)) {
            Ok(false) => None,
            _ => Some(Held(self.0.clone())),
        }
    }
}

/// Files released (`Releaser::hold`), opened again on drop.
pub struct Held(Sender<Cmd>);

impl Drop for Held {
    fn drop(&mut self) {
        let _ = self.0.send(Cmd::Reacquire);
    }
}

/// The same file, its path's case aside, as Windows has it.
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
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
    /// What goes on decoding the sound ending after what `a` holds; none
    /// when only `a` is left of it (a manual change in the middle of
    /// another fade, or after the last track's end was decoded).
    from: Option<Open>,
    /// The id and loudness gain of the sound ending.
    id: u64,
    gain: f32,
    /// Its length in frames, and the frames mixed so far.
    len: u64,
    done: u64,
    /// Decoded and not yet mixed, of each.
    a: VecDeque<f32>,
    b: VecDeque<f32>,
    a_ended: bool,
    shape: Shape,
}

/// How a fade goes from one sound to the other.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// Both heard together, at equal power (`fade_gains`).
    Cross,
    /// The one fading out over the first half, the other fading in over
    /// the second (`dip_gains`): a seek without the crossfade.
    Dip,
}

/// The gains of the sound left and the one coming at `t` (0 to 1) of a
/// dip: each `fade_shape` over its half, silence between.
pub fn dip_gains(t: f32) -> (f32, f32) {
    if t < 0.5 {
        (fade_shape(1.0 - 2.0 * t), 0.0)
    } else {
        (0.0, fade_shape(2.0 * t - 1.0))
    }
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
    /// A place sought in the track heard: shown as the position until it
    /// is heard, so seeks one after another go on from it.
    sought: bool,
}

/// How far ahead of what is heard the device is kept written, in
/// seconds: the processing is applied at write time, so a change of its
/// settings is heard this soon, and a seek, a pause or another track
/// after it (what the device holds plays out). The device's buffer is
/// larger (`output::BUFFER`); the ring in memory, a second deep, covers
/// the decoding, and this only the thread's own turns: it polls every
/// 10 ms at Pro Audio priority with the timer at 1 ms, writes between
/// the blocks it decodes, and leaves opening files and seeking in them
/// to the "open" thread (`opener`). Measured on a hard disk: at least 30
/// of the 60 ms held (`engine_keeps_the_device_written`); it was 150 ms.
const WRITE_AHEAD: f32 = 0.06;

/// How long what is heard fades to silence before it is cut (a pause, a
/// stop, a seek, another track) and back in after (a resume, the place
/// sought), in seconds: a wave cut where it stands is a click. The fades
/// are in the samples written: the stream's volume would act at once, but
/// the audio engine applies it once a period (10 ms), a step, and a 30 ms
/// fade of it was three clicks (recorded through VB-Audio's cable).
const FADE_S: f32 = 0.03;

/// The longest wait for a fade out written to be heard.
const FADE_WAIT: Duration = Duration::from_millis(500);

/// The gain of a fade at `x` (0: silent, 1: whole): a raised cosine, so
/// neither end is a corner.
pub fn fade_shape(x: f32) -> f32 {
    0.5 - 0.5 * (x.clamp(0.0, 1.0) * std::f32::consts::PI).cos()
}

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
    /// Zeros still needed to emit the fixed DSP delay at the final end.
    /// None until draining begins; Some(0) prevents a second flush.
    tail_remaining: Option<usize>,
    drained_since: Option<Instant>,
    checked_device: Instant,
    /// The device wanted (its id; empty: the default).
    device: String,
    dsp: dsp::DspSettings,
    /// The processing for the output's format, made with it.
    chain: Option<dsp::Chain>,
    /// Files are released (`Cmd::Release`): the device is written as far
    /// ahead as its buffer goes, so opening them again (300 ms for an
    /// AAC, `Decoder::reacquire`) is not heard as a gap.
    holding: bool,
    /// The crossfade of a seek (ms; 0: none).
    seek_fade_ms: u64,
    /// The "open" thread's requests (`opener`): a ticket, the path,
    /// what for.
    opener: Sender<(u64, PathBuf, Ask)>,
    /// The last ticket given.
    ticket: u64,
    /// The track to play and the next one while their files are opened.
    play_asked: Option<Asked>,
    preload_asked: Option<Asked>,
    /// A seek in the track heard while it is opened again there.
    seek_asked: Option<SeekAsked>,
    /// The commands after a Play, waiting for its file to be opened.
    deferred: VecDeque<Cmd>,
    /// A fade in under way over the frames written next: how many of it
    /// are written (a resume, a place sought within a track; a track's
    /// own start is not faded).
    fade_in: Option<u64>,
}

fn run(
    rx: &Receiver<Cmd>,
    back: Sender<Cmd>,
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
    let (opener_tx, asks) = channel();
    if let Err(e) = std::thread::Builder::new()
        .name("open".into())
        .spawn(move || opener(&asks, &back))
    {
        // Opened on this thread then (`Player::ask`).
        log::error!("cannot start the open thread: {e}");
    }
    run_player(rx, opener_tx, events, shared, ctx, device, dsp);
    // Decoders handed over and not taken, released while Media Foundation
    // is up here.
    while rx.try_recv().is_ok() {}
}

fn run_player(
    rx: &Receiver<Cmd>,
    opener: Sender<(u64, PathBuf, Ask)>,
    events: &Sender<Event>,
    shared: &Shared,
    ctx: &egui::Context,
    device: String,
    dsp: dsp::DspSettings,
) {
    let mut p = Player {
        opener,
        ticket: 0,
        play_asked: None,
        preload_asked: None,
        seek_asked: None,
        deferred: VecDeque::new(),
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
        tail_remaining: None,
        drained_since: None,
        checked_device: Instant::now(),
        device,
        dsp,
        chain: None,
        holding: false,
        fade_in: None,
        seek_fade_ms: 0,
    };
    let mut timer = FineTimer(false);
    loop {
        let busy = p.decoding.is_some() && !p.paused;
        timer.set(busy);
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

/// Windows' timer at 1 ms while sound plays, so the audio thread's waits
/// of 10 ms last 10 ms, not up to 15.6 + 10: the device holds little
/// (`WRITE_AHEAD`). Back to Windows' own when nothing plays (paused,
/// stopped) and when dropped: the finer timer wakes the processor more
/// often, and a program that only sits there should not cost power.
struct FineTimer(bool);

impl FineTimer {
    fn set(&mut self, on: bool) {
        if on != self.0 {
            self.0 = on;
            unsafe {
                if on {
                    windows::Win32::Media::timeBeginPeriod(1);
                } else {
                    windows::Win32::Media::timeEndPeriod(1);
                }
            };
        }
    }
}

impl Drop for FineTimer {
    fn drop(&mut self) {
        self.set(false);
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
        // A track to play is being opened: the commands after it wait, in
        // order, for it to begin; another Play or a Stop makes it moot,
        // and the seeks, pauses and resumes meant for it with it.
        if self.play_asked.is_some() {
            match &cmd {
                Cmd::Opened { .. } => {}
                Cmd::Play { .. } | Cmd::Stop => {
                    self.play_asked = None;
                    self.deferred
                        .retain(|c| !matches!(c, Cmd::Seek(_) | Cmd::Pause | Cmd::Resume));
                    self.run_deferred();
                }
                _ => {
                    self.deferred.push_back(cmd);
                    return;
                }
            }
        }
        match cmd {
            Cmd::Play {
                id,
                path,
                gain,
                fade_ms,
            } => {
                // What was preloaded came after the track played so far;
                // the one after this is preloaded anew once it starts.
                self.next = None;
                self.preload_asked = None;
                self.seek_asked = None;
                self.ask(
                    Asked {
                        ticket: 0,
                        id,
                        path,
                        gain,
                        fade_ms,
                    },
                    true,
                );
            }
            Cmd::Opened { ticket, decoder } => self.opened(ticket, decoder.0),
            Cmd::Preload(None) => {
                self.preload_asked = None;
                self.next = None;
                // Stop after this one: one already behind it in the ring
                // is taken back if it can be.
                if self.take_back_next(None) {
                    self.eos = true;
                }
            }
            Cmd::Preload(Some((id, path, gain, fade_ms))) => {
                if self.next.as_ref().is_some_and(|n| n.id == id)
                    || self.preload_asked.as_ref().is_some_and(|a| a.id == id)
                {
                    return;
                }
                self.ask(
                    Asked {
                        ticket: 0,
                        id,
                        path,
                        gain,
                        fade_ms,
                    },
                    false,
                );
            }
            Cmd::Pause => {
                self.fade_out();
                self.paused = true;
                // The wait for the last frames to be heard starts again
                // after the pause, not counting it.
                self.drained_since = None;
                if let Some(out) = &mut self.out {
                    let _ = out.stop();
                }
            }
            Cmd::Resume => {
                let paused = std::mem::replace(&mut self.paused, false);
                if self.started && paused {
                    // What was written before the pause has been heard
                    // (`fade_out`): what comes next fades in.
                    self.fade_in = Some(0);
                    if let Some(out) = &mut self.out {
                        let _ = out.start();
                    }
                }
            }
            Cmd::Stop => self.stop(),
            Cmd::Seek(ms) => self.seek(ms),
            Cmd::SeekFade(ms) => self.seek_fade_ms = ms,
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
                    && fade.id == id
                {
                    fade.gain = gain;
                    if let Some(from) = &mut fade.from {
                        from.gain = gain;
                    }
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
            Cmd::Release { paths, done } => {
                let mut any = false;
                for open in self.opens() {
                    if !open.decoder.released() && paths.iter().any(|p| same_file(p, &open.path)) {
                        log::debug!("audio: {} released", open.path.display());
                        open.decoder.release();
                        any = true;
                    }
                }
                self.holding |= any;
                let _ = done.send(any);
            }
            Cmd::Reacquire => self.reacquire(),
        }
    }

    /// `asked`'s file to be opened on the "open" thread, as the track to
    /// play (`play`) or the next one; opened here when that thread is
    /// gone.
    fn ask(&mut self, mut asked: Asked, play: bool) {
        self.ticket += 1;
        asked.ticket = self.ticket;
        let path = asked.path.clone();
        if play {
            self.play_asked = Some(asked);
        } else {
            self.preload_asked = Some(asked);
        }
        let ask = if play { Ask::Play } else { Ask::Next };
        if let Err(e) = self.opener.send((self.ticket, path, ask)) {
            let (ticket, path, _) = e.0;
            self.opened(ticket, Decoder::open(&path));
        }
    }

    /// The file asked for with `ticket` opened: the track to play begins
    /// (and the commands after it run), the next one is preloaded. A file
    /// no longer wanted is dropped.
    fn opened(&mut self, ticket: u64, decoder: windows::core::Result<Decoder>) {
        if let Some(a) = self.play_asked.take_if(|a| a.ticket == ticket) {
            match decoder {
                Ok(decoder) => {
                    let open = Open::new(a.id, a.path, decoder, a.gain, 0);
                    if let Some(open) = self.cross_into(open, a.fade_ms, 0, false, Shape::Cross) {
                        self.begin(open, 0);
                    }
                }
                Err(e) => {
                    // Not the track before going on meanwhile.
                    self.stop();
                    self.send(Event::Failed {
                        id: a.id,
                        message: mf::describe(&e),
                    });
                }
            }
            self.run_deferred();
        } else if let Some(s) = self.seek_asked.take_if(|s| s.ticket == ticket) {
            self.sought(s, decoder);
        } else if let Some(a) = self.preload_asked.take_if(|a| a.ticket == ticket) {
            match decoder {
                Ok(decoder) => {
                    let id = a.id;
                    self.next = Some(Open::new(id, a.path, decoder, a.gain, a.fade_ms));
                    // Another one is next now (Play next in the last
                    // second of a track): the one gone on into the ring
                    // behind the track heard is taken back, and this one
                    // goes there.
                    if self.take_back_next(Some(id)) {
                        self.track_decoded();
                    }
                    // The track decoded to its end before the next was
                    // known (a short one, one with nothing in it, an entry
                    // added at the end): it goes on now.
                    if self.eos {
                        self.eos = false;
                        self.drained_since = None;
                        self.track_decoded();
                    }
                }
                Err(e) => {
                    self.next = None;
                    self.send(Event::Failed {
                        id: a.id,
                        message: mf::describe(&e),
                    });
                }
            }
        }
    }

    /// The commands that waited for a track to begin, in order (one of
    /// them may be another Play: those after it wait again).
    fn run_deferred(&mut self) {
        for cmd in std::mem::take(&mut self.deferred) {
            self.handle(cmd);
        }
    }

    /// The tracks open: the one decoding, the next one, the one a fade
    /// leaves.
    fn opens(&mut self) -> impl Iterator<Item = &mut Open> {
        self.decoding
            .iter_mut()
            .chain(self.next.iter_mut())
            .chain(self.fade.iter_mut().filter_map(|f| f.from.as_mut()))
    }

    /// A decoder released (`Cmd::Release`) waits: nothing more is decoded
    /// meanwhile.
    fn waiting(&self) -> bool {
        self.decoding.as_ref().is_some_and(|d| d.decoder.released())
            || self.fade.as_ref().is_some_and(|f| {
                !f.a_ended && f.from.as_ref().is_some_and(|o| o.decoder.released())
            })
    }

    /// The files released opened again, each going on from the frame it
    /// had decoded. One that cannot be (removed, or now another format)
    /// ends there: the track decoding as at its end, the next one as a
    /// failed preload, the one a fade leaves in silence.
    fn reacquire(&mut self) {
        self.holding = false;
        let mut failed = (false, None, false);
        for (slot, open) in self
            .decoding
            .iter_mut()
            .map(|o| (0, o))
            .chain(self.next.iter_mut().map(|o| (1, o)))
            .chain(
                self.fade
                    .iter_mut()
                    .filter_map(|f| f.from.as_mut().map(|o| (2, o))),
            )
        {
            if !open.decoder.released() {
                continue;
            }
            if let Err(e) = open.decoder.reacquire(&open.path, open.decoded) {
                log::warn!(
                    "cannot open {} again: {}",
                    open.path.display(),
                    mf::describe(&e)
                );
                match slot {
                    0 => failed.0 = true,
                    1 => failed.1 = Some((open.id, mf::describe(&e))),
                    _ => failed.2 = true,
                }
            }
        }
        if failed.2
            && let Some(fade) = &mut self.fade
        {
            fade.a_ended = true;
        }
        if let Some((id, message)) = failed.1 {
            self.next = None;
            self.send(Event::Failed { id, message });
        }
        if failed.0 {
            self.decoding = None;
            self.fade = None;
            self.track_decoded();
        }
    }

    /// The frames of a fade (`FADE_S`).
    fn fade_frames(&self) -> u64 {
        (self.format.0 as f32 * FADE_S) as u64
    }

    /// `block` (raw samples from the ring) processed, faded if needed,
    /// written to the device, then told to the visualizations.
    fn write_block(&mut self, block: Vec<f32>) -> windows::core::Result<()> {
        self.write_faded_block(block, false)
    }

    /// Playback fades act after DSP: applying a fade to its input would
    /// leave the last delayed frames unfaded when the device is stopped.
    fn write_faded_block(
        &mut self,
        mut block: Vec<f32>,
        fade_out: bool,
    ) -> windows::core::Result<()> {
        let channels = usize::from(self.format.1.max(1));
        let frames = block.len() / channels;
        if frames == 0 {
            return Ok(());
        }
        self.process(&mut block, frames);
        if let Some(done) = self.fade_in {
            let len = self.fade_frames().max(1);
            for (i, frame) in block.chunks_exact_mut(channels).enumerate() {
                let at = done + i as u64;
                if at >= len {
                    break;
                }
                let g = fade_shape(at as f32 / len as f32);
                frame.iter_mut().for_each(|x| *x *= g);
            }
            let done = done + frames as u64;
            self.fade_in = (done < len).then_some(done);
        }
        if fade_out {
            for (i, frame) in block.chunks_exact_mut(channels).enumerate() {
                let g = fade_shape(1.0 - (i + 1) as f32 / frames as f32);
                frame.iter_mut().for_each(|x| *x *= g);
            }
        }
        let Some(out) = self.out.as_mut() else {
            return Ok(());
        };
        out.write(&block)?;
        self.shared
            .vis
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(&block, channels, self.written);
        self.written += frames as u64;
        Ok(())
    }

    /// What is heard faded to silence before it is cut: the next
    /// `FADE_S` of the ring written fading out after what the device
    /// holds, and waited for until heard (at most `FADE_WAIT`; the thread
    /// waits, the commands with it); the ring goes on after them. Nothing
    /// when nothing is heard (not started, paused), and no fade when the
    /// ring is empty (the end of what was to be played).
    fn fade_out(&mut self) {
        if !self.started || self.paused || self.out.is_none() {
            return;
        }
        let channels = usize::from(self.format.1.max(1));
        let len = (self.fade_frames() as usize).min(self.ring.len() / channels);
        if len > 0 {
            let block: Vec<f32> = self.ring.drain(..len * channels).collect();
            if self.write_faded_block(block, true).is_err() {
                return;
            }
        }
        let began = Instant::now();
        while began.elapsed() < FADE_WAIT {
            match self.out.as_mut().map(|o| o.heard()) {
                Some(Ok(heard)) if heard < self.written => {}
                _ => break,
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn stop(&mut self) {
        self.seek_asked = None;
        self.fade_out();
        self.fade_in = None;
        self.decoding = None;
        self.next = None;
        self.fade = None;
        self.ring.clear();
        self.segments.clear();
        self.heard_id = None;
        self.eos = false;
        self.format_change = false;
        self.tail_remaining = None;
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
        // The sound before (another track, another place) fades out; a
        // place within the track fades in, its start is its own.
        self.fade_out();
        self.fade_in = (from_ms > 0).then_some(0);
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
            sought: false,
        });
        self.heard_id = None;
        self.decoding = Some(open);
        self.eos = false;
        self.format_change = false;
        self.tail_remaining = None;
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

    /// While the track heard is the one decoding, it is opened again at
    /// the place sought on the "open" thread (the first read after a seek
    /// takes Media Foundation up to 90 ms, more than the device holds),
    /// the place left playing on from the ring meanwhile (`sought`). Else
    /// (paused, not started, the next track already in the ring, files
    /// released) the track begins again there.
    fn seek(&mut self, ms: u64) {
        self.seek_asked = None;
        if self.started && !self.paused && !self.holding {
            let in_place = match (self.heard(), &self.decoding) {
                (Some(seg), Some(d)) if seg.id == d.id => Some((seg.id, seg.path.clone())),
                _ => None,
            };
            if let Some((id, path)) = in_place {
                self.ticket += 1;
                if self.opener.send((self.ticket, path, Ask::Seek(ms))).is_ok() {
                    self.seek_asked = Some(SeekAsked {
                        ticket: self.ticket,
                        id,
                        ms,
                    });
                    self.shared.position_ms.store(ms, Relaxed);
                    return;
                }
            }
        }
        self.restart_heard(ms);
    }

    /// The track heard opened again at the place sought (`seek`): it goes
    /// on in the stream after what the device holds, from the ring,
    /// crossfaded from the place left (`seek_fade_ms`) or after it has
    /// faded out (`Shape::Dip`); the device is neither waited for nor
    /// started again (each cost about as much as it holds, measured).
    /// Begun again there when that cannot be; nothing when the track is
    /// no longer heard.
    fn sought(&mut self, asked: SeekAsked, decoder: windows::core::Result<Decoder>) {
        let Some(decoding) = &self.decoding else {
            return;
        };
        if decoding.id != asked.id || self.heard().is_none_or(|s| s.id != asked.id) {
            return;
        }
        let (fade_ms, shape) = if self.seek_fade_ms > 0 {
            (self.seek_fade_ms, Shape::Cross)
        } else {
            ((2.0 * FADE_S * 1000.0) as u64, Shape::Dip)
        };
        match decoder {
            Ok(decoder) if !self.paused && !self.holding => {
                let mut open = Open::new(asked.id, decoding.path.clone(), decoder, decoding.gain, 0);
                open.decoded = asked.ms * u64::from(open.decoder.rate) / 1000;
                if self.cross_into(open, fade_ms, asked.ms, true, shape).is_none() {
                    return;
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("cannot open to seek: {}", mf::describe(&e)),
        }
        self.restart_heard(asked.ms);
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
        // Its start already told; a seek right after its Play (taking up
        // where the last run left off) comes before that, and the start is
        // still news: the player preloads the next track on it.
        let announced = self.heard_id == Some(id);
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
                self.heard_id = announced.then_some(id);
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

    /// The device written from the ring up to `WRITE_AHEAD` (its whole
    /// buffer while files are released), and started once it holds that
    /// much or, `last`, anything (the ring grows no more this turn).
    fn feed(&mut self, channels: usize, last: bool) -> windows::core::Result<()> {
        let Some(out) = self.out.as_mut() else {
            return Ok(());
        };
        // The device applies the gain to what it plays, so a change is
        // heard at once, not after the buffer written ahead.
        out.set_gain(f32::from_bits(self.shared.gain.load(Relaxed)))?;
        let room = out.room()? as usize;
        let held = (out.buffer_frames() as usize).saturating_sub(room);
        if self.started && !self.holding && !self.eos && !self.format_change {
            let ms = held as u64 * 1000 / u64::from(self.format.0.max(1));
            self.shared.least_held_ms.fetch_min(ms, Relaxed);
        }
        let ahead = if self.holding {
            out.buffer_frames() as usize
        } else {
            (self.format.0 as f32 * WRITE_AHEAD) as usize
        };
        let available = room.min(ahead.saturating_sub(held));
        let mut frames = available.min(self.ring.len() / channels);
        if frames > 0 {
            let block: Vec<f32> = self.ring.drain(..frames * channels).collect();
            self.write_block(block)?;
        }
        // Only the final end (or a format change) needs zeros. Between
        // gapless tracks the next track itself carries the delay out.
        if self.ring.is_empty() && (self.eos || self.format_change) {
            let delay = self.chain.as_ref().map_or(0, |c| c.latency_frames());
            let remaining = self.tail_remaining.get_or_insert(delay);
            let tail = (*remaining).min(available - frames);
            *remaining -= tail;
            if tail > 0 {
                self.write_block(vec![0.0; tail * channels])?;
                frames += tail;
            }
        }
        if !self.started && self.written > 0 && (last || held + frames >= ahead) {
            self.out.as_mut().expect("written to").start()?;
            self.started = true;
        }
        Ok(())
    }

    /// Decode, write, follow the clock; the next track at the end.
    fn step(&mut self) -> windows::core::Result<()> {
        if self.decoding.is_none() && self.ring.is_empty() && !self.eos && !self.format_change {
            return Ok(());
        }
        let Some(out) = self.out.as_ref() else {
            return Ok(());
        };
        let target = out.buffer_frames() as usize * 2;
        let channels = usize::from(self.format.1.max(1));
        if !self.paused {
            let target = target * channels;
            // The device written between the blocks decoded: a block read
            // from the disk may take tens of milliseconds, and the device
            // holds little (`WRITE_AHEAD`).
            self.feed(channels, false)?;
            while self.ring.len() < target && !self.eos && !self.format_change {
                // A file released while its tag is written: the ring
                // plays on until it is opened again.
                if self.waiting() {
                    break;
                }
                if self.fade.is_some() {
                    self.fade_step(channels);
                } else if self.start_fade(channels) {
                    continue;
                } else {
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
                self.feed(channels, false)?;
            }
            self.feed(channels, true)?;
        }

        let out = self.out.as_mut().expect("checked above");
        let output_heard = out.heard()?;
        self.vis().set_heard(output_heard, self.format.0);
        let delay = self.chain.as_ref().map_or(0, |c| c.latency_frames()) as u64;
        let heard = output_heard.min(self.written).saturating_sub(delay);
        while self.segments.len() > 1 && self.segments[1].start <= heard {
            self.segments.pop_front();
        }
        if let Some(seg) = self.segments.front() {
            let rate = u64::from(self.format.0.max(1));
            // A place sought and not heard yet is the position meanwhile.
            let sought = self
                .seek_asked
                .as_ref()
                .filter(|s| s.id == seg.id)
                .map(|s| s.ms)
                .or_else(|| {
                    self.segments
                        .iter()
                        .rfind(|s| s.sought && s.start > heard)
                        .map(|s| s.origin_ms)
                });
            let position = sought
                .unwrap_or_else(|| seg.origin_ms + heard.saturating_sub(seg.start) * 1000 / rate);
            self.shared.position_ms.store(position, Relaxed);
            self.shared.duration_ms.store(seg.duration_ms, Relaxed);
            if output_heard >= delay && self.heard_id != Some(seg.id) {
                self.heard_id = Some(seg.id);
                let id = seg.id;
                self.send(Event::Started { id });
            }
        }

        // The end: everything written has been heard (or the clock stopped
        // short of it for a while).
        if (self.eos || self.format_change)
            && self.ring.is_empty()
            && self.tail_remaining == Some(0)
            && !self.paused
        {
            let since = *self.drained_since.get_or_insert_with(Instant::now);
            if output_heard >= self.written || since.elapsed() > Duration::from_millis(500) {
                self.drained_since = None;
                if self.format_change
                    && let Some(next) = self.next.take()
                {
                    self.begin(next, 0);
                } else {
                    let id = self.segments.back().map_or(0, |s| s.id);
                    self.decoding = None;
                    self.next = None;
                    self.vis().clear();
                    self.segments.clear();
                    self.heard_id = None;
                    self.eos = false;
                    self.format_change = false;
                    self.tail_remaining = None;
                    self.started = false;
                    if let Some(out) = &mut self.out {
                        let _ = out.reset();
                    }
                    self.send(Event::Ended { id });
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

    /// `open` crossfaded into over `fade_ms` from the sound heard, when it
    /// can be: something is heard (started, not paused), of `open`'s
    /// format, no file released. `open` starts at `origin_ms` of its own
    /// (a seek; its decoder sought already), and the track preloaded is
    /// kept when `keep_next` (a seek within the track heard: the player
    /// preloads anew only on a track's start). The sound ending is what is not yet
    /// written to the device (the ring), then the decoder it came from,
    /// so the change is heard after what the device holds (`WRITE_AHEAD`);
    /// with no decoder to go on (the ring holding another fade's mix, or
    /// the end of what was to be played), the fade is no longer than the
    /// ring. Else `open` is handed back, to be begun as usual.
    fn cross_into(
        &mut self,
        open: Open,
        fade_ms: u64,
        origin_ms: u64,
        keep_next: bool,
        shape: Shape,
    ) -> Option<Open> {
        let channels = usize::from(self.format.1.max(1));
        let ring = (self.ring.len() / channels) as u64;
        if fade_ms == 0
            || !self.started
            || self.paused
            || self.holding
            || self.out.is_none()
            || ring == 0
            || (open.decoder.rate, open.decoder.channels) != self.format
        {
            return Some(open);
        }
        let mixing = self.fade.is_some();
        // Not longer than half of what is left of the track, as between
        // tracks.
        let left = open.total_frames().saturating_sub(open.decoded);
        let mut len = (fade_ms * u64::from(self.format.0) / 1000).min(left / 2);
        // Only the ring to fade from: the fade no longer than it.
        let alone = mixing || self.eos || self.format_change || self.decoding.is_none();
        match shape {
            Shape::Cross if alone => len = len.min(ring),
            // Its fade out whole, or none: cut short, it would be a click.
            Shape::Dip if alone && len / 2 > ring => return Some(open),
            _ => {}
        }
        if len == 0 {
            return Some(open);
        }
        self.fade = None;
        let from = self.decoding.take().filter(|_| !mixing);
        // The sound the ring starts with; the tracks it would have gone
        // on to are not heard now.
        let (id, gain) = self
            .segments
            .iter()
            .rev()
            .find(|s| s.start <= self.written)
            .map_or((0, 1.0), |s| (s.id, s.gain));
        self.segments.retain(|s| s.start < self.written);
        self.segments.push_back(Segment {
            // A dip's sound sought comes after its silent middle.
            start: self.written
                + match shape {
                    Shape::Cross => 0,
                    Shape::Dip => len / 2,
                },
            id: open.id,
            path: open.path.clone(),
            origin_ms,
            duration_ms: (open.decoder.duration / (SECOND / 1000)) as u64,
            gain: open.gain,
            sought: keep_next,
        });
        log::debug!("audio: {shape:?} of {len} frames into {}", open.id);
        self.fade = Some(Fade {
            from,
            id,
            gain,
            len,
            done: 0,
            a: std::mem::take(&mut self.ring),
            b: VecDeque::new(),
            a_ended: false,
            shape,
        });
        self.decoding = Some(open);
        if !keep_next {
            self.next = None;
        }
        self.eos = false;
        self.format_change = false;
        self.tail_remaining = None;
        self.drained_since = None;
        None
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
            sought: false,
        });
        log::debug!("audio: crossfade of {left} frames into {}", next.id);
        self.fade = Some(Fade {
            id: from.id,
            gain: from.gain,
            from: Some(from),
            len: left,
            done: 0,
            a: VecDeque::new(),
            b: VecDeque::new(),
            a_ended: false,
            shape: Shape::Cross,
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
            let Some(from) = fade.from.as_mut() else {
                fade.a_ended = true;
                break;
            };
            match from.decoder.next() {
                Ok(Some(block)) => {
                    // Where it would be opened again (`reacquire`).
                    from.decoded += (block.len() / channels) as u64;
                    fade.a.extend(block);
                }
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
            fade.gain / next.gain
        } else {
            1.0
        };
        let mut n = 0;
        while fade.done + n < fade.len {
            let at = fade.done + n;
            let t = at as f32 / fade.len as f32;
            let (ga, gb) = match fade.shape {
                Shape::Cross => fade_gains(t),
                Shape::Dip => dip_gains(t),
            };
            // A dip's sound sought starts after its silent middle: none of
            // it is passed over.
            let takes_b = fade.shape == Shape::Cross || 2 * at >= fade.len;
            if takes_b && fade.b.len() < channels {
                break;
            }
            for _ in 0..channels {
                let a = fade.a.pop_front().unwrap_or(0.0);
                let b = if takes_b {
                    fade.b.pop_front().unwrap_or(0.0)
                } else {
                    0.0
                };
                self.ring.push_back(a * ga * ratio + b * gb);
            }
            n += 1;
        }
        fade.done += n;
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

    /// The track decoding, gone on into the ring after the one heard (the
    /// preload of before), dropped with its samples, unless it is `keep`:
    /// possible while none of them has been written to the device and no
    /// fade mixes it. True if it was.
    fn take_back_next(&mut self, keep: Option<u64>) -> bool {
        let channels = usize::from(self.format.1.max(1));
        let Some(decoding) = &self.decoding else {
            return false;
        };
        let start = match self.segments.back() {
            Some(seg)
                if self.segments.len() >= 2
                    && self.fade.is_none()
                    && seg.id == decoding.id
                    && Some(seg.id) != keep
                    && seg.origin_ms == 0
                    && seg.start >= self.written =>
            {
                seg.start
            }
            _ => return false,
        };
        let keep_samples = (start - self.written) as usize * channels;
        if keep_samples > self.ring.len() {
            return false;
        }
        log::debug!("audio: the next track taken back from the ring");
        self.ring.truncate(keep_samples);
        self.segments.pop_back();
        self.decoding = None;
        self.eos = false;
        self.tail_remaining = None;
        self.drained_since = None;
        true
    }

    /// The decoding track ended: the next one goes on in the same ring if
    /// it has the same format.
    fn track_decoded(&mut self) {
        let channels = u64::from(self.format.1.max(1));
        match self.next.take() {
            Some(next) if (next.decoder.rate, next.decoder.channels) == self.format => {
                self.tail_remaining = None;
                self.segments.push_back(Segment {
                    start: self.written + self.ring.len() as u64 / channels,
                    id: next.id,
                    path: next.path.clone(),
                    origin_ms: 0,
                    duration_ms: (next.decoder.duration / (SECOND / 1000)) as u64,
                    gain: next.gain,
                    sought: false,
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
        // The device before, when it still plays (the default changed).
        self.fade_out();
        self.out = None;
        if self.heard().is_some() {
            self.restart_heard(self.shared.position_ms.load(Relaxed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock controlled by the test, with no audio device or COM.
    struct RecordedOutput {
        samples: Arc<Mutex<Vec<f32>>>,
        clock: Arc<AtomicU64>,
    }
    impl Output for RecordedOutput {
        fn room(&mut self) -> windows::core::Result<u32> {
            Ok(8192)
        }
        fn write(&mut self, samples: &[f32]) -> windows::core::Result<()> {
            self.samples.lock().unwrap().extend_from_slice(samples);
            Ok(())
        }
        fn set_gain(&mut self, _gain: f32) -> windows::core::Result<()> {
            Ok(())
        }
        fn start(&mut self) -> windows::core::Result<()> {
            Ok(())
        }
        fn stop(&mut self) -> windows::core::Result<()> {
            Ok(())
        }
        fn reset(&mut self) -> windows::core::Result<()> {
            self.clock.store(0, Relaxed);
            Ok(())
        }
        fn heard(&mut self) -> windows::core::Result<u64> {
            Ok(self.clock.load(Relaxed))
        }
        fn buffer_frames(&self) -> u32 {
            8192
        }
        fn device_changed(&mut self) -> bool {
            false
        }
    }

    #[test]
    fn final_dsp_tail_and_gapless_segment_clock_are_preserved() {
        for format_change in [false, true] {
            let (events, seen) = channel();
            let (opener, _) = channel();
            let shared = Shared::default();
            let ctx = egui::Context::default();
            let samples = Arc::new(Mutex::new(Vec::new()));
            let clock = Arc::new(AtomicU64::new(0));
            let dsp = dsp::DspSettings::default();
            let chain = dsp::Chain::new(48000, 2, &dsp);
            let delay = chain.latency_frames();
            let original: Vec<f32> = (0..2000).map(|i| 0.2 * (i as f32 * 0.17).sin()).collect();
            let mut p = Player {
                events: &events,
                shared: &shared,
                ctx: &ctx,
                out: Some(Box::new(RecordedOutput {
                    samples: Arc::clone(&samples),
                    clock: Arc::clone(&clock),
                })),
                format: (48000, 2),
                decoding: None,
                next: None,
                fade: None,
                ring: original.iter().copied().collect(),
                written: 0,
                segments: [1, 2]
                    .into_iter()
                    .map(|id| Segment {
                        start: (id - 1) * 500,
                        id,
                        path: PathBuf::new(),
                        origin_ms: 0,
                        duration_ms: 10,
                        gain: 1.0,
                        sought: false,
                    })
                    .collect(),
                heard_id: None,
                paused: false,
                started: false,
                eos: !format_change,
                format_change,
                tail_remaining: None,
                drained_since: None,
                checked_device: Instant::now(),
                device: String::new(),
                dsp,
                chain: Some(chain),
                holding: false,
                seek_fade_ms: 0,
                opener,
                ticket: 0,
                play_asked: None,
                preload_asked: None,
                seek_asked: None,
                deferred: VecDeque::new(),
                fade_in: None,
            };
            p.step().unwrap();
            let captured = samples.lock().unwrap().clone();
            assert_eq!(captured.len(), original.len() + delay * 2);
            assert!(captured[..delay * 2].iter().all(|&x| x == 0.0));
            assert_eq!(&captured[delay * 2..], &original);
            assert_eq!(p.tail_remaining, Some(0));
            assert!(seen.try_recv().is_err());

            // Segment two is announced when its delayed first frame is
            // heard, and no flush is inserted at the shared boundary.
            clock.store(delay as u64 + 499, Relaxed);
            p.step().unwrap();
            assert!(matches!(seen.try_recv(), Ok(Event::Started { id: 1 })));
            clock.store(delay as u64 + 500, Relaxed);
            p.step().unwrap();
            assert!(matches!(seen.try_recv(), Ok(Event::Started { id: 2 })));
            assert_eq!(shared.position_ms.load(Relaxed), 0);
            assert_eq!(samples.lock().unwrap().len(), captured.len());
            clock.store(p.written, Relaxed);
            p.step().unwrap();
            assert!(matches!(seen.try_recv(), Ok(Event::Ended { id: 2 })));
            assert_eq!(samples.lock().unwrap().len(), captured.len());
            p.step().unwrap();
            p.step().unwrap();
            assert!(seen.try_recv().is_err());
            assert_eq!(samples.lock().unwrap().len(), captured.len());

            // Pause and resume fades reach silence at the device side even
            // while the chain holds nonzero samples in its fixed delay.
            p.write_block(vec![0.2; delay * 2]).unwrap();
            p.write_faded_block(vec![0.2; 2880], true).unwrap();
            let before_resume = samples.lock().unwrap().len();
            assert_eq!(samples.lock().unwrap()[before_resume - 1], 0.0);
            p.fade_in = Some(0);
            p.write_block(vec![0.2; 2880]).unwrap();
            let output = samples.lock().unwrap();
            assert_eq!(output[before_resume], 0.0);
            assert!((output[output.len() - 1] - 0.2).abs() < 1e-6);
        }
    }

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
    fn a_fade_has_no_corners() {
        assert_eq!(fade_shape(0.0), 0.0);
        assert_eq!(fade_shape(1.0), 1.0);
        assert!((fade_shape(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(fade_shape(-1.0), 0.0);
        assert_eq!(fade_shape(2.0), 1.0);
        // Flat at both ends, rising throughout.
        let d = 1e-3;
        assert!(fade_shape(d) < 1e-5 && 1.0 - fade_shape(1.0 - d) < 1e-5);
        for i in 0..100 {
            let x = i as f32 / 100.0;
            assert!(fade_shape(x + 0.01) > fade_shape(x));
        }
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

    /// A decoder released and opened again gives the samples it would have
    /// given: none lost or repeated where it was (`METALAMP_SAMPLE`).
    #[test]
    #[ignore = "needs METALAMP_SAMPLE, an audio file"]
    fn decoder_reacquires_where_it_was() {
        let path = PathBuf::from(std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE"));
        let _com = mf::com_init_mta();
        let _mf = mf::Mf::start().unwrap();
        let read = |d: &mut Decoder, frames: usize| {
            let channels = usize::from(d.channels);
            let mut out = Vec::new();
            while out.len() < frames * channels {
                match d.next().unwrap() {
                    Some(block) => out.extend(block),
                    None => break,
                }
            }
            out
        };
        let mut whole = Decoder::open(&path).unwrap();
        let rate = whole.rate as usize;
        let expected = read(&mut whole, rate * 12);
        let channels = usize::from(whole.channels);
        let mut d = Decoder::open(&path).unwrap();
        let mut got = read(&mut d, rate * 5);
        // Up to a block past 5 s: where it stopped.
        let frames = (got.len() / channels) as u64;
        d.release();
        assert!(d.released());
        let started = std::time::Instant::now();
        d.reacquire(&path, frames).unwrap();
        println!("opened again in {:?}", started.elapsed());
        got.extend(read(&mut d, rate * 5));
        let n = got.len().min(expected.len());
        assert!(n > (frames as usize + rate) * channels, "it went on");
        let at = frames as usize * channels;
        // An AAC decoder gives samples about 1 % off after a seek: there,
        // the second after the place close to what it would have been.
        let aac = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("m4a") || e.eq_ignore_ascii_case("aac"));
        if aac {
            let to = (at + rate * channels).min(n);
            let error: f32 = (at..to).map(|i| (got[i] - expected[i]).powi(2)).sum();
            let energy: f32 = (at..to).map(|i| expected[i].powi(2)).sum();
            println!("error {:.4} of the energy", error / energy);
            assert!(error < 0.05 * energy, "released after {frames} frames");
        } else {
            let first_wrong = (0..n).find(|&i| (got[i] - expected[i]).abs() > 1e-6);
            assert_eq!(first_wrong, None, "released after {frames} frames");
        }
    }

    /// A copy of `METALAMP_SAMPLE` playing (silent, gain 0) is released:
    /// it can then be opened for writing with no sharing at all, the
    /// position goes on from the ring meanwhile, and once it is opened
    /// again it plays on, without a failure or an end.
    #[test]
    #[ignore = "needs METALAMP_SAMPLE and a sound device; plays a few seconds silently"]
    fn engine_releases_a_file_written() {
        use std::os::windows::fs::OpenOptionsExt;
        let sample = PathBuf::from(std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE"));
        let dir = std::env::temp_dir().join(format!("metalamp_release_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join(sample.file_name().unwrap());
        std::fs::copy(&sample, &copy).unwrap();
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.0);
        engine.send(Cmd::Play {
            id: 1,
            path: copy.clone(),
            gain: 1.0,
            fade_ms: 0,
        });
        let events = |ms: u64| {
            let mut seen = Vec::new();
            let until = std::time::Instant::now() + Duration::from_millis(ms);
            while std::time::Instant::now() < until {
                seen.extend(engine.poll());
                std::thread::sleep(Duration::from_millis(10));
            }
            seen
        };
        let started = events(1500);
        assert!(
            matches!(started.as_slice(), [Event::Started { id: 1 }]),
            "{started:?}"
        );
        let exclusive = || {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&copy)
        };
        assert!(exclusive().is_err(), "the decoder holds the file");
        let releaser = engine.releaser();
        // Its name in another case, as a playlist may spell it.
        let upper = PathBuf::from(copy.to_string_lossy().to_uppercase());
        let held = releaser.hold(vec![upper]).expect("released");
        let before = engine.position_ms();
        drop(exclusive().expect("open for writing once released"));
        // Written meanwhile: the tag grows by 20 KB, the audio moves on in
        // the file.
        let mut changes = crate::library::edit::Changes::default();
        changes.main[crate::library::edit::Key::Comment as usize] = Some("x".repeat(20_000));
        crate::library::edit::write(&copy, &changes, Default::default()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let during = engine.position_ms();
        assert!(during > before, "the ring plays on: {before} → {during}");
        drop(held);
        let after = events(1500);
        assert!(after.is_empty(), "{after:?}");
        let end = engine.position_ms();
        assert!(end > during + 1000, "it plays on: {during} → {end}");
        assert!(releaser.hold(vec![dir.join("other.mp3")]).is_none());
        drop(engine);
        std::fs::remove_dir_all(&dir).unwrap();
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
            fade_ms: 0,
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
        assert!(wait(&|e| matches!(e, Event::Ended { .. }), 4));
    }

    /// Another next track asked for when the one preloaded has gone on
    /// into the ring (the last second of the first) is the one heard:
    /// the preloaded one is taken back. `METALAMP_SAMPLE`, played three
    /// times under other ids.
    #[test]
    #[ignore = "needs METALAMP_SAMPLE and a sound device; plays a few seconds"]
    fn engine_takes_back_the_next() {
        use std::time::Instant;
        let file = PathBuf::from(std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE"));
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.05);
        let started = |secs: u64| {
            let until = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < until {
                for e in engine.poll() {
                    println!("{e:?} at {} ms", engine.position_ms());
                    match e {
                        Event::Started { id } => return Some(id),
                        Event::Failed { .. } => panic!("{e:?}"),
                        _ => {}
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            None
        };
        engine.send(Cmd::Play {
            id: 1,
            path: file.clone(),
            gain: 1.0,
            fade_ms: 0,
        });
        assert_eq!(started(5), Some(1));
        let duration = engine.duration_ms();
        assert!(duration > 5000, "{duration}");
        engine.send(Cmd::Preload(Some((2, file.clone(), 1.0, 0))));
        // Its decoder ends at once: the ring holds about a second.
        engine.send(Cmd::Seek(duration - 900));
        std::thread::sleep(Duration::from_millis(200));
        engine.send(Cmd::Preload(Some((3, file, 1.0, 0))));
        assert_eq!(started(4), Some(3));
        engine.send(Cmd::Stop);
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
            fade_ms: 0,
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
                assert!(
                    !matches!(e, Event::Ended { .. } | Event::Failed { .. }),
                    "{e:?}"
                );
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
            fade_ms: 0,
        });
        assert!(wait(&|e| matches!(e, Event::Started { id: 1 }), 5));
        std::thread::sleep(Duration::from_millis(500));
        engine.send(Cmd::Pause);
        std::thread::sleep(Duration::from_millis(300));
        engine.send(Cmd::Play {
            id: 2,
            path: file,
            gain: 1.0,
            fade_ms: 0,
        });
        assert!(wait(&|e| matches!(e, Event::Started { id: 2 }), 5));
        std::thread::sleep(Duration::from_millis(800));
        let position = engine.position_ms();
        assert!(
            position > 300,
            "the position stands at {position} ms: still paused"
        );
    }

    /// A Play with a Seek right after it (taking up where the last run
    /// left off) is still `Started`: without it the player preloaded
    /// nothing and stopped at the track's end. `METALAMP_SAMPLE` as above.
    #[test]
    #[ignore = "needs METALAMP_SAMPLE and a sound device; plays a second"]
    fn play_and_seek_is_started() {
        use std::time::Instant;
        let file = PathBuf::from(std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE"));
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.05);
        engine.send(Cmd::Play {
            id: 1,
            path: file,
            gain: 1.0,
            fade_ms: 0,
        });
        engine.send(Cmd::Seek(10_000));
        let until = Instant::now() + Duration::from_secs(5);
        let mut started = false;
        while !started && Instant::now() < until {
            for e in engine.poll() {
                println!("{e:?} at {} ms", engine.position_ms());
                assert!(!matches!(e, Event::Failed { .. }), "{e:?}");
                started |= matches!(e, Event::Started { id: 1 });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        engine.send(Cmd::Stop);
        assert!(started, "no Started for a play sought at once");
    }

    /// A WAV of a sine of `hz` at half scale, 48 kHz stereo 16-bit, `secs`
    /// long, faded in and out over 20 ms (its own ends make no click).
    fn sine_wav(path: &std::path::Path, secs: u32, hz: f32) {
        tones_wav(path, secs, |_| hz);
    }

    /// As `sine_wav`, the tone's frequency at each second `hz_at` says.
    fn tones_wav(path: &std::path::Path, secs: u32, hz_at: impl Fn(u32) -> f32) {
        const RATE: u32 = 48_000;
        let frames = RATE * secs;
        let fade = RATE / 50;
        let mut data = Vec::with_capacity(frames as usize * 4);
        for i in 0..frames {
            let edge = i.min(frames - 1 - i);
            let env = (edge as f32 / fade as f32).min(1.0);
            let hz = hz_at(i / RATE);
            let x = 0.5 * env * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin();
            let s = (x * 32767.0) as i16;
            data.extend_from_slice(&s.to_le_bytes());
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(path, wav).unwrap();
    }

    /// The id of the active endpoint of `flow` whose name holds `part`.
    fn endpoint_named(flow: windows::Win32::Media::Audio::EDataFlow, part: &str) -> Option<String> {
        use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
        use windows::Win32::Media::Audio::{
            DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator,
        };
        use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};
        let _com = mf::com_init_mta();
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let all = enumerator
                .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
                .ok()?;
            for i in 0..all.GetCount().ok()? {
                let device = all.Item(i).ok()?;
                let name = device
                    .OpenPropertyStore(STGM_READ)
                    .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                if name.contains(part) {
                    let id = device.GetId().ok()?;
                    let text = id.to_string().ok();
                    windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as *const _));
                    return text;
                }
            }
            None
        }
    }

    /// What a capture device records (VB-Audio's virtual cable: what is
    /// played into its input comes out here), its first channel and rate.
    struct Recorder {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<(u32, Vec<f32>)>>,
    }

    impl Recorder {
        fn start(id: String) -> Recorder {
            let stop = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&stop);
            let (ready, started) = channel();
            let thread = std::thread::spawn(move || {
                let _com = mf::com_init_mta();
                capture(&id, &flag, &ready).expect("capture")
            });
            started.recv().expect("the capture started");
            Recorder {
                stop,
                thread: Some(thread),
            }
        }

        fn finish(mut self) -> (u32, Vec<f32>) {
            self.stop.store(true, Relaxed);
            self.thread.take().unwrap().join().unwrap()
        }
    }

    fn capture(
        id: &str,
        stop: &AtomicBool,
        ready: &Sender<()>,
    ) -> windows::core::Result<(u32, Vec<f32>)> {
        use windows::Win32::Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, IAudioCaptureClient,
            IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
        };
        use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDevice(&windows::core::HSTRING::from(id))?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let format = client.GetMixFormat()?;
            let rate = (*format).nSamplesPerSec;
            let channels = usize::from((*format).nChannels);
            let bits = (*format).wBitsPerSample;
            assert_eq!(bits, 32, "a float mix format");
            client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 2_000_000, 0, format, None)?;
            CoTaskMemFree(Some(format as *const _));
            let capture: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            let _ = ready.send(());
            let mut out = Vec::new();
            while !stop.load(Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
                while capture.GetNextPacketSize()? > 0 {
                    let mut data = std::ptr::null_mut();
                    let mut frames = 0u32;
                    let mut flags = 0u32;
                    capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                    let samples =
                        std::slice::from_raw_parts(data.cast::<f32>(), frames as usize * channels);
                    out.extend(
                        (0..frames as usize)
                            .map(|f| if silent { 0.0 } else { samples[f * channels] }),
                    );
                    capture.ReleaseBuffer(frames)?;
                }
            }
            client.Stop()?;
            Ok((rate, out))
        }
    }

    /// Where a captured sine bends sharply (a click): the second difference
    /// over `ratio` of the loudest sample (a 440 Hz sine's own is 0.0033 of
    /// it at 48 kHz), as (ms into the capture, its size over the loudest),
    /// one per 5 ms.
    fn clicks(rate: u32, x: &[f32], ratio: f32) -> Vec<(u64, f32)> {
        let top = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let mut found: Vec<(u64, f32)> = Vec::new();
        let gap = rate as usize / 200;
        let mut last = None;
        for n in 2..x.len() {
            let d = (x[n] - 2.0 * x[n - 1] + x[n - 2]).abs() / top.max(1e-9);
            if d > ratio {
                let ms = n as u64 * 1000 / u64::from(rate);
                match last {
                    Some(at) if n - at < gap => {
                        let e = found.last_mut().unwrap();
                        e.1 = e.1.max(d);
                    }
                    _ => found.push((ms, d)),
                }
                last = Some(n);
            }
        }
        found
    }

    /// Pause, resume, seek and stop heard without a click: a sine played
    /// into VB-Audio's virtual cable and recorded from its other end
    /// bends no sharper than a sine does. Nothing reaches the speakers.
    #[test]
    #[ignore = "needs VB-Audio Virtual Cable; plays a tone into it for about 4 s"]
    fn engine_pauses_without_a_click() {
        use windows::Win32::Media::Audio::{eCapture, eRender};
        let input = endpoint_named(eRender, "CABLE Input").expect("VB-Audio's CABLE Input");
        let output = endpoint_named(eCapture, "CABLE Output").expect("VB-Audio's CABLE Output");
        let dir = std::env::temp_dir().join(format!("metalamp_click_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sine.wav");
        sine_wav(&file, 60, 440.0);
        let engine = Engine::start(egui::Context::default(), input, dsp::DspSettings::default());
        // Silence into the cable while the recording starts.
        engine.set_gain(0.0);
        engine.send(Cmd::Play {
            id: 0,
            path: file.clone(),
            gain: 1.0,
            fade_ms: 0,
        });
        let recorder = Recorder::start(output);
        engine.send(Cmd::Stop);
        std::thread::sleep(Duration::from_millis(200));
        let t = std::time::Instant::now();
        engine.set_gain(0.5);
        engine.send(Cmd::Play {
            id: 1,
            path: file,
            gain: 1.0,
            fade_ms: 0,
        });
        let pause = |ms: u64| std::thread::sleep(Duration::from_millis(ms));
        pause(700);
        // (what, the position heard, ms since the play): where to look.
        let mut marks = Vec::new();
        let at = |what: &'static str, marks: &mut Vec<(&str, u64, u128)>| {
            marks.push((what, engine.position_ms(), t.elapsed().as_millis()));
        };
        at("pause", &mut marks);
        engine.send(Cmd::Pause);
        pause(400);
        at("resume", &mut marks);
        engine.send(Cmd::Resume);
        pause(600);
        at("seek", &mut marks);
        engine.send(Cmd::Seek(7_013));
        pause(600);
        at("stop", &mut marks);
        engine.send(Cmd::Stop);
        pause(300);
        drop(engine);
        let (rate, x) = recorder.finish();
        if let Ok(to) = std::env::var("METALAMP_CAPTURE") {
            let bytes: Vec<u8> = x.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(to, bytes).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
        println!("{marks:?}");
        println!(
            "captured {} ms at {rate} Hz",
            x.len() as u64 * 1000 / u64::from(rate)
        );
        let found = clicks(rate, &x, 0.01);
        for (ms, d) in &found {
            println!("a bend at {ms} ms: {d:.4} of the peak");
        }
        assert!(found.is_empty(), "{} clicks", found.len());
    }

    /// The amplitude of a sine of `hz` in `x` (a least-squares fit).
    fn amplitude(x: &[f32], rate: u32, hz: f32) -> f32 {
        let w = std::f32::consts::TAU * hz / rate as f32;
        let (mut s, mut c) = (0.0f32, 0.0f32);
        for (i, v) in x.iter().enumerate() {
            s += v * (w * i as f32).sin();
            c += v * (w * i as f32).cos();
        }
        2.0 * (s * s + c * c).sqrt() / x.len() as f32
    }

    /// A track chosen while another plays, with a crossfade of 1 s: the
    /// one before (440 Hz) fades as the new one (660 Hz) comes in, over
    /// about a second, both heard together meanwhile, without a bend
    /// sharper than the two sines make; the new one is `Started` soon
    /// (what the device holds), from its start. Played on the default
    /// device at gain 0: nothing is heard; the samples are the
    /// visualizations' tap, taken before the device's volume.
    #[test]
    #[ignore = "needs a sound device; plays silently for about 4 s"]
    fn engine_crossfades_into_a_track_chosen() {
        let dir = std::env::temp_dir().join(format!("metalamp_cross_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
        sine_wav(&a, 10, 440.0);
        sine_wav(&b, 10, 660.0);
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.0);
        engine.send(Cmd::Play {
            id: 1,
            path: a,
            gain: 1.0,
            fade_ms: 0,
        });
        std::thread::sleep(Duration::from_millis(1500));
        let asked = std::time::Instant::now();
        engine.send(Cmd::Play {
            id: 2,
            path: b,
            gain: 1.0,
            fade_ms: 1000,
        });
        let mut started = None;
        while asked.elapsed() < Duration::from_millis(2200) {
            for e in engine.poll() {
                assert!(
                    !matches!(e, Event::Ended { .. } | Event::Failed { .. }),
                    "{e:?}"
                );
                if matches!(e, Event::Started { id: 2 }) && started.is_none() {
                    started = Some((asked.elapsed(), engine.position_ms()));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut x = Vec::new();
        let heard = engine.heard_samples(48_000 * 3, &mut x).expect("heard");
        drop(engine);
        std::fs::remove_dir_all(&dir).unwrap();
        let (after, position) = started.expect("the new track started");
        println!("started {} ms after, at {position} ms", after.as_millis());
        assert!(after < Duration::from_millis(400), "{after:?}");
        assert!(position < 300, "{position}");
        assert_eq!(heard.rate, 48_000);
        // A bend: the two sines' own second difference stays under 0.006.
        let bends: Vec<usize> = (2..x.len())
            .filter(|&n| (x[n] - 2.0 * x[n - 1] + x[n - 2]).abs() > 0.02)
            .collect();
        assert!(
            bends.is_empty(),
            "bends at {:?}",
            &bends[..bends.len().min(10)]
        );
        // Each 50 ms (a whole number of periods of both, so neither leaks
        // into the other's measure): how much of each tone.
        let step = 2400;
        let levels: Vec<(f32, f32)> = x
            .chunks_exact(step)
            .map(|c| (amplitude(c, 48_000, 440.0), amplitude(c, 48_000, 660.0)))
            .collect();
        let first_b = levels
            .iter()
            .position(|l| l.1 > 0.05)
            .expect("660 Hz heard");
        let last_a = levels
            .iter()
            .rposition(|l| l.0 > 0.05)
            .expect("440 Hz heard");
        let both = levels.iter().filter(|l| l.0 > 0.15 && l.1 > 0.15).count();
        // The tones are over a tenth of their whole from 6 % to 94 % of
        // the fade: about 870 ms.
        let span = (last_a - first_b) * 50;
        println!("the fade: {span} ms, both over 0.15 for {} ms", both * 50);
        assert!((700..=1100).contains(&span), "{span} ms; {levels:.2?}");
        assert!(both * 50 >= 300, "{} ms", both * 50);
        let end = levels.last().unwrap();
        assert!(end.0 < 0.01 && end.1 > 0.45, "{end:?}");
    }

    /// A seek with the crossfade on: the place left (440 Hz) fades into
    /// the one sought (660 Hz, past the middle of the same file) over
    /// about 0.3 s without a bend, no `Started` for it (the same track),
    /// and the track preloaded is kept: a seek near the end goes on into
    /// it. Silent (gain 0, the visualizations' tap).
    #[test]
    #[ignore = "needs a sound device; plays silently for about 6 s"]
    fn engine_crossfades_a_seek() {
        let dir = std::env::temp_dir().join(format!("metalamp_seek_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
        tones_wav(&a, 20, |s| if s < 10 { 440.0 } else { 660.0 });
        sine_wav(&b, 5, 550.0);
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.0);
        engine.send(Cmd::SeekFade(300));
        engine.send(Cmd::Play {
            id: 1,
            path: a,
            gain: 1.0,
            fade_ms: 0,
        });
        engine.send(Cmd::Preload(Some((2, b, 1.0, 0))));
        let events = |ms: u64| {
            let until = std::time::Instant::now() + Duration::from_millis(ms);
            let mut seen = Vec::new();
            while std::time::Instant::now() < until {
                seen.extend(engine.poll());
                std::thread::sleep(Duration::from_millis(5));
            }
            seen
        };
        let first = events(1500);
        assert!(
            matches!(first.as_slice(), [Event::Started { id: 1 }]),
            "{first:?}"
        );
        engine.send(Cmd::Seek(15_000));
        let during = events(900);
        assert!(during.is_empty(), "{during:?}");
        let position = engine.position_ms();
        assert!((15_400..16_000).contains(&position), "{position}");
        let mut x = Vec::new();
        engine.heard_samples(48_000 * 2, &mut x).expect("heard");
        let bends: Vec<usize> = (2..x.len())
            .filter(|&n| (x[n] - 2.0 * x[n - 1] + x[n - 2]).abs() > 0.02)
            .collect();
        assert!(
            bends.is_empty(),
            "bends at {:?}",
            &bends[..bends.len().min(10)]
        );
        // Each 50 ms, as in the test above.
        let step = 2400;
        let levels: Vec<(f32, f32)> = x
            .chunks_exact(step)
            .map(|c| (amplitude(c, 48_000, 440.0), amplitude(c, 48_000, 660.0)))
            .collect();
        let first_b = levels
            .iter()
            .position(|l| l.1 > 0.05)
            .expect("660 Hz heard");
        let last_a = levels
            .iter()
            .rposition(|l| l.0 > 0.05)
            .expect("440 Hz heard");
        let both = levels.iter().filter(|l| l.0 > 0.15 && l.1 > 0.15).count();
        // About 260 ms of the 300 over a tenth, measured in 50 ms.
        let span = (last_a - first_b) * 50;
        println!("the fade: {span} ms, both over 0.15 for {} ms", both * 50);
        assert!((150..=400).contains(&span), "{span} ms; {levels:.2?}");
        assert!(both >= 2, "{levels:.2?}");
        // Near the end, the preloaded track follows.
        engine.send(Cmd::Seek(19_000));
        let end = events(2500);
        assert!(
            end.iter().any(|e| matches!(e, Event::Started { id: 2 })),
            "{end:?}"
        );
        assert!(
            !end.iter().any(|e| matches!(e, Event::Ended { .. })),
            "{end:?}"
        );
        drop(engine);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A seek without the crossfade: the place left (400 Hz) fades out,
    /// then the one sought (600 Hz) fades in, never both at once and
    /// without a bend; the position is the place sought at once, and no
    /// `Started` comes for it. Silent (gain 0, the visualizations' tap).
    #[test]
    #[ignore = "needs a sound device; plays silently for about 3 s"]
    fn engine_dips_into_a_seek() {
        let dir = std::env::temp_dir().join(format!("metalamp_dip_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.wav");
        // Whole periods in 10 ms, so neither leaks into the other's measure.
        tones_wav(&a, 20, |s| if s < 10 { 400.0 } else { 600.0 });
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.0);
        engine.send(Cmd::SeekFade(0));
        engine.send(Cmd::Play {
            id: 1,
            path: a,
            gain: 1.0,
            fade_ms: 0,
        });
        std::thread::sleep(Duration::from_millis(1500));
        let first = engine.poll();
        assert!(
            matches!(first.as_slice(), [Event::Started { id: 1 }]),
            "{first:?}"
        );
        engine.send(Cmd::Seek(15_000));
        std::thread::sleep(Duration::from_millis(20));
        let position = engine.position_ms();
        assert!((15_000..15_050).contains(&position), "{position}");
        std::thread::sleep(Duration::from_millis(600));
        let during = engine.poll();
        assert!(during.is_empty(), "{during:?}");
        let mut x = Vec::new();
        engine.heard_samples(48_000, &mut x).expect("heard");
        drop(engine);
        std::fs::remove_dir_all(&dir).unwrap();
        let bends: Vec<usize> = (2..x.len())
            .filter(|&n| (x[n] - 2.0 * x[n - 1] + x[n - 2]).abs() > 0.02)
            .collect();
        assert!(
            bends.is_empty(),
            "bends at {:?}",
            &bends[..bends.len().min(10)]
        );
        // Each 10 ms: how much of each tone.
        let step = 480;
        let levels: Vec<(f32, f32)> = x
            .chunks_exact(step)
            .map(|c| (amplitude(c, 48_000, 400.0), amplitude(c, 48_000, 600.0)))
            .collect();
        let last_a = levels
            .iter()
            .rposition(|l| l.0 > 0.05)
            .expect("400 Hz heard");
        let first_b = levels
            .iter()
            .position(|l| l.1 > 0.05)
            .expect("600 Hz heard");
        assert!(last_a < first_b, "{levels:.2?}");
        let end = levels.last().unwrap();
        assert!(end.0 < 0.01 && end.1 > 0.45, "{end:?}");
    }

    /// How soon a seek is heard: from the command to the place sought
    /// (600 Hz) at half its level in what the device's clock says is
    /// heard, with the crossfade off and on. Silent (gain 0). Measured:
    /// about 160 ms off, 220 on (265 and 305 when the device was kept
    /// written 150 ms ahead and a seek without the crossfade waited for it
    /// to be heard and started it again).
    #[test]
    #[ignore = "needs a sound device; plays silently for about 8 s"]
    fn engine_seek_latency() {
        let dir = std::env::temp_dir().join(format!("metalamp_lat_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.wav");
        tones_wav(&a, 30, |s| if s < 15 { 400.0 } else { 600.0 });
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.0);
        for fade in [0, 300, 0, 300] {
            engine.send(Cmd::SeekFade(fade));
            engine.send(Cmd::Play {
                id: 1,
                path: a.clone(),
                gain: 1.0,
                fade_ms: 0,
            });
            std::thread::sleep(Duration::from_millis(1200));
            let _ = engine.poll();
            let asked = std::time::Instant::now();
            engine.send(Cmd::Seek(20_000));
            let mut x = Vec::new();
            let mut at = None;
            while asked.elapsed() < Duration::from_millis(1500) {
                // The last 10 ms heard (480 frames: whole periods of both).
                if engine.heard_samples(480, &mut x).is_some()
                    && x.len() == 480
                    && amplitude(&x, 48_000, 600.0) > 0.5 * 0.5
                {
                    at = Some(asked.elapsed());
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            println!("crossfade {fade} ms: heard after {at:?}");
            let at = at.expect("the place sought heard");
            assert!(at < Duration::from_millis(250 + fade), "{at:?}");
            engine.send(Cmd::Stop);
            std::thread::sleep(Duration::from_millis(200));
        }
        drop(engine);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Real files of `METALAMP_MUSIC` (16 at random, likely not in
    /// Windows' cache), each played with the next preloaded, sought to its
    /// middle, then the next one played: the device never runs dry, the
    /// thread's turns coming while files open and are read from the disk.
    /// Silent. Measured on a hard disk: at least 30 ms of the 60 held; 0
    /// while the first read after a seek (up to 90 ms) was on the audio
    /// thread, 40 of 150 when files were opened there (50 ms for one).
    #[test]
    #[ignore = "needs METALAMP_MUSIC and a sound device; plays silently for about a minute"]
    fn engine_keeps_the_device_written() {
        let root = PathBuf::from(std::env::var("METALAMP_MUSIC").expect("METALAMP_MUSIC"));
        let mut files: Vec<PathBuf> = walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| e.into_path())
            .filter(|p| crate::filetypes::is_audio(p))
            .collect();
        assert!(files.len() >= 16, "{} files", files.len());
        let mut seed = u64::from(std::process::id()) | 1;
        let mut pick = Vec::new();
        for _ in 0..16 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            pick.push(files.swap_remove((seed % files.len() as u64) as usize));
        }
        let engine = Engine::start(
            egui::Context::default(),
            String::new(),
            dsp::DspSettings::default(),
        );
        engine.set_gain(0.0);
        let mut worst = u64::MAX;
        for (i, f) in pick.iter().enumerate() {
            engine.shared.least_held_ms.store(u64::MAX, Relaxed);
            engine.send(Cmd::Play {
                id: i as u64 * 2,
                path: f.clone(),
                gain: 1.0,
                fade_ms: 0,
            });
            std::thread::sleep(Duration::from_millis(300));
            let next = &pick[(i + 1) % pick.len()];
            engine.send(Cmd::Preload(Some((i as u64 * 2 + 1, next.clone(), 1.0, 0))));
            std::thread::sleep(Duration::from_millis(1500));
            engine.send(Cmd::Seek(engine.duration_ms() / 2));
            std::thread::sleep(Duration::from_millis(1500));
            let _ = engine.poll();
            let held = engine.shared.least_held_ms.load(Relaxed);
            worst = worst.min(held);
            println!("{held:>4} ms held  {}", f.display());
        }
        println!("the least held: {worst} ms");
        assert!(worst >= 10, "{worst} ms");
    }
}
