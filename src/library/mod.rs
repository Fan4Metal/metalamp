//! The media library: the tracks of the music folders with their tags,
//! grouped into artists and albums as Winamp's media library shows them.
//!
//! The scan (`scan.rs`) reads the tags (`tags.rs`) and builds a
//! [`Library`] on its own thread; the UI only swaps in the new `Arc`. The
//! index (`index.rs`) keeps the tracks between runs, so a start shows the
//! library at once and a rescan reads only the files that changed.

pub mod index;
pub mod scan;
pub mod tags;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::format::natural_cmp;

/// A track's index in [`Library::tracks`].
pub type TrackId = u32;
/// An album's index in [`Library::albums`].
pub type AlbumId = u32;
/// An artist's index in [`Library::artists`].
pub type ArtistId = u32;

/// One audio file with its tags and properties. Numbers are 0 where the
/// file has none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub path: PathBuf,
    /// The file's size and modification time (Unix seconds) when its tags
    /// were read: a rescan reads the file again only when these changed.
    pub size: u64,
    pub mtime: u64,
    pub title: String,
    pub artist: String,
    pub album_artist: String,
    pub album: String,
    pub year: u16,
    pub track: u16,
    pub disc: u16,
    pub genre: String,
    pub duration_ms: u32,
    /// Kilobits per second.
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u8,
    /// The tag holds a picture (the album's cover can come from it).
    pub has_cover: bool,
}

impl Track {
    /// The artist an album is filed under: the album artist, else the
    /// track's artist.
    pub fn filed_artist(&self) -> &str {
        if self.album_artist.trim().is_empty() {
            &self.artist
        } else {
            &self.album_artist
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Album {
    pub artist: String,
    pub title: String,
    /// The earliest year of its tracks, 0 if none has one.
    pub year: u16,
    /// The genre of its first track.
    pub genre: String,
    /// In the order of the album: disc, track number, then the file name.
    pub tracks: Vec<TrackId>,
    pub length_ms: u64,
    pub size: u64,
    /// The latest modification time of its files (Unix seconds).
    pub updated: u64,
    /// The folder of its first track.
    pub folder: PathBuf,
    /// Lower-case artist and title: tracks are one album when these match,
    /// as in Winamp's media library, whatever their years (a compilation's
    /// tracks often carry the years of their first release).
    pub key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Artist {
    pub name: String,
    /// By year, then title.
    pub albums: Vec<AlbumId>,
    pub tracks: u32,
}

/// The whole library's sums, for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub tracks: u64,
    pub length_ms: u64,
    pub bytes: u64,
}

#[derive(Debug, Default)]
pub struct Library {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    /// By name, ignoring case, digits by value.
    pub artists: Vec<Artist>,
    pub totals: Totals,
    /// Each track's album.
    pub album_of: Vec<AlbumId>,
    /// Each track's words for the search: artist, album artist, album,
    /// title, genre and year in lower case.
    pub search: Vec<String>,
    /// Each track by its path.
    pub by_path: HashMap<PathBuf, TrackId>,
}

/// The name shown for an empty artist or album.
pub fn unknown_artist() -> &'static str {
    tr!("Unknown artist", "Неизвестный исполнитель")
}

impl Library {
    /// Groups `tracks` into albums and artists.
    pub fn new(mut tracks: Vec<Track>) -> Library {
        // A stable order of tracks makes the grouping deterministic.
        tracks.sort_by(|a, b| a.path.cmp(&b.path));
        let mut albums: Vec<Album> = Vec::new();
        let mut album_index: HashMap<String, AlbumId> = HashMap::new();
        let mut album_of = Vec::with_capacity(tracks.len());
        let mut totals = Totals::default();
        for (id, t) in tracks.iter().enumerate() {
            let artist = t.filed_artist().trim().to_owned();
            let key = format!(
                "{}\u{1}{}",
                artist.to_lowercase(),
                t.album.trim().to_lowercase()
            );
            let album = *album_index.entry(key.clone()).or_insert_with(|| {
                albums.push(Album {
                    artist: artist.clone(),
                    title: t.album.trim().to_owned(),
                    year: t.year,
                    genre: t.genre.clone(),
                    tracks: Vec::new(),
                    length_ms: 0,
                    size: 0,
                    updated: 0,
                    folder: t.path.parent().map(Path::to_path_buf).unwrap_or_default(),
                    key,
                });
                (albums.len() - 1) as AlbumId
            });
            let a = &mut albums[album as usize];
            a.tracks.push(id as TrackId);
            a.length_ms += u64::from(t.duration_ms);
            a.size += t.size;
            a.updated = a.updated.max(t.mtime);
            if t.year > 0 && (a.year == 0 || t.year < a.year) {
                a.year = t.year;
            }
            album_of.push(album);
            totals.tracks += 1;
            totals.length_ms += u64::from(t.duration_ms);
            totals.bytes += t.size;
        }
        for a in &mut albums {
            a.tracks
                .sort_by(|&x, &y| album_order(&tracks[x as usize], &tracks[y as usize]));
            if let Some(&first) = a.tracks.first() {
                let t = &tracks[first as usize];
                a.folder = t.path.parent().map(Path::to_path_buf).unwrap_or_default();
                a.genre = t.genre.clone();
            }
        }

        let mut artist_index: HashMap<String, ArtistId> = HashMap::new();
        let mut artists: Vec<Artist> = Vec::new();
        for (id, a) in albums.iter().enumerate() {
            let artist = *artist_index
                .entry(a.artist.to_lowercase())
                .or_insert_with(|| {
                    artists.push(Artist {
                        name: a.artist.clone(),
                        albums: Vec::new(),
                        tracks: 0,
                    });
                    (artists.len() - 1) as ArtistId
                });
            let artist = &mut artists[artist as usize];
            artist.albums.push(id as AlbumId);
            artist.tracks += a.tracks.len() as u32;
        }
        for artist in &mut artists {
            artist.albums.sort_by(|&x, &y| {
                let (x, y) = (&albums[x as usize], &albums[y as usize]);
                x.year
                    .cmp(&y.year)
                    .then_with(|| natural_cmp(&x.title, &y.title))
            });
        }
        // An empty name (no tag, no folder to take it from) goes last.
        artists.sort_by(|a, b| match (a.name.is_empty(), b.name.is_empty()) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => natural_cmp(&a.name, &b.name),
        });

        let search = tracks
            .iter()
            .map(|t| {
                let year = if t.year > 0 {
                    t.year.to_string()
                } else {
                    String::new()
                };
                format!(
                    "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
                    t.artist, t.album_artist, t.album, t.title, t.genre, year
                )
                .to_lowercase()
            })
            .collect();
        let by_path = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.path.clone(), i as TrackId))
            .collect();
        Library {
            tracks,
            albums,
            artists,
            totals,
            album_of,
            search,
            by_path,
        }
    }

    /// Whether every word of `query` (already in lower case, split on
    /// spaces) is in the track's search text.
    pub fn matches(&self, track: TrackId, words: &[String]) -> bool {
        let text = &self.search[track as usize];
        words.iter().all(|w| text.contains(w.as_str()))
    }
}

/// The order of tracks within an album: disc, track number, then the file
/// name as Explorer sorts it.
fn album_order(a: &Track, b: &Track) -> Ordering {
    a.disc
        .cmp(&b.disc)
        .then_with(|| a.track.cmp(&b.track))
        .then_with(|| {
            let name = |t: &Track| {
                t.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            natural_cmp(&name(a), &name(b))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(
        path: &str,
        artist: &str,
        album_artist: &str,
        album: &str,
        year: u16,
        n: u16,
    ) -> Track {
        Track {
            path: PathBuf::from(path),
            title: format!("t{n}"),
            artist: artist.into(),
            album_artist: album_artist.into(),
            album: album.into(),
            year,
            track: n,
            duration_ms: 1000,
            size: 10,
            ..Track::default()
        }
    }

    #[test]
    fn groups_by_album_artist_then_artist() {
        let lib = Library::new(vec![
            track("c/1.mp3", "Guest", "Ария", "Крещение Огнём", 2003, 2),
            track("c/0.mp3", "Ария", "", "Крещение Огнём", 2003, 1),
            track("d/1.mp3", "ABBA", "", "Gold", 1992, 1),
        ]);
        assert_eq!(lib.albums.len(), 2);
        assert_eq!(lib.artists.len(), 2);
        assert_eq!(lib.artists[0].name, "ABBA");
        assert_eq!(lib.artists[1].name, "Ария");
        let aria = &lib.albums[lib.artists[1].albums[0] as usize];
        assert_eq!(aria.tracks.len(), 2);
        // Disc and track order, not path order.
        assert_eq!(lib.tracks[aria.tracks[0] as usize].track, 1);
        assert_eq!(lib.totals.tracks, 3);
        assert_eq!(lib.totals.length_ms, 3000);
        assert_eq!(lib.totals.bytes, 30);
    }

    #[test]
    fn one_album_whatever_the_years_of_its_tracks() {
        let lib = Library::new(vec![
            track("a/1.mp3", "Король и Шут", "", "Избранное", 1999, 2),
            track("a/2.mp3", "Король и Шут", "", "Избранное", 1996, 1),
            track("a/3.mp3", "король и шут", "", "избранное", 0, 3),
            track(
                "b/1.mp3",
                "Король и Шут",
                "",
                "Акустический альбом",
                1999,
                1,
            ),
        ]);
        assert_eq!(lib.albums.len(), 2);
        assert_eq!(lib.artists.len(), 1);
        // By year within the artist; an album's year is its earliest.
        let albums: Vec<(u16, usize)> = lib.artists[0]
            .albums
            .iter()
            .map(|&a| {
                (
                    lib.albums[a as usize].year,
                    lib.albums[a as usize].tracks.len(),
                )
            })
            .collect();
        assert_eq!(albums, [(1996, 3), (1999, 1)]);
    }

    #[test]
    fn search_needs_every_word() {
        let lib = Library::new(vec![track("a/1.mp3", "Ария", "", "Химера", 2001, 1)]);
        let words = |q: &str| {
            q.split_whitespace()
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        };
        assert!(lib.matches(0, &words("ария химера")));
        assert!(lib.matches(0, &words("2001")));
        assert!(!lib.matches(0, &words("ария кипелов")));
    }
}
