//! The audio file types MetalAMP plays: those Media Foundation decodes on
//! Windows 10 (MP3, AAC in MP4 or ADTS, FLAC from 1607, WMA, WAV), and the
//! M3U playlists. The library scan takes the audio extensions; `assoc.rs`
//! registers every type in Windows with its own icon (`type_icon.rs`,
//! metalmpv's design: a white page with a note or a list and the format's
//! name on a band). Dependency-free: `build.rs` includes this file to draw
//! the icons embedded in the executable.

// build.rs uses the table only for the icons.
#![allow(dead_code)]

use std::path::Path;

pub struct FileType {
    /// ProgID suffix: the type is registered as `metalamp.<id>`.
    pub id: &'static str,
    /// Text on the icon.
    pub label: &'static str,
    /// Extensions, lowercase, without the dot.
    pub extensions: &'static [&'static str],
    /// Name in Explorer's Type column.
    pub name_en: &'static str,
    pub name_ru: &'static str,
    pub colors: Colors,
    pub kind: Kind,
}

/// What a type's files hold: the icon shows a note or a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Video,
    Audio,
    Playlist,
}

/// The icon's band (white letters on it) and the ground of its picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub band: [u8; 3],
    pub light: [u8; 3],
}

// metalmpv's hues (after GOM Player's), the band and the light ground.
const ORANGE: Colors = Colors {
    band: [0xd1, 0x35, 0x2a],
    light: [0xf5, 0x90, 0x81],
};
const CRIMSON: Colors = Colors {
    band: [0x91, 0x27, 0x55],
    light: [0xcb, 0x7b, 0x99],
};
const TEAL: Colors = Colors {
    band: [0x17, 0x86, 0x7e],
    light: [0x69, 0xba, 0xb6],
};
const LIME: Colors = Colors {
    band: [0x73, 0xa0, 0x49],
    light: [0xb7, 0xd6, 0x8a],
};
const BLUE: Colors = Colors {
    band: [0x04, 0x56, 0x9b],
    light: [0x66, 0x9d, 0xc9],
};
const AMBER: Colors = Colors {
    band: [0xc2, 0x7c, 0x0e],
    light: [0xf2, 0xbe, 0x5a],
};

const fn audio(
    id: &'static str,
    label: &'static str,
    extensions: &'static [&'static str],
    name_en: &'static str,
    name_ru: &'static str,
    colors: Colors,
) -> FileType {
    FileType {
        id,
        label,
        extensions,
        name_en,
        name_ru,
        colors,
        kind: Kind::Audio,
    }
}

/// New types go at the end: a type's icon is its place (`icon_id`), which
/// Explorer remembers.
pub const FILE_TYPES: &[FileType] = &[
    audio("mp3", "MP3", &["mp3"], "MP3 Audio", "Аудио MP3", ORANGE),
    audio("flac", "FLAC", &["flac"], "FLAC Audio", "Аудио FLAC", BLUE),
    audio(
        "m4a",
        "M4A",
        &["m4a", "aac"],
        "AAC Audio",
        "Аудио AAC",
        LIME,
    ),
    audio("wav", "WAV", &["wav"], "WAVE Audio", "Аудио WAVE", TEAL),
    audio(
        "wma",
        "WMA",
        &["wma"],
        "Windows Media Audio",
        "Аудио Windows Media",
        CRIMSON,
    ),
    FileType {
        id: "m3u",
        label: "M3U",
        extensions: &["m3u", "m3u8"],
        name_en: "M3U Playlist",
        name_ru: "Плейлист M3U",
        colors: AMBER,
        kind: Kind::Playlist,
    },
];

/// Lower-case audio extensions without the dot: the library's files.
pub const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "flac", "m4a", "aac", "wav", "wma"];

/// Whether `path` has one of [`AUDIO_EXTENSIONS`], in any case (`.MP3`).
pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

/// Whether `path` is an M3U or M3U8 playlist, in any case.
pub fn is_playlist(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("m3u") || e.eq_ignore_ascii_case("m3u8"))
}

/// Resource ID of the icon of `FILE_TYPES[i]` in the executable (the app
/// icon is 1).
pub const fn icon_id(i: usize) -> u16 {
    101 + i as u16
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

    /// The audio types' extensions are the library's, each once.
    #[test]
    fn types_cover_the_audio_extensions() {
        let mut registered: Vec<&str> = FILE_TYPES
            .iter()
            .filter(|t| t.kind == Kind::Audio)
            .flat_map(|t| t.extensions.iter().copied())
            .collect();
        let mut listed = AUDIO_EXTENSIONS.to_vec();
        registered.sort_unstable();
        listed.sort_unstable();
        assert_eq!(registered, listed);
        assert_eq!(icon_id(0), 101);
    }
}
