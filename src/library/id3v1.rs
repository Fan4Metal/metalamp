//! An MP3's ID3v1 tag (ID3v1.1: the track in the comment's last byte),
//! written by hand: lofty writes its text as Latin-1, so a Cyrillic title
//! became question marks, while the players and car stereos that read
//! ID3v1 take its bytes in the system's code page (Windows-1251 on a
//! Russian Windows), as Winamp and the old taggers wrote them. The text is
//! written in the code page chosen (`set_code_page`, the settings'
//! `id3v1_encoding`; by default the ANSI code page, `CP_ACP`), a letter it
//! lacks as "?", each field cut to its length on a letter's boundary; the
//! genre is its number in the list of genres (255, none, for another).
//! Read, lofty takes the bytes as Latin-1 too: `read_in_code_page` reads its
//! text again in the same code page, so a tag in Windows-1251 is not shown
//! as "Àðèÿ".

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// The code page ID3v1 is read and written in; 0: Windows' own.
static CODE_PAGE: AtomicU32 = AtomicU32::new(0);

/// Latin-1's code page: what lofty reads ID3v1 in, and what an index of
/// version 2 holds (`index.rs`).
pub const LATIN1: u32 = 28591;

/// Sets the code page ID3v1 is read and written in (0: Windows' own);
/// whether it is another than before.
pub fn set_code_page(code_page: u32) -> bool {
    CODE_PAGE.swap(code_page, Relaxed) != code_page
}

/// The code page ID3v1 is read and written in, Windows' own by its number.
pub fn code_page() -> u32 {
    match CODE_PAGE.load(Relaxed) {
        // SAFETY: no arguments.
        0 => unsafe { windows_sys::Win32::Globalization::GetACP() },
        n => n,
    }
}

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
    let code_page = code_page();
    let mut out = [0u8; 128];
    out[..3].copy_from_slice(MARK);
    let mut put = |at: usize, size: usize, text: &str| {
        let bytes = encoded(text, size, code_page);
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
/// as the Latin-1 letter of its number) read again in the code page chosen,
/// the one it is written in.
pub fn read_in_code_page(tag: &mut lofty::tag::Tag) {
    use lofty::tag::{ItemValue, TagItem};
    let code_page = code_page();
    let items: Vec<TagItem> = tag.items().cloned().collect();
    tag.retain(|_| false);
    for item in items {
        let (key, value) = item.consume();
        let value = match value {
            ItemValue::Text(text) => ItemValue::Text(from_latin1(text, code_page)),
            other => other,
        };
        tag.push_unchecked(TagItem::new(key, value));
    }
}

/// `text`, made of Latin-1 letters standing for bytes, read as those bytes
/// in code_page. Text with a letter past U+00FF did not come of
/// such bytes and stays as it is, so does ASCII.
fn from_latin1(text: String, code_page: u32) -> String {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;
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
            code_page,
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

/// `text` in `code_page`, at most `size` bytes, cut between letters.
fn encoded(text: &str, size: usize, code_page: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    for c in text.trim().chars() {
        let bytes = encoded_letter(c, code_page);
        if out.len() + bytes.len() > size {
            break;
        }
        out.extend_from_slice(&bytes);
    }
    out
}

/// One letter in `code_page` ("?" when it has none).
fn encoded_letter(c: char, code_page: u32) -> Vec<u8> {
    use windows_sys::Win32::Globalization::WideCharToMultiByte;
    let mut wide = [0u16; 2];
    let wide = c.encode_utf16(&mut wide);
    let mut out = [0u8; 8];
    let mut lossy = 0;
    // SAFETY: both buffers are valid for the lengths given.
    let n = unsafe {
        WideCharToMultiByte(
            code_page,
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
    fn latin1_read_in_a_code_page() {
        assert_eq!(from_latin1("Abc 12".into(), 1251), "Abc 12");
        assert_eq!(from_latin1("Ария".into(), 1251), "Ария");
        assert_eq!(from_latin1("Àðèÿ".into(), 1251), "Ария");
        assert_eq!(from_latin1("Mot\u{f6}rhead".into(), 1252), "Motörhead");
        // 0x80: the euro sign in 1252, a control letter in Latin-1.
        assert_eq!(from_latin1("\u{80}".into(), 1252), "€");
        assert_eq!(from_latin1("\u{8a}\u{a0}\u{ac}\u{a0}".into(), 866), "Кама");
        assert_eq!(from_latin1("\u{eb}\u{c9}\u{ce}\u{cf}".into(), 20866), "Кино");
        assert_eq!(from_latin1("\u{8a}\u{f3}\u{e9}".into(), 1250), "Šóé");
        let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::Id3v1);
        tag.insert_text(lofty::tag::ItemKey::TrackTitle, "Abc".into());
        tag.insert_text(lofty::tag::ItemKey::Genre, "Metal".into());
        read_in_code_page(&mut tag);
        assert_eq!(tag.get_string(lofty::tag::ItemKey::TrackTitle), Some("Abc"));
        assert_eq!(tag.get_string(lofty::tag::ItemKey::Genre), Some("Metal"));
    }

    /// A letter the code page has is one byte; one it lacks is "?".
    #[test]
    fn text_in_a_code_page() {
        assert_eq!(encoded("Abc", 30, 1251), b"Abc");
        assert_eq!(encoded("Abc", 2, 1251), b"Ab");
        assert_eq!(encoded("\u{1F3B8}", 30, 1251), b"?");
        assert_eq!(encoded("Ария", 30, 1251), [0xc0, 0xf0, 0xe8, 0xff]);
        assert_eq!(encoded("Кино", 30, 20866), [0xeb, 0xc9, 0xce, 0xcf]);
        assert_eq!(encoded("Кама", 30, 866), [0x8a, 0xa0, 0xac, 0xa0]);
        assert_eq!(encoded("Motörhead", 30, 1252), b"Mot\xf6rhead");
        assert_eq!(encoded("Ария", 30, 1252), b"????");
        // Each code page offered reads back what it writes.
        for e in crate::settings::Id3v1Encoding::ALL {
            let cp = match e.code_page() {
                0 => code_page(),
                n => n,
            };
            let bytes = encoded("Abc", 30, cp);
            let latin1: String = bytes.iter().map(|&b| char::from(b)).collect();
            assert_eq!(from_latin1(latin1, cp), "Abc", "{e:?}");
        }
    }
}