//! Scanning the music folders on a thread of its own: the index first
//! (shown at once), then the folders walked, the tags of new and changed
//! files read on a few threads, the library grouped and the index saved.
//! The UI takes the results with [`Scan::poll`] and swaps in the new
//! library.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rayon::prelude::*;

use super::{Library, Track, index, tags};

/// What a scan tells the UI.
pub enum ScanMsg {
    /// A library to show before the scan ends: the index's, before the
    /// folders are looked at, then (on a long scan, the first one) what
    /// has been read so far, every `CHUNK` files.
    Loaded(Arc<Library>),
    /// Tags read so far, of how many to read.
    Progress { done: usize, total: usize },
    /// The library as the folders now hold it.
    Done(Scanned),
}

pub struct Scanned {
    pub library: Arc<Library>,
    /// Files whose tags were read (new or changed); the rest came from the
    /// index.
    pub read: usize,
    pub took: Duration,
}

/// Files read between two libraries shown while a long scan goes on.
const CHUNK: usize = 500;

/// A file of a supported type found in the folders.
pub struct Found {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
}

/// A scan under way.
pub struct Scan {
    cancel: Arc<AtomicBool>,
    rx: Receiver<ScanMsg>,
}

/// Wakes the UI when a message is sent. The first scan starts before the
/// window exists (`main`), so the context comes later (`App::new`).
pub type Wake = Arc<OnceLock<egui::Context>>;

impl Scan {
    /// Loads the index when `load_index`, then scans `folders` (if any),
    /// taking unchanged files from what was loaded or from `previous`.
    pub fn start(
        folders: Vec<PathBuf>,
        previous: Option<Arc<Library>>,
        load_index: bool,
        wake: Wake,
    ) -> Scan {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let stop = Arc::clone(&cancel);
        let spawned = std::thread::Builder::new()
            .name("scan".into())
            .spawn(move || run(folders, previous, load_index, &stop, &tx, &wake));
        if let Err(e) = spawned {
            log::warn!("cannot start the scan: {e}");
        }
        Scan { cancel, rx }
    }

    /// The messages sent since the last call.
    pub fn poll(&self) -> Vec<ScanMsg> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Scan {
    /// A scan replaced or dropped stops at its next file; it neither sends
    /// nor saves anything more.
    fn drop(&mut self) {
        self.cancel.store(true, Relaxed);
    }
}

fn run(
    folders: Vec<PathBuf>,
    previous: Option<Arc<Library>>,
    load_index: bool,
    cancel: &AtomicBool,
    tx: &Sender<ScanMsg>,
    wake: &Wake,
) {
    let send = |msg| {
        let _ = tx.send(msg);
        if let Some(ctx) = wake.get() {
            ctx.request_repaint();
        }
    };
    let start = Instant::now();
    let mut read_all = false;
    let previous = if load_index {
        let (tracks, stale) = index::load();
        read_all = stale;
        let library = Arc::new(Library::new(tracks));
        log::debug!(
            "index: {} tracks in {} ms",
            library.tracks.len(),
            start.elapsed().as_millis()
        );
        send(ScanMsg::Loaded(Arc::clone(&library)));
        Some(library)
    } else {
        previous
    };
    if folders.is_empty() && !load_index {
        // The last folder removed: an empty library.
        index::save(&[]);
        send(ScanMsg::Done(Scanned {
            library: Arc::new(Library::default()),
            read: 0,
            took: start.elapsed(),
        }));
        return;
    }
    if folders.is_empty() {
        // The index read and no folder to look at: done.
        send(ScanMsg::Done(Scanned {
            library: previous.unwrap_or_default(),
            read: 0,
            took: start.elapsed(),
        }));
        return;
    }
    let started = Instant::now();
    let Walked { found, unreadable } = walk(&folders, cancel);
    if cancel.load(Relaxed) {
        return;
    }
    // An index of an older version is shown while every file is read
    // again, not replaced by the part read so far.
    let known: HashMap<&Path, &Track> = previous
        .as_deref()
        .filter(|_| !read_all)
        .map(|l| l.tracks.iter().map(|t| (t.path.as_path(), t)).collect())
        .unwrap_or_default();
    let (mut tracks, read) = read_tracks(
        &found,
        &known,
        cancel,
        &|done, total| send(ScanMsg::Progress { done, total }),
        &|so_far| {
            if !read_all {
                send(ScanMsg::Loaded(Arc::new(Library::new(so_far.to_vec()))));
            }
        },
    );
    if cancel.load(Relaxed) {
        return;
    }
    // A folder that could not be read (a disk not plugged in, a network
    // share away) keeps its tracks as they were, rather than lose them
    // until it is read whole again.
    if !unreadable.is_empty()
        && let Some(previous) = &previous
    {
        log::warn!("folders not read, their tracks kept: {unreadable:?}");
        let listed: std::collections::HashSet<&Path> =
            tracks.iter().map(|t| t.path.as_path()).collect();
        let kept: Vec<Track> = previous
            .tracks
            .iter()
            .filter(|t| {
                !listed.contains(t.path.as_path())
                    && unreadable.iter().any(|u| t.path.starts_with(u))
            })
            .cloned()
            .collect();
        tracks.extend(kept);
    }
    // Fewer kept than known: files were deleted or their folder dropped.
    let removed = tracks.len() - read < known.len();
    let library = Arc::new(Library::new(tracks));
    if read > 0 || removed {
        index::save(&library.tracks);
    }
    log::debug!(
        "scan: {} files, {read} read, in {} ms",
        found.len(),
        started.elapsed().as_millis()
    );
    send(ScanMsg::Done(Scanned {
        library,
        read,
        took: started.elapsed(),
    }));
}

/// What [`walk`] found.
pub struct Walked {
    /// The files of supported types.
    pub found: Vec<Found>,
    /// The folders (a library folder itself among them) and files that
    /// could not be read: what the index holds under them is kept.
    pub unreadable: Vec<PathBuf>,
}

/// The files of supported types under `folders`, with their sizes and
/// modification times. A folder listed inside another is walked once.
pub fn walk(folders: &[PathBuf], cancel: &AtomicBool) -> Walked {
    let mut seen = std::collections::HashSet::new();
    let mut walked = Walked {
        found: Vec::new(),
        unreadable: Vec::new(),
    };
    for folder in folders {
        for entry in walkdir::WalkDir::new(folder).follow_links(false) {
            if cancel.load(Relaxed) {
                return walked;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    if let Some(path) = e.path() {
                        walked.unreadable.push(path.to_path_buf());
                    }
                    continue;
                }
            };
            if !entry.file_type().is_file() || !crate::filetypes::is_audio(entry.path()) {
                continue;
            }
            if !seen.insert(entry.path().to_path_buf()) {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                walked.unreadable.push(entry.into_path());
                continue;
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            walked.found.push(Found {
                path: entry.into_path(),
                size: meta.len(),
                mtime,
            });
        }
    }
    walked
}

/// The tracks of `found`: those `known` with the same size and time as
/// they are, the others read anew (on up to eight threads: more only queue
/// on the disk). Also how many were read. `progress` hears of the reads at
/// most ten times a second; when there are more than [`CHUNK`] to read,
/// `partial` gets the tracks so far after each chunk. A file that cannot be
/// opened now (locked, gone, its share away) keeps what was `known` of it,
/// with its old size and time, so that the next scan reads it again; one
/// not known is left out until then.
pub fn read_tracks(
    found: &[Found],
    known: &HashMap<&Path, &Track>,
    cancel: &AtomicBool,
    progress: &(dyn Fn(usize, usize) + Sync),
    partial: &dyn Fn(&[Track]),
) -> (Vec<Track>, usize) {
    let mut tracks = Vec::with_capacity(found.len());
    let mut to_read = Vec::new();
    for f in found {
        match known.get(f.path.as_path()) {
            Some(t) if t.size == f.size && t.mtime == f.mtime => tracks.push((*t).clone()),
            _ => to_read.push(f),
        }
    }
    let total = to_read.len();
    if total == 0 {
        return (tracks, 0);
    }
    let threads = std::env::var("METALAMP_SCAN_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .min(8)
        });
    let done = AtomicUsize::new(0);
    let last_report = std::sync::Mutex::new(Instant::now());
    let read = |f: &&Found| -> Option<Track> {
        if cancel.load(Relaxed) {
            return None;
        }
        let track = tags::read(&f.path, f.size, f.mtime)
            .or_else(|| known.get(f.path.as_path()).map(|t| (*t).clone()));
        let n = done.fetch_add(1, Relaxed) + 1;
        if let Ok(mut last) = last_report.try_lock()
            && last.elapsed() >= Duration::from_millis(100)
        {
            *last = Instant::now();
            progress(n, total);
        }
        track
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("tags {i}"))
        .build()
        .ok();
    let mut read_count = 0;
    for chunk in to_read.chunks(CHUNK) {
        let new: Vec<Track> = match &pool {
            Some(pool) => pool.install(|| chunk.par_iter().filter_map(read).collect()),
            None => chunk.iter().filter_map(read).collect(),
        };
        read_count += new.len();
        tracks.extend(new);
        if cancel.load(Relaxed) {
            break;
        }
        if total > CHUNK && read_count < total {
            partial(&tracks);
        }
    }
    (tracks, read_count)
}
