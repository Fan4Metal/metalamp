//! Writing a track's tag (the file info window's editing, `ui::info`):
//! the main fields, the other text fields the tag already has, and the
//! usual ones it can hold added (`Values::addable`, `STANDARD`). The
//! file's own tag is read (an MP3's first ID3v2
//! tag, as the library reads it: `tags::first_tag_view`), split into the
//! fields every tag has and the rest (`SplitTag`), the fields changed, and
//! the two merged back (`MergeTag`) and written: frames lofty cannot map
//! to the common fields (pictures stay too, private frames, ratings of
//! other players) are kept. The cover is changed among the pictures of the
//! common fields (`CoverChange`): lofty moves an ID3v2's APIC frames, a
//! FLAC's PICTURE blocks and an MP4's `covr` there and back. MP3 and
//! WAV (ID3v2), FLAC (Vorbis comments) and M4A (iTunes atoms); WMA is
//! read but not written by lofty. How an MP3's tags are written is the
//! settings' (`WriteTags`): the ID3v2 version (the file's own and a new
//! tag's 2.3 by default), and whether its ID3v1 and APE tags, which would
//! keep the values from before, are written alike (by default when it has
//! them), removed or left as they were.

use std::io::BufReader;
use std::path::Path;

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType};
use lofty::ogg::OggPictureStorage;
use lofty::prelude::*;
use lofty::TextEncoding;
use lofty::id3::v2::{Frame, FrameId, Id3v2Tag, TextInformationFrame};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::items::Timestamp;
use lofty::tag::{ItemKey, ItemValue, MergeTag, SplitTag, Tag, TagItem, TagType};

use crate::settings::{ApeTag, Id3Version, Id3v1Tag};

/// How an MP3's tags are written (the settings' Tags section).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteTags {
    pub id3v2: Id3Version,
    pub id3v1: Id3v1Tag,
    pub ape: ApeTag,
}

/// The main fields, in the order the window shows them (`Key::ALL`, and
/// `key as usize` is the place in it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Composer,
    Year,
    Genre,
    Track,
    Disc,
    Comment,
}

/// How many main fields there are.
pub const MAIN: usize = 10;

impl Key {
    pub const ALL: [Key; MAIN] = [
        Key::Title,
        Key::Artist,
        Key::Album,
        Key::AlbumArtist,
        Key::Composer,
        Key::Year,
        Key::Genre,
        Key::Track,
        Key::Disc,
        Key::Comment,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Key::Title => tr!("Title", "Название"),
            Key::Artist => tr!("Artist", "Исполнитель"),
            Key::AlbumArtist => tr!("Album artist", "Исполнитель альбома"),
            Key::Album => tr!("Album", "Альбом"),
            Key::Composer => tr!("Composer", "Композитор"),
            Key::Year => tr!("Year", "Год"),
            Key::Track => tr!("Track", "Номер"),
            Key::Disc => tr!("Disc", "Диск"),
            Key::Genre => tr!("Genre", "Жанр"),
            Key::Comment => tr!("Comment", "Комментарий"),
        }
    }

    /// The tag's keys the field is read from, the first one the tag has
    /// (ID3v2.3's TYER is read as ID3v2.4's TDRC); the track and disc
    /// numbers carry their totals (`details`).
    pub fn items(self) -> &'static [ItemKey] {
        match self {
            Key::Title => &[ItemKey::TrackTitle],
            Key::Artist => &[ItemKey::TrackArtist],
            Key::Album => &[ItemKey::AlbumTitle],
            Key::AlbumArtist => &[ItemKey::AlbumArtist],
            Key::Composer => &[ItemKey::Composer],
            Key::Year => &[ItemKey::Year, ItemKey::RecordingDate],
            Key::Genre => &[ItemKey::Genre],
            Key::Track => &[ItemKey::TrackNumber],
            Key::Disc => &[ItemKey::DiscNumber],
            Key::Comment => &[ItemKey::Comment],
        }
    }

    /// A number field: the year, `3` or `3/12` for the track and the disc.
    pub fn is_number(self) -> bool {
        matches!(self, Key::Year | Key::Track | Key::Disc)
    }

    /// `value` can be written to this field: any text, or for a number
    /// field a number (with its total) or a date, or nothing.
    pub fn takes(self, value: &str) -> bool {
        !self.is_number() || number_text(self, value).is_ok()
    }
}

/// A key of the tag that is none of the main fields (nor a total of
/// theirs), nor the length (the window's chip) or the lyrics (a section
/// of their own): the window's "Additional" fields.
pub fn is_other(key: ItemKey) -> bool {
    !Key::ALL.iter().any(|k| k.items().contains(&key))
        && !matches!(
            key,
            ItemKey::TrackTotal
                | ItemKey::DiscTotal
                | ItemKey::Length
                | ItemKey::Lyrics
                | ItemKey::UnsyncLyrics
        )
}

/// The groups the fields that can be added are offered in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    People,
    Release,
    Music,
    Links,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::People, Group::Release, Group::Music, Group::Links];

    pub fn label(self) -> &'static str {
        match self {
            Group::People => tr!("People", "Участники"),
            Group::Release => tr!("Release", "Издание"),
            Group::Music => tr!("Music", "Музыка"),
            Group::Links => tr!("Links", "Ссылки"),
        }
    }
}

/// The usual fields of taggers beyond the main ones, offered to be added
/// to a tag that lacks them ([`addable`]): those the file's tag can hold.
/// Not the sort orders, nor the podcasts' and the videos' fields.
const STANDARD: &[(Group, ItemKey)] = &[
    (Group::People, ItemKey::OriginalArtist),
    (Group::People, ItemKey::Conductor),
    (Group::People, ItemKey::Lyricist),
    (Group::People, ItemKey::OriginalLyricist),
    (Group::People, ItemKey::Remixer),
    (Group::People, ItemKey::Arranger),
    (Group::People, ItemKey::Producer),
    (Group::People, ItemKey::Engineer),
    (Group::Release, ItemKey::Publisher),
    (Group::Release, ItemKey::Label),
    (Group::Release, ItemKey::CopyrightMessage),
    (Group::Release, ItemKey::OriginalAlbumTitle),
    (Group::Release, ItemKey::SetSubtitle),
    (Group::Release, ItemKey::OriginalReleaseDate),
    (Group::Release, ItemKey::ReleaseDate),
    (Group::Release, ItemKey::Isrc),
    (Group::Release, ItemKey::CatalogNumber),
    (Group::Release, ItemKey::Barcode),
    (Group::Release, ItemKey::EncodedBy),
    (Group::Release, ItemKey::InternetRadioStationName),
    (Group::Music, ItemKey::ContentGroup),
    (Group::Music, ItemKey::TrackSubtitle),
    (Group::Music, ItemKey::Bpm),
    (Group::Music, ItemKey::IntegerBpm),
    (Group::Music, ItemKey::InitialKey),
    (Group::Music, ItemKey::Mood),
    (Group::Music, ItemKey::Language),
    (Group::Links, ItemKey::AudioFileUrl),
    (Group::Links, ItemKey::TrackArtistUrl),
    (Group::Links, ItemKey::AudioSourceUrl),
    (Group::Links, ItemKey::RadioStationUrl),
    (Group::Links, ItemKey::CommercialInformationUrl),
    (Group::Links, ItemKey::PublisherUrl),
    (Group::Links, ItemKey::CopyrightUrl),
    (Group::Links, ItemKey::PaymentUrl),
];

/// ID3v2.4's frames lofty leaves out of a tag written as 2.3.
const NOT_IN_23: &[&str] = &["TDRL", "TMOO", "TSST", "TSOA", "TSOP", "TSOT"];

/// What the tag edited is: its type, and for ID3v2 whether it is written
/// as 2.3 (`Id3Version::writes_23`).
#[derive(Clone, Copy, Debug)]
struct Kind {
    tag: TagType,
    id3v23: bool,
}

/// The [`STANDARD`] fields a tag of `kind` can hold, as the key it is read
/// back as (ID3v2's TPUB is the publisher, not the label), each once.
fn standard_for(kind: Kind) -> Vec<(Group, ItemKey)> {
    let mut out: Vec<(Group, ItemKey)> = Vec::new();
    for &(group, key) in STANDARD {
        let Some(mapped) = key.map_key(kind.tag) else {
            continue;
        };
        if kind.tag == TagType::Id3v2 && kind.id3v23 && NOT_IN_23.contains(&mapped) {
            continue;
        }
        // MP4's `tmpo` holds a number, which lofty writes from text as
        // text: its freeform BPM instead.
        if kind.tag == TagType::Mp4Ilst && key == ItemKey::IntegerBpm {
            continue;
        }
        // The people ID3v2 lists in TIPL are keys of their own.
        let read_as = match ItemKey::from_key(kind.tag, mapped) {
            Some(k) if mapped != "TIPL" => k,
            _ => key,
        };
        if !out.iter().any(|&(_, k)| k == read_as) {
            out.push((group, read_as));
        }
    }
    out
}

/// The files whose tags can be written (`with_tag`'s formats), by the
/// extension.
pub fn writable(path: &Path) -> bool {
    matches!(
        FileType::from_path(path),
        Some(FileType::Mpeg | FileType::Wav | FileType::Flac | FileType::Mp4)
    )
}

/// A tag's fields as the window edits them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    /// In [`Key::ALL`]'s order; empty: none.
    pub main: [String; MAIN],
    /// The other text fields ([`is_other`]), several values of a key
    /// joined with "; ".
    pub other: Vec<(ItemKey, String)>,
    /// The usual fields the tag can hold, by group ([`STANDARD`]), those
    /// it lacks to be added.
    pub standard: Vec<(Group, ItemKey)>,
    /// The lyrics (the first lyrics item: one language's), lines ending
    /// with `\n`; empty: none.
    pub lyrics: String,
}

/// What to write.
#[derive(Clone, Debug, Default)]
pub struct Changes {
    /// For each main field, the value to put (empty: the field removed),
    /// or `None` to leave it as it is.
    pub main: [Option<String>; MAIN],
    /// Other fields to put (values split at "; "; empty: removed).
    pub other: Vec<(ItemKey, String)>,
    /// The lyrics put (empty: removed), or `None` to leave them.
    pub lyrics: Option<String>,
    /// The cover put or removed, or `None` to leave it as it is.
    pub cover: Option<CoverChange>,
    /// The parts of the tag beyond the main fields that stay, the rest
    /// removed (an album's preparation, `prepare`); `None`: all stay.
    pub keep: Option<Vec<TagPart>>,
}

/// The parts of a tag beyond the main fields, kept or removed as a whole
/// when an album is prepared (`prepare::Profile::keep`). The main fields
/// (title, artists, album, year, genre, numbers, the length) and the front
/// cover are none of them: they always stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagPart {
    Composer,
    Comment,
    Lyrics,
    ReplayGain,
    MusicBrainz,
    Rating,
    /// The pictures other than the front cover (the back, the booklet).
    Pictures,
    /// The other people: conductor, lyricist, remixer…
    People,
    /// Publisher, label, copyright, ISRC, catalogue number, dates…
    Release,
    /// BPM, key, mood, language, grouping…
    Music,
    /// Encoded by, the encoder's name and settings.
    Encoder,
    Links,
    Sorting,
    /// Whatever else: private frames, players' and rippers' own.
    Other,
}

impl TagPart {
    pub const ALL: [TagPart; 14] = [
        TagPart::Composer,
        TagPart::Comment,
        TagPart::Lyrics,
        TagPart::ReplayGain,
        TagPart::MusicBrainz,
        TagPart::Rating,
        TagPart::Pictures,
        TagPart::People,
        TagPart::Release,
        TagPart::Music,
        TagPart::Encoder,
        TagPart::Links,
        TagPart::Sorting,
        TagPart::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TagPart::Composer => tr!("Composer", "Композитор"),
            TagPart::Comment => tr!("Comment", "Комментарий"),
            TagPart::Lyrics => tr!("Lyrics", "Текст песни"),
            TagPart::ReplayGain => "ReplayGain",
            TagPart::MusicBrainz => "MusicBrainz",
            TagPart::Rating => tr!("Rating", "Рейтинг"),
            TagPart::Pictures => tr!("Other pictures", "Другие картинки"),
            TagPart::People => tr!("People", "Участники"),
            TagPart::Release => tr!("Release", "Издание"),
            TagPart::Music => tr!("Music", "Музыка"),
            TagPart::Encoder => tr!("Encoder", "Кодировщик"),
            TagPart::Links => tr!("Links", "Ссылки"),
            TagPart::Sorting => tr!("Sort orders", "Сортировка"),
            TagPart::Other => tr!("Other", "Прочее"),
        }
    }

    /// What it holds, for the tooltip.
    pub fn hint(self) -> &'static str {
        match self {
            TagPart::Composer => "TCOM",
            TagPart::Comment => "COMM",
            TagPart::Lyrics => tr!("USLT, SYLT", "USLT, SYLT"),
            TagPart::ReplayGain => tr!("Track and album gain and peak", "Усиление и пик трека и альбома"),
            TagPart::MusicBrainz => tr!("MusicBrainz and AcoustID identifiers", "Идентификаторы MusicBrainz и AcoustID"),
            TagPart::Rating => tr!("Ratings and play counts of other players (POPM, PCNT)", "Рейтинги и счётчики других плееров (POPM, PCNT)"),
            TagPart::Pictures => tr!("Back cover, booklet, disc… (the front cover stays)", "Задняя обложка, буклет, диск… (передняя обложка остаётся)"),
            TagPart::People => tr!("Conductor, lyricist, remixer, producer…", "Дирижёр, автор текста, ремиксер, продюсер…"),
            TagPart::Release => tr!("Publisher, label, copyright, ISRC, catalogue number, release dates…", "Издатель, лейбл, копирайт, ISRC, каталожный номер, даты выпуска…"),
            TagPart::Music => tr!("BPM, key, mood, language, grouping…", "BPM, тональность, настроение, язык, группа…"),
            TagPart::Encoder => tr!("Encoded by, encoder and its settings (TENC, TSSE)", "Кем закодировано, кодировщик и его настройки (TENC, TSSE)"),
            TagPart::Links => tr!("URLs (W*** frames)", "Адреса URL (кадры W***)"),
            TagPart::Sorting => tr!("Sort orders of the titles and names", "Порядок сортировки названий и имён"),
            TagPart::Other => tr!("Private frames, rippers' and players' own fields", "Служебные кадры, поля рипперов и плееров"),
        }
    }
}

/// The part an item of a tag is of; `None`: a main field, which stays.
pub fn part_of_key(key: ItemKey) -> Option<TagPart> {
    use ItemKey as K;
    Some(match key {
        K::TrackTitle
        | K::TrackArtist
        | K::AlbumTitle
        | K::AlbumArtist
        | K::Year
        | K::RecordingDate
        | K::Genre
        | K::TrackNumber
        | K::TrackTotal
        | K::DiscNumber
        | K::DiscTotal
        | K::Length => return None,
        K::Composer => TagPart::Composer,
        K::Comment => TagPart::Comment,
        K::Lyrics | K::UnsyncLyrics => TagPart::Lyrics,
        K::ReplayGainAlbumGain
        | K::ReplayGainAlbumPeak
        | K::ReplayGainTrackGain
        | K::ReplayGainTrackPeak => TagPart::ReplayGain,
        K::MusicBrainzRecordingId
        | K::MusicBrainzTrackId
        | K::MusicBrainzReleaseId
        | K::MusicBrainzReleaseGroupId
        | K::MusicBrainzArtistId
        | K::MusicBrainzReleaseArtistId
        | K::MusicBrainzWorkId
        | K::MusicBrainzReleaseType => TagPart::MusicBrainz,
        K::Popularimeter => TagPart::Rating,
        K::EncodedBy | K::EncoderSoftware | K::EncoderSettings => TagPart::Encoder,
        K::AlbumTitleSortOrder
        | K::AlbumArtistSortOrder
        | K::TrackTitleSortOrder
        | K::TrackArtistSortOrder
        | K::ComposerSortOrder
        | K::ShowNameSortOrder => TagPart::Sorting,
        K::ReleaseCountry => TagPart::Release,
        other => match STANDARD.iter().find(|(_, k)| *k == other) {
            Some((Group::People, _)) => TagPart::People,
            Some((Group::Release, _)) => TagPart::Release,
            Some((Group::Music, _)) => TagPart::Music,
            Some((Group::Links, _)) => TagPart::Links,
            None => TagPart::Other,
        },
    })
}

/// The part an ID3v2 frame is of, as the tag has it before lofty splits
/// it (so the frames it would keep apart, PRIV and the like, count too);
/// `None`: a main field or a picture (pictures are decided with the
/// common fields, `strip_items`).
fn part_of_frame(frame: &Frame) -> Option<TagPart> {
    let id = frame.id_str();
    match id {
        "APIC" | "TLEN" | "TYER" | "TDAT" | "TIME" => return None,
        "COMM" => return Some(TagPart::Comment),
        "USLT" | "SYLT" => return Some(TagPart::Lyrics),
        "POPM" | "PCNT" => return Some(TagPart::Rating),
        "UFID" => return Some(TagPart::MusicBrainz),
        "RVA2" | "RVAD" => return Some(TagPart::ReplayGain),
        "WXXX" => return Some(TagPart::Links),
        _ => {}
    }
    if let Frame::UserText(f) = frame {
        let description = f.description.to_uppercase();
        if description.starts_with("REPLAYGAIN") {
            return Some(TagPart::ReplayGain);
        }
        if description.starts_with("MUSICBRAINZ") || description.starts_with("ACOUSTID") {
            return Some(TagPart::MusicBrainz);
        }
        return match ItemKey::from_key(TagType::Id3v2, &f.description) {
            Some(key) => part_of_key(key),
            None => Some(TagPart::Other),
        };
    }
    match ItemKey::from_key(TagType::Id3v2, id) {
        Some(key) => part_of_key(key),
        None => Some(TagPart::Other),
    }
}

/// An item of a tag beyond the main fields, as an album's preparation
/// lists what it removes.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub part: TagPart,
    /// As the tag names it: an ID3v2 frame's ID (a user text frame's
    /// with its description, `TXXX:ARTISTS`), a Vorbis comment's or an
    /// MP4 atom's key, a picture's type.
    pub code: String,
    /// The field it is, when lofty knows it (for its name in words).
    pub key: Option<ItemKey>,
    /// Its text, empty when it has none (or is not text).
    pub value: String,
}

/// An ID3v2 frame as an [`Item`].
fn frame_item(frame: &Frame, part: TagPart) -> Item {
    let (code, key, value) = match frame {
        Frame::UserText(f) => (
            format!("TXXX:{}", f.description),
            ItemKey::from_key(TagType::Id3v2, &f.description),
            f.content.to_string(),
        ),
        Frame::UserUrl(f) => (
            format!("WXXX:{}", f.description),
            None,
            f.content.to_string(),
        ),
        _ => {
            let id = frame.id_str();
            let value = match frame {
                Frame::Text(f) => f.value.to_string(),
                Frame::Comment(f) => f.content.to_string(),
                Frame::UnsynchronizedText(f) => f.content.to_string(),
                Frame::Url(f) => f.url().to_owned(),
                _ => String::new(),
            };
            (id.to_owned(), ItemKey::from_key(TagType::Id3v2, id), value)
        }
    };
    Item {
        part,
        code,
        key,
        value: value.trim_matches('\0').trim().to_owned(),
    }
}

/// Removes the frames of the parts not in `keep`, before the tag is
/// split; what was removed.
fn strip_frames(tag: &mut Id3v2Tag, keep: &[TagPart]) -> Vec<Item> {
    let mut removed = Vec::new();
    tag.retain(|frame| match part_of_frame(frame) {
        Some(part) if !keep.contains(&part) => {
            removed.push(frame_item(frame, part));
            false
        }
        _ => true,
    });
    removed
}

/// Removes the common tag's items of the parts not in `keep`, and its
/// pictures other than the front cover (the first front cover, else the
/// first picture) unless `TagPart::Pictures` is kept; what was removed.
fn strip_items(tag: &mut Tag, keep: &[TagPart]) -> Vec<Item> {
    let mut removed = Vec::new();
    let tag_type = tag.tag_type();
    tag.retain(|item| match part_of_key(item.key()) {
        Some(part) if !keep.contains(&part) => {
            let code = item
                .key()
                .map_key(tag_type)
                .map_or_else(|| format!("{:?}", item.key()), str::to_owned);
            let value = item.value().text().or(item.value().locator()).unwrap_or("");
            removed.push(Item {
                part,
                code,
                key: Some(item.key()),
                value: value.trim().to_owned(),
            });
            false
        }
        _ => true,
    });
    if !keep.contains(&TagPart::Pictures) && tag.pictures().len() > 1 {
        let cover = tag
            .pictures()
            .iter()
            .position(|p| p.pic_type() == PictureType::CoverFront)
            .unwrap_or(0);
        for i in (0..tag.pictures().len()).rev() {
            if i != cover {
                let picture = tag.remove_picture(i);
                removed.push(Item {
                    part: TagPart::Pictures,
                    code: format!("{:?}", picture.pic_type()),
                    key: None,
                    value: String::new(),
                });
            }
        }
    }
    removed
}

/// What an album's preparation needs to know of a file's tag: its main
/// fields, the parts beyond them it holds (each item by part and name), and
/// its cover (the front cover, else the first picture), as the bytes.
#[derive(Clone, Debug, Default)]
pub struct Inspected {
    pub main: [String; MAIN],
    pub parts: Vec<Item>,
    pub cover: Option<Vec<u8>>,
    /// An MP3's ID3v1 and APE tags.
    pub id3v1: bool,
    pub ape: bool,
}

/// Reads `path`'s tag as [`Inspected`] (nothing is written).
pub fn inspect(path: &Path) -> Result<Inspected, String> {
    let mut inspected = with_tag(
        path,
        |tag, _, raw| {
            let main = values(tag);
            let cover = tag
                .pictures()
                .iter()
                .find(|p| p.pic_type() == PictureType::CoverFront)
                .or_else(|| tag.pictures().first())
                .map(|p| p.data().to_vec());
            let mut parts = raw;
            parts.extend(strip_items(&mut tag.clone(), &[]));
            Ok(Inspected {
                main,
                parts,
                cover,
                id3v1: false,
                ape: false,
            })
        },
        None,
        Id3Version::default(),
        Some(&[]),
    )?;
    if FileType::from_path(path) == Some(FileType::Mpeg) {
        let options = ParseOptions::new().read_properties(false);
        if let Ok(file) = std::fs::File::open(path)
            && let Ok(mp3) = lofty::mpeg::MpegFile::read_from(&mut BufReader::new(file), options)
        {
            inspected.id3v1 = mp3.id3v1().is_some();
            inspected.ape = mp3.ape().is_some();
        }
    }
    Ok(inspected)
}

/// What becomes of the tag's cover: the pictures shown as it (its front
/// covers, else its first picture, as `details` and the thumbnails take
/// it) replaced by a front cover, or removed; the other pictures (the
/// back cover, the booklet) are kept.
#[derive(Clone, Debug)]
pub enum CoverChange {
    /// A JPEG or a PNG, written as it is (`covers::for_tag` made it).
    Set(std::sync::Arc<Vec<u8>>),
    Remove,
}

/// The values of `path`'s tag (all empty when it has none); `version`
/// the ID3v2 version it would be written in, which decides the fields
/// that can be added.
pub fn read(path: &Path, version: Id3Version) -> Result<Values, String> {
    with_tag(
        path,
        |tag, kind, _| {
            let lyrics = LYRICS
                .iter()
                .find_map(|k| tag.get_strings(*k).next())
                .map(|t| t.replace("\r\n", "\n").replace('\r', "\n"))
                .unwrap_or_default();
            Ok(Values {
                main: values(tag),
                other: super::details::fields_of(tag)
                    .into_iter()
                    .filter(|(k, _)| is_other(*k))
                    .collect(),
                standard: standard_for(kind),
                lyrics,
            })
        },
        None,
        version,
        None,
    )
}

/// The keys lyrics are read from, in this order: ID3v2's USLT, a FLAC's
/// `UNSYNCEDLYRICS` and `LYRICS`, MP4's `©lyr` (either).
const LYRICS: [ItemKey; 2] = [ItemKey::UnsyncLyrics, ItemKey::Lyrics];

/// The lyrics put as the tag's first lyrics item (its language and
/// description kept; a new one's key the first of [`LYRICS`] the tag's
/// type holds), its line ends as the item before had them, else Windows'
/// (as the taggers there write them); the other items (other languages)
/// kept. Empty: every lyrics item removed.
fn put_lyrics(tag: &mut Tag, text: &str) {
    let text = text.trim_end();
    if text.is_empty() {
        for key in LYRICS {
            tag.remove_key(key);
        }
        return;
    }
    let first = LYRICS.iter().find_map(|k| tag.get(*k).cloned());
    let key = match &first {
        Some(item) => item.key(),
        None => match LYRICS.iter().find(|k| k.map_key(tag.tag_type()).is_some()) {
            Some(k) => *k,
            None => return,
        },
    };
    let unix = first
        .as_ref()
        .and_then(|i| i.value().text())
        .is_some_and(|t| t.contains('\n') && !t.contains('\r'));
    let text = if unix {
        text.to_owned()
    } else {
        text.replace('\n', "\r\n")
    };
    let rest: Vec<TagItem> = tag.get_items(key).skip(1).cloned().collect();
    tag.remove_key(key);
    let mut item = TagItem::new(key, ItemValue::Text(text));
    if let Some(first) = &first {
        item.set_lang(*first.lang());
        item.set_description(first.description().to_owned());
    }
    tag.push(item);
    for item in rest {
        tag.push(item);
    }
}

/// Writes `changes` to `path`'s tag (one is made when it has none), and
/// to an MP3's ID3v1 and APE tags as `how` says.
pub fn write(path: &Path, changes: &Changes, how: WriteTags) -> Result<(), String> {
    // Checked before the file is touched: a bad number writes nothing.
    for (key, change) in Key::ALL.iter().zip(&changes.main) {
        if let Some(v) = change
            && key.is_number()
        {
            number_text(*key, v)?;
        }
    }
    // A read-only file is left as it is (the user's choice), and said so
    // plainly rather than as Windows' "access denied".
    if std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
        return Err(read_only());
    }
    with_tag(
        path,
        |tag, _, _| apply(tag, changes),
        Some(changes),
        how.id3v2,
        changes.keep.as_deref(),
    )?;
    if FileType::from_path(path) == Some(FileType::Mpeg) {
        other_tags(path, changes, how)?;
    }
    Ok(())
}

/// An MP3's APE and ID3v1 tags after its ID3v2 was written: the fields
/// changed written into its APE tag too (the rest of it, MP3Gain's
/// ReplayGain among it, kept; a cover only into one that has pictures), or
/// the tag removed; its ID3v1 written from the ID3v2 as it now is, or
/// removed (`id3v1`).
fn other_tags(path: &Path, changes: &Changes, how: WriteTags) -> Result<(), String> {
    if how.ape == ApeTag::Keep && how.id3v1 == Id3v1Tag::Keep {
        return Ok(());
    }
    let failed = |e: &dyn std::error::Error| in_use(&described(e));
    let options = ParseOptions::new().read_properties(false);
    let (has_ape, has_id3v1) = {
        let mut reader = BufReader::new(std::fs::File::open(path).map_err(|e| failed(&e))?);
        let file =
            lofty::mpeg::MpegFile::read_from(&mut reader, options).map_err(|e| failed(&e))?;
        match how.ape {
            ApeTag::Update if file.ape().is_some() => {
                let ape = file.ape().cloned().unwrap_or_default();
                drop(reader);
                let (rest, mut common) = ape.split_tag();
                let mut changes = changes.clone();
                if common.pictures().is_empty() {
                    changes.cover = None;
                }
                apply(&mut common, &changes)?;
                rest.merge_tag(common)
                    .save_to_path(path, WriteOptions::default())
                    .map_err(|e| failed(&e))?;
            }
            _ => {}
        }
        (file.ape().is_some(), file.id3v1().is_some())
    };
    if how.ape == ApeTag::Remove && has_ape {
        // `remove_from_path` opens the file for reading only, and writing
        // to it is then refused (os error 5).
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| failed(&e))?;
        TagType::Ape
            .remove_from(&mut file, WriteOptions::default())
            .map_err(|e| failed(&e))?;
    }
    match how.id3v1 {
        Id3v1Tag::Remove if has_id3v1 => {
            super::id3v1::remove(path).map_err(|e| failed(&e))?;
        }
        Id3v1Tag::Always | Id3v1Tag::Update if how.id3v1 == Id3v1Tag::Always || has_id3v1 => {
            let values = read(path, how.id3v2)?;
            let field = |key: Key| values.main[key as usize].as_str();
            let fields = super::id3v1::Fields {
                title: field(Key::Title),
                artist: field(Key::Artist),
                album: field(Key::Album),
                year: field(Key::Year),
                comment: field(Key::Comment),
                track: field(Key::Track),
                genre: field(Key::Genre),
            };
            super::id3v1::write(path, &fields).map_err(|e| failed(&e))?;
        }
        _ => {}
    }
    Ok(())
}

/// Reads `path`'s own tag, lets `f` see (and change) its common fields,
/// and when it writes `changes`, writes it back with the rest of it kept.
/// With `keep`, an ID3v2 tag's frames of the other parts are removed
/// before it is split (`strip_frames`), and `f` is told which.
fn with_tag<R>(
    path: &Path,
    f: impl FnOnce(&mut Tag, Kind, Vec<Item>) -> Result<R, String>,
    changes: Option<&Changes>,
    version: Id3Version,
    keep: Option<&[TagPart]>,
) -> Result<R, String> {
    let strip = |tag: &mut Id3v2Tag| keep.map(|k| strip_frames(tag, k)).unwrap_or_default();
    let save = changes.is_some();
    let numbers = |tag: &mut Id3v2Tag, read: &[(&'static str, String)]| {
        if let Some(changes) = changes {
            keep_numbers(tag, changes, read);
        }
        mend_languages(tag);
    };
    let (view, file_type) = super::tags::first_tag_view(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(view);
    let options = ParseOptions::new().read_properties(false);
    let err = |e: lofty::error::FileParseError| described(&e);
    // Whether the ID3v2 tag is written as 2.3, by the version it had.
    let id3v23 = |tag: Option<&Id3v2Tag>| {
        let original = tag.map(|t| match t.original_version() {
            lofty::id3::v2::Id3v2Version::V2 => 2,
            lofty::id3::v2::Id3v2Version::V3 => 3,
            lofty::id3::v2::Id3v2Version::V4 => 4,
        });
        version.writes_23(original)
    };
    let id3 = |v23: bool| {
        let kind = Kind {
            tag: TagType::Id3v2,
            id3v23: v23,
        };
        (WriteOptions::default().use_id3v23(v23), kind)
    };
    let of = |tag: TagType| Kind {
        tag,
        id3v23: false,
    };
    let write = WriteOptions::default();
    match file_type {
        Some(FileType::Mpeg) => {
            let file = lofty::mpeg::MpegFile::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            if save && let Some(tag) = file.id3v2() {
                not_losing(path, tag)?;
            }
            let (write, kind) = id3(id3v23(file.id3v2()));
            let tag = file.id3v2().cloned().unwrap_or_else(|| {
                seeded(
                    [
                        file.ape().cloned().map(Tag::from),
                        file.id3v1().cloned().map(|v1| {
                            let mut tag = Tag::from(v1);
                            super::id3v1::read_in_code_page(&mut tag);
                            tag
                        }),
                    ]
                    .into_iter()
                    .flatten(),
                )
            });
            let mut tag = mended(tag);
            let raw = strip(&mut tag);
            let read = numbers_read(&tag);
            edit(tag, move |t| f(t, kind, raw), save, path, write, |t| numbers(t, &read))
        }
        Some(FileType::Wav) => {
            let file = lofty::iff::wav::WavFile::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            let (write, kind) = id3(id3v23(file.id3v2()));
            let tag = file
                .id3v2()
                .cloned()
                .unwrap_or_else(|| seeded(file.riff_info().cloned().map(Tag::from)));
            let mut tag = mended(tag);
            let raw = strip(&mut tag);
            let read = numbers_read(&tag);
            edit(tag, move |t| f(t, kind, raw), save, path, write, |t| numbers(t, &read))
        }
        Some(FileType::Flac) => {
            let file = lofty::flac::FlacFile::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            let mut tag = file.vorbis_comments().cloned().unwrap_or_default();
            // lofty keeps a FLAC's PICTURE blocks beside its comments and
            // writes only the tag's: they go along, or a save dropped the
            // cover.
            for (picture, info) in file.pictures() {
                let _ = tag.insert_picture(picture.clone(), Some(*info));
            }
            let kind = of(TagType::VorbisComments);
            edit(tag, move |t| f(t, kind, Vec::new()), save, path, write, |_| {})
        }
        Some(FileType::Mp4) => {
            let file = lofty::mp4::Mp4File::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            let kind = of(TagType::Mp4Ilst);
            edit(
                file.ilst().cloned().unwrap_or_default(),
                move |t| f(t, kind, Vec::new()),
                save,
                path,
                write,
                |_| {},
            )
        }
        _ => Err(tr!(
            "tags of this format cannot be written",
            "теги этого формата записать нельзя"
        )
        .to_owned()),
    }
}

/// A file without an ID3v2 tag gets one holding what its other tags say
/// (an MP3's APE, then what only its ID3v1 has; a WAV's RIFF INFO), which
/// the library read: written from one field alone, it became the tag
/// read, the rest lost.
fn seeded(others: impl IntoIterator<Item = Tag>) -> lofty::id3::v2::Id3v2Tag {
    let mut others = others.into_iter();
    let Some(mut tag) = others.next() else {
        return Default::default();
    };
    for other in others {
        for item in other.items() {
            if tag.get(item.key()).is_none() {
                tag.insert(item.clone());
            }
        }
    }
    // ID3v1's year is lofty's `Year`, which ID3v2 does not take: the
    // recording date (TDRC, or TYER in 2.3) is the year players read.
    if tag.get_string(ItemKey::RecordingDate).is_none()
        && let Some(year) = tag.get_string(ItemKey::Year).map(str::to_owned)
    {
        tag.remove_key(ItemKey::Year);
        tag.insert_text(ItemKey::RecordingDate, year);
    }
    tag.into()
}

/// `tag` split, its common fields given to `f`, merged back, `fix`ed and,
/// when `save`, written to `path`.
fn edit<T, R>(
    tag: T,
    f: impl FnOnce(&mut Tag) -> Result<R, String>,
    save: bool,
    path: &Path,
    write: WriteOptions,
    fix: impl FnOnce(&mut <T::Remainder as MergeTag>::Merged),
) -> Result<R, String>
where
    T: SplitTag,
    <T::Remainder as MergeTag>::Merged: TagExt,
{
    let (rest, mut common) = tag.split_tag();
    let out = f(&mut common)?;
    if save {
        let mut merged = rest.merge_tag(common);
        fix(&mut merged);
        merged
            .save_to_path(path, write)
            .map_err(|e| in_use(&described(&e)))?;
    }
    Ok(out)
}

/// Comments and lyrics (COMM, USLT) as old taggers (MusicMatch) wrote them
/// made such as lofty writes back: a language that is not three ASCII
/// letters (`\0\0\0`, read, but refused when written, so such a file
/// could not be edited at all: 29 of D:\Music's MP3s, Aqua's Aquarium
/// among them) given ID3's "unknown", `XXX`; NULs at either end of the
/// content or the description cut (MusicMatch wrote `\0Bio\0` as the
/// content of the comment described "Bio"; lofty splits the content at NULs into
/// values, the description only on the first, and the frames it made of
/// them were written so that it could not read the tag back).
fn mend_languages(tag: &mut Id3v2Tag) {
    tag.retain_mut(|frame| {
        let (language, description, content) = match frame {
            Frame::Comment(f) => (&mut f.language, &mut f.description, &mut f.content),
            Frame::UnsynchronizedText(f) => (&mut f.language, &mut f.description, &mut f.content),
            _ => return true,
        };
        if !language.iter().all(u8::is_ascii_alphabetic) {
            *language = *b"XXX";
        }
        for text in [description, content] {
            if text.starts_with('\0') || text.ends_with('\0') {
                *text = std::borrow::Cow::Owned(text.trim_matches('\0').to_owned());
            }
        }
        true
    });
}

/// Frames lofty turns into others as it reads an ID3v2.3 tag: TYER, TDAT
/// and TIME into TDRC, TORY into TDOR, IPLS into TIPL.
const CONVERTED: [&str; 5] = ["TYER", "TDAT", "TIME", "TORY", "IPLS"];

/// Refuses to write an MP3 whose tag lofty did not read whole: it takes
/// NULs after a frame's text (an old tagger's USLT, Aqua's Aquarium) for
/// the padding and stops there without an error, so the frames after them
/// (the cover, PRIV) were not in what it read and a write lost them.
fn not_losing(path: &Path, tag: &Id3v2Tag) -> Result<(), String> {
    let lost = lost_frames(path, tag);
    if lost.is_empty() {
        return Ok(());
    }
    log::warn!("{}: lofty did not read {lost:?}: not written", path.display());
    let lost = lost.join(", ");
    Err(tr!(
        format!("the tag cannot be read whole: its frames {lost} would be lost, so it is not written"),
        format!("тег читается не полностью: при записи потерялись бы кадры {lost}, поэтому файл не записан")
    ))
}

/// The frames with something in them that the file's first ID3v2 tag
/// holds (`raw_frames`) and lofty's `tag` lacks.
fn lost_frames(path: &Path, tag: &Id3v2Tag) -> Vec<String> {
    let Some(raw) = raw_frames(path) else {
        return Vec::new();
    };
    let read: std::collections::HashSet<&str> = tag.iter().map(|f| f.id_str()).collect();
    let mut lost: Vec<String> = raw
        .into_iter()
        .filter(|id| !read.contains(id.as_str()) && !CONVERTED.contains(&id.as_str()))
        .collect();
    lost.sort();
    lost.dedup();
    lost
}

/// The IDs of the frames of more than 8 bytes in the ID3v2.3 or 2.4 tag at
/// the file's start, walked by their sizes as the bytes give them (2.4's
/// syncsafe, or plain as iTunes wrote them when only those land on the
/// next frame); `None` without such a tag, or one unsynchronised or not
/// walked to its end so.
fn raw_frames(path: &Path) -> Option<Vec<String>> {
    use std::io::Read;
    let syncsafe = |b: &[u8]| b.iter().fold(0usize, |n, &c| n << 7 | usize::from(c & 0x7f));
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 10];
    file.read_exact(&mut head).ok()?;
    if &head[..3] != b"ID3" || !(3..=4).contains(&head[3]) || head[5] & 0x80 != 0 {
        return None;
    }
    let v4 = head[3] == 4;
    let mut tag = vec![0; syncsafe(&head[6..10])];
    file.read_exact(&mut tag).ok()?;
    let is_id = |at: usize| {
        tag.get(at..at + 4)
            .is_some_and(|b| b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
    };
    // A frame ends at the tag's end, the padding or the next frame.
    let fits = |next: usize| next == tag.len() || tag.get(next) == Some(&0) || is_id(next);
    let mut at = 0;
    if head[5] & 0x40 != 0 {
        let size = tag.get(0..4)?;
        at = if v4 {
            syncsafe(size)
        } else {
            u32::from_be_bytes(size.try_into().ok()?) as usize + 4
        };
    }
    let mut ids = Vec::new();
    while at + 10 <= tag.len() && is_id(at) {
        let bytes = &tag[at + 4..at + 8];
        let plain = u32::from_be_bytes(bytes.try_into().ok()?) as usize;
        let size = if v4 && fits(at + 10 + syncsafe(bytes)) {
            syncsafe(bytes)
        } else if fits(at + 10 + plain) {
            plain
        } else {
            return None;
        };
        if size > 8 {
            ids.push(String::from_utf8_lossy(&tag[at..at + 4]).into_owned());
        }
        at += 10 + size;
    }
    Some(ids)
}

/// `tag` with its languages mended before lofty splits it
/// (`mend_languages`).
fn mended(mut tag: Id3v2Tag) -> Id3v2Tag {
    mend_languages(&mut tag);
    tag
}

/// The track and disc numbers' frames (TRCK, TPOS) as the tag has them.
fn numbers_read(tag: &Id3v2Tag) -> Vec<(&'static str, String)> {
    ["TRCK", "TPOS"]
        .into_iter()
        .filter_map(|id| {
            let text = tag.get_text(&FrameId::Valid(std::borrow::Cow::Borrowed(id)))?;
            Some((id, text.to_owned()))
        })
        .collect()
}

/// lofty writes an ID3v2 tag's track and disc numbers from numbers
/// (`3/12`): the frames written are put back as typed (`03/12`), and
/// those not changed as they were read.
fn keep_numbers(tag: &mut Id3v2Tag, changes: &Changes, read: &[(&'static str, String)]) {
    for (key, id) in [(Key::Track, "TRCK"), (Key::Disc, "TPOS")] {
        let frame_id = FrameId::Valid(std::borrow::Cow::Borrowed(id));
        let Some(value) = &changes.main[key as usize] else {
            // Not changed: as it was read (lofty wrote `01` as `1`).
            if let Some((_, text)) = read.iter().find(|(i, _)| *i == id)
                && tag.get_text(&frame_id).is_some()
            {
                tag.insert(Frame::Text(TextInformationFrame::new(
                    frame_id,
                    TextEncoding::Latin1,
                    text.clone(),
                )));
            }
            continue;
        };
        let Ok(Some((number, total))) = number_text(key, value) else {
            continue;
        };
        let text = match total {
            Some(total) => format!("{number}/{total}"),
            None => number,
        };
        let id = FrameId::Valid(std::borrow::Cow::Borrowed(id));
        tag.insert(Frame::Text(TextInformationFrame::new(
            id,
            TextEncoding::Latin1,
            text,
        )));
    }
}

/// An error with its causes: lofty's errors say only "failed to write to
/// file" and keep the reason (an I/O error, os error 32) as their source.
fn described(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(e) = cause {
        let more = e.to_string();
        if !text.contains(&more) {
            text = format!("{text}: {more}");
        }
        cause = e.source();
    }
    text
}

/// Windows' "used by another process" (a sharing violation, os error
/// 32) said plainly: the player lets go of its own files while they are
/// written (`audio::Releaser`), so another program holds it.
fn in_use(error: &str) -> String {
    if error.contains("os error 32") {
        tr!(
            "the file is in use by another program",
            "файл занят другой программой"
        )
        .to_owned()
    } else {
        error.to_owned()
    }
}

/// Why a file with the read-only attribute is not written.
pub fn read_only() -> String {
    tr!("the file is read-only", "файл только для чтения").to_owned()
}

/// The file's ID3v2 tag is of version 2.3 (most taggers' default).
#[cfg(test)]
fn is_id3v23(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|()| &head[..3] == b"ID3" && head[3] == 3)
}

fn values(tag: &Tag) -> [String; MAIN] {
    let text =
        |s: Option<std::borrow::Cow<'_, str>>| s.map(|s| s.trim().to_owned()).unwrap_or_default();
    let item = |key: ItemKey| {
        tag.get_string(key)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    // As the tag has them (`03/12`), as the file info shows them.
    let numbered = |number: ItemKey, total: ItemKey| {
        let (n, t) = (item(number), item(total));
        if n.is_empty() || t.is_empty() || n.contains('/') {
            n
        } else {
            format!("{n}/{t}")
        }
    };
    Key::ALL.map(|k| match k {
        Key::Title => text(tag.title()),
        Key::Artist => text(tag.artist()),
        Key::AlbumArtist => item(ItemKey::AlbumArtist),
        Key::Composer => item(ItemKey::Composer),
        Key::Album => text(tag.album()),
        Key::Year => match item(ItemKey::Year) {
            y if y.is_empty() => item(ItemKey::RecordingDate),
            y => y,
        },
        Key::Track => numbered(ItemKey::TrackNumber, ItemKey::TrackTotal),
        Key::Disc => numbered(ItemKey::DiscNumber, ItemKey::DiscTotal),
        Key::Genre => text(tag.genre()),
        Key::Comment => text(tag.comment()),
    })
}

/// A number field's value as it is written: the number and its total as
/// typed (`03`, `03/12`; spaces left out), the year as a date (`1987`,
/// `1987-05-01`); none when empty; an error naming the field otherwise.
fn number_text(key: Key, value: &str) -> Result<Option<(String, Option<String>)>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let bad = || {
        tr!(
            format!("{}: not a number: {value}", key.label()),
            format!("{}: не число: {value}", key.label())
        )
    };
    if key == Key::Year {
        // `1987`, `1987-05`, `1987-05-01`, `1987-05-01T12:30:00`: four
        // digits, then two a part (lofty's parse would take `19871` as
        // 1987-01).
        let parts: Vec<&str> = value.split(['-', 'T', ':']).collect();
        let shaped = parts.len() <= 6
            && parts.iter().enumerate().all(|(i, p)| {
                p.len() == if i == 0 { 4 } else { 2 } && p.bytes().all(|b| b.is_ascii_digit())
            });
        let date = shaped.then(|| value.parse::<Timestamp>().ok()).flatten();
        let in_range = |d: &Timestamp| {
            d.year >= 1
                && d.month.is_none_or(|m| (1..=12).contains(&m))
                && d.day.is_none_or(|d| (1..=31).contains(&d))
                && d.hour.is_none_or(|h| h < 24)
                && d.minute.is_none_or(|m| m < 60)
                && d.second.is_none_or(|s| s < 60)
        };
        return match date {
            Some(d) if in_range(&d) => Ok(Some((value.to_owned(), None))),
            _ => Err(bad()),
        };
    }
    let number = |s: &str| {
        let s = s.trim();
        s.parse::<u32>().map(|_| s.to_owned()).map_err(|_| bad())
    };
    let (n, total) = value.split_once('/').unwrap_or((value, ""));
    let total = match total.trim() {
        "" => None,
        t => Some(number(t)?),
    };
    Ok(Some((number(n)?, total)))
}

fn apply(tag: &mut Tag, changes: &Changes) -> Result<(), String> {
    if let Some(keep) = &changes.keep {
        strip_items(tag, keep);
    }
    for (key, value) in &changes.other {
        put_other(tag, *key, value);
    }
    for (&key, change) in Key::ALL.iter().zip(&changes.main) {
        let Some(value) = change else { continue };
        let value = value.trim().to_owned();
        let empty = value.is_empty();
        match key {
            Key::Title if empty => tag.remove_title(),
            Key::Title => tag.set_title(value),
            Key::Artist if empty => tag.remove_artist(),
            Key::Artist => tag.set_artist(value),
            Key::AlbumArtist | Key::Composer => {
                let item = if key == Key::Composer {
                    ItemKey::Composer
                } else {
                    ItemKey::AlbumArtist
                };
                if empty {
                    tag.remove_key(item);
                } else {
                    tag.insert_text(item, value);
                }
            }
            Key::Album if empty => tag.remove_album(),
            Key::Album => tag.set_album(value),
            Key::Genre if empty => tag.remove_genre(),
            Key::Genre => tag.set_genre(value),
            Key::Comment if empty => tag.remove_comment(),
            Key::Comment => tag.set_comment(value),
            Key::Year => match number_text(key, &value)? {
                None => tag.remove_date(),
                Some((date, _)) => {
                    if let Ok(date) = date.parse::<Timestamp>() {
                        tag.set_date(date);
                    }
                }
            },
            // As typed: Vorbis comments keep `03`; ID3v2's frames are put
            // back so (`keep_numbers`); MP4 holds numbers.
            Key::Track | Key::Disc => {
                let (number_key, total_key) = if key == Key::Track {
                    tag.remove_track();
                    tag.remove_track_total();
                    (ItemKey::TrackNumber, ItemKey::TrackTotal)
                } else {
                    tag.remove_disk();
                    tag.remove_disk_total();
                    (ItemKey::DiscNumber, ItemKey::DiscTotal)
                };
                if let Some((n, total)) = number_text(key, &value)? {
                    tag.insert_text(number_key, n);
                    if let Some(t) = total {
                        tag.insert_text(total_key, t);
                    }
                }
            }
        }
    }
    if let Some(lyrics) = &changes.lyrics {
        put_lyrics(tag, lyrics);
    }
    if let Some(change) = &changes.cover {
        put_cover(tag, change);
    }
    Ok(())
}

/// The cover changed as `change` says, the new one first.
fn put_cover(tag: &mut Tag, change: &CoverChange) {
    let mut pictures: Vec<Picture> = Vec::new();
    while !tag.pictures().is_empty() {
        pictures.push(tag.remove_picture(0));
    }
    if pictures
        .iter()
        .any(|p| p.pic_type() == PictureType::CoverFront)
    {
        pictures.retain(|p| p.pic_type() != PictureType::CoverFront);
    } else if !pictures.is_empty() {
        pictures.remove(0);
    }
    if let CoverChange::Set(data) = change {
        let mime = match image::guess_format(data) {
            Ok(image::ImageFormat::Png) => MimeType::Png,
            _ => MimeType::Jpeg,
        };
        let cover = Picture::unchecked(data.to_vec())
            .pic_type(PictureType::CoverFront)
            .mime_type(mime)
            .build();
        pictures.insert(0, cover);
    }
    for p in pictures {
        tag.push_picture(p);
    }
}

/// Puts `value` as `key`'s values (split at "; ", as the window joined
/// them), of the kind the tag had for it (a URL frame's link stays one);
/// an empty value removes the key.
fn put_other(tag: &mut Tag, key: ItemKey, value: &str) {
    let link = tag
        .get(key)
        .is_some_and(|i| matches!(i.value(), ItemValue::Locator(_)));
    tag.remove_key(key);
    for part in value.split("; ").map(str::trim).filter(|p| !p.is_empty()) {
        let value = if link {
            ItemValue::Locator(part.to_owned())
        } else {
            ItemValue::Text(part.to_owned())
        };
        tag.push(TagItem::new(key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        let n = |s: &str| s.to_owned();
        assert_eq!(number_text(Key::Track, " 03 "), Ok(Some((n("03"), None))));
        assert_eq!(
            number_text(Key::Track, "03 / 12"),
            Ok(Some((n("03"), Some(n("12")))))
        );
        assert_eq!(number_text(Key::Disc, ""), Ok(None));
        assert!(number_text(Key::Track, "three").is_err());
        assert!(number_text(Key::Track, "3/x").is_err());
        assert!(number_text(Key::Year, "0").is_err());
        assert!(number_text(Key::Year, "nineteen").is_err());
        assert_eq!(number_text(Key::Year, "1999"), Ok(Some((n("1999"), None))));
        assert!(Key::Year.takes("1987-05-01"));
        assert!(Key::Year.takes("1987-05-01T12:30:00"));
        assert!(!Key::Year.takes("19871"));
        assert!(!Key::Year.takes("1987-5"));
        assert!(!Key::Year.takes("1987-13"));
    }

    /// `key as usize` is the key's place in `ALL`, which the window and
    /// `Changes::main` index by.
    #[test]
    fn keys_at_their_places() {
        assert!(Key::ALL.iter().enumerate().all(|(i, k)| *k as usize == i));
    }

    /// A FLAC of no samples with a front cover (a PNG's signature and some
    /// bytes).
    fn flac_with_cover() -> Vec<u8> {
        let block = |kind: u8, last: bool, body: &[u8]| {
            let len = body.len() as u32;
            let mut b = vec![kind | if last { 0x80 } else { 0 }];
            b.extend_from_slice(&len.to_be_bytes()[1..]);
            b.extend_from_slice(body);
            b
        };
        // STREAMINFO: blocks of 4096, 44.1 kHz, 2 channels, 16 bits, no
        // samples, no MD5.
        let mut info = vec![0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0];
        info.extend_from_slice(&[0x0A, 0xC4, 0x42, 0xF0, 0, 0, 0, 0]);
        info.extend_from_slice(&[0; 16]);
        let png = b"\x89PNG\r\n\x1a\n not really an image";
        let mut picture = Vec::new();
        picture.extend_from_slice(&3u32.to_be_bytes());
        picture.extend_from_slice(&9u32.to_be_bytes());
        picture.extend_from_slice(b"image/png");
        picture.extend_from_slice(&0u32.to_be_bytes());
        for n in [1u32, 1, 24, 0] {
            picture.extend_from_slice(&n.to_be_bytes());
        }
        picture.extend_from_slice(&(png.len() as u32).to_be_bytes());
        picture.extend_from_slice(png);
        let mut bytes = b"fLaC".to_vec();
        bytes.extend(block(0, false, &info));
        bytes.extend(block(6, true, &picture));
        bytes
    }

    /// A FLAC with a cover keeps it when a field is written (lofty writes
    /// only the pictures of the tag it is given).
    #[test]
    fn flac_keeps_its_cover() {
        let dir = std::env::temp_dir().join(format!("metalamp_flac_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cover.flac");
        std::fs::write(&file, flac_with_cover()).unwrap();
        let pictures = |path: &Path| {
            let mut f = std::fs::File::open(path).unwrap();
            let flac = lofty::flac::FlacFile::read_from(&mut f, ParseOptions::new()).unwrap();
            flac.pictures().len()
        };
        assert_eq!(pictures(&file), 1);
        let mut changes: Changes = Default::default();
        changes.main[Key::Genre as usize] = Some("Metal".into());
        changes.main[Key::Track as usize] = Some("03/12".into());
        write(&file, &changes, WriteTags::default()).unwrap();
        let main = read(&file, Id3Version::default()).unwrap().main;
        assert_eq!(main[Key::Genre as usize], "Metal");
        // lofty reads a FLAC's numbers as numbers; the comment keeps them
        // as typed.
        assert_eq!(main[Key::Track as usize], "3/12");
        let mut f = std::fs::File::open(&file).unwrap();
        let raw = ParseOptions::new().implicit_conversions(false);
        let flac = lofty::flac::FlacFile::read_from(&mut f, raw).unwrap();
        let comments = flac.vorbis_comments().unwrap();
        assert_eq!(comments.get("TRACKNUMBER"), Some("03"));
        assert_eq!(comments.get("TRACKTOTAL"), Some("12"));
        drop(f);
        assert_eq!(pictures(&file), 1, "the cover is lost");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn picture(kind: PictureType, data: &[u8]) -> Picture {
        Picture::unchecked(data.to_vec())
            .pic_type(kind)
            .mime_type(MimeType::Jpeg)
            .build()
    }

    /// The cover is the front covers, else the first picture: those are
    /// replaced (the new one first) or removed, the others kept.
    #[test]
    fn cover_replaced_or_removed() {
        let set = CoverChange::Set(std::sync::Arc::new(b"\xFF\xD8new".to_vec()));
        let kinds = |tag: &Tag| -> Vec<(PictureType, Vec<u8>)> {
            tag.pictures()
                .iter()
                .map(|p| (p.pic_type(), p.data().to_vec()))
                .collect()
        };
        // No front cover: the first picture is the cover.
        let mut tag = Tag::new(lofty::tag::TagType::Id3v2);
        tag.push_picture(picture(PictureType::Other, b"other"));
        tag.push_picture(picture(PictureType::CoverBack, b"back"));
        put_cover(&mut tag, &set);
        assert_eq!(
            kinds(&tag),
            [
                (PictureType::CoverFront, b"\xFF\xD8new".to_vec()),
                (PictureType::CoverBack, b"back".to_vec())
            ]
        );
        // Front covers wherever they are.
        let mut tag = Tag::new(lofty::tag::TagType::Id3v2);
        tag.push_picture(picture(PictureType::Artist, b"artist"));
        tag.push_picture(picture(PictureType::CoverFront, b"front"));
        put_cover(&mut tag, &CoverChange::Remove);
        assert_eq!(kinds(&tag), [(PictureType::Artist, b"artist".to_vec())]);
        let mut tag = Tag::new(lofty::tag::TagType::Id3v2);
        put_cover(&mut tag, &CoverChange::Remove);
        assert!(tag.pictures().is_empty());
    }

    /// A tiny JPEG, a real one.
    fn jpeg() -> Vec<u8> {
        let picture = image::DynamicImage::new_rgb8(4, 4);
        let mut out = Vec::new();
        picture
            .write_to(
                &mut std::io::Cursor::new(&mut out),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        out
    }

    /// A FLAC's cover replaced and removed: its PICTURE blocks, the
    /// blocks' lengths still leading to the audio after them.
    #[test]
    fn flac_cover_written() {
        let dir = std::env::temp_dir().join(format!("metalamp_flac_cover_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cover.flac");
        let audio = b"\xFF\xF8 the frames that follow".repeat(40);
        let mut bytes = flac_with_cover();
        bytes.extend_from_slice(&audio);
        std::fs::write(&file, bytes).unwrap();
        // The blocks (type, length) by their headers, and what follows.
        let blocks = |path: &Path| {
            let bytes = std::fs::read(path).unwrap();
            let mut at = 4;
            let mut found = Vec::new();
            loop {
                let head = &bytes[at..at + 4];
                let len = u32::from_be_bytes([0, head[1], head[2], head[3]]) as usize;
                found.push(head[0] & 0x7F);
                at += 4 + len;
                if head[0] & 0x80 != 0 {
                    return (found, bytes[at..].to_vec());
                }
            }
        };
        let pictures = |path: &Path| {
            let mut f = std::fs::File::open(path).unwrap();
            let options = ParseOptions::new().read_properties(false);
            let flac = lofty::flac::FlacFile::read_from(&mut f, options).unwrap();
            flac.pictures()
                .iter()
                .map(|(p, _)| (p.pic_type(), p.data().to_vec()))
                .collect::<Vec<_>>()
        };
        let data = jpeg();
        let changes = Changes {
            cover: Some(CoverChange::Set(std::sync::Arc::new(data.clone()))),
            ..Default::default()
        };
        write(&file, &changes, WriteTags::default()).unwrap();
        let (kinds, after) = blocks(&file);
        assert_eq!(after, audio, "{kinds:?}");
        assert_eq!(pictures(&file), [(PictureType::CoverFront, data)]);
        let changes = Changes {
            cover: Some(CoverChange::Remove),
            ..Default::default()
        };
        write(&file, &changes, WriteTags::default()).unwrap();
        let (kinds, after) = blocks(&file);
        assert_eq!(after, audio, "{kinds:?}");
        assert!(!kinds.contains(&6), "{kinds:?}");
        assert!(pictures(&file).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file held open for reading only (the track playing) is named in
    /// use: lofty's error keeps the sharing violation as its source.
    #[test]
    fn a_file_in_use_is_said_so() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("metalamp_in_use_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("held.flac");
        std::fs::write(&file, flac_with_cover()).unwrap();
        // FILE_SHARE_READ, as Media Foundation opens a file it plays.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&file)
            .unwrap();
        let mut changes: Changes = Default::default();
        changes.main[Key::Genre as usize] = Some("Metal".into());
        assert_eq!(
            write(&file, &changes, WriteTags::default()),
            Err(in_use("os error 32"))
        );
        drop(held);
        write(&file, &changes, WriteTags::default()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A read-only file is not written, and said so; its bytes stay.
    #[test]
    fn a_read_only_file_is_said_so() {
        let dir = std::env::temp_dir().join(format!("metalamp_read_only_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("kept.flac");
        let bytes = flac_with_cover();
        std::fs::write(&file, &bytes).unwrap();
        let set = |read_only: bool| {
            let mut p = std::fs::metadata(&file).unwrap().permissions();
            p.set_readonly(read_only);
            std::fs::set_permissions(&file, p).unwrap();
        };
        set(true);
        let mut changes: Changes = Default::default();
        changes.main[Key::Genre as usize] = Some("Metal".into());
        changes.lyrics = Some("[00:01.00]words".into());
        let result = write(&file, &changes, WriteTags::default());
        let after = std::fs::read(&file).unwrap();
        set(false);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result, Err(read_only()));
        assert_eq!(after, bytes);
    }

    /// An MP3 without ID3v2 gets one with the cover (APIC) and a field.
    #[test]
    fn mp3_cover_written() {
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
            bytes.extend_from_slice(&[0; 413]);
        }
        let dir = std::env::temp_dir().join(format!("metalamp_mp3_cover_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("plain.mp3");
        std::fs::write(&file, &bytes).unwrap();
        let data = jpeg();
        let mut changes = Changes {
            cover: Some(CoverChange::Set(std::sync::Arc::new(data.clone()))),
            ..Default::default()
        };
        changes.main[Key::Album as usize] = Some("Album".into());
        write(&file, &changes, WriteTags::default()).unwrap();
        let mut f = std::fs::File::open(&file).unwrap();
        let mp3 = lofty::mpeg::MpegFile::read_from(&mut f, ParseOptions::new()).unwrap();
        let tag = mp3.id3v2().unwrap();
        let apic: Vec<_> = tag
            .into_iter()
            .filter_map(|frame| match frame {
                Frame::Picture(p) => Some((p.picture.pic_type(), p.picture.data().to_vec())),
                _ => None,
            })
            .collect();
        assert_eq!(apic, [(PictureType::CoverFront, data)]);
        assert_eq!(tag.album().as_deref(), Some("Album"));
        drop(f);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An MP3 of an ID3v2.3 tag of `frames` (ID, body) and a few frames of
    /// silence.
    fn mp3_with(frames: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut tag = Vec::new();
        for (id, body) in frames {
            tag.extend_from_slice(&id[..]);
            tag.extend_from_slice(&(body.len() as u32).to_be_bytes());
            tag.extend_from_slice(&[0, 0]);
            tag.extend_from_slice(body);
        }
        let size = tag.len() as u32;
        let mut bytes = b"ID3\x03\x00\x00".to_vec();
        bytes.extend_from_slice(&[
            (size >> 21 & 0x7f) as u8,
            (size >> 14 & 0x7f) as u8,
            (size >> 7 & 0x7f) as u8,
            (size & 0x7f) as u8,
        ]);
        bytes.extend_from_slice(&tag);
        for _ in 0..4 {
            bytes.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
            bytes.extend_from_slice(&[0; 413]);
        }
        bytes
    }

    /// An MP3 whose comments MusicMatch wrote (no language, `\0\0\0`; the
    /// content `\0Bio\0` under the description "Bio") is written: the
    /// comments kept as they were, their language "unknown".
    #[test]
    fn a_comment_without_a_language_is_written() {
        // COMM: Latin-1, language 0 0 0, an empty description, "Hello".
        let plain = [&[0u8, 0, 0, 0, 0][..], b"Hello"].concat();
        let described = [&[0u8, 0, 0, 0][..], b"Bio\0\0Bio\0"].concat();
        let bytes = mp3_with(&[(b"COMM", plain), (b"COMM", described)]);
        let dir = std::env::temp_dir().join(format!("metalamp_comm_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("comment.mp3");
        std::fs::write(&file, &bytes).unwrap();
        let mut changes: Changes = Default::default();
        changes.main[Key::Genre as usize] = Some("Metal".into());
        changes.lyrics = Some("[00:01.00]words".into());
        let written = write(&file, &changes, WriteTags::default());
        let mut f = std::fs::File::open(&file).unwrap();
        let mp3 = lofty::mpeg::MpegFile::read_from(
            &mut f,
            ParseOptions::new().parsing_mode(lofty::config::ParsingMode::Strict),
        )
        .unwrap();
        drop(f);
        std::fs::remove_dir_all(&dir).unwrap();
        written.unwrap();
        let tag = mp3.id3v2().unwrap();
        // Every COMM frame (`comments()` gives those without a description).
        let mut comments: Vec<_> = tag
            .iter()
            .filter_map(|frame| match frame {
                Frame::Comment(c) => Some((
                    c.language,
                    c.description.to_string(),
                    c.content.to_string(),
                )),
                _ => None,
            })
            .collect();
        comments.sort();
        assert_eq!(
            comments,
            [
                (*b"XXX", String::new(), "Hello".to_owned()),
                (*b"XXX", "Bio".to_owned(), "Bio".to_owned())
            ]
        );
        assert_eq!(tag.genre().as_deref(), Some("Metal"));
        assert_eq!(
            tag.unsync_text().next().map(|t| t.content.to_string()),
            Some("[00:01.00]words".to_owned())
        );
    }

    /// A tag lofty stops reading in the middle of without an error (NULs
    /// after a USLT's text taken for the padding, as in Aqua's Aquarium) is
    /// not written: the frames after them would be lost.
    #[test]
    fn a_tag_read_in_part_is_not_written() {
        // USLT: UTF-16, "eng", an empty description and text, then NULs.
        let uslt = [
            &[1u8][..],
            b"eng",
            &[0xFF, 0xFE, 0, 0, 0xFF, 0xFE, 0, 0, 0, 0],
        ]
        .concat();
        let private = [&b"owner\0"[..], &[7u8; 24]].concat();
        let bytes = mp3_with(&[
            (b"TIT2", [&[0u8][..], b"Title"].concat()),
            (b"USLT", uslt),
            (b"PRIV", private),
        ]);
        let dir = std::env::temp_dir().join(format!("metalamp_part_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("part.mp3");
        std::fs::write(&file, &bytes).unwrap();
        let raw = raw_frames(&file);
        let mut changes: Changes = Default::default();
        changes.main[Key::Genre as usize] = Some("Metal".into());
        let written = write(&file, &changes, WriteTags::default());
        let after = std::fs::read(&file).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(raw.unwrap(), ["USLT", "PRIV"]);
        let e = written.unwrap_err();
        assert!(e.contains("PRIV"), "{e}");
        assert_eq!(after, bytes);
    }

    /// With `METALAMP_MUSIC`: the MP3s of a folder whose tag lofty does not
    /// read whole (`lost_frames`), which are not written, and the frames.
    #[test]
    #[ignore]
    fn tags_read_in_part() {
        let root = std::env::var("METALAMP_MUSIC").expect("METALAMP_MUSIC");
        let (mut files, mut refused) = (0, 0);
        let mut frames: std::collections::BTreeMap<String, usize> = Default::default();
        for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
            {
                continue;
            }
            files += 1;
            let Ok((view, _)) = crate::library::tags::first_tag_view(path) else {
                continue;
            };
            let mut reader = BufReader::new(view);
            let options = ParseOptions::new().read_properties(false);
            let Ok(file) = lofty::mpeg::MpegFile::read_from(&mut reader, options) else {
                continue;
            };
            let Some(tag) = file.id3v2() else {
                continue;
            };
            let lost = lost_frames(path, tag);
            if !lost.is_empty() {
                refused += 1;
                if refused <= 10 {
                    println!("{}: {lost:?}", path.display());
                }
                for id in lost {
                    *frames.entry(id).or_default() += 1;
                }
            }
        }
        println!("{refused} of {files} not read whole; frames {frames:?}");
    }

    /// An MP3 with only an ID3v1 tag shows it, and keeps what it says when
    /// one field is written (an ID3v2 tag of that field alone became the
    /// tag the library read).
    #[test]
    fn id3v1_alone_is_kept() {
        // Frames of silence: MPEG-1 layer III, 128 kbit/s, 44.1 kHz.
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
            bytes.extend_from_slice(&[0; 413]);
        }
        let field = |s: &str, n: usize| {
            let mut f = s.as_bytes().to_vec();
            f.resize(n, 0);
            f
        };
        bytes.extend_from_slice(b"TAG");
        bytes.extend(field("Old title", 30));
        bytes.extend(field("Old artist", 30));
        bytes.extend(field("Old album", 30));
        bytes.extend(field("1999", 4));
        bytes.extend(field("", 30));
        bytes.push(9); // Metal
        let dir = std::env::temp_dir().join(format!("metalamp_id3v1_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("old.mp3");
        std::fs::write(&file, &bytes).unwrap();
        let before = read(&file, Id3Version::default()).unwrap().main;
        assert_eq!(before[Key::Title as usize], "Old title");
        assert_eq!(before[Key::Album as usize], "Old album");
        let mut changes: Changes = Default::default();
        changes.main[Key::AlbumArtist as usize] = Some("Old band".into());
        write(&file, &changes, WriteTags::default()).unwrap();
        let after = read(&file, Id3Version::default()).unwrap().main;
        assert_eq!(after[Key::AlbumArtist as usize], "Old band");
        assert_eq!(after[Key::Title as usize], "Old title");
        assert_eq!(after[Key::Artist as usize], "Old artist");
        assert_eq!(after[Key::Album as usize], "Old album");
        assert_eq!(after[Key::Year as usize], "1999");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An MP3 with only an ID3v1 tag in the ANSI code page (Windows-1251 on
    /// a Russian Windows, as `id3v1::write` writes it): the library, the
    /// file info and its editing read it in that code page, not as Latin-1.
    #[test]
    fn id3v1_is_read_in_the_ansi_code_page() {
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
            bytes.extend_from_slice(&[0; 413]);
        }
        // Cyrillic where the code page has it, else Western letters.
        let (title, artist) = if unsafe { windows_sys::Win32::Globalization::GetACP() } == 1251 {
            ("Беспечный ангел", "Ария")
        } else {
            ("Café", "Motörhead")
        };
        bytes.extend_from_slice(&super::super::id3v1::block(&super::super::id3v1::Fields {
            title,
            artist,
            album: "Album",
            year: "1997",
            genre: "Metal",
            ..Default::default()
        }));
        let dir = std::env::temp_dir().join(format!("metalamp_id3v1_ansi_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ansi.mp3");
        std::fs::write(&file, &bytes).unwrap();
        let track = super::super::tags::read(&file, bytes.len() as u64, 0).unwrap();
        assert_eq!((track.title.as_str(), track.artist.as_str()), (title, artist));
        assert_eq!((track.album.as_str(), track.genre.as_str()), ("Album", "Metal"));
        let details = super::super::details::read(&file).unwrap();
        assert!(
            details.fields.iter().any(|(_, value)| value == title),
            "{:?}",
            details.fields
        );
        let values = read(&file, Id3Version::default()).unwrap().main;
        assert_eq!(values[Key::Title as usize], title);
        assert_eq!(values[Key::Artist as usize], artist);
        // An ID3v2 seeded from it by a change keeps the letters.
        let mut changes: Changes = Default::default();
        changes.main[Key::AlbumArtist as usize] = Some(artist.into());
        write(&file, &changes, WriteTags::default()).unwrap();
        let mut f = std::fs::File::open(&file).unwrap();
        let mp3 = lofty::mpeg::MpegFile::read_from(&mut f, ParseOptions::new()).unwrap();
        assert_eq!(mp3.id3v2().unwrap().title().as_deref(), Some(title));
        drop(f);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An MP3 with APE and ID3v1 tags and no ID3v2: by default its new
    /// ID3v2 is 2.3 and the other two get the field changed (the APE's
    /// ReplayGain kept); then 2.4, and the other two removed.
    #[test]
    fn how_an_mp3s_tags_are_written() {
        use lofty::ape::{ApeItem, ApeTag as Ape};
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
            bytes.extend_from_slice(&[0; 413]);
        }
        let dir = std::env::temp_dir().join(format!("metalamp_mp3_tags_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tags.mp3");
        std::fs::write(&file, &bytes).unwrap();
        let mut ape = Ape::default();
        ape.set_title("Old title".into());
        ape.insert(
            ApeItem::new(
                "REPLAYGAIN_TRACK_GAIN".into(),
                ItemValue::Text("-7.5 dB".into()),
            )
            .unwrap(),
        );
        ape.save_to_path(&file, WriteOptions::default()).unwrap();
        super::super::id3v1::write(
            &file,
            &super::super::id3v1::Fields {
                title: "Old title",
                artist: "Band",
                ..Default::default()
            },
        )
        .unwrap();
        let mut changes: Changes = Default::default();
        changes.main[Key::Title as usize] = Some("New title".into());
        write(&file, &changes, WriteTags::default()).unwrap();
        let written = std::fs::read(&file).unwrap();
        assert_eq!(&written[..4], b"ID3\x03");
        let tail = &written[written.len() - 128..];
        assert_eq!(&tail[..12], b"TAGNew title");
        assert_eq!(&tail[33..37], b"Band");
        let read_ape = || {
            let mut f = std::fs::File::open(&file).unwrap();
            lofty::mpeg::MpegFile::read_from(&mut f, ParseOptions::new())
                .unwrap()
                .ape()
                .cloned()
        };
        let ape = read_ape().unwrap();
        assert_eq!(ape.title().as_deref(), Some("New title"));
        assert!(ape.get("REPLAYGAIN_TRACK_GAIN").is_some());

        let how = WriteTags {
            id3v2: Id3Version::V24,
            id3v1: Id3v1Tag::Remove,
            ape: ApeTag::Remove,
        };
        changes.main[Key::Title as usize] = Some("Newer title".into());
        write(&file, &changes, how).unwrap();
        let written = std::fs::read(&file).unwrap();
        assert_eq!(&written[..4], b"ID3\x04");
        assert_ne!(&written[written.len() - 128..written.len() - 125], b"TAG");
        assert!(read_ape().is_none());
        assert_eq!(
            read(&file, Id3Version::default()).unwrap().main[Key::Title as usize],
            "Newer title"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_version_written() {
        let file23 = Id3Version::File23;
        assert!(file23.writes_23(None) && file23.writes_23(Some(3)) && file23.writes_23(Some(2)));
        assert!(!file23.writes_23(Some(4)));
        let file24 = Id3Version::File24;
        assert!(!file24.writes_23(None) && file24.writes_23(Some(3)) && !file24.writes_23(Some(4)));
        assert!(Id3Version::V23.writes_23(Some(4)) && !Id3Version::V24.writes_23(Some(3)));
    }

    /// Every field offered to be added, written into an MP3's ID3v2 (2.3
    /// and 2.4) and a FLAC's comments, is read back as that field with
    /// that value, and is no longer offered; 2.3 is not offered what it
    /// cannot hold.
    #[test]
    fn the_fields_added_are_read_back() {
        let value = |key: ItemKey| match key {
            ItemKey::OriginalReleaseDate | ItemKey::ReleaseDate => "2001".to_owned(),
            ItemKey::Bpm | ItemKey::IntegerBpm => "120".to_owned(),
            k if format!("{k:?}").ends_with("Url") => format!("http://example.com/{k:?}"),
            k => format!("{k:?} value"),
        };
        let dir = std::env::temp_dir().join(format!("metalamp_added_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mp3 = Vec::new();
        for _ in 0..4 {
            mp3.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
            mp3.extend_from_slice(&[0; 413]);
        }
        let cases = [
            ("v23.mp3", mp3.clone(), Id3Version::V23),
            ("v24.mp3", mp3, Id3Version::V24),
            ("comments.flac", flac_with_cover(), Id3Version::default()),
        ];
        for (name, bytes, version) in cases {
            let file = dir.join(name);
            std::fs::write(&file, bytes).unwrap();
            let offered = read(&file, version).unwrap().standard;
            assert!(offered.len() >= 20, "{name}: {offered:?}");
            if version == Id3Version::V23 {
                assert!(!offered.iter().any(|(_, k)| *k == ItemKey::Mood));
            }
            let changes = Changes {
                other: offered.iter().map(|&(_, k)| (k, value(k))).collect(),
                ..Default::default()
            };
            let how = WriteTags {
                id3v2: version,
                ..Default::default()
            };
            write(&file, &changes, how).unwrap();
            let back = read(&file, version).unwrap();
            for &(_, key) in &offered {
                let found = back.other.iter().find(|(k, _)| *k == key);
                assert_eq!(
                    found.map(|(_, v)| v.as_str()),
                    Some(value(key).as_str()),
                    "{name}: {key:?} in {:?}",
                    back.other
                );
            }
            assert_eq!(back.standard, offered, "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Lyrics written into an MP3 (USLT) and a FLAC, read back with `\n`
    /// line ends; an MP3's language and description kept when changed, its
    /// lines ending as Windows' taggers end them; then removed.
    #[test]
    fn lyrics_written() {
        let dir = std::env::temp_dir().join(format!("metalamp_lyrics_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mp3 = Vec::new();
        for _ in 0..4 {
            mp3.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
            mp3.extend_from_slice(&[0; 413]);
        }
        let uslt = |path: &Path| {
            let mut f = std::fs::File::open(path).unwrap();
            let file = lofty::mpeg::MpegFile::read_from(&mut f, ParseOptions::new()).unwrap();
            file.id3v2()
                .unwrap()
                .into_iter()
                .filter_map(|frame| match frame {
                    Frame::UnsynchronizedText(u) => {
                        Some((u.language, u.description.to_string(), u.content.to_string()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let lyrics = |text: &str| Changes {
            lyrics: Some(text.to_owned()),
            ..Default::default()
        };
        let mut flac = flac_with_cover();
        flac.extend_from_slice(&b"\xFF\xF8 the frames that follow".repeat(40));
        for (name, bytes) in [("song.mp3", mp3), ("song.flac", flac)] {
            let file = dir.join(name);
            std::fs::write(&file, bytes).unwrap();
            assert_eq!(read(&file, Id3Version::default()).unwrap().lyrics, "");
            write(&file, &lyrics("First line\nSecond line\n"), WriteTags::default()).unwrap();
            assert_eq!(
                read(&file, Id3Version::default()).unwrap().lyrics,
                "First line\nSecond line"
            );
            if name.ends_with(".mp3") {
                let mut tag = Tag::new(TagType::Id3v2);
                let mut item =
                    TagItem::new(ItemKey::UnsyncLyrics, ItemValue::Text("Old".into()));
                item.set_lang(*b"rus");
                item.set_description("Слова".into());
                tag.push(item);
                tag.save_to_path(&file, WriteOptions::default()).unwrap();
                write(&file, &lyrics("Новая\nстрока"), WriteTags::default()).unwrap();
                assert_eq!(
                    uslt(&file),
                    [(*b"rus", "Слова".to_owned(), "Новая\r\nстрока".to_owned())]
                );
            }
            write(&file, &lyrics(""), WriteTags::default()).unwrap();
            assert_eq!(read(&file, Id3Version::default()).unwrap().lyrics, "");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The other fields: several values split at "; ", a link kept a
    /// link, an empty value removing the key; the main fields' keys are
    /// not among them.
    #[test]
    fn other_fields() {
        let mut tag = Tag::new(lofty::tag::TagType::Id3v2);
        tag.insert(TagItem::new(
            ItemKey::AudioFileUrl,
            ItemValue::Locator("http://old".into()),
        ));
        tag.insert_text(ItemKey::Publisher, "Old".into());
        tag.insert_text(ItemKey::Mood, "Calm".into());
        put_other(&mut tag, ItemKey::AudioFileUrl, "http://new");
        put_other(&mut tag, ItemKey::Publisher, "A; B ;  ");
        put_other(&mut tag, ItemKey::Mood, " ");
        assert!(matches!(
            tag.get(ItemKey::AudioFileUrl).map(TagItem::value),
            Some(ItemValue::Locator(s)) if s == "http://new"
        ));
        let publishers: Vec<_> = tag
            .get_strings(ItemKey::Publisher)
            .map(str::to_owned)
            .collect();
        assert_eq!(publishers, ["A", "B"]);
        assert!(tag.get(ItemKey::Mood).is_none());
        assert!(!is_other(ItemKey::RecordingDate));
        assert!(!is_other(ItemKey::TrackTotal));
        assert!(!is_other(ItemKey::UnsyncLyrics));
        assert!(is_other(ItemKey::Publisher));
    }

    #[test]
    fn writable_by_extension() {
        assert!(writable(Path::new("a.MP3")));
        assert!(writable(Path::new("a.flac")));
        assert!(!writable(Path::new("a.wma")));
        assert!(!writable(Path::new("a")));
    }

    /// A copy of `METALAMP_SAMPLE` (an MP3) with its tag changed, read
    /// back, and changed back: the fields written are read, the others
    /// stay, and the frames the editor does not know are kept.
    #[test]
    #[ignore]
    fn writes_a_copy_of_a_real_file() {
        let Some(sample) = std::env::var_os("METALAMP_SAMPLE") else {
            panic!("METALAMP_SAMPLE is not set");
        };
        let dir = std::env::temp_dir().join(format!("metalamp_edit_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("copy.mp3");
        std::fs::copy(&sample, &copy).unwrap();
        let before = read(&copy, Id3Version::default()).unwrap().main;
        let v23 = is_id3v23(&copy);
        let frames_before = id3_frames(&copy);
        let mut changes: Changes = Default::default();
        changes.main[Key::Title as usize] = Some("Edited title".into());
        changes.main[Key::AlbumArtist as usize] = Some("Various Artists".into());
        changes.main[Key::Track as usize] = Some("07/09".into());
        changes.other = vec![(ItemKey::Publisher, "Hansa; BMG".into())];
        let cover = jpeg();
        changes.cover = Some(CoverChange::Set(std::sync::Arc::new(cover.clone())));
        let audio = |path: &Path| {
            let bytes = std::fs::read(path).unwrap();
            // The tag's size, synchsafe.
            let size = bytes[6..10]
                .iter()
                .fold(0usize, |n, &b| n << 7 | usize::from(b));
            let start = if &bytes[..3] == b"ID3" { 10 + size } else { 0 };
            // lofty may leave padding: the audio starts at the first frame.
            let first = bytes[start..]
                .windows(2)
                .position(|w| w[0] == 0xFF && w[1] & 0xE0 == 0xE0)
                .unwrap();
            // The tags at the end (ID3v1, then an APE tag before it) are
            // written too.
            let mut end = bytes.len();
            if end >= 128 && &bytes[end - 128..end - 125] == b"TAG" {
                end -= 128;
            }
            if end >= 32 && &bytes[end - 32..end - 24] == b"APETAGEX" {
                let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                let (size, flags) = (word(end - 20) as usize, word(end - 12));
                end -= size + if flags & 0x8000_0000 != 0 { 32 } else { 0 };
            }
            bytes[start + first..end].to_vec()
        };
        let audio_before = audio(&copy);
        write(&copy, &changes, WriteTags::default()).unwrap();
        assert!(audio(&copy) == audio_before, "the audio changed");
        let mut f = std::fs::File::open(&copy).unwrap();
        let mp3 = lofty::mpeg::MpegFile::read_from(&mut f, ParseOptions::new()).unwrap();
        let front: Vec<Vec<u8>> = mp3
            .id3v2()
            .unwrap()
            .into_iter()
            .filter_map(|frame| match frame {
                Frame::Picture(p) if p.picture.pic_type() == PictureType::CoverFront => {
                    Some(p.picture.data().to_vec())
                }
                _ => None,
            })
            .collect();
        assert_eq!(front, [cover]);
        drop(f);
        let read_back = read(&copy, Id3Version::default()).unwrap();
        assert!(
            read_back
                .other
                .contains(&(ItemKey::Publisher, "Hansa; BMG".into())),
            "{:?}",
            read_back.other
        );
        let after = read_back.main;
        // Written in the version it had.
        assert_eq!(is_id3v23(&copy), v23);
        assert_eq!(after[Key::Title as usize], "Edited title");
        assert_eq!(after[Key::AlbumArtist as usize], "Various Artists");
        assert_eq!(after[Key::Track as usize], "07/09");
        assert_eq!(after[Key::Artist as usize], before[Key::Artist as usize]);
        assert_eq!(after[Key::Album as usize], before[Key::Album as usize]);
        // Every frame of the original is still there (TPE2 may be new).
        let frames_after = id3_frames(&copy);
        for f in &frames_before {
            assert!(frames_after.contains(f), "{f} lost: {frames_after:?}");
        }
        println!("before {before:?}\nafter {after:?}\nframes {frames_after:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn id3_frames(path: &Path) -> Vec<String> {
        let mut f = std::fs::File::open(path).unwrap();
        let file = lofty::mpeg::MpegFile::read_from(&mut f, ParseOptions::new()).unwrap();
        let mut ids: Vec<String> = file
            .id3v2()
            .map(|t| t.into_iter().map(|fr| fr.id_str().to_owned()).collect())
            .unwrap_or_default();
        ids.sort();
        ids.dedup();
        ids
    }

    /// A number not changed stays as it was written (`01/12`): lofty
    /// writes it from the number (`1/12`).
    #[test]
    fn numbers_not_changed_stay() {
        let text = |s: &str| [&[0u8][..], s.as_bytes()].concat();
        let bytes = mp3_with(&[(b"TRCK", text("01/12")), (b"TPOS", text("01"))]);
        let dir = std::env::temp_dir().join(format!("metalamp_trck_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("numbers.mp3");
        std::fs::write(&file, &bytes).unwrap();
        let mut changes: Changes = Default::default();
        changes.main[Key::Genre as usize] = Some("Metal".into());
        let written = write(&file, &changes, WriteTags::default());
        let values = read(&file, Id3Version::default());
        std::fs::remove_dir_all(&dir).unwrap();
        written.unwrap();
        let values = values.unwrap();
        assert_eq!(values.main[Key::Track as usize], "01/12");
        assert_eq!(values.main[Key::Disc as usize], "01");
    }

    /// The parts of a tag are listed by `inspect`, and a write keeping
    /// some removes the rest, PRIV (which lofty keeps apart) too.
    #[test]
    fn parts_listed_and_removed() {
        let text = |s: &str| [&[0u8][..], s.as_bytes()].concat();
        let bytes = mp3_with(&[
            (b"TIT2", text("Song")),
            (b"TRCK", text("1/16")),
            (b"COMM", [&[0u8][..], b"eng\0hi"].concat()),
            (b"TENC", text("Someone")),
            (b"PRIV", b"owner\0data".to_vec()),
            (b"TXXX", [&[0u8][..], b"REPLAYGAIN_TRACK_GAIN\0-6.5 dB"].concat()),
        ]);
        let dir = std::env::temp_dir().join(format!("metalamp_parts_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("parts.mp3");
        std::fs::write(&file, &bytes).unwrap();
        let inspected = inspect(&file);
        let mut changes = Changes {
            keep: Some(vec![TagPart::ReplayGain]),
            ..Default::default()
        };
        changes.main[Key::Track as usize] = Some("01".into());
        let written = write(&file, &changes, WriteTags::default());
        let frames = id3_frames(&file);
        let again = inspect(&file);
        std::fs::remove_dir_all(&dir).unwrap();
        let inspected = inspected.unwrap();
        let mut parts: Vec<TagPart> = inspected.parts.iter().map(|i| i.part).collect();
        parts.sort_by_key(|p| *p as u8);
        assert_eq!(
            parts,
            [TagPart::Comment, TagPart::ReplayGain, TagPart::Encoder, TagPart::Other]
        );
        let item = |code: &str| inspected.parts.iter().find(|i| i.code == code).unwrap();
        assert_eq!(item("COMM").key, Some(ItemKey::Comment));
        assert_eq!(item("COMM").value, "hi");
        assert_eq!(item("PRIV").key, None);
        let gain = item("TXXX:REPLAYGAIN_TRACK_GAIN");
        assert_eq!(gain.key, Some(ItemKey::ReplayGainTrackGain));
        assert_eq!(gain.value, "-6.5 dB");
        assert_eq!(inspected.main[Key::Track as usize], "1/16");
        written.unwrap();
        assert_eq!(frames, ["TIT2", "TRCK", "TXXX"]);
        assert_eq!(again.unwrap().main[Key::Track as usize], "01");
    }
}
