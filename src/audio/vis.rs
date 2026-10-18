//! The tap for the visualization: a copy of the samples written to the
//! device, mixed to mono, by their frame index, and the device's clock, so
//! the UI can take the samples heard at this moment (the device plays
//! what was written up to half a second ago).

/// Frames kept: at 48 kHz about 5 s, enough for the device's buffer and
/// the window asked for at any rate up to 192 kHz.
const CAP: usize = 1 << 18;

/// What `Tap::latest` gave: the sample rate, and the frame index (the
/// device's count) past the last frame, so a caller can tell how many
/// frames are new since it last asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heard {
    pub rate: u32,
    pub end: u64,
}

pub struct Tap {
    buf: Vec<f32>,
    /// The frame index (the device's count) past the last sample pushed.
    end: u64,
    /// How many of the last frames are valid (at most `CAP`).
    filled: u64,
    /// The device's frames heard so far.
    heard: u64,
    rate: u32,
}

impl Default for Tap {
    fn default() -> Self {
        Tap {
            buf: vec![0.0; CAP],
            end: 0,
            filled: 0,
            heard: 0,
            rate: 0,
        }
    }
}

impl Tap {
    /// `samples` of `channels` interleaved, written at frame `start`. A
    /// `start` other than the end so far is a new count (the device was
    /// reset): what was kept is dropped.
    pub fn push(&mut self, samples: &[f32], channels: usize, start: u64) {
        let channels = channels.max(1);
        if start != self.end {
            self.end = start;
            self.filled = 0;
        }
        let mut frames = 0;
        for frame in samples.chunks_exact(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            self.buf[(self.end % CAP as u64) as usize] = mono;
            self.end += 1;
            frames += 1;
        }
        self.filled = (self.filled + frames).min(CAP as u64);
    }

    pub fn set_heard(&mut self, heard: u64, rate: u32) {
        self.heard = heard;
        self.rate = rate;
    }

    /// Nothing plays: the window is silence.
    pub fn clear(&mut self) {
        self.end = 0;
        self.filled = 0;
        self.heard = 0;
    }

    /// The `n` frames heard last into `out`, the sample rate and where
    /// they end; `None` when fewer than `n` were heard since the device
    /// started.
    pub fn latest(&self, n: usize, out: &mut Vec<f32>) -> Option<Heard> {
        let end = self.heard.min(self.end);
        let oldest = self.end - self.filled;
        if end < oldest + n as u64 {
            return None;
        }
        out.clear();
        out.extend((end - n as u64..end).map(|i| self.buf[(i % CAP as u64) as usize]));
        Some(Heard {
            rate: self.rate,
            end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gives_the_frames_heard_last() {
        let mut tap = Tap::default();
        // Stereo frames 0..100, each frame's value its index.
        let samples: Vec<f32> = (0..100).flat_map(|i| [i as f32, i as f32]).collect();
        tap.push(&samples, 2, 0);
        let mut out = Vec::new();
        // Nothing heard yet.
        assert_eq!(tap.latest(4, &mut out), None);
        tap.set_heard(50, 44100);
        assert_eq!(
            tap.latest(4, &mut out),
            Some(Heard {
                rate: 44100,
                end: 50
            })
        );
        assert_eq!(out, [46.0, 47.0, 48.0, 49.0]);
        // The clock cannot be past what was written.
        tap.set_heard(1000, 44100);
        tap.latest(2, &mut out);
        assert_eq!(out, [98.0, 99.0]);
        // A new count drops the old frames.
        tap.push(&[7.0, 7.0], 2, 0);
        tap.set_heard(1, 44100);
        assert_eq!(tap.latest(1, &mut out).map(|h| h.rate), Some(44100));
        assert_eq!(out, [7.0]);
        assert_eq!(tap.latest(2, &mut out), None);
        tap.clear();
        assert_eq!(tap.latest(1, &mut out), None);
    }

    #[test]
    fn wraps_around() {
        let mut tap = Tap::default();
        let block: Vec<f32> = (0..CAP as u32).map(|i| i as f32).collect();
        tap.push(&block, 1, 0);
        tap.push(&[-1.0, -2.0], 1, CAP as u64);
        tap.set_heard(CAP as u64 + 2, 48000);
        let mut out = Vec::new();
        assert_eq!(tap.latest(3, &mut out).map(|h| h.rate), Some(48000));
        assert_eq!(out, [(CAP - 1) as f32, -1.0, -2.0]);
        // Only CAP frames are kept.
        assert_eq!(tap.latest(CAP + 1, &mut out), None);
        assert_eq!(tap.latest(CAP, &mut out).map(|h| h.rate), Some(48000));
    }
}
