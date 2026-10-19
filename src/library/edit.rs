//! Writing the main fields of a track's tag (the tag editor,
//! `ui::tag_editor`). The file's own tag is read (an MP3's first ID3v2
//! tag, as the library reads it: `tags::first_tag_view`), split into the
//! fields every tag has and the rest (`SplitTag`), the fields changed, and
//! the two merged back (`MergeTag`) and written: frames lofty cannot map
//! to the common fields (pictures stay too, private frames, ratings of
//! other players) are kept. An ID3v2.3 tag is written as 2.3. MP3 and
//! WAV (ID3v2), FLAC (Vorbis comments) and M4A (iTunes atoms); WMA is
//! read but not written by lofty.

use std::io::BufReader;
use std::path::Path;

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType};
use lofty::prelude::*;
use lofty::tag::{ItemKey, MergeTag, SplitTag, Tag};

/// The fields the editor shows, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Title,
    Artist,
    AlbumArtist,
    Album,
    Year,
    Track,
    Disc,
    Genre,
    Comment,
}

impl Key {
    pub const ALL: [Key; 9] = [
        Key::Title,
        Key::Artist,
        Key::AlbumArtist,
        Key::Album,
        Key::Year,
        Key::Track,
        Key::Disc,
        Key::Genre,
        Key::Comment,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Key::Title => tr!("Title", "Название"),
            Key::Artist => tr!("Artist", "Исполнитель"),
            Key::AlbumArtist => tr!("Album artist", "Исполнитель альбома"),
            Key::Album => tr!("Album", "Альбом"),
            Key::Year => tr!("Year", "Год"),
            Key::Track => tr!("Track", "Номер"),
            Key::Disc => tr!("Disc", "Диск"),
            Key::Genre => tr!("Genre", "Жанр"),
            Key::Comment => tr!("Comment", "Комментарий"),
        }
    }

    /// A number field: the year, `3` or `3/12` for the track and the disc.
    pub fn is_number(self) -> bool {
        matches!(self, Key::Year | Key::Track | Key::Disc)
    }
}

/// The fields' values in [`Key::ALL`]'s order; empty: none.
pub type Values = [String; 9];

/// What to write: for each field, the value to put (empty: the field
/// removed), or `None` to leave it as it is.
pub type Changes = [Option<String>; 9];

/// The values of `path`'s tag (all empty when it has none).
pub fn read(path: &Path) -> Result<Values, String> {
    with_tag(path, |tag| Ok(values(tag)), false)
}

/// Writes `changes` to `path`'s tag (one is made when it has none).
pub fn write(path: &Path, changes: &Changes) -> Result<(), String> {
    // Checked before the file is touched: a bad number writes nothing.
    for (key, change) in Key::ALL.iter().zip(changes) {
        if let Some(v) = change
            && key.is_number()
        {
            parse_number(*key, v)?;
        }
    }
    with_tag(path, |tag| apply(tag, changes), true)
}

/// Reads `path`'s own tag, lets `f` see (and change) its common fields,
/// and when `save`, writes it back with the rest of it kept.
fn with_tag<R>(
    path: &Path,
    f: impl FnOnce(&mut Tag) -> Result<R, String>,
    save: bool,
) -> Result<R, String> {
    let (view, file_type) = super::tags::first_tag_view(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(view);
    let options = ParseOptions::new().read_properties(false);
    let err = |e: lofty::error::FileParseError| e.to_string();
    let write = WriteOptions::default().use_id3v23(is_id3v23(path));
    match file_type {
        Some(FileType::Mpeg) => {
            let file = lofty::mpeg::MpegFile::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            edit(
                file.id3v2().cloned().unwrap_or_default(),
                f,
                save,
                path,
                write,
            )
        }
        Some(FileType::Wav) => {
            let file = lofty::iff::wav::WavFile::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            edit(
                file.id3v2().cloned().unwrap_or_default(),
                f,
                save,
                path,
                write,
            )
        }
        Some(FileType::Flac) => {
            let file = lofty::flac::FlacFile::read_from(&mut reader, options).map_err(err)?;
            drop(reader);
            let tag = file.vorbis_comments().cloned().unwrap_or_default();
            edit(tag, f, save, path, write)
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
            )
        }
        _ => Err(tr!(
            "tags of this format cannot be written",
            "теги этого формата записать нельзя"
        )
        .to_owned()),
    }
}

/// `tag` split, its common fields given to `f`, merged back and, when
/// `save`, written to `path`.
fn edit<T, R>(
    tag: T,
    f: impl FnOnce(&mut Tag) -> Result<R, String>,
    save: bool,
    path: &Path,
    write: WriteOptions,
) -> Result<R, String>
where
    T: SplitTag,
    <T::Remainder as MergeTag>::Merged: TagExt,
{
    let (rest, mut common) = tag.split_tag();
    let out = f(&mut common)?;
    if save {
        rest.merge_tag(common)
            .save_to_path(path, write)
            .map_err(|e| in_use(&e.to_string()))?;
    }
    Ok(out)
}

/// Windows' "used by another process" (a sharing violation, os error
/// 32) said plainly: the track playing is held open by the player.
fn in_use(error: &str) -> String {
    if error.contains("os error 32") {
        tr!(
            "the file is in use (the track playing? Stop it and save again)",
            "файл занят (играющий трек? Остановите его и сохраните снова)"
        )
        .to_owned()
    } else {
        error.to_owned()
    }
}

/// The file's ID3v2 tag is of version 2.3 (most taggers' default), to be
/// written as such; lofty would write 2.4.
fn is_id3v23(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|()| &head[..3] == b"ID3" && head[3] == 3)
}

fn values(tag: &Tag) -> Values {
    let text =
        |s: Option<std::borrow::Cow<'_, str>>| s.map(|s| s.trim().to_owned()).unwrap_or_default();
    let numbered = |n: Option<u32>, total: Option<u32>| match (n, total) {
        (Some(n), Some(t)) => format!("{n}/{t}"),
        (Some(n), None) => n.to_string(),
        _ => String::new(),
    };
    Key::ALL.map(|k| match k {
        Key::Title => text(tag.title()),
        Key::Artist => text(tag.artist()),
        Key::AlbumArtist => tag
            .get_string(ItemKey::AlbumArtist)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default(),
        Key::Album => text(tag.album()),
        Key::Year => tag.date().map(|d| d.year.to_string()).unwrap_or_default(),
        Key::Track => numbered(tag.track(), tag.track_total()),
        Key::Disc => numbered(tag.disk(), tag.disk_total()),
        Key::Genre => text(tag.genre()),
        Key::Comment => text(tag.comment()),
    })
}

/// A number field's value: the number and its total (`3/12`), none when
/// empty; an error naming the field otherwise.
fn parse_number(key: Key, value: &str) -> Result<Option<(u32, Option<u32>)>, String> {
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
    let (n, total) = value.split_once('/').unwrap_or((value, ""));
    let n: u32 = n.trim().parse().map_err(|_| bad())?;
    let total = match total.trim() {
        "" => None,
        t => Some(t.parse().map_err(|_| bad())?),
    };
    if key == Key::Year && !(1..=9999).contains(&n) {
        return Err(bad());
    }
    Ok(Some((n, total)))
}

fn apply(tag: &mut Tag, changes: &Changes) -> Result<(), String> {
    for (&key, change) in Key::ALL.iter().zip(changes) {
        let Some(value) = change else { continue };
        let value = value.trim().to_owned();
        let empty = value.is_empty();
        match key {
            Key::Title if empty => tag.remove_title(),
            Key::Title => tag.set_title(value),
            Key::Artist if empty => tag.remove_artist(),
            Key::Artist => tag.set_artist(value),
            Key::AlbumArtist if empty => tag.remove_key(ItemKey::AlbumArtist),
            Key::AlbumArtist => {
                tag.insert_text(ItemKey::AlbumArtist, value);
            }
            Key::Album if empty => tag.remove_album(),
            Key::Album => tag.set_album(value),
            Key::Genre if empty => tag.remove_genre(),
            Key::Genre => tag.set_genre(value),
            Key::Comment if empty => tag.remove_comment(),
            Key::Comment => tag.set_comment(value),
            Key::Year => match parse_number(key, &value)? {
                None => tag.remove_date(),
                Some((year, _)) => tag.set_date(lofty::tag::items::Timestamp {
                    year: year as u16,
                    ..Default::default()
                }),
            },
            Key::Track => {
                tag.remove_track();
                tag.remove_track_total();
                if let Some((n, total)) = parse_number(key, &value)? {
                    tag.set_track(n);
                    if let Some(t) = total {
                        tag.set_track_total(t);
                    }
                }
            }
            Key::Disc => {
                tag.remove_disk();
                tag.remove_disk_total();
                if let Some((n, total)) = parse_number(key, &value)? {
                    tag.set_disk(n);
                    if let Some(t) = total {
                        tag.set_disk_total(t);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_number(Key::Track, " 3 "), Ok(Some((3, None))));
        assert_eq!(parse_number(Key::Track, "3/12"), Ok(Some((3, Some(12)))));
        assert_eq!(parse_number(Key::Disc, ""), Ok(None));
        assert!(parse_number(Key::Track, "three").is_err());
        assert!(parse_number(Key::Year, "0").is_err());
        assert_eq!(parse_number(Key::Year, "1999"), Ok(Some((1999, None))));
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
        let before = read(&copy).unwrap();
        let v23 = is_id3v23(&copy);
        let frames_before = id3_frames(&copy);
        let mut changes: Changes = Default::default();
        changes[0] = Some("Edited title".into());
        changes[2] = Some("Various Artists".into());
        changes[5] = Some("7/9".into());
        write(&copy, &changes).unwrap();
        let after = read(&copy).unwrap();
        // Written in the version it had.
        assert_eq!(is_id3v23(&copy), v23);
        assert_eq!(after[0], "Edited title");
        assert_eq!(after[2], "Various Artists");
        assert_eq!(after[5], "7/9");
        assert_eq!(after[1], before[1]);
        assert_eq!(after[3], before[3]);
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
