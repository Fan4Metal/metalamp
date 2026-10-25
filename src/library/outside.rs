//! The file heard when the library does not hold it (opened from Explorer,
//! Ctrl+O, dropped on the window): its tags, properties and cover read on
//! a thread of their own, so the player bar shows them as for a track of
//! the library.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
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

fn read_track(path: &Path) -> Option<Track> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    tags::read(path, meta.len(), mtime)
}
