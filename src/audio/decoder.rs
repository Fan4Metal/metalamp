//! A file decoded to interleaved 32-bit float by Media Foundation's source
//! reader (qrec's `trim::Sound`, for audio files): Windows' own decoders
//! for MP3, AAC (MP4 and ADTS), FLAC, WMA and WAV, the reader converting
//! their output to float.

use std::collections::VecDeque;
use std::path::Path;

use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Media::MediaFoundation::{
    IMFSourceReader, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_MEDIASOURCE,
    MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READERF_ERROR, MFAudioFormat_Float, MFCreateMediaType, MFCreateSourceReaderFromURL,
    MFMediaType_Audio,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::core::{GUID, PCWSTR, Result};

/// One second in the 100 ns units of Media Foundation.
pub const SECOND: i64 = 10_000_000;

pub struct Decoder {
    /// None while released (`release`): the file is closed.
    reader: Option<IMFSourceReader>,
    pub rate: u32,
    pub channels: u16,
    /// The file's length in 100 ns units, 0 if it does not say.
    pub duration: i64,
    /// Samples before this time (100 ns) are dropped: what a seek asked
    /// for, as the reader starts at the frame that holds it.
    skip_to: Option<i64>,
    /// An MP3's music among the samples decoded (`mp3gap`): what is
    /// before and after it is the encoder's silence, cut for gapless
    /// playback.
    trim: Option<super::mp3gap::Trim>,
    /// The index of the next sample the reader gives (per channel).
    at: u64,
    /// The first block, read at the opening to learn the format the
    /// decoder settles on (`first_block`).
    pending: Option<Vec<f32>>,
    /// The last samples given (`TAIL` frames), by which `reacquire` finds
    /// where it was.
    tail: VecDeque<f32>,
    /// Samples read past that place by `reacquire`, given next (through
    /// the trim, as the reader's would be).
    carry: Option<Vec<f32>>,
}

/// The frames `Decoder::tail` keeps.
const TAIL: usize = 1024;
/// How far before where it was `reacquire` starts looking, and how much
/// it decodes looking, in seconds: Media Foundation's seek in a VBR MP3
/// lands a second or so off (measured: 1.04 s), its times after it too.
const LOOK_BACK: i64 = 4;
const LOOK_FOR: usize = 12;

const STREAM: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

impl Decoder {
    /// Media Foundation must be started on this thread (`mf::Mf`).
    pub fn open(path: &Path) -> Result<Decoder> {
        let url = crate::win::wide(path);
        let reader = unsafe { MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None)? };
        unsafe {
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(STREAM, true)?;
            let float = MFCreateMediaType()?;
            float.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            float.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
            reader.SetCurrentMediaType(STREAM, None, &float)?;
        }
        let (mut rate, mut channels) = format_of(&reader)?;
        let duration = unsafe {
            reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
        }
        .ok()
        .and_then(|d| u64::try_from(&d).ok())
        .unwrap_or(0) as i64;
        let is_mp3 = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
        let trim = if is_mp3 {
            let head = super::mp3gap::read_head(path);
            super::mp3gap::parse(&head).and_then(|i| super::mp3gap::trim_of(&i))
        } else {
            None
        };
        // An AAC with SBR not signalled in its header (HE-AAC) is decoded
        // at twice the rate the file says, which the reader tells with the
        // first sample: read, so the output is opened for what comes. Not
        // an MP3, whose samples are counted from the start for the trim.
        let mut pending = None;
        if !is_mp3 {
            let (block, changed) = first_block(&reader)?;
            if changed {
                (rate, channels) = format_of(&reader)?;
                log::debug!("{}: decoded as {rate} Hz, {channels} ch", path.display());
            }
            pending = block;
        }
        // The music's length, not the encoder's padding with it.
        let duration = match trim {
            Some(t) if rate > 0 => {
                (i128::from(t.length) * i128::from(SECOND) / i128::from(rate)) as i64
            }
            _ => duration,
        };
        Ok(Decoder {
            reader: Some(reader),
            rate,
            channels,
            duration,
            skip_to: None,
            trim,
            at: 0,
            pending,
            tail: VecDeque::new(),
            carry: None,
        })
    }

    /// The reader's time of the music's time `time` (both 100 ns).
    fn reader_time(&self, time: i64) -> i64 {
        match self.trim {
            Some(t) if self.rate > 0 => {
                time + (i128::from(t.start) * i128::from(SECOND) / i128::from(self.rate)) as i64
            }
            _ => time,
        }
    }

    /// The file closed (its tag is being written), the format, the length
    /// and the last samples given kept; nothing is read until `reacquire`.
    pub fn release(&mut self) {
        self.reader = None;
        // Not given yet: read again with the rest.
        self.pending = None;
        self.skip_to = None;
        self.carry = None;
    }

    pub fn released(&self) -> bool {
        self.reader.is_none()
    }

    /// The file at `path` opened again, going on where the decoder was
    /// released, about `frames` into the music (as `seek` counts): found
    /// by the last samples it gave, decoded again from a little before
    /// (the file's tag may have grown, the audio moved; a seek by time is
    /// a second off in a VBR MP3), else by the time. Fails if it now
    /// decodes to another format.
    pub fn reacquire(&mut self, path: &Path, frames: u64) -> Result<()> {
        let fresh = Decoder::open(path)?;
        if (fresh.rate, fresh.channels) != (self.rate, self.channels) {
            return Err(E_FAIL.into());
        }
        let (at, mut tail) = (self.at, std::mem::take(&mut self.tail));
        *self = fresh;
        if frames == 0 && tail.is_empty() {
            return Ok(());
        }
        if self.find(frames, tail.make_contiguous())? {
            self.at = at;
            self.tail = tail;
            return Ok(());
        }
        log::debug!("audio: the place released not found again; by the time");
        let time = i128::from(frames) * i128::from(SECOND) / i128::from(self.rate.max(1));
        self.seek(time as i64)
    }

    /// Decodes from `LOOK_BACK` seconds before `frames` until the samples
    /// `tail` ends with come; what follows them is carried. Whether they
    /// came within `LOOK_FOR` seconds.
    fn find(&mut self, frames: u64, tail: &[f32]) -> Result<bool> {
        let channels = usize::from(self.channels.max(1));
        let Some(reader) = self.reader.clone() else {
            return Ok(false);
        };
        // Too few to be sure of the place (just after a seek), or silence,
        // which any silence would match.
        if tail.len() < 256 * channels || tail.iter().all(|x| x.abs() <= 1e-6) {
            return Ok(false);
        }
        let back = i128::from(frames) * i128::from(SECOND) / i128::from(self.rate.max(1));
        let back = back as i64 - LOOK_BACK * SECOND;
        let mut buf: Vec<f32> = self.pending.take().unwrap_or_default();
        if back > 0 {
            let position = PROPVARIANT::from(self.reader_time(back));
            unsafe { reader.SetCurrentPosition(&GUID::zeroed(), &position)? };
            buf.clear();
        }
        let limit = LOOK_FOR * self.rate as usize * channels;
        let mut searched = 0;
        loop {
            // Every place the tail may end at, among the samples so far.
            let mut end = (searched + tail.len()).max(tail.len());
            while end <= buf.len() {
                if same(&buf[end - tail.len()..end], tail) {
                    self.carry = Some(buf[end..].to_vec());
                    return Ok(true);
                }
                end += channels;
            }
            searched = (end - tail.len()).min(buf.len().saturating_sub(tail.len()));
            if buf.len() >= limit {
                // Not the same samples (an AAC decoder's after a seek are
                // about 1 % off): the closest place, if far closer than
                // any other.
                return Ok(match closest(&buf, tail, channels) {
                    Some(end) => {
                        self.carry = Some(buf[end..].to_vec());
                        true
                    }
                    None => false,
                });
            }
            match read(&reader)? {
                Some((floats, _)) => buf.extend(floats),
                None => return Ok(false),
            }
        }
    }

    /// The next samples come from `time` (100 ns) on.
    pub fn seek(&mut self, time: i64) -> Result<()> {
        let Some(reader) = &self.reader else {
            return Err(E_FAIL.into());
        };
        let time = self.reader_time(time.max(0));
        let position = PROPVARIANT::from(time);
        unsafe { reader.SetCurrentPosition(&GUID::zeroed(), &position)? };
        self.skip_to = Some(time);
        self.pending = None;
        self.carry = None;
        self.tail.clear();
        Ok(())
    }

    /// The next block of samples; `None` at the end.
    pub fn next(&mut self) -> Result<Option<Vec<f32>>> {
        let block = self.next_block()?;
        if let Some(block) = &block {
            let keep = TAIL * usize::from(self.channels.max(1));
            self.tail.extend(block.iter().rev().take(keep).rev());
            let over = self.tail.len().saturating_sub(keep);
            self.tail.drain(..over);
        }
        Ok(block)
    }

    fn next_block(&mut self) -> Result<Option<Vec<f32>>> {
        if let Some(block) = self.pending.take() {
            return Ok(Some(block));
        }
        let Some(reader) = self.reader.clone() else {
            return Err(E_FAIL.into());
        };
        loop {
            let (mut floats, time) = match self.carry.take() {
                Some(carried) => (carried, None),
                None => match read(&reader)? {
                    Some((floats, time)) => (floats, Some(time)),
                    None => return Ok(None),
                },
            };
            if let Some(to) = self.skip_to
                && let Some(time) = time
            {
                // What comes before the time sought, in the block that
                // holds it.
                let channels = usize::from(self.channels.max(1));
                let skip = ((to - time).max(0) as i128 * i128::from(self.rate) / i128::from(SECOND))
                    as usize
                    * channels;
                if skip >= floats.len() {
                    continue;
                }
                floats.drain(..skip);
                self.skip_to = None;
                // The first sample kept: the one sought, or the block's first
                // when the reader came out past it.
                self.at =
                    (i128::from(to.max(time)) * i128::from(self.rate) / i128::from(SECOND)) as u64;
            }
            let Some(trim) = self.trim else {
                return Ok(Some(floats));
            };
            // Only the music: the encoder's delay before it and its
            // padding after it are cut.
            let channels = usize::from(self.channels.max(1));
            let frames = (floats.len() / channels) as u64;
            let (from, to) = (self.at, self.at + frames);
            self.at = to;
            let end = trim.start + trim.length;
            if from >= end {
                return Ok(None);
            }
            let keep_from = trim.start.saturating_sub(from).min(frames) as usize;
            let keep_to = (end.min(to) - from) as usize;
            if keep_from >= keep_to {
                continue;
            }
            floats.truncate(keep_to * channels);
            floats.drain(..keep_from * channels);
            return Ok(Some(floats));
        }
    }
}

/// Two runs of samples the same (a decoder gives the same floats for the
/// same frames).
fn same(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-6)
}

/// Where in `buf` the run most like `tail` ends: its squared error under
/// 5 % of the tail's energy, and that of any place more than a block from
/// it over 30 % (measured on an AAC: 1.1 % and 94 %); else none.
fn closest(buf: &[f32], tail: &[f32], channels: usize) -> Option<usize> {
    let energy: f32 = tail.iter().map(|x| x * x).sum();
    let apart = 2048 * channels;
    let (mut best, mut at, mut second) = (f32::MAX, 0, f32::MAX);
    let mut end = tail.len();
    while end <= buf.len() {
        let mut sse = 0.0f32;
        // Given up once it is no better than the two so far.
        let bound = second.min(energy);
        for (x, y) in buf[end - tail.len()..end].iter().zip(tail) {
            sse += (x - y) * (x - y);
            if sse > bound {
                break;
            }
        }
        if sse < best {
            if end.abs_diff(at) > apart {
                second = best;
            }
            (best, at) = (sse, end);
        } else if sse < second && end.abs_diff(at) > apart {
            second = sse;
        }
        end += channels;
    }
    (best < 0.05 * energy && second > 0.3 * energy).then_some(at)
}

/// The rate and channels the reader gives now.
fn format_of(reader: &IMFSourceReader) -> Result<(u32, u16)> {
    let current = unsafe { reader.GetCurrentMediaType(STREAM)? };
    let rate = unsafe { current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)? };
    let channels = unsafe { current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? } as u16;
    Ok((rate, channels))
}

/// The first block and whether the reader changed its format before it.
fn first_block(reader: &IMFSourceReader) -> Result<(Option<Vec<f32>>, bool)> {
    let mut changed = false;
    let block = read_with(reader, &mut |flags| {
        changed |= flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0;
    })?;
    Ok((block.map(|(floats, _)| floats), changed))
}

/// The next sample's floats and time; `None` at the end. An error the
/// reader reports in its flags is an error: it gives nothing more.
fn read(reader: &IMFSourceReader) -> Result<Option<(Vec<f32>, i64)>> {
    read_with(reader, &mut |_| {})
}

fn read_with(
    reader: &IMFSourceReader,
    seen: &mut dyn FnMut(u32),
) -> Result<Option<(Vec<f32>, i64)>> {
    loop {
        let (mut flags, mut time, mut sample) = (0u32, 0i64, None);
        unsafe {
            reader.ReadSample(
                STREAM,
                0,
                None,
                Some(&mut flags),
                Some(&mut time),
                Some(&mut sample),
            )?
        };
        seen(flags);
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            return Err(E_FAIL.into());
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Ok(None);
        }
        let Some(sample) = sample else { continue };
        let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
        let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
        unsafe { buffer.Lock(&mut data, None, Some(&mut length))? };
        let bytes = unsafe { std::slice::from_raw_parts(data, length as usize) };
        let floats: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        unsafe { buffer.Unlock()? };
        return Ok(Some((floats, time)));
    }
}
