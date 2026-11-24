//! The library index between runs: `library.json` next to the settings,
//! the tracks with their tags, sizes and modification times. A start shows
//! the library from it at once; the rescan after it reads only the files
//! whose size or time changed. Compact JSON (not pretty): about 300 bytes a
//! track. Also the playlists found in the folders (`playlists`, none in an
//! index from before them).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::Track;

/// Raised when the meaning of a field changes: an index of another version
/// is dropped and the library read again.
const VERSION: u32 = 3;
/// Older versions whose tracks are shown until they are read again: 1
/// took an MP3's later ID3v2 tags over its first (`tags::open`).
const READ_AGAIN: [u32; 1] = [1];
/// Older versions whose tracks are kept, the code page of their ID3v1 tags
/// (`code_page`) taken as Latin-1: 2 read ID3v1 as lofty does.
const LATIN1_ID3V1: [u32; 1] = [2];
const FILE: &str = "library.json";

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    tracks: Vec<Track>,
    #[serde(default)]
    playlists: Vec<PathBuf>,
    /// The code page the MP3s' ID3v1 tags were read in (`id3v1::code_page`);
    /// none in an index from before it was kept: Latin-1 in version 2,
    /// Windows' own after.
    #[serde(default)]
    code_page: Option<u32>,
}

/// What the index holds: the tracks, whether every file must be read again,
/// the playlists.
pub struct Loaded {
    pub tracks: Vec<Track>,
    pub stale: bool,
    pub playlists: Vec<PathBuf>,
}

fn path() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join(FILE))
}

/// The tracks of the last scan, and whether every file must be read
/// again (an index of a version in `READ_AGAIN`); none without an index
/// or with one of another version.
pub fn load() -> Loaded {
    let none = || Loaded {
        tracks: Vec::new(),
        stale: false,
        playlists: Vec::new(),
    };
    let Some(path) = path() else {
        return none();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return none();
    };
    // Tracks of an index from before `added` was kept: the file's time.
    let dated = |mut tracks: Vec<Track>| {
        for t in &mut tracks {
            if t.added == 0 {
                t.added = t.mtime;
            }
        }
        tracks
    };
    let index = match serde_json::from_slice::<Index>(&bytes) {
        Ok(index)
            if index.version == VERSION
                || READ_AGAIN.contains(&index.version)
                || LATIN1_ID3V1.contains(&index.version) =>
        {
            index
        }
        Ok(_) => return none(),
        Err(e) => {
            log::warn!("cannot read {}: {e}", path.display());
            return none();
        }
    };
    let read_in = index.code_page.unwrap_or(if LATIN1_ID3V1.contains(&index.version) {
        super::id3v1::LATIN1
    } else {
        // SAFETY: no arguments.
        unsafe { windows_sys::Win32::Globalization::GetACP() }
    });
    let mut tracks = dated(index.tracks);
    let now = super::id3v1::code_page();
    if read_in != now {
        let n = read_again_in(&mut tracks, read_in);
        log::debug!("ID3v1 read in {read_in}, now {now}: {n} MP3s to read again");
    }
    Loaded {
        tracks,
        stale: READ_AGAIN.contains(&index.version),
        playlists: index.playlists,
    }
}

/// Forgets the time of each MP3 whose text may have come of an ID3v1 tag
/// read in the code page `was` (`may_be_id3v1`), so the scan reads it again
/// (its `added` is set by then, and kept); how many.
fn read_again_in(tracks: &mut [Track], was: u32) -> usize {
    let mut n = 0;
    for t in tracks.iter_mut().filter(|t| may_be_id3v1(t, was)) {
        t.mtime = 0;
        n += 1;
    }
    n
}

/// An MP3 whose title, artist, album or genre holds a letter an ID3v1 tag
/// read in the code page `was` may give: past ASCII; read as Latin-1, of
/// U+0080–U+00FF.
fn may_be_id3v1(t: &Track, was: u32) -> bool {
    let mp3 = t
        .path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    let from_byte = |c: char| match was {
        super::id3v1::LATIN1 => ('\u{80}'..='\u{ff}').contains(&c),
        _ => !c.is_ascii(),
    };
    mp3 && [&t.title, &t.artist, &t.album, &t.genre]
        .iter()
        .any(|s| s.chars().any(from_byte))
}

/// Writes the index (to a temporary file, then renamed over the old one).
/// The index file's writes.
static WRITES: crate::kept::Kept = crate::kept::Kept::new();

/// The number of a scan's saves, taken when the scan starts: a scan
/// replaced while it saved does not write over the one after it.
pub fn ask() -> u64 {
    WRITES.ask()
}

/// Saves `tracks` as the scan numbered `number` (from [`ask`]) has them.
/// A path JSON cannot hold (a name not valid Unicode, which Windows
/// allows) is left out, read again by each scan, rather than fail the
/// whole index. `code_page`: the one their ID3v1 tags were read in.
pub fn save(tracks: &[Track], playlists: &[PathBuf], code_page: u32, number: u64) {
    let Some(path) = path() else { return };
    #[derive(Serialize)]
    struct IndexRef<'a> {
        version: u32,
        tracks: Vec<&'a Track>,
        playlists: Vec<&'a PathBuf>,
        code_page: u32,
    }
    let kept: Vec<&Track> = tracks
        .iter()
        .filter(|t| t.path.to_str().is_some())
        .collect();
    if kept.len() < tracks.len() {
        log::warn!(
            "{} files with names not in Unicode left out of the index",
            tracks.len() - kept.len()
        );
    }
    let index = IndexRef {
        version: VERSION,
        tracks: kept,
        playlists: playlists.iter().filter(|p| p.to_str().is_some()).collect(),
        code_page,
    };
    let result = serde_json::to_vec(&index)
        .map_err(std::io::Error::other)
        .and_then(|bytes| WRITES.write(number, &path, &bytes));
    if let Err(e) = result {
        log::warn!("cannot write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_other_versions() {
        let tracks = vec![Track {
            path: "D:\\Music\\Ария\\01 - Патриот.mp3".into(),
            title: "Патриот".into(),
            year: 2003,
            duration_ms: 300_000,
            ..Track::default()
        }];
        let json = serde_json::to_vec(&Index {
            version: VERSION,
            tracks: tracks.clone(),
            playlists: vec!["D:\\Music\\best.m3u8".into()],
            code_page: Some(1251),
        })
        .unwrap();
        let back: Index = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.tracks, tracks);
        assert_eq!(back.playlists, [PathBuf::from("D:\\Music\\best.m3u8")]);
        assert_eq!(back.code_page, Some(1251));
        // A track written by a later version with a field this one lacks.
        let later = r#"{"version":1,"tracks":[{"path":"a.mp3","title":"A","rating":5}]}"#;
        let index: Index = serde_json::from_str(later).unwrap();
        assert_eq!(index.tracks[0].title, "A");
        // Before the playlists were kept: none.
        assert!(index.playlists.is_empty());
        assert_eq!(index.code_page, None);
        assert_eq!(index.tracks[0].year, 0);
    }

    /// Of an index whose ID3v1 tags were read in another code page, only
    /// the MP3s with letters such a tag may give are read again: read as
    /// Latin-1, those of U+0080–U+00FF; in a code page, any past ASCII.
    #[test]
    fn mp3s_read_in_another_code_page_are_read_again() {
        let track = |path: &str, title: &str| Track {
            path: path.into(),
            title: title.into(),
            mtime: 7,
            ..Track::default()
        };
        let tracks = || {
            vec![
                track("a.mp3", "Àðèÿ"),
                track("b.MP3", "Motörhead"),
                track("c.flac", "Àðèÿ"),
                track("d.mp3", "Ария"),
                track("e.mp3", "Plain"),
            ]
        };
        let times = |was: u32| {
            let mut tracks = tracks();
            let n = read_again_in(&mut tracks, was);
            let times: Vec<u64> = tracks.iter().map(|t| t.mtime).collect();
            (n, times)
        };
        assert_eq!(times(super::super::id3v1::LATIN1), (2, vec![0, 0, 7, 7, 7]));
        assert_eq!(times(1251), (3, vec![0, 0, 7, 0, 7]));
    }
}