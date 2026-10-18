//! The enhancer: what consumer "sound improvers" (DFX, SRS WOW,
//! AudioBurst PowerFX) do, in four amounts of 0 to 10, each 0 doing
//! nothing.
//!
//! - **Bass**: a low shelf under 100 Hz, and harmonics of the band under
//!   120 Hz (a soft saturator fed the band at a levelled gain, its
//!   output kept between 90 and 500 Hz), which small speakers that
//!   cannot play the fundamental still make heard as bass (the missing
//!   fundamental).
//! - **Clarity**: harmonics of everything over 4 kHz (a soft saturator,
//!   its output over 6 kHz) and a high shelf over 9 kHz: the air an
//!   MP3's low-pass took away.
//! - **Width**: the side signal over 250 Hz raised (the bass stays in
//!   the middle), up to twice.
//! - **Warmth**: a blend with a soft saturation of the whole, which
//!   rounds the peaks as tape does.

use super::biquad::{Biquad, Coeffs};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Amounts {
    /// 0 to 1 each.
    pub bass: f32,
    pub clarity: f32,
    pub width: f32,
    pub warmth: f32,
}

impl Amounts {
    pub fn is_zero(&self) -> bool {
        self.bass <= 0.0 && self.clarity <= 0.0 && self.width <= 0.0 && self.warmth <= 0.0
    }
}

pub struct Enhancer {
    rate: u32,
    channels: usize,
    a: Amounts,
    // Bass: the band (mono), its envelope, the harmonics' band, the shelf
    // per channel.
    bass_band: Biquad,
    bass_env: f32,
    env_release: f32,
    harm_hp: Biquad,
    harm_lp: Biquad,
    bass_shelf: Vec<Biquad>,
    // Clarity, per channel.
    air_hp: Vec<Biquad>,
    air_out: Vec<Biquad>,
    air_shelf: Vec<Biquad>,
    // Width: the side's high-pass.
    side_hp: Biquad,
}

/// The saturator's drive for the bass harmonics and the air.
const BASS_DRIVE: f32 = 2.5;
const AIR_DRIVE: f32 = 3.0;
/// Warmth's saturation drive.
const WARMTH_DRIVE: f32 = 2.5;

impl Enhancer {
    pub fn new(rate: u32, channels: usize, a: Amounts) -> Enhancer {
        let per = |c: Coeffs| vec![Biquad::new(c); channels];
        let mut e = Enhancer {
            rate,
            channels,
            a: Amounts::default(),
            bass_band: Biquad::new(Coeffs::low_pass(
                rate,
                120.0,
                std::f32::consts::FRAC_1_SQRT_2,
            )),
            bass_env: 0.0,
            env_release: 1.0 - (-1.0 / (rate as f32 * 0.2)).exp(),
            harm_hp: Biquad::new(Coeffs::high_pass(
                rate,
                90.0,
                std::f32::consts::FRAC_1_SQRT_2,
            )),
            harm_lp: Biquad::new(Coeffs::low_pass(
                rate,
                500.0,
                std::f32::consts::FRAC_1_SQRT_2,
            )),
            bass_shelf: per(Coeffs::default()),
            air_hp: per(Coeffs::high_pass(
                rate,
                4000.0,
                std::f32::consts::FRAC_1_SQRT_2,
            )),
            air_out: per(Coeffs::high_pass(
                rate,
                6000.0,
                std::f32::consts::FRAC_1_SQRT_2,
            )),
            air_shelf: per(Coeffs::default()),
            side_hp: Biquad::new(Coeffs::high_pass(
                rate,
                250.0,
                std::f32::consts::FRAC_1_SQRT_2,
            )),
        };
        e.configure(a);
        e
    }

    /// New amounts; the filters' state is kept.
    pub fn configure(&mut self, a: Amounts) {
        self.a = Amounts {
            bass: a.bass.clamp(0.0, 1.0),
            clarity: a.clarity.clamp(0.0, 1.0),
            width: a.width.clamp(0.0, 1.0),
            warmth: a.warmth.clamp(0.0, 1.0),
        };
        let shelf = Coeffs::low_shelf(self.rate, 100.0, 5.0 * self.a.bass);
        for f in &mut self.bass_shelf {
            f.set(shelf);
        }
        let air = Coeffs::high_shelf(self.rate, 9000.0, 2.5 * self.a.clarity);
        for f in &mut self.air_shelf {
            f.set(air);
        }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        if self.a.is_zero() {
            return;
        }
        let n = self.channels;
        let a = self.a;
        for frame in samples.chunks_exact_mut(n) {
            // Bass.
            if a.bass > 0.0 {
                let mono = frame.iter().sum::<f32>() / n as f32;
                let low = self.bass_band.run(mono);
                // The envelope: up at once, down over 200 ms, so the
                // saturator sees the same level whatever the music's.
                let mag = low.abs();
                self.bass_env = if mag > self.bass_env {
                    mag
                } else {
                    self.bass_env + (mag - self.bass_env) * self.env_release
                };
                let norm = low / self.bass_env.max(1e-3);
                let sat = (BASS_DRIVE * norm).tanh() * self.bass_env;
                let harm = self.harm_lp.run(self.harm_hp.run(sat));
                for (ch, x) in frame.iter_mut().enumerate() {
                    *x = self.bass_shelf[ch].run(*x) + 0.7 * a.bass * harm;
                }
            }
            // Clarity.
            if a.clarity > 0.0 {
                for (ch, x) in frame.iter_mut().enumerate() {
                    let hp = self.air_hp[ch].run(*x);
                    let sat = (AIR_DRIVE * hp).tanh() / AIR_DRIVE;
                    let air = self.air_out[ch].run(sat);
                    *x = self.air_shelf[ch].run(*x) + 1.2 * a.clarity * air;
                }
            }
            // Width.
            if a.width > 0.0 && n == 2 {
                let (l, r) = (frame[0], frame[1]);
                let mid = (l + r) / 2.0;
                let side = (l - r) / 2.0;
                let side = side + a.width * self.side_hp.run(side);
                frame[0] = mid + side;
                frame[1] = mid - side;
            }
            // Warmth.
            if a.warmth > 0.0 {
                for x in frame.iter_mut() {
                    let sat = (WARMTH_DRIVE * *x).tanh() / WARMTH_DRIVE;
                    // Unity at -12 dBFS, so the music's level stays while
                    // the peaks round off.
                    let sat = sat * (0.25 * WARMTH_DRIVE / (0.25 * WARMTH_DRIVE).tanh());
                    *x += a.warmth * (sat - *x);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(rate: u32, f: f32, amp: f32, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .flat_map(|i| {
                let x = amp * (2.0 * PI * f * i as f32 / rate as f32).sin();
                [x, x]
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|&x| f64::from(x * x)).sum::<f64>() / samples.len() as f64).sqrt()
            as f32
    }

    #[test]
    fn zero_amounts_change_nothing() {
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, Amounts::default());
        let original = sine(rate, 440.0, 0.5, 0.5);
        let mut s = original.clone();
        e.process(&mut s);
        assert_eq!(s, original);
    }

    #[test]
    fn bass_adds_harmonics_above_the_fundamental() {
        let rate = 48000;
        let mut e = Enhancer::new(
            rate,
            2,
            Amounts {
                bass: 1.0,
                ..Default::default()
            },
        );
        // A 60 Hz tone: after the enhancer there is energy between 90 and
        // 500 Hz that was not there (its harmonics), measured with a
        // band-pass.
        let mut s = sine(rate, 60.0, 0.3, 2.0);
        e.process(&mut s);
        let mut hp = Biquad::new(Coeffs::high_pass(
            rate,
            100.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ));
        let mut lp = Biquad::new(Coeffs::low_pass(
            rate,
            500.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ));
        let band: Vec<f32> = s[rate as usize..]
            .iter()
            .step_by(2)
            .map(|&x| lp.run(hp.run(x)))
            .collect();
        assert!(rms(&band) > 0.02, "{}", rms(&band));
    }

    #[test]
    fn width_widens_and_keeps_the_middle() {
        let rate = 48000;
        let mut e = Enhancer::new(
            rate,
            2,
            Amounts {
                width: 1.0,
                ..Default::default()
            },
        );
        // Left only at 1 kHz: side and mid equal; the side doubles.
        let mut s: Vec<f32> = (0..rate as usize)
            .flat_map(|i| {
                [
                    0.5 * (2.0 * PI * 1000.0 * i as f32 / rate as f32).sin(),
                    0.0,
                ]
            })
            .collect();
        e.process(&mut s);
        let tail = &s[rate as usize / 2 * 2..];
        let mid: Vec<f32> = tail.chunks(2).map(|f| (f[0] + f[1]) / 2.0).collect();
        let side: Vec<f32> = tail.chunks(2).map(|f| (f[0] - f[1]) / 2.0).collect();
        assert!((rms(&mid) - 0.25 / 2f32.sqrt()).abs() < 0.01);
        assert!(
            (rms(&side) / rms(&mid) - 2.0).abs() < 0.1,
            "{}",
            rms(&side) / rms(&mid)
        );
    }

    #[test]
    fn warmth_rounds_peaks_and_keeps_quiet_levels() {
        let rate = 48000;
        let mut e = Enhancer::new(
            rate,
            2,
            Amounts {
                warmth: 1.0,
                ..Default::default()
            },
        );
        let mut loud = sine(rate, 440.0, 1.0, 0.5);
        e.process(&mut loud);
        let peak = loud.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak < 0.8, "{peak}");
        let mut quiet = sine(rate, 440.0, 0.1, 0.5);
        e.process(&mut quiet);
        // Within about 1 dB of its level (the saturation's small-signal
        // gain is a little over 1).
        let r = rms(&quiet[rate as usize / 2..]);
        assert!((r / (0.1 / 2f32.sqrt()) - 1.0).abs() < 0.15, "{r}");
    }
}
