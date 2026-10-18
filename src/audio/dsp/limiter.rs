//! A look-ahead peak limiter at the end of the chain: the signal is
//! delayed `LOOKAHEAD`, the gain needed to keep each peak under the
//! ceiling is known that far ahead (a sliding-window minimum), and the
//! gain follows it down quickly and up slowly, so nothing the equalizer,
//! the enhancer or a raised quiet track pushes over full scale clips.

use std::collections::VecDeque;

/// Seconds of warning the gain has before a peak.
const LOOKAHEAD: f32 = 0.005;
/// The time constants of the gain falling to a peak's need and rising
/// back after it, in seconds.
const ATTACK: f32 = 0.0012;
const RELEASE: f32 = 0.08;

pub struct Limiter {
    channels: usize,
    ceiling: f32,
    /// The frames delayed, interleaved, as a ring.
    delay: Vec<f32>,
    frames: usize,
    at: usize,
    /// The gain each delayed frame needs, with its index, the minimum at
    /// the front (a monotonic queue).
    needs: VecDeque<(u64, f32)>,
    index: u64,
    gain: f32,
    attack: f32,
    release: f32,
}

impl Limiter {
    pub fn new(rate: u32, channels: usize, ceiling: f32) -> Limiter {
        let frames = ((rate as f32 * LOOKAHEAD) as usize).max(1);
        Limiter {
            channels,
            ceiling,
            delay: vec![0.0; frames * channels],
            frames,
            at: 0,
            needs: VecDeque::new(),
            index: 0,
            gain: 1.0,
            attack: 1.0 - (-1.0 / (rate as f32 * ATTACK)).exp(),
            release: 1.0 - (-1.0 / (rate as f32 * RELEASE)).exp(),
        }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        let n = self.channels;
        for frame in samples.chunks_exact_mut(n) {
            // The gain this frame will need when it comes out of the delay.
            let peak = frame.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
            let need = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            while self.needs.back().is_some_and(|&(_, g)| g >= need) {
                self.needs.pop_back();
            }
            self.needs.push_back((self.index, need));
            // Frames older than the window leave it.
            let oldest = self.index.saturating_sub(self.frames as u64 - 1);
            while self.needs.front().is_some_and(|&(i, _)| i < oldest) {
                self.needs.pop_front();
            }
            let target = self.needs.front().map_or(1.0, |&(_, g)| g);
            let coef = if target < self.gain {
                self.attack
            } else {
                self.release
            };
            self.gain += (target - self.gain) * coef;
            // Swap the frame with the one delayed and apply the gain.
            let slot = &mut self.delay[self.at * n..(self.at + 1) * n];
            for (x, d) in frame.iter_mut().zip(slot.iter_mut()) {
                let out = *d * self.gain;
                *d = *x;
                *x = out.clamp(-self.ceiling, self.ceiling);
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
}
