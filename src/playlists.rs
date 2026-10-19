//! Saved playlists: M3U8 files in `playlists\` next to the settings, one
//! per name (the playlist's menu saves the list under a name, opens or
//! deletes one). Written as `playlist.m3u8` is (`Queue::m3u8`), read with
//! `queue::m3u_paths`, so they open in other players too.

use std::path::PathBuf;

const FOLDER: &str = "playlists";
const EXTENSION: &str = "m3u8";
/// The longest name, in letters.
pub const MAX_NAME: usize = 60;

fn folder() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join(FOLDER))
}

/// A name as a file name: the letters Windows refuses in one become "_",
/// dots and spaces at the ends dropped.
fn file_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .take(MAX_NAME)
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ');
    if cleaned.is_empty() {
        "_".to_owned()
    } else {
        format!("{cleaned}.{EXTENSION}")
    }
}

/// The names of the saved playlists, in natural order.
pub fn list() -> Vec<String> {
    let Some(dir) = folder() else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
        })
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort_by(|a, b| crate::format::natural_cmp(a, b));
    names
}

/// Saves `text` (an M3U8 list) under `name`, over one of that name.
pub fn save(name: &str, text: &str) -> std::io::Result<()> {
    let dir = folder().ok_or_else(|| std::io::Error::other("no folder for the settings"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file_name(name));
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, text.as_bytes())?;
    std::fs::rename(&tmp, &path)
}

/// The tracks of the playlist `name`.
pub fn load(name: &str) -> Option<Vec<PathBuf>> {
    let dir = folder()?;
    let path = dir.join(file_name(name));
    let bytes = std::fs::read(&path).ok()?;
    let text = crate::queue::m3u_text(&bytes);
    Some(crate::queue::m3u_paths(&text, &dir))
}

pub fn delete(name: &str) {
    if let Some(dir) = folder() {
        let _ = std::fs::remove_file(dir.join(file_name(name)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_file_names() {
        assert_eq!(file_name("Rock"), "Rock.m3u8");
        assert_eq!(file_name(" AC/DC: best? "), "AC_DC_ best_.m3u8");
        assert_eq!(file_name("..."), "_");
        assert_eq!(file_name("Ночь."), "Ночь.m3u8");
    }
}
