//! An MP3's ID3v1 tag (ID3v1.1: the track in the comment's last byte),
//! written by hand: lofty writes its text as Latin-1, so a Cyrillic title
//! became question marks, while the players and car stereos that read
//! ID3v1 take its bytes in the system's code page (Windows-1251 on a
//! Russian Windows), as Winamp and the old taggers wrote them. The text is
//! written in the ANSI code page (`CP_ACP`), a letter it lacks as "?", each
//! field cut to its length on a letter's boundary; the genre is its number
//! in the list of genres (255, none, for another). Read, lofty takes the
//! bytes as Latin-1 too: `read_in_ansi` reads its text again in the same
//! code page, so a tag in Windows-1251 is not shown as "Àðèÿ".

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// The tag's size and the mark it starts with.
const SIZE: u64 = 128;
const MARK: &[u8; 3] = b"TAG";
/// No genre.
const NO_GENRE: u8 = 255;

/// The fields written, as the file info shows them (`edit::Values`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Fields<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    /// A date (`1987`, `1987-05-01`): its year is written.
    pub year: &'a str,
    pub comment: &'a str,
    /// `3` or `03/12`: the number is written.
    pub track: &'a str,
    pub genre: &'a str,
}

/// Writes `fields` as the file's ID3v1 tag: over the one it has, else at
/// its end.
pub fn write(path: &Path, fields: &Fields<'_>) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let at = match tag_at(&mut file)? {
        Some(at) => at,
        None => file.seek(SeekFrom::End(0))?,
    };
    file.seek(SeekFrom::Start(at))?;
    file.write_all(&block(fields))
}

/// Takes the file's ID3v1 tag off its end; false when it had none.
pub fn remove(path: &Path) -> std::io::Result<bool> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    match tag_at(&mut file)? {
        Some(at) => file.set_len(at).map(|()| true),
        None => Ok(false),
    }
}

/// Where the file's ID3v1 tag starts, if it has one.
fn tag_at(file: &mut std::fs::File) -> std::io::Result<Option<u64>> {
    let len = file.seek(SeekFrom::End(0))?;
    if len < SIZE {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(len - SIZE))?;
    let mut mark = [0u8; 3];
    file.read_exact(&mut mark)?;
    Ok((&mark == MARK).then_some(len - SIZE))
}

/// The tag's 128 bytes.
pub fn block(fields: &Fields<'_>) -> [u8; 128] {
    let mut out = [0u8; 128];
    out[..3].copy_from_slice(MARK);
    let mut put = |at: usize, size: usize, text: &str| {
        let bytes = ansi(text, size);
        out[at..at + bytes.len()].copy_from_slice(&bytes);
    };
    put(3, 30, fields.title);
    put(33, 30, fields.artist);
    put(63, 30, fields.album);
    let year: String = fields.year.trim().chars().take(4).collect();
    if year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()) {
        put(93, 4, &year);
    }
    // ID3v1.1: 28 bytes of comment, a zero, the track.
    put(97, 28, fields.comment);
    let track = fields
        .track
        .split('/')
        .next()
        .and_then(|n| n.trim().parse::<u32>().ok())
        .filter(|&n| (1..=255).contains(&n));
    out[125] = 0;
    out[126] = track.map_or(0, |n| n as u8);
    out[127] = genre_number(fields.genre);
    out
}

/// The genre's number in the list (any case), a number as it is (`17`,
/// `(17)`), else none.
fn genre_number(genre: &str) -> u8 {
    let genre = genre.trim();
    let bare = genre.trim_start_matches('(').trim_end_matches(')');
    if let Ok(n) = bare.parse::<u8>()
        && usize::from(n) < lofty::id3::v1::GENRES.len()
    {
        return n;
    }
    lofty::id3::v1::GENRES
        .iter()
        .position(|g| g.eq_ignore_ascii_case(genre))
        .map_or(NO_GENRE, |i| i as u8)
}

/// The text of an ID3v1 tag lofty read (`TagType::Id3v1`, each byte taken
/// as the Latin-1 letter of its number) read again in the ANSI code page,
/// the one it is written in.
pub fn read_in_ansi(tag: &mut lofty::tag::Tag) {
    use lofty::tag::{ItemValue, TagItem};
    let items: Vec<TagItem> = tag.items().cloned().collect();
    tag.retain(|_| false);
    for item in items {
        let (key, value) = item.consume();
        let value = match value {
            ItemValue::Text(text) => ItemValue::Text(from_latin1(text)),
            other => other,
        };
        tag.push_unchecked(TagItem::new(key, value));
    }
}

/// `text`, made of Latin-1 letters standing for bytes, read as those bytes
/// in the ANSI code page. Text with a letter past U+00FF did not come of
/// such bytes and stays as it is, so does ASCII.
fn from_latin1(text: String) -> String {
    use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};
    let Some(bytes) = text
        .chars()
        .map(|c| u8::try_from(c).ok())
        .collect::<Option<Vec<u8>>>()
    else {
        return text;
    };
    if bytes.is_ascii() {
        return text;
    }
    // At most a letter a byte (a code page of one byte a letter, or two).
    let mut wide = vec![0u16; bytes.len()];
    // SAFETY: both buffers are valid for the lengths given.
    let n = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            wide.len() as i32,
        )
    };
    if n <= 0 {
        return text;
    }
    String::from_utf16_lossy(&wide[..n as usize])
}

/// `text` in the ANSI code page, at most `size` bytes, cut between
/// letters.
fn ansi(text: &str, size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    for c in text.trim().chars() {
        let bytes = ansi_letter(c);
        if out.len() + bytes.len() > size {
            break;
        }
        out.extend_from_slice(&bytes);
    }
    out
}

/// One letter in the ANSI code page ("?" when it has none).
fn ansi_letter(c: char) -> Vec<u8> {
    use windows_sys::Win32::Globalization::{CP_ACP, WideCharToMultiByte};
    let mut wide = [0u16; 2];
    let wide = c.encode_utf16(&mut wide);
    let mut out = [0u8; 8];
    let mut lossy = 0;
    // SAFETY: both buffers are valid for the lengths given.
    let n = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            0,
            wide.as_ptr(),
            wide.len() as i32,
            out.as_mut_ptr(),
            out.len() as i32,
            std::ptr::null(),
            &mut lossy,
        )
    };
    if n <= 0 || lossy != 0 {
        return vec![b'?'];
    }
    out[..n as usize].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_holds_the_fields() {
        let b = block(&Fields {
            title: "Bohemian Rhapsody",
            artist: "Queen",
            album: "A Night at the Opera, which is a longer title",
            year: "1975-10-31",
            comment: "",
            track: "11/12",
            genre: "Rock",
        });
        assert_eq!(&b[..3], b"TAG");
        assert_eq!(&b[3..20], b"Bohemian Rhapsody");
        assert_eq!(b[20], 0);
        assert_eq!(&b[33..38], b"Queen");
        // Cut at 30 bytes.
        assert_eq!(&b[63..93], b"A Night at the Opera, which is");
        assert_eq!(&b[93..97], b"1975");
        assert_eq!((b[125], b[126], b[127]), (0, 11, 17));
        assert_eq!(genre_number("(9)"), 9);
        assert_eq!(genre_number("metal"), 9);
        assert_eq!(genre_number("Russian rock"), NO_GENRE);
    }

    #[test]
    fn written_over_the_one_there_and_removed() {
        let dir = std::env::temp_dir().join(format!("metalamp_id3v1_block_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.mp3");
        std::fs::write(&path, vec![0x55u8; 1000]).unwrap();
        let fields = Fields {
            title: "One",
            ..Fields::default()
        };
        write(&path, &fields).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 1128);
        // Again: in its place, not added.
        write(
            &path,
            &Fields {
                title: "Two",
                ..fields
            },
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 1128);
        assert_eq!(&bytes[1003..1006], b"Two");
        assert!(remove(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), vec![0x55u8; 1000]);
        assert!(!remove(&path).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Latin-1 letters standing for bytes are read in the code page; ASCII
    /// and text with letters past U+00FF stay as they are.
    #[test]
    fn latin1_read_in_the_ansi_code_page() {
        assert_eq!(from_latin1("Abc 12".into()), "Abc 12");
        assert_eq!(from_latin1("Ария".into()), "Ария");
        let ansi = unsafe { windows_sys::Win32::Globalization::GetACP() };
        if ansi == 1251 {
            assert_eq!(from_latin1("Àðèÿ".into()), "Ария");
        } else if ansi == 1252 {
            assert_eq!(from_latin1("Mot\u{f6}rhead".into()), "Motörhead");
            // 0x80: the euro sign in 1252, a control letter in Latin-1.
            assert_eq!(from_latin1("\u{80}".into()), "€");
        }
        let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::Id3v1);
        tag.insert_text(lofty::tag::ItemKey::TrackTitle, "Abc".into());
        tag.insert_text(lofty::tag::ItemKey::Genre, "Metal".into());
        read_in_ansi(&mut tag);
        assert_eq!(tag.get_string(lofty::tag::ItemKey::TrackTitle), Some("Abc"));
        assert_eq!(tag.get_string(lofty::tag::ItemKey::Genre), Some("Metal"));
    }

    /// A letter the code page has is one byte; one it lacks is "?". On a
    /// Russian Windows Cyrillic is Windows-1251.
    #[test]
    fn text_in_the_ansi_code_page() {
        assert_eq!(ansi("Abc", 30), b"Abc");
        assert_eq!(ansi("Abc", 2), b"Ab");
        assert_eq!(ansi("\u{1F3B8}", 30), b"?");
        let cyrillic = ansi("Ария", 30);
        assert_eq!(cyrillic.len(), 4);
        if unsafe { windows_sys::Win32::Globalization::GetACP() } == 1251 {
            assert_eq!(cyrillic, [0xc0, 0xf0, 0xe8, 0xff]);
        }
    }
}
