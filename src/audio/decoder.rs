//! A file decoded to interleaved 32-bit float by Media Foundation's source
//! reader (qrec's `trim::Sound`, for audio files): Windows' own decoders
//! for MP3, AAC (MP4 and ADTS), FLAC, WMA and WAV, the reader converting
//! their output to float.

use std::path::Path;

use windows::Win32::Media::MediaFoundation::{
    IMFSourceReader, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_PD_DURATION, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_MEDIASOURCE,
    MF_SOURCE_READERF_ENDOFSTREAM, MFAudioFormat_Float, MFCreateMediaType,
    MFCreateSourceReaderFromURL, MFMediaType_Audio,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::core::{GUID, PCWSTR, Result};

/// One second in the 100 ns units of Media Foundation.
pub const SECOND: i64 = 10_000_000;

pub struct Decoder {
    reader: IMFSourceReader,
    pub rate: u32,
    pub channels: u16,
    /// The file's length in 100 ns units, 0 if it does not say.
    pub duration: i64,
    /// Samples before this time (100 ns) are dropped: what a seek asked
    /// for, as the reader starts at the frame that holds it.
    skip_to: Option<i64>,
}

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
        let current = unsafe { reader.GetCurrentMediaType(STREAM)? };
        let rate = unsafe { current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)? };
        let channels = unsafe { current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? } as u16;
        let duration = unsafe {
            reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
        }
        .ok()
        .and_then(|d| u64::try_from(&d).ok())
        .unwrap_or(0) as i64;
        Ok(Decoder {
            reader,
            rate,
            channels,
            duration,
            skip_to: None,
        })
    }

    /// The next samples come from `time` (100 ns) on.
    pub fn seek(&mut self, time: i64) -> Result<()> {
        let time = time.max(0);
        let position = PROPVARIANT::from(time);
        unsafe { self.reader.SetCurrentPosition(&GUID::zeroed(), &position)? };
        self.skip_to = Some(time);
        Ok(())
    }

    /// The next block of samples; `None` at the end.
    pub fn next(&mut self) -> Result<Option<Vec<f32>>> {
        loop {
            let (mut flags, mut time, mut sample) = (0u32, 0i64, None);
            unsafe {
                self.reader.ReadSample(
                    STREAM,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut time),
                    Some(&mut sample),
                )?
            };
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
            let Some(sample) = sample else { continue };
            let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
            let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
            unsafe { buffer.Lock(&mut data, None, Some(&mut length))? };
            let bytes = unsafe { std::slice::from_raw_parts(data, length as usize) };
            let mut floats: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            unsafe { buffer.Unlock()? };
            if let Some(to) = self.skip_to {
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
            }
            return Ok(Some(floats));
        }
    }
}
