//! Settings kept between runs in eframe's storage
//! (`%APPDATA%\MetalAMP\data\app.ron`, or [`PORTABLE_FILE`] next to the exe,
//! see [`location`]). Window size and position and egui's own state (panel
//! widths) are saved by eframe itself (`persist_window`).
//!
//! Plain strings through `eframe::Storage`: no serde derives needed, and an
//! unknown or malformed value falls back to its default, so settings of an
//! older or newer version never make the file unreadable.
//!
//! metalmd keeps structured settings as JSON instead (serde + `directories`,
//! `%APPDATA%\MetalMD\config\settings.json`, `#[serde(default)]`, a `.tmp`
//! + rename write): take that variant when the settings are nested.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::i18n::LangChoice;

const THEME: &str = "theme";
const LANGUAGE: &str = "language";
const CHECK_UPDATES: &str = "check_updates";
const LAST_UPDATE_CHECK: &str = "last_update_check";

/// Settings file that makes a copy portable: when it lies next to the exe
/// (even empty) and can be written, the settings are kept in it instead of
/// the user profile. The portable zip ships an empty one
/// (`PORTABLE_SETTINGS` in tools/make_release.py).
pub const PORTABLE_FILE: &str = "app.ron";

/// Where the settings of this run are kept.
#[derive(Debug)]
pub struct Location {
    /// The settings file, `None` if there is nowhere to keep it.
    pub file: Option<PathBuf>,
    /// `file` is [`PORTABLE_FILE`] next to the exe.
    pub portable: bool,
}

impl Location {
    /// Whether settings of an earlier run are there (a portable file starts
    /// empty). `NativeOptions::centered` is set only when there are none:
    /// eframe would otherwise re-centre a restored window.
    pub fn has_saved(&self) -> bool {
        self.file
            .as_ref()
            .is_some_and(|f| f.metadata().is_ok_and(|m| m.len() > 0))
    }
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
                file: eframe::storage_dir(crate::APP_ID).map(|d| d.join("app.ron")),
                portable: false,
            },
        }
    })
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

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Light, dark or as in Windows.
    pub theme: egui::ThemePreference,
    /// Interface language, or the one of Windows.
    pub language: LangChoice,
    /// Ask GitHub for a newer release at start-up (off unless enabled).
    pub check_updates: bool,
    /// Unix seconds of the last update check that got an answer.
    pub last_update_check: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: egui::ThemePreference::System,
            language: LangChoice::System,
            check_updates: false,
            last_update_check: 0,
        }
    }
}

impl Settings {
    /// Read settings; missing or unknown values keep their defaults.
    pub fn load(storage: &dyn eframe::Storage) -> Self {
        let d = Self::default();
        let get = |key| storage.get_string(key);
        Self {
            theme: match get(THEME).as_deref() {
                Some("system") => egui::ThemePreference::System,
                Some("light") => egui::ThemePreference::Light,
                Some("dark") => egui::ThemePreference::Dark,
                _ => d.theme,
            },
            language: get(LANGUAGE)
                .as_deref()
                .map_or(d.language, LangChoice::from_name),
            check_updates: match get(CHECK_UPDATES).as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => d.check_updates,
            },
            last_update_check: get(LAST_UPDATE_CHECK)
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.last_update_check),
        }
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let theme = match self.theme {
            egui::ThemePreference::System => "system",
            egui::ThemePreference::Light => "light",
            egui::ThemePreference::Dark => "dark",
        };
        storage.set_string(THEME, theme.into());
        storage.set_string(LANGUAGE, self.language.name().into());
        storage.set_string(CHECK_UPDATES, self.check_updates.to_string());
        storage.set_string(LAST_UPDATE_CHECK, self.last_update_check.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::Storage;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemStorage(HashMap<String, String>);

    impl Storage for MemStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn round_trip() {
        let s = Settings {
            theme: egui::ThemePreference::Dark,
            language: LangChoice::Ru,
            check_updates: true,
            last_update_check: 1_790_000_000,
        };
        let mut storage = MemStorage::default();
        s.save(&mut storage);
        assert_eq!(Settings::load(&storage), s);
    }

    #[test]
    fn missing_or_unknown_values_use_defaults() {
        assert_eq!(Settings::load(&MemStorage::default()), Settings::default());
        let mut storage = MemStorage::default();
        storage.set_string(THEME, "bogus".into());
        storage.set_string(LANGUAGE, "de".into());
        storage.set_string(CHECK_UPDATES, "yes".into());
        storage.set_string(LAST_UPDATE_CHECK, "soon".into());
        assert_eq!(Settings::load(&storage), Settings::default());
    }

    #[test]
    fn portable_file_only_when_present_and_writable() {
        let dir = std::env::temp_dir().join(format!("portable_test_{}", std::process::id()));
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
}
