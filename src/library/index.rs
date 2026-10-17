//! The library index between runs: `library.json` next to the settings,
//! the tracks with their tags, sizes and modification times. A start shows
//! the library from it at once; the rescan after it reads only the files
//! whose size or time changed. Compact JSON (not pretty): about 300 bytes a
//! track.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::Track;

/// Raised when the meaning of a field changes: an index of another version
/// is dropped and the library read again.
const VERSION: u32 = 1;
const FILE: &str = "library.json";

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    tracks: Vec<Track>,
}

fn path() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join(FILE))
}

/// The tracks of the last scan; empty without an index or with one of
/// another version.
pub fn load() -> Vec<Track> {
    let Some(path) = path() else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    match serde_json::from_slice::<Index>(&bytes) {
        Ok(index) if index.version == VERSION => index.tracks,
        Ok(_) => Vec::new(),
        Err(e) => {
            log::warn!("cannot read {}: {e}", path.display());
            Vec::new()
        }
    }
}

/// Writes the index (to a temporary file, then renamed over the old one).
pub fn save(tracks: &[Track]) {
    let Some(path) = path() else { return };
    #[derive(Serialize)]
    struct IndexRef<'a> {
        version: u32,
        tracks: &'a [Track],
    }
    let index = IndexRef {
        version: VERSION,
        tracks,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let result = serde_json::to_vec(&index)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&tmp, bytes))
        .and_then(|()| std::fs::rename(&tmp, &path));
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
        })
        .unwrap();
        let back: Index = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.tracks, tracks);
        // A track written by a later version with a field this one lacks.
        let later = r#"{"version":1,"tracks":[{"path":"a.mp3","title":"A","rating":5}]}"#;
        let index: Index = serde_json::from_str(later).unwrap();
        assert_eq!(index.tracks[0].title, "A");
        assert_eq!(index.tracks[0].year, 0);
    }
}
