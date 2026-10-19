//! User settings, stored as JSON (`%APPDATA%\MetalAMP\config\settings.json`,
//! or [`PORTABLE_FILE`] next to the exe for a portable copy). Fields missing
//! from the file (written by an earlier version) take their defaults,
//! unknown ones (a later version's) are ignored, and so is a field whose
//! value cannot be read (a name a later version added, a typo): it takes
//! its default and the rest are kept ([`Settings::from_json`]). The
//! library index, the playlist and the cover cache live in the same
//! folder ([`data_dir`]), in files of their own.

use crate::i18n::LangChoice;
use crate::queue::Repeat;
use crate::ui::table::Sort;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The settings file's writes.
static FILE: crate::kept::Kept = crate::kept::Kept::new();

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
    /// Interface language, chosen in the settings window.
    #[serde(with = "lang_name")]
    pub language: LangChoice,
    /// Check for a newer release at start-up, at most once a day; off
    /// until turned on in the settings window.
    pub check_updates: bool,
    /// Unix seconds of the last update check that got an answer.
    pub last_update_check: u64,
    pub window: Option<WindowPlace>,
    /// The folders of the media library.
    pub library_folders: Vec<PathBuf>,
    /// The views of the library (filters over its tracks, `library::filter`),
    /// and the name of the one in use (empty: the whole library).
    pub views: Vec<crate::library::filter::Filter>,
    pub view: String,
    /// Albums as a grid of covers or as a table.
    pub album_view: AlbumView,
    /// The side of a cover in the grid, in points (`library::MIN_COVER`
    /// to `MAX_COVER`).
    pub cover_size: f32,
    /// The artists' column's width and the tracks' table's height, in
    /// points.
    pub artists_width: f32,
    pub tracks_height: f32,
    /// Each table's column widths, by the table's name.
    pub columns: HashMap<String, Vec<f32>>,
    /// Each table's sort, by the table's name; none: the library's order.
    pub sorts: HashMap<String, Sort>,
    /// 0 to 100.
    pub volume: u8,
    pub mute: bool,
    pub repeat: Repeat,
    pub shuffle: bool,
    /// The playlist on the right, and its width in points.
    pub playlist_visible: bool,
    pub playlist_width: f32,
    /// The cover of the track playing under the playlist.
    pub playlist_cover: bool,
    /// The playlist's current entry (`playlist.m3u8` holds the list).
    pub playlist_current: Option<usize>,
    /// Where in it playing was left (ms; 0: at its start), taken up by
    /// the first Play after a start.
    pub playlist_position: u64,
    /// The player bar's time counts down to the track's end.
    pub time_remaining: bool,
    pub bar_layout: BarLayout,
    /// What the bar's display shows.
    pub visualization: Visual,
    /// The output device's endpoint id (empty: Windows' default) and its
    /// name, shown while it is not present.
    pub output_device: String,
    pub output_device_name: String,
    /// The sound processing (`audio::dsp`).
    pub dsp: crate::audio::dsp::DspSettings,
    /// The user's equalizer presets (the classic ones are built in).
    pub eq_presets: Vec<EqPreset>,
    /// The crossfade between tracks, in seconds (0: none), and whether
    /// between the tracks of one folder too (else albums play gapless).
    pub crossfade: u8,
    pub crossfade_in_albums: bool,
    /// When the media keys are the player's.
    pub media_keys: MediaKeys,
    /// The look: the base theme and the controls' colour.
    pub theme: crate::palette::Theme,
    pub accent: crate::palette::Accent,
    /// The selected rows' ground and the track playing's text.
    pub selection: crate::palette::Tint,
    pub playing: crate::palette::Tint,
}

impl Settings {
    /// The view in use, if `view` names one of `views`.
    pub fn current_view(&self) -> Option<&crate::library::filter::Filter> {
        if self.view.is_empty() {
            return None;
        }
        self.views.iter().find(|v| v.name == self.view)
    }

    /// What the palette is made of.
    pub fn look(&self) -> crate::palette::Look {
        crate::palette::Look {
            theme: self.theme,
            accent: self.accent,
            selection: self.selection,
            playing: self.playing,
        }
    }
}

/// When the media keys (of keyboards, headsets, remotes) control the
/// player.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKeys {
    /// Also while another window is active, through Windows' media
    /// controls (`smtc.rs`), whose flyout shows the track.
    #[default]
    Always,
    /// Only while the player's window is active.
    Window,
    /// None: they go on to Windows and the other programs.
    Off,
}

impl MediaKeys {
    pub const ALL: [MediaKeys; 3] = [MediaKeys::Always, MediaKeys::Window, MediaKeys::Off];

    pub fn label(self) -> &'static str {
        match self {
            MediaKeys::Always => tr!("Always", "Всегда"),
            MediaKeys::Window => tr!("While the player is active", "Когда активно окно плеера"),
            MediaKeys::Off => tr!("Never", "Никогда"),
        }
    }
}

/// An equalizer setting saved under a name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EqPreset {
    pub name: String,
    pub preamp: f32,
    pub bands: [f32; 10],
}

/// The player bar's layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarLayout {
    /// The compact bar of one row.
    Classic,
    /// The time in large digits, a visualization on the right. An
    /// unknown value (an older name) reads as this one, so it is last.
    #[default]
    #[serde(other)]
    Full,
}

impl BarLayout {
    pub const ALL: [BarLayout; 2] = [BarLayout::Full, BarLayout::Classic];

    pub fn label(self) -> &'static str {
        match self {
            BarLayout::Full => tr!("With a visualization", "С визуализацией"),
            BarLayout::Classic => tr!("Compact", "Компактная"),
        }
    }
}

/// What the display on the right of the player bar shows (`ui::visual`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visual {
    /// Bars over log-spaced bands with falling peaks.
    #[default]
    Spectrum,
    /// The sound's peaks rolling from the right.
    Waveform,
    /// The wave itself.
    Scope,
    /// The spectrum's bands as colours rolling from the right.
    Spectrogram,
    /// An empty display, and no frames drawn for it.
    Off,
}

impl Visual {
    pub const ALL: [Visual; 5] = [
        Visual::Spectrum,
        Visual::Waveform,
        Visual::Scope,
        Visual::Spectrogram,
        Visual::Off,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Visual::Spectrum => tr!("Spectrum", "Спектр"),
            Visual::Waveform => tr!("Waveform", "Волна"),
            Visual::Scope => tr!("Oscilloscope", "Осциллограф"),
            Visual::Spectrogram => tr!("Spectrogram", "Спектрограмма"),
            Visual::Off => tr!("Off", "Выключена"),
        }
    }

    /// The one a click on the display goes on to.
    pub fn next(self) -> Visual {
        let i = Visual::ALL.iter().position(|&v| v == self).unwrap_or(0);
        Visual::ALL[(i + 1) % Visual::ALL.len()]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlbumView {
    #[default]
    Grid,
    Table,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: LangChoice::System,
            check_updates: false,
            last_update_check: 0,
            window: None,
            library_folders: Vec::new(),
            views: Vec::new(),
            view: String::new(),
            album_view: AlbumView::Grid,
            cover_size: 152.0,
            artists_width: 260.0,
            tracks_height: 300.0,
            columns: HashMap::new(),
            sorts: HashMap::new(),
            volume: 80,
            mute: false,
            repeat: Repeat::Off,
            shuffle: false,
            playlist_visible: true,
            playlist_width: 340.0,
            playlist_cover: true,
            playlist_current: None,
            playlist_position: 0,
            time_remaining: false,
            bar_layout: BarLayout::default(),
            visualization: Visual::default(),
            output_device: String::new(),
            output_device_name: String::new(),
            dsp: Default::default(),
            eq_presets: Vec::new(),
            crossfade: 0,
            crossfade_in_albums: false,
            media_keys: MediaKeys::default(),
            theme: Default::default(),
            accent: Default::default(),
            selection: Default::default(),
            playing: Default::default(),
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
    /// Reads the settings file; a missing or empty one gives defaults. A
    /// file that is not JSON at all (cut short) is kept as
    /// `settings.json.bad` before the defaults are saved over it.
    pub fn load() -> Settings {
        let Some(path) = &location().file else {
            return Settings::default();
        };
        let Ok(bytes) = std::fs::read(path) else {
            return Settings::default();
        };
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Settings::default();
        }
        Settings::from_json(&bytes).unwrap_or_else(|| {
            log::warn!("{} cannot be read; kept as .bad", path.display());
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            Settings::default()
        })
    }

    /// The settings in `bytes`, each field read on its own: one that
    /// cannot be read takes its default and costs no other. `None` when
    /// `bytes` is not a JSON object.
    pub fn from_json(bytes: &[u8]) -> Option<Settings> {
        use serde_json::Value;
        let Ok(Value::Object(file)) = serde_json::from_slice::<Value>(bytes) else {
            return None;
        };
        let Ok(Value::Object(mut merged)) = serde_json::to_value(Settings::default()) else {
            return None;
        };
        let reads = |m: &serde_json::Map<String, Value>| {
            serde_json::from_value::<Settings>(Value::Object(m.clone())).is_ok()
        };
        for (key, value) in file {
            // Unknown fields are left out; a known one is kept if the
            // whole still reads with it.
            let Some(default) = merged.get(&key).cloned() else {
                continue;
            };
            merged.insert(key.clone(), value);
            if !reads(&merged) {
                log::warn!("settings: {key} cannot be read, the default taken");
                merged.insert(key, default);
            }
        }
        serde_json::from_value(Value::Object(merged)).ok()
    }

    pub fn save(&self) {
        self.save_numbered(FILE.ask());
    }

    /// Saves a copy on a background thread. Saves are numbered when asked
    /// for, so one asked for later, here or in [`Settings::save`], wins even
    /// if this one finishes after it.
    pub fn save_in_background(&self) {
        let number = FILE.ask();
        let settings = self.clone();
        std::thread::spawn(move || settings.save_numbered(number));
    }

    /// Writes the file unless a later save was written already.
    fn save_numbered(&self, number: u64) {
        let Some(path) = &location().file else {
            return;
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(self)
            && let Err(e) = FILE.write(number, path, &bytes)
        {
            log::warn!("cannot write {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_bar_layout_reads_as_the_full_one() {
        let read = |v: &str| serde_json::from_str::<BarLayout>(&format!("\"{v}\"")).unwrap();
        assert_eq!(read("full"), BarLayout::Full);
        assert_eq!(read("classic"), BarLayout::Classic);
        assert_eq!(read("an older name"), BarLayout::Full);
        assert_eq!(serde_json::to_string(&BarLayout::Full).unwrap(), "\"full\"");
    }

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
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        let partial: Settings = serde_json::from_str(r#"{"language":"en","future":1}"#).unwrap();
        assert_eq!(partial.language, LangChoice::En);
        assert!(!partial.check_updates);
        assert_eq!(partial.window, None);
    }

    #[test]
    fn a_field_that_cannot_be_read_costs_no_other() {
        let json = r#"{"theme":"ultraviolet","visualization":7,"media_keys":"later",
            "repeat":"twice","album_view":null,"dsp":{"bands":[1,2]},
            "library_folders":["D:\\Music"],"volume":35,"future":{"a":1}}"#;
        let s = Settings::from_json(json.as_bytes()).unwrap();
        assert_eq!(s.library_folders, vec![PathBuf::from(r"D:\Music")]);
        assert_eq!(s.volume, 35);
        let d = Settings::default();
        assert_eq!(s.theme, d.theme);
        assert_eq!(s.visualization, d.visualization);
        assert_eq!(s.media_keys, d.media_keys);
        assert_eq!(s.repeat, d.repeat);
        assert_eq!(s.album_view, d.album_view);
        assert_eq!(s.dsp, d.dsp);
        assert_eq!(Settings::from_json(b"{\"volume\": 3"), None);
        assert_eq!(Settings::from_json(b"[1]"), None);
        let all = serde_json::to_vec(&Settings {
            volume: 7,
            ..Settings::default()
        })
        .unwrap();
        assert_eq!(Settings::from_json(&all).unwrap().volume, 7);
    }

    #[test]
    fn unknown_language_reads_as_system() {
        let s: Settings = serde_json::from_str(r#"{"language":"de"}"#).unwrap();
        assert_eq!(s.language, LangChoice::System);
        let s: Settings = serde_json::from_str(r#"{"language":5}"#).unwrap_or_default();
        assert_eq!(s.language, LangChoice::System);
    }
}
