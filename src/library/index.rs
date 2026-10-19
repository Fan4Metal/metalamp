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
const VERSION: u32 = 2;
/// Older versions whose tracks are shown until they are read again: 1
/// took an MP3's later ID3v2 tags over its first (`tags::open`).
const READ_AGAIN: [u32; 1] = [1];
const FILE: &str = "library.json";

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    tracks: Vec<Track>,
    #[serde(default)]
    playlists: Vec<PathBuf>,
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
    match serde_json::from_slice::<Index>(&bytes) {
        Ok(index) if index.version == VERSION || READ_AGAIN.contains(&index.version) => Loaded {
            tracks: dated(index.tracks),
            stale: index.version != VERSION,
            playlists: index.playlists,
        },
        Ok(_) => none(),
        Err(e) => {
            log::warn!("cannot read {}: {e}", path.display());
            none()
        }
    }
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
/// whole index.
pub fn save(tracks: &[Track], playlists: &[PathBuf], number: u64) {
    let Some(path) = path() else { return };
    #[derive(Serialize)]
    struct IndexRef<'a> {
        version: u32,
        tracks: Vec<&'a Track>,
        playlists: Vec<&'a PathBuf>,
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
        })
        .unwrap();
        let back: Index = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.tracks, tracks);
        assert_eq!(back.playlists, [PathBuf::from("D:\\Music\\best.m3u8")]);
        // A track written by a later version with a field this one lacks.
        let later = r#"{"version":1,"tracks":[{"path":"a.mp3","title":"A","rating":5}]}"#;
        let index: Index = serde_json::from_str(later).unwrap();
        assert_eq!(index.tracks[0].title, "A");
        // Before the playlists were kept: none.
        assert!(index.playlists.is_empty());
        assert_eq!(index.tracks[0].year, 0);
    }
}
