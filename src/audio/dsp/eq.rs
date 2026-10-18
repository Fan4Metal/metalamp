//! The ten-band graphic equalizer with Winamp's bands (60 Hz to 16 kHz)
//! and its preamp: a peaking filter per band and channel, a band at 0 dB
//! left out.

use super::biquad::{Biquad, Coeffs, db_to_gain};

/// Winamp's bands, in hertz.
pub const BANDS: [f32; 10] = [
    60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0,
];
/// The bells' width: about 1.4 octaves, so neighbouring bands join
/// without a dip.
const Q: f32 = 1.0;
/// A band's gain is kept within this many decibels either way.
pub const RANGE_DB: f32 = 12.0;

/// Winamp's presets (its `winamp.q1`), in decibels of its ±12 dB scale;
/// the names as they are shown and in Russian.
pub const PRESETS: [(&str, &str, [f32; 10]); 17] = [
    (
        "Classical",
        "Классика",
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -4.5, -4.5, -4.5, -6.0],
    ),
    (
        "Club",
        "Клуб",
        [0.0, 0.0, 5.0, 3.5, 3.5, 3.5, 2.0, 0.0, 0.0, 0.0],
    ),
    (
        "Dance",
        "Танцевальная",
        [6.0, 4.5, 1.5, 0.0, 0.0, -3.5, -4.5, -4.5, 0.0, 0.0],
    ),
    (
        "Full bass",
        "Полный бас",
        [-5.0, 6.0, 6.0, 3.5, 1.0, -2.5, -5.0, -6.5, -7.0, -7.0],
    ),
    (
        "Full bass and treble",
        "Бас и верх",
        [4.5, 3.5, 0.0, -4.5, -3.0, 1.0, 5.0, 7.0, 7.5, 7.5],
    ),
    (
        "Full treble",
        "Полный верх",
        [-6.0, -6.0, -6.0, -2.5, 1.5, 7.0, 10.0, 10.0, 10.0, 10.5],
    ),
    (
        "Laptop speakers, headphones",
        "Ноутбук, наушники",
        [3.0, 7.0, 3.5, -2.0, -1.5, 1.0, 3.0, 6.0, 8.0, 9.0],
    ),
    (
        "Large hall",
        "Большой зал",
        [6.5, 6.5, 3.5, 3.5, 0.0, -3.0, -3.0, -3.0, 0.0, 0.0],
    ),
    (
        "Live",
        "Концерт",
        [-3.0, 0.0, 2.5, 3.5, 3.5, 3.5, 2.5, 1.5, 1.5, 1.5],
    ),
    (
        "Party",
        "Вечеринка",
        [4.5, 4.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 4.5, 4.5],
    ),
    (
        "Pop",
        "Поп",
        [-1.0, 3.0, 4.5, 5.0, 3.5, 0.0, -1.5, -1.5, -1.0, -1.0],
    ),
    (
        "Reggae",
        "Регги",
        [0.0, 0.0, 0.0, -3.5, 0.0, 4.0, 4.0, 0.0, 0.0, 0.0],
    ),
    (
        "Rock",
        "Рок",
        [5.0, 3.0, -3.5, -5.0, -2.0, 2.5, 5.5, 7.0, 7.0, 7.0],
    ),
    (
        "Ska",
        "Ска",
        [-1.5, -3.0, -2.5, 0.0, 2.5, 3.5, 5.5, 6.0, 7.0, 6.0],
    ),
    (
        "Soft",
        "Мягкая",
        [3.0, 1.0, 0.0, -1.5, 0.0, 2.5, 5.0, 6.0, 7.0, 7.5],
    ),
    (
        "Soft rock",
        "Мягкий рок",
        [2.5, 2.5, 1.5, 0.0, -2.5, -3.5, -2.0, 0.0, 1.5, 5.5],
    ),
    (
        "Techno",
        "Техно",
        [5.0, 3.5, 0.0, -3.5, -3.0, 0.0, 5.0, 6.0, 6.0, 5.5],
    ),
];

pub struct Equalizer {
    rate: u32,
    channels: usize,
    /// One filter per band and channel: `filters[band][channel]`.
    filters: Vec<Vec<Biquad>>,
    /// The bands in use (not at 0 dB, under the Nyquist frequency).
    used: Vec<usize>,
    preamp: f32,
}

impl Equalizer {
    pub fn new(rate: u32, channels: usize, preamp_db: f32, gains: &[f32; 10]) -> Equalizer {
        let mut eq = Equalizer {
            rate,
            channels,
            filters: vec![vec![Biquad::default(); channels]; BANDS.len()],
            used: Vec::new(),
            preamp: 1.0,
        };
        eq.configure(preamp_db, gains);
        eq
    }

    /// New gains; the filters' state is kept.
    pub fn configure(&mut self, preamp_db: f32, gains: &[f32; 10]) {
        self.preamp = db_to_gain(preamp_db.clamp(-RANGE_DB, RANGE_DB));
        self.used.clear();
        for (b, &f0) in BANDS.iter().enumerate() {
            let gain = gains[b].clamp(-RANGE_DB, RANGE_DB);
            if gain.abs() < 0.01 || f0 >= self.rate as f32 * 0.45 {
                continue;
            }
            let c = Coeffs::peaking(self.rate, f0, Q, gain);
            for f in &mut self.filters[b] {
                f.set(c);
            }
            self.used.push(b);
        }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(self.channels) {
            for (ch, x) in frame.iter_mut().enumerate() {
                let mut y = *x * self.preamp;
                for &b in &self.used {
                    y = self.filters[b][ch].run(y);
                }
                *x = y;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::biquad::gain_to_db;
    use super::*;
    use std::f32::consts::PI;

    /// The gain of `eq` for a stereo sine of `f` Hz.
    fn gain_at(eq: &mut Equalizer, rate: u32, f: f32) -> f32 {
        let n = rate as usize;
        let mut block: Vec<f32> = (0..2 * n)
            .flat_map(|i| {
                let x = (2.0 * PI * f * i as f32 / rate as f32).sin();
                [x, x]
            })
            .collect();
        eq.process(&mut block);
        let tail = &block[2 * n..];
        let rms = (tail.iter().map(|&y| f64::from(y * y)).sum::<f64>() / tail.len() as f64).sqrt();
        gain_to_db(rms as f32 * 2f32.sqrt())
    }

    #[test]
    fn presets_fit_the_range() {
        for (name, _, bands) in PRESETS {
            assert!(bands.iter().all(|b| b.abs() <= RANGE_DB), "{name}");
        }
    }

    #[test]
    fn bands_and_preamp() {
        let mut gains = [0.0; 10];
        gains[4] = 6.0;
        let mut eq = Equalizer::new(44100, 2, -3.0, &gains);
        assert!((gain_at(&mut eq, 44100, 1000.0) - 3.0).abs() < 0.15);
        assert!((gain_at(&mut eq, 44100, 60.0) + 3.0).abs() < 0.2);
        assert_eq!(eq.used, [4]);
        eq.configure(0.0, &[0.0; 10]);
        assert!(eq.used.is_empty());
        assert!(gain_at(&mut eq, 44100, 1000.0).abs() < 0.01);
    }
}
