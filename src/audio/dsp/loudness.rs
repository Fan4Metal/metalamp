//! Loudness after EBU R128 / ITU-R BS.1770: K-weighting (a high shelf
//! and a high-pass, the coefficients derived for any sample rate as
//! libebur128 does), the mean square over 400 ms blocks every 100 ms,
//! the blocks gated at −70 LUFS and at 10 LU under their mean, and the
//! integrated loudness of what passes. The sample peak comes along, for
//! the gain cap.

use super::biquad::{Biquad, Coeffs};

/// A block is this many 100 ms steps.
const STEPS_PER_BLOCK: usize = 4;
const ABSOLUTE_GATE: f64 = -70.0;
const RELATIVE_GATE: f64 = -10.0;

pub struct Meter {
    channels: usize,
    weights: Vec<f64>,
    /// The two K-weighting stages per channel.
    pre: Vec<Biquad>,
    rlb: Vec<Biquad>,
    /// Frames per 100 ms step, and the energy gathered in the current one.
    step_frames: usize,
    in_step: usize,
    step_energy: f64,
    /// The last steps' energies, a block being the sum of four.
    steps: Vec<f64>,
    /// Each block's mean square (weighted, summed over the channels).
    blocks: Vec<f64>,
    peak: f32,
}

impl Meter {
    pub fn new(rate: u32, channels: usize) -> Meter {
        let channels = channels.max(1);
        let (pre, rlb) = k_weighting(rate);
        // L, R, C at 1; the LFE of a 5.1 layout left out; the surrounds
        // at +1.5 dB.
        let weights = (0..channels)
            .map(|ch| match (channels, ch) {
                (6, 3) => 0.0,
                (6, 4 | 5) => 1.41,
                _ => 1.0,
            })
            .collect();
        Meter {
            channels,
            weights,
            pre: vec![Biquad::new(pre); channels],
            rlb: vec![Biquad::new(rlb); channels],
            step_frames: (rate as usize / 10).max(1),
            in_step: 0,
            step_energy: 0.0,
            steps: Vec::new(),
            blocks: Vec::new(),
            peak: 0.0,
        }
    }

    /// Interleaved frames.
    pub fn add(&mut self, samples: &[f32]) {
        for frame in samples.chunks_exact(self.channels) {
            for (ch, &x) in frame.iter().enumerate() {
                self.peak = self.peak.max(x.abs());
                let y = self.rlb[ch].run(self.pre[ch].run(x));
                self.step_energy += self.weights[ch] * f64::from(y * y);
            }
            self.in_step += 1;
            if self.in_step == self.step_frames {
                self.steps.push(self.step_energy / self.step_frames as f64);
                self.step_energy = 0.0;
                self.in_step = 0;
                if self.steps.len() >= STEPS_PER_BLOCK {
                    let n = self.steps.len();
                    let block: f64 = self.steps[n - STEPS_PER_BLOCK..].iter().sum::<f64>()
                        / STEPS_PER_BLOCK as f64;
                    self.blocks.push(block);
                    if n > 2 * STEPS_PER_BLOCK {
                        self.steps.drain(..n - STEPS_PER_BLOCK);
                    }
                }
            }
        }
    }

    /// The integrated loudness in LUFS; `None` when nothing passed the
    /// gates (silence, or less than 400 ms).
    pub fn integrated(&self) -> Option<f32> {
        let loud = |energy: f64| -0.691 + 10.0 * energy.log10();
        let above: Vec<f64> = self
            .blocks
            .iter()
            .copied()
            .filter(|&e| loud(e) > ABSOLUTE_GATE)
            .collect();
        if above.is_empty() {
            return None;
        }
        let mean = above.iter().sum::<f64>() / above.len() as f64;
        let threshold = loud(mean) + RELATIVE_GATE;
        let gated: Vec<f64> = above.into_iter().filter(|&e| loud(e) > threshold).collect();
        if gated.is_empty() {
            return None;
        }
        let mean = gated.iter().sum::<f64>() / gated.len() as f64;
        Some(loud(mean) as f32)
    }

    /// The largest sample magnitude seen.
    pub fn peak(&self) -> f32 {
        self.peak
    }
}

/// The K-weighting's two stages for `rate`: the pre-filter (a high shelf
/// of +4 dB above 1.5 kHz) and the RLB high-pass at 38 Hz, from their
/// analogue prototypes (libebur128's way, so any rate gives the curve
/// BS.1770 tabulates for 48 kHz).
fn k_weighting(rate: u32) -> (Coeffs, Coeffs) {
    let rate = f64::from(rate.max(1));
    let pre = {
        let f0 = 1681.974450955533;
        let g = 3.999843853973347;
        let q = 0.7071752369554196;
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        Coeffs {
            b0: ((vh + vb * k / q + k * k) / a0) as f32,
            b1: (2.0 * (k * k - vh) / a0) as f32,
            b2: ((vh - vb * k / q + k * k) / a0) as f32,
            a1: (2.0 * (k * k - 1.0) / a0) as f32,
            a2: ((1.0 - k / q + k * k) / a0) as f32,
        }
    };
    let rlb = {
        let f0 = 38.13547087602444;
        let q = 0.5003270373238773;
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Coeffs {
            b0: 1.0,
            b1: -2.0,
            b2: 1.0,
            a1: (2.0 * (k * k - 1.0) / a0) as f32,
            a2: ((1.0 - k / q + k * k) / a0) as f32,
        }
    };
    (pre, rlb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn coefficients_at_48k_are_the_standards() {
        let (pre, rlb) = k_weighting(48000);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(close(pre.b0, 1.535_124_9) && close(pre.b1, -2.691_696_2));
        assert!(close(pre.b2, 1.198_392_8));
        assert!(close(pre.a1, -1.690_659_3) && close(pre.a2, 0.732_480_8));
        assert!(close(rlb.a1, -1.990_047_5) && close(rlb.a2, 0.990_072_3));
    }

    /// EBU Tech 3341: a 1 kHz stereo sine at −23 dBFS reads −23 LUFS.
    #[test]
    fn sine_at_minus_23_reads_minus_23_lufs() {
        for rate in [44100u32, 48000] {
            let mut m = Meter::new(rate, 2);
            let a = 10f32.powf(-23.0 / 20.0);
            let samples: Vec<f32> = (0..5 * rate as usize)
                .flat_map(|i| {
                    let x = a * (2.0 * PI * 1000.0 * i as f32 / rate as f32).sin();
                    [x, x]
                })
                .collect();
            m.add(&samples);
            let l = m.integrated().unwrap();
            assert!((l + 23.0).abs() < 0.1, "{rate}: {l}");
            assert!((m.peak() - a).abs() < 1e-3);
        }
    }

    #[test]
    fn silence_is_gated_out_and_quiet_tails_do_not_count() {
        let rate = 48000;
        let mut m = Meter::new(rate, 2);
        m.add(&vec![0.0; 4 * rate as usize]);
        assert_eq!(m.integrated(), None);
        // Loud music then a long quiet tail 20 LU under: the tail is
        // under the relative gate, so the loudness is the music's.
        let a = 10f32.powf(-20.0 / 20.0);
        let loud: Vec<f32> = (0..3 * rate as usize)
            .flat_map(|i| {
                let x = a * (2.0 * PI * 1000.0 * i as f32 / rate as f32).sin();
                [x, x]
            })
            .collect();
        let quiet: Vec<f32> = loud.iter().map(|x| x * 0.1).collect();
        m.add(&loud);
        for _ in 0..5 {
            m.add(&quiet);
        }
        // The blocks across the change pull it down a little.
        let l = m.integrated().unwrap();
        assert!((l + 20.0).abs() < 0.5, "{l}");
    }
}
