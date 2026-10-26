//! The file heard when the library does not hold it (opened from Explorer,
//! Ctrl+O, dropped on the window): its tags, properties and cover read on
//! a thread of their own, so the player bar shows them as for a track of
//! the library.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::UNIX_EPOCH;

use egui::{TextureHandle, TextureOptions};

use super::{Track, tags};

/// The cover's largest side as read, in pixels: the playlist's cover is
/// up to 360 points wide.
const COVER_PX: u32 = 512;

/// What was read of a file: its track, its cover and the cover's colours.
type Read = (
    Option<Track>,
    Option<egui::ColorImage>,
    Option<[egui::Color32; 3]>,
);

#[derive(Default)]
pub struct Outside {
    /// The file asked for last.
    path: Option<PathBuf>,
    /// Its track once read.
    track: Option<Track>,
    /// Its cover once read; the one before meanwhile (the next file of a
    /// folder shows no gap).
    cover: Option<TextureHandle>,
    /// The cover's colours (`palette::cover_colours`), kept as the cover.
    colours: Option<[egui::Color32; 3]>,
    rx: Option<Receiver<Read>>,
}

impl Outside {
    /// Follows the file heard that the library does not hold (`None`:
    /// none, or one it holds): a new one is read on a thread.
    pub fn follow(&mut self, ctx: &egui::Context, path: Option<&Path>) {
        if let Some(rx) = &self.rx
            && let Ok((track, image, colours)) = rx.try_recv()
        {
            self.rx = None;
            self.track = track;
            self.colours = colours;
            self.cover =
                image.map(|i| ctx.load_texture("outside_cover", i, TextureOptions::LINEAR));
        }
        if self.path.as_deref() == path {
            return;
        }
        self.path = path.map(Path::to_path_buf);
        self.track = None;
        self.rx = None;
        let Some(path) = path else {
            self.cover = None;
            self.colours = None;
            return;
        };
        let (tx, rx) = channel();
        let path = path.to_path_buf();
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("outside".into())
            .spawn(move || {
                let read = crate::guard::catch(&path, || {
                    let cover = crate::covers::large(&path, COVER_PX);
                    let colours = cover
                        .as_ref()
                        .and_then(|c| crate::palette::cover_colours(&c.pixels));
                    (read_track(&path), cover, colours)
                })
                .unwrap_or_default();
                let _ = tx.send(read);
                wake.request_repaint();
            });
        if spawned.is_ok() {
            self.rx = Some(rx);
        }
    }

    /// The track of `path` once read.
    pub fn track(&self, path: &Path) -> Option<&Track> {
        self.track.as_ref().filter(|t| t.path == path)
    }

    /// The cover of the file followed.
    pub fn cover(&self) -> Option<TextureHandle> {
        self.cover.clone()
    }

    /// The colours of the cover of the file followed (the one before's
    /// while it is read).
    pub fn colours(&self) -> Option<[egui::Color32; 3]> {
        self.colours
    }

    /// Reads the file followed again (its tags were written).
    pub fn read_again(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.path.take() {
            self.follow(ctx, Some(&path));
        }
    }
}

/// The tracks of the playlist's files the library does not hold, read one
/// after another on a thread of their own as the playlist asks for them,
/// so its rows show their lines by the template and their lengths.
#[derive(Default)]
pub struct Loose {
    /// Each file asked for and still in the playlist: its track once read
    /// (`None` meanwhile, or when it could not be read).
    tracks: HashMap<PathBuf, Option<Track>>,
    /// Raised whenever a track comes in or the tracks are dropped.
    pub revision: u64,
    worker: Option<(Sender<PathBuf>, Receiver<Loaded>)>,
}

/// A file read by `Loose`'s thread and its track.
type Loaded = (PathBuf, Option<Track>);

impl Loose {
    /// Takes the tracks read since the frame before.
    pub fn take_read(&mut self) {
        let Some((_, rx)) = &self.worker else {
            return;
        };
        while let Ok((path, track)) = rx.try_recv() {
            // A file taken out of the playlist meanwhile is not kept.
            if let Some(slot) = self.tracks.get_mut(&path) {
                *slot = track;
                self.revision += 1;
            }
        }
    }

    /// The track of `path` once read; a file not asked for yet is read.
    pub fn track(&mut self, ctx: &egui::Context, path: &Path) -> Option<&Track> {
        if !self.tracks.contains_key(path) {
            self.tracks.insert(path.to_path_buf(), None);
            self.ask(ctx, path);
        }
        self.tracks.get(path)?.as_ref()
    }

    fn ask(&mut self, ctx: &egui::Context, path: &Path) {
        if self.worker.is_none() {
            let (tx, paths) = channel::<PathBuf>();
            let (done, rx) = channel();
            let wake = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("loose".into())
                .spawn(move || {
                    // Ends once the sender is dropped with the app.
                    for path in paths {
                        let track = crate::guard::catch(&path, || read_track(&path)).flatten();
                        if done.send((path, track)).is_err() {
                            break;
                        }
                        wake.request_repaint();
                    }
                });
            if spawned.is_err() {
                return;
            }
            self.worker = Some((tx, rx));
        }
        if let Some((tx, _)) = &self.worker {
            let _ = tx.send(path.to_path_buf());
        }
    }

    /// Keeps only the files `kept` says are still in the playlist.
    pub fn retain(&mut self, kept: impl Fn(&Path) -> bool) {
        self.tracks.retain(|p, _| kept(p));
    }

    /// Drops every track read, to read them again (tags were written).
    pub fn forget(&mut self) {
        self.tracks.clear();
        self.revision += 1;
    }
}

fn read_track(path: &Path) -> Option<Track> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    tags::read(path, meta.len(), mtime)
}
