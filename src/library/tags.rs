//! Reading a track's tags and properties with lofty. What a tag lacks is
//! taken from the folders, laid out as most collections are:
//! `Artist\(Year) Album\NN - Title.mp3`.

use std::path::Path;

use lofty::config::ParseOptions;
use lofty::prelude::*;
use lofty::probe::Probe;

use super::Track;

/// The track at `path`, of `size` bytes modified at `mtime`. A file lofty
/// cannot read is still listed, with what its name and folders give.
pub fn read(path: &Path, size: u64, mtime: u64) -> Track {
    let mut track = Track {
        path: path.to_path_buf(),
        size,
        mtime,
        ..Track::default()
    };
    match read_tagged(path) {
        Ok(file) => fill(&mut track, &file),
        Err(e) => log::debug!("no tags in {}: {e}", path.display()),
    }
    fill_from_names(&mut track);
    track
}

fn read_tagged(path: &Path) -> Result<lofty::file::TaggedFile, lofty::error::FileParseError> {
    // The whole ID3v2 tag is read anyway; its pictures only tell whether
    // the album's cover can come from the file (`covers.rs` reads it again
    // for the one track it needs).
    Probe::open(path)?
        .options(ParseOptions::new().read_properties(true))
        .read()
}

fn fill(track: &mut Track, file: &lofty::file::TaggedFile) {
    let props = file.properties();
    track.duration_ms = props.duration().as_millis().min(u32::MAX as u128) as u32;
    track.bitrate = props
        .audio_bitrate()
        .or(props.overall_bitrate())
        .unwrap_or(0);
    track.sample_rate = props.sample_rate().unwrap_or(0);
    track.channels = props.channels().unwrap_or(0);
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return;
    };
    let text = |s: Option<std::borrow::Cow<'_, str>>| s.map(|s| clean(&s)).unwrap_or_default();
    track.title = text(tag.title());
    track.artist = text(tag.artist());
    track.album = text(tag.album());
    track.genre = text(tag.genre());
    track.album_artist = tag
        .get_string(ItemKey::AlbumArtist)
        .map(clean)
        .unwrap_or_default();
    track.year = tag.date().map(|d| d.year).unwrap_or(0);
    track.track = tag.track().unwrap_or(0).min(u16::MAX as u32) as u16;
    track.disc = tag.disk().unwrap_or(0).min(u16::MAX as u32) as u16;
    track.has_cover = !tag.pictures().is_empty();
}

/// Tag text without the NULs and surrounding spaces some taggers leave.
fn clean(s: &str) -> String {
    s.trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_owned()
}

/// What the tag lacked, from the file's name and its folders.
fn fill_from_names(track: &mut Track) {
    let album_dir = track.path.parent();
    let artist_dir = album_dir.and_then(Path::parent);
    let name = |p: Option<&Path>| {
        p.and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    if track.title.is_empty() {
        let stem = track
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (number, title) = split_number(&stem);
        track.title = title.to_owned();
        if track.track == 0 {
            track.track = number.unwrap_or(0);
        }
    }
    let folder = name(album_dir);
    let (folder_year, folder_album) = split_year(&folder);
    if track.album.is_empty() {
        track.album = folder_album.to_owned();
    }
    if track.year == 0 {
        track.year = folder_year.unwrap_or(0);
    }
    if track.artist.is_empty() && track.album_artist.is_empty() {
        track.artist = name(artist_dir);
    }
}

/// `"01 - Title"` → `(Some(1), "Title")`; also `"01. Title"`, `"01 Title"`.
fn split_number(stem: &str) -> (Option<u16>, &str) {
    let digits = stem.len() - stem.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || digits > 3 {
        return (None, stem);
    }
    let rest = stem[digits..].trim_start_matches([' ', '-', '.', '_']);
    if rest.is_empty() {
        return (None, stem);
    }
    (stem[..digits].parse().ok(), rest)
}

/// `"(2003) Крещение Огнём"` → `(Some(2003), "Крещение Огнём")`; also
/// `"2003 - Album"`.
fn split_year(folder: &str) -> (Option<u16>, &str) {
    let s = folder.trim();
    let (inner, rest) = if let Some(after) = s.strip_prefix('(') {
        match after.split_once(')') {
            Some((year, rest)) => (year, rest),
            None => return (None, s),
        }
    } else if s.len() > 4 && s.is_char_boundary(4) {
        (&s[..4], &s[4..])
    } else {
        return (None, s);
    };
    match inner.trim().parse::<u16>() {
        Ok(year) if (1900..=2100).contains(&year) => {
            let rest = rest.trim_start_matches([' ', '-', '.', '_']);
            (Some(year), if rest.is_empty() { s } else { rest })
        }
        _ => (None, s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_years_from_names() {
        assert_eq!(split_number("01 - Патриот"), (Some(1), "Патриот"));
        assert_eq!(split_number("7. Song"), (Some(7), "Song"));
        assert_eq!(split_number("1999"), (None, "1999"));
        assert_eq!(split_number("Song"), (None, "Song"));
        assert_eq!(
            split_year("(2003) Крещение Огнём"),
            (Some(2003), "Крещение Огнём")
        );
        assert_eq!(
            split_year("1988 - 11 Of The Best"),
            (Some(1988), "11 Of The Best")
        );
        assert_eq!(split_year("Gold"), (None, "Gold"));
        assert_eq!(split_year("(Live) Gold"), (None, "(Live) Gold"));
    }

    #[test]
    fn untagged_file_takes_its_folders() {
        let mut t = Track {
            path: "D:\\Music\\Ария\\(2003) Крещение Огнём\\01 - Патриот.mp3".into(),
            ..Track::default()
        };
        fill_from_names(&mut t);
        assert_eq!(t.title, "Патриот");
        assert_eq!(t.track, 1);
        assert_eq!(t.album, "Крещение Огнём");
        assert_eq!(t.year, 2003);
        assert_eq!(t.artist, "Ария");
    }

    /// A real file of the user's library (`METALAMP_SAMPLE`): its tags read.
    #[test]
    #[ignore = "needs METALAMP_SAMPLE, an MP3 with tags"]
    fn reads_a_real_file() {
        let path = std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE");
        let size = std::fs::metadata(&path).unwrap().len();
        let t = read(Path::new(&path), size, 0);
        println!("{t:#?}");
        assert!(!t.title.is_empty());
        assert!(t.duration_ms > 0);
    }
}
