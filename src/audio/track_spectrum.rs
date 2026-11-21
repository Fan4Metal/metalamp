//! The spectrum of a whole track, as Spek draws it: the file decoded on a
//! "spectrum" thread of its own (Media Foundation, as the loudness'), its
//! channels mixed to one, Hann windows of `FFT` frames, and the mean power
//! of each bin over the windows of a column, `COLUMNS` across the track.
//! The columns come one after another (the picture fills from the left)
//! into the `Job` the window holds; a job is cancelled when the window
//! closes or another file is asked for, so the file is let go within a
//! block (its tags may be written next).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The window, in frames (bins of 21.5 Hz at 44.1 kHz).
pub const FFT: usize = 2048;
/// The bins kept: from 0 Hz to just under half the rate.
pub const BINS: usize = FFT / 2;
/// The columns across the whole track.
pub const COLUMNS: usize = 2048;
/// The level of a stored 0, in dB of a full-scale sine; 255 is 0 dB.
pub const FLOOR_DB: f32 = -120.0;
/// Windows averaged into a column at most: a long track is not read
/// window by window (the decoding costs more than these then).
const WINDOWS_PER_COLUMN: f64 = 8.0;

/// What has been worked out of a file.
#[derive(Default)]
pub struct Grid {
    /// The rate decoded (an HE-AAC's twice the file's), 0 until known.
    pub rate: u32,
    /// The levels of the columns done, `BINS` a column from 0 Hz up:
    /// 0 is `FLOOR_DB`, 255 is 0 dB.
    pub levels: Vec<u8>,
    /// The length the `COLUMNS` span, in seconds: the file's as it says
    /// until it is decoded, then as decoded.
    pub seconds: f64,
    pub done: bool,
    /// Why it could not be decoded.
    pub error: Option<String>,
}

impl Grid {
    /// The columns done.
    pub fn columns(&self) -> usize {
        self.levels.len() / BINS
    }

    /// The level stored for `column` and `bin`, if worked out.
    pub fn at(&self, column: usize, bin: usize) -> Option<u8> {
        self.levels.get(column * BINS + bin.min(BINS - 1)).copied()
    }
}

/// One file's spectrum being worked out, shared with the thread.
#[derive(Default)]
pub struct Job {
    cancelled: AtomicBool,
    /// Raised with each change of the grid.
    revision: AtomicU64,
    grid: Mutex<Grid>,
}

impl Job {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    pub fn grid(&self) -> std::sync::MutexGuard<'_, Grid> {
        self.grid.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn changed(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }
}

/// The thread working out the spectra asked for, started with the first.
#[derive(Default)]
pub struct Worker {
    asks: Option<Sender<(PathBuf, Arc<Job>)>>,
}

impl Worker {
    /// Starts working out `path`'s spectrum; `ctx` is woken as columns
    /// come.
    pub fn start(&mut self, ctx: &egui::Context, path: &Path) -> Arc<Job> {
        let job = Arc::new(Job::default());
        if self.asks.is_none() {
            let (tx, rx) = channel();
            let ctx = ctx.clone();
            let started = std::thread::Builder::new()
                .name("spectrum".into())
                .spawn(move || thread(&rx, &ctx));
            match started {
                Ok(_) => self.asks = Some(tx),
                Err(e) => log::warn!("no spectrum thread: {e}"),
            }
        }
        let sent = self
            .asks
            .as_ref()
            .is_some_and(|tx| tx.send((path.to_path_buf(), job.clone())).is_ok());
        if !sent {
            let mut grid = job.grid();
            grid.done = true;
            grid.error = Some("no thread to decode it".into());
        }
        job
    }
}

fn thread(asks: &Receiver<(PathBuf, Arc<Job>)>, ctx: &egui::Context) {
    use super::mf;
    let _com = mf::com_init_mta();
    let _mf = match mf::Mf::start() {
        Ok(m) => m,
        Err(e) => {
            let message = mf::describe(&e);
            log::error!("spectrum: Media Foundation: {message}");
            // Each job asked for told so, not left waiting.
            while let Ok((_, job)) = asks.recv() {
                let mut grid = job.grid();
                grid.done = true;
                grid.error = Some(message.clone());
                drop(grid);
                job.changed();
                ctx.request_repaint();
            }
            return;
        }
    };
    let mut fft = Fft::new();
    while let Ok(mut ask) = asks.recv() {
        // Only the last of those asked for meanwhile.
        while let Ok(next) = asks.try_recv() {
            ask.1.cancel();
            ask = next;
        }
        let (path, job) = ask;
        if job.cancelled() {
            continue;
        }
        let started = Instant::now();
        let result = crate::guard::catch(&path, || work_out(&path, &job, &mut fft, ctx))
            .unwrap_or_else(|| Err("the file could not be decoded".into()));
        {
            let mut grid = job.grid();
            grid.done = true;
            if let Err(e) = result {
                log::warn!("spectrum of {}: {e}", path.display());
                grid.error = Some(e);
            }
        }
        job.changed();
        ctx.request_repaint();
        log::debug!(
            "spectrum of {} in {} ms{}",
            path.display(),
            started.elapsed().as_millis(),
            if job.cancelled() { " (cancelled)" } else { "" }
        );
    }
}

/// The window and the twiddles of the FFT, made once.
struct Fft {
    window: Vec<f32>,
    twiddles: Vec<(f32, f32)>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Fft {
    fn new() -> Fft {
        use std::f32::consts::PI;
        Fft {
            window: (0..FFT)
                .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FFT as f32).cos())
                .collect(),
            twiddles: (0..FFT / 2)
                .map(|k| {
                    let a = -2.0 * PI * k as f32 / FFT as f32;
                    (a.cos(), a.sin())
                })
                .collect(),
            re: vec![0.0; FFT],
            im: vec![0.0; FFT],
        }
    }

    /// Adds the power of each bin of `frames` (`FFT` of them), 1 for a
    /// full-scale sine in its bin, to `sums`.
    fn add_power(&mut self, frames: &[f32], sums: &mut [f32]) {
        for ((re, im), (&x, &w)) in self
            .re
            .iter_mut()
            .zip(self.im.iter_mut())
            .zip(frames.iter().zip(&self.window))
        {
            *re = x * w;
            *im = 0.0;
        }
        super::dsp::fft::fft(&mut self.re, &mut self.im, &self.twiddles);
        // A sine of amplitude A gives A/2 times the window's sum (N/2).
        let scale = 1.0 / (FFT as f32 / 4.0).powi(2);
        for (k, sum) in sums.iter_mut().enumerate() {
            *sum += (self.re[k] * self.re[k] + self.im[k] * self.im[k]) * scale;
        }
    }
}

/// A column's mean power as stored levels.
fn levels_of(sums: &[f32], windows: u32, out: &mut Vec<u8>) {
    let n = windows.max(1) as f32;
    out.extend(sums.iter().map(|&s| level_of(s / n)));
}

/// A power (1: a full-scale sine) as a stored level.
pub fn level_of(power: f32) -> u8 {
    let db = 10.0 * power.max(1e-30).log10();
    ((db - FLOOR_DB) / -FLOOR_DB * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// A stored level in dB.
pub fn db_of(level: u8) -> f32 {
    FLOOR_DB + f32::from(level) / 255.0 * -FLOOR_DB
}

/// Decodes `path` and puts its columns into `job` as they are done.
fn work_out(path: &Path, job: &Job, fft: &mut Fft, ctx: &egui::Context) -> Result<(), String> {
    use super::decoder::{Decoder, SECOND};
    let mut decoder = Decoder::open(path).map_err(|e| super::mf::describe(&e))?;
    let rate = decoder.rate.max(1);
    let channels = usize::from(decoder.channels.max(1));
    let expected = (i128::from(decoder.duration) * i128::from(rate) / i128::from(SECOND)) as u64;
    {
        let mut grid = job.grid();
        grid.rate = rate;
        grid.seconds = expected as f64 / f64::from(rate);
    }
    job.changed();
    ctx.request_repaint();
    let mut columns = Columns::new(expected, rate);
    let mut last_shown = Instant::now();
    while !job.cancelled() {
        match decoder.next().map_err(|e| super::mf::describe(&e))? {
            Some(block) => {
                let mono = block
                    .chunks_exact(channels)
                    .map(|f| f.iter().sum::<f32>() / channels as f32);
                columns.push(mono, fft);
            }
            None => break,
        }
        // The columns done shown about 12 times a second.
        if columns.done.len() > job.grid().levels.len()
            && last_shown.elapsed() > Duration::from_millis(80)
        {
            last_shown = Instant::now();
            let mut grid = job.grid();
            let from = grid.levels.len();
            grid.levels.extend_from_slice(&columns.done[from..]);
            drop(grid);
            job.changed();
            ctx.request_repaint();
        }
    }
    if job.cancelled() {
        return Ok(());
    }
    let frames = columns.frames;
    let levels = columns.finish(fft);
    let mut grid = job.grid();
    grid.levels = levels;
    if frames > 0 {
        grid.seconds = frames as f64 / f64::from(rate);
    }
    Ok(())
}

/// The windows gathered into columns as the frames come.
struct Columns {
    /// Frames a column holds, as the length the file says (else half a
    /// second, resampled at the end).
    per_column: f64,
    /// Frames from one window to the next.
    hop: usize,
    /// The frames not yet passed by the windows, from `start`; the first
    /// window is centred on the track's first frame (half of it silence).
    pending: Vec<f32>,
    start: usize,
    /// The frame the next window is centred on.
    centre: u64,
    /// The frames given.
    frames: u64,
    /// The column being gathered, its sums and windows.
    column: usize,
    sums: Vec<f32>,
    windows: u32,
    /// The columns gathered, `BINS` a column.
    done: Vec<u8>,
}

impl Columns {
    fn new(expected: u64, rate: u32) -> Columns {
        let per_column = if expected > 0 {
            expected as f64 / COLUMNS as f64
        } else {
            f64::from(rate) / 2.0
        };
        Columns {
            per_column,
            hop: ((per_column / WINDOWS_PER_COLUMN) as usize).max(64),
            pending: vec![0.0; FFT / 2],
            start: 0,
            centre: 0,
            frames: 0,
            column: 0,
            sums: vec![0.0; BINS],
            windows: 0,
            done: Vec::with_capacity(COLUMNS * BINS),
        }
    }

    fn push(&mut self, mono: impl Iterator<Item = f32>, fft: &mut Fft) {
        let before = self.pending.len();
        self.pending.extend(mono);
        self.frames += (self.pending.len() - before) as u64;
        self.windows_while_full(fft);
    }

    fn windows_while_full(&mut self, fft: &mut Fft) {
        while self.pending.len() >= self.start + FFT {
            self.window(fft);
        }
        // What the windows have passed (a long track's hop passes frames
        // not yet given: what is left of it is passed when they come).
        let passed = self.start.min(self.pending.len());
        if passed > 1 << 16 || passed == self.pending.len() {
            self.pending.drain(..passed);
            self.start -= passed;
        }
    }

    /// The window from `start` added to its column.
    fn window(&mut self, fft: &mut Fft) {
        let column = (self.centre as f64 / self.per_column) as usize;
        while column > self.column {
            self.close_column();
        }
        fft.add_power(&self.pending[self.start..self.start + FFT], &mut self.sums);
        self.windows += 1;
        self.start += self.hop;
        self.centre += self.hop as u64;
    }

    /// The column gathered stored; one no window fell in (a track shorter
    /// than its columns' windows) as the one before.
    fn close_column(&mut self) {
        if self.windows == 0 && self.done.len() >= BINS {
            let last = self.done.len() - BINS;
            self.done.extend_from_within(last..);
        } else {
            levels_of(&self.sums, self.windows, &mut self.done);
        }
        self.sums.fill(0.0);
        self.windows = 0;
        self.column += 1;
    }

    /// The rest windowed (the last window centred near the last frame),
    /// and the columns made `COLUMNS` (the file longer or shorter than it
    /// said).
    fn finish(mut self, fft: &mut Fft) -> Vec<u8> {
        if self.frames == 0 {
            return Vec::new();
        }
        self.pending.extend(std::iter::repeat_n(0.0, FFT / 2));
        while self.pending.len() >= self.start + FFT && self.centre < self.frames {
            self.window(fft);
        }
        self.close_column();
        resample(&self.done, COLUMNS)
    }
}

/// `levels` (`BINS` a column) made `to` columns: the loudest of those a
/// column covers, or the nearest when there are fewer.
fn resample(levels: &[u8], to: usize) -> Vec<u8> {
    let from = levels.len() / BINS;
    if from == to || from == 0 {
        return levels.to_vec();
    }
    let mut out = Vec::with_capacity(to * BINS);
    for c in 0..to {
        let a = c * from / to;
        let b = ((c + 1) * from / to).clamp(a + 1, from);
        let first = out.len();
        out.extend_from_slice(&levels[a * BINS..(a + 1) * BINS]);
        for src in a + 1..b {
            for (o, &l) in out[first..]
                .iter_mut()
                .zip(&levels[src * BINS..(src + 1) * BINS])
            {
                *o = (*o).max(l);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn columns_of(samples: &[f32], expected: u64, rate: u32) -> Vec<u8> {
        let mut fft = Fft::new();
        let mut c = Columns::new(expected, rate);
        for block in samples.chunks(4410) {
            c.push(block.iter().copied(), &mut fft);
        }
        c.finish(&mut fft)
    }

    #[test]
    fn a_full_scale_sine_reads_0_db_in_its_bin() {
        let rate = 44100;
        let bin = 93; // 2002.6 Hz
        let f = bin as f32 * rate as f32 / FFT as f32;
        let n = rate as usize * 3;
        let sine: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * f * i as f32 / rate as f32).sin())
            .collect();
        let levels = columns_of(&sine, n as u64, rate);
        assert_eq!(levels.len(), COLUMNS * BINS);
        let middle = &levels[COLUMNS / 2 * BINS..(COLUMNS / 2 + 1) * BINS];
        let (loudest, &l) = middle.iter().enumerate().max_by_key(|(_, l)| **l).unwrap();
        assert_eq!(loudest, bin);
        assert!(db_of(l).abs() < 0.6, "{}", db_of(l));
        // Hann's leakage stays near the sine.
        assert!(
            db_of(middle[bin + 40]) < -90.0,
            "{}",
            db_of(middle[bin + 40])
        );
    }

    #[test]
    fn a_file_longer_than_it_said_fills_the_columns() {
        let rate = 8000;
        let n = rate as usize * 10;
        // Silence, then a loud second half.
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                if i < n / 2 {
                    0.0
                } else {
                    (i as f32 * 0.7).sin() * 0.5
                }
            })
            .collect();
        // It said half its length: the columns are made `COLUMNS` again,
        // the loud half on the right.
        let levels = columns_of(&samples, n as u64 / 2, rate);
        assert_eq!(levels.len(), COLUMNS * BINS);
        let loudest = |c: usize| *levels[c * BINS..(c + 1) * BINS].iter().max().unwrap();
        assert_eq!(loudest(COLUMNS / 8), 0);
        assert!(loudest(COLUMNS * 7 / 8) > 200);
    }

    #[test]
    fn a_short_file_has_every_column() {
        let rate = 44100;
        let n = 1000;
        let samples = vec![0.25f32; n];
        let levels = columns_of(&samples, n as u64, rate);
        assert_eq!(levels.len(), COLUMNS * BINS);
        // DC in every column.
        assert!((0..COLUMNS).all(|c| levels[c * BINS] > 150));
    }

    /// `METALAMP_SAMPLE`: the file's spectrum worked out on this thread,
    /// with the time taken and where its highs end (an MP3's low-pass).
    #[test]
    #[ignore]
    fn spectrum_of_a_real_file() {
        let path = std::env::var_os("METALAMP_SAMPLE").expect("METALAMP_SAMPLE");
        let _com = crate::audio::mf::com_init_mta();
        let _mf = crate::audio::mf::Mf::start().unwrap();
        let job = Job::default();
        let started = Instant::now();
        work_out(
            Path::new(&path),
            &job,
            &mut Fft::new(),
            &egui::Context::default(),
        )
        .unwrap();
        let grid = job.grid();
        assert_eq!(grid.columns(), COLUMNS);
        // The highest bin over −90 dB in a tenth of the columns or more.
        let loud = (0..BINS)
            .rev()
            .find(|&b| {
                (0..COLUMNS)
                    .filter(|&c| db_of(grid.at(c, b).unwrap()) > -90.0)
                    .count()
                    > COLUMNS / 10
            })
            .unwrap_or(0);
        println!(
            "{:.1} s at {} Hz in {} ms; highs up to {} Hz",
            grid.seconds,
            grid.rate,
            started.elapsed().as_millis(),
            loud * grid.rate as usize / FFT
        );
    }

    #[test]
    fn levels_round_trip() {
        assert_eq!(level_of(1.0), 255);
        assert_eq!(level_of(0.0), 0);
        assert!((db_of(level_of(1e-6)) + 60.0).abs() < 0.5);
    }
}
