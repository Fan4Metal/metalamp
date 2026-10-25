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

/// The settings as the app last kept them for the end of the session,
/// written by the window's subclass itself (`save_kept`).
static AT_END: std::sync::Mutex<Option<Settings>> = std::sync::Mutex::new(None);

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
    /// The window's place with the library, and in the player and
    /// playlist layout (`playlist_window`): each layout keeps its own.
    pub window: Option<WindowPlace>,
    pub playlist_window: Option<WindowPlace>,
    /// The library, the player bar and the playlist beside it; or the
    /// player bar over the playlist alone, as the early Winamp.
    pub window_layout: WindowLayout,
    /// The visualization at all (the settings' switch): off, its display
    /// is dark and takes no click; on, `visualization` is the one chosen,
    /// "none" among them.
    pub visual_enabled: bool,
    /// A click on the visualization goes on to the next one (the right
    /// button lists them either way).
    pub visual_click: bool,
    /// The visualization's colours, apart from the theme's.
    pub visual_colours: crate::palette::VisualColours,
    /// The playlist's line of a track (`title_format`), and whether its
    /// rows show their numbers and lengths.
    pub playlist_format: String,
    pub playlist_numbers: bool,
    pub playlist_lengths: bool,
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
    /// Each table's columns shown and their order, by the table's name;
    /// none: all of them in their own order.
    pub arrangements: HashMap<String, crate::ui::table::Arrangement>,
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
    /// Playlists of the library's folders taken out of that section (the
    /// files stay).
    pub hidden_playlists: Vec<PathBuf>,
    /// The playlist's current entry (`playlist.m3u8` holds the list).
    pub playlist_current: Option<usize>,
    /// Where in it playing was left (ms; 0: at its start), taken up by
    /// the first Play after a start.
    pub playlist_position: u64,
    /// The player bar's time counts down to the track's end.
    pub time_remaining: bool,
    pub bar_layout: BarLayout,
    /// What the bar's display shows with the library, and in the player
    /// and playlist layout (each its own: `visual`).
    pub visualization: Visual,
    pub playlist_visualization: Visual,
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
    /// How large a cover put into tags may be (the file info window).
    pub tag_cover_side: CoverSide,
    /// The keys as changed on the settings' Keys page (`keymap.rs`; only
    /// the changes are written).
    pub keys: crate::keymap::Keymap,
    /// An icon in the notification area; minimized, the window leaves the
    /// taskbar for it.
    pub tray: bool,
    /// The window over the others (Ctrl+T), and the mini player: the
    /// cover and the least of the controls, always over the others
    /// (Ctrl+M; `window` keeps the whole window's place meanwhile).
    pub on_top: bool,
    pub mini: bool,
    /// The mini player's window where it was left (pixels, screen
    /// coordinates); none: the work area's bottom right corner.
    pub mini_rect: Option<[i32; 4]>,
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
            visual: self.visual_colours,
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

/// How large a cover put into tags may be: a larger picture is made that
/// large, as a JPEG (`covers::for_tag`), so an album's tracks do not each
/// carry megabytes of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverSide {
    /// As it is.
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "500")]
    Px500,
    #[default]
    #[serde(rename = "1000")]
    Px1000,
    #[serde(rename = "1500")]
    Px1500,
}

impl CoverSide {
    pub const ALL: [CoverSide; 4] = [
        CoverSide::Original,
        CoverSide::Px500,
        CoverSide::Px1000,
        CoverSide::Px1500,
    ];

    /// The largest side in pixels; none: any.
    pub fn pixels(self) -> Option<u32> {
        match self {
            CoverSide::Original => None,
            CoverSide::Px500 => Some(500),
            CoverSide::Px1000 => Some(1000),
            CoverSide::Px1500 => Some(1500),
        }
    }

    /// As the cover's menu offers it: "As it is", or the pixels.
    pub fn label(self) -> String {
        match self.pixels() {
            None => tr!("As it is", "Как есть").to_owned(),
            Some(px) => px.to_string(),
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

/// What the window holds: the library, or the player and the playlist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowLayout {
    /// The player bar over the playlist, without the library and the
    /// status line (the early Winamp's).
    Playlist,
    /// The library with the playlist beside it. An unknown value reads as
    /// this one, so it is last.
    #[default]
    #[serde(other)]
    Library,
}

impl WindowLayout {
    pub const ALL: [WindowLayout; 2] = [WindowLayout::Library, WindowLayout::Playlist];

    pub fn label(self) -> &'static str {
        match self {
            WindowLayout::Library => tr!("With the library", "С библиотекой"),
            WindowLayout::Playlist => tr!("Player and playlist", "Плеер и плейлист"),
        }
    }
}

/// The player and playlist layout's least size and its size at first,
/// in points.
pub const PLAYLIST_MIN_SIZE: [f32; 2] = [440.0, 400.0];
pub const PLAYLIST_SIZE: [f32; 2] = [440.0, 780.0];

/// The player bar's layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarLayout {
    /// The compact bar of one row.
    Classic,
    /// The player and playlist layout's own (not chosen: `App::bar_layout`):
    /// narrow, the time over the visualization in a display on the left,
    /// as the early Winamp's.
    Winamp,
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
            BarLayout::Winamp => tr!("Narrow", "Узкая"),
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
    /// The spectrum's bars in lit segments, as a hi-fi's LEDs.
    Led,
    /// The spectrum as a smooth line, filled under it.
    Curve,
    /// The spectrum's bars up and down from the middle line.
    Mirror,
    /// The spectrum's bars around the cover, the bass at the top.
    Radial,
    /// The spectra of the last moments as lines one behind the other.
    Ridges,
    /// The spectrum's bands as colours rolling from the right.
    Spectrogram,
    /// The sound's peaks rolling from the right.
    Waveform,
    /// The wave itself.
    Scope,
    /// The twelve notes' loudness, the octaves added up.
    Chroma,
    /// The notes on a piano's keys, and a bar over each.
    Piano,
    /// An empty display, and no frames drawn for it.
    Off,
}

impl Visual {
    /// As the display's menu and the settings list them, "none" last.
    pub const ALL: [Visual; 12] = [
        Visual::Spectrum,
        Visual::Led,
        Visual::Curve,
        Visual::Mirror,
        Visual::Radial,
        Visual::Ridges,
        Visual::Spectrogram,
        Visual::Waveform,
        Visual::Scope,
        Visual::Chroma,
        Visual::Piano,
        Visual::Off,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Visual::Spectrum => tr!("Spectrum", "Спектр"),
            Visual::Led => tr!("LED spectrum", "Светодиодный спектр"),
            Visual::Curve => tr!("Spectrum curve", "Спектр-кривая"),
            Visual::Mirror => tr!("Mirrored spectrum", "Зеркальный спектр"),
            Visual::Radial => tr!("Radial spectrum", "Круговой спектр"),
            Visual::Ridges => tr!("Ridges", "Хребты"),
            Visual::Spectrogram => tr!("Spectrogram", "Спектрограмма"),
            Visual::Waveform => tr!("Waveform", "Волна"),
            Visual::Scope => tr!("Oscilloscope", "Осциллограф"),
            Visual::Chroma => tr!("Notes", "Ноты"),
            Visual::Piano => tr!("Piano", "Клавиатура"),
            Visual::Off => tr!("No visualization", "Без визуализации"),
        }
    }

    /// The first of a group in the menus (a line over it): the spectrum's
    /// histories, the wave, the notes, none.
    pub fn starts_group(self) -> bool {
        matches!(
            self,
            Visual::Ridges | Visual::Waveform | Visual::Chroma | Visual::Off
        )
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
            playlist_window: None,
            window_layout: WindowLayout::default(),
            visual_enabled: true,
            visual_click: true,
            visual_colours: Default::default(),
            playlist_format: crate::title_format::DEFAULT.to_owned(),
            playlist_numbers: true,
            playlist_lengths: true,
            library_folders: Vec::new(),
            views: Vec::new(),
            view: String::new(),
            album_view: AlbumView::Grid,
            cover_size: 152.0,
            artists_width: 260.0,
            tracks_height: 300.0,
            columns: HashMap::new(),
            sorts: HashMap::new(),
            arrangements: HashMap::new(),
            volume: 80,
            mute: false,
            repeat: Repeat::Off,
            shuffle: false,
            playlist_visible: true,
            playlist_width: 340.0,
            playlist_cover: true,
            hidden_playlists: Vec::new(),
            playlist_current: None,
            playlist_position: 0,
            time_remaining: false,
            bar_layout: BarLayout::default(),
            visualization: Visual::default(),
            playlist_visualization: Visual::default(),
            output_device: String::new(),
            output_device_name: String::new(),
            dsp: Default::default(),
            eq_presets: Vec::new(),
            crossfade: 0,
            crossfade_in_albums: false,
            media_keys: MediaKeys::default(),
            tag_cover_side: CoverSide::default(),
            keys: crate::keymap::Keymap::default(),
            tray: false,
            on_top: false,
            mini: false,
            mini_rect: None,
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
    /// The place of the window in the layout chosen.
    pub fn place(&self) -> Option<WindowPlace> {
        match self.window_layout {
            WindowLayout::Library => self.window,
            WindowLayout::Playlist => self.playlist_window,
        }
    }

    pub fn place_mut(&mut self) -> &mut Option<WindowPlace> {
        match self.window_layout {
            WindowLayout::Library => &mut self.window,
            WindowLayout::Playlist => &mut self.playlist_window,
        }
    }

    /// The window's least size in the layout chosen, in points.
    pub fn min_size(&self) -> [f32; 2] {
        match self.window_layout {
            WindowLayout::Library => crate::MIN_SIZE,
            WindowLayout::Playlist => PLAYLIST_MIN_SIZE,
        }
    }

    /// The window's size at first in the layout chosen, in points.
    pub fn default_size(&self) -> [f32; 2] {
        match self.window_layout {
            WindowLayout::Library => crate::DEFAULT_SIZE,
            WindowLayout::Playlist => PLAYLIST_SIZE,
        }
    }

    /// The visualization chosen for the window layout in use.
    pub fn visual(&self) -> Visual {
        match self.window_layout {
            WindowLayout::Library => self.visualization,
            WindowLayout::Playlist => self.playlist_visualization,
        }
    }

    pub fn visual_mut(&mut self) -> &mut Visual {
        match self.window_layout {
            WindowLayout::Library => &mut self.visualization,
            WindowLayout::Playlist => &mut self.playlist_visualization,
        }
    }

    /// The visualization shown: the layout's while the switch is on, else
    /// none.
    pub fn visual_shown(&self) -> Visual {
        if self.visual_enabled {
            self.visual()
        } else {
            Visual::Off
        }
    }

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

    /// Kept as they are, for `save_kept` at the end of the session.
    pub fn keep_for_the_end(&self) {
        if let Ok(mut kept) = AT_END.lock()
            && kept.as_ref() != Some(self)
        {
            *kept = Some(self.clone());
        }
    }

    /// Saves a copy on a background thread. Saves are numbered when asked
    /// for, so one asked for later, here or in [`Settings::save`], wins even
    /// if this one finishes after it.
    pub fn save_in_background(&self) {
        let number = FILE.ask();
        let settings = self.clone();
        std::thread::spawn(move || settings.save_numbered(number));
    }

    /// Writes the settings last kept (`keep_for_the_end`), at once: the
    /// session ends (`WM_ENDSESSION`), and Windows may end the process as
    /// soon as the message is answered, before the app's next frame.
    pub fn save_kept() {
        let kept = AT_END.lock().ok().and_then(|k| k.clone());
        if let Some(settings) = kept {
            settings.save();
        }
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
    #[test]
    fn the_visualization_switch_keeps_the_one_chosen() {
        let mut s = Settings {
            visualization: Visual::Scope,
            visual_enabled: false,
            ..Settings::default()
        };
        assert_eq!(s.visual_shown(), Visual::Off);
        s.visual_enabled = true;
        assert_eq!(s.visual_shown(), Visual::Scope);
        // Each window layout its own.
        s.window_layout = WindowLayout::Playlist;
        assert_eq!(s.visual_shown(), Visual::default());
        *s.visual_mut() = Visual::Waveform;
        assert_eq!(s.visualization, Visual::Scope);
        s.window_layout = WindowLayout::Library;
        assert_eq!(s.visual_shown(), Visual::Scope);
        // A click goes through "none" too.
        assert_eq!(Visual::Piano.next(), Visual::Off);
        assert_eq!(Visual::Off.next(), Visual::Spectrum);
        assert_eq!(Visual::Spectrum.next(), Visual::Led);
        // Kept by their names.
        for (v, name) in [
            (Visual::Led, "led"),
            (Visual::Ridges, "ridges"),
            (Visual::Chroma, "chroma"),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn each_layout_keeps_its_place() {
        let mut s = Settings::default();
        let place = WindowPlace {
            normal: [10, 20, 650, 800],
            maximized: false,
        };
        s.window_layout = WindowLayout::Playlist;
        *s.place_mut() = Some(place);
        assert_eq!(s.playlist_window, Some(place));
        assert_eq!(s.window, None);
        assert_eq!(s.min_size(), PLAYLIST_MIN_SIZE);
        s.window_layout = WindowLayout::Library;
        assert_eq!(s.place(), None);
        // An unknown layout reads as the library's.
        let s = Settings::from_json(br#"{"window_layout":"skin"}"#).unwrap();
        assert_eq!(s.window_layout, WindowLayout::Library);
    }

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
