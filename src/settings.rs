//! User settings, stored as JSON (`%APPDATA%\MetalAMP\config\settings.json`,
//! or [`PORTABLE_FILE`] next to the exe for a portable copy). Fields missing
//! from the file (written by an earlier version) take their defaults, and
//! unknown ones (a later version's) are ignored. The library index, the
//! playlist and the cover cache live in the same folder ([`data_dir`]), in
//! files of their own.

use crate::i18n::LangChoice;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// The number of the last save asked for, and of the last one written.
static SAVES: AtomicU64 = AtomicU64::new(0);
static WRITTEN: Mutex<u64> = Mutex::new(0);

/// Where the window was: its restored rectangle in pixels as Windows keeps
/// it (`GetWindowPlacement().rcNormalPosition`: left, top, right, bottom in
/// work-area coordinates, also while maximized), and whether it was
/// maximized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlace {
    pub normal: [i32; 4],
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Interface language, chosen in About.
    #[serde(with = "lang_name")]
    pub language: LangChoice,
    /// Check for a newer release at start-up, at most once a day; off
    /// until turned on in About.
    pub check_updates: bool,
    /// Unix seconds of the last update check that got an answer.
    pub last_update_check: u64,
    pub window: Option<WindowPlace>,
    /// The folders of the media library.
    pub library_folders: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: LangChoice::System,
            check_updates: false,
            last_update_check: 0,
            window: None,
            library_folders: Vec::new(),
        }
    }
}

/// The language as its name in the settings (`system`, `en`, `ru`); an
/// unknown name reads as `system`.
mod lang_name {
    use crate::i18n::LangChoice;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(lang: &LangChoice, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(lang.name())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<LangChoice, D::Error> {
        let name = String::deserialize(d).unwrap_or_default();
        Ok(LangChoice::from_name(&name))
    }
}

/// Settings file that makes a copy portable: when it lies next to the exe
/// (even empty) and can be written, the settings are kept in it instead of
/// the user profile. The portable zip ships an empty one.
pub const PORTABLE_FILE: &str = "metalamp.json";

/// Where the settings of this run are kept.
#[derive(Debug)]
pub struct Location {
    /// The settings file, `None` if there is nowhere to keep it.
    pub file: Option<PathBuf>,
    /// `file` is [`PORTABLE_FILE`] next to the exe.
    pub portable: bool,
}

/// Where the settings are kept, decided once per run.
pub fn location() -> &'static Location {
    static LOCATION: OnceLock<Location> = OnceLock::new();
    LOCATION.get_or_init(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf));
        match exe_dir.and_then(|d| portable_file_in(&d)) {
            Some(file) => Location {
                file: Some(file),
                portable: true,
            },
            None => Location {
                file: directories::ProjectDirs::from("", "", crate::APP_ID)
                    .map(|dirs| dirs.config_dir().join("settings.json")),
                portable: false,
            },
        }
    })
}

/// The folder of the settings file, where the library index, the playlist
/// and the covers are kept too.
pub fn data_dir() -> Option<PathBuf> {
    location()
        .file
        .as_ref()
        .and_then(|f| f.parent().map(Path::to_path_buf))
}

/// [`PORTABLE_FILE`] in `dir` if it is there and can be written; a copy on
/// a read-only medium keeps its settings in the profile instead of losing
/// them.
fn portable_file_in(dir: &Path) -> Option<PathBuf> {
    let file = dir.join(PORTABLE_FILE);
    // Opening for appending checks the permission without changing the file.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .ok()
        .map(|_| file)
}

impl Settings {
    /// Reads the settings file; missing, empty or malformed files give
    /// defaults.
    pub fn load() -> Settings {
        location()
            .file
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        self.save_numbered(SAVES.fetch_add(1, Ordering::SeqCst) + 1);
    }

    /// Saves a copy on a background thread. Saves are numbered when asked
    /// for, so one asked for later, here or in [`Settings::save`], wins even
    /// if this one finishes after it.
    pub fn save_in_background(&self) {
        let number = SAVES.fetch_add(1, Ordering::SeqCst) + 1;
        let settings = self.clone();
        std::thread::spawn(move || settings.save_numbered(number));
    }

    /// Writes the file unless a later save was written already. One writer
    /// at a time: they share the temporary file.
    fn save_numbered(&self, number: u64) {
        let Some(path) = &location().file else {
            return;
        };
        let mut written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
        if *written > number {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_ok() {
                *written = number;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_file_only_when_present_and_writable() {
        let dir = std::env::temp_dir().join(format!("metalamp_portable_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(PORTABLE_FILE);
        assert_eq!(portable_file_in(&dir), None);
        std::fs::write(&file, "").unwrap();
        assert_eq!(portable_file_in(&dir), Some(file.clone()));
        let mut perm = std::fs::metadata(&file).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&file, perm.clone()).unwrap();
        assert_eq!(portable_file_in(&dir), None);
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&file, perm).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn round_trip_and_unknown_fields() {
        let settings = Settings {
            language: LangChoice::Ru,
            check_updates: true,
            last_update_check: 42,
            window: Some(WindowPlace {
                normal: [10, 20, 1290, 820],
                maximized: true,
            }),
            library_folders: vec![PathBuf::from(r"D:\Music")],
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        let partial: Settings = serde_json::from_str(r#"{"language":"en","future":1}"#).unwrap();
        assert_eq!(partial.language, LangChoice::En);
        assert!(!partial.check_updates);
        assert_eq!(partial.window, None);
    }

    #[test]
    fn unknown_language_reads_as_system() {
        let s: Settings = serde_json::from_str(r#"{"language":"de"}"#).unwrap();
        assert_eq!(s.language, LangChoice::System);
        let s: Settings = serde_json::from_str(r#"{"language":5}"#).unwrap_or_default();
        assert_eq!(s.language, LangChoice::System);
    }
}
