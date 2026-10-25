//! The notes heard, for the piano and the chromagram: the samples heard
//! now through a Hann window and an FFT of `N` (8192) frames, whose bins
//! (5.4 Hz at 44.1 kHz) are finer than a semitone above about 100 Hz.
//! A note's level is the loudest bin within half a semitone of it (between
//! the two bins around it when narrower than a bin), on the spectrum's
//! scale and tilt; a pitch class's is the power of its notes from C3 to
//! B7 added up, relative to the loudest class (a chromagram shows which
//! notes sound, not how loud), and nothing under `CHROMA_GATE_DB`.

use std::f32::consts::PI;
use std::time::Instant;

/// The window: 8192 frames, 186 ms at 44.1 kHz.
pub const N: usize = 8192;
/// The piano's notes, MIDI: C1 to C8.
pub const LOWEST: u8 = 24;
pub const HIGHEST: u8 = 108;
/// The chromagram's notes, MIDI: C3 to B7 (lower ones blur into their
/// neighbours, a semitone narrower than the window's main lobe).
const CHROMA_LO: u8 = 48;
const CHROMA_HI: u8 = 107;
/// The keys' scale, as the spectrum's: the floor under a full key, the
/// level of a full key, the tilt per octave about 500 Hz.
const RANGE_DB: f32 = 60.0;
const TOP_DB: f32 = -6.0;
const TILT_DB_PER_OCTAVE: f32 = 3.0;
const TILT_REF_HZ: f32 = 500.0;
/// The chromagram: the decibels shown under the loudest class, and the
/// loudest class's level under which it is taken for silence.
const CHROMA_RANGE_DB: f32 = 12.0;
const CHROMA_GATE_DB: f32 = -60.0;
/// A key's level falls the whole way in this many seconds; a class's
/// slower, so a chord stays readable between its strokes.
const FALL_S: f32 = 0.45;
const CHROMA_FALL_S: f32 = 0.7;

/// The pitch classes' names, C first.
pub const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Whether pitch class `class` (0: C) is a black key.
pub fn is_black(class: usize) -> bool {
    matches!(class, 1 | 3 | 6 | 8 | 10)
}

pub struct Notes {
    window: Vec<f32>,
    twiddles: Vec<(f32, f32)>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Each note's level from `LOWEST` to `HIGHEST`, 0 to 1.
    pub keys: Vec<f32>,
    /// Each pitch class's level, C first, 0 to 1.
    pub chroma: [f32; 12],
    last: Instant,
}

impl Default for Notes {
    fn default() -> Self {
        Self::new()
    }
}

impl Notes {
    pub fn new() -> Notes {
        let window = (0..N)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / N as f32).cos())
            .collect();
        let twiddles = (0..N / 2)
            .map(|k| {
                let a = -2.0 * PI * k as f32 / N as f32;
                (a.cos(), a.sin())
            })
            .collect();
        Notes {
            window,
            twiddles,
            re: vec![0.0; N],
            im: vec![0.0; N],
            keys: vec![0.0; usize::from(HIGHEST - LOWEST) + 1],
            chroma: [0.0; 12],
            last: Instant::now(),
        }
    }

    /// Moves the levels on with the samples heard now (`N` of them at
    /// `rate`; `None`: silence): up at once, down at their speed.
    pub fn feed(&mut self, samples: Option<&[f32]>, rate: u32) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let (keys, chroma) = match samples {
            Some(s) if s.len() == N && rate > 0 => self.measure(s, rate),
            _ => (vec![0.0; self.keys.len()], [0.0; 12]),
        };
        for (key, level) in self.keys.iter_mut().zip(keys) {
            *key = level.max(*key - dt / FALL_S).max(0.0);
        }
        for (class, level) in self.chroma.iter_mut().zip(chroma) {
            *class = level.max(*class - dt / CHROMA_FALL_S).max(0.0);
        }
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.keys.iter().chain(&self.chroma).any(|&l| l > 0.0)
    }

    /// The keys' and the classes' levels of `samples` at `rate`.
    fn measure(&mut self, samples: &[f32], rate: u32) -> (Vec<f32>, [f32; 12]) {
        for ((re, im), (s, w)) in self
            .re
            .iter_mut()
            .zip(&mut self.im)
            .zip(samples.iter().zip(&self.window))
        {
            *re = s * w;
            *im = 0.0;
        }
        super::spectrum::fft(&mut self.re, &mut self.im, &self.twiddles);
        let bin_hz = rate as f32 / N as f32;
        let nyquist = rate as f32 / 2.0;
        let magnitude = |k: usize| (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt();
        // A note's amplitude, a full-scale sine 1 (a Hann window halves
        // its peak, the FFT gives N/2 of it).
        let amplitude = |m: u8| {
            let f = frequency(m);
            let (lo, hi) = (f * 2f32.powf(-1.0 / 24.0), f * 2f32.powf(1.0 / 24.0));
            if hi >= nyquist * 0.95 {
                return 0.0;
            }
            let (k0, k1) = (lo / bin_hz, hi / bin_hz);
            let mag = if k1 - k0 < 1.0 {
                let c = f / bin_hz;
                let k = (c as usize).min(N / 2 - 2);
                let t = c - k as f32;
                magnitude(k) * (1.0 - t) + magnitude(k + 1) * t
            } else {
                let first = (k0.ceil() as usize).min(N / 2 - 1);
                let last = (k1.floor() as usize).clamp(first, N / 2 - 1);
                (first..=last).map(magnitude).fold(0.0, f32::max)
            };
            mag * 4.0 / N as f32
        };
        let keys = (LOWEST..=HIGHEST)
            .map(|m| {
                let db = 20.0 * amplitude(m).max(1e-9).log10();
                let tilt = TILT_DB_PER_OCTAVE * (frequency(m) / TILT_REF_HZ).log2();
                ((db + tilt - TOP_DB + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0)
            })
            .collect();
        let mut power = [0.0f32; 12];
        for m in CHROMA_LO..=CHROMA_HI {
            let a = amplitude(m);
            power[usize::from(m % 12)] += a * a;
        }
        let db = power.map(|p| 10.0 * p.max(1e-18).log10());
        let loudest = db.iter().copied().fold(f32::MIN, f32::max);
        let chroma = if loudest < CHROMA_GATE_DB {
            [0.0; 12]
        } else {
            db.map(|d| ((d - loudest + CHROMA_RANGE_DB) / CHROMA_RANGE_DB).clamp(0.0, 1.0))
        };
        (keys, chroma)
    }
}

/// The frequency of MIDI note `m` (A4, 69, at 440 Hz).
pub fn frequency(m: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(m) - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freqs: &[f32], rate: u32) -> Vec<f32> {
        (0..N)
            .map(|i| {
                freqs
                    .iter()
                    .map(|f| 0.3 * (2.0 * PI * f * i as f32 / rate as f32).sin())
                    .sum()
            })
            .collect()
    }

    #[test]
    fn an_a_lights_its_key_and_its_class() {
        let mut notes = Notes::new();
        let rate = 44100;
        notes.feed(Some(&tone(&[440.0], rate)), rate);
        let (loudest, _) = notes
            .keys
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        assert_eq!(loudest + usize::from(LOWEST), 69);
        // The neighbours a semitone off are far under it.
        let a = usize::from(69 - LOWEST);
        assert!(
            notes.keys[a] - notes.keys[a + 1] > 0.3,
            "{:?}",
            &notes.keys[a - 2..a + 3]
        );
        assert_eq!(notes.chroma[9], 1.0);
        for (c, &l) in notes.chroma.iter().enumerate() {
            if c != 9 {
                assert!(l < 0.3, "{} at {l}", NAMES[c]);
            }
        }
    }

    #[test]
    fn a_chord_lights_its_three_classes() {
        let mut notes = Notes::new();
        let rate = 48000;
        // C major: C4, E4, G4.
        notes.feed(
            Some(&tone(&[frequency(60), frequency(64), frequency(67)], rate)),
            rate,
        );
        for c in [0, 4, 7] {
            assert!(notes.chroma[c] > 0.8, "{} at {}", NAMES[c], notes.chroma[c]);
        }
        for c in [1, 2, 3, 5, 6, 8, 9, 10, 11] {
            assert!(notes.chroma[c] < 0.4, "{} at {}", NAMES[c], notes.chroma[c]);
        }
    }

    #[test]
    fn silence_shows_nothing_and_levels_fall() {
        let mut notes = Notes::new();
        notes.feed(Some(&vec![0.0; N]), 44100);
        assert!(!notes.active());
        notes.feed(Some(&tone(&[1000.0], 44100)), 44100);
        assert!(notes.active());
        notes.last = Instant::now() - std::time::Duration::from_millis(100);
        let before = notes.chroma;
        notes.feed(None, 0);
        assert!(notes.chroma.iter().zip(before).all(|(&a, b)| a <= b));
        assert!(notes.active());
        assert!(is_black(1) && !is_black(4) && frequency(69) == 440.0);
    }
}
