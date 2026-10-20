//! Everything a file says about itself, for the file info window: every
//! field of its main tag (not only those the library keeps), the tags it
//! holds, its pictures and its audio properties, and its cover. Read when
//! the window asks, on a thread of its own (`ui::info`): a few milliseconds
//! a file, more on a disk that was asleep.

use std::path::{Path, PathBuf};

use lofty::config::ParseOptions;
use lofty::picture::{MimeType, PictureType};
use lofty::prelude::*;
use lofty::tag::{ItemValue, TagType};

pub struct Details {
    pub path: PathBuf,
    pub size: u64,
    /// The tags in the file (`ID3v2.3`, `ID3v1`, `APE`, …), the one whose
    /// fields are shown first.
    pub tags: Vec<String>,
    /// That tag's fields: the usual ones in their usual order (`ORDER`),
    /// then the rest as they come. A key given more than once keeps every
    /// value, joined with "; "; the track and disc numbers carry their
    /// totals ("3/10").
    pub fields: Vec<(ItemKey, String)>,
    pub pictures: Vec<Picture>,
    pub duration_ms: u64,
    /// Kilobits per second.
    pub bitrate: u32,
    pub sample_rate: u32,
    pub bit_depth: Option<u8>,
    pub channels: u8,
    /// An MP3's LAME tag: the encoder's delay and padding, in samples.
    pub gapless: Option<(u32, u32)>,
    /// The front cover of the tag (else its first picture, else the
    /// folder's), at most [`COVER_SIDE`] pixels a side.
    pub cover: Option<egui::ColorImage>,
}

/// The largest side of [`Details::cover`].
const COVER_SIDE: u32 = 400;

pub struct Picture {
    pub kind: PictureType,
    /// `JPEG`, `PNG`, or the MIME type as written.
    pub format: String,
    /// The picture's size in pixels, when its header tells.
    pub pixels: Option<(u32, u32)>,
    pub bytes: usize,
}

/// The fields in the order players and taggers show them (ID3v2).
const ORDER: [ItemKey; 19] = [
    ItemKey::TrackNumber,
    ItemKey::DiscNumber,
    ItemKey::TrackTitle,
    ItemKey::TrackArtist,
    ItemKey::AlbumTitle,
    ItemKey::Year,
    ItemKey::RecordingDate,
    ItemKey::Genre,
    ItemKey::Comment,
    ItemKey::AlbumArtist,
    ItemKey::Composer,
    ItemKey::Publisher,
    ItemKey::OriginalArtist,
    ItemKey::CopyrightMessage,
    ItemKey::AudioFileUrl,
    ItemKey::EncodedBy,
    ItemKey::Bpm,
    ItemKey::ReplayGainTrackGain,
    ItemKey::ReplayGainAlbumGain,
];

pub fn read(path: &Path) -> Result<Details, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let file = super::tags::open(path, ParseOptions::new().read_properties(true))
        .map_err(|e| e.to_string())?;
    let props = file.properties();
    let head = crate::audio::mp3gap::read_head(path);
    let main = file.primary_tag().or_else(|| file.first_tag());
    let mut tags: Vec<String> = main
        .into_iter()
        .chain(
            file.tags()
                .iter()
                .filter(|t| Some(t.tag_type()) != main.map(|m| m.tag_type())),
        )
        .map(|t| tag_name(t.tag_type(), &head))
        .collect();
    tags.dedup();
    let (fields, pictures) = match main {
        Some(tag) => (fields_of(tag), tag.pictures().iter().map(picture).collect()),
        None => (Vec::new(), Vec::new()),
    };
    let tagged = main.and_then(|tag| {
        let pictures = tag.pictures();
        pictures
            .iter()
            .find(|p| p.pic_type() == PictureType::CoverFront)
            .or_else(|| pictures.first())
            .and_then(|p| crate::covers::decode(p.data()))
    });
    let cover = tagged
        .or_else(|| {
            let found = crate::covers::in_folder(path.parent()?)?;
            crate::covers::decode(&std::fs::read(found).ok()?)
        })
        .map(|picture| {
            let picture = if picture.width().max(picture.height()) > COVER_SIDE {
                picture.resize(
                    COVER_SIDE,
                    COVER_SIDE,
                    image::imageops::FilterType::Triangle,
                )
            } else {
                picture
            };
            crate::covers::color_image(&picture)
        });
    let is_mp3 = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    Ok(Details {
        path: path.to_path_buf(),
        size,
        tags,
        fields,
        pictures,
        duration_ms: props.duration().as_millis() as u64,
        bitrate: props
            .audio_bitrate()
            .or(props.overall_bitrate())
            .unwrap_or(0),
        sample_rate: props.sample_rate().unwrap_or(0),
        bit_depth: props.bit_depth(),
        channels: props.channels().unwrap_or(0),
        gapless: is_mp3
            .then(|| crate::audio::mp3gap::parse(&head))
            .flatten()
            .map(|g| (g.delay, g.padding)),
        cover,
    })
}

/// A tag's name as taggers show it; an ID3v2 tag at the file's start with
/// its version (lofty reads every version as 2.4).
fn tag_name(tag: TagType, head: &[u8]) -> String {
    match tag {
        TagType::Id3v2 if head.len() > 3 && &head[..3] == b"ID3" => format!("ID3v2.{}", head[3]),
        TagType::Id3v2 => "ID3v2".into(),
        TagType::Id3v1 => "ID3v1".into(),
        TagType::Ape => "APE".into(),
        TagType::Mp4Ilst => "MP4".into(),
        TagType::VorbisComments => "Vorbis".into(),
        TagType::RiffInfo => "RIFF INFO".into(),
        other => format!("{other:?}"),
    }
}

pub(super) fn fields_of(tag: &lofty::tag::Tag) -> Vec<(ItemKey, String)> {
    // Every text value by key, keys in the order they came.
    let mut found: Vec<(ItemKey, Vec<String>)> = Vec::new();
    for item in tag.items() {
        let text = match item.value() {
            ItemValue::Text(s) | ItemValue::Locator(s) => clean(s),
            ItemValue::Binary(_) => continue,
        };
        if text.is_empty() {
            continue;
        }
        match found.iter_mut().find(|(k, _)| *k == item.key()) {
            Some((_, values)) => {
                if !values.contains(&text) {
                    values.push(text);
                }
            }
            None => found.push((item.key(), vec![text])),
        }
    }
    let take = |found: &mut Vec<(ItemKey, Vec<String>)>, key: ItemKey| {
        found
            .iter()
            .position(|(k, _)| *k == key)
            .map(|i| found.remove(i).1.join("; "))
    };
    let mut out = Vec::new();
    for key in ORDER {
        let Some(mut value) = take(&mut found, key) else {
            continue;
        };
        let total = match key {
            ItemKey::TrackNumber => take(&mut found, ItemKey::TrackTotal),
            ItemKey::DiscNumber => take(&mut found, ItemKey::DiscTotal),
            _ => None,
        };
        if let Some(total) = total {
            value = with_total(&value, &total);
        }
        out.push((key, value));
    }
    out.extend(found.into_iter().map(|(k, v)| (k, v.join("; "))));
    out
}

/// A number with its total, unless the number already has one (`3/10`).
fn with_total(number: &str, total: &str) -> String {
    if number.contains('/') {
        number.to_owned()
    } else {
        format!("{number}/{total}")
    }
}

/// Tag text without the NULs and surrounding spaces some taggers leave.
fn clean(s: &str) -> String {
    s.trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_owned()
}

fn picture(p: &lofty::picture::Picture) -> Picture {
    let format = match p.mime_type() {
        Some(MimeType::Jpeg) => "JPEG".to_owned(),
        Some(MimeType::Png) => "PNG".to_owned(),
        Some(MimeType::Gif) => "GIF".to_owned(),
        Some(MimeType::Bmp) => "BMP".to_owned(),
        Some(other) => other.as_str().to_owned(),
        None => String::new(),
    };
    // The header only: no picture is decoded.
    let pixels = image::ImageReader::new(std::io::Cursor::new(p.data()))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok());
    Picture {
        kind: p.pic_type(),
        format,
        pixels,
        bytes: p.data().len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_with_totals() {
        assert_eq!(with_total("3", "10"), "3/10");
        assert_eq!(with_total("3/12", "10"), "3/12");
    }

    #[test]
    fn id3v2_named_with_its_version() {
        assert_eq!(tag_name(TagType::Id3v2, b"ID3\x03\x00"), "ID3v2.3");
        assert_eq!(tag_name(TagType::Id3v2, b"RIFF"), "ID3v2");
        assert_eq!(tag_name(TagType::Id3v1, b""), "ID3v1");
    }

    #[test]
    fn fields_in_their_order_then_the_rest() {
        let mut tag = lofty::tag::Tag::new(TagType::Id3v2);
        tag.insert_text(ItemKey::Composer, "Холстинин".into());
        tag.insert_text(ItemKey::TrackTitle, "Патриот".into());
        tag.insert_text(ItemKey::TrackNumber, "1".into());
        tag.insert_text(ItemKey::TrackTotal, "9".into());
        tag.insert_text(ItemKey::Mood, "  \0".into());
        tag.insert_text(ItemKey::Label, "CD-Maximum".into());
        let fields = fields_of(&tag);
        let keys: Vec<ItemKey> = fields.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            [
                ItemKey::TrackNumber,
                ItemKey::TrackTitle,
                ItemKey::Composer,
                ItemKey::Label
            ]
        );
        assert_eq!(fields[0].1, "1/9");
    }

    /// `METALAMP_SAMPLE=<a tagged file> cargo test details_of_a_real_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn details_of_a_real_file() {
        let path = std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE");
        let d = read(Path::new(&path)).unwrap();
        println!("{:?} {:?}", d.tags, d.gapless);
        for (k, v) in &d.fields {
            println!("{k:?}: {v}");
        }
        for p in &d.pictures {
            println!("{:?} {} {:?} {}", p.kind, p.format, p.pixels, p.bytes);
        }
        assert!(!d.tags.is_empty());
        assert!(d.duration_ms > 0);
    }
}
