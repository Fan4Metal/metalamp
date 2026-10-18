//! Second-order IIR filters from the Audio EQ Cookbook (Robert
//! Bristow-Johnson): peaking, shelves, low- and high-pass; run in the
//! transposed direct form II, one state per channel.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Default for Coeffs {
    /// Passes the signal through.
    fn default() -> Self {
        Coeffs {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }
}

impl Coeffs {
    fn from_raw(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Coeffs {
        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// `f0` kept under the Nyquist frequency, as an angle.
    fn omega(rate: u32, f0: f32) -> f32 {
        let f0 = f0.min(rate as f32 * 0.49).max(1.0);
        2.0 * PI * f0 / rate as f32
    }

    /// A bell of `gain_db` at `f0`, `q` wide.
    pub fn peaking(rate: u32, f0: f32, q: f32, gain_db: f32) -> Coeffs {
        let a = 10f32.powf(gain_db / 40.0);
        let w = Self::omega(rate, f0);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        Self::from_raw(
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        )
    }

    /// Everything under `f0` raised by `gain_db` (a slope of 1).
    pub fn low_shelf(rate: u32, f0: f32, gain_db: f32) -> Coeffs {
        let a = 10f32.powf(gain_db / 40.0);
        let w = Self::omega(rate, f0);
        let (s, c) = w.sin_cos();
        let alpha = s / 2.0 * 2f32.sqrt();
        let k = 2.0 * a.sqrt() * alpha;
        Self::from_raw(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        )
    }

    /// Everything over `f0` raised by `gain_db` (a slope of 1).
    pub fn high_shelf(rate: u32, f0: f32, gain_db: f32) -> Coeffs {
        let a = 10f32.powf(gain_db / 40.0);
        let w = Self::omega(rate, f0);
        let (s, c) = w.sin_cos();
        let alpha = s / 2.0 * 2f32.sqrt();
        let k = 2.0 * a.sqrt() * alpha;
        Self::from_raw(
            a * ((a + 1.0) + (a - 1.0) * c + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - k),
            (a + 1.0) - (a - 1.0) * c + k,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - k,
        )
    }

    pub fn low_pass(rate: u32, f0: f32, q: f32) -> Coeffs {
        let w = Self::omega(rate, f0);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        Self::from_raw(
            (1.0 - c) / 2.0,
            1.0 - c,
            (1.0 - c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }

    pub fn high_pass(rate: u32, f0: f32, q: f32) -> Coeffs {
        let w = Self::omega(rate, f0);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        Self::from_raw(
            (1.0 + c) / 2.0,
            -(1.0 + c),
            (1.0 + c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }
}

/// One filter on one channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    pub c: Coeffs,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn new(c: Coeffs) -> Biquad {
        Biquad {
            c,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// New coefficients, the state kept (no click when a gain changes).
    pub fn set(&mut self, c: Coeffs) {
        self.c = c;
    }

    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        let c = &self.c;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// The gain in decibels as a factor.
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-9).log10()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The gain in dB of `filter` for a sine of `f` Hz at `rate`, from
    /// the RMS of a second of it after a second of settling.
    pub(crate) fn gain_at(mut filter: Biquad, rate: u32, f: f32) -> f32 {
        let n = rate as usize;
        let (mut sum, mut count) = (0.0f64, 0usize);
        for i in 0..2 * n {
            let x = (2.0 * PI * f * i as f32 / rate as f32).sin();
            let y = filter.run(x);
            if i >= n {
                sum += f64::from(y * y);
                count += 1;
            }
        }
        let rms = (sum / count as f64).sqrt() as f32;
        gain_to_db(rms * 2f32.sqrt())
    }

    #[test]
    fn peaking_bell_has_its_gain_at_its_centre_and_none_far_away() {
        let c = Coeffs::peaking(44100, 1000.0, 1.0, 6.0);
        let f = Biquad::new(c);
        assert!((gain_at(f, 44100, 1000.0) - 6.0).abs() < 0.1);
        assert!(gain_at(f, 44100, 60.0).abs() < 0.2);
        assert!(gain_at(f, 44100, 15000.0).abs() < 0.2);
        let cut = Biquad::new(Coeffs::peaking(44100, 1000.0, 1.0, -12.0));
        assert!((gain_at(cut, 44100, 1000.0) + 12.0).abs() < 0.1);
    }

    #[test]
    fn shelves_and_passes() {
        let low = Biquad::new(Coeffs::low_shelf(48000, 200.0, 6.0));
        assert!((gain_at(low, 48000, 30.0) - 6.0).abs() < 0.2);
        assert!(gain_at(low, 48000, 5000.0).abs() < 0.1);
        let high = Biquad::new(Coeffs::high_shelf(48000, 8000.0, 4.0));
        assert!((gain_at(high, 48000, 18000.0) - 4.0).abs() < 0.3);
        assert!(gain_at(high, 48000, 500.0).abs() < 0.1);
        let lp = Biquad::new(Coeffs::low_pass(
            48000,
            1000.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ));
        assert!((gain_at(lp, 48000, 1000.0) + 3.0).abs() < 0.2);
        assert!(gain_at(lp, 48000, 8000.0) < -30.0);
        let hp = Biquad::new(Coeffs::high_pass(
            48000,
            1000.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ));
        assert!((gain_at(hp, 48000, 1000.0) + 3.0).abs() < 0.2);
        assert!(gain_at(hp, 48000, 100.0) < -35.0);
    }
}
