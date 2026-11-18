//! Black-box fit of the Modern 135% profile. A shared stereo detector
//! controls each frequency region. Gain is real, preserving channel phase.
//! The dry delay plus overlap-added gain *difference* makes bypass exact.

use super::{enhancer, fft::fft};
use std::f32::consts::TAU;

// Lower/upper edges in Hz and the sensitivity of the six energy detectors
// per unit of mean-square signal. Fitted on independently shaped noise
// at two spectral resolutions. Normalization is folded into these weights.
const EDGES: [f32; 21] = [
    0., 40., 56.57, 80., 113.14, 160., 226.27, 320., 452.55, 640., 905.1, 1280., 1810.19, 2560.,
    3620.39, 5120., 7240.77, 10240., 14481.55, 20480., 22050.,
];
const SENSITIVITY: [[f32; 6]; 20] = [
    [14., 0., 0., 0., 0., 0.],
    [539., 19., 0., 0., 7., 0.],
    [3097., 0., 0., 0., 0., 0.],
    [1607., 0., 0., 0., 0., 0.],
    [1670., 1771., 0., 0., 11., 0.],
    [31., 6377., 0., 0., 0., 0.],
    [0., 195., 1500., 0., 0., 0.],
    [0., 0., 1456., 0., 0., 9.],
    [0., 0., 1523., 0., 0., 8.],
    [0., 20., 1546., 0., 0., 0.],
    [0., 0., 1074., 388., 0., 0.],
    [0., 0., 97., 1207., 0., 12.],
    [0., 0., 0., 1321., 10., 9.],
    [0., 0., 0., 1254., 19., 0.],
    [0., 0., 9., 1259., 77., 7.],
    [0., 0., 0., 1063., 920., 0.],
    [0., 0., 0., 594., 2114., 0.],
    [0., 10., 0., 58., 3433., 50.],
    [0., 0., 0., 0., 422., 5664.],
    [0., 0., 61., 0., 146., 6234.],
];
// Reserve inside the measured profile. Abrupt level steps otherwise hold the
// final look-ahead limiter down longer than the reference recording.
const OUTPUT_TRIM_DB: f32 = -2.0;
const SLOPE: [f32; 6] = [0.646, 0.8, 0.766, 0.772, 0.742, 0.711];
// Hz, interpolated detector index, gain at the reference pink-noise
// level, and maximum gain measured on a separate quiet plateau (dB).
const CURVE: &[[f32; 4]] = &[
    [0., 0., 10.74, 19.4],
    [43., 0., 12.11, 19.38],
    [86., 0., 15.05, 20.29],
    [108., 0.08, 15.75, 21.69],
    [129., 0.31, 16.5, 23.6],
    [151., 0.76, 18.09, 28.],
    [172., 0.95, 19.07, 30.14],
    [194., 1., 19.27, 30.7],
    [215., 1.14, 18.03, 29.16],
    [237., 1.45, 15.49, 26.67],
    [258., 1.79, 12.86, 25.06],
    [280., 1.98, 11.92, 24.64],
    [350., 1.99, 13.01, 24.05],
    [390., 2., 12.82, 24.07],
    [495., 2., 9.94, 22.05],
    [580., 2., 11.05, 22.8],
    [750., 2., 13.32, 24.28],
    [850., 2., 12.58, 24.43],
    [1000., 2.26, 11.06, 23.73],
    [1200., 2.6, 11.42, 24.95],
    [1420., 2.91, 11.9, 25.8],
    [1660., 3., 10.96, 26.03],
    [1990., 3., 8.53, 24.17],
    [2300., 3., 9.81, 25.1],
    [3150., 3., 12.05, 27.11],
    [3950., 3., 9.89, 25.19],
    [4800., 3.08, 9.5, 24.37],
    [5600., 3.23, 9.47, 24.06],
    [6300., 3.32, 10.11, 23.92],
    [7300., 3.43, 12.01, 24.92],
    [8300., 3.51, 13.31, 25.64],
    [10000., 3.62, 14.25, 25.68],
    [12500., 4., 15.23, 25.36],
    [13900., 4., 14.9, 24.1],
    [15000., 4.45, 15.57, 23.11],
    [16000., 5., 16.55, 22.2],
    [19000., 5., 16.68, 21.36],
    [20000., 5., 16.16, 21.36],
];

struct Bin {
    detector: [f32; 6],
    band: usize,
    blend: f32,
    base: f32,
    cap: f32,
    audible: f32,
}

impl Bin {
    fn new(hz: f32, step: f32, scale: f32) -> Self {
        let i = CURVE
            .partition_point(|k| k[0] < hz)
            .clamp(1, CURVE.len() - 1);
        let (a, b) = (CURVE[i - 1], CURVE[i]);
        let t = ((hz - a[0]) / (b[0] - a[0])).clamp(0., 1.);
        let at = |i: usize| a[i] + t * (b[i] - a[i]);
        let index = at(1);
        let band = (index as usize).min(4);
        let mut detector = [0.; 6];
        for (edges, weights) in EDGES.windows(2).zip(SENSITIVITY) {
            let overlap =
                ((hz + step * 0.5).min(edges[1]) - (hz - step * 0.5).max(edges[0])).clamp(0., step);
            for (d, w) in detector.iter_mut().zip(weights) {
                *d += w * (overlap / step) * scale;
            }
        }
        Self {
            detector,
            band,
            blend: index - band as f32,
            base: 10f32.powf((at(2) + OUTPUT_TRIM_DB) / 20.),
            cap: 10f32.powf((at(3) + OUTPUT_TRIM_DB) / 20.),
            audible: ((22050. - hz) / 2050.).clamp(0., 1.),
        }
    }
}

struct Channel {
    delay: Vec<f32>,
    residual: Vec<f32>,
    real: Vec<f32>,
    imag: Vec<f32>,
    full_range: bool,
}

pub(super) struct PowerFx {
    enabled: bool,
    tail: usize,
    pos: usize,
    channels: Vec<Channel>,
    window: Vec<f32>,
    twiddles: Vec<(f32, f32)>,
    bins: Vec<Bin>,
    bin_gain: Vec<f32>,
    env: [f32; 6],
    gain: [f32; 6],
    average: f32,
    attack: f32,
    release: f32,
}

impl PowerFx {
    pub(super) fn new(rate: u32, channels: usize, enabled: bool) -> Self {
        let rate = rate.max(1) as f32;
        let n = 1usize << (10. + (rate / 44100.).log2()).round().clamp(7., 13.) as u32;
        let angle = TAU / n as f32;
        let window: Vec<f32> = (0..n)
            .map(|i| 0.5 - 0.5 * (i as f32 * angle).cos())
            .collect();
        let scale = 2.
            / (n as f32
                * window.iter().map(|w| w * w).sum::<f32>()
                * (channels - usize::from(channels >= 6)) as f32);
        let step = rate / n as f32;
        let smooth = |seconds: f32| 1. - (-(n as f32 / 4.) / (rate * seconds)).exp();
        Self {
            enabled,
            tail: 0,
            pos: 0,
            channels: (0..channels)
                .map(|i| Channel {
                    delay: vec![0.; n],
                    residual: vec![0.; n],
                    real: vec![0.; n],
                    imag: vec![0.; n],
                    full_range: channels < 6 || i != 3,
                })
                .collect(),
            window,
            twiddles: (0..n / 2)
                .map(|i| {
                    let (s, c) = (-angle * i as f32).sin_cos();
                    (c, s)
                })
                .collect(),
            bins: (0..=n / 2)
                .map(|i| Bin::new(i as f32 * step, step, scale))
                .collect(),
            bin_gain: vec![0.; n / 2 + 1],
            env: [1.; 6],
            gain: [0.; 6],
            average: smooth(0.041),
            attack: smooth(0.016),
            release: smooth(0.052),
        }
    }

    pub(super) fn configure(&mut self, enabled: bool) {
        if enabled != self.enabled {
            if enabled {
                self.env.fill(1.);
                self.gain.fill(0.);
            } else {
                self.tail = self.latency_frames() + enhancer::LATENCY;
            }
            self.enabled = enabled;
        }
    }

    pub(super) fn active(&self) -> bool {
        self.enabled || self.tail > 0
    }
    pub(super) fn latency_frames(&self) -> usize {
        self.window.len()
    }

    pub(super) fn process(&mut self, samples: &mut [f32]) {
        let n = self.window.len();
        for frame in samples.chunks_exact_mut(self.channels.len()) {
            for (ch, x) in self.channels.iter_mut().zip(frame) {
                let dry =
                    std::mem::replace(&mut ch.delay[self.pos], if x.is_finite() { *x } else { 0. });
                let wet = std::mem::take(&mut ch.residual[self.pos]);
                *x = if wet == 0. { dry } else { dry + wet };
            }
            self.pos = (self.pos + 1) & (n - 1);
            self.tail = self.tail.saturating_sub(1);
            if self.enabled && self.pos.is_multiple_of(n / 4) {
                self.synthesize();
            }
        }
    }

    fn synthesize(&mut self) {
        let n = self.window.len();
        let mask = n - 1;
        let mut power = [0.; 6];
        for ch in self.channels.iter_mut().filter(|c| c.full_range) {
            for (i, (r, &w)) in ch.real.iter_mut().zip(&self.window).enumerate() {
                *r = ch.delay[(self.pos + i) & mask] * w;
            }
            ch.imag.fill(0.);
            fft(&mut ch.real, &mut ch.imag, &self.twiddles);
            for (i, bin) in self.bins.iter().enumerate() {
                let energy = ch.real[i] * ch.real[i] + ch.imag[i] * ch.imag[i];
                for (p, w) in power.iter_mut().zip(bin.detector) {
                    *p += energy * w;
                }
            }
        }
        for (env, p) in self.env.iter_mut().zip(power) {
            *env += self.average * (p - *env);
        }
        // Cap the detector as well as the per-bin gain so long silences
        // cannot leave the compressor wound up when music resumes.
        let max_delta = [9.7, 11.1, 12.3, 15.2, 11.4, 5.7];
        for (i, g) in self.gain.iter_mut().enumerate() {
            let energy = self.env[i];
            let target = (-10. * SLOPE[i] * energy.max(1e-12).log10()).clamp(-35., max_delta[i]);
            let alpha = if target < *g {
                self.attack
            } else {
                self.release
            };
            *g += alpha * (target - *g);
        }
        // Deep spectral notches reveal that adjacent gains combine in
        // amplitude. Interpolating decibels leaves crossover regions dull.
        let linear = self.gain.map(|db| 10f32.powf(db / 20.));
        for (g, b) in self.bin_gain.iter_mut().zip(&self.bins) {
            let gain = linear[b.band] + b.blend * (linear[b.band + 1] - linear[b.band]);
            *g = ((b.base * gain).min(b.cap) - 1.) * b.audible;
        }
        // Four periodic Hann windows overlap with a squared sum of 1.5.
        let scale = 1. / (1.5 * n as f32);
        for ch in self.channels.iter_mut().filter(|c| c.full_range) {
            for i in 0..n {
                let g = self.bin_gain[i.min(n - i)];
                ch.real[i] *= g;
                ch.imag[i] *= -g;
            }
            fft(&mut ch.real, &mut ch.imag, &self.twiddles);
            for (i, (&x, &w)) in ch.real.iter().zip(&self.window).enumerate() {
                ch.residual[(self.pos + i) & mask] += x * w * scale;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_tone_level_changes_follow_the_measured_profile() {
        // Independent Winamp control recording. All six sine amplitudes
        // go from -37 to -23 dBFS, then only 185 Hz returns to -37.
        const HZ: [f64; 6] = [70., 185., 700., 2800., 10500., 16500.];
        let rate = 44100;
        let mut fx = PowerFx::new(rate, 1, true);
        let mut output = Vec::new();
        for step in 0..3 {
            let mut samples = vec![0.; 2 * rate as usize];
            let mut phasors = vec![[0.; 6]; rate as usize];
            for (i, phase) in phasors.iter_mut().enumerate() {
                *phase = HZ.map(|hz| std::f64::consts::TAU * hz * i as f64 / rate as f64);
                let signal: f64 = phase
                    .iter()
                    .enumerate()
                    .map(|(b, p)| {
                        let level = if step == 0 || step == 2 && b == 1 {
                            -37.
                        } else {
                            -23.
                        };
                        p.sin() * 10f64.powf(level / 20.)
                    })
                    .sum();
                samples[i] = signal as f32;
                samples[i + rate as usize] = signal as f32;
            }
            fx.process(&mut samples);
            let mut bins = [(0., 0.); 6];
            for (&x, phase) in samples[rate as usize..].iter().zip(phasors) {
                for ((re, im), p) in bins.iter_mut().zip(phase) {
                    let (s, c) = p.sin_cos();
                    *re += f64::from(x) * c;
                    *im += f64::from(x) * s;
                }
            }
            output.push(bins.map(|(r, i)| 20. * r.hypot(i).log10()));
        }
        // Relative output levels cancel both capture volume and profile trim.
        let reference = [6.49, 2.64, 3.25, 3.20, 3.45, 4.01];
        for (b, want) in reference.into_iter().enumerate() {
            let change = output[1][b] - output[0][b];
            assert!((change - want).abs() < 2., "{} Hz: {change}", HZ[b]);
        }
        for (b, (&after, &before)) in output[2].iter().zip(&output[1]).enumerate() {
            let change = after - before;
            if b == 1 {
                assert!((change + 2.85).abs() < 2.6, "{change}");
            } else {
                assert!(change.abs() < 1., "{change}");
            }
        }
    }

    #[test]
    fn linked_gain_preserves_polarity_and_compresses_level_steps() {
        let rate = 44100;
        let mut fx = PowerFx::new(rate, 2, true);
        let mut outputs = Vec::new();
        for amplitude in [0.01, 0.1] {
            let mut audio: Vec<_> = (0..rate)
                .flat_map(|i| {
                    let x = amplitude * (TAU * 185. * i as f32 / rate as f32).sin();
                    [x, -0.25 * x]
                })
                .collect();
            fx.process(&mut audio);
            assert!(
                audio
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|s| (s[0] + 4. * s[1]).abs() < 1e-6)
            );
            let settled = &audio[rate as usize..];
            outputs.push(settled.iter().step_by(2).map(|x| x * x).sum::<f32>().sqrt());
        }
        let output_step_db = 20. * (outputs[1] / outputs[0]).log10();
        assert!((2.0..14.0).contains(&output_step_db), "{output_step_db}");
    }

    #[test]
    fn switching_modes_across_blocks_drains_to_an_exact_wire() {
        for rate in [22050, 44100, 48000, 96000, 192000] {
            let mut whole = PowerFx::new(rate, 1, false);
            let mut split = PowerFx::new(rate, 1, false);
            let delay = whole.latency_frames();
            let input: Vec<_> = (0..delay * 10)
                .map(|i| 0.03 * (i as f32 * 0.31).sin())
                .collect();
            let mut a = input.clone();
            let mut b = input.clone();
            for (i, (x, y)) in a
                .chunks_mut(2 * delay)
                .zip(b.chunks_mut(2 * delay))
                .enumerate()
            {
                let on = i == 1 || i == 3;
                whole.configure(on);
                split.configure(on);
                whole.process(x);
                for chunk in y.chunks_mut(103) {
                    split.process(chunk);
                }
            }
            assert_eq!(a, b);
            assert!(a[..delay].iter().all(|&x| x == 0.));
            assert_eq!(&a[9 * delay..], &input[8 * delay..9 * delay]);
        }
    }
}
