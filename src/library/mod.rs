//! The media library: the tracks of the music folders with their tags,
//! grouped into artists and albums.
//!
//! The scan (`scan.rs`) reads the tags (`tags.rs`) and builds a
//! [`Library`] on its own thread; the UI only swaps in the new `Arc`. The
//! index (`index.rs`) keeps the tracks between runs, so a start shows the
//! library at once and a rescan reads only the files that changed.

pub mod details;
pub mod edit;
pub mod filter;
pub mod index;
pub mod outside;
pub mod scan;
pub mod tags;
pub mod watch;

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
    /// When the file came into the library (Unix seconds): the scan that
    /// first found it, kept when it is read again; the file's time for the
    /// tracks of an index written before this was kept, and for the first
    /// scan of a library (which would make all of it new). 0: not known.
    pub added: u64,
    /// The artist its album is filed under when the tags leave it out
    /// ([`filed_without_album_artist`]): [`VARIOUS_ARTISTS`] for a
    /// compilation, the artist for an album with guests. Worked out by
    /// `Library::new` each time, not kept.
    #[serde(skip)]
    pub filed: Option<String>,
}

/// The artist a compilation without an album artist is filed under: the
/// name taggers and players use, in every language.
pub const VARIOUS_ARTISTS: &str = "Various Artists";

impl Track {
    /// When it came into the library, or the file's time if not known.
    pub fn added_or_mtime(&self) -> u64 {
        if self.added > 0 {
            self.added
        } else {
            self.mtime
        }
    }
}

impl Track {
    /// The artist an album is filed under: the album artist, else the
    /// track's artist.
    pub fn filed_artist(&self) -> &str {
        if let Some(filed) = &self.filed {
            filed
        } else if self.album_artist.trim().is_empty() {
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
    /// whatever their years (a compilation's tracks often carry the years
    /// of their first release), unless they are editions in folders of
    /// their own ([`editions`]): then the edition's year follows.
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
    /// Each track by its path, and by its path in lower case (`track_at`).
    pub by_path: HashMap<PathBuf, TrackId>,
    by_folded: HashMap<String, TrackId>,
    /// The M3U and M3U8 playlists in the library's folders, by path (the
    /// playlist's saved playlists section lists them after its own).
    pub playlists: Vec<PathBuf>,
    /// This library among those of the run (each scan makes new ones;
    /// 0: an empty one): what was built for one is not of another. Not
    /// its address, which a later one may take once it is dropped.
    pub generation: u64,
}

/// A path in lower case, for lookups that ignore its case as Windows does.
fn folded(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
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
        let filed = filed_without_album_artist(&tracks);
        for (t, f) in tracks.iter_mut().zip(filed) {
            t.filed = f;
        }
        let mut albums: Vec<Album> = Vec::new();
        let mut album_index: HashMap<String, AlbumId> = HashMap::new();
        let mut album_of = Vec::with_capacity(tracks.len());
        let mut totals = Totals::default();
        let editions = editions(&tracks);
        for (id, t) in tracks.iter().enumerate() {
            let artist = t.filed_artist().trim().to_owned();
            let mut key = album_key(t);
            if let Some(year) = editions.get(&(key.clone(), folder_of(t))) {
                key = format!("{key}\u{1}{year}");
            }
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
                search_form(&format!(
                    "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
                    t.artist, t.album_artist, t.album, t.title, t.genre, year
                ))
            })
            .collect();
        let by_path: HashMap<PathBuf, TrackId> = tracks
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
            by_folded: by_path.iter().map(|(p, &id)| (folded(p), id)).collect(),
            by_path,
            playlists: Vec::new(),
            generation: {
                static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
            },
        }
    }

    /// The track at `path`, its case as Windows sees it, ignored: a
    /// playlist may write `21st century` for the folder `21st Century`.
    pub fn track_at(&self, path: &Path) -> Option<TrackId> {
        self.by_path
            .get(path)
            .or_else(|| self.by_folded.get(&folded(path)))
            .copied()
    }

    /// `path` as the library writes it, if it holds the file (its case
    /// may differ), else as it is.
    pub fn path_as_known(&self, path: PathBuf) -> PathBuf {
        match self.track_at(&path) {
            Some(id) if self.tracks[id as usize].path != path => {
                self.tracks[id as usize].path.clone()
            }
            _ => path,
        }
    }

    /// The library with `playlists`, found in its folders.
    pub fn with_playlists(mut self, playlists: Vec<PathBuf>) -> Library {
        self.playlists = playlists;
        self
    }

    /// Whether every word of `query` (already in lower case, split on
    /// spaces) is in the track's search text.
    pub fn matches(&self, track: TrackId, words: &[String]) -> bool {
        let text = &self.search[track as usize];
        words.iter().all(|w| text.contains(w.as_str()))
    }
}

/// Text as the search compares it: lower case, and "ё" as "е" (both are
/// typed as "е" more often than not).
pub fn search_form(text: &str) -> String {
    text.to_lowercase().replace('ё', "е")
}

/// The words of a search query, in [`search_form`].
pub fn query_words(query: &str) -> Vec<String> {
    query.split_whitespace().map(search_form).collect()
}

/// The artist to file tracks under whose tags have no album artist, where
/// the track's artist would split their album (one album per artist): the
/// tracks of a folder sharing an album title (not empty), none of them
/// with an album artist. By at least three main artists of whom none has
/// more than half of them (at least three tracks): a compilation, filed
/// under [`VARIOUS_ARTISTS`]. By one main artist under several names (an
/// album with guests: "A", "A feat. B", "A & C"): filed under that artist,
/// as one of its tracks names it alone (else as the first names it). The
/// main artist is the name before "feat.", "&", "," and the like.
fn filed_without_album_artist(tracks: &[Track]) -> Vec<Option<String>> {
    let mut groups: HashMap<(PathBuf, String), Vec<usize>> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        let album = t.album.trim().to_lowercase();
        if album.is_empty() {
            continue;
        }
        groups.entry((folder_of(t), album)).or_default().push(i);
    }
    let mut filed = vec![None; tracks.len()];
    for ids in groups.values() {
        if ids.len() < 2
            || ids
                .iter()
                .any(|&i| !tracks[i].album_artist.trim().is_empty())
        {
            continue;
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        for &i in ids {
            *counts.entry(main_artist(&tracks[i].artist)).or_default() += 1;
        }
        let most = counts.values().copied().max().unwrap_or(0);
        let name = if ids.len() >= 3 && counts.len() >= 3 && most * 2 <= ids.len() {
            Some(VARIOUS_ARTISTS.to_owned())
        } else if counts.len() == 1 {
            // One artist; under several names only with guests.
            let names: std::collections::HashSet<String> = ids
                .iter()
                .map(|&i| tracks[i].artist.trim().to_lowercase())
                .collect();
            (names.len() > 1).then(|| {
                let main = counts.keys().next().cloned().unwrap_or_default();
                ids.iter()
                    .map(|&i| tracks[i].artist.trim())
                    .find(|a| a.to_lowercase() == main)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        // None names it alone: what the names begin with,
                        // before a guest; not the main artist cut at its
                        // own "&" or "," (Earth, Wind & Fire).
                        let names: Vec<&str> =
                            ids.iter().map(|&i| tracks[i].artist.trim()).collect();
                        let shared = shared_name(&names);
                        if shared.is_empty() {
                            let first = names[0];
                            first.get(..main.len()).unwrap_or(first).trim().to_owned()
                        } else {
                            shared
                        }
                    })
            })
        } else {
            None
        };
        if let Some(name) = name {
            for &i in ids {
                filed[i] = Some(name.clone());
            }
        }
    }
    filed
}

/// What every name of `names` begins with (whatever the case), cut before
/// a guest ("feat.", "with", "vs") and the signs left at its end: "Edguy"
/// of "Edguy feat. A" and "Edguy feat. B", "Earth, Wind & Fire" of it and
/// "Earth, Wind & Fire feat. The Emotions"; empty when nothing is shared.
fn shared_name(names: &[&str]) -> String {
    let Some((first, rest)) = names.split_first() else {
        return String::new();
    };
    let chars = rest
        .iter()
        .map(|n| {
            first
                .chars()
                .zip(n.chars())
                .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                .count()
        })
        .min()
        .unwrap_or(first.chars().count());
    let end = first
        .char_indices()
        .nth(chars)
        .map_or(first.len(), |(i, _)| i);
    let mut shared = &first[..end];
    // Not into a word ("Above & Beyond f" of "feat." and "ft.").
    if first[end..].starts_with(char::is_alphanumeric) {
        let word = shared
            .char_indices()
            .rfind(|(_, c)| !c.is_alphanumeric())
            .map_or(0, |(i, c)| i + c.len_utf8());
        shared = &shared[..word];
    }
    // ASCII lower case keeps the places of the bytes.
    let lower = shared.to_ascii_lowercase();
    let cut = [
        " feat.",
        " feat ",
        " ft.",
        " ft ",
        " featuring ",
        " vs.",
        " vs ",
        " with ",
    ]
    .iter()
    .filter_map(|sep| lower.find(sep))
    .min()
    .unwrap_or(shared.len());
    shared[..cut]
        .trim_end_matches(|c: char| c.is_whitespace() || ",&/;-(".contains(c))
        .trim()
        .to_owned()
}

/// The artist before any guests: "A feat. B", "A & B", "A, B" are "a".
fn main_artist(artist: &str) -> String {
    let lower = artist.trim().to_lowercase();
    let cut = [
        " feat.",
        " feat ",
        " ft.",
        " ft ",
        " featuring ",
        " & ",
        " and ",
        ", ",
        " x ",
        " vs.",
        " vs ",
        " with ",
        " / ",
        ";",
    ]
    .iter()
    .filter_map(|sep| lower.find(sep))
    .min()
    .unwrap_or(lower.len());
    lower[..cut].trim().to_owned()
}

/// The lower-case filed artist and album title of `t`'s album.
fn album_key(t: &Track) -> String {
    format!(
        "{}\u{1}{}",
        t.filed_artist().trim().to_lowercase(),
        t.album.trim().to_lowercase()
    )
}

fn folder_of(t: &Track) -> PathBuf {
    t.path.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Albums of one artist and title in folders of their own and of other
/// years (a Greatest Hits of 1990 and one of 2005, a remaster beside the
/// original) are editions, told apart by the year: the edition's year of
/// each such (album key, folder). Folders of one album stay one: those of
/// one year, those of other discs (CD1, CD2 of a compilation whose tracks
/// carry other years), and a folder without years.
fn editions(tracks: &[Track]) -> HashMap<(String, PathBuf), u16> {
    use std::collections::{BTreeMap, BTreeSet};
    // Each album key's folders: the earliest year and the discs in each.
    let mut folders: HashMap<String, BTreeMap<PathBuf, (u16, BTreeSet<u16>)>> = HashMap::new();
    for t in tracks {
        let f = folders
            .entry(album_key(t))
            .or_default()
            .entry(folder_of(t))
            .or_insert((0, BTreeSet::new()));
        if t.year > 0 && (f.0 == 0 || t.year < f.0) {
            f.0 = t.year;
        }
        // Discs not tagged: the folder's name may tell (CD1, CD2 of a
        // compilation whose tracks carry their years of release).
        let disc = match t.disc {
            0 => t
                .path
                .parent()
                .and_then(Path::file_name)
                .and_then(|n| disc_in_name(&n.to_string_lossy()))
                .unwrap_or(0),
            d => d,
        };
        f.1.insert(disc);
    }
    let mut out = HashMap::new();
    for (key, folders) in folders {
        if folders.len() < 2 {
            continue;
        }
        // Parts: a year and the discs seen in it.
        let mut parts: Vec<(u16, BTreeSet<u16>)> = Vec::new();
        let mut part_of: Vec<(PathBuf, usize)> = Vec::new();
        let mut by_year: Vec<(&PathBuf, &(u16, BTreeSet<u16>))> = folders.iter().collect();
        by_year.sort_by_key(|(_, (year, _))| *year == 0);
        for (folder, (year, discs)) in by_year {
            let joins = parts
                .iter()
                .position(|(y, d)| *year == 0 || *y == *year || d.is_disjoint(discs));
            let i = match joins {
                Some(i) => i,
                None => {
                    parts.push((*year, BTreeSet::new()));
                    parts.len() - 1
                }
            };
            parts[i].1.extend(discs.iter().copied());
            part_of.push((folder.clone(), i));
        }
        if parts.len() > 1 {
            for (folder, i) in part_of {
                out.insert((key.clone(), folder), parts[i].0);
            }
        }
    }
    out
}

/// The disc a folder's name gives: "CD1", "CD 2", "Disc 3 - Bonus",
/// "disk_4", "Диск 1".
fn disc_in_name(name: &str) -> Option<u16> {
    let lower = name.trim().to_lowercase();
    let rest = ["cd", "disc", "disk", "диск"]
        .iter()
        .find_map(|p| lower.strip_prefix(p))?;
    let rest = rest.trim_start_matches([' ', '_', '-', '.', '#']);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let after = rest[digits.len()..].chars().next();
    if after.is_some_and(char::is_alphanumeric) {
        return None;
    }
    digits.parse().ok().filter(|&n| n > 0)
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

    #[test]
    fn a_track_is_found_whatever_the_case_of_its_path() {
        let t = |path: &str| Track {
            path: path.into(),
            title: "Lady Unforgettable".into(),
            ..Track::default()
        };
        let lib = Library::new(vec![t("D:/Music/Blue System/(1994) 21st Century/08.mp3")]);
        let written = PathBuf::from("D:/Music/Blue System/(1994) 21st century/08.mp3");
        assert_eq!(lib.track_at(&written), Some(0));
        assert_eq!(
            lib.path_as_known(written),
            PathBuf::from("D:/Music/Blue System/(1994) 21st Century/08.mp3")
        );
        let other = PathBuf::from("D:/Music/other.mp3");
        assert_eq!(lib.track_at(&other), None);
        assert_eq!(lib.path_as_known(other.clone()), other);
    }

    fn in_folder(folder: &str, artist: &str, album: &str, n: u16) -> Track {
        Track {
            path: PathBuf::from(format!(r"D:\Music\{folder}\{n:02}.mp3")),
            artist: artist.into(),
            album: album.into(),
            track: n,
            ..Track::default()
        }
    }

    #[test]
    fn albums_without_an_album_artist_are_not_split_by_artist() {
        let comp = ["Abba", "Queen", "Toto", "Abba feat. X"]
            .iter()
            .enumerate()
            .map(|(i, a)| in_folder("Hits", a, "Hits of the 80s", i as u16 + 1));
        // An artist's album with guests stays the artist's.
        let guests = ["Ayreon", "Ayreon feat. Floor", "Ayreon & Hansi", "Ayreon"]
            .iter()
            .enumerate()
            .map(|(i, a)| in_folder("Ayreon", a, "01011001", i as u16 + 1));
        // Guests on every track: the name before them.
        let all_guests = ["Edguy feat. A", "Edguy feat. B"]
            .iter()
            .enumerate()
            .map(|(i, a)| in_folder("Edguy", a, "Mandrake", i as u16 + 1));
        let lib = Library::new(comp.chain(guests).chain(all_guests).collect());
        assert_eq!(lib.albums.len(), 3, "{:?}", lib.albums);
        let names: Vec<&str> = lib.artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Ayreon", "Edguy", VARIOUS_ARTISTS]);
        // One with an album artist is left as tagged.
        let mut tagged: Vec<Track> = ["Abba", "Queen", "Toto"]
            .iter()
            .enumerate()
            .map(|(i, a)| in_folder("Tagged", a, "Mix", i as u16 + 1))
            .collect();
        tagged[0].album_artist = "DJ".into();
        let lib = Library::new(tagged);
        assert_eq!(lib.albums.len(), 3);
        assert_eq!(main_artist("Hans Zimmer & Lisa Gerrard"), "hans zimmer");
        // A band with "&" or "," in its name keeps it whole.
        let band = [
            "Earth, Wind & Fire",
            "Earth, Wind & Fire feat. The Emotions",
            "Earth, Wind & Fire",
        ]
        .iter()
        .enumerate()
        .map(|(i, a)| in_folder("EWF", a, "Faces", i as u16 + 1));
        let all_guests = ["Above & Beyond feat. A", "Above & Beyond ft. B"]
            .iter()
            .enumerate()
            .map(|(i, a)| in_folder("AB", a, "Group Therapy", i as u16 + 1));
        let lib = Library::new(band.chain(all_guests).collect());
        let names: Vec<&str> = lib.artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Above & Beyond", "Earth, Wind & Fire"]);
        assert_eq!(lib.albums.len(), 2);
        assert_eq!(shared_name(&["Ария", "АРИЯ feat. Кипелов"]), "Ария");
        assert_eq!(shared_name(&["Abba", "Queen"]), "");
    }

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
    fn editions_in_folders_of_their_own_are_two_albums() {
        let at = |path: &str, year: u16, disc: u16| Track {
            disc,
            ..track(path, "Band", "", "Greatest Hits", year, 1)
        };
        // Two editions: other folders, other years.
        let lib = Library::new(vec![
            at("a/1990/1.mp3", 1990, 0),
            at("a/1990/2.mp3", 1988, 0),
            at("a/2005/1.mp3", 2005, 0),
        ]);
        let years: Vec<u16> = lib.albums.iter().map(|a| a.year).collect();
        assert_eq!(years.len(), 2);
        assert!(years.contains(&1988) && years.contains(&2005));
        assert_ne!(lib.albums[0].key, lib.albums[1].key);
        // One album: its discs in folders of their own, whatever the years;
        // and a folder without years.
        let lib = Library::new(vec![
            at("b/CD1/1.mp3", 1985, 1),
            at("b/CD2/1.mp3", 1991, 2),
            at("b/extra/1.mp3", 0, 0),
        ]);
        assert_eq!(lib.albums.len(), 1);
        // One folder: one album, whatever the years.
        let lib = Library::new(vec![at("c/1.mp3", 1970, 0), at("c/2.mp3", 1999, 0)]);
        assert_eq!(lib.albums.len(), 1);
        // Discs in folders named for them, not tagged: one album.
        let lib = Library::new(vec![
            at("d/CD1/1.mp3", 1985, 0),
            at("d/CD 2/1.mp3", 1979, 0),
        ]);
        assert_eq!(lib.albums.len(), 1);
        assert_eq!(disc_in_name("Disc 3 - Bonus"), Some(3));
        assert_eq!(disc_in_name("Диск 1"), Some(1));
        assert_eq!(disc_in_name("CD1"), Some(1));
        assert_eq!(disc_in_name("CDX"), None);
        assert_eq!(disc_in_name("Discography"), None);
        assert_eq!(disc_in_name("1990"), None);
    }

    #[test]
    fn search_needs_every_word() {
        let lib = Library::new(vec![
            track("a/1.mp3", "Ария", "", "Химера", 2001, 1),
            track("b/1.mp3", "Ария", "", "Крещение огнём", 2003, 1),
        ]);
        assert!(lib.matches(0, &query_words("ария химера")));
        assert!(lib.matches(0, &query_words("2001")));
        assert!(!lib.matches(0, &query_words("ария кипелов")));
        assert!(lib.matches(1, &query_words("огнем")));
        assert!(lib.matches(1, &query_words("ОГНЁМ")));
    }
}
