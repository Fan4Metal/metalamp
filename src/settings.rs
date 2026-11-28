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
    /// The window's place with the library, in the player and playlist
    /// layout (`playlist_window`) and as the player alone
    /// (`player_window`): each shape keeps its own.
    pub window: Option<WindowPlace>,
    pub playlist_window: Option<WindowPlace>,
    pub player_window: Option<WindowPlace>,
    /// The library, the player bar and the playlist beside it; or the
    /// player bar over the playlist alone, as the early Winamp.
    pub window_layout: WindowLayout,
    /// In the player and playlist layout, the playlist folded away: the
    /// window is the player bar alone (`Shape::Player`).
    pub player_only: bool,
    /// The visualization at all (the settings' switch): off, its display
    /// is dark and takes no click; on, `visualization` is the one chosen,
    /// "none" among them.
    pub visual_enabled: bool,
    /// A click on the visualization goes on to the next one (the right
    /// button lists them either way).
    pub visual_click: bool,
    /// The visualization's colours, apart from the theme's, with the
    /// library and in the player and playlist layout (each its own:
    /// `visual_colours_mut`, `look`).
    pub visual_colours: crate::palette::VisualColours,
    pub playlist_visual_colours: crate::palette::VisualColours,
    /// The user's own colours for the visualizations, shown where either
    /// layout chooses them (`VisualColours::Scheme`).
    pub visual_scheme: crate::palette::VisualScheme,
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
    /// Which source of album artwork is tried first in the library and player.
    pub cover_priority: CoverPriority,
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
    /// The panel under the playlist (the layout with the library): shown,
    /// and what it shows (`UnderShows`: the lyrics of the track playing
    /// when it has any, else its cover; its cover; its lyrics,
    /// `ui/lyrics.rs`). Read from the older `playlist_cover` and
    /// `playlist_lyrics` (`from_json`).
    pub playlist_under_shown: bool,
    pub playlist_under: UnderShows,
    /// The player and playlist layout: the lyrics in a window of their own
    /// (`ui::lyrics::LyricsWindow`), and its place (points: left, top, and
    /// the inner width and height).
    pub lyrics_window: bool,
    pub lyrics_window_place: Option<[f32; 4]>,
    /// Lyrics a file lacks are looked for on LRCLIB (`lyrics.rs`).
    pub lyrics_online: bool,
    /// The file's own lyrics or LRCLIB's first.
    pub lyrics_priority: LyricsPriority,
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
    /// The user's enhancer presets (beside the built-in ones).
    pub enhancer_presets: Vec<EnhancerPreset>,
    /// The equalizer's sliders and the enhancer's section unfolded in the
    /// sound tuning window.
    pub eq_bands_open: bool,
    pub eq_enhancer_open: bool,
    /// The crossfade between tracks, in seconds (0: none), and whether
    /// between the tracks of one folder too (else albums play gapless).
    pub crossfade: u8,
    pub crossfade_in_albums: bool,
    /// The crossfade into a track chosen while another plays (Next,
    /// Previous, a double click), in seconds (0: none, the sound before
    /// fades out over 30 ms).
    pub crossfade_manual: f32,
    /// A seek crossfades from the place left into the one sought
    /// (`player::SEEK_CROSSFADE_MS`); off: the one fades out, the other in.
    pub crossfade_seek: bool,
    /// How far the seek keys (Shift+Left/Right, the arrows over the
    /// player bar) go, in seconds (1–`MAX_SEEK_STEP`).
    pub seek_step: u8,
    /// How far the volume keys (Ctrl+Up/Down, the arrows over the player
    /// bar) and a wheel notch over it go, in percent
    /// (1–`MAX_VOLUME_STEP`).
    pub volume_step: u8,
    /// When the media keys are the player's.
    pub media_keys: MediaKeys,
    /// How large a cover put into tags may be (the file info window).
    pub tag_cover_side: CoverSide,
    /// The album's preparation's presets (`library::prepare`; none: a
    /// first run's, `first_presets`, made when the window opens) and the
    /// name of the one in use.
    pub prepare_presets: Vec<crate::library::prepare::Preset>,
    pub prepare_preset: String,
    /// The ID3v2 version an MP3's (or a WAV's) tag is written in.
    pub id3v2_version: Id3Version,
    /// What becomes of an MP3's ID3v1 tag when its tags are written.
    pub id3v1: Id3v1Tag,
    /// The code page an MP3's ID3v1 tag is read and written in.
    pub id3v1_encoding: Id3v1Encoding,
    /// What becomes of an MP3's APE tag when its tags are written.
    pub ape: ApeTag,
    /// The keys as changed on the settings' Keys page (`keymap.rs`; only
    /// the changes are written).
    pub keys: crate::keymap::Keymap,
    /// An icon in the notification area; minimized, the window leaves the
    /// taskbar for it.
    pub tray: bool,
    /// The program only in the notification area: the icon there always and
    /// the window never on the taskbar, shown or minimized.
    pub tray_only: bool,
    /// When a track starting is announced in a small window by the
    /// notification area (`notice.rs`), and for how many seconds
    /// (1–`MAX_NOTICE_SECONDS`).
    pub track_notice: TrackNotice,
    pub track_notice_seconds: u8,
    /// The window over the others (Ctrl+T), and the mini player: the
    /// cover and the least of the controls, always over the others
    /// (Ctrl+M; `window` keeps the whole window's place meanwhile).
    pub on_top: bool,
    pub mini: bool,
    /// The mini player's window where it was left (pixels, screen
    /// coordinates); none: the work area's bottom right corner.
    pub mini_rect: Option<[i32; 4]>,
    /// The mini player shows the title and the artist (a click on the
    /// cover switches it), and the cover as it is or as a record's label
    /// (its menu).
    pub mini_names: bool,
    pub mini_style: MiniStyle,
    /// The look: the base theme and the controls' colour.
    pub theme: crate::palette::Theme,
    pub accent: crate::palette::Accent,
    /// The selected rows' ground and the track playing's text.
    pub selection: crate::palette::Tint,
    pub playing: crate::palette::Tint,
}

impl Settings {
    /// The seek keys' step in seconds, within its bounds (a file may hold
    /// 0 or more than the slider offers).
    pub fn seek_seconds(&self) -> i64 {
        i64::from(self.seek_step.clamp(1, MAX_SEEK_STEP))
    }

    /// The volume keys' and the wheel's step in percent, within its
    /// bounds.
    pub fn volume_percent(&self) -> i32 {
        i32::from(self.volume_step.clamp(1, MAX_VOLUME_STEP))
    }

    /// How long a track's notice stays, within its bounds.
    pub fn notice_seconds(&self) -> u32 {
        u32::from(self.track_notice_seconds.clamp(1, MAX_NOTICE_SECONDS))
    }

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
            visual: match self.window_layout {
                WindowLayout::Library => self.visual_colours,
                WindowLayout::Playlist => self.playlist_visual_colours,
            },
            scheme: self.visual_scheme,
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

/// When a track starting is announced (`notice.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackNotice {
    /// Never.
    Off,
    /// While the window is minimized (or in the tray), as Winamp's.
    #[default]
    Minimized,
    /// Also while another window is active (not while the mini player,
    /// which shows the track, is in sight).
    Inactive,
}

impl TrackNotice {
    pub const ALL: [TrackNotice; 3] = [
        TrackNotice::Minimized,
        TrackNotice::Inactive,
        TrackNotice::Off,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TrackNotice::Off => tr!("Never", "Никогда"),
            TrackNotice::Minimized => tr!("While the window is minimized", "Когда окно свёрнуто"),
            TrackNotice::Inactive => tr!("While the window is not active", "Когда окно неактивно"),
        }
    }
}

/// How long a track's notice stays, in seconds.
pub const DEFAULT_NOTICE_SECONDS: u8 = 4;
pub const MAX_NOTICE_SECONDS: u8 = 15;

/// Which cover is preferred when both the tag and the folder have one.
/// The other source is used when the preferred picture is absent or unreadable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverPriority {
    #[default]
    Tags,
    Folder,
}

impl CoverPriority {
    pub const ALL: [Self; 2] = [Self::Tags, Self::Folder];

    pub fn label(self) -> &'static str {
        match self {
            Self::Tags => tr!("Tags first", "Сначала из тегов"),
            Self::Folder => tr!("Folder first", "Сначала из папки"),
        }
    }
}

/// What the panel under the playlist shows: the lyrics of the track
/// playing when it has any, else its cover (the default: the panel never
/// shows "No lyrics found"); its cover; its lyrics. Chosen in the panel's
/// corner and its menu; one button shows or hides the panel (two, the
/// cover's and the microphone's, kept a choice not seen).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnderShows {
    #[default]
    Auto,
    Cover,
    Lyrics,
}

impl UnderShows {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Cover, Self::Lyrics];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => tr!("Lyrics if there are any, else the cover", "Текст, если он есть, иначе обложка"),
            Self::Cover => tr!("The cover", "Обложка"),
            Self::Lyrics => tr!("The lyrics", "Текст"),
        }
    }
}

/// Which lyrics are shown when the file (its tag or an `.lrc` file beside
/// it) and LRCLIB both have them. The other is shown when the preferred
/// has none; a choice made for a track in the panel's menu goes first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsPriority {
    #[default]
    File,
    Lrclib,
}

impl LyricsPriority {
    pub const ALL: [Self; 2] = [Self::File, Self::Lrclib];

    pub fn label(self) -> &'static str {
        match self {
            Self::File => tr!("The file first", "Сначала из файла"),
            Self::Lrclib => tr!("LRCLIB first", "Сначала из LRCLIB"),
        }
    }
}

/// How large a cover put into tags may be: a larger picture is made that
/// large, as a JPEG (`covers::for_tag`), so an album's tracks do not each
/// carry megabytes of it.
/// Kept as `original` or the pixels (`1000`; one's own size likewise).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CoverSide {
    /// As it is.
    Original,
    #[default]
    Px600,
    Px1000,
    Px1500,
    /// One's own size, within `CoverSide::OWN`, none of the ready ones.
    Custom(u32),
}

impl CoverSide {
    pub const ALL: [CoverSide; 4] = [
        CoverSide::Original,
        CoverSide::Px600,
        CoverSide::Px1000,
        CoverSide::Px1500,
    ];

    /// The sizes one's own may be: covers are decoded up to 8192 pixels.
    pub const OWN: std::ops::RangeInclusive<u32> = 100..=8192;

    /// The largest side in pixels; none: any.
    pub fn pixels(self) -> Option<u32> {
        match self {
            CoverSide::Original => None,
            CoverSide::Px600 => Some(600),
            CoverSide::Px1000 => Some(1000),
            CoverSide::Px1500 => Some(1500),
            CoverSide::Custom(px) => Some(px),
        }
    }

    /// That many pixels: a ready size when it is one, else one's own;
    /// none outside `OWN`.
    pub fn of_pixels(px: u32) -> Option<CoverSide> {
        let ready = CoverSide::ALL.into_iter().find(|s| s.pixels() == Some(px));
        ready.or_else(|| CoverSide::OWN.contains(&px).then_some(CoverSide::Custom(px)))
    }

    /// As typed into the cover's menu or kept: `original`, or the pixels.
    pub fn typed(text: &str) -> Option<CoverSide> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("original") {
            return Some(CoverSide::Original);
        }
        CoverSide::of_pixels(text.parse().ok()?)
    }

    pub fn is_custom(self) -> bool {
        matches!(self, CoverSide::Custom(_))
    }

    /// As the cover's menu offers it: "As it is", or the pixels.
    pub fn label(self) -> String {
        match self.pixels() {
            None => tr!("As it is", "Как есть").to_owned(),
            Some(px) => px.to_string(),
        }
    }
}

impl Serialize for CoverSide {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.pixels() {
            None => s.serialize_str("original"),
            Some(px) => s.serialize_str(&px.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for CoverSide {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        CoverSide::typed(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("not a cover side: {text}")))
    }
}

/// The ID3v2 version tags are written in: the file's own, a new tag
/// (or one of 2.2, which is not written) in 2.3, which every program and
/// device reads, or in 2.4; or always one of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Id3Version {
    #[default]
    File23,
    File24,
    #[serde(rename = "2.3")]
    V23,
    #[serde(rename = "2.4")]
    V24,
}

impl Id3Version {
    pub const ALL: [Id3Version; 4] = [
        Id3Version::File23,
        Id3Version::File24,
        Id3Version::V23,
        Id3Version::V24,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Id3Version::File23 => tr!("As in the file, new ones 2.3", "Как в файле, новые — 2.3"),
            Id3Version::File24 => tr!("As in the file, new ones 2.4", "Как в файле, новые — 2.4"),
            Id3Version::V23 => tr!("Always 2.3", "Всегда 2.3"),
            Id3Version::V24 => tr!("Always 2.4", "Всегда 2.4"),
        }
    }

    /// Whether a tag of `original` (2, 3 or 4; none: no tag) is written
    /// as 2.3 (else 2.4).
    pub fn writes_23(self, original: Option<u8>) -> bool {
        match self {
            Id3Version::File23 => original != Some(4),
            Id3Version::File24 => original == Some(3),
            Id3Version::V23 => true,
            Id3Version::V24 => false,
        }
    }
}

/// What becomes of an MP3's ID3v1 tag (the 128 bytes at its end that old
/// players and car stereos read) when its tags are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Id3v1Tag {
    /// Written with the same fields when the file has one.
    #[default]
    Update,
    Remove,
    /// Left as it was.
    Keep,
    /// Written, also into a file without one.
    Always,
}

impl Id3v1Tag {
    pub const ALL: [Id3v1Tag; 4] = [
        Id3v1Tag::Update,
        Id3v1Tag::Remove,
        Id3v1Tag::Keep,
        Id3v1Tag::Always,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Id3v1Tag::Update => tr!("Updated when there is one", "Обновлять, если есть"),
            Id3v1Tag::Remove => tr!("Removed", "Удалять"),
            Id3v1Tag::Keep => tr!("Left as it was", "Не трогать"),
            Id3v1Tag::Always => tr!("Always written", "Писать всегда"),
        }
    }
}

/// The code page of an MP3's ID3v1 tag, whose fields are bytes, not
/// letters: Windows' own (the ANSI code page, as Winamp and the old
/// taggers wrote them and car stereos read them), or one named, for files
/// made on a Windows of another language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Id3v1Encoding {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "1251")]
    Cyrillic,
    #[serde(rename = "1252")]
    Western,
    #[serde(rename = "1250")]
    CentralEuropean,
    #[serde(rename = "866")]
    Dos,
    #[serde(rename = "koi8-r")]
    Koi8R,
}

impl Id3v1Encoding {
    pub const ALL: [Id3v1Encoding; 6] = [
        Id3v1Encoding::System,
        Id3v1Encoding::Cyrillic,
        Id3v1Encoding::Western,
        Id3v1Encoding::CentralEuropean,
        Id3v1Encoding::Dos,
        Id3v1Encoding::Koi8R,
    ];

    /// Its Windows code page; 0 for Windows' own (`CP_ACP`).
    pub fn code_page(self) -> u32 {
        match self {
            Id3v1Encoding::System => 0,
            Id3v1Encoding::Cyrillic => 1251,
            Id3v1Encoding::Western => 1252,
            Id3v1Encoding::CentralEuropean => 1250,
            Id3v1Encoding::Dos => 866,
            Id3v1Encoding::Koi8R => 20866,
        }
    }

    pub fn label(self) -> String {
        match self {
            Id3v1Encoding::System => {
                // SAFETY: no arguments.
                let ansi = unsafe { windows_sys::Win32::Globalization::GetACP() };
                let own = Self::ALL[1..]
                    .iter()
                    .find(|e| e.code_page() == ansi)
                    .map_or_else(|| ansi.to_string(), |e| e.name().to_owned());
                format!("{} ({own})", tr!("As in Windows", "Как в Windows"))
            }
            other => format!("{} — {}", other.name(), other.script()),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Id3v1Encoding::System => "",
            Id3v1Encoding::Cyrillic => "Windows-1251",
            Id3v1Encoding::Western => "Windows-1252",
            Id3v1Encoding::CentralEuropean => "Windows-1250",
            Id3v1Encoding::Dos => "DOS 866",
            Id3v1Encoding::Koi8R => "KOI8-R",
        }
    }

    fn script(self) -> &'static str {
        match self {
            Id3v1Encoding::System => "",
            Id3v1Encoding::Cyrillic | Id3v1Encoding::Dos | Id3v1Encoding::Koi8R => {
                tr!("Cyrillic", "кириллица")
            }
            Id3v1Encoding::Western => tr!("Western European", "западноевропейская"),
            Id3v1Encoding::CentralEuropean => {
                tr!("Central European", "центральноевропейская")
            }
        }
    }
}

/// What becomes of an MP3's APE tag (foobar2000's, MP3Gain's) when its
/// tags are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApeTag {
    /// The fields changed written into it too, the rest of it kept.
    #[default]
    Update,
    Remove,
    /// Left as it was.
    Keep,
}

impl ApeTag {
    pub const ALL: [ApeTag; 3] = [ApeTag::Update, ApeTag::Remove, ApeTag::Keep];

    pub fn label(self) -> &'static str {
        match self {
            ApeTag::Update => tr!("Updated when there is one", "Обновлять, если есть"),
            ApeTag::Remove => tr!("Removed", "Удалять"),
            ApeTag::Keep => tr!("Left as it was", "Не трогать"),
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

/// The enhancer's settings saved under a name: `{name, drive, …}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnhancerPreset {
    pub name: String,
    #[serde(flatten)]
    pub settings: crate::audio::dsp::Enhancement,
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
pub const PLAYLIST_MIN_SIZE: [f32; 2] = [380.0, 400.0];
pub const PLAYLIST_SIZE: [f32; 2] = [440.0, 780.0];

/// What the window is: the library's layout, the player and playlist
/// layout, or that layout with the playlist folded away, the player bar
/// alone (`Settings::player_only`). Each keeps its own place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Library,
    Playlist,
    Player,
}

/// The player alone's height in points: the title row and the narrow bar.
pub fn player_height() -> f32 {
    crate::ui::title::HEIGHT + crate::ui::player_bar::height(BarLayout::Winamp)
}

/// The seek keys' step by default and at most, in seconds.
pub const DEFAULT_SEEK_STEP: u8 = 5;
pub const MAX_SEEK_STEP: u8 = 60;

/// The volume keys' step by default and at most, in percent.
pub const DEFAULT_VOLUME_STEP: u8 = 5;
pub const MAX_VOLUME_STEP: u8 = 25;

/// How the mini player shows the cover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniStyle {
    /// The whole square (also for a value not known: `from_json`).
    #[default]
    Cover,
    /// A record turning, the cover its label.
    Record,
    /// The record seen closer: a larger label, a narrower band of
    /// grooves.
    CloseUp,
}

impl MiniStyle {
    /// A record, round in a window clear about it.
    pub fn is_record(self) -> bool {
        self != MiniStyle::Cover
    }
}

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
    /// The spectra of the last moments as lines one behind the other.
    Ridges,
    /// A string per band held at a nut on the left, the levels running
    /// right along them as vibrating swells, plucked by the beats.
    Strata,
    /// The spectrum's bands as colours rolling from the right.
    Spectrogram,
    /// A flight over a grid with mountains of the spectrum, a sun over
    /// the horizon.
    Synthwave,
    /// A flight through the stars, streaking as it speeds up with the
    /// music, a jump on the strong beats.
    Hyperspace,
    /// Bolts from a core in the middle, striking on the beats (the radial
    /// spectrum's place, which it takes in older settings).
    #[serde(alias = "radial")]
    Lightning,
    /// The waves heard flying out from the middle in swirling trails, a
    /// picture with feedback as MilkDrop's.
    Vortex,
    /// Flames rising from the spectrum along the bottom, as Doom's fire.
    Fire,
    /// Rockets sent up as notes start, shells bursting on the beats (the
    /// sparks' place, which it takes in older settings).
    #[serde(alias = "sparks")]
    Fireworks,
    /// Curtains of light over the spectrum, swaying.
    Aurora,
    /// A sheet in a frame, its near edge pushed by the spectrum, stones
    /// dropped on it on the beats, the treble raining on it.
    Cloth,
    /// A pond, drops falling on it as notes start (the bass on the left),
    /// their rings crossing; a great wave on the beats.
    Ripples,
    /// Three ribbons of light (the bass, the middle, the treble) winding
    /// across, turning, swelling with their bands, bent by the beats.
    Ribbons,
    /// The sound's peaks rolling from the right.
    Waveform,
    /// The wave itself.
    Scope,
    /// The stereo field: the mid up, the side across, as a phosphor's
    /// trace; and the channels' correlation.
    Goniometer,
    /// The twelve notes' loudness, the octaves added up.
    Chroma,
    /// The notes on a piano's keys, and a bar over each.
    Piano,
    /// The notes on a guitar's neck: the strings ringing, the frets
    /// played lit.
    Strings,
    /// A harp's strings, one a band, plucked as the bands' notes start.
    Harp,
    /// An empty display, and no frames drawn for it.
    Off,
}

impl Visual {
    /// As the display's menu and the settings list them, "none" last.
    pub const ALL: [Visual; 25] = [
        Visual::Spectrum,
        Visual::Led,
        Visual::Curve,
        Visual::Mirror,
        Visual::Ridges,
        Visual::Strata,
        Visual::Spectrogram,
        Visual::Synthwave,
        Visual::Hyperspace,
        Visual::Lightning,
        Visual::Vortex,
        Visual::Fire,
        Visual::Fireworks,
        Visual::Aurora,
        Visual::Cloth,
        Visual::Ripples,
        Visual::Ribbons,
        Visual::Waveform,
        Visual::Scope,
        Visual::Goniometer,
        Visual::Chroma,
        Visual::Piano,
        Visual::Strings,
        Visual::Harp,
        Visual::Off,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Visual::Spectrum => tr!("Spectrum", "Спектр"),
            Visual::Led => tr!("LED spectrum", "Светодиодный спектр"),
            Visual::Curve => tr!("Spectrum curve", "Спектр-кривая"),
            Visual::Mirror => tr!("Mirrored spectrum", "Зеркальный спектр"),
            Visual::Ridges => tr!("Ridges", "Хребты"),
            Visual::Strata => tr!("Strata", "Пласты"),
            Visual::Spectrogram => tr!("Spectrogram", "Спектрограмма"),
            Visual::Synthwave => tr!("Synthwave", "Синтвейв"),
            Visual::Hyperspace => tr!("Hyperspace", "Гиперпространство"),
            Visual::Lightning => tr!("Lightning", "Молнии"),
            Visual::Vortex => tr!("Vortex", "Вихрь"),
            Visual::Fire => tr!("Fire", "Огонь"),
            Visual::Fireworks => tr!("Fireworks", "Фейерверк"),
            Visual::Aurora => tr!("Aurora", "Северное сияние"),
            Visual::Cloth => tr!("Cloth", "Полотно"),
            Visual::Ripples => tr!("Ripples", "Рябь"),
            Visual::Ribbons => tr!("Ribbons", "Ленты"),
            Visual::Waveform => tr!("Waveform", "Волна"),
            Visual::Scope => tr!("Oscilloscope", "Осциллограф"),
            Visual::Goniometer => tr!("Goniometer", "Гониометр"),
            Visual::Chroma => tr!("Notes", "Ноты"),
            Visual::Piano => tr!("Piano", "Клавиатура"),
            Visual::Strings => tr!("Guitar strings", "Гитарные струны"),
            Visual::Harp => tr!("Harp", "Арфа"),
            Visual::Off => tr!("No visualization", "Без визуализации"),
        }
    }

    /// The first of a group in the menus (a line over it): the spectrum's
    /// histories, the scenes, the wave, the notes, none.
    pub fn starts_group(self) -> bool {
        matches!(
            self,
            Visual::Ridges | Visual::Synthwave | Visual::Waveform | Visual::Chroma | Visual::Off
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
            player_window: None,
            window_layout: WindowLayout::default(),
            player_only: false,
            visual_enabled: true,
            visual_click: true,
            visual_colours: Default::default(),
            playlist_visual_colours: Default::default(),
            visual_scheme: Default::default(),
            playlist_format: crate::title_format::DEFAULT.to_owned(),
            playlist_numbers: true,
            playlist_lengths: true,
            library_folders: Vec::new(),
            views: Vec::new(),
            view: String::new(),
            album_view: AlbumView::Grid,
            cover_size: 152.0,
            cover_priority: CoverPriority::default(),
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
            playlist_under_shown: true,
            playlist_under: UnderShows::Auto,
            lyrics_window: false,
            lyrics_window_place: None,
            lyrics_online: true,
            lyrics_priority: LyricsPriority::default(),
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
            enhancer_presets: Vec::new(),
            eq_bands_open: true,
            eq_enhancer_open: true,
            crossfade: 0,
            crossfade_in_albums: false,
            crossfade_manual: 0.0,
            crossfade_seek: false,
            seek_step: DEFAULT_SEEK_STEP,
            volume_step: DEFAULT_VOLUME_STEP,
            media_keys: MediaKeys::default(),
            tag_cover_side: CoverSide::default(),
            prepare_presets: Vec::new(),
            prepare_preset: String::new(),
            id3v2_version: Id3Version::default(),
            id3v1: Id3v1Tag::default(),
            id3v1_encoding: Id3v1Encoding::default(),
            ape: ApeTag::default(),
            keys: crate::keymap::Keymap::default(),
            tray: false,
            tray_only: false,
            track_notice: TrackNotice::default(),
            track_notice_seconds: DEFAULT_NOTICE_SECONDS,
            on_top: false,
            mini: false,
            mini_rect: None,
            mini_names: true,
            mini_style: MiniStyle::Cover,
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
    /// What the window is in the layout chosen.
    pub fn shape(&self) -> Shape {
        match self.window_layout {
            WindowLayout::Library => Shape::Library,
            WindowLayout::Playlist if self.player_only => Shape::Player,
            WindowLayout::Playlist => Shape::Playlist,
        }
    }

    /// The place of the window in the shape it has.
    pub fn place(&self) -> Option<WindowPlace> {
        self.place_of(self.shape())
    }

    pub fn place_of(&self, shape: Shape) -> Option<WindowPlace> {
        match shape {
            Shape::Library => self.window,
            Shape::Playlist => self.playlist_window,
            Shape::Player => self.player_window,
        }
    }

    pub fn place_mut(&mut self) -> &mut Option<WindowPlace> {
        self.place_of_mut(self.shape())
    }

    pub fn place_of_mut(&mut self, shape: Shape) -> &mut Option<WindowPlace> {
        match shape {
            Shape::Library => &mut self.window,
            Shape::Playlist => &mut self.playlist_window,
            Shape::Player => &mut self.player_window,
        }
    }

    /// The window's least size in the shape it has, in points.
    pub fn min_size(&self) -> [f32; 2] {
        match self.shape() {
            Shape::Library => crate::MIN_SIZE,
            Shape::Playlist => PLAYLIST_MIN_SIZE,
            Shape::Player => [PLAYLIST_MIN_SIZE[0], player_height()],
        }
    }

    /// The window's greatest height in the shape it has, in points (none:
    /// any): the player alone is as high as its bar.
    pub fn most_height(&self) -> Option<f32> {
        (self.shape() == Shape::Player).then(player_height)
    }

    /// The window's size at first in the shape it has, in points.
    pub fn default_size(&self) -> [f32; 2] {
        match self.shape() {
            Shape::Library => crate::DEFAULT_SIZE,
            Shape::Playlist => PLAYLIST_SIZE,
            Shape::Player => [PLAYLIST_SIZE[0], player_height()],
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

    /// The visualization's colours of the window layout in use (`look`
    /// reads them).
    pub fn visual_colours_mut(&mut self) -> &mut crate::palette::VisualColours {
        match self.window_layout {
            WindowLayout::Library => &mut self.visual_colours,
            WindowLayout::Playlist => &mut self.playlist_visual_colours,
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
        let Ok(Value::Object(mut file)) = serde_json::from_slice::<Value>(bytes) else {
            return None;
        };
        // Before the panel under the playlist had one switch: shown when
        // the cover or the lyrics were, showing either as there are lyrics.
        if !file.contains_key("playlist_under_shown") {
            let was = |key: &str, default: bool| file.get(key).and_then(Value::as_bool).unwrap_or(default);
            if file.contains_key("playlist_cover") || file.contains_key("playlist_lyrics") {
                let shown = was("playlist_cover", true) || was("playlist_lyrics", false);
                file.insert("playlist_under_shown".into(), Value::Bool(shown));
            }
        }
        // Before the own scheme, the visualization's own colour: the
        // scheme's three colours now (the library's layout's first).
        if !file.contains_key("visual_scheme") {
            let own = ["visual_colours", "playlist_visual_colours"]
                .iter()
                .filter_map(|key| file.get(*key).and_then(Value::as_str))
                .find_map(crate::palette::parse_hex);
            if let Some([r, g, b]) = own {
                let scheme = crate::palette::VisualScheme::of_one(u32::from_be_bytes([0, r, g, b]));
                if let Ok(value) = serde_json::to_value(scheme) {
                    file.insert("visual_scheme".into(), value);
                }
            }
        }
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
    /// The panel under the playlist: shown as the older switches had the
    /// cover or the lyrics shown, then showing either as there are lyrics.
    #[test]
    fn the_panel_under_the_playlist_reads_the_older_switches() {
        let read = |json: &str| {
            let s = Settings::from_json(json.as_bytes()).unwrap();
            (s.playlist_under_shown, s.playlist_under)
        };
        assert_eq!(read("{}"), (true, UnderShows::Auto));
        assert_eq!(read(r#"{"playlist_cover":false}"#), (false, UnderShows::Auto));
        assert_eq!(
            read(r#"{"playlist_cover":false,"playlist_lyrics":true}"#),
            (true, UnderShows::Auto)
        );
        assert_eq!(read(r#"{"playlist_lyrics":false}"#), (true, UnderShows::Auto));
        assert_eq!(
            read(r#"{"playlist_cover":true,"playlist_under_shown":false,"playlist_under":"lyrics"}"#),
            (false, UnderShows::Lyrics)
        );
    }

    /// An older version's own colour of the visualization is the scheme,
    /// its three colours that colour; a scheme kept is not overwritten.
    #[test]
    fn the_own_colour_becomes_the_scheme() {
        use crate::palette::{VisualColours, VisualScheme};
        let s = Settings::from_json(br##"{"visual_colours":"#2F7CF0"}"##).unwrap();
        assert_eq!(s.visual_colours, VisualColours::Scheme);
        assert_eq!(s.visual_scheme, VisualScheme::of_one(0x2f7cf0));
        let s = Settings::from_json(br##"{"playlist_visual_colours":"#102030"}"##).unwrap();
        assert_eq!(s.visual_colours, VisualColours::Own);
        assert_eq!(s.playlist_visual_colours, VisualColours::Scheme);
        assert_eq!(s.visual_scheme, VisualScheme::of_one(0x102030));
        let kept = r##"{"visual_colours":"#2F7CF0","visual_scheme":{"low":"#010203","middle":"#040506","high":"#070809","peak":"#0A0B0C"}}"##;
        let s = Settings::from_json(kept.as_bytes()).unwrap();
        assert_eq!(
            s.visual_scheme,
            VisualScheme {
                low: 0x010203,
                middle: 0x040506,
                high: 0x070809,
                peak: Some(0x0a0b0c)
            }
        );
        let s = Settings::from_json(b"{}").unwrap();
        assert_eq!(s.visual_scheme, VisualScheme::default());
    }

    /// The ID3v1 code page is kept by its name; a file without it, or with
    /// one this version does not know, reads Windows' own.
    #[test]
    fn the_id3v1_code_page_is_kept() {
        let s = Settings::from_json(br#"{"id3v1_encoding":"koi8-r"}"#).unwrap();
        assert_eq!(s.id3v1_encoding, Id3v1Encoding::Koi8R);
        assert_eq!(s.id3v1_encoding.code_page(), 20866);
        let s = Settings::from_json(br#"{"id3v1_encoding":"1253"}"#).unwrap();
        assert_eq!(s.id3v1_encoding, Id3v1Encoding::System);
        assert_eq!(Settings::from_json(b"{}").unwrap().id3v1_encoding.code_page(), 0);
        for e in Id3v1Encoding::ALL {
            let json = serde_json::to_string(&e).unwrap();
            assert_eq!(serde_json::from_str::<Id3v1Encoding>(&json).unwrap(), e);
            assert!(!e.label().is_empty());
        }
    }

    /// The cover's side is kept as `original` or its pixels, one's own
    /// size too; a ready size typed is that one; outside `OWN` it is not
    /// taken, and a file holding such reads the default.
    #[test]
    fn the_covers_side_is_kept() {
        for side in CoverSide::ALL.into_iter().chain([CoverSide::Custom(800)]) {
            let json = serde_json::to_string(&side).unwrap();
            assert_eq!(serde_json::from_str::<CoverSide>(&json).unwrap(), side);
        }
        assert_eq!(serde_json::to_string(&CoverSide::Original).unwrap(), r#""original""#);
        assert_eq!(serde_json::to_string(&CoverSide::Custom(1200)).unwrap(), r#""1200""#);
        assert_eq!(CoverSide::typed(" 600 "), Some(CoverSide::Px600));
        assert_eq!(CoverSide::typed("500"), Some(CoverSide::Custom(500)));
        assert_eq!(CoverSide::default(), CoverSide::Px600);
        assert_eq!(CoverSide::typed("1200"), Some(CoverSide::Custom(1200)));
        assert_eq!(CoverSide::typed("99"), None);
        assert_eq!(CoverSide::typed("9000"), None);
        assert_eq!(CoverSide::typed("abc"), None);
        let s = Settings::from_json(br#"{"tag_cover_side":"700"}"#).unwrap();
        assert_eq!(s.tag_cover_side, CoverSide::Custom(700));
        let s = Settings::from_json(br#"{"tag_cover_side":"50"}"#).unwrap();
        assert_eq!(s.tag_cover_side, CoverSide::default());
    }

    #[test]
    fn the_enhancers_settings_are_kept() {
        let mut new = Settings::default();
        assert_eq!(
            new.dsp.enhancement,
            crate::audio::dsp::Enhancement::LOUD
        );
        new.dsp.enhancer = true;
        new.dsp.enhancement.drive = 90;
        new.dsp.enhancement.bass_type = -40;
        new.dsp.enhancement.ambience = 3.5;
        new.enhancer_presets.push(EnhancerPreset {
            name: "Mine".to_owned(),
            settings: new.dsp.enhancement,
        });
        let json = serde_json::to_vec(&new).unwrap();
        let again = Settings::from_json(&json).unwrap();
        assert_eq!(again.dsp, new.dsp);
        assert_eq!(again.enhancer_presets, new.enhancer_presets);
        // One's own preset is its name beside the controls.
        let text = String::from_utf8(json).unwrap();
        assert!(
            text.contains(r#"{"name":"Mine","drive":90,"bass_type":-40"#),
            "{text}"
        );
    }

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
        // And its own colours.
        use crate::palette::VisualColours;
        *s.visual_colours_mut() = VisualColours::Fire;
        s.window_layout = WindowLayout::Playlist;
        assert_eq!(s.look().visual, VisualColours::Own);
        *s.visual_colours_mut() = VisualColours::Rainbow;
        assert_eq!(s.visual_colours, VisualColours::Fire);
        s.window_layout = WindowLayout::Library;
        assert_eq!(s.look().visual, VisualColours::Fire);
        // A click goes through "none" too.
        assert_eq!(Visual::Piano.next(), Visual::Strings);
        assert_eq!(Visual::Strings.next(), Visual::Harp);
        assert_eq!(Visual::Harp.next(), Visual::Off);
        assert_eq!(Visual::Off.next(), Visual::Spectrum);
        assert_eq!(Visual::Spectrum.next(), Visual::Led);
        // Kept by their names.
        for (v, name) in [
            (Visual::Led, "led"),
            (Visual::Ridges, "ridges"),
            (Visual::Strata, "strata"),
            (Visual::Chroma, "chroma"),
            (Visual::Fire, "fire"),
            (Visual::Fireworks, "fireworks"),
            (Visual::Hyperspace, "hyperspace"),
            (Visual::Aurora, "aurora"),
            (Visual::Cloth, "cloth"),
            (Visual::Ripples, "ripples"),
            (Visual::Ribbons, "ribbons"),
            (Visual::Goniometer, "goniometer"),
            (Visual::Strings, "strings"),
            (Visual::Harp, "harp"),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), format!("\"{name}\""));
        }
        // The radial spectrum gave way to the lightning, the sparks to the
        // fireworks.
        assert_eq!(
            serde_json::from_str::<Visual>("\"radial\"").unwrap(),
            Visual::Lightning
        );
        assert_eq!(
            serde_json::from_str::<Visual>("\"sparks\"").unwrap(),
            Visual::Fireworks
        );
        assert_eq!(
            serde_json::to_string(&Visual::Synthwave).unwrap(),
            "\"synthwave\""
        );
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
        // The player alone: a place of its own, as high as the bar.
        s.player_only = true;
        assert_eq!(s.shape(), Shape::Player);
        assert_eq!(s.place(), None);
        assert_eq!(s.min_size()[1], player_height());
        assert_eq!(s.most_height(), Some(player_height()));
        s.player_only = false;
        assert_eq!(s.place(), Some(place));
        assert_eq!(s.most_height(), None);
        s.window_layout = WindowLayout::Library;
        assert_eq!(s.place(), None);
        // Folded away only in the player and playlist layout.
        s.player_only = true;
        assert_eq!(s.shape(), Shape::Library);
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
            cover_priority: CoverPriority::Folder,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        let partial: Settings = serde_json::from_str(r#"{"language":"en","future":1}"#).unwrap();
        assert_eq!(partial.language, LangChoice::En);
        assert!(!partial.check_updates);
        assert_eq!(partial.window, None);
        assert_eq!(partial.cover_priority, CoverPriority::Tags);
    }

    #[test]
    fn a_field_that_cannot_be_read_costs_no_other() {
        let json = r#"{"theme":"ultraviolet","visualization":7,"media_keys":"later","cover_priority":"later",
            "repeat":"twice","album_view":null,"dsp":{"bands":[1,2]},
            "library_folders":["D:\\Music"],"volume":35,"future":{"a":1}}"#;
        let s = Settings::from_json(json.as_bytes()).unwrap();
        assert_eq!(s.library_folders, vec![PathBuf::from(r"D:\Music")]);
        assert_eq!(s.volume, 35);
        let d = Settings::default();
        assert_eq!(s.theme, d.theme);
        assert_eq!(s.visualization, d.visualization);
        assert_eq!(s.media_keys, d.media_keys);
        assert_eq!(s.cover_priority, CoverPriority::Tags);
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
