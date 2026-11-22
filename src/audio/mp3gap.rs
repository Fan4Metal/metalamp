//! An MP3's gapless information: the Xing/Info header in its first frame
//! (the number of frames) and the LAME tag after it (the encoder's delay
//! and padding). An MP3 encoder adds silence before the music (the delay,
//! 576 samples for LAME) and after it (the padding, to fill the last
//! frame); without cutting them, an album meant to play without pauses
//! clicks or gaps between tracks.

/// Samples Media Foundation's MP3 decoder gives before the encoder's
/// delay, past those of the Xing/Info frame, which it decodes as silence
/// (a frame's: 1152 for MPEG-1, 576 for MPEG-2 and 2.5, 16 to 24 kHz): it
/// leaves out the first 528 of its own delay (529). Measured with
/// `--bench gapless`: it gives the frames' samples plus a frame less 528
/// (624 for MPEG-1, 48 for MPEG-2), the end ends `padding - 529` samples
/// into the padding, and the music starts a frame + 1 + the delay into
/// what it gives (1153 + the delay for MPEG-1, 577 + it for MPEG-2).
pub const DECODER_OFFSET: u64 = 1;

/// The music's place among the samples the decoder gives: from `start`,
/// `length` samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trim {
    pub start: u64,
    pub length: u64,
}

/// Where the music is in an MP3 decoded by Media Foundation, from the
/// file's LAME tag; `None` without one that counts the frames, or with
/// figures no encoder writes (a damaged tag).
pub fn trim_of(info: &GapInfo) -> Option<Trim> {
    if info.delay > 4000 || info.padding > 4000 {
        return None;
    }
    Some(Trim {
        start: u64::from(info.samples_per_frame) + DECODER_OFFSET + u64::from(info.delay),
        length: info.music_samples()?,
    })
}

/// The start of `file`: its ID3v2 tags and 8 KB after them.
pub fn read_head(file: &std::path::Path) -> Vec<u8> {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(file) else {
        return Vec::new();
    };
    let end = leading_tags(&mut f).last().map_or(0, |t| t.end);
    let mut head = Vec::new();
    let _ = std::io::Seek::rewind(&mut f);
    let _ = f.take(end + 8192).read_to_end(&mut head);
    head
}

/// Where the ID3v2 tags at the start of `file` lie, one after another
/// (a tagger that wrote a new tag before an old one leaves both; at
/// most 8).
pub fn leading_tags(file: &mut (impl std::io::Read + std::io::Seek)) -> Vec<std::ops::Range<u64>> {
    let mut tags = Vec::new();
    let mut at = 0;
    let mut header = [0u8; 10];
    let end = file.seek(std::io::SeekFrom::End(0)).unwrap_or(0);
    while tags.len() < 8
        && file.seek(std::io::SeekFrom::Start(at)).is_ok()
        && file.read_exact(&mut header).is_ok()
    {
        let len = id3v2_len(&header) as u64;
        // A size past the file's end is no tag (a damaged one).
        if len == 0 || at + len > end {
            break;
        }
        tags.push(at..at + len);
        at += len;
    }
    tags
}

/// What the header says, in samples per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GapInfo {
    /// Frames of audio after the header frame, if the header counts them.
    pub frames: Option<u32>,
    pub samples_per_frame: u32,
    pub delay: u32,
    pub padding: u32,
}

impl GapInfo {
    /// Samples of music: all the frames' minus the delay and the padding.
    pub fn music_samples(&self) -> Option<u64> {
        let total = u64::from(self.frames?) * u64::from(self.samples_per_frame);
        total.checked_sub(u64::from(self.delay) + u64::from(self.padding))
    }
}

/// The ID3v2 tag's length at the start of `data` (header and footer
/// included), 0 without one (a version or a size that cannot be is none).
pub fn id3v2_len(data: &[u8]) -> usize {
    if data.len() < 10
        || &data[..3] != b"ID3"
        || !(2..=4).contains(&data[3])
        || data[6..10].iter().any(|&b| b >= 0x80)
    {
        return 0;
    }
    let size = data[6..10]
        .iter()
        .fold(0usize, |acc, &b| (acc << 7) | usize::from(b));
    let footer = if data[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

/// The length of all the ID3v2 tags one after another at the start of
/// `data`.
fn id3v2_tags_len(data: &[u8]) -> usize {
    let mut at = 0;
    for _ in 0..8 {
        match data.get(at..).map(id3v2_len) {
            Some(len) if len > 0 => at += len,
            _ => break,
        }
    }
    at
}

/// Where the first frame's Xing/Info header and LAME tag lie.
struct Located<'a> {
    /// The first frame, from its header on.
    header: &'a [u8],
    frames: Option<u32>,
    samples_per_frame: u32,
    /// The Xing header's quality (`100 - 10 × V - q`), when it has one.
    quality: Option<u32>,
    /// Where the LAME tag starts in `header` (none: a Xing/Info header
    /// with something else after it), and the encoder's name in its
    /// first 4 bytes.
    lame: Option<usize>,
    /// What follows the name is LAME's filler (`LAME3.85UUUU…`), not a
    /// tag: old versions wrote it into their frames, and the first frame
    /// of a file without an info tag carries it where the tag would be.
    filler: bool,
}

/// LAME's filler after its name: 0x55 ("U") or 0xAA over and over.
fn is_filler(bytes: &[u8]) -> bool {
    bytes
        .windows(12)
        .any(|w| (w[0] == 0x55 || w[0] == 0xaa) && w.iter().all(|&b| b == w[0]))
}

/// The first frame of an MP3 whose start (past the ID3v2 tags,
/// `id3v2_tags_len`) is `data`, and what its Xing/Info header holds;
/// `None` without a Xing/Info header or a LAME tag where it would be.
fn locate(data: &[u8]) -> Option<Located<'_>> {
    let start = id3v2_tags_len(data);
    let data = data.get(start..)?;
    // The first frame header: a sync, MPEG 1, 2 or 2.5, Layer III.
    let at = (0..data.len().saturating_sub(4)).find(|&i| {
        let (b1, b2) = (data[i + 1], data[i + 2]);
        data[i] == 0xff
            && b1 & 0xe0 == 0xe0
            && (b1 >> 3) & 3 != 1
            && (b1 >> 1) & 3 == 1
            && b2 >> 4 != 0xf
            && (b2 >> 2) & 3 != 3
    })?;
    let header = &data[at..];
    let mpeg1 = (header[1] >> 3) & 3 == 3;
    let mono = header[3] >> 6 == 3;
    let side_info = match (mpeg1, mono) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let samples_per_frame = if mpeg1 { 1152 } else { 576 };
    let x = 4 + side_info;
    let tag = header.get(x..x + 4)?;
    let u32_at = |i: usize| -> Option<u32> {
        header
            .get(i..i + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (frames, quality, lame) = if tag == b"Xing" || tag == b"Info" {
        let flags = u32_at(x + 4)?;
        let mut p = x + 8;
        let mut frames = None;
        let mut quality = None;
        if flags & 1 != 0 {
            frames = u32_at(p);
            p += 4;
        }
        if flags & 2 != 0 {
            p += 4;
        }
        if flags & 4 != 0 {
            p += 100;
        }
        if flags & 8 != 0 {
            quality = u32_at(p);
            p += 4;
        }
        (frames, quality, p)
    } else if tag == b"LAME" || tag == b"Lavc" || tag == b"Lavf" {
        // A LAME tag where the Xing header would be, without one.
        (None, None, x)
    } else {
        return None;
    };
    let name = header.get(lame..lame + 4)?;
    let lame =
        (name == b"LAME" || name == b"Lavc" || name == b"Lavf" || name == b"L3.9").then_some(lame);
    let filler =
        lame.is_some_and(|l| is_filler(header.get(l..).map_or(&[][..], |h| &h[..h.len().min(40)])));
    Some(Located {
        header,
        frames,
        samples_per_frame,
        quality,
        lame,
        filler,
    })
}

/// The gapless information of an MP3 whose start (past the ID3v2 tags,
/// `id3v2_tags_len`) is `data`; `None` without a Xing/Info header with a
/// LAME tag.
pub fn parse(data: &[u8]) -> Option<GapInfo> {
    let found = locate(data)?;
    // LAME's filler where the tag would be, without a Xing header: none.
    if found.filler && found.frames.is_none() && found.quality.is_none() {
        return None;
    }
    let Some(lame) = found.lame.filter(|_| !found.filler) else {
        return Some(GapInfo {
            frames: found.frames,
            samples_per_frame: found.samples_per_frame,
            delay: 0,
            padding: 0,
        });
    };
    // 12 bits of delay and 12 of padding, 21 bytes into the LAME tag.
    let d = found.header.get(lame + 21..lame + 24)?;
    let delay = (u32::from(d[0]) << 4) | (u32::from(d[1]) >> 4);
    let padding = ((u32::from(d[1]) & 0x0f) << 8) | u32::from(d[2]);
    Some(GapInfo {
        frames: found.frames,
        samples_per_frame: found.samples_per_frame,
        delay,
        padding,
    })
}

/// What an MP3's LAME tag says of the encoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encoder {
    /// Its name and version as written: `LAME3.99r`, `LAME3.100`,
    /// `Lavc58.91`, `Lavc60.3` (written `Lavc60.3.`), `Lavf`.
    pub name: String,
    /// Its settings as MediaInfo puts them together from the tag's fields
    /// (`-m j -V 4 -q 3 -lowpass 20.5`); empty when the tag is not
    /// LAME's own (FFmpeg's leaves the fields empty).
    pub settings: String,
}

/// The encoder of an MP3 whose start (past the ID3v2 tags) is `data`, from
/// its LAME tag: the name in its first 9 bytes, the settings from the VBR
/// method and lowpass (bytes 9 and 10), the bitrate (20), the stereo mode
/// (24) and the Xing header's quality.
pub fn encoder(data: &[u8]) -> Option<Encoder> {
    let found = locate(data)?;
    let lame = found.lame?;
    // The name: 9 bytes in a tag; before the filler without one
    // (`LAME3.88 (alpha)`, `LAME3.97.3`).
    let tag = found.header.get(lame..)?;
    let most = if found.filler { 24 } else { 9 };
    let name: String = tag[..tag.len().min(most)]
        .iter()
        .take_while(|&&b| (b.is_ascii_graphic() || b == b' ') && !(found.filler && b == 0x55))
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_owned();
    // FFmpeg cuts its name to 9 letters: `Lavc60.3.100` as `Lavc60.3.`.
    let name = name.trim_end_matches('.').to_owned();
    if name.is_empty() {
        return None;
    }
    if found.filler {
        return Some(Encoder {
            name,
            settings: String::new(),
        });
    }
    let tag = tag.get(..25)?;
    let own = name.starts_with("LAME") || name.starts_with("L3.9");
    let settings = if own {
        settings_of(tag, found.quality, frame_kbps(found.header))
    } else {
        String::new()
    };
    Some(Encoder { name, settings })
}

/// The bitrate of the frame whose header starts `header`, in kbps (0: free
/// or not one).
fn frame_kbps(header: &[u8]) -> u32 {
    const MPEG1: [u32; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const MPEG2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let index = usize::from(header[2] >> 4);
    let table = if (header[1] >> 3) & 3 == 3 {
        MPEG1
    } else {
        MPEG2
    };
    table.get(index).copied().unwrap_or(0)
}

/// The settings of a LAME tag `tag` (its first 25 bytes at least), the
/// Xing header's `quality` and the bitrate of the frame holding them, as
/// MediaInfo writes them: the stereo mode, the bitrate mode (`-V`,
/// `--vbr-old -V`, `--abr`, `-b`), `-q`, `-lowpass` in kHz.
fn settings_of(tag: &[u8], quality: Option<u32>, frame_kbps: u32) -> String {
    let method = tag[9] & 0x0f;
    let lowpass = u32::from(tag[10]);
    // The tag's bitrate stops at 255 ("255 or more"); a CBR file's first
    // frame has the bitrate itself.
    let bitrate = match (tag[20], method) {
        (255, 1 | 8) if frame_kbps > 0 => frame_kbps,
        (b, _) => u32::from(b),
    };
    let stereo = (tag[24] >> 2) & 7;
    // V and q from `100 - 10 × V - q`.
    let vq = quality
        .filter(|&q| q <= 100)
        .map(|q| ((100 - q) / 10, (100 - q) % 10));
    let mut parts = Vec::new();
    let mode = match stereo {
        0 => Some("m"),
        1 => Some("s"),
        2 => Some("d"),
        3 => Some("j"),
        4 => Some("f"),
        _ => None,
    };
    if let Some(m) = mode {
        parts.push(format!("-m {m}"));
    }
    let v = vq.map(|(v, _)| format!("-V {v}"));
    match method {
        1 | 8 if bitrate > 0 => parts.push(format!("-b {bitrate}")),
        2 | 9 if bitrate > 0 => parts.push(format!("--abr {bitrate}")),
        3 => {
            parts.push("--vbr-old".to_owned());
            parts.extend(v);
        }
        4..=6 => parts.extend(v),
        _ => {}
    }
    if let Some((_, q)) = vq {
        parts.push(format!("-q {q}"));
    }
    if lowpass > 0 {
        let khz = if lowpass % 10 == 0 {
            format!("{}", lowpass / 10)
        } else {
            format!("{}.{}", lowpass / 10, lowpass % 10)
        };
        parts.push(format!("-lowpass {khz}"));
    }
    parts.join(" ")
}
#[cfg(test)]
mod tests {
    use super::*;

    /// An ID3v2 tag of `body` bytes, then an MPEG 1 Layer III stereo
    /// frame with an Info header (frames counted) and a LAME tag.
    fn file(tag: &[u8; 4], body: usize, frames: u32, delay: u32, padding: u32) -> Vec<u8> {
        let mut v = b"ID3\x03\x00\x00".to_vec();
        v.extend([0, 0, (body >> 7) as u8 & 0x7f, body as u8 & 0x7f]);
        v.extend(std::iter::repeat_n(0u8, body));
        // 128 kbps, 44.1 kHz, stereo.
        v.extend([0xff, 0xfb, 0x90, 0x00]);
        v.extend([0u8; 32]);
        v.extend(tag);
        v.extend(0x0fu32.to_be_bytes());
        v.extend(frames.to_be_bytes());
        v.extend(1_000_000u32.to_be_bytes());
        v.extend([0u8; 100]);
        v.extend(50u32.to_be_bytes());
        let mut lame = b"LAME3.100".to_vec();
        lame.extend([0u8; 12]);
        lame.push((delay >> 4) as u8);
        lame.push((((delay & 0xf) << 4) | (padding >> 8)) as u8);
        lame.push(padding as u8);
        v.extend(lame);
        v.extend([0u8; 64]);
        v
    }

    #[test]
    fn reads_the_lame_tag_after_an_info_header() {
        let data = file(b"Info", 300, 9000, 576, 1424);
        let info = parse(&data).unwrap();
        assert_eq!(
            info,
            GapInfo {
                frames: Some(9000),
                samples_per_frame: 1152,
                delay: 576,
                padding: 1424,
            }
        );
        assert_eq!(info.music_samples(), Some(9000 * 1152 - 576 - 1424));
        assert!(parse(&file(b"Xing", 0, 10, 576, 0)).is_some());
    }

    #[test]
    fn the_encoder_and_its_settings() {
        let mut data = file(b"Info", 0, 10, 576, 924);
        let at = data.windows(4).position(|w| w == b"LAME").unwrap();
        data[at..at + 9].copy_from_slice(b"LAME3.99r");
        // VBR (the new method), 20.5 kHz, joint stereo, quality 100 - 40 - 3.
        data[at + 9] = 0x04;
        data[at + 10] = 205;
        data[at + 24] = 3 << 2;
        data[at - 4..at].copy_from_slice(&57u32.to_be_bytes());
        assert_eq!(
            encoder(&data),
            Some(Encoder {
                name: "LAME3.99r".into(),
                settings: "-m j -V 4 -q 3 -lowpass 20.5".into(),
            })
        );
        // The delay and padding still read.
        assert_eq!(
            (parse(&data).unwrap().delay, parse(&data).unwrap().padding),
            (576, 924)
        );
        // CBR at 128 kbps (the frame's), 20 kHz; and at the tag's own.
        data[at + 9] = 0x01;
        data[at + 10] = 200;
        data[at + 20] = 255;
        assert_eq!(
            encoder(&data).unwrap().settings,
            "-m j -b 128 -q 3 -lowpass 20"
        );
        data[at + 20] = 192;
        assert_eq!(
            encoder(&data).unwrap().settings,
            "-m j -b 192 -q 3 -lowpass 20"
        );
        // The old VBR method.
        data[at + 9] = 0x03;
        assert_eq!(
            encoder(&data).unwrap().settings,
            "-m j --vbr-old -V 4 -q 3 -lowpass 20"
        );
        // FFmpeg's tag: its name only.
        data[at..at + 9].copy_from_slice(b"Lavc58.91");
        assert_eq!(
            encoder(&data),
            Some(Encoder {
                name: "Lavc58.91".into(),
                settings: String::new(),
            })
        );
        // Its name cut short after a dot.
        data[at..at + 9].copy_from_slice(b"Lavc60.3.");
        assert_eq!(encoder(&data).unwrap().name, "Lavc60.3");
        // No header, no encoder.
        assert_eq!(encoder(b"not an mp3"), None);
    }

    #[test]
    fn lames_filler_is_no_tag() {
        // A first frame with LAME's name and filler where a Xing header
        // would be: the name, nothing else.
        let mut v = vec![0xff, 0xfb, 0x90, 0x00];
        v.extend([0u8; 32]);
        v.extend(b"LAME3.88 (alpha)");
        v.extend([0x55; 40]);
        assert_eq!(
            encoder(&v),
            Some(Encoder {
                name: "LAME3.88 (alpha)".into(),
                settings: String::new(),
            })
        );
        assert_eq!(parse(&v), None);
        let mut v = vec![0xff, 0xfb, 0x90, 0x00];
        v.extend([0u8; 32]);
        v.extend(b"LAME3.85");
        v.extend([0x55; 40]);
        assert_eq!(encoder(&v).unwrap().name, "LAME3.85");
        let at = v.len() - 40;
        v[at..].fill(0xaa);
        assert_eq!(
            encoder(&v).unwrap(),
            Encoder {
                name: "LAME3.85".into(),
                settings: String::new()
            }
        );
        assert_eq!(parse(&v), None);
        // After a Xing header: its frames count, no delay or padding.
        let mut data = file(b"Xing", 0, 10, 576, 924);
        let at = data.windows(4).position(|w| w == b"LAME").unwrap();
        data[at + 9..at + 40].fill(0x55);
        let info = parse(&data).unwrap();
        assert_eq!((info.frames, info.delay, info.padding), (Some(10), 0, 0));
        assert_eq!(encoder(&data).unwrap().settings, "");
    }

    /// The encoders of the MP3s under `METALAMP_MUSIC=<folder>`, counted,
    /// to compare with MediaInfo: `cargo test encoders_of_real_files --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn encoders_of_real_files() {
        let root = std::env::var("METALAMP_MUSIC").expect("METALAMP_MUSIC=<folder>");
        let mut seen = std::collections::BTreeMap::<String, (usize, String)>::new();
        for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
            {
                continue;
            }
            let line = encoder(&read_head(path)).map_or("(none)".to_owned(), |e| {
                format!("{} ({})", e.name, e.settings)
            });
            let slot = seen.entry(line).or_insert((0, path.display().to_string()));
            slot.0 += 1;
        }
        for (line, (count, example)) in seen {
            println!("{count:5}  {line}  e.g. {example}");
        }
    }

    #[test]
    fn no_header_no_information() {
        let mut data = file(b"Info", 20, 10, 576, 100);
        // The header's name spoilt: an MP3 without one.
        let at = data.windows(4).position(|w| w == b"Info").unwrap();
        data[at] = b'X';
        data[at + 1] = b'X';
        assert_eq!(parse(&data), None);
        assert_eq!(parse(b"not an mp3"), None);
        // Cut short within the LAME tag.
        let data = file(b"Info", 0, 10, 576, 100);
        assert_eq!(parse(&data[..data.len() - 90]), None);
    }

    #[test]
    fn trim_from_the_tag() {
        let info = parse(&file(b"Info", 0, 7755, 576, 876)).unwrap();
        assert_eq!(
            trim_of(&info),
            Some(Trim {
                start: 1153 + 576,
                length: 7755 * 1152 - 576 - 876,
            })
        );
        // No frame count, or a tag of nonsense: no trim.
        let no_frames = GapInfo {
            frames: None,
            ..info
        };
        assert_eq!(trim_of(&no_frames), None);
        let junk = GapInfo {
            delay: 0x555 * 4,
            ..info
        };
        assert_eq!(trim_of(&junk), None);
    }

    #[test]
    fn id3v2_length_with_and_without_a_footer() {
        assert_eq!(id3v2_len(b"ID3\x04\x00\x00\x00\x00\x01\x00rest"), 10 + 128);
        assert_eq!(
            id3v2_len(b"ID3\x04\x00\x10\x00\x00\x00\x05rest"),
            10 + 5 + 10
        );
        assert_eq!(id3v2_len(b"\xff\xfb\x90\x00"), 0);
        assert_eq!(id3v2_len(b"ID3\x04\x00\x00\x00\x00\x81\x00rest"), 0);
    }

    #[test]
    fn tags_one_after_another() {
        let mut data = b"ID3\x03\x00\x00\x00\x00\x00\x02ab".to_vec();
        data.extend(b"ID3\x04\x00\x00\x00\x00\x00\x01c");
        data.extend(b"\xff\xfb\x90\x00");
        assert_eq!(id3v2_tags_len(&data), 12 + 11);
        let tags = leading_tags(&mut std::io::Cursor::new(&data));
        assert_eq!(tags, [0..12, 12..23]);
        assert!(leading_tags(&mut std::io::Cursor::new(b"\xff\xfb\x90\x00")).is_empty());
        // A second tag that says it is longer than the file: not a tag.
        let mut data = b"ID3\x03\x00\x00\x00\x00\x00\x02ab".to_vec();
        data.extend(b"ID3\x04\x00\x00\x00\x00\x7f\x7fc");
        let tags = leading_tags(&mut std::io::Cursor::new(&data));
        assert_eq!((tags.len(), &tags[0]), (1, &(0..12)));
    }
}
