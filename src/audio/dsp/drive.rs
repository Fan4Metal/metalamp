//! Drive: an automatic gain control lifting the sound towards a loudness,
//! as AudioBurst PowerFX's Drive does (measured through a virtual cable:
//! a 1 kHz tone stepping 20 dB came out 4.7 dB apart, about 4:1, every
//! level raised, none lowered; down in 30 to 60 ms, up in about 0.5 s;
//! music came out at -6 to -9 LUFS, Alphaville's Forever Young 10.5 LU
//! louder, its crest factor 5.7 dB less). The level is the K-weighted
//! power (BS.1770's, as LUFS) followed over `DETECT_S`; the gain is
//! `(target - level) * (1 - 1 / ratio)`, never below 0 dB and at most
//! `MOST_DB`, the three growing with the amount, so a loud passage is left
//! as it is and a quiet one is brought up towards it. At the end of a
//! passage the activity detector holds the boost briefly, then releases
//! it to unity. No look-ahead and no delay: what a quick rise of the level
//! puts over the ceiling before the gain falls is the limiter's, which
//! runs whenever the drive does (`Chain::process`).

use super::activity::{Activity, STEP as SUB};
use super::biquad::{Biquad, db_to_gain};
use super::loudness::{channel_weights, k_weighting};

/// The target loudness: `TARGET_FROM` LUFS (loudness normalisation's)
/// raised by `TARGET_SPAN` at the whole amount, -10 LUFS; PowerFX went to
/// -6 to -9, which the limiter would hold only by squashing every peak.
const TARGET_FROM: f32 = -18.0;
const TARGET_SPAN: f32 = 8.0;
/// The ratio: 1 + `RATIO_SPAN` times the amount, 4:1 at the whole.
const RATIO_SPAN: f32 = 3.0;
/// The most the gain may raise, times the amount (PowerFX: 26 dB).
pub const MOST_DB: f32 = 18.0;
/// The level's time constant (power, one pole).
const DETECT_S: f32 = 0.1;
/// The gain falls in this (a loud passage after a quiet one) and rises in
/// that (a quiet one after a loud one).
const FALL_S: f32 = 0.05;
const RISE_S: f32 = 0.4;
/// Wait for the loudness estimate to fill at onset.
const GATE_LUFS: f32 = -60.0;

pub struct Drive {
    weights: Vec<f32>,
    pre: Vec<Biquad>,
    rlb: Vec<Biquad>,
    /// The K-weighted power summed over the channels, followed.
    power: f32,
    detect: f32,
    fall: f32,
    rise: f32,
    /// 0 to 1.
    amount: f32,
    /// The gain now, dB, and its factor at the last step and at this one.
    db: f32,
    from: f32,
    to: f32,
    sub: usize,
    /// The level at the last step, LUFS (for tests and the bench).
    level: f32,
}

impl Drive {
    pub fn new(rate: u32, channels: usize, amount: f32) -> Drive {
        let rate = rate.max(1) as f32;
        let channels = channels.max(1);
        let (pre, rlb) = k_weighting(rate as u32);
        let per_step = |seconds: f32| 1.0 - (-(SUB as f32) / (rate * seconds)).exp();
        Drive {
            weights: channel_weights(channels),
            pre: vec![Biquad::new(pre); channels],
            rlb: vec![Biquad::new(rlb); channels],
            power: 0.0,
            detect: 1.0 - (-1.0 / (rate * DETECT_S)).exp(),
            fall: per_step(FALL_S),
            rise: per_step(RISE_S),
            amount: clamped(amount),
            db: 0.0,
            from: 1.0,
            to: 1.0,
            sub: 0,
            level: -100.0,
        }
    }

    pub fn configure(&mut self, amount: f32) {
        self.amount = clamped(amount);
    }

    /// Also while the gain goes back to 0 dB after the amount went to 0:
    /// the limiter must go on until it has.
    pub fn active(&self) -> bool {
        self.amount > 0.0 || self.db != 0.0 || self.to != 1.0
    }

    /// The gain now, dB.
    pub fn gain_db(&self) -> f32 {
        self.db
    }

    /// The level the gain follows, LUFS.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// The gain carried into a chain made anew (another track, a seek),
    /// so the level does not start from 0 dB and climb again.
    pub fn keep(&mut self, db: f32) {
        self.db = db.max(0.0);
        let gain = if self.db == 0.0 {
            1.0
        } else {
            db_to_gain(self.db)
        };
        self.from = gain;
        self.to = gain;
    }

    fn step(&mut self, activity: Activity) {
        self.level = if self.power > 0.0 {
            -0.691 + 10.0 * self.power.log10()
        } else {
            -100.0
        };
        let a = self.amount;
        let wanted = if self.level < GATE_LUFS {
            self.db
        } else {
            let goal = TARGET_FROM + TARGET_SPAN * a;
            let ratio = 1.0 + RATIO_SPAN * a;
            ((goal - self.level) * (1.0 - 1.0 / ratio)).max(0.0)
        };
        let target = activity.target(wanted, self.db).min(MOST_DB * a);
        let k = if target < self.db {
            self.fall
        } else {
            self.rise
        };
        self.db += (target - self.db) * k;
        if target == 0.0 && self.db < 1e-3 {
            self.db = 0.0;
        }
        self.from = self.to;
        self.to = if self.db == 0.0 {
            1.0
        } else {
            db_to_gain(self.db)
        };
    }

    pub(super) fn process_frame(&mut self, frame: &mut [f32], activity: Activity) {
        if self.sub == 0 {
            self.step(activity);
        }
        let gain = self.from + (self.to - self.from) * (self.sub + 1) as f32 / SUB as f32;
        self.sub = (self.sub + 1) % SUB;
        let mut power = 0.0;
        for (ch, x) in frame.iter_mut().enumerate() {
            let y = self.rlb[ch].run(self.pre[ch].run(*x));
            power += self.weights[ch] * (y * y);
            if gain != 1.0 {
                *x *= gain;
            }
        }
        self.power += (power - self.power) * self.detect;
    }
}

fn clamped(amount: f32) -> f32 {
    if amount.is_finite() {
        amount.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::dsp::biquad::gain_to_db;
    use std::f32::consts::PI;

    const RATE: u32 = 48000;

    struct Fixture {
        drive: Drive,
        detector: crate::audio::dsp::activity::Detector,
        channels: usize,
    }

    impl Fixture {
        fn new(channels: usize, amount: f32) -> Self {
            Self {
                drive: Drive::new(RATE, channels, amount),
                detector: crate::audio::dsp::activity::Detector::new(RATE),
                channels,
            }
        }

        fn process(&mut self, samples: &mut [f32]) {
            for frame in samples.chunks_exact_mut(self.channels) {
                let state = self.detector.observe(frame);
                self.drive.process_frame(frame, state);
            }
        }
    }

    /// A stereo 1 kHz tone of `peak`, `seconds` long.
    fn tone(peak: f32, seconds: f32) -> Vec<f32> {
        (0..(RATE as f32 * seconds) as usize)
            .flat_map(|i| {
                let x = peak * (2.0 * PI * 1000.0 * i as f32 / RATE as f32).sin();
                [x, x]
            })
            .collect()
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0f32, |m, x| m.max(x.abs()))
    }

    /// The gain (dB) a tone of `level` dBFS settles at after 3 s.
    fn settled(level: f32, amount: f32) -> f32 {
        let mut d = Fixture::new(2, amount);
        let mut s = tone(db_to_gain(level), 3.0);
        d.process(&mut s);
        gain_to_db(peak(&s[s.len() - 2 * 4800..]) / db_to_gain(level))
    }

    #[test]
    fn nothing_at_zero_is_an_exact_wire() {
        let original = tone(0.3, 0.5);
        let mut s = original.clone();
        let mut d = Fixture::new(2, 0.0);
        d.process(&mut s);
        assert_eq!(s, original);
        assert!(!d.drive.active());
    }

    #[test]
    fn quiet_is_raised_towards_the_target_about_four_to_one() {
        // Two tones 20 dB apart come out about 5 dB apart (4:1).
        let quiet = settled(-32.0, 1.0);
        let loud = settled(-12.0, 1.0);
        let apart = (-12.0 + loud) - (-32.0 + quiet);
        assert!((4.0..6.0).contains(&apart), "{quiet} / {loud}: {apart} dB");
        assert!(quiet > 10.0, "{quiet} dB");
    }

    #[test]
    fn loud_sound_is_never_turned_down() {
        // A tone at -3 dBFS is over the target: never quieter than it came
        // (at its onset the level is still being followed, and the gain
        // rises a little before it settles), then exactly as it came.
        let original = tone(db_to_gain(-3.0), 2.0);
        let mut s = original.clone();
        let mut d = Fixture::new(2, 1.0);
        d.process(&mut s);
        assert_eq!(d.drive.gain_db(), 0.0);
        assert!(
            s.iter()
                .zip(&original)
                .all(|(y, x)| y.abs() >= x.abs() - 1e-7)
        );
        let last = s.len() - 2 * RATE as usize / 2;
        assert!(s[last..] == original[last..], "not settled to a wire");
    }

    #[test]
    fn the_amount_sets_how_far() {
        let half = settled(-30.0, 0.5);
        let whole = settled(-30.0, 1.0);
        assert!(half > 2.0 && half < whole - 3.0, "{half} / {whole}");
    }

    #[test]
    fn pauses_release_the_gain_without_raising_the_noise_floor() {
        let mut d = Fixture::new(2, 1.0);
        let mut hiss = tone(db_to_gain(-75.0), 2.0);
        d.process(&mut hiss);
        assert_eq!(d.drive.gain_db(), 0.0);
        d.process(&mut tone(db_to_gain(-3.0), 2.0));
        for _ in 0..100 {
            d.process(&mut [0.0; 960]);
            assert_eq!(d.drive.gain_db(), 0.0);
        }
        d.process(&mut tone(db_to_gain(-35.0), 3.0));
        assert!(d.drive.gain_db() > 17.5);
        d.process(&mut [0.0; 960]);
        d.drive.configure(0.1);
        d.process(&mut [0.0; 4800]);
        assert!(d.drive.gain_db() < 9.0, "amount cap during the hold");
        d.process(&mut vec![0.0; 2 * RATE as usize]);
        assert_eq!(d.drive.gain_db(), 0.0);
        let noise = tone(db_to_gain(-75.0), 0.3);
        let mut out = noise.clone();
        d.process(&mut out);
        assert_eq!(out, noise);
    }

    #[test]
    fn surround_level_agrees_with_the_meter_and_ignores_lfe() {
        use super::super::loudness::Meter;
        let front = tone(db_to_gain(-30.0), 2.0);
        for ch in [0, 4] {
            let mut samples: Vec<f32> = front
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .flat_map(|(i, pair)| {
                    let mut frame = [0.0; 6];
                    frame[ch] = pair[0];
                    frame[3] = 0.7 * (2.0 * PI * 60.0 * i as f32 / RATE as f32).sin();
                    frame
                })
                .collect();
            let mut meter = Meter::new(RATE, 6);
            meter.add(&samples);
            let measured = meter.integrated().unwrap();
            let mut d = Fixture::new(6, 1.0);
            d.process(&mut samples);
            assert!((d.drive.level() - measured).abs() < 0.05);
            assert!((d.drive.gain_db() - 0.75 * (-10.0 - measured)).abs() < 0.2);
        }
    }

    #[test]
    fn falls_fast_and_rises_slowly() {
        let mut d = Fixture::new(2, 1.0);
        d.process(&mut tone(db_to_gain(-35.0), 3.0));
        let high = d.drive.gain_db();
        d.process(&mut tone(db_to_gain(-15.0), 0.2));
        let low_soon = d.drive.gain_db();
        d.process(&mut tone(db_to_gain(-15.0), 2.0));
        let low = d.drive.gain_db();
        assert!(
            low_soon - low < 0.3 * (high - low),
            "{high} -> {low_soon} (0.2 s) -> {low}"
        );
        d.process(&mut tone(db_to_gain(-35.0), 0.2));
        let up_soon = d.drive.gain_db();
        assert!(
            up_soon - low < 0.6 * (high - low),
            "{low} -> {up_soon} (0.2 s), to {high}"
        );
    }

    #[test]
    fn a_gain_kept_goes_on_and_zero_settles_back_exactly() {
        let mut d = Fixture::new(2, 1.0);
        d.drive.keep(8.0);
        assert!((d.drive.gain_db() - 8.0).abs() < 1e-6);
        d.drive.configure(0.0);
        let mut s = tone(0.1, 2.0);
        d.process(&mut s);
        assert_eq!(d.drive.gain_db(), 0.0);
        assert!(!d.drive.active());
        let original = tone(0.1, 0.1);
        let mut s = original.clone();
        d.process(&mut s);
        assert_eq!(s, original);
    }

    #[test]
    fn processing_is_independent_of_block_boundaries() {
        let original = tone(0.02, 1.0);
        let mut whole = original.clone();
        let mut split = original;
        Fixture::new(2, 0.7).process(&mut whole);
        let mut d = Fixture::new(2, 0.7);
        for block in split.chunks_mut(2 * 37) {
            d.process(block);
        }
        assert_eq!(whole, split);
    }
}
