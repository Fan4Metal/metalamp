//! The loudness of the tracks, for normalisation: measured after EBU R128
//! (`audio::dsp::loudness`) on a thread of its own the first time a
//! track is about to be played (the track heard when the setting is on,
//! the one preloaded while the current plays), kept in `loudness.json`
//! next to the settings as `path: [LUFS, peak]`, so a track is decoded
//! for it once. The player asks with `measure`, takes the answers with
//! `poll` and sends the engine the gains.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use serde::{Deserialize, Serialize};

const FILE: &str = "loudness.json";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Default)]
struct Index {
    version: u32,
    /// The path as text to `[LUFS, sample peak]`.
    tracks: HashMap<String, [f32; 2]>,
}

/// A track's measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measured {
    pub lufs: f32,
    pub peak: f32,
}

pub struct Loudness {
    known: HashMap<PathBuf, Measured>,
    /// Being measured, or failed to be (not tried again in this run).
    pending: HashSet<PathBuf>,
    asks: Sender<PathBuf>,
    answers: Receiver<(PathBuf, Option<Measured>)>,
    file: Option<PathBuf>,
}

impl Loudness {
    /// Reads the file and starts the measuring thread; `ctx` is woken
    /// with each answer.
    pub fn load(ctx: egui::Context) -> Loudness {
        let file = crate::settings::data_dir().map(|d| d.join(FILE));
        let known = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|bytes| serde_json::from_slice::<Index>(&bytes).ok())
            .filter(|i| i.version == VERSION)
            .map(|i| {
                i.tracks
                    .into_iter()
                    .map(|(p, [lufs, peak])| (PathBuf::from(p), Measured { lufs, peak }))
                    .collect()
            })
            .unwrap_or_default();
        let (asks, ask_rx) = channel::<PathBuf>();
        let (answer_tx, answers) = channel();
        let spawned = std::thread::Builder::new()
            .name("loudness".into())
            .spawn(move || measure_thread(&ask_rx, &answer_tx, &ctx));
        if let Err(e) = spawned {
            log::error!("cannot start the loudness thread: {e}");
        }
        Loudness {
            known,
            pending: HashSet::new(),
            asks,
            answers,
            file,
        }
    }

    pub fn get(&self, path: &Path) -> Option<Measured> {
        self.known.get(path).copied()
    }

    /// Measure `path` unless it is known or being measured.
    pub fn measure(&mut self, path: &Path) {
        if self.known.contains_key(path) || self.pending.contains(path) {
            return;
        }
        self.pending.insert(path.to_path_buf());
        let _ = self.asks.send(path.to_path_buf());
    }

    /// The tracks measured since the last call; the file is written when
    /// there were any.
    pub fn poll(&mut self) -> Vec<(PathBuf, Measured)> {
        let mut out = Vec::new();
        for (path, measured) in self.answers.try_iter() {
            // One that could not be measured stays pending: it is not
            // decoded again each time it is played.
            if let Some(m) = measured {
                self.pending.remove(&path);
                self.known.insert(path.clone(), m);
                out.push((path, m));
            }
        }
        if !out.is_empty() {
            self.save();
        }
        out
    }

    fn save(&self) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let index = Index {
            version: VERSION,
            tracks: self
                .known
                .iter()
                .map(|(p, m)| (p.to_string_lossy().into_owned(), [m.lufs, m.peak]))
                .collect(),
        };
        static FILE: crate::kept::Kept = crate::kept::Kept::new();
        let number = FILE.ask();
        std::thread::spawn(move || {
            if let Ok(bytes) = serde_json::to_vec(&index)
                && let Err(e) = FILE.write(number, &file, &bytes)
            {
                log::warn!("cannot write {}: {e}", file.display());
            }
        });
    }
}

/// Decodes each path asked for whole and meters it; Media Foundation on
/// this thread, as the audio thread has its own.
fn measure_thread(
    asks: &Receiver<PathBuf>,
    answers: &Sender<(PathBuf, Option<Measured>)>,
    ctx: &egui::Context,
) {
    use crate::audio::mf;
    let _com = mf::com_init_mta();
    let _mf = match mf::Mf::start() {
        Ok(m) => m,
        Err(e) => {
            log::error!("loudness: Media Foundation: {}", mf::describe(&e));
            return;
        }
    };
    while let Ok(path) = asks.recv() {
        let started = std::time::Instant::now();
        let measured = measure_file(&path);
        match measured {
            Some(m) => log::debug!(
                "loudness: {} {:.1} LUFS, peak {:.2} in {} ms",
                path.display(),
                m.lufs,
                m.peak,
                started.elapsed().as_millis()
            ),
            None => log::warn!("loudness: cannot measure {}", path.display()),
        }
        if answers.send((path, measured)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// `path` decoded whole and metered; `None` when it cannot be decoded or
/// is silent.
pub fn measure_file(path: &Path) -> Option<Measured> {
    use crate::audio::decoder::Decoder;
    use crate::audio::dsp::loudness::Meter;
    let mut decoder = Decoder::open(path).ok()?;
    let mut meter = Meter::new(decoder.rate, usize::from(decoder.channels.max(1)));
    while let Ok(Some(block)) = decoder.next() {
        meter.add(&block);
    }
    let lufs = meter.integrated()?;
    Some(Measured {
        lufs,
        peak: meter.peak(),
    })
}
