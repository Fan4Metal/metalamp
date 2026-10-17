//! The audio file types the program plays: those Media Foundation decodes
//! on Windows 10 (MP3, AAC in MP4 or ADTS, FLAC from 1607, WMA, WAV). The
//! library scan takes these extensions; the file associations
//! (`--register`) will name the same.

use std::path::Path;

/// Lower-case extensions without the dot.
pub const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "m4a", "aac", "flac", "wma", "wav"];

/// Whether `path` has one of [`AUDIO_EXTENSIONS`], in any case (`.MP3`).
pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_by_extension_in_any_case() {
        assert!(is_audio(Path::new("a/01 - x.mp3")));
        assert!(is_audio(Path::new("a/01 - x.MP3")));
        assert!(is_audio(Path::new("b.flac")));
        assert!(!is_audio(Path::new("folder.jpg")));
        assert!(!is_audio(Path::new("Thumbs.db")));
        assert!(!is_audio(Path::new("list.m3u8")));
        assert!(!is_audio(Path::new("mp3")));
    }
}
