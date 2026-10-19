//! What the player knows of each track beyond its tags: how many times it
//! was played, when last, and the user's rating. Kept in `stats.json` next
//! to the settings by the track's path, so the music's files are never
//! written for it (a file moved or renamed starts afresh).
//!
//! A play counts once a track has been heard for half its length or four
//! minutes, whichever comes first (the scrobblers' rule), in time actually
//! heard: a seek past the middle does not count it, a pause does not run
//! the clock. `App::count_play` feeds the clock each frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE: &str = "stats.json";
const VERSION: u32 = 1;

/// The file's writes.
static WRITES: crate::kept::Kept = crate::kept::Kept::new();

/// One track's numbers; all 0 for a track never played nor rated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stat {
    pub plays: u32,
    /// Unix seconds of the last play counted; 0: never.
    pub last: u64,
    /// 0 (none) to 5 stars.
    pub rating: u8,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct File {
    version: u32,
    /// The path as text to its numbers.
    tracks: HashMap<String, Stat>,
}

#[derive(Default)]
pub struct Stats {
    tracks: HashMap<PathBuf, Stat>,
    /// Raised by every change, so that a view built on the numbers knows
    /// it is out of date.
    pub revision: u64,
    file: Option<PathBuf>,
}

/// The longest a track must be heard for its play to count.
const COUNT_AFTER_MS: u64 = 4 * 60 * 1000;

/// How long a track of `duration_ms` must be heard for its play to count:
/// half of it, at most four minutes (a track of unknown length: the four
/// minutes, or its end).
pub fn count_after(duration_ms: u64) -> u64 {
    if duration_ms == 0 {
        COUNT_AFTER_MS
    } else {
        (duration_ms / 2).min(COUNT_AFTER_MS)
    }
}

impl Stats {
    /// Reads `stats.json` next to the settings; none, or one that cannot
    /// be read, is an empty start.
    pub fn load() -> Stats {
        let file = crate::settings::data_dir().map(|d| d.join(FILE));
        let tracks = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
            .filter(|f| f.version == VERSION)
            .map(|f| {
                f.tracks
                    .into_iter()
                    .map(|(p, s)| (PathBuf::from(p), s))
                    .collect()
            })
            .unwrap_or_default();
        Stats {
            tracks,
            revision: 0,
            file,
        }
    }

    pub fn get(&self, path: &Path) -> Stat {
        self.tracks.get(path).copied().unwrap_or_default()
    }

    /// A play of `path` counted at `now` (Unix seconds).
    pub fn played(&mut self, path: &Path, now: u64) {
        let s = self.tracks.entry(path.to_path_buf()).or_default();
        s.plays = s.plays.saturating_add(1);
        s.last = now;
        self.changed();
    }

    /// `paths` rated `rating` stars (0: no rating).
    pub fn rate(&mut self, paths: &[PathBuf], rating: u8) {
        let rating = rating.min(5);
        for path in paths {
            let s = self.tracks.entry(path.clone()).or_default();
            s.rating = rating;
        }
        // Tracks with nothing left to say are left out of the file.
        self.tracks.retain(|_, s| *s != Stat::default());
        self.changed();
    }

    fn changed(&mut self) {
        self.revision += 1;
        self.save_in_background();
    }

    /// Writes the file on a thread of its own; a later write wins.
    fn save_in_background(&self) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let content = File {
            version: VERSION,
            tracks: self
                .tracks
                .iter()
                .map(|(p, s)| (p.to_string_lossy().into_owned(), *s))
                .collect(),
        };
        let number = WRITES.ask();
        std::thread::spawn(move || match serde_json::to_vec(&content) {
            Ok(bytes) => {
                if let Err(e) = WRITES.write(number, &file, &bytes) {
                    log::warn!("cannot save {}: {e}", file.display());
                }
            }
            Err(e) => log::warn!("cannot write the statistics: {e}"),
        });
    }
}

/// The clock of the track heard: how long it has been heard since it
/// started, and whether its play was counted.
#[derive(Default)]
pub struct Listening {
    /// The entry heard (the player's id) and when the clock last ran.
    entry: Option<u64>,
    heard_ms: u64,
    counted: bool,
    last_tick: Option<std::time::Instant>,
}

impl Listening {
    /// A new start of a track (also the same one again, repeated): the
    /// clock from 0.
    pub fn restart(&mut self, entry: Option<u64>) {
        *self = Listening {
            entry,
            ..Listening::default()
        };
    }

    /// The clock run to now while `playing`; true once (per start) when
    /// the track of `duration_ms` has been heard long enough to count.
    pub fn tick(&mut self, playing: bool, duration_ms: u64) -> bool {
        let now = std::time::Instant::now();
        let elapsed = self.last_tick.map(|t| now.duration_since(t));
        self.last_tick = Some(now);
        if !playing || self.entry.is_none() || self.counted {
            return false;
        }
        // A frame late by more than two seconds (the window asleep) adds
        // no more than that: the sound may have been held meanwhile.
        let ms = elapsed.map_or(0, |d| d.as_millis().min(2000) as u64);
        self.heard_ms += ms;
        if self.heard_ms >= count_after(duration_ms) {
            self.counted = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_play_counts_after_half_or_four_minutes() {
        assert_eq!(count_after(3 * 60_000), 90_000);
        assert_eq!(count_after(20 * 60_000), 4 * 60_000);
        assert_eq!(count_after(0), 4 * 60_000);
    }

    #[test]
    fn plays_and_ratings() {
        let mut s = Stats::default();
        let a = PathBuf::from(r"D:\Music\a.mp3");
        s.played(&a, 100);
        s.played(&a, 200);
        assert_eq!(
            s.get(&a),
            Stat {
                plays: 2,
                last: 200,
                rating: 0
            }
        );
        s.rate(std::slice::from_ref(&a), 9);
        assert_eq!(s.get(&a).rating, 5);
        let b = PathBuf::from(r"D:\Music\b.mp3");
        s.rate(std::slice::from_ref(&b), 3);
        s.rate(std::slice::from_ref(&b), 0);
        assert!(!s.tracks.contains_key(&b));
        assert_eq!(s.revision, 5);
    }

    #[test]
    fn the_clock_counts_once_and_only_while_playing() {
        let mut l = Listening::default();
        l.restart(Some(1));
        assert!(!l.tick(true, 1000));
        l.heard_ms = 499;
        l.last_tick = Some(std::time::Instant::now() - std::time::Duration::from_millis(5));
        assert!(l.tick(true, 1000));
        assert!(!l.tick(true, 1000));
        l.restart(Some(1));
        l.heard_ms = 499;
        l.last_tick = Some(std::time::Instant::now() - std::time::Duration::from_millis(5));
        assert!(!l.tick(false, 1000));
    }
}
