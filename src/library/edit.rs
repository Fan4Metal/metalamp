//! Writing a track's tag (the file info window's editing, `ui::info`):
//! the main fields, and the other text fields the tag already has. The
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
}

/// What to write.
#[derive(Clone, Debug, Default)]
pub struct Changes {
    /// For each main field, the value to put (empty: the field removed),
    /// or `None` to leave it as it is.
    pub main: [Option<String>; MAIN],
    /// Other fields to put (values split at "; "; empty: removed).
    pub other: Vec<(ItemKey, String)>,
    /// The cover put or removed, or `None` to leave it as it is.
    pub cover: Option<CoverChange>,
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

/// The values of `path`'s tag (all empty when it has none).
pub fn read(path: &Path) -> Result<Values, String> {
    with_tag(
        path,
        |tag| {
            Ok(Values {
                main: values(tag),
                other: super::details::fields_of(tag)
                    .into_iter()
                    .filter(|(k, _)| is_other(*k))
                    .collect(),
            })
        },
        None,
        Id3Version::default(),
    )
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
    with_tag(path, |tag| apply(tag, changes), Some(changes), how.id3v2)?;
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
            let values = read(path)?;
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
fn with_tag<R>(
    path: &Path,
    f: impl FnOnce(&mut Tag) -> Result<R, String>,
    changes: Option<&Changes>,
    version: Id3Version,
) -> Result<R, String> {
    let save = changes.is_some();
    let numbers = |tag: &mut Id3v2Tag| {
        if let Some(changes) = changes {
            keep_numbers(tag, changes);
        }
    };
    let (view, file_type) = super::tags::first_tag_view(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(view);
    let options = ParseOptions::new().read_properties(false);
    let err = |e: lofty::error::FileParseError| described(&e);
    // The version the ID3v2 tag is written in, by the one it had.
    let id3 = |tag: Option<&Id3v2Tag>| {
        let original = tag.map(|t| match t.original_version() {
            lofty::id3::v2::Id3v2Version::V2 => 2,
            lofty::id3::v2::Id3v2Version::V3 => 3,
            lofty::id3::v2::Id3v2Version::V4 => 4,
        });
        WriteOptions::default().use_id3v23(version.writes_23(original))
    };
    let write = WriteOptions::default();
    match file_type {
        Some(FileType::Mpeg) => {
            let file = lofty::mpeg::MpegFile::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            let write = id3(file.id3v2());
            let tag = file.id3v2().cloned().unwrap_or_else(|| {
                seeded(
                    [
                        file.ape().cloned().map(Tag::from),
                        file.id3v1().cloned().map(Tag::from),
                    ]
                    .into_iter()
                    .flatten(),
                )
            });
            edit(tag, f, save, path, write, numbers)
        }
        Some(FileType::Wav) => {
            let file = lofty::iff::wav::WavFile::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            let write = id3(file.id3v2());
            let tag = file
                .id3v2()
                .cloned()
                .unwrap_or_else(|| seeded(file.riff_info().cloned().map(Tag::from)));
            edit(tag, f, save, path, write, numbers)
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
            edit(tag, f, save, path, write, |_| {})
        }
        Some(FileType::Mp4) => {
            let file = lofty::mp4::Mp4File::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            edit(
                file.ilst().cloned().unwrap_or_default(),
                f,
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

/// lofty writes an ID3v2 tag's track and disc numbers from numbers
/// (`3/12`): the frames written are put back as typed (`03/12`).
fn keep_numbers(tag: &mut Id3v2Tag, changes: &Changes) {
    for (key, id) in [(Key::Track, "TRCK"), (Key::Disc, "TPOS")] {
        let Some(value) = &changes.main[key as usize] else {
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
        let main = read(&file).unwrap().main;
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
        let before = read(&file).unwrap().main;
        assert_eq!(before[Key::Title as usize], "Old title");
        assert_eq!(before[Key::Album as usize], "Old album");
        let mut changes: Changes = Default::default();
        changes.main[Key::AlbumArtist as usize] = Some("Old band".into());
        write(&file, &changes, WriteTags::default()).unwrap();
        let after = read(&file).unwrap().main;
        assert_eq!(after[Key::AlbumArtist as usize], "Old band");
        assert_eq!(after[Key::Title as usize], "Old title");
        assert_eq!(after[Key::Artist as usize], "Old artist");
        assert_eq!(after[Key::Album as usize], "Old album");
        assert_eq!(after[Key::Year as usize], "1999");
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
            read(&file).unwrap().main[Key::Title as usize],
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
        let before = read(&copy).unwrap().main;
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
        let read_back = read(&copy).unwrap();
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
}
