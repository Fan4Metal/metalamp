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
    /// taking unchanged files from what was loaded or from `previous`, or,
    /// `read_all`, reading every file again (`previous` shown meanwhile).
    pub fn start(
        folders: Vec<PathBuf>,
        previous: Option<Arc<Library>>,
        load_index: bool,
        read_all: bool,
        wake: Wake,
    ) -> Scan {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let stop = Arc::clone(&cancel);
        let asked = Asked {
            folders,
            previous,
            load_index,
            read_all,
            number: index::ask(),
        };
        let spawned = std::thread::Builder::new()
            .name("scan".into())
            .spawn(move || run(asked, &stop, &tx, &wake));
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

/// What a scan was asked to do ([`Scan::start`]), and the number of its
/// index saves.
struct Asked {
    folders: Vec<PathBuf>,
    previous: Option<Arc<Library>>,
    load_index: bool,
    read_all: bool,
    number: u64,
}

fn run(asked: Asked, cancel: &AtomicBool, tx: &Sender<ScanMsg>, wake: &Wake) {
    let Asked {
        folders,
        previous,
        load_index,
        read_all: read_again,
        number,
    } = asked;
    let send = |msg| {
        let _ = tx.send(msg);
        if let Some(ctx) = wake.get() {
            ctx.request_repaint();
        }
    };
    let start = Instant::now();
    let mut read_all = read_again;
    let previous = if load_index {
        let loaded = index::load();
        read_all = loaded.stale;
        let library = Arc::new(Library::new(loaded.tracks).with_playlists(loaded.playlists));
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
        index::save(&[], &[], number);
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
    let Walked {
        found,
        mut playlists,
        unreadable,
    } = walk(&folders, cancel);
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
    // A folder that could not be read (a disk not plugged in, a network
    // share away) keeps its tracks as they were, rather than lose them
    // until it is read whole again.
    let mut kept: Vec<Track> = Vec::new();
    if !unreadable.is_empty()
        && let Some(previous) = &previous
    {
        log::warn!("folders not read, their tracks kept: {unreadable:?}");
        let listed: std::collections::HashSet<&Path> =
            found.iter().map(|f| f.path.as_path()).collect();
        kept = previous
            .tracks
            .iter()
            .filter(|t| {
                !listed.contains(t.path.as_path())
                    && unreadable.iter().any(|u| t.path.starts_with(u))
            })
            .cloned()
            .collect();
        // Its playlists too.
        for p in &previous.playlists {
            if !playlists.contains(p) && unreadable.iter().any(|u| p.starts_with(u)) {
                playlists.push(p.clone());
            }
        }
    }
    let playlists_changed = previous.as_deref().is_none_or(|l| l.playlists != playlists);
    let (mut tracks, read) = read_tracks(
        &found,
        &known,
        cancel,
        &|done, total| send(ScanMsg::Progress { done, total }),
        &|so_far| {
            // A long scan saves as it goes: one stopped by the end of the
            // program goes on from there next time.
            let mut saved = so_far.to_vec();
            saved.extend(kept.iter().cloned());
            index::save(&saved, &playlists, number);
            if !read_all {
                send(ScanMsg::Loaded(Arc::new(
                    Library::new(saved).with_playlists(playlists.clone()),
                )));
            }
        },
    );
    if cancel.load(Relaxed) {
        return;
    }
    tracks.extend(kept);
    // Fewer kept than known: files were deleted or their folder dropped.
    let removed = tracks.len() - read < known.len();
    let library = Arc::new(Library::new(tracks).with_playlists(playlists));
    if read > 0 || removed || playlists_changed {
        index::save(&library.tracks, &library.playlists, number);
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
    /// The M3U and M3U8 playlists, in natural order of their paths.
    pub playlists: Vec<PathBuf>,
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
        playlists: Vec::new(),
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
            if !entry.file_type().is_file() {
                continue;
            }
            if crate::filetypes::is_playlist(entry.path()) {
                if seen.insert(entry.path().to_path_buf()) {
                    walked.playlists.push(entry.into_path());
                }
                continue;
            }
            if !crate::filetypes::is_audio(entry.path()) {
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
        .playlists
        .sort_by(|a, b| crate::format::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
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
    // A file read again keeps when it came; a new one came now, but on a
    // library's first scan (nothing known) its own time stands in.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let first_scan = known.is_empty();
    let read = |f: &&Found| -> Option<Track> {
        if cancel.load(Relaxed) {
            return None;
        }
        let track = crate::guard::catch(&f.path, || tags::read(&f.path, f.size, f.mtime))
            .flatten()
            .map(|mut t| {
                t.added = match known.get(f.path.as_path()) {
                    Some(old) => old.added_or_mtime(),
                    None if first_scan => f.mtime,
                    None => now,
                };
                t
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walk_finds_the_playlists_too() {
        let dir = std::env::temp_dir().join(format!("metalamp_walk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["a.mp3", "B.M3U8", "sub/c.m3u", "notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let walked = walk(std::slice::from_ref(&dir), &AtomicBool::new(false));
        let names = |paths: Vec<&Path>| -> Vec<String> {
            paths
                .iter()
                .map(|p| {
                    p.strip_prefix(&dir)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect()
        };
        assert_eq!(
            names(walked.found.iter().map(|f| f.path.as_path()).collect()),
            ["a.mp3"]
        );
        assert_eq!(
            names(walked.playlists.iter().map(PathBuf::as_path).collect()),
            ["B.M3U8", "sub/c.m3u"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
