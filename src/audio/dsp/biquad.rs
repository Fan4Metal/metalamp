//! A second-order IIR filter (the loudness meter's K-weighting) run in the
//! transposed direct form II, one state per channel, and the conversions
//! between decibels and gains.

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
mod tests {
    use super::*;

    #[test]
    fn a_wire_passes_and_the_conversions_agree() {
        let mut f = Biquad::new(Coeffs::default());
        assert_eq!(f.run(0.25), 0.25);
        assert!((gain_to_db(db_to_gain(-7.5)) + 7.5).abs() < 1e-5);
    }
}
