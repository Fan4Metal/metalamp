//! Linked look-ahead limiter with 4x inter-sample peak detection.
//! A finite linear attack reaches the required gain before the peak;
//! limiting changes gain, never clips individual waveform samples.
//! Recovery is faster for isolated transients than sustained overload.
use std::collections::VecDeque;
const LOOKAHEAD: f32 = 0.005;
const TAPS: usize = 32;
const DETECTOR_DELAY: usize = TAPS / 2;
const SAFETY: f32 = 0.98;

/// Windowed-sinc interpolation of the interval DETECTOR_DELAY frames
/// back. Three fractional phases plus the original sample, DC-normalized.
/// This detector delays only the sidechain, not the original waveform.
pub(crate) struct TruePeak {
    coefficients: [[f32; TAPS]; 3],
    history: Vec<[f32; TAPS]>,
    previous: Vec<[f32; 2]>,
    at: usize,
}
impl TruePeak {
    pub(crate) fn new(channels: usize) -> Self {
        fn i0(x: f64) -> f64 {
            let (mut sum, mut term) = (1.0, 1.0);
            for k in 1..40 {
                term *= x * x / (4.0 * (k * k) as f64);
                sum += term;
                if term < sum * 1e-16 {
                    break;
                }
            }
            sum
        }
        let coefficients = std::array::from_fn(|p| {
            let mut c = std::array::from_fn(|k| {
                let d = DETECTOR_DELAY as f64 - k as f64 - (p + 1) as f64 / 4.0;
                let sinc = (std::f64::consts::PI * d).sin() / (std::f64::consts::PI * d);
                let window = i0(8.6 * (1.0 - (d / 16.0).powi(2)).max(0.0).sqrt()) / i0(8.6);
                (sinc * window) as f32
            });
            let sum: f32 = c.iter().sum();
            for x in &mut c {
                *x /= sum;
            }
            c
        });
        Self {
            coefficients,
            history: vec![[0.0; TAPS]; channels],
            previous: vec![[0.0; 2]; channels],
            at: 0,
        }
    }
    pub(crate) fn feed(&mut self, frame: &[f32]) -> f32 {
        let mut peak = 0f32;
        for ((&x, h), previous) in frame.iter().zip(&mut self.history).zip(&mut self.previous) {
            h[self.at] = x;
            let mut phases = [0.0; 4];
            phases[0] = h[(self.at + TAPS - DETECTOR_DELAY) % TAPS];
            for (phase, c) in phases[1..].iter_mut().zip(&self.coefficients) {
                for (k, &v) in c.iter().enumerate() {
                    *phase += v * h[(self.at + TAPS - k) % TAPS];
                }
            }
            for sample in phases {
                let c = sample.abs();
                let [a, b] = *previous;
                peak = peak.max(c);
                // Refine a local maximum between the 4x grid points.
                // Without this a high-frequency sine can hide its crest
                // halfway between phases even with a perfect FIR.
                let curve = 2.0 * b - a - c;
                if b >= a && b >= c && curve > 1e-12 {
                    peak = peak.max(b + (a - c).powi(2) / (8.0 * curve));
                }
                *previous = [b, c];
            }
        }
        self.at = (self.at + 1) % TAPS;
        peak
    }
}

pub struct Limiter {
    channels: usize,
    ceiling: f32,
    delay: Vec<f32>,
    frames: usize,
    at: usize,
    lookahead: usize,
    needs: VecDeque<(u64, f32)>,
    index: u64,
    gain: f64,
    attack_step: f64,
    fast_release: f64,
    slow_release: f64,
    sustained: f64,
    mean: f64,
    detector: TruePeak,
}
impl Limiter {
    pub fn new(rate: u32, channels: usize, ceiling: f32) -> Self {
        let rate = rate.max(1);
        let channels = channels.max(1);
        let lookahead = ((rate as f32 * LOOKAHEAD) as usize).max(1);
        let frames = lookahead + DETECTOR_DELAY;
        Self {
            channels,
            ceiling,
            delay: vec![0.0; frames * channels],
            frames,
            at: 0,
            lookahead,
            needs: VecDeque::with_capacity(lookahead + 3),
            index: 0,
            gain: 1.0,
            attack_step: 1.0 / lookahead as f64,
            fast_release: 1.0 - (-1.0 / (f64::from(rate) * 0.06)).exp(),
            slow_release: 1.0 - (-1.0 / (f64::from(rate) * 0.24)).exp(),
            sustained: 0.0,
            mean: 1.0 - (-1.0 / (f64::from(rate) * 0.1)).exp(),
            detector: TruePeak::new(channels),
        }
    }
    pub fn reduction_db(&self) -> f32 {
        (-20.0 * self.gain.max(1e-9).log10()) as f32
    }
    /// The audio delay stays in place while limiting is bypassed.
    pub(crate) fn latency_frames(&self) -> usize {
        self.frames
    }
    pub fn process(&mut self, samples: &mut [f32]) {
        self.process_enabled(samples, true);
    }
    /// Keep the detector and delay warm while bypassed. Switching the
    /// limiter changes only its gain, never the stream's time or history.
    pub(crate) fn process_enabled(&mut self, samples: &mut [f32], enabled: bool) {
        for frame in samples.chunks_exact_mut(self.channels) {
            for x in frame.iter_mut() {
                if !x.is_finite() {
                    *x = 0.0;
                }
            }
            let peak = self.detector.feed(frame);
            let need = (self.ceiling * SAFETY / peak.max(1e-12)).min(1.0);
            while self.needs.back().is_some_and(|&(_, g)| g >= need) {
                self.needs.pop_back();
            }
            self.needs.push_back((self.index, need));
            // Include the interval of the sample being emitted. Keeping
            // one extra interval protects its interpolated neighbourhood.
            let oldest = self.index.saturating_sub(self.lookahead as u64 + 1);
            while self.needs.front().is_some_and(|&(i, _)| i < oldest) {
                self.needs.pop_front();
            }
            let target = if enabled {
                f64::from(self.needs.front().map_or(1.0, |&(_, g)| g))
            } else {
                1.0
            };
            self.sustained += ((1.0 - target) - self.sustained) * self.mean;
            if target < self.gain {
                self.gain = (self.gain - self.attack_step).max(target);
            } else {
                let mix = (4.0 * self.sustained).min(1.0);
                let k = self.fast_release + (self.slow_release - self.fast_release) * mix;
                self.gain += (target - self.gain) * k;
                if (target - self.gain).abs() < 1e-7 {
                    self.gain = target;
                }
            }
            let slot = &mut self.delay[self.at * self.channels..(self.at + 1) * self.channels];
            // Sample-peak guard is a gain decision shared by all channels;
            // normally the interpolated detector has already covered it.
            let out_peak = slot.iter().fold(0f32, |m, x| m.max(x.abs()));
            if enabled {
                self.gain = self
                    .gain
                    .min(f64::from((self.ceiling / out_peak.max(1e-12)).min(1.0)));
            }
            for (x, d) in frame.iter_mut().zip(slot) {
                let out = *d * self.gain as f32;
                *d = *x;
                *x = out;
            }
            self.at = (self.at + 1) % self.frames;
            self.index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_peaks_under_the_ceiling_and_leaves_quiet_sound_alone() {
        let rate = 48000;
        let mut l = Limiter::new(rate, 2, 0.9);
        // A second of a quiet sine, then a loud burst, then quiet again.
        let mut samples: Vec<f32> = (0..3 * rate as usize)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                let a = if (1.0..1.5).contains(&t) { 2.0 } else { 0.3 };
                let x = a * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
                [x, x]
            })
            .collect();
        l.process(&mut samples);
        let peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak <= 0.9 + 1e-6, "{peak}");
        // The quiet part before the burst comes through untouched (after
        // the delay).
        let before = &samples[2 * (rate as usize / 2)..2 * (rate as usize - 1000)];
        let max_before = before.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!((max_before - 0.3).abs() < 0.01, "{max_before}");
        // The burst is held near the ceiling, not squashed far under it.
        let during = &samples[2 * (rate as usize + 5000)..2 * (rate as usize + 20000)];
        let max_during = during.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(max_during > 0.85, "{max_during}");
        // Hard clipping is the last resort, not the rule: few samples sit
        // exactly at the ceiling.
        let at_ceiling = during.iter().filter(|&&x| x.abs() >= 0.9 - 1e-6).count();
        assert!(at_ceiling < during.len() / 50, "{at_ceiling}");
    }
    /// Independent 16x, 64-tap Hann-windowed reconstruction. Different
    /// resolution, window and length from the production detector.
    fn reconstructed_peak(s: &[f32], channels: usize) -> f64 {
        let frames = s.len() / channels;
        let mut peak = 0f64;
        for i in frames - 1024..frames - 40 {
            for phase in 0..16 {
                let t = phase as f64 / 16.0;
                let mut sum = 0.0;
                let mut y = vec![0.0; channels];
                for j in -31i32..=32 {
                    let d = j as f64 - t;
                    let sinc = if d.abs() < 1e-12 {
                        1.0
                    } else {
                        (std::f64::consts::PI * d).sin() / (std::f64::consts::PI * d)
                    };
                    let c = sinc * (0.5 + 0.5 * (std::f64::consts::PI * d / 32.0).cos());
                    sum += c;
                    for (ch, v) in y.iter_mut().enumerate() {
                        *v += c * s[((i as i32 + j) as usize) * channels + ch] as f64;
                    }
                }
                for v in y {
                    peak = peak.max((v / sum).abs());
                }
            }
        }
        peak
    }
    #[test]
    fn catches_inter_sample_peaks_at_different_rates_and_phases() {
        for rate in [44100, 48000, 96000] {
            for fraction in [0.11, 0.25, 0.37, 0.45] {
                for phase in [0.13, std::f64::consts::FRAC_PI_4, 1.17] {
                    let mut s: Vec<f32> = (0..rate / 5)
                        .flat_map(|i| {
                            let x = 1.2
                                * (std::f64::consts::TAU * fraction * i as f64 + phase).sin()
                                    as f32;
                            [x, x * 0.5]
                        })
                        .collect();
                    let mut l = Limiter::new(rate, 2, 0.9);
                    l.process(&mut s);
                    let peak = reconstructed_peak(&s, 2);
                    assert!(peak <= 0.905, "{rate}, {fraction}, {phase}: {peak}");
                    assert!(peak > 0.82, "over-limited: {peak}");
                    assert!(
                        s.as_chunks::<2>()
                            .0
                            .iter()
                            .all(|f| (f[0] * 0.5 - f[1]).abs() < 1e-6)
                    );
                }
            }
        }
    }
    #[test]
    fn an_isolated_peak_changes_gain_without_clipping_the_waveform() {
        let mut s = vec![0.0; 48000];
        s[2000] = 2.0;
        s[2001] = 1.0;
        let mut l = Limiter::new(48000, 2, 0.9);
        l.process(&mut s);
        let peak = s.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak < 0.9 && peak > 0.85, "{peak}");
        assert!(
            s.as_chunks::<2>()
                .0
                .iter()
                .all(|f| (f[0] * 0.5 - f[1]).abs() < 1e-7)
        );
    }
    #[test]
    fn limiter_is_independent_of_block_boundaries() {
        let original: Vec<f32> = (0..12000).map(|i| 1.3 * (i as f32 * 0.71).sin()).collect();
        let mut a = original.clone();
        let mut b = original;
        Limiter::new(48000, 2, 0.9).process(&mut a);
        let mut l = Limiter::new(48000, 2, 0.9);
        for part in b.chunks_mut(46) {
            l.process(part);
        }
        assert_eq!(a, b);
    }

    #[test]
    fn bypass_releases_attenuation_without_dropping_the_delay() {
        for rate in [48000, 192000] {
            let mut l = Limiter::new(rate, 2, 0.9);
            let delay = l.latency_frames();
            let mut loud = vec![1.2; rate as usize / 2];
            l.process(&mut loud);
            let held = loud[loud.len() - 1];
            assert!(held < 0.9 && held > 0.85, "{held}");
            let mut bypassed = vec![1.2; rate as usize * 6];
            l.process_enabled(&mut bypassed, false);
            assert!((bypassed[0] - held).abs() < 0.001);
            assert_eq!(bypassed[bypassed.len() - 1], 1.2, "{rate}");
            assert!(bypassed.iter().all(|&x| x >= held));

            // A quiet non-repeating signal crosses another switch with every
            // frame intact. The history includes the bypassed interval.
            let original: Vec<f32> = (0..4000).map(|i| i as f32 / 20000.0).collect();
            let mut s = original.clone();
            s.extend(std::iter::repeat_n(0.0, delay * 2));
            let mut l = Limiter::new(rate, 2, 0.9);
            l.process_enabled(&mut s[..400], false);
            l.process_enabled(&mut s[400..], true);
            assert_eq!(&s[delay * 2..], &original);
        }
    }
}
