//! An MP3's gapless information: the Xing/Info header in its first frame
//! (the number of frames) and the LAME tag after it (the encoder's delay
//! and padding). An MP3 encoder adds silence before the music (the delay,
//! 576 samples for LAME) and after it (the padding, to fill the last
//! frame); without cutting them, an album meant to play without pauses
//! clicks or gaps between tracks.

/// Samples Media Foundation's MP3 decoder gives before the encoder's
/// delay: it decodes the Xing/Info frame as silence (1152) and leaves out
/// the first 528 of its own delay (529). Measured with `--bench gapless`:
/// it gives the frames' samples plus 624 (1152 - 528), the end ends
/// `padding - 529` samples into the padding, and the music starts 1153 +
/// the delay into what it gives.
pub const DECODER_OFFSET: u64 = 1153;

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
        start: DECODER_OFFSET + u64::from(info.delay),
        length: info.music_samples()?,
    })
}

/// The start of `file`: its ID3v2 tag and 8 KB after it.
pub fn read_head(file: &std::path::Path) -> Vec<u8> {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(file) else {
        return Vec::new();
    };
    let mut head = vec![0u8; 10];
    if f.read_exact(&mut head).is_err() {
        return Vec::new();
    }
    let len = id3v2_len(&head).max(10);
    let mut rest = Vec::new();
    let _ = f.take((len - 10 + 8192) as u64).read_to_end(&mut rest);
    head.extend(rest);
    head
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
/// included), 0 without one.
pub fn id3v2_len(data: &[u8]) -> usize {
    if data.len() < 10 || &data[..3] != b"ID3" {
        return 0;
    }
    let size = data[6..10]
        .iter()
        .fold(0usize, |acc, &b| (acc << 7) | usize::from(b & 0x7f));
    let footer = if data[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

/// The gapless information of an MP3 whose start (past the ID3v2 tag,
/// whose length is `id3v2_len`) is `data`; `None` without a Xing/Info
/// header with a LAME tag.
pub fn parse(data: &[u8]) -> Option<GapInfo> {
    let start = id3v2_len(data);
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
    let (frames, lame) = if tag == b"Xing" || tag == b"Info" {
        let flags = u32_at(x + 4)?;
        let mut p = x + 8;
        let mut frames = None;
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
            p += 4;
        }
        (frames, p)
    } else if tag == b"LAME" || tag == b"Lavc" || tag == b"Lavf" {
        // A LAME tag where the Xing header would be, without one.
        (None, x)
    } else {
        return None;
    };
    let name = header.get(lame..lame + 4)?;
    if name != b"LAME" && name != b"Lavc" && name != b"Lavf" && name != b"L3.9" {
        return Some(GapInfo {
            frames,
            samples_per_frame,
            delay: 0,
            padding: 0,
        });
    }
    // 12 bits of delay and 12 of padding, 21 bytes into the LAME tag.
    let d = header.get(lame + 21..lame + 24)?;
    let delay = (u32::from(d[0]) << 4) | (u32::from(d[1]) >> 4);
    let padding = ((u32::from(d[1]) & 0x0f) << 8) | u32::from(d[2]);
    Some(GapInfo {
        frames,
        samples_per_frame,
        delay,
        padding,
    })
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
    }
}
