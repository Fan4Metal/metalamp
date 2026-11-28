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

use super::edit::{self, Key, MAIN, TagPart, WriteTags};

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

/// What the preparation does (the settings' `prepare`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// The edition cut from the album's title: a part in brackets at its
    /// end holding one of `editions` (`Album [Deluxe Edition]`).
    pub album_title: bool,
    /// The words of an edition, separated by commas.
    pub editions: String,
    /// Track numbers as `01` (three digits past 99 tracks), without the
    /// total; the disc's number only on an album of several discs.
    pub numbers: bool,
    /// The album, album artist, year and genre alike in every file (the
    /// most common one), the empty ones filled: the album artist from the
    /// artists, the title from the file's name, the artist from the album
    /// artist.
    pub unify: bool,
    /// The year as its four digits (`2011-05-01` → `2011`).
    pub year_only: bool,
    /// The tag's parts not in `keep` removed.
    pub clean: bool,
    pub keep: Vec<TagPart>,
    /// An MP3's ID3v1 and APE tags removed (else as the settings say).
    pub only_id3v2: bool,
    /// The fields every file is to have.
    pub required: Vec<Need>,
    /// `folder.jpg` and the same cover in every file's tag.
    pub cover: bool,
    /// `folder.jpg` the picture the tags get (at most the tags' side);
    /// else the source as it is, the tags a copy at most that side.
    pub folder_jpg_same: bool,
    pub rename_files: bool,
    /// The files' names, by `title_format`'s templates (the extension
    /// follows); for an album of several artists the second.
    pub file_name: String,
    pub file_name_various: String,
    pub rename_folder: bool,
    pub folder_name: String,
    pub extras: Extras,
    pub extras_folder: String,
    /// A file with the read-only attribute has it taken off to be written.
    pub clear_read_only: bool,
}

/// The editions' words by default.
pub const EDITIONS: &str = "deluxe, limited, special, expanded, anniversary, collector, \
    remaster, reissue, bonus track, edition, digipak, japan, tour, version";

impl Default for Profile {
    fn default() -> Self {
        Profile {
            album_title: true,
            editions: EDITIONS.to_owned(),
            numbers: true,
            unify: true,
            year_only: true,
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
            cover: true,
            folder_jpg_same: true,
            rename_files: true,
            file_name: "%track% - %title%".to_owned(),
            file_name_various: "%track% - %artist% - %title%".to_owned(),
            rename_folder: true,
            folder_name: "(%year%) %album%".to_owned(),
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

/// The ready profiles: everything; the tags and the cover alone; the
/// names alone.
pub fn ready() -> [(&'static str, Profile); 3] {
    let all = Profile::default();
    let tags = Profile {
        rename_files: false,
        rename_folder: false,
        extras: Extras::Keep,
        ..Profile::default()
    };
    let names = Profile {
        album_title: false,
        numbers: false,
        unify: false,
        year_only: false,
        clean: false,
        cover: false,
        extras: Extras::Keep,
        required: Vec::new(),
        ..Profile::default()
    };
    [
        (tr!("Everything", "Всё"), all),
        (tr!("Tags and cover", "Теги и обложка"), tags),
        (tr!("Names only", "Только имена"), names),
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

/// The cover the album gets, and where it came from.
#[derive(Clone, Debug)]
pub struct CoverSource {
    /// The picture, or the track whose tag held it.
    pub from: PathBuf,
    pub in_tag: bool,
    pub original: Arc<Vec<u8>>,
    /// As the tags get it (`covers::for_tag` at the settings' side).
    pub for_tag: Arc<Vec<u8>>,
    pub size: (u32, u32),
    pub tag_size: (u32, u32),
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
    pub cover: Option<CoverSource>,
    /// Audio files of the folders that are not the album's: the folder is
    /// not renamed then.
    pub foreign: Vec<PathBuf>,
}

/// Reads the album of the files at `paths` (on a thread: every tag is
/// read, the cover decoded); `side` the tags' largest cover.
pub fn read(paths: Vec<PathBuf>, side: Option<u32>) -> Source {
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
    let cover = cover_source(&folder, folder_jpg.as_ref(), &files, side);
    Source {
        folder,
        one_place,
        files,
        extras,
        folder_jpg,
        cover,
        foreign,
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

/// The cover the album gets: `folder.jpg`, else another picture of the
/// folder named as a cover, else the cover most of its tags hold.
fn cover_source(
    folder: &Path,
    folder_jpg: Option<&(PathBuf, Arc<Vec<u8>>)>,
    files: &[FileRead],
    side: Option<u32>,
) -> Option<CoverSource> {
    let mut candidates: Vec<(PathBuf, bool, Arc<Vec<u8>>)> = Vec::new();
    if let Some((path, bytes)) = folder_jpg {
        candidates.push((path.clone(), false, bytes.clone()));
    }
    if let Some(path) = crate::covers::in_folder(folder)
        && !is_folder_jpg(&path)
        && let Ok(bytes) = std::fs::read(&path)
    {
        candidates.push((path, false, Arc::new(bytes)));
    }
    let mut counts: HashMap<&[u8], (usize, &Path)> = HashMap::new();
    for f in files {
        if let Ok(tag) = &f.tag
            && let Some(cover) = &tag.cover
        {
            counts.entry(cover.as_slice()).or_insert((0, &f.path)).0 += 1;
        }
    }
    if let Some((bytes, (_, path))) = counts.into_iter().max_by_key(|(_, (n, _))| *n) {
        candidates.push((path.to_path_buf(), true, Arc::new(bytes.to_vec())));
    }
    candidates.into_iter().find_map(|(from, in_tag, original)| {
        let made = crate::covers::for_tag(original.to_vec(), side, 16).ok()?;
        Some(CoverSource {
            from,
            in_tag,
            original,
            for_tag: Arc::new(made.data),
            size: made.from,
            tag_size: made.to,
        })
    })
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

/// `title` without the parts in brackets at its end that name an edition
/// (any of `words`, separated by commas, in any case).
pub fn without_edition(title: &str, words: &str) -> String {
    let words: Vec<String> = words
        .split(',')
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
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
        let inner = title[open + 1..title.len() - 1].to_lowercase();
        if !words.iter().any(|w| inner.contains(w.as_str())) {
            break;
        }
        let cut = title[..open].trim_end().trim_end_matches(['-', '–']).trim_end();
        if cut.is_empty() {
            break;
        }
        title = cut.to_owned();
    }
    title
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
    /// The tag's items removed, by part and name.
    pub removed: Vec<(TagPart, String)>,
    /// The album's cover put into its tag.
    pub cover: bool,
    /// Its ID3v1 or APE tag removed.
    pub other_tags: bool,
    /// Its new name, in its folder.
    pub rename: Option<PathBuf>,
    /// Why its tag is not written (a format not written, a tag that
    /// cannot be read, the read-only attribute kept).
    pub skip: Option<String>,
    /// The fields it is to have and lacks.
    pub missing: Vec<Need>,
    pub read_only: bool,
}

impl FilePlan {
    /// Its tag is written.
    pub fn writes(&self) -> bool {
        self.skip.is_none()
            && (!self.fields.is_empty() || !self.removed.is_empty() || self.cover || self.other_tags)
    }
}

/// What the preparation would do.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub folder: PathBuf,
    pub new_folder: Option<PathBuf>,
    pub files: Vec<FilePlan>,
    /// `folder.jpg` written with these bytes.
    pub folder_jpg: Option<Arc<Vec<u8>>>,
    /// The cover the tags get.
    pub cover: Option<Arc<Vec<u8>>>,
    /// The other files deleted or moved.
    pub extras: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl Plan {
    /// Nothing to do.
    pub fn is_empty(&self) -> bool {
        self.new_folder.is_none()
            && self.folder_jpg.is_none()
            && self.extras.is_empty()
            && self.files.iter().all(|f| !f.writes() && f.rename.is_none())
    }

    pub fn writes(&self) -> usize {
        self.files.iter().filter(|f| f.writes()).count()
    }

    pub fn renames(&self) -> usize {
        self.files.iter().filter(|f| f.rename.is_some()).count()
    }
}

/// What `profile` would do to the album `source` read.
pub fn plan(source: &Source, profile: &Profile) -> Plan {
    let n = source.files.len();
    let original: Vec<[String; MAIN]> = source
        .files
        .iter()
        .map(|f| f.tag.as_ref().map(|t| t.main.clone()).unwrap_or_default())
        .collect();
    let mut values = original.clone();
    let at = |k: Key| k as usize;
    let mut warnings = Vec::new();

    if profile.album_title {
        for v in &mut values {
            v[at(Key::Album)] = without_edition(&v[at(Key::Album)], &profile.editions);
        }
    }
    if profile.year_only {
        for v in &mut values {
            v[at(Key::Year)] = year_only(&v[at(Key::Year)]);
        }
    }
    let main_artists: HashSet<String> = values
        .iter()
        .map(|v| super::main_artist(&v[at(Key::Artist)]))
        .filter(|a| !a.is_empty())
        .collect();
    if profile.unify && n > 0 {
        let common = |values: &[[String; MAIN]], k: Key| {
            most_common(values.iter().map(|v| v[at(k)].as_str()))
        };
        if let Some(album) = common(&values, Key::Album) {
            values.iter_mut().for_each(|v| v[at(Key::Album)] = album.clone());
        }
        let album_artist = common(&values, Key::AlbumArtist).or_else(|| {
            let mut counts: HashMap<&String, usize> = HashMap::new();
            for v in &values {
                *counts
                    .entry(&v[at(Key::Artist)])
                    .or_default() += 1;
            }
            let most = counts.values().copied().max().unwrap_or(0);
            if main_artists.len() >= 3 && most * 2 <= n {
                Some(super::VARIOUS_ARTISTS.to_owned())
            } else {
                common(&values, Key::Artist)
            }
        });
        if let Some(aa) = &album_artist {
            values.iter_mut().for_each(|v| v[at(Key::AlbumArtist)] = aa.clone());
        }
        let various = album_artist
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case(super::VARIOUS_ARTISTS));
        for (v, f) in values.iter_mut().zip(&source.files) {
            if v[at(Key::Artist)].trim().is_empty()
                && let Some(aa) = &album_artist
                && !various
            {
                v[at(Key::Artist)] = aa.clone();
            }
            if v[at(Key::Title)].trim().is_empty() {
                v[at(Key::Title)] = title_from_name(&f.path);
            }
        }
        for k in [Key::Year, Key::Genre] {
            if let Some(value) = common(&values, k) {
                for v in &mut values {
                    // A compilation's tracks keep their own years.
                    if !various || v[at(k)].trim().is_empty() {
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
    let discs: HashSet<u32> = values
        .iter()
        .filter_map(|v| number_of(&v[at(Key::Disc)]))
        .filter(|&d| d > 0)
        .collect();
    let several_discs = discs.len() > 1 || folders.len() > 1;
    let numbers: Vec<Option<u32>> = values
        .iter()
        .zip(&source.files)
        .map(|(v, f)| number_of(&v[at(Key::Track)]).or_else(|| leading_number(&stem(&f.path))))
        .collect();
    let most = numbers.iter().flatten().copied().max().unwrap_or(0).max(n as u32);
    let width = most.to_string().len().max(2);
    let padded = |number: Option<u32>| number.map(|t| format!("{t:0width$}"));
    if profile.numbers {
        for ((v, f), number) in values.iter_mut().zip(&source.files).zip(&numbers) {
            if let Some(t) = padded(*number) {
                v[at(Key::Track)] = t;
            }
            v[at(Key::Disc)] = if several_discs {
                number_of(&v[at(Key::Disc)])
                    .filter(|&d| d > 0)
                    .or_else(|| (folders.len() > 1).then(|| disc_of_folder(&f.path)).flatten())
                    .map(|d| d.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
        }
    }

    let various = values.iter().any(|v| {
        let aa = super::main_artist(&v[at(Key::AlbumArtist)]);
        !aa.is_empty() && super::main_artist(&v[at(Key::Artist)]) != aa
    });
    let cover = source.cover.as_ref().filter(|_| profile.cover);
    let mut files: Vec<FilePlan> = Vec::with_capacity(n);
    for (i, f) in source.files.iter().enumerate() {
        let mut plan = FilePlan {
            path: f.path.clone(),
            read_only: f.read_only,
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
                    .filter(|(p, _)| !profile.keep.contains(p))
                    .cloned()
                    .collect();
            }
            plan.cover = cover.is_some_and(|c| tag.cover.as_deref() != Some(c.for_tag.as_slice()));
            plan.other_tags = profile.only_id3v2 && (tag.id3v1 || tag.ape);
            let lyrics = tag.parts.iter().any(|(p, _)| *p == TagPart::Lyrics)
                && !(profile.clean && !profile.keep.contains(&TagPart::Lyrics));
            for need in &profile.required {
                let lacks = match need {
                    Need::Cover => cover.is_none() && tag.cover.is_none(),
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
            let template = if several_discs && folders.len() == 1 {
                format!("%disc%-{template}")
            } else {
                template.clone()
            };
            let v = &values[i];
            let track = padded(number_of(&v[at(Key::Track)]).or(numbers[i])).unwrap_or_default();
            let name = crate::title_format::render(&template, &|field| {
                Some(match field {
                    "track" => track.clone(),
                    "title" => v[at(Key::Title)].clone(),
                    "artist" => v[at(Key::Artist)].clone(),
                    "album" => v[at(Key::Album)].clone(),
                    "albumartist" => v[at(Key::AlbumArtist)].clone(),
                    "year" => year_only(&v[at(Key::Year)]),
                    "genre" => v[at(Key::Genre)].clone(),
                    "disc" => number_of(&v[at(Key::Disc)]).map(|d| d.to_string()).unwrap_or_default(),
                    "filename" => stem(&f.path),
                    _ => return None,
                })
            });
            let name = as_file_name(&name);
            let extension = f
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !name.is_empty() && !v[at(Key::Title)].trim().is_empty() {
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
            let name = crate::title_format::render(&profile.folder_name, &|field| {
                Some(match field {
                    "album" => v[at(Key::Album)].clone(),
                    "albumartist" => v[at(Key::AlbumArtist)].clone(),
                    "artist" => v[at(Key::Artist)].clone(),
                    "year" => year_only(&v[at(Key::Year)]),
                    "genre" => v[at(Key::Genre)].clone(),
                    _ => return None,
                })
            });
            let name = as_file_name(&name);
            let new = source.folder.with_file_name(&name);
            if !name.is_empty() && new.file_name() != source.folder.file_name() {
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
    if let Some(c) = cover {
        let wanted = if profile.folder_jpg_same {
            &c.for_tag
        } else {
            &c.original
        };
        let same = source
            .folder_jpg
            .as_ref()
            .is_some_and(|(_, bytes)| bytes.as_slice() == wanted.as_slice());
        if !same && source.one_place {
            folder_jpg = Some(wanted.clone());
        }
    } else if profile.cover {
        warnings.push(
            tr!(
                "No cover was found: not folder.jpg, nor a picture, nor a tag's.",
                "Обложка не найдена: нет ни folder.jpg, ни картинки в папке, ни обложки в тегах."
            )
            .to_owned(),
        );
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
        cover: cover.map(|c| c.for_tag.clone()),
        extras,
        warnings,
    }
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
                changes.cover = Some(edit::CoverChange::Set(cover.clone()));
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
                "removed": f.removed.iter().map(|(_, name)| name).collect::<Vec<_>>(),
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
        let cut = |t: &str| without_edition(t, EDITIONS);
        assert_eq!(cut("Amaranthe [Deluxe]"), "Amaranthe");
        assert_eq!(cut("Album (Limited Edition) [Remastered 2015]"), "Album");
        assert_eq!(cut("Album - (Deluxe Edition)"), "Album");
        assert_eq!(cut("Live (In Moscow)"), "Live (In Moscow)");
        assert_eq!(cut("(Deluxe)"), "(Deluxe)");
        assert_eq!(cut("Album (Deluxe"), "Album (Deluxe");
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

    fn file(path: &str, main: &[(Key, &str)], parts: &[TagPart]) -> FileRead {
        let mut values: [String; MAIN] = Default::default();
        for (k, v) in main {
            values[*k as usize] = (*v).to_owned();
        }
        FileRead {
            path: PathBuf::from(path),
            tag: Ok(edit::Inspected {
                main: values,
                parts: parts.iter().map(|p| (*p, format!("{p:?}"))).collect(),
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
        let plan = plan(&source, &Profile::default());
        let first = &plan.files[0];
        let field = |f: &FilePlan, k: Key| f.fields.iter().find(|(key, ..)| *key == k).map(|(_, _, a)| a.clone());
        assert_eq!(field(first, Key::Album).as_deref(), Some("Amaranthe"));
        assert_eq!(field(first, Key::AlbumArtist).as_deref(), Some("Amaranthe"));
        assert_eq!(field(first, Key::Track).as_deref(), Some("01"));
        assert_eq!(field(first, Key::Disc).as_deref(), Some(""));
        assert_eq!(field(first, Key::Year).as_deref(), Some("2011"));
        assert_eq!(field(first, Key::Genre), None, "the first's genre is one of the most common");
        assert_eq!(field(&plan.files[1], Key::Genre).as_deref(), Some("Metal"));
        assert_eq!(first.removed.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [TagPart::Comment]);
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
        let again = super::plan(&prepared, &Profile::default());
        assert!(again.is_empty(), "{again:#?}");
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
        let source = read(paths, Some(600));
        let first = plan(&source, &profile);
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
                f.removed.iter().map(|(_, n)| n).collect::<Vec<_>>(),
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
        let again = plan(&read(paths, Some(600)), &profile);
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
