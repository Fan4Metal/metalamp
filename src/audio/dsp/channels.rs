//! The balance and mono, on the first two channels (front left and right):
//! mono gives both their mean, the balance turns the other side down
//! (none of them raises a level, so the limiter is not needed for them).
//! Their gains slide to the ones asked for in about `SLIDE_S`, so moving
//! the slider or switching mono is not a click; at rest in the middle and
//! not mono, it is a wire.

/// How long a change takes, in seconds (the time constant).
const SLIDE_S: f32 = 0.05;

/// The gains of the left and the right at `balance` (-100: the left
/// alone, 0: both, 100: the right alone): the side away from it turned
/// down in a straight line, the other kept.
pub fn balance_gains(balance: i8) -> (f32, f32) {
    let b = f32::from(balance.clamp(-100, 100)) / 100.0;
    ((1.0 - b).min(1.0), (1.0 + b).min(1.0))
}

pub struct Channels {
    channels: usize,
    /// Now: the left's and the right's gain and how far towards mono
    /// (0 to 1).
    left: f32,
    right: f32,
    mono: f32,
    slide: f32,
}

impl Channels {
    pub fn new(rate: u32, channels: usize) -> Channels {
        Channels {
            channels,
            left: 1.0,
            right: 1.0,
            mono: 0.0,
            slide: 1.0 - (-1.0 / (rate.max(1) as f32 * SLIDE_S)).exp(),
        }
    }

    /// The gains asked for set at once (a new chain for the same settings
    /// starts where they are).
    pub fn settle(&mut self, balance: i8, mono: bool) {
        (self.left, self.right) = balance_gains(balance);
        self.mono = if mono { 1.0 } else { 0.0 };
    }

    /// Interleaved `samples` with the balance and mono asked for.
    pub fn process(&mut self, samples: &mut [f32], balance: i8, mono: bool) {
        if self.channels < 2 {
            return;
        }
        let (left, right) = balance_gains(balance);
        let to_mono = if mono { 1.0 } else { 0.0 };
        let at_rest = self.left == left && self.right == right && self.mono == to_mono;
        if at_rest && left == 1.0 && right == 1.0 && to_mono == 0.0 {
            return;
        }
        let slide = |now: &mut f32, to: f32, k: f32| {
            if (*now - to).abs() > 1e-4 {
                *now += (to - *now) * k;
            } else {
                *now = to;
            }
        };
        for frame in samples.chunks_exact_mut(self.channels) {
            if !at_rest {
                slide(&mut self.left, left, self.slide);
                slide(&mut self.right, right, self.slide);
                slide(&mut self.mono, to_mono, self.slide);
            }
            let (l, r) = (frame[0], frame[1]);
            let mean = 0.5 * (l + r);
            frame[0] = (l + (mean - l) * self.mono) * self.left;
            frame[1] = (r + (mean - r) * self.mono) * self.right;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_balance_turns_the_other_side_down() {
        assert_eq!(balance_gains(0), (1.0, 1.0));
        assert_eq!(balance_gains(100), (0.0, 1.0));
        assert_eq!(balance_gains(-100), (1.0, 0.0));
        assert_eq!(balance_gains(50), (0.5, 1.0));
        assert_eq!(balance_gains(-25), (1.0, 0.75));
        assert_eq!(balance_gains(i8::MIN), (1.0, 0.0));
    }

    /// Stereo frames of a left and a right level.
    fn frames(n: usize, l: f32, r: f32) -> Vec<f32> {
        (0..n).flat_map(|_| [l, r]).collect()
    }

    #[test]
    fn in_the_middle_and_not_mono_it_is_a_wire() {
        let mut c = Channels::new(48_000, 2);
        let mut s = frames(100, 0.3, -0.7);
        c.process(&mut s, 0, false);
        assert_eq!(s, frames(100, 0.3, -0.7));
        // One channel: nothing to do.
        let mut c = Channels::new(48_000, 1);
        let mut s = vec![0.5; 10];
        c.process(&mut s, 100, true);
        assert_eq!(s, vec![0.5; 10]);
    }

    #[test]
    fn mono_and_balance_settle_where_asked() {
        let mut c = Channels::new(48_000, 2);
        let mut s = frames(48_000, 0.8, 0.2);
        c.process(&mut s, 50, true);
        // After a second: both at the mean 0.5, the left halved.
        let end = &s[s.len() - 2..];
        assert!((end[0] - 0.25).abs() < 1e-3, "{end:?}");
        assert!((end[1] - 0.5).abs() < 1e-3, "{end:?}");
        // Settled at once, the same from the first frame.
        let mut c = Channels::new(48_000, 2);
        c.settle(50, true);
        let mut s = frames(2, 0.8, 0.2);
        c.process(&mut s, 50, true);
        assert!((s[0] - 0.25).abs() < 1e-6 && (s[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_change_slides_without_a_step() {
        let mut c = Channels::new(48_000, 2);
        let mut s = frames(48_000, 1.0, 1.0);
        c.process(&mut s, -100, false);
        let right: Vec<f32> = s.iter().skip(1).step_by(2).copied().collect();
        // No step between frames bigger than the first one's, about
        // 1/2400 of the way; down to nothing in the end.
        let biggest = right
            .windows(2)
            .map(|w| (w[0] - w[1]).abs())
            .fold(0.0, f32::max);
        assert!(biggest < 1e-3, "{biggest}");
        assert!(right[right.len() - 1] < 1e-3);
        // Halfway down within a time constant or so.
        assert!((0.3..0.45).contains(&right[2400]), "{}", right[2400]);
        // The other channels of a 5.1 stream are left alone.
        let mut c = Channels::new(48_000, 6);
        c.settle(100, true);
        let mut s = vec![0.5f32; 12];
        c.process(&mut s, 100, true);
        assert_eq!(&s[2..6], &[0.5; 4]);
        assert_eq!(s[0], 0.0);
    }
}
