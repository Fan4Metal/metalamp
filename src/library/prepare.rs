//! Preparing an album, as its owner did it in Tag&Rename: the album's
//! title without the edition (`[Deluxe]`), the track numbers as `01`, the
//! album artist, year and genre alike in every file, the tag's parts not
//! wanted removed, `folder.jpg` and the same cover in every tag, the files
//! named `01 - Title.mp3` and the folder `(2011) Album`, the other files
//! of the folder deleted. What is done is the user's [`Profile`].
//!
//! Three steps: [`read`] (a thread: every file's tag `edit::inspect`ed,
//! the folder's other files, the cover's source made ready for the tags),
//! [`plan`] (pure: what would change, worked out again whenever the profile
//! changes, so the window shows it at once) and [`apply`] (a thread: the
//! tags written, `folder.jpg`, the other files, the files and the folder
//! renamed, a journal written next to the settings).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use super::edit::{self, Item, Key, MAIN, TagPart, WriteTags};

/// A field every file is to have; a file without it is pointed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Year,
    Genre,
    Track,
    Cover,
    Lyrics,
}

impl Need {
    pub const ALL: [Need; 9] = [
        Need::Title,
        Need::Artist,
        Need::Album,
        Need::AlbumArtist,
        Need::Year,
        Need::Genre,
        Need::Track,
        Need::Cover,
        Need::Lyrics,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Need::Cover => tr!("Cover", "Обложка"),
            Need::Lyrics => tr!("Lyrics", "Текст песни"),
            other => other.key().map_or("", Key::label),
        }
    }

    fn key(self) -> Option<Key> {
        Some(match self {
            Need::Title => Key::Title,
            Need::Artist => Key::Artist,
            Need::Album => Key::Album,
            Need::AlbumArtist => Key::AlbumArtist,
            Need::Year => Key::Year,
            Need::Genre => Key::Genre,
            Need::Track => Key::Track,
            Need::Cover | Need::Lyrics => return None,
        })
    }
}

/// What becomes of the folder's files that are neither the album's audio
/// nor `folder.jpg` (scans, `.cue`, `.log`, `.m3u`, `.nfo`…).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extras {
    Keep,
    /// Into the recycle bin.
    #[default]
    Delete,
    /// Into a subfolder (`Profile::extras_folder`).
    Folder,
}

impl Extras {
    pub const ALL: [Extras; 3] = [Extras::Keep, Extras::Delete, Extras::Folder];

    pub fn label(self) -> &'static str {
        match self {
            Extras::Keep => tr!("Keep", "Оставить"),
            Extras::Delete => tr!("Into the recycle bin", "В корзину"),
            Extras::Folder => tr!("Into a subfolder", "В подпапку"),
        }
    }
}

/// How the track's number is written (Tag&Rename's "Format Track #").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackFormat {
    /// As it is.
    Keep,
    /// `1`
    Plain,
    /// `01`
    Two,
    /// `001`
    Three,
    /// `01`, or `001` on an album of more than 99 tracks.
    #[default]
    Auto,
    /// `01/16`: the number and the disc's tracks.
    WithTotal,
}

impl TrackFormat {
    pub const ALL: [TrackFormat; 6] = [
        TrackFormat::Keep,
        TrackFormat::Plain,
        TrackFormat::Two,
        TrackFormat::Three,
        TrackFormat::Auto,
        TrackFormat::WithTotal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TrackFormat::Keep => tr!("As it is", "Как есть"),
            TrackFormat::Plain => "x",
            TrackFormat::Two => "0x",
            TrackFormat::Three => "00x",
            TrackFormat::Auto => tr!("0x, 00x past 99 tracks", "0x, при 100+ треках 00x"),
            TrackFormat::WithTotal => "0x/xx",
        }
    }

    /// `n` of `total`, `width` the digits of the album's largest number.
    fn format(self, n: u32, total: u32, width: usize) -> Option<String> {
        Some(match self {
            TrackFormat::Keep => return None,
            TrackFormat::Plain => n.to_string(),
            TrackFormat::Two => format!("{n:02}"),
            TrackFormat::Three => format!("{n:03}"),
            TrackFormat::Auto => format!("{n:0width$}"),
            TrackFormat::WithTotal => format!("{n:0width$}/{total:0width$}"),
        })
    }
}

/// How the disc's number is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscFormat {
    Keep,
    Remove,
    /// `2` on an album of several discs, none on one of one.
    #[default]
    Several,
    /// `1` always.
    Plain,
    /// `1/2`.
    WithTotal,
}

impl DiscFormat {
    pub const ALL: [DiscFormat; 5] = [
        DiscFormat::Keep,
        DiscFormat::Remove,
        DiscFormat::Several,
        DiscFormat::Plain,
        DiscFormat::WithTotal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DiscFormat::Keep => tr!("As it is", "Как есть"),
            DiscFormat::Remove => tr!("Removed", "Удалять"),
            DiscFormat::Several => tr!("x, only with several discs", "x, только у нескольких дисков"),
            DiscFormat::Plain => "x",
            DiscFormat::WithTotal => "x/x",
        }
    }
}

/// A field made alike in every file (the value most of them have).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unify {
    Album,
    AlbumArtist,
    Year,
    Genre,
}

impl Unify {
    pub const ALL: [Unify; 4] = [Unify::Album, Unify::AlbumArtist, Unify::Year, Unify::Genre];

    pub fn key(self) -> Key {
        match self {
            Unify::Album => Key::Album,
            Unify::AlbumArtist => Key::AlbumArtist,
            Unify::Year => Key::Year,
            Unify::Genre => Key::Genre,
        }
    }
}

/// An empty field filled from another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    /// From the artists (Various Artists for a compilation).
    AlbumArtist,
    /// From the file's name, without its number.
    Title,
    /// From the album artist.
    Artist,
}

impl Fill {
    pub const ALL: [Fill; 3] = [Fill::AlbumArtist, Fill::Title, Fill::Artist];

    pub fn label(self) -> &'static str {
        match self {
            Fill::AlbumArtist => tr!("Album artist from the artists", "Исполнитель альбома — из исполнителей"),
            Fill::Title => tr!("Title from the file's name", "Название — из имени файла"),
            Fill::Artist => tr!("Artist from the album artist", "Исполнитель — из исполнителя альбома"),
        }
    }
}

/// What becomes of the covers in the tags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagCover {
    /// Left as they are.
    Keep,
    /// Put only into the files without one.
    Missing,
    /// A cover that fits (`Profile::cover_min`, `cover_max`, `cover_kb`)
    /// stays; the others are replaced.
    #[default]
    Fitting,
    /// Every file gets the album's cover.
    Replace,
}

impl TagCover {
    pub const ALL: [TagCover; 4] = [TagCover::Keep, TagCover::Missing, TagCover::Fitting, TagCover::Replace];

    pub fn label(self) -> &'static str {
        match self {
            TagCover::Keep => tr!("Left as they are", "Не трогать"),
            TagCover::Missing => tr!("Put in where none", "Добавить, где нет"),
            TagCover::Fitting => tr!("Replace those that do not fit", "Заменить неподходящие"),
            TagCover::Replace => tr!("Replace in every file", "Заменить во всех"),
        }
    }
}

/// Where the album's cover is taken from first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverFrom {
    /// `folder.jpg`, another picture of the folder, then the tags.
    #[default]
    Folder,
    /// The cover most tags hold, then the folder.
    Tags,
}

impl CoverFrom {
    pub const ALL: [CoverFrom; 2] = [CoverFrom::Folder, CoverFrom::Tags];

    pub fn label(self) -> &'static str {
        match self {
            CoverFrom::Folder => tr!("The folder, then the tags", "Из папки, затем из тегов"),
            CoverFrom::Tags => tr!("The tags, then the folder", "Из тегов, затем из папки"),
        }
    }
}

/// What becomes of `folder.jpg`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderJpg {
    Keep,
    /// The picture the tags get.
    #[default]
    Same,
    /// The cover's picture as it is (a PNG made a JPEG).
    Original,
}

impl FolderJpg {
    pub const ALL: [FolderJpg; 3] = [FolderJpg::Keep, FolderJpg::Same, FolderJpg::Original];

    pub fn label(self) -> &'static str {
        match self {
            FolderJpg::Keep => tr!("Left as it is", "Не трогать"),
            FolderJpg::Same => tr!("The picture the tags get", "Та же, что в тегах"),
            FolderJpg::Original => tr!("The source picture as it is", "Исходная картинка"),
        }
    }
}

/// What the preparation does: a preset's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub track: TrackFormat,
    pub disc: DiscFormat,
    /// The edition cut from the album's title: a part in brackets at its
    /// end holding one of `editions` (`Album [Deluxe Edition]`).
    pub album_title: bool,
    /// The words of an edition, separated by commas.
    pub editions: String,
    /// Every part in brackets at the title's end cut, whatever it says.
    pub album_brackets: bool,
    /// The year as its four digits (`2011-05-01` → `2011`).
    pub year_only: bool,
    pub unify: Vec<Unify>,
    pub fill: Vec<Fill>,
    /// The tag's parts not in `keep` removed.
    pub clean: bool,
    pub keep: Vec<TagPart>,
    /// An MP3's ID3v1 and APE tags removed (else as the settings say).
    pub only_id3v2: bool,
    /// The fields every file is to have.
    pub required: Vec<Need>,
    /// Fields read from the files' and the folder's names by masks
    /// (Tag&Rename's "Get Tags from File Name"), into the empty ones.
    pub from_names: bool,
    /// What is read goes over the tags' values too.
    pub names_over_tags: bool,
    /// The files' masks: each file is read by the first that fits it.
    pub file_masks: Vec<String>,
    /// The folder's masks: the first that fits.
    pub folder_masks: Vec<String>,
    /// `_` in the names read as a space.
    pub names_underscores: bool,
    /// The letters genres are separated by in one field (`Synth, Pop`);
    /// none: the field is one genre.
    pub genre_split: String,
    /// How many genres stay, the first ones; 0: all.
    pub genre_count: u8,
    /// What is written between the genres that stay.
    pub genre_join: String,
    /// The genre the artist's other albums in the library have, when
    /// they have one.
    pub genre_from_artist: bool,
    /// Each genre spelt as the library (or the usual genres) spells it.
    pub genre_spelling: bool,
    pub tag_cover: TagCover,
    /// A cover in a tag fits when its sides are within these pixels and
    /// it is no larger than `cover_kb` kilobytes.
    pub cover_min: u32,
    pub cover_max: u32,
    pub cover_kb: u32,
    pub cover_from: CoverFrom,
    /// The largest side of the cover put into the tags.
    pub cover_side: crate::settings::CoverSide,
    pub folder_jpg: FolderJpg,
    pub rename_files: bool,
    /// The files' names, by `title_format`'s templates (the extension
    /// follows); for an album of several artists the second.
    pub file_name: String,
    pub file_name_various: String,
    pub rename_folder: bool,
    pub folder_name: String,
    /// Spaces in the names made underscores.
    pub underscores: bool,
    pub extras: Extras,
    pub extras_folder: String,
    /// A file with the read-only attribute has it taken off to be written.
    pub clear_read_only: bool,
}

/// The editions' words by default: what a release is (an edition, a
/// version, an EP), not what the music is.
pub const EDITIONS: &str = "deluxe, limited, special, expanded, anniversary, collector, \
    remaster, reissue, bonus track, edition, digipak, japan, japanese, tour, version, \
    ep, single, lp, demo, promo";

/// The words by default before `ep`, `single` and the like were among
/// them: a preset that kept them gets the new ones.
pub const EDITIONS_BEFORE: &str = "deluxe, limited, special, expanded, anniversary, collector, \
    remaster, reissue, bonus track, edition, digipak, japan, tour, version";

/// The files' masks by default: `01 - Title`, `01. Title`, `01 Title`.
pub const FILE_MASKS: [&str; 3] = ["%track% - %title%", "%track%. %title%", "%track% %title%"];

/// The folder's masks by default; the last takes any name as the album's.
pub const FOLDER_MASKS: [&str; 8] = [
    "(%year%) %album%",
    "%year% - %album%",
    "%artist% - %year% - %album%",
    "%artist% - (%year%) %album%",
    "%artist% - %album% (%year%)",
    "%album% (%year%)",
    "%artist% - %album%",
    "%album%",
];

impl Default for Profile {
    /// Its first owner's way (Tag&Rename's), the preset by default.
    fn default() -> Self {
        Profile {
            track: TrackFormat::Auto,
            disc: DiscFormat::Several,
            album_title: true,
            editions: EDITIONS.to_owned(),
            album_brackets: false,
            year_only: true,
            unify: Unify::ALL.to_vec(),
            fill: Fill::ALL.to_vec(),
            clean: true,
            keep: vec![
                TagPart::Composer,
                TagPart::Lyrics,
                TagPart::ReplayGain,
                TagPart::MusicBrainz,
                TagPart::Rating,
                TagPart::People,
                TagPart::Release,
                TagPart::Music,
            ],
            only_id3v2: false,
            genre_split: ";,/|".to_owned(),
            genre_count: 1,
            genre_join: "; ".to_owned(),
            genre_from_artist: true,
            genre_spelling: true,
            required: vec![
                Need::Title,
                Need::Artist,
                Need::Album,
                Need::AlbumArtist,
                Need::Year,
                Need::Genre,
                Need::Track,
                Need::Cover,
            ],
            from_names: true,
            names_over_tags: false,
            file_masks: FILE_MASKS.iter().map(|m| (*m).to_owned()).collect(),
            folder_masks: FOLDER_MASKS.iter().map(|m| (*m).to_owned()).collect(),
            names_underscores: true,
            tag_cover: TagCover::Fitting,
            cover_min: 500,
            cover_max: 1500,
            cover_kb: 1000,
            cover_from: CoverFrom::Folder,
            cover_side: crate::settings::CoverSide::Px600,
            folder_jpg: FolderJpg::Same,
            rename_files: true,
            file_name: "%track% - %title%".to_owned(),
            file_name_various: "%track% - %artist% - %title%".to_owned(),
            rename_folder: true,
            folder_name: "(%year%) %album%".to_owned(),
            underscores: false,
            extras: Extras::Delete,
            extras_folder: "Scans".to_owned(),
            clear_read_only: true,
        }
    }
}

/// A profile kept under a name (the settings' `prepare_presets`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub profile: Profile,
}

/// The presets a first run gets: the album as its first owner keeps them,
/// and one that only renames.
pub fn first_presets() -> Vec<Preset> {
    let names = Profile {
        track: TrackFormat::Keep,
        disc: DiscFormat::Keep,
        album_title: false,
        year_only: false,
        unify: Vec::new(),
        fill: Vec::new(),
        clean: false,
        required: Vec::new(),
        tag_cover: TagCover::Keep,
        folder_jpg: FolderJpg::Keep,
        extras: Extras::Keep,
        genre_split: String::new(),
        genre_count: 0,
        genre_from_artist: false,
        genre_spelling: false,
        from_names: false,
        ..Profile::default()
    };
    vec![
        Preset {
            name: tr!("Standard", "Стандартный").to_owned(),
            profile: Profile::default(),
        },
        Preset {
            name: tr!("Names only", "Только имена").to_owned(),
            profile: names,
        },
    ]
}

/// A file as read.
#[derive(Clone, Debug)]
pub struct FileRead {
    pub path: PathBuf,
    pub tag: Result<edit::Inspected, String>,
    pub read_only: bool,
    pub writable: bool,
}

/// Where a cover the album may get is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    FolderJpg,
    /// Another picture of the folder named as a cover.
    Picture,
    /// A tag's: the one most files hold.
    Tag,
}

/// A cover the album may get.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// The picture, or a track whose tag holds it.
    pub from: PathBuf,
    pub kind: Kind,
    pub bytes: Arc<Vec<u8>>,
    pub size: Option<(u32, u32)>,
}

/// A cover made ready for the tags (`covers::for_tag`).
#[derive(Clone, Debug)]
pub struct Made {
    pub data: Arc<Vec<u8>>,
    pub size: (u32, u32),
}

/// `bytes` made ready for the tags, no side over `side`.
pub fn make_cover(bytes: &[u8], side: Option<u32>) -> Option<Made> {
    let made = crate::covers::for_tag(bytes.to_vec(), side, 16).ok()?;
    Some(Made {
        data: Arc::new(made.data),
        size: made.to,
    })
}

/// A picture's size from its header.
pub fn picture_size(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// What was read of an album and its folder.
#[derive(Clone, Debug, Default)]
pub struct Source {
    /// The album's folder: its files' one folder, or the folder holding
    /// theirs (the discs' `CD1`, `CD2`).
    pub folder: PathBuf,
    /// The files are in `folder` or its subfolders a level down; else
    /// all over, and the folder is not renamed.
    pub one_place: bool,
    /// In the album's order (disc, number, name).
    pub files: Vec<FileRead>,
    /// The folder's other files (and its subfolders without audio).
    pub extras: Vec<PathBuf>,
    pub folder_jpg: Option<(PathBuf, Arc<Vec<u8>>)>,
    /// The covers the album may get.
    pub candidates: Vec<Candidate>,
    /// Audio files of the folders that are not the album's: the folder is
    /// not renamed then.
    pub foreign: Vec<PathBuf>,
    /// What the library knows (set by the window, not read).
    pub known: Known,
}

/// What the library knows that a preparation uses: the genres as it
/// spells them, and each artist's genre.
#[derive(Clone, Debug, Default)]
pub struct Known {
    /// The library's genres and the usual ones it lacks.
    pub genres: Vec<String>,
    /// Lower-case artist (as albums are filed) to the genre most of its
    /// tracks have, the album prepared left out.
    pub artists: HashMap<String, String>,
}

/// What `library` knows, the files at `paths` (the album prepared) left
/// out of the artists' genres.
pub fn known(library: &super::Library, paths: &[PathBuf]) -> Known {
    let mine: HashSet<String> = paths.iter().map(|p| folded(p)).collect();
    let mut by_artist: HashMap<String, Vec<&str>> = HashMap::new();
    for t in &library.tracks {
        if !mine.contains(&folded(&t.path)) {
            by_artist
                .entry(t.filed_artist().trim().to_lowercase())
                .or_default()
                .push(&t.genre);
        }
    }
    let artists = by_artist
        .into_iter()
        .filter_map(|(artist, genres)| Some((artist, most_common(genres.into_iter())?)))
        .collect();
    let mut genres = super::genres::of_library(library);
    genres.extend(
        super::genres::common_besides(&genres)
            .into_iter()
            .map(str::to_owned),
    );
    Known { genres, artists }
}

/// What becomes of the album's genre this time, apart from the preset.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum GenreChoice {
    /// As the preset says.
    #[default]
    Preset,
    /// Left as the files have it.
    Keep,
    /// This one in every file.
    Own(String),
}

/// Which masks read the names this time, apart from the preset.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum MaskChoice {
    /// The preset's.
    #[default]
    Preset,
    /// None: nothing is read from the names.
    Off,
    /// This one.
    Own(String),
}

/// What is chosen for the album prepared, apart from its preset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThisAlbum {
    pub genre: GenreChoice,
    pub file_mask: MaskChoice,
    pub folder_mask: MaskChoice,
    /// Values typed for the album, put into every file as they are (an
    /// empty one is not put).
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<String>,
}

impl ThisAlbum {
    /// The value typed for a field of the album.
    pub fn own(&self, key: Key) -> Option<&str> {
        match key {
            Key::Album => &self.album,
            Key::AlbumArtist => &self.album_artist,
            Key::Year => &self.year,
            _ => &None,
        }
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    }

    pub fn own_mut(&mut self, key: Key) -> Option<&mut Option<String>> {
        match key {
            Key::Album => Some(&mut self.album),
            Key::AlbumArtist => Some(&mut self.album_artist),
            Key::Year => Some(&mut self.year),
            _ => None,
        }
    }
}

/// A genre field as the preset wants it: split at `genre_split`, each
/// spelt as the library spells it, each once, the first `genre_count`
/// joined with `genre_join`.
pub fn genres_of(text: &str, profile: &Profile, known: &Known) -> String {
    let split: Vec<char> = profile.genre_split.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out: Vec<String> = Vec::new();
    for g in text.split(|c| split.contains(&c)).map(str::trim).filter(|g| !g.is_empty()) {
        let lower = g.to_lowercase();
        let g = if profile.genre_spelling {
            known
                .genres
                .iter()
                .find(|k| k.to_lowercase() == lower)
                .cloned()
                .unwrap_or_else(|| g.to_owned())
        } else {
            g.to_owned()
        };
        if !out.iter().any(|o| o.to_lowercase() == lower) {
            out.push(g);
        }
    }
    if profile.genre_count > 0 {
        out.truncate(usize::from(profile.genre_count));
    }
    out.join(&profile.genre_join)
}

/// Reads the album of the files at `paths` (on a thread: every tag is
/// read, the covers it may get looked for).
pub fn read(paths: Vec<PathBuf>) -> Source {
    let mut files: Vec<FileRead> = paths
        .iter()
        .map(|path| {
            let tag = crate::guard::catch(path, || edit::inspect(path))
                .unwrap_or_else(|| Err(tr!("the tag cannot be read", "тег не читается").to_owned()));
            FileRead {
                path: path.clone(),
                tag,
                read_only: std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly()),
                writable: edit::writable(path),
            }
        })
        .collect();
    files.sort_by(|a, b| {
        let key = |f: &FileRead| {
            let main = f.tag.as_ref().map(|t| t.main.clone()).unwrap_or_default();
            (
                number_of(&main[Key::Disc as usize]).unwrap_or(0),
                number_of(&main[Key::Track as usize])
                    .or_else(|| leading_number(&stem(&f.path)))
                    .unwrap_or(u32::MAX),
            )
        };
        key(a).cmp(&key(b)).then_with(|| {
            crate::format::natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy())
        })
    });
    let (folder, one_place) = album_folder(&paths);
    let mine: HashSet<String> = paths.iter().map(|p| folded(p)).collect();
    let parents: HashSet<String> = paths.iter().filter_map(|p| p.parent()).map(folded).collect();
    let mut extras = Vec::new();
    let mut foreign = Vec::new();
    let mut folder_jpg = None;
    let mut look = |dir: &Path, top: bool, extras: &mut Vec<PathBuf>, foreign: &mut Vec<PathBuf>| {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut discs = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if top && parents.contains(&folded(&path)) {
                    discs.push(path);
                } else if holds_audio(&path) {
                    foreign.push(path);
                } else {
                    extras.push(path);
                }
            } else if crate::filetypes::is_audio(&path) {
                if !mine.contains(&folded(&path)) {
                    foreign.push(path);
                }
            } else if top && is_folder_jpg(&path) {
                if let Ok(bytes) = std::fs::read(&path) {
                    folder_jpg = Some((path, Arc::new(bytes)));
                }
            } else {
                extras.push(path);
            }
        }
        discs
    };
    if one_place {
        let discs = look(&folder, true, &mut extras, &mut foreign);
        for disc in discs {
            look(&disc, false, &mut extras, &mut foreign);
        }
    }
    extras.sort();
    foreign.sort();
    let candidates = candidates(&folder, folder_jpg.as_ref(), &files);
    Source {
        folder,
        one_place,
        files,
        extras,
        folder_jpg,
        candidates,
        foreign,
        known: Known::default(),
    }
}

/// The folder an album's files are in, and whether they are all in it or
/// its subfolders a level down.
fn album_folder(paths: &[PathBuf]) -> (PathBuf, bool) {
    let parents: Vec<&Path> = paths.iter().filter_map(|p| p.parent()).collect();
    let Some(first) = parents.first() else {
        return (PathBuf::new(), false);
    };
    if parents.iter().all(|p| folded(p) == folded(first)) {
        return (first.to_path_buf(), true);
    }
    // The discs' folders: their common parent.
    let grand: Vec<&Path> = parents.iter().filter_map(|p| p.parent()).collect();
    if let Some(g) = grand.first()
        && grand.iter().all(|p| folded(p) == folded(g))
    {
        return (g.to_path_buf(), true);
    }
    let mut common = first.to_path_buf();
    while !parents.iter().all(|p| p.starts_with(&common)) {
        if !common.pop() {
            break;
        }
    }
    (common, false)
}

/// The covers the album may get: `folder.jpg`, another picture of the
/// folder named as a cover, the cover most of its tags hold.
fn candidates(
    folder: &Path,
    folder_jpg: Option<&(PathBuf, Arc<Vec<u8>>)>,
    files: &[FileRead],
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut push = |from: PathBuf, kind: Kind, bytes: Arc<Vec<u8>>| {
        let size = picture_size(&bytes);
        out.push(Candidate {
            from,
            kind,
            bytes,
            size,
        });
    };
    if let Some((path, bytes)) = folder_jpg {
        push(path.clone(), Kind::FolderJpg, bytes.clone());
    }
    if let Some(path) = crate::covers::in_folder(folder)
        && !is_folder_jpg(&path)
        && let Ok(bytes) = std::fs::read(&path)
    {
        push(path, Kind::Picture, Arc::new(bytes));
    }
    let mut counts: Vec<(&[u8], usize, &Path)> = Vec::new();
    for f in files {
        if let Ok(tag) = &f.tag
            && let Some(cover) = &tag.cover
        {
            match counts.iter_mut().find(|(b, ..)| *b == cover.as_slice()) {
                Some((_, n, _)) => *n += 1,
                None => counts.push((cover.as_slice(), 1, &f.path)),
            }
        }
    }
    let most = counts.iter().map(|(_, n, _)| *n).max().unwrap_or(0);
    if let Some((bytes, _, path)) = counts.into_iter().find(|(_, n, _)| *n == most) {
        push(path.to_path_buf(), Kind::Tag, Arc::new(bytes.to_vec()));
    }
    out
}

fn is_folder_jpg(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("folder.jpg"))
}

fn holds_audio(dir: &Path) -> bool {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .any(|e| e.file_type().is_file() && crate::filetypes::is_audio(e.path()))
}

/// A path as Windows compares it.
fn folded(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The number of `3`, `03` or `3/12`.
fn number_of(text: &str) -> Option<u32> {
    text.split('/').next()?.trim().parse().ok()
}

/// The number a name starts with (`01 - Title`, `1. Title`).
fn leading_number(text: &str) -> Option<u32> {
    let digits: String = text.trim().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// A file's name without the number it starts with and the separator
/// after it (`01 - Title` → `Title`).
fn title_from_name(path: &Path) -> String {
    let stem = stem(path);
    let rest = stem.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() == stem.len() {
        return stem.trim().to_owned();
    }
    rest.trim_start_matches([' ', '-', '.', '_'])
        .trim()
        .to_owned()
}

/// The digits in a disc's folder's name (`CD2`, `Disc 2`).
fn disc_of_folder(path: &Path) -> Option<u32> {
    let name = path.parent()?.file_name()?.to_string_lossy().into_owned();
    let digits: String = name
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The fields a mask reads: a template's, and `skip` for a part that is
/// not read.
pub const MASK_FIELDS: [&str; 9] = [
    "track",
    "title",
    "artist",
    "album",
    "albumartist",
    "year",
    "disc",
    "genre",
    "skip",
];

/// A mask's piece: text as it is, or a field.
enum Piece {
    Text(Vec<char>),
    Field(String),
}

fn pieces(mask: &str) -> Vec<Piece> {
    let chars: Vec<char> = mask.chars().collect();
    let mut out = Vec::new();
    let mut text = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '%'
            && let Some(end) = chars[at + 1..].iter().position(|&c| c == '%')
        {
            let name: String = chars[at + 1..at + 1 + end].iter().collect::<String>().to_lowercase();
            if MASK_FIELDS.contains(&name.as_str()) {
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Field(name));
                at += end + 2;
                continue;
            }
        }
        text.push(chars[at]);
        at += 1;
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    out
}

/// What `mask` reads from `name`: each field and its value (trimmed);
/// none when the name does not fit it. `%year%` is four digits, `%track%`
/// and `%disc%` one to three, the other fields anything but blank; the
/// text between them is matched in any case.
pub fn read_name(mask: &str, name: &str) -> Option<Vec<(String, String)>> {
    let pieces = pieces(mask);
    if !pieces.iter().any(|p| matches!(p, Piece::Field(f) if f != "skip")) {
        return None;
    }
    let chars: Vec<char> = name.chars().collect();
    let mut found = Vec::new();
    fits_mask(&pieces, &chars, &mut found).then_some(found)
}

fn fits_mask(pieces: &[Piece], text: &[char], found: &mut Vec<(String, String)>) -> bool {
    let Some((first, rest)) = pieces.split_first() else {
        return text.is_empty();
    };
    match first {
        Piece::Text(t) => {
            text.len() >= t.len()
                && text[..t.len()]
                    .iter()
                    .zip(t)
                    .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                && fits_mask(rest, &text[t.len()..], found)
        }
        Piece::Field(name) => {
            let digits = matches!(name.as_str(), "year" | "track" | "disc");
            // Numbers as long as they go; text as short as fits, the last
            // field all that is left.
            let lengths: Vec<usize> = match name.as_str() {
                "year" => vec![4],
                "track" | "disc" => vec![3, 2, 1],
                _ if rest.is_empty() => vec![text.len()],
                _ => (1..=text.len()).collect(),
            };
            for len in lengths.into_iter().filter(|&l| l <= text.len()) {
                let value: String = text[..len].iter().collect();
                let ok = if digits {
                    value.chars().all(|c| c.is_ascii_digit())
                } else {
                    !value.trim().is_empty()
                };
                if !ok {
                    continue;
                }
                found.push((name.clone(), value.trim().to_owned()));
                if fits_mask(rest, &text[len..], found) {
                    return true;
                }
                found.pop();
            }
            false
        }
    }
}

/// What a mask reads `path` as: as many of its last parts as the mask
/// has (`%artist%\%year% - %album%` two), a file's without its extension,
/// `_` made spaces when asked.
fn name_for_mask(mask: &str, path: &Path, file: bool, underscores: bool) -> String {
    let levels = mask.matches(['\\', '/']).count() + 1;
    let path = if file { path.with_extension("") } else { path.to_path_buf() };
    let parts: Vec<String> = path
        .components()
        .rev()
        .take(levels)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let name: Vec<String> = parts.into_iter().rev().collect();
    let name = name.join("\\");
    if underscores { name.replace('_', " ") } else { name }
}

/// The tag's field a mask's field fills: the folder's `%artist%` is the
/// album artist.
fn mask_key(field: &str, folder: bool) -> Option<Key> {
    Some(match field {
        "track" => Key::Track,
        "title" => Key::Title,
        "artist" if folder => Key::AlbumArtist,
        "artist" => Key::Artist,
        "album" => Key::Album,
        "albumartist" => Key::AlbumArtist,
        "year" => Key::Year,
        "disc" => Key::Disc,
        "genre" => Key::Genre,
        _ => return None,
    })
}

/// `title` without the parts in brackets at its end that name an edition
/// (any of `words`, separated by commas, in any case).
pub fn without_edition(title: &str, words: &str, all: bool) -> String {
    edition_cut(title, words, all).0
}

/// A part cut from an album's title, and the word of the list that named
/// it (none: every part in brackets at the end is cut).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cut {
    pub part: String,
    pub word: Option<String>,
}

/// `without_edition`, with what was cut and why, from the end.
pub fn edition_cut(title: &str, words: &str, all: bool) -> (String, Vec<Cut>) {
    let phrases: Vec<Vec<String>> = words.split(',').map(words_of).filter(|p| !p.is_empty()).collect();
    let named_by = |inner: &str| {
        let said = words_of(inner);
        phrases
            .iter()
            .find(|phrase| {
                (0..said.len()).any(|at| {
                    phrase
                        .iter()
                        .enumerate()
                        .all(|(i, w)| said.get(at + i).is_some_and(|s| same_word(s, w)))
                })
            })
            .map(|phrase| phrase.join(" "))
    };
    let mut cuts = Vec::new();
    let mut title = title.trim().to_owned();
    loop {
        let close = match title.chars().last() {
            Some(')') => '(',
            Some(']') => '[',
            _ => break,
        };
        let Some(open) = title.rfind(close) else {
            break;
        };
        let word = named_by(&title[open + 1..title.len() - 1]);
        if !all && word.is_none() {
            break;
        }
        let cut = title[..open].trim_end().trim_end_matches(['-', '–']).trim_end();
        if cut.is_empty() {
            break;
        }
        cuts.push(Cut {
            part: title[open..].to_owned(),
            word: if all { None } else { word },
        });
        title = cut.to_owned();
    }
    (title, cuts)
}

/// The lower-case words of `text` (an apostrophe kept: "collector's").
fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// `said` is the word `word`, or it with an ending: "editions",
/// "remastered", "reissued", "collector's" (so "ep" is not "deep").
fn same_word(said: &str, word: &str) -> bool {
    said == word
        || ["s", "es", "ed", "d", "'s"]
            .iter()
            .any(|end| said.strip_suffix(end) == Some(word))
}

/// A year's four digits, when it starts with them.
fn year_only(year: &str) -> String {
    let year = year.trim();
    if year.len() > 4 && year.as_bytes()[..4].iter().all(u8::is_ascii_digit) {
        year[..4].to_owned()
    } else {
        year.to_owned()
    }
}

/// `text` as a file's or folder's name: the letters Windows refuses made
/// others (`:` a dash), spaces run together, dots and spaces at the end
/// dropped; a device's name (CON, NUL…) followed by "_".
pub fn as_file_name(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            ':' => out.push_str(" -"),
            '/' | '\\' | '|' => out.push('-'),
            '"' => out.push('\''),
            '?' | '*' | '<' | '>' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let mut spaced = String::new();
    for word in out.split(' ').filter(|w| !w.is_empty()) {
        if !spaced.is_empty() {
            spaced.push(' ');
        }
        spaced.push_str(word);
    }
    let name = spaced.trim_end_matches(['.', ' ']).to_owned();
    let upper = name.split('.').next().unwrap_or("").to_uppercase();
    let device = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if device { format!("{name}_") } else { name }
}

/// The value most of `values` have, of those not empty (the first of the
/// most common, in the files' order).
fn most_common<'a>(values: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for v in values.map(str::trim).filter(|v| !v.is_empty()) {
        match counts.iter_mut().find(|(c, _)| *c == v) {
            Some((_, n)) => *n += 1,
            None => counts.push((v, 1)),
        }
    }
    let most = counts.iter().map(|(_, n)| *n).max()?;
    counts
        .into_iter()
        .find(|(_, n)| *n == most)
        .map(|(v, _)| v.to_owned())
}

/// What becomes of one file.
#[derive(Clone, Debug, Default)]
pub struct FilePlan {
    pub path: PathBuf,
    /// The fields changed: before, after.
    pub fields: Vec<(Key, String, String)>,
    /// The tag's items removed.
    pub removed: Vec<Item>,
    /// The album's cover put into its tag.
    pub cover: bool,
    /// Its cover now: its size and bytes.
    pub cover_now: Option<((u32, u32), usize)>,
    /// Its ID3v1 or APE tag removed.
    pub other_tags: bool,
    /// Its new name, in its folder.
    pub rename: Option<PathBuf>,
    /// Why its tag is not written (a format not written, a tag that
    /// cannot be read, the read-only attribute kept).
    pub skip: Option<String>,
    /// The fields it is to have and lacks.
    pub missing: Vec<Need>,
    /// The fields its name's template needs and it lacks: it keeps its
    /// name (no `01 - .mp3`).
    pub unnamed: Vec<&'static str>,
    /// The mask its name was read by, and the fields it filled.
    pub mask: Option<String>,
    pub from_name: Vec<Key>,
    pub read_only: bool,
}

impl FilePlan {
    /// Its tag is written.
    pub fn writes(&self) -> bool {
        self.skip.is_none()
            && (!self.fields.is_empty() || !self.removed.is_empty() || self.cover || self.other_tags)
    }

    /// Anything changes.
    pub fn changes(&self) -> bool {
        self.writes() || self.rename.is_some()
    }
}

/// The album's cover as the plan has it.
#[derive(Clone, Debug)]
pub struct CoverPlan {
    pub from: PathBuf,
    pub kind: Kind,
    /// The source's size, and the size the tags get.
    pub size: (u32, u32),
    pub made: Made,
}

/// What the preparation would do.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub folder: PathBuf,
    pub new_folder: Option<PathBuf>,
    pub files: Vec<FilePlan>,
    /// `folder.jpg` written with these bytes.
    pub folder_jpg: Option<Arc<Vec<u8>>>,
    /// `folder.jpg` now: its size and bytes.
    pub folder_jpg_now: Option<((u32, u32), usize)>,
    /// The cover the tags get.
    pub cover: Option<CoverPlan>,
    /// The other files deleted or moved.
    pub extras: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// What was cut from the album's title (the one most files have), and
    /// why, from the end.
    pub album_cuts: Vec<Cut>,
    /// The folder's template, when the folder is renamed.
    pub folder_template: String,
    /// The files' masks were tried; each used, and how many files it read.
    pub file_masks_tried: bool,
    pub file_masks: Vec<(String, usize)>,
    /// The folder's masks were tried; the one that fitted, and the fields
    /// it filled (in some file).
    pub folder_masks_tried: bool,
    pub folder_mask: Option<String>,
    pub from_folder: Vec<Key>,
    /// The fields given the album's own values (`ThisAlbum`).
    pub own: Vec<Key>,
    /// The album, its artist and year most files will have.
    pub album: String,
    pub album_artist: String,
    pub year: String,
    /// The genre most files have, before and after.
    pub genre: (String, String),
}

impl Plan {
    /// Nothing to do.
    pub fn is_empty(&self) -> bool {
        self.new_folder.is_none()
            && self.folder_jpg.is_none()
            && self.extras.is_empty()
            && self.files.iter().all(|f| !f.changes())
    }

    pub fn writes(&self) -> usize {
        self.files.iter().filter(|f| f.writes()).count()
    }

    pub fn renames(&self) -> usize {
        self.files.iter().filter(|f| f.rename.is_some()).count()
    }
}

/// A cover fits the profile: its sides within `cover_min`…`cover_max`,
/// its bytes no more than `cover_kb` kilobytes.
fn fits(profile: &Profile, size: Option<(u32, u32)>, bytes: usize) -> bool {
    size.is_some_and(|(w, h)| {
        w.min(h) >= profile.cover_min
            && w.max(h) <= profile.cover_max
            && bytes as u64 <= u64::from(profile.cover_kb) * 1024
    })
}

/// What `profile` would do to the album `source` read; `make` makes the
/// cover of a candidate (its index) ready for the tags at a side (the
/// window keeps what it made).
pub fn plan(
    source: &Source,
    profile: &Profile,
    this: &ThisAlbum,
    make: &mut dyn FnMut(usize, Option<u32>) -> Option<Made>,
) -> Plan {
    let genre = &this.genre;
    let n = source.files.len();
    let original: Vec<[String; MAIN]> = source
        .files
        .iter()
        .map(|f| f.tag.as_ref().map(|t| t.main.clone()).unwrap_or_default())
        .collect();
    let mut values = original.clone();
    let at = |k: Key| k as usize;
    let mut warnings = Vec::new();
    let unify = |u: Unify| profile.unify.contains(&u);
    let fill = |f: Fill| profile.fill.contains(&f);

    // The names read by the masks: each file by the first that fits it,
    // the folder by the first that fits; into the empty fields (or all).
    let masks = |choice: &MaskChoice, preset: &[String]| -> Vec<String> {
        match choice {
            MaskChoice::Preset if profile.from_names => preset.to_vec(),
            MaskChoice::Own(m) => vec![m.clone()],
            _ => Vec::new(),
        }
        .into_iter()
        .filter(|m| !m.trim().is_empty())
        .collect()
    };
    let file_masks = masks(&this.file_mask, &profile.file_masks);
    let folder_masks = masks(&this.folder_mask, &profile.folder_masks);
    let take = |slot: &mut String, value: &str| {
        let value = value.trim();
        let free = slot.trim().is_empty() || profile.names_over_tags;
        if value.is_empty() || !free || slot.trim() == value {
            return false;
        }
        *slot = value.to_owned();
        true
    };
    let mut read_by: Vec<Option<String>> = vec![None; n];
    let mut from_name: Vec<Vec<Key>> = vec![Vec::new(); n];
    for (i, f) in source.files.iter().enumerate() {
        for mask in &file_masks {
            let name = name_for_mask(mask, &f.path, true, profile.names_underscores);
            let Some(found) = read_name(mask, &name) else {
                continue;
            };
            for (field, value) in found {
                if let Some(k) = mask_key(&field, false)
                    && take(&mut values[i][at(k)], &value)
                {
                    from_name[i].push(k);
                }
            }
            read_by[i] = Some(mask.clone());
            break;
        }
    }
    let mut folder_mask = None;
    let mut from_folder: Vec<Key> = Vec::new();
    if source.one_place && n > 0 {
        for mask in &folder_masks {
            let name = name_for_mask(mask, &source.folder, false, profile.names_underscores);
            let Some(found) = read_name(mask, &name) else {
                continue;
            };
            for (field, value) in found {
                let Some(k) = mask_key(&field, true) else {
                    continue;
                };
                // A field of the album the tags know (or the album artist,
                // known from the artists) is not taken from the folder:
                // "Best of - Vol 2" is no artist's.
                let empty = |k: Key| values.iter().all(|v| v[at(k)].trim().is_empty());
                let known = !empty(k) || (k == Key::AlbumArtist && !empty(Key::Artist));
                if known && !profile.names_over_tags {
                    continue;
                }
                for v in &mut values {
                    if take(&mut v[at(k)], &value) && !from_folder.contains(&k) {
                        from_folder.push(k);
                    }
                }
            }
            folder_mask = Some(mask.clone());
            break;
        }
    }
    let mut masks_used: Vec<(String, usize)> = Vec::new();
    for m in read_by.iter().flatten() {
        match masks_used.iter_mut().find(|(u, _)| u == m) {
            Some((_, count)) => *count += 1,
            None => masks_used.push((m.clone(), 1)),
        }
    }

    if profile.album_title {
        for v in &mut values {
            v[at(Key::Album)] =
                without_edition(&v[at(Key::Album)], &profile.editions, profile.album_brackets);
        }
    }
    if profile.year_only {
        for v in &mut values {
            v[at(Key::Year)] = year_only(&v[at(Key::Year)]);
        }
    }
    // This album's own values, as typed.
    let own: Vec<Key> = [Key::Album, Key::AlbumArtist, Key::Year]
        .into_iter()
        .filter(|&k| this.own(k).is_some())
        .collect();
    for &k in &own {
        let value = this.own(k).unwrap_or_default().to_owned();
        values.iter_mut().for_each(|v| v[at(k)] = value.clone());
    }
    let common = |values: &[[String; MAIN]], k: Key| {
        most_common(values.iter().map(|v| v[at(k)].as_str()))
    };
    if n > 0 {
        if unify(Unify::Album)
            && let Some(album) = common(&values, Key::Album)
        {
            values.iter_mut().for_each(|v| v[at(Key::Album)] = album.clone());
        }
        // The album artist: the most common one, else from the artists.
        let main_artists: HashSet<String> = values
            .iter()
            .map(|v| super::main_artist(&v[at(Key::Artist)]))
            .filter(|a| !a.is_empty())
            .collect();
        let from_artists = || {
            let mut counts: HashMap<&String, usize> = HashMap::new();
            for v in &values {
                *counts.entry(&v[at(Key::Artist)]).or_default() += 1;
            }
            let most = counts.values().copied().max().unwrap_or(0);
            if main_artists.len() >= 3 && most * 2 <= n {
                Some(super::VARIOUS_ARTISTS.to_owned())
            } else {
                common(&values, Key::Artist)
            }
        };
        let album_artist = common(&values, Key::AlbumArtist)
            .or_else(|| fill(Fill::AlbumArtist).then(from_artists).flatten());
        if let Some(aa) = &album_artist {
            for v in &mut values {
                if unify(Unify::AlbumArtist)
                    || (fill(Fill::AlbumArtist) && v[at(Key::AlbumArtist)].trim().is_empty())
                {
                    v[at(Key::AlbumArtist)] = aa.clone();
                }
            }
        }
        let various = album_artist
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case(super::VARIOUS_ARTISTS));
        for (v, f) in values.iter_mut().zip(&source.files) {
            if fill(Fill::Artist)
                && v[at(Key::Artist)].trim().is_empty()
                && !various
                && !v[at(Key::AlbumArtist)].trim().is_empty()
            {
                v[at(Key::Artist)] = v[at(Key::AlbumArtist)].clone();
            }
            if fill(Fill::Title) && v[at(Key::Title)].trim().is_empty() {
                v[at(Key::Title)] = title_from_name(&f.path);
            }
        }
        // The genre: this album's own, as it is, or as the preset says
        // (the artist's other albums' first, the list cut and spelt).
        match genre {
            GenreChoice::Own(g) => {
                values.iter_mut().for_each(|v| v[at(Key::Genre)] = g.trim().to_owned());
            }
            GenreChoice::Keep => {}
            GenreChoice::Preset => {
                for v in &mut values {
                    let artist = match v[at(Key::AlbumArtist)].trim() {
                        "" => v[at(Key::Artist)].trim().to_lowercase(),
                        aa => aa.to_lowercase(),
                    };
                    let theirs = profile
                        .genre_from_artist
                        .then(|| source.known.artists.get(&artist))
                        .flatten();
                    let text = theirs.unwrap_or(&v[at(Key::Genre)]).clone();
                    v[at(Key::Genre)] = genres_of(&text, profile, &source.known);
                }
            }
        }
        for (u, k) in [(Unify::Year, Key::Year), (Unify::Genre, Key::Genre)] {
            if k == Key::Genre && *genre == GenreChoice::Keep {
                continue;
            }
            if unify(u)
                && let Some(value) = common(&values, k)
            {
                for v in &mut values {
                    // A compilation's tracks keep their own years.
                    if !various || k != Key::Year || v[at(k)].trim().is_empty() {
                        v[at(k)] = value.clone();
                    }
                }
            }
        }
    }

    // The discs: several numbers, or several folders.
    let folders: HashSet<String> = source
        .files
        .iter()
        .filter_map(|f| f.path.parent())
        .map(folded)
        .collect();
    let disc_numbers: Vec<Option<u32>> = values
        .iter()
        .zip(&source.files)
        .map(|(v, f)| {
            number_of(&v[at(Key::Disc)])
                .filter(|&d| d > 0)
                .or_else(|| (folders.len() > 1).then(|| disc_of_folder(&f.path)).flatten())
        })
        .collect();
    let discs: HashSet<u32> = disc_numbers.iter().flatten().copied().collect();
    let several_discs = discs.len() > 1 || folders.len() > 1;
    let disc_count = discs.iter().copied().max().unwrap_or(1).max(discs.len() as u32);
    let numbers: Vec<Option<u32>> = values
        .iter()
        .zip(&source.files)
        .map(|(v, f)| number_of(&v[at(Key::Track)]).or_else(|| leading_number(&stem(&f.path))))
        .collect();
    let most = numbers.iter().flatten().copied().max().unwrap_or(0).max(n as u32);
    let width = most.to_string().len().max(2);
    // A disc's tracks: its files or its largest number.
    let total_of = |disc: Option<u32>| {
        let on: Vec<usize> = (0..n).filter(|&i| disc_numbers[i] == disc).collect();
        let largest = on.iter().filter_map(|&i| numbers[i]).max().unwrap_or(0);
        largest.max(on.len() as u32)
    };
    for i in 0..n {
        if let Some(number) = numbers[i]
            && let Some(t) = profile.track.format(number, total_of(disc_numbers[i]), width)
        {
            values[i][at(Key::Track)] = t;
        }
        let disc = disc_numbers[i];
        let written = match profile.disc {
            DiscFormat::Keep => None,
            DiscFormat::Remove => Some(String::new()),
            DiscFormat::Several if several_discs => Some(disc.map(|d| d.to_string()).unwrap_or_default()),
            DiscFormat::Several => Some(String::new()),
            DiscFormat::Plain => Some(disc.unwrap_or(1).to_string()),
            DiscFormat::WithTotal => Some(format!("{}/{disc_count}", disc.unwrap_or(1))),
        };
        if let Some(d) = written {
            values[i][at(Key::Disc)] = d;
        }
    }

    // The cover: its source, then what each tag gets.
    let candidate = {
        let order: [Kind; 3] = match profile.cover_from {
            CoverFrom::Folder => [Kind::FolderJpg, Kind::Picture, Kind::Tag],
            CoverFrom::Tags => [Kind::Tag, Kind::FolderJpg, Kind::Picture],
        };
        let of = |kind: Kind| {
            source
                .candidates
                .iter()
                .position(|c| c.kind == kind && c.size.is_some())
        };
        // Tags whose cover fits keep it: it is the album's then.
        let fitting_tag = (profile.tag_cover == TagCover::Fitting)
            .then(|| of(Kind::Tag))
            .flatten()
            .filter(|&i| {
                let c = &source.candidates[i];
                fits(profile, c.size, c.bytes.len())
            });
        fitting_tag.or_else(|| order.into_iter().find_map(of))
    };
    let touches_cover = profile.tag_cover != TagCover::Keep || profile.folder_jpg != FolderJpg::Keep;
    let cover = candidate
        .filter(|_| touches_cover)
        .and_then(|i| {
            let c = &source.candidates[i];
            let made = make(i, profile.cover_side.pixels())?;
            Some(CoverPlan {
                from: c.from.clone(),
                kind: c.kind,
                size: c.size.unwrap_or_default(),
                made,
            })
        });
    if touches_cover && cover.is_none() {
        warnings.push(
            tr!(
                "No cover was found: not folder.jpg, nor a picture, nor a tag's.",
                "Обложка не найдена: нет ни folder.jpg, ни картинки в папке, ни обложки в тегах."
            )
            .to_owned(),
        );
    }

    let various = values.iter().any(|v| {
        let aa = super::main_artist(&v[at(Key::AlbumArtist)]);
        !aa.is_empty() && super::main_artist(&v[at(Key::Artist)]) != aa
    });
    let mut files: Vec<FilePlan> = Vec::with_capacity(n);
    for (i, f) in source.files.iter().enumerate() {
        let mut plan = FilePlan {
            path: f.path.clone(),
            read_only: f.read_only,
            mask: read_by[i].clone(),
            from_name: from_name[i].clone(),
            ..FilePlan::default()
        };
        match &f.tag {
            Err(e) => plan.skip = Some(e.clone()),
            Ok(_) if !f.writable => {
                plan.skip = Some(
                    tr!(
                        "tags of this format are not written",
                        "теги этого формата не записываются"
                    )
                    .to_owned(),
                )
            }
            Ok(_) if f.read_only && !profile.clear_read_only => {
                plan.skip = Some(edit::read_only())
            }
            Ok(_) => {}
        }
        if let Ok(tag) = &f.tag {
            for k in [
                Key::Title,
                Key::Artist,
                Key::Album,
                Key::AlbumArtist,
                Key::Year,
                Key::Genre,
                Key::Track,
                Key::Disc,
            ] {
                let (before, after) = (original[i][at(k)].trim(), values[i][at(k)].trim());
                if before != after {
                    plan.fields.push((k, before.to_owned(), after.to_owned()));
                }
            }
            if profile.clean {
                plan.removed = tag
                    .parts
                    .iter()
                    .filter(|i| !profile.keep.contains(&i.part))
                    .cloned()
                    .collect();
            }
            plan.cover_now = tag
                .cover
                .as_ref()
                .map(|c| (picture_size(c).unwrap_or_default(), c.len()));
            plan.cover = cover.as_ref().is_some_and(|c| {
                let now = tag.cover.as_deref();
                let differs = now != Some(c.made.data.as_slice());
                differs
                    && match profile.tag_cover {
                        TagCover::Keep => false,
                        TagCover::Missing => now.is_none(),
                        TagCover::Fitting => {
                            now.is_none_or(|b| !fits(profile, picture_size(b), b.len()))
                        }
                        TagCover::Replace => true,
                    }
            });
            plan.other_tags = profile.only_id3v2 && (tag.id3v1 || tag.ape);
            let lyrics = tag.parts.iter().any(|i| i.part == TagPart::Lyrics)
                && !(profile.clean && !profile.keep.contains(&TagPart::Lyrics));
            for need in &profile.required {
                let lacks = match need {
                    Need::Cover => !plan.cover && tag.cover.is_none(),
                    Need::Lyrics => !lyrics,
                    other => other
                        .key()
                        .is_some_and(|k| values[i][at(k)].trim().is_empty()),
                };
                if lacks {
                    plan.missing.push(*need);
                }
            }
        }
        if profile.rename_files {
            let template = if various {
                &profile.file_name_various
            } else {
                &profile.file_name
            };
            let template = if several_discs && folders.len() == 1 && !template.contains("%disc%") {
                format!("%disc%-{template}")
            } else {
                template.clone()
            };
            let v = &values[i];
            let track = number_of(&v[at(Key::Track)])
                .or(numbers[i])
                .map(|t| format!("{t:0width$}"))
                .unwrap_or_default();
            let (name, empty) = crate::title_format::render_checked(&template, &|field| {
                Some(match field {
                    "track" => track.clone(),
                    "title" => v[at(Key::Title)].clone(),
                    "artist" => v[at(Key::Artist)].clone(),
                    "album" => v[at(Key::Album)].clone(),
                    "albumartist" => v[at(Key::AlbumArtist)].clone(),
                    "year" => year_only(&v[at(Key::Year)]),
                    "genre" => v[at(Key::Genre)].clone(),
                    "disc" => disc_numbers[i].map(|d| d.to_string()).unwrap_or_default(),
                    "filename" => stem(&f.path),
                    _ => return None,
                })
            });
            let name = named(&name, profile.underscores);
            let extension = f
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            plan.unnamed = labels_of(&empty);
            if !name.is_empty() && plan.unnamed.is_empty() {
                let new = f.path.with_file_name(format!("{name}.{extension}"));
                if new.file_name() != f.path.file_name() {
                    plan.rename = Some(new);
                }
            }
        }
        files.push(plan);
    }

    // Two files of one name, or a name another file has.
    let mut taken: HashMap<String, usize> = HashMap::new();
    for f in &files {
        let to = f.rename.as_ref().unwrap_or(&f.path);
        *taken.entry(folded(to)).or_default() += 1;
    }
    let mine: HashSet<String> = files.iter().map(|f| folded(&f.path)).collect();
    let mut clashes = 0;
    for f in &mut files {
        if let Some(to) = &f.rename {
            let key = folded(to);
            let clash = taken.get(&key).copied().unwrap_or(0) > 1
                || (to.exists() && !mine.contains(&key));
            if clash {
                f.rename = None;
                clashes += 1;
            }
        }
    }
    if clashes > 0 {
        warnings.push(tr!(
            format!("{clashes} files would get a name another file has: they keep theirs."),
            format!("{clashes} файл(ов) получили бы имя, которое уже занято: их имена не меняются.")
        ));
    }
    let unnamed = files.iter().filter(|f| !f.unnamed.is_empty()).count();
    if unnamed > 0 {
        let mut lacking: Vec<&str> = Vec::new();
        for l in files.iter().flat_map(|f| &f.unnamed) {
            if !lacking.contains(l) {
                lacking.push(l);
            }
        }
        warnings.push(tr!(
            format!(
                "{unnamed} files keep their names: the template needs {}, and they lack it.",
                quoted(&lacking)
            ),
            format!(
                "{unnamed} файл(ов) не переименовываются: шаблону нужны поля {}, а они пустые.",
                quoted(&lacking)
            )
        ));
    }

    let mut new_folder = None;
    if profile.rename_folder && n > 0 {
        if !source.one_place {
            warnings.push(
                tr!(
                    "The files are in different folders: the folder is not renamed.",
                    "Файлы лежат в разных папках: папка не переименовывается."
                )
                .to_owned(),
            );
        } else if !source.foreign.is_empty() {
            warnings.push(tr!(
                format!(
                    "The folder holds other audio ({}): it is not renamed.",
                    source.foreign.len()
                ),
                format!(
                    "В папке есть другие аудиофайлы ({}): папка не переименовывается.",
                    source.foreign.len()
                )
            ));
        } else {
            let v = &values[0];
            let (name, empty) = crate::title_format::render_checked(&profile.folder_name, &|field| {
                Some(match field {
                    "album" => v[at(Key::Album)].clone(),
                    "albumartist" => v[at(Key::AlbumArtist)].clone(),
                    "artist" => v[at(Key::Artist)].clone(),
                    "year" => year_only(&v[at(Key::Year)]),
                    "genre" => v[at(Key::Genre)].clone(),
                    _ => return None,
                })
            });
            let name = named(&name, profile.underscores);
            let new = source.folder.with_file_name(&name);
            let lacking = labels_of(&empty);
            if !lacking.is_empty() {
                warnings.push(tr!(
                    format!(
                        "The folder is not renamed: its template needs {}, and the tags lack it.",
                        quoted(&lacking)
                    ),
                    format!(
                        "Папка не переименовывается: шаблону нужны поля {}, а в тегах они пустые.",
                        quoted(&lacking)
                    )
                ));
            } else if !name.is_empty() && new.file_name() != source.folder.file_name() {
                if new.exists() && folded(&new) != folded(&source.folder) {
                    warnings.push(tr!(
                        format!("A folder named \"{name}\" exists: the folder is not renamed."),
                        format!("Папка «{name}» уже есть: папка не переименовывается.")
                    ));
                } else {
                    new_folder = Some(new);
                }
            }
        }
    }

    let mut folder_jpg = None;
    if let Some(c) = &cover
        && source.one_place
    {
        let wanted = match profile.folder_jpg {
            FolderJpg::Keep => None,
            FolderJpg::Same => Some(c.made.data.clone()),
            FolderJpg::Original => candidate.map(|i| source.candidates[i].bytes.clone()),
        };
        if let Some(wanted) = wanted {
            let same = source
                .folder_jpg
                .as_ref()
                .is_some_and(|(_, bytes)| bytes.as_slice() == wanted.as_slice());
            if !same {
                folder_jpg = Some(wanted);
            }
        }
    }

    let extras = match profile.extras {
        Extras::Keep => Vec::new(),
        Extras::Delete => source.extras.clone(),
        Extras::Folder => {
            let into = folded(&source.folder.join(&profile.extras_folder));
            source
                .extras
                .iter()
                .filter(|p| folded(p) != into)
                .cloned()
                .collect()
        }
    };

    let skipped = files.iter().filter(|f| f.skip.is_some()).count();
    if skipped > 0 {
        warnings.push(tr!(
            format!("{skipped} files' tags are not written (the reason is by each)."),
            format!("Теги {skipped} файл(ов) не записываются (причина указана у файла).")
        ));
    }
    for need in &profile.required {
        let lacking = files.iter().filter(|f| f.missing.contains(need)).count();
        if lacking > 0 {
            warnings.push(tr!(
                format!("No {} in {lacking} files.", need.label().to_lowercase()),
                format!("Нет поля «{}» в {lacking} файл(ах).", need.label())
            ));
        }
    }
    let longest = files
        .iter()
        .map(|f| {
            let path = f.rename.as_ref().unwrap_or(&f.path);
            match &new_folder {
                Some(new) => path
                    .strip_prefix(&source.folder)
                    .map_or(path.clone(), |rest| new.join(rest)),
                None => path.clone(),
            }
        })
        .map(|p| p.as_os_str().len())
        .max()
        .unwrap_or(0);
    if longest > 259 {
        warnings.push(tr!(
            format!("A path would be {longest} characters long, more than Windows' 259."),
            format!("Путь получился бы длиной {longest} символов, больше 259 по правилам Windows.")
        ));
    }

    Plan {
        folder: source.folder.clone(),
        new_folder,
        files,
        folder_jpg,
        genre: (
            common(&original, Key::Genre).unwrap_or_default(),
            common(&values, Key::Genre).unwrap_or_default(),
        ),
        folder_jpg_now: source
            .folder_jpg
            .as_ref()
            .map(|(_, b)| (picture_size(b).unwrap_or_default(), b.len())),
        cover,
        extras,
        warnings,
        album_cuts: if profile.album_title && !own.contains(&Key::Album) {
            common(&original, Key::Album)
                .map(|a| edition_cut(&a, &profile.editions, profile.album_brackets).1)
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        folder_template: profile.folder_name.clone(),
        file_masks_tried: !file_masks.is_empty(),
        file_masks: masks_used,
        folder_masks_tried: !folder_masks.is_empty() && source.one_place,
        folder_mask,
        from_folder,
        album: common(&values, Key::Album).unwrap_or_default(),
        album_artist: common(&values, Key::AlbumArtist).unwrap_or_default(),
        year: common(&values, Key::Year).unwrap_or_default(),
        own,
    }
}

/// The names people read of a template's fields.
fn labels_of(fields: &[String]) -> Vec<&'static str> {
    let mut out = Vec::new();
    for f in fields {
        let label = crate::title_format::field_label(f).unwrap_or("?");
        if !out.contains(&label) {
            out.push(label);
        }
    }
    out
}

/// «A», «B».
pub fn quoted(labels: &[&str]) -> String {
    let quoted: Vec<String> = labels
        .iter()
        .map(|l| tr!(format!("\"{l}\""), format!("«{l}»")))
        .collect();
    quoted.join(", ")
}

/// A name for a file or a folder (`as_file_name`), its spaces made
/// underscores when asked.
fn named(text: &str, underscores: bool) -> String {
    let name = as_file_name(text);
    if underscores { name.replace(' ', "_") } else { name }
}

/// What the preparation did.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub written: usize,
    /// The album's files: where they were, where they are.
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub folder: Option<(PathBuf, PathBuf)>,
    pub folder_jpg: bool,
    pub extras: usize,
    pub errors: Vec<(PathBuf, String)>,
    pub journal: Option<PathBuf>,
}

/// Does `plan` (on a thread), counting the steps done into `done` (the
/// files written, then the ones renamed). `releaser` lets the player let go
/// of the files meanwhile.
pub fn apply(
    plan: &Plan,
    profile: &Profile,
    how: WriteTags,
    releaser: Option<&crate::audio::Releaser>,
    done: &AtomicUsize,
) -> Outcome {
    let mut outcome = Outcome::default();
    let all: Vec<PathBuf> = plan.files.iter().map(|f| f.path.clone()).collect();
    let held = releaser.and_then(|r| r.hold(all));
    let how = if profile.only_id3v2 {
        WriteTags {
            id3v1: crate::settings::Id3v1Tag::Remove,
            ape: crate::settings::ApeTag::Remove,
            ..how
        }
    } else {
        how
    };
    let error = |outcome: &mut Outcome, path: &Path, e: String| {
        log::warn!("prepare: {}: {e}", path.display());
        outcome.errors.push((path.to_path_buf(), e));
    };

    // The tags.
    for f in &plan.files {
        if f.writes() {
            if f.read_only && profile.clear_read_only {
                writable(&f.path);
            }
            let mut changes = edit::Changes::default();
            for (k, _, after) in &f.fields {
                changes.main[*k as usize] = Some(after.clone());
            }
            if profile.clean {
                changes.keep = Some(profile.keep.clone());
            }
            if f.cover
                && let Some(cover) = &plan.cover
            {
                changes.cover = Some(edit::CoverChange::Set(cover.made.data.clone()));
            }
            match crate::guard::catch(&f.path, || edit::write(&f.path, &changes, how)) {
                Some(Ok(())) => outcome.written += 1,
                Some(Err(e)) => error(&mut outcome, &f.path, e),
                None => error(&mut outcome, &f.path, "panic".into()),
            }
        }
        done.fetch_add(1, Ordering::Relaxed);
    }

    // The cover beside them.
    if let Some(bytes) = &plan.folder_jpg {
        let (path, exists) = crate::covers::folder_jpg(&plan.folder);
        if exists {
            writable(&path);
        }
        match crate::covers::save_folder_jpg(&plan.folder, bytes) {
            Ok(_) => outcome.folder_jpg = true,
            Err(e) => error(&mut outcome, &path, e),
        }
    }

    // The other files.
    if !plan.extras.is_empty() {
        match profile.extras {
            Extras::Keep => {}
            Extras::Delete => match crate::win::recycle(&plan.extras) {
                Ok(()) => outcome.extras = plan.extras.len(),
                Err(e) => error(&mut outcome, &plan.folder, e),
            },
            Extras::Folder => {
                let into = plan.folder.join(as_file_name(&profile.extras_folder));
                if let Err(e) = std::fs::create_dir_all(&into) {
                    error(&mut outcome, &into, e.to_string());
                } else {
                    for path in &plan.extras {
                        let Some(name) = path.file_name() else { continue };
                        match std::fs::rename(path, into.join(name)) {
                            Ok(()) => outcome.extras += 1,
                            Err(e) => error(&mut outcome, path, e.to_string()),
                        }
                    }
                }
            }
        }
    }

    // The files' names: each aside first, so names may change places and
    // a name may change only its case.
    let mut moved: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut aside = Vec::new();
    for f in &plan.files {
        let Some(to) = &f.rename else { continue };
        let tmp = f.path.with_file_name(format!(
            "{}.metalamp-tmp",
            f.path.file_name().unwrap_or_default().to_string_lossy()
        ));
        match std::fs::rename(&f.path, &tmp) {
            Ok(()) => aside.push((f.path.clone(), tmp, to.clone())),
            Err(e) => error(&mut outcome, &f.path, in_use(e)),
        }
    }
    for (from, tmp, to) in aside {
        match std::fs::rename(&tmp, &to) {
            Ok(()) => {
                moved.insert(from, to);
            }
            Err(e) => {
                let _ = std::fs::rename(&tmp, &from);
                error(&mut outcome, &from, e.to_string());
            }
        }
        done.fetch_add(1, Ordering::Relaxed);
    }

    // The folder's.
    let mut folder = None;
    if let Some(new) = &plan.new_folder {
        let renamed = if folded(new) == folded(&plan.folder) {
            let tmp = plan.folder.with_file_name(format!(
                "{}.metalamp-tmp",
                plan.folder.file_name().unwrap_or_default().to_string_lossy()
            ));
            std::fs::rename(&plan.folder, &tmp).and_then(|()| {
                std::fs::rename(&tmp, new).inspect_err(|_| {
                    let _ = std::fs::rename(&tmp, &plan.folder);
                })
            })
        } else {
            std::fs::rename(&plan.folder, new)
        };
        match renamed {
            Ok(()) => folder = Some((plan.folder.clone(), new.clone())),
            Err(e) => error(
                &mut outcome,
                &plan.folder,
                tr!(
                    format!("the folder is not renamed: {} (is it open in Explorer?)", in_use(e)),
                    format!("папка не переименована: {} (не открыта ли она в Проводнике?)", in_use(e))
                ),
            ),
        }
    }
    drop(held);

    for f in &plan.files {
        let mut to = moved.get(&f.path).cloned().unwrap_or_else(|| f.path.clone());
        if let Some((old, new)) = &folder
            && let Ok(rest) = to.strip_prefix(old)
        {
            to = new.join(rest);
        }
        if to != f.path {
            outcome.moved.push((f.path.clone(), to));
        }
    }
    outcome.folder = folder;
    outcome.journal = journal(plan, &outcome);
    outcome
}

/// The read-only attribute taken off.
fn writable(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut permissions = meta.permissions();
        if permissions.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
}

fn in_use(e: std::io::Error) -> String {
    if e.raw_os_error() == Some(32) {
        tr!("in use by another program", "занят другой программой").to_owned()
    } else {
        e.to_string()
    }
}

/// What was done, written as `prepare\<seconds>.json` next to the
/// settings: the names before and after, the fields before and after,
/// what was removed (nothing is undone from it yet).
fn journal(plan: &Plan, outcome: &Outcome) -> Option<PathBuf> {
    // Tests leave the user's settings folder alone.
    if cfg!(test) {
        return None;
    }
    let dir = crate::settings::data_dir()?.join("prepare");
    std::fs::create_dir_all(&dir).ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let moved: HashMap<&PathBuf, &PathBuf> = outcome.moved.iter().map(|(a, b)| (a, b)).collect();
    let files: Vec<serde_json::Value> = plan
        .files
        .iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path,
                "now": moved.get(&f.path),
                "fields": f.fields.iter().map(|(k, before, after)| {
                    serde_json::json!([format!("{k:?}"), before, after])
                }).collect::<Vec<_>>(),
                "removed": f.removed.iter().map(|i| serde_json::json!([i.code, i.value])).collect::<Vec<_>>(),
                "cover": f.cover,
            })
        })
        .collect();
    let content = serde_json::json!({
        "version": 1,
        "time": now,
        "folder": plan.folder,
        "new_folder": outcome.folder.as_ref().map(|(_, new)| new),
        "files": files,
        "extras": plan.extras,
        "errors": outcome.errors.iter().map(|(p, e)| serde_json::json!([p, e])).collect::<Vec<_>>(),
    });
    let path = dir.join(format!("{now}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&content).ok()?).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editions_cut() {
        let cut = |t: &str| without_edition(t, EDITIONS, false);
        assert_eq!(cut("Amaranthe [Deluxe]"), "Amaranthe");
        assert_eq!(cut("Album (Limited Edition) [Remastered 2015]"), "Album");
        assert_eq!(cut("Album - (Deluxe Edition)"), "Album");
        assert_eq!(cut("Live (In Moscow)"), "Live (In Moscow)");
        assert_eq!(cut("(Deluxe)"), "(Deluxe)");
        assert_eq!(cut("Album (Deluxe"), "Album (Deluxe");
        assert_eq!(
            cut("Melodies of Green and Blue (Acoustic Version) (EP)"),
            "Melodies of Green and Blue"
        );
        assert_eq!(cut("Album (Collector's Editions)"), "Album");
        assert_eq!(cut("Album (Japanese Reissued)"), "Album");
        // Whole words: "ep" is not in "Deep".
        assert_eq!(cut("Album (Deep Cuts)"), "Album (Deep Cuts)");
        assert_eq!(cut("Album (Single Mix) (In Moscow)"), "Album (Single Mix) (In Moscow)");
        let all = |t: &str| without_edition(t, EDITIONS, true);
        assert_eq!(all("Live (In Moscow) [Bootleg]"), "Live");
        let (_, cuts) = edition_cut("Melodies (Acoustic Version) (EP)", EDITIONS, false);
        let why: Vec<(&str, Option<&str>)> =
            cuts.iter().map(|c| (c.part.as_str(), c.word.as_deref())).collect();
        assert_eq!(why, [("(EP)", Some("ep")), ("(Acoustic Version)", Some("version"))]);
    }

    #[test]
    fn names_windows_takes() {
        assert_eq!(as_file_name("Act I: The Lake"), "Act I - The Lake");
        assert_eq!(as_file_name("AC/DC?  Live*"), "AC-DC Live");
        assert_eq!(as_file_name("Say \"Hi\"..."), "Say 'Hi'");
        assert_eq!(as_file_name("con"), "con_");
        assert_eq!(as_file_name("Console"), "Console");
    }

    #[test]
    fn numbers_and_names() {
        assert_eq!(number_of("03/12"), Some(3));
        assert_eq!(leading_number("01 - Song"), Some(1));
        assert_eq!(title_from_name(Path::new("01 - Leave Everything Behind.mp3")), "Leave Everything Behind");
        assert_eq!(title_from_name(Path::new("Song.mp3")), "Song");
        assert_eq!(year_only("2011-05-01"), "2011");
        assert_eq!(disc_of_folder(Path::new(r"D:\A\CD2\01.mp3")), Some(2));
    }

    fn no_cover(_: usize, _: Option<u32>) -> Option<Made> {
        None
    }

    /// The covers made as the window makes them.
    fn made(source: &Source) -> impl FnMut(usize, Option<u32>) -> Option<Made> + '_ {
        |i, side| make_cover(&source.candidates[i].bytes, side)
    }

    #[test]
    fn track_formats() {
        let f = |t: TrackFormat, n, total, width| t.format(n, total, width);
        assert_eq!(f(TrackFormat::Plain, 3, 12, 2).as_deref(), Some("3"));
        assert_eq!(f(TrackFormat::Two, 3, 12, 3).as_deref(), Some("03"));
        assert_eq!(f(TrackFormat::Three, 3, 12, 2).as_deref(), Some("003"));
        assert_eq!(f(TrackFormat::Auto, 3, 120, 3).as_deref(), Some("003"));
        assert_eq!(f(TrackFormat::WithTotal, 3, 12, 2).as_deref(), Some("03/12"));
        assert_eq!(f(TrackFormat::Keep, 3, 12, 2), None);
    }

    #[test]
    fn genres_cut_and_spelt() {
        let known = Known {
            genres: vec!["Modern Metal".into(), "Synthwave".into()],
            ..Default::default()
        };
        let profile = Profile::default();
        let g = |t: &str, p: &Profile| genres_of(t, p, &known);
        assert_eq!(g("Dark Synth, Shredwave, Synthwave", &profile), "Dark Synth");
        assert_eq!(g("modern metal", &profile), "Modern Metal");
        let all = Profile {
            genre_count: 0,
            ..Profile::default()
        };
        assert_eq!(g("synthwave/Retrowave; SYNTHWAVE", &all), "Synthwave; Retrowave");
        let names = &first_presets()[1].profile;
        assert_eq!(g("Dark Synth, Shredwave", names), "Dark Synth, Shredwave");
    }

    /// The artist's other albums give the genre; one's own wins over it,
    /// and "as it is" keeps the files' own.
    #[test]
    fn the_artists_genre() {
        let main = [
            (Key::Title, "Overture"),
            (Key::Artist, "Meteor"),
            (Key::Album, "Phantom Zone"),
            (Key::Genre, "Dark Synth, Shredwave, Synthwave, Retrowave, Outrun"),
            (Key::Track, "01"),
        ];
        let mut source = Source {
            folder: PathBuf::from(r"Z:\Meteor\(2022) Phantom Zone"),
            one_place: true,
            files: vec![file(r"Z:\Meteor\(2022) Phantom Zone\01 - Overture.mp3", &main, &[])],
            ..Default::default()
        };
        let genre = |source: &Source, choice: &GenreChoice| {
            let this = ThisAlbum {
                genre: choice.clone(),
                ..ThisAlbum::default()
            };
            plan(source, &Profile::default(), &this, &mut no_cover).genre.1
        };
        assert_eq!(genre(&source, &GenreChoice::Preset), "Dark Synth");
        source.known.artists.insert("meteor".into(), "Synthwave".into());
        assert_eq!(genre(&source, &GenreChoice::Preset), "Synthwave");
        assert_eq!(genre(&source, &GenreChoice::Own("Darksynth".into())), "Darksynth");
        assert_eq!(
            genre(&source, &GenreChoice::Keep),
            "Dark Synth, Shredwave, Synthwave, Retrowave, Outrun"
        );
    }

    /// A tag's cover that fits stays, and becomes the album's.
    #[test]
    fn a_fitting_cover_stays() {
        let profile = Profile::default();
        assert!(fits(&profile, Some((600, 600)), 80_000));
        assert!(!fits(&profile, Some((300, 300)), 20_000));
        assert!(!fits(&profile, Some((3000, 3000)), 900_000));
        assert!(!fits(&profile, Some((800, 800)), 2_000_000));
    }

    fn file(path: &str, main: &[(Key, &str)], parts: &[TagPart]) -> FileRead {
        let mut values: [String; MAIN] = Default::default();
        for (k, v) in main {
            values[*k as usize] = (*v).to_owned();
        }
        FileRead {
            path: PathBuf::from(path),
            tag: Ok(edit::Inspected {
                main: values,
                parts: parts
                    .iter()
                    .map(|p| Item {
                        part: *p,
                        code: format!("{p:?}"),
                        key: None,
                        value: String::new(),
                    })
                    .collect(),
                ..Default::default()
            }),
            read_only: false,
            writable: true,
        }
    }

    /// An album as downloaded comes out as its owner keeps them.
    #[test]
    fn an_album_planned() {
        let tags = |n: &str, title: &str| {
            [
                (Key::Title, title.to_owned()),
                (Key::Artist, "Amaranthe".to_owned()),
                (Key::Album, "Amaranthe [Deluxe Edition]".to_owned()),
                (Key::Year, "2011-03-30".to_owned()),
                (Key::Genre, if n == "1/2" { "Metal" } else { "Modern Metal" }.to_owned()),
                (Key::Track, n.to_owned()),
                (Key::Disc, "1/1".to_owned()),
            ]
        };
        let a = tags("1/2", "Leave Everything Behind");
        let b = tags("2/2", "Hunger: Part 1");
        fn borrow(t: &[(Key, String)]) -> Vec<(Key, &str)> {
            t.iter().map(|(k, v)| (*k, v.as_str())).collect()
        }
        let source = Source {
            folder: PathBuf::from(r"Z:\Music\Amaranthe\Amaranthe - 2011 [Deluxe]"),
            one_place: true,
            files: vec![
                file(
                    r"Z:\Music\Amaranthe\Amaranthe - 2011 [Deluxe]\1-amaranthe-leave.mp3",
                    &borrow(&a),
                    &[TagPart::Comment, TagPart::ReplayGain],
                ),
                file(
                    r"Z:\Music\Amaranthe\Amaranthe - 2011 [Deluxe]\2-amaranthe-hunger.mp3",
                    &borrow(&b),
                    &[TagPart::Encoder],
                ),
            ],
            ..Default::default()
        };
        let plan = plan(&source, &Profile::default(), &ThisAlbum::default(), &mut no_cover);
        let first = &plan.files[0];
        let field = |f: &FilePlan, k: Key| f.fields.iter().find(|(key, ..)| *key == k).map(|(_, _, a)| a.clone());
        assert_eq!(field(first, Key::Album).as_deref(), Some("Amaranthe"));
        assert_eq!(field(first, Key::AlbumArtist).as_deref(), Some("Amaranthe"));
        assert_eq!(field(first, Key::Track).as_deref(), Some("01"));
        assert_eq!(field(first, Key::Disc).as_deref(), Some(""));
        assert_eq!(field(first, Key::Year).as_deref(), Some("2011"));
        assert_eq!(field(first, Key::Genre), None, "the first's genre is one of the most common");
        assert_eq!(field(&plan.files[1], Key::Genre).as_deref(), Some("Metal"));
        assert_eq!(first.removed.iter().map(|i| i.part).collect::<Vec<_>>(), [TagPart::Comment]);
        assert_eq!(
            first.rename.as_deref(),
            Some(Path::new(r"Z:\Music\Amaranthe\Amaranthe - 2011 [Deluxe]\01 - Leave Everything Behind.mp3"))
        );
        assert_eq!(
            plan.files[1].rename.as_deref(),
            Some(Path::new(r"Z:\Music\Amaranthe\Amaranthe - 2011 [Deluxe]\02 - Hunger - Part 1.mp3"))
        );
        assert_eq!(
            plan.new_folder.as_deref(),
            Some(Path::new(r"Z:\Music\Amaranthe\(2011) Amaranthe"))
        );
        // Prepared, nothing more to do (but the cover, which none has).
        let prepared = Source {
            folder: PathBuf::from(r"Z:\Music\Amaranthe\(2011) Amaranthe"),
            files: plan
                .files
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let mut main = source.files[i].tag.as_ref().unwrap().main.clone();
                    for (k, _, after) in &f.fields {
                        main[*k as usize] = after.clone();
                    }
                    FileRead {
                        path: PathBuf::from(r"Z:\Music\Amaranthe\(2011) Amaranthe")
                            .join(f.rename.as_ref().unwrap().file_name().unwrap()),
                        tag: Ok(edit::Inspected {
                            main,
                            ..Default::default()
                        }),
                        read_only: false,
                        writable: true,
                    }
                })
                .collect(),
            ..source.clone()
        };
        let again = super::plan(&prepared, &Profile::default(), &ThisAlbum::default(), &mut no_cover);
        assert!(again.is_empty(), "{again:#?}");
    }

    #[test]
    fn names_read_by_masks() {
        let read = |mask: &str, name: &str| {
            read_name(mask, name).map(|f| f.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>())
        };
        assert_eq!(
            read("%track% - %title%", "01 - Hunger - Part 1").unwrap(),
            ["track=01", "title=Hunger - Part 1"]
        );
        assert_eq!(read("%track% - %title%", "01. Song"), None);
        assert_eq!(read("%track%. %title%", "01. Song").unwrap(), ["track=01", "title=Song"]);
        // A year is four digits: "Amaranthe - 2011" is no "%year% - %album%".
        assert_eq!(read("%year% - %album%", "Amaranthe - 2011"), None);
        assert_eq!(
            read("%artist% - %year% - %album%", "Ария - 1998 - Генератор зла").unwrap(),
            ["artist=Ария", "year=1998", "album=Генератор зла"]
        );
        assert_eq!(read("%skip% %track% %title%", "CD1 03 Song").unwrap(), ["skip=CD1", "track=03", "title=Song"]);
        assert_eq!(read("no fields", "no fields"), None);
        assert_eq!(
            name_for_mask(r"%artist%\%year% - %album%", Path::new(r"G:\T\Band\2021 - A_B"), false, true),
            r"Band\2021 - A B"
        );
        assert_eq!(name_for_mask("%track% - %title%", Path::new(r"G:\T\01 - A.b.mp3"), true, false), "01 - A.b");
    }

    #[test]
    fn an_album_without_tags() {
        let folder = r"G:\Torrents\Temperance\2021 - Melodies of Green and Blue (Acoustic Version) (EP)";
        let source = Source {
            folder: PathBuf::from(folder),
            one_place: true,
            files: vec![
                file(&format!(r"{folder}\01 - Pure Life Unfolds.mp3"), &[], &[]),
                file(&format!(r"{folder}\02_-_Start_Another_Round.mp3"), &[], &[]),
            ],
            ..Default::default()
        };
        let field = |f: &FilePlan, k: Key| f.fields.iter().find(|(key, ..)| *key == k).map(|(_, _, a)| a.clone());
        let plan = plan(&source, &Profile::default(), &ThisAlbum::default(), &mut no_cover);
        let second = &plan.files[1];
        assert_eq!(field(second, Key::Title).as_deref(), Some("Start Another Round"));
        assert_eq!(field(second, Key::Track).as_deref(), Some("02"));
        assert_eq!(field(second, Key::Album).as_deref(), Some("Melodies of Green and Blue"));
        assert_eq!(field(second, Key::Year).as_deref(), Some("2021"));
        assert_eq!(second.from_name, [Key::Track, Key::Title]);
        assert_eq!(plan.folder_mask.as_deref(), Some("%year% - %album%"));
        assert_eq!(plan.file_masks, [("%track% - %title%".to_owned(), 2)]);
        assert_eq!(
            plan.new_folder.as_deref(),
            Some(Path::new(r"G:\Torrents\Temperance\(2021) Melodies of Green and Blue"))
        );
        assert_eq!(second.rename.as_deref().and_then(Path::file_name), Some("02 - Start Another Round.mp3".as_ref()));
        // The artist from the folder above, by one's own mask.
        let this = ThisAlbum {
            folder_mask: MaskChoice::Own(r"%artist%\%year% - %album%".into()),
            ..ThisAlbum::default()
        };
        let plan = super::plan(&source, &Profile::default(), &this, &mut no_cover);
        assert_eq!(field(&plan.files[0], Key::AlbumArtist).as_deref(), Some("Temperance"));
        assert_eq!(field(&plan.files[0], Key::Artist).as_deref(), Some("Temperance"));
        // Not when a track has an artist: the album artist is theirs.
        let mut tagged = source.clone();
        tagged.files[1] = file(&format!(r"{folder}\02 - Start Another Round.mp3"), &[(Key::Artist, "Band")], &[]);
        let plan = super::plan(&tagged, &Profile::default(), &this, &mut no_cover);
        assert_eq!(field(&plan.files[0], Key::AlbumArtist).as_deref(), Some("Band"));
        assert!(!plan.from_folder.contains(&Key::AlbumArtist));
        // Not read: nothing but the title from the name.
        let this = ThisAlbum {
            file_mask: MaskChoice::Off,
            folder_mask: MaskChoice::Off,
            ..ThisAlbum::default()
        };
        let plan = super::plan(&source, &Profile::default(), &this, &mut no_cover);
        assert_eq!(field(&plan.files[1], Key::Album), None);
        assert_eq!(field(&plan.files[1], Key::Title).as_deref(), Some("Start_Another_Round"));
        // Typed for the album: as typed, the edition not cut; empty, not put.
        let this = ThisAlbum {
            album: Some(" Melodies (EP) ".into()),
            year: Some(String::new()),
            ..ThisAlbum::default()
        };
        let plan = super::plan(&source, &Profile::default(), &this, &mut no_cover);
        assert_eq!(field(&plan.files[1], Key::Album).as_deref(), Some("Melodies (EP)"));
        assert_eq!(field(&plan.files[1], Key::Year).as_deref(), Some("2021"));
        assert_eq!((plan.album.as_str(), plan.own.as_slice()), ("Melodies (EP)", [Key::Album].as_slice()));
        assert!(plan.album_cuts.is_empty());
    }

    #[test]
    fn names_lacking_a_field_kept() {
        // No year, and a file with neither a number nor a title, its name
        // without digits: "() Album" and " - .mp3" are not made.
        let source = Source {
            folder: PathBuf::from(r"Z:\Music\Album"),
            one_place: true,
            files: vec![
                file(
                    r"Z:\Music\Album\one.mp3",
                    &[(Key::Title, "One"), (Key::Album, "Album"), (Key::Track, "1")],
                    &[],
                ),
                file(r"Z:\Music\Album\two.mp3", &[(Key::Album, "Album")], &[]),
            ],
            ..Default::default()
        };
        let profile = Profile {
            fill: Vec::new(),
            ..Profile::default()
        };
        let plan = plan(&source, &profile, &ThisAlbum::default(), &mut no_cover);
        assert_eq!(plan.new_folder, None);
        assert!(plan.warnings.iter().any(|w| w.contains("Year") || w.contains("Год")), "{:?}", plan.warnings);
        assert_eq!(
            plan.files[0].rename.as_deref(),
            Some(Path::new(r"Z:\Music\Album\01 - One.mp3"))
        );
        assert_eq!(plan.files[1].rename, None);
        assert_eq!(plan.files[1].unnamed.len(), 2, "{:?}", plan.files[1].unnamed);
    }

    /// `METALAMP_ALBUM=<folder> cargo test --release prepares_a_copy -- --ignored --nocapture`:
    /// a copy of the folder prepared as the default profile says (the
    /// other files kept), printed, and prepared again: nothing left to do.
    #[test]
    #[ignore]
    fn prepares_a_copy_of_a_real_album() {
        let Some(album) = std::env::var_os("METALAMP_ALBUM").map(PathBuf::from) else {
            return;
        };
        let root = std::env::temp_dir().join(format!("metalamp_prepare_{}", std::process::id()));
        let copy = root.join(album.file_name().unwrap());
        for entry in walkdir::WalkDir::new(&album).into_iter().filter_map(Result::ok) {
            let to = copy.join(entry.path().strip_prefix(&album).unwrap());
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&to).unwrap();
            } else {
                std::fs::copy(entry.path(), &to).unwrap();
            }
        }
        let paths: Vec<PathBuf> = walkdir::WalkDir::new(&copy)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file() && crate::filetypes::is_audio(e.path()))
            .map(walkdir::DirEntry::into_path)
            .collect();
        let profile = Profile {
            extras: Extras::Keep,
            ..Profile::default()
        };
        let source = read(paths);
        let first = plan(&source, &profile, &ThisAlbum::default(), &mut made(&source));
        println!("folder: {:?} -> {:?}", first.folder, first.new_folder);
        println!("folder.jpg written: {}", first.folder_jpg.is_some());
        for w in &first.warnings {
            println!("! {w}");
        }
        for f in &first.files {
            println!(
                "{:?} -> {:?}\n   {:?}\n   removed {:?}\n   cover {} skip {:?} missing {:?}",
                f.path.file_name().unwrap(),
                f.rename.as_ref().and_then(|r| r.file_name()),
                f.fields,
                f.removed.iter().map(|i| &i.code).collect::<Vec<_>>(),
                f.cover,
                f.skip,
                f.missing
            );
        }
        let done = AtomicUsize::new(0);
        let outcome = apply(&first, &profile, WriteTags::default(), None, &done);
        println!("written {}, moved {}, errors {:?}", outcome.written, outcome.moved.len(), outcome.errors);
        let folder = outcome.folder.as_ref().map_or(copy.clone(), |(_, new)| new.clone());
        let paths: Vec<PathBuf> = walkdir::WalkDir::new(&folder)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file() && crate::filetypes::is_audio(e.path()))
            .map(walkdir::DirEntry::into_path)
            .collect();
        let source = read(paths);
        let again = plan(&source, &profile, &ThisAlbum::default(), &mut made(&source));
        let _ = std::fs::remove_dir_all(&root);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        let left: Vec<String> = again
            .files
            .iter()
            .filter(|f| f.writes() || f.rename.is_some())
            .map(|f| format!("{:?}: {:?} {:?} {:?}", f.path.file_name(), f.fields, f.removed, f.rename))
            .collect();
        assert!(
            again.is_empty(),
            "{left:#?} folder {:?} jpg {}",
            again.new_folder,
            again.folder_jpg.is_some()
        );
    }
}
