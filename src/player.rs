//! The player: the playlist (`queue.rs`), the audio engine (`audio`), and
//! what joins them: which entry is heard, which is preloaded for a gapless
//! change, what a failure skips to, the volume.
//!
//! Each `Play` and `Preload` sent to the engine carries a token of its own,
//! mapped here to the playlist entry: an entry played twice in a row
//! (repeat one) starts anew with a new token, so its start is news.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::audio::dsp::{self, DspSettings};
use crate::audio::{self, Cmd, Engine, Event};
use crate::loudness::Loudness;
use crate::queue::{Queue, Repeat};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Playing,
    Paused,
}

pub struct Player {
    engine: Engine,
    pub queue: Queue,
    pub status: Status,
    pub volume: u8,
    pub mute: bool,
    /// The entry heard (its id), while playing or paused.
    pub playing: Option<u64>,
    /// Tokens of the engine's `Play` and `Preload` to playlist entry ids.
    tokens: HashMap<u64, u64>,
    last_token: u64,
    /// The token of the last `Play`: a failure of one before it is old
    /// news (the user went on meanwhile).
    last_play: u64,
    /// The token and entry preloaded.
    preloaded: Option<(u64, u64)>,
    /// Failures one after the other: past the playlist's length, stop.
    failed_in_row: usize,
    /// The last failure, for the status line.
    pub error: Option<String>,
    /// Stop when the track heard ends (Shift+V): nothing is preloaded;
    /// off again once it has stopped.
    pub stop_after: bool,
    /// Where to take up the entry (its id) the last run left off at (ms),
    /// when it is played next; any other play forgets it.
    pub resume: Option<(u64, u64)>,
    /// The tracks' loudness, for normalisation.
    loudness: Loudness,
    dsp: DspSettings,
}

/// What changed with the engine's events, for the app.
#[derive(Default)]
pub struct Changes {
    /// A new entry is heard (the title, the cover).
    pub track: bool,
}

impl Player {
    pub fn new(
        ctx: &egui::Context,
        volume: u8,
        mute: bool,
        repeat: Repeat,
        shuffle: bool,
        device: String,
        dsp: DspSettings,
    ) -> Player {
        let engine = Engine::start(ctx.clone(), device, dsp.clone());
        let loudness = Loudness::load(ctx.clone());
        engine.set_gain(audio::gain_of(volume, mute));
        let mut queue = Queue::default();
        queue.repeat = repeat;
        queue.set_shuffle(shuffle);
        Player {
            engine,
            queue,
            status: Status::Stopped,
            volume,
            mute,
            playing: None,
            tokens: HashMap::new(),
            last_token: 0,
            last_play: 0,
            preloaded: None,
            failed_in_row: 0,
            error: None,
            resume: None,
            stop_after: false,
            loudness,
            dsp,
        }
    }

    /// The loudness gain of `path` for the engine: known, or 1 while it
    /// is measured (the gain follows with `Cmd::Gain`); 1 when
    /// normalisation is off.
    fn gain_for(&mut self, path: &Path) -> f32 {
        if !self.dsp.normalize {
            return 1.0;
        }
        match self.loudness.get(path) {
            Some(m) => dsp::track_gain(m.lufs, m.peak, self.dsp.limiter),
            None => {
                self.loudness.measure(path);
                1.0
            }
        }
    }

    /// The path of the entry a token stands for.
    fn path_of_token(&self, token: u64) -> Option<PathBuf> {
        let entry = *self.tokens.get(&token)?;
        let i = self.queue.index_of(entry)?;
        Some(self.queue.entries()[i].path.clone())
    }

    /// New processing settings for the engine; normalisation turned on
    /// gives the tracks open their gains.
    pub fn set_dsp(&mut self, dsp: &DspSettings) {
        // The tracks' gains anew: normalisation turned on, or the limiter
        // switched (it sets how far a quiet track is raised).
        let regain = dsp.normalize && (!self.dsp.normalize || dsp.limiter != self.dsp.limiter);
        self.dsp = dsp.clone();
        self.engine.send(Cmd::Dsp(dsp.clone()));
        if regain {
            let live: Vec<(u64, PathBuf)> = self
                .tokens
                .keys()
                .filter_map(|&t| self.path_of_token(t).map(|p| (t, p)))
                .collect();
            for (token, path) in live {
                let gain = self.gain_for(&path);
                self.engine.send(Cmd::Gain { id: token, gain });
            }
        }
    }

    fn token_for(&mut self, entry: u64) -> u64 {
        self.last_token += 1;
        self.tokens.insert(self.last_token, entry);
        self.last_token
    }

    /// Play the playlist's entry at `index` from its start.
    pub fn play_index(&mut self, index: usize) {
        let Some(entry) = self.queue.entries().get(index).cloned() else {
            return;
        };
        self.queue.set_current(Some(index));
        let token = self.token_for(entry.id);
        self.last_play = token;
        self.preloaded = None;
        let gain = self.gain_for(&entry.path);
        self.engine.send(Cmd::Play {
            id: token,
            path: entry.path,
            gain,
        });
        // The place the last run left this entry at.
        if let Some((id, ms)) = self.resume.take()
            && id == entry.id
        {
            self.engine.send(Cmd::Seek(ms));
        }
        self.status = Status::Playing;
        self.error = None;
    }

    /// The playlist becomes `paths`, played from the one at `start`.
    pub fn play_paths(&mut self, paths: Vec<PathBuf>, start: usize) {
        if paths.is_empty() {
            return;
        }
        let start = start.min(paths.len() - 1);
        self.queue.replace(paths);
        self.queue.set_current(Some(start));
        // Shuffled: the order starts with the one chosen.
        let shuffle = self.queue.shuffle();
        self.queue.set_shuffle(shuffle);
        self.failed_in_row = 0;
        self.play_index(start);
    }

    /// `paths` at the playlist's end; played if nothing plays.
    pub fn enqueue(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let first = self.queue.add(paths);
        if self.status == Status::Stopped {
            self.play_index(first);
        } else {
            self.refresh_preload();
        }
    }

    /// `paths` put in the playlist at `at` (`next`: right after the
    /// current entry in the order of play); the index of the first.
    pub fn insert(&mut self, at: usize, paths: Vec<PathBuf>, next: bool) -> usize {
        let first = self.queue.insert(at, paths, next);
        self.refresh_preload();
        first
    }

    /// `paths` right after the current entry (Play next); played if
    /// nothing plays, as `enqueue`.
    pub fn play_next(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let at = self.queue.current().map_or(self.queue.len(), |c| c + 1);
        let first = self.insert(at, paths, true);
        if self.status == Status::Stopped {
            self.play_index(first);
        }
    }

    /// The playlist's entries at `indices` moved right after the current
    /// one; the index of the first.
    pub fn move_next(&mut self, indices: &[usize]) -> usize {
        let at = self.queue.move_next(indices);
        self.refresh_preload();
        at
    }

    /// Stop after the track heard, or not (Shift+V).
    pub fn toggle_stop_after(&mut self) {
        self.stop_after = !self.stop_after;
        self.refresh_preload();
    }

    pub fn toggle_pause(&mut self) {
        match self.status {
            Status::Playing => {
                self.engine.send(Cmd::Pause);
                self.status = Status::Paused;
            }
            Status::Paused => {
                self.engine.send(Cmd::Resume);
                self.status = Status::Playing;
            }
            Status::Stopped => self.play(),
        }
    }

    /// Play: the current entry, else the first.
    pub fn play(&mut self) {
        match self.status {
            Status::Paused => self.toggle_pause(),
            Status::Playing => {}
            Status::Stopped if self.queue.is_empty() => {}
            Status::Stopped => {
                if let Some(i) = self.queue.current().or_else(|| self.queue.after(None)) {
                    self.play_index(i);
                }
            }
        }
    }

    pub fn stop(&mut self) {
        self.engine.send(Cmd::Stop);
        self.status = Status::Stopped;
        self.playing = None;
        self.preloaded = None;
        // A Started already on its way is of nothing now.
        self.tokens.clear();
    }

    /// The entry after the current one (round to the first with repeat
    /// all); with repeat one, still the next one: the button asks for it.
    pub fn next(&mut self) {
        let current = self.queue.current();
        let repeat = self.queue.repeat;
        if repeat == Repeat::One {
            self.queue.repeat = Repeat::Off;
        }
        let next = self.queue.after(current).or_else(|| {
            // The end: round to the start.
            self.queue.after(None)
        });
        self.queue.repeat = repeat;
        if let Some(i) = next {
            self.play_index(i);
        }
    }

    /// The start of the track when past its first 3 seconds, else the
    /// entry before.
    pub fn previous(&mut self) {
        if self.status != Status::Stopped && self.engine.position_ms() > 3000 {
            self.seek(0);
            return;
        }
        if let Some(i) = self.queue.before(self.queue.current()) {
            self.play_index(i);
        }
    }

    pub fn seek(&mut self, ms: u64) {
        if self.status != Status::Stopped {
            self.engine.send(Cmd::Seek(ms));
        }
    }

    /// Seconds forward (or back) from the position.
    pub fn seek_by(&mut self, seconds: i64) {
        let at = self.engine.position_ms() as i64 + seconds * 1000;
        let end = self.engine.duration_ms() as i64;
        let at = if end > 0 { at.min(end - 500) } else { at };
        self.seek(at.max(0) as u64);
    }

    pub fn position_ms(&self) -> u64 {
        self.engine.position_ms()
    }

    /// Where playing is to be taken up from at the next start: the
    /// position while playing or paused, the place still waiting to be
    /// taken up, else 0.
    pub fn position_to_keep(&self) -> u64 {
        match self.status {
            Status::Stopped => self
                .resume
                .filter(|&(id, _)| self.queue.current_entry().is_some_and(|e| e.id == id))
                .map_or(0, |(_, ms)| ms),
            _ => self.position_ms(),
        }
    }

    /// Play on `device` (its id; empty: the default) from now on.
    pub fn set_device(&self, device: &str) {
        self.engine.send(Cmd::Device(device.to_owned()));
    }

    /// The `n` frames heard last, for the visualizations, their sample
    /// rate and where they end.
    pub fn heard_samples(&self, n: usize, out: &mut Vec<f32>) -> Option<crate::audio::vis::Heard> {
        self.engine.heard_samples(n, out)
    }

    pub fn duration_ms(&self) -> u64 {
        self.engine.duration_ms()
    }

    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
        if self.volume > 0 {
            self.mute = false;
        }
        self.engine.set_gain(audio::gain_of(self.volume, self.mute));
    }

    pub fn toggle_mute(&mut self) {
        self.mute = !self.mute;
        self.engine.set_gain(audio::gain_of(self.volume, self.mute));
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.queue.repeat = repeat;
        self.refresh_preload();
    }

    pub fn set_shuffle(&mut self, on: bool) {
        self.queue.set_shuffle(on);
        self.refresh_preload();
    }

    /// The playlist changed: tell the engine what comes after the current
    /// entry, if that is not what it holds.
    pub fn refresh_preload(&mut self) {
        if self.status == Status::Stopped {
            return;
        }
        // To stop after this one: nothing to go on to.
        if self.stop_after {
            if self.preloaded.take().is_some() {
                self.engine.send(Cmd::Preload(None));
            }
            return;
        }
        let next = self.queue.after(self.queue.current());
        let entry = next.map(|i| self.queue.entries()[i].clone());
        // Already preloaded. With repeat one the entry preloaded is the
        // current one; once it starts, `preloaded` is cleared and it is
        // preloaded again with a new token.
        match (&entry, self.preloaded) {
            (Some(e), Some((_, id))) if e.id == id => return,
            (None, None) => return,
            _ => {}
        }
        match entry {
            Some(e) => {
                let token = self.token_for(e.id);
                self.preloaded = Some((token, e.id));
                let gain = self.gain_for(&e.path);
                self.engine.send(Cmd::Preload(Some((token, e.path, gain))));
            }
            None => {
                self.preloaded = None;
                self.engine.send(Cmd::Preload(None));
            }
        }
    }

    /// The engine's events.
    pub fn poll(&mut self) -> Changes {
        let mut changes = Changes::default();
        // Tracks measured meanwhile: their gains to the engine, for the
        // ones it has open.
        for (path, m) in self.loudness.poll() {
            if !self.dsp.normalize {
                continue;
            }
            let gain = dsp::track_gain(m.lufs, m.peak, self.dsp.limiter);
            let tokens: Vec<u64> = self
                .tokens
                .keys()
                .copied()
                .filter(|&t| self.path_of_token(t).as_deref() == Some(path.as_path()))
                .collect();
            for token in tokens {
                self.engine.send(Cmd::Gain { id: token, gain });
            }
        }
        for event in self.engine.poll() {
            match event {
                Event::Started { id: token } => {
                    let Some(&entry) = self.tokens.get(&token) else {
                        continue;
                    };
                    self.tokens.retain(|&t, _| t >= token);
                    if self.preloaded.is_some_and(|(t, _)| t == token) {
                        self.preloaded = None;
                    }
                    self.playing = Some(entry);
                    if let Some(i) = self.queue.index_of(entry) {
                        self.queue.set_current(Some(i));
                    }
                    if self.status == Status::Stopped {
                        self.status = Status::Playing;
                    }
                    self.failed_in_row = 0;
                    changes.track = true;
                    self.refresh_preload();
                }
                Event::Ended => {
                    // Stopped as asked: the next play goes on as usual.
                    self.stop_after = false;
                    self.status = Status::Stopped;
                    self.playing = None;
                    self.preloaded = None;
                    changes.track = true;
                }
                Event::Failed { id: token, message } => {
                    let Some(&entry) = self.tokens.get(&token) else {
                        continue;
                    };
                    let path = self
                        .queue
                        .index_of(entry)
                        .map(|i| self.queue.entries()[i].path.display().to_string())
                        .unwrap_or_default();
                    log::warn!("cannot play {path}: {message}");
                    self.error = Some(format!("{path}: {message}"));
                    let from = self.queue.index_of(entry);
                    if self.preloaded.is_some_and(|(t, _)| t == token) {
                        // Preload the one after it instead, unless the
                        // whole list failed (its disk gone, round and
                        // round with repeat all).
                        self.preloaded = None;
                        self.failed_in_row += 1;
                        if self.failed_in_row < self.queue.len()
                            && let Some(next) = self.queue.after(from).filter(|&n| Some(n) != from)
                            && let Some(e) = self.queue.entries().get(next).cloned()
                        {
                            let token = self.token_for(e.id);
                            self.preloaded = Some((token, e.id));
                            let gain = self.gain_for(&e.path);
                            self.engine.send(Cmd::Preload(Some((token, e.path, gain))));
                        }
                        continue;
                    }
                    if token < self.last_play {
                        continue;
                    }
                    self.failed_in_row += 1;
                    let next = if self.queue.repeat == Repeat::One {
                        None
                    } else {
                        self.queue.after(from)
                    };
                    match next {
                        Some(i) if self.failed_in_row < self.queue.len() => self.play_index(i),
                        _ => {
                            self.status = Status::Stopped;
                            self.playing = None;
                        }
                    }
                    changes.track = true;
                }
            }
        }
        changes
    }

    /// The path heard (or paused), if any.
    pub fn playing_path(&self) -> Option<&PathBuf> {
        let id = self.playing?;
        self.queue
            .index_of(id)
            .map(|i| &self.queue.entries()[i].path)
    }
}
