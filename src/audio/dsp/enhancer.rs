//! Bass depth, adaptive bass, brilliance and stereo width. The bass
//! detector only sees low frequencies; treble transients cannot duck its
//! gain. Brilliance is a treble shelf raised as far as the treble is dull
//! against the middle (what AudioBurst PowerFX's bands do, measured
//! through a virtual cable: `BRIGHT_*`). The boosts turn the
//! whole sound down only as far as they would push its peaks over the
//! ceiling and over the peaks it had (an adaptive reserve, at most the
//! largest boost): with headroom, as after loudness normalisation, a boost
//! is heard as a boost and nothing else gets quieter. No delay: every
//! stage is a filter on the sample it is given.

use super::CEILING_DB;
use super::activity::{Activity, Detector, STEP as SUB};
use super::biquad::{Biquad, Coeffs, db_to_gain, gain_to_db};
use std::f32::consts::FRAC_1_SQRT_2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Amounts {
    /// 0 to 1 each.
    pub bass: f32,
    pub adaptive_bass: f32,
    pub brilliance: f32,
    pub width: f32,
}
impl Amounts {
    fn clamped(self) -> Self {
        let finite = |x: f32| {
            if x.is_finite() {
                x.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        Self {
            bass: finite(self.bass),
            adaptive_bass: finite(self.adaptive_bass),
            brilliance: finite(self.brilliance),
            width: finite(self.width),
        }
    }
    /// The most the reserve may take (dB): the largest boost. The reserve
    /// itself is only what the peaks need (`Enhancer::step`); all-zero
    /// controls keep exact unity.
    pub fn most_reserve_db(self, channels: usize) -> f32 {
        (BASS_SHELF_DB * self.bass)
            .max(ADAPTIVE_BASS_MAX_DB * self.adaptive_bass)
            .max(BRIGHT_MOST_DB * self.brilliance)
            .max(if channels == 2 {
                WIDTH_DB * self.width
            } else {
                0.0
            })
            .min(6.0)
    }
}
const SLIDE_S: f32 = 0.05;
pub const BASS_SHELF_DB: f32 = 6.0;
pub const BASS_SHELF_HZ: f32 = 120.0;
const ADAPTIVE_BASS_MAX_DB: f32 = 13.0;
// Pink-noise RMS power of the 40–120 Hz detector relative to the
// 1.25 kHz reference (about -1.5 dB at 44.1–192 kHz).
const BASS_PINK_DB: f32 = -1.5;
const BASS_RELEASE_S: f32 = 0.2;
const WIDTH_DB: f32 = 6.0;
const WIDTH_HZ: f32 = 600.0;
/// The peaks before and after the enhancer are followed at once up and
/// over this time constant down.
const PEAK_RELEASE_S: f32 = 1.5;
/// The reserve moves linearly in dB: quickly in (the limiter covers the
/// first moments), slowly out, so it is a level, not a pumping.
const RESERVE_ATTACK_DB_S: f32 = 20.0;
const RESERVE_RELEASE_DB_S: f32 = 3.0;
fn release(rate: u32, seconds: f32) -> f32 {
    1.0 - (-1.0 / (rate.max(1) as f32 * seconds)).exp()
}
fn follow(env: f32, mag: f32, release: f32) -> f32 {
    if mag > env {
        mag
    } else {
        env + (mag - env) * release
    }
}
struct Bass {
    release: f32,
    shelf: Vec<Biquad>,
    low: Vec<Biquad>,
    // High-pass after the existing 120 Hz low-pass, and a mid reference.
    detectors: Vec<[Biquad; 2]>,
    low_power: f32,
    mid_power: f32,
    mean: f32,
    adaptive: ShelfGain,
    shelf_db: f32,
    low_part: Vec<Biquad>,
    added: Vec<f32>,
    low_env: f32,
    /// The low end with the whole boost.
    boosted_env: f32,
    given: f32,
    attack: f32,
    give_release: f32,
    /// The level the low end with its boost may reach: the one the reserve
    /// brings the peaks to (`Enhancer::step`).
    limit: f32,
}
impl Bass {
    fn new(rate: u32, channels: usize) -> Self {
        Self {
            release: release(rate, BASS_RELEASE_S),
            shelf: vec![Biquad::default(); channels],
            low: vec![Biquad::new(Coeffs::low_pass(rate, 120.0, FRAC_1_SQRT_2)); channels],
            detectors: vec![
                [
                    Biquad::new(Coeffs::high_pass(rate, 40.0, FRAC_1_SQRT_2)),
                    Biquad::new(Coeffs::band_pass(rate, REFERENCE_HZ, REFERENCE_Q)),
                ];
                channels
            ],
            low_power: 0.0,
            mid_power: 0.0,
            mean: release(rate, BAND_MEAN_S),
            adaptive: ShelfGain::new(rate),
            shelf_db: 0.0,
            low_part: vec![Biquad::new(Coeffs::low_pass(rate, 120.0, FRAC_1_SQRT_2)); channels],
            added: vec![0.0; channels],
            low_env: 0.0,
            boosted_env: 0.0,
            given: 1.0,
            attack: release(rate, 0.005),
            give_release: release(rate, 0.18),
            limit: db_to_gain(CEILING_DB),
        }
    }
    fn set_shelf(&mut self, rate: u32, a: Amounts, activity: Activity) {
        let mut wanted = 0.0;
        if a.adaptive_bass > 0.0 && self.mid_power > TONE_FLOOR {
            let balance =
                10.0 * (self.low_power.max(TONE_FLOOR) / self.mid_power).log10() - BASS_PINK_DB;
            // The measured low band pulls two thirds of the way to pink +6 dB.
            wanted =
                a.adaptive_bass * ((6.0 - balance) * (2.0 / 3.0)).clamp(0.0, ADAPTIVE_BASS_MAX_DB);
        }
        self.adaptive.update(wanted, activity);
        let db = (BASS_SHELF_DB * a.bass).max(self.adaptive.db);
        if db == self.shelf_db {
            return;
        }
        self.shelf_db = db;
        let c = if db == 0.0 {
            Coeffs::default()
        } else {
            Coeffs::low_shelf(rate, BASS_SHELF_HZ, db)
        };
        for f in &mut self.shelf {
            f.set(c);
        }
    }
    /// Adds the boost as far as `limit` lets it; returns how much higher
    /// the frame's peak would be with the whole boost, which the reserve
    /// makes room for.
    fn run(&mut self, frame: &mut [f32]) -> f32 {
        let mut low = 0f32;
        let mut boosted = 0f32;
        let (mut low_power, mut mid_power) = (0.0, 0.0);
        for (ch, &x) in frame.iter().enumerate() {
            let lows = self.low[ch].run(x);
            if frame.len() != 6 || ch != 3 {
                let [hp, reference] = &mut self.detectors[ch];
                low_power += hp.run(lows).powi(2);
                mid_power += reference.run(x).powi(2);
            }
            low = low.max(lows.abs());
            let part = self.shelf[ch].run(x) - x;
            self.added[ch] = part;
            // Through the same low-pass, so its phase is the low end's.
            boosted = boosted.max((lows + self.low_part[ch].run(part)).abs());
        }
        self.low_power += (low_power - self.low_power) * self.mean;
        self.mid_power += (mid_power - self.mid_power) * self.mean;
        self.low_env = follow(self.low_env, low, self.release);
        self.boosted_env = follow(self.boosted_env, boosted, self.release);
        // Only the LF envelope controls bass gain, against the level the
        // reserve aims at: an absolute level under it held the boost back
        // where the reserve would have made room (a 60 Hz sine at 0.5 got
        // +2.7 dB of 5.5, at 0.75 nothing), and the reserve, seeing only
        // what passed, never did. This catches what the reserve, moving
        // at 20 dB/s and at most the largest boost, has not yet; the final
        // limiter the mixed peaks. The low end with the whole boost is
        // followed as it is: the low end's envelope and the boost's added
        // up stood over it by their phases, and held the bass 0.4 dB back
        // where the reserve had made room.
        let grown = self.boosted_env - self.low_env;
        let target = if grown <= 1e-6 {
            1.0
        } else {
            ((self.limit - self.low_env) / grown).clamp(0.0, 1.0)
        };
        let k = if target < self.given {
            self.attack
        } else {
            self.give_release
        };
        self.given += (target - self.given) * k;
        let (mut given, mut whole) = (0f32, 0f32);
        for (x, part) in frame.iter_mut().zip(&self.added) {
            whole = whole.max((*x + part).abs());
            *x += self.given * part;
            given = given.max(x.abs());
        }
        (whole - given).max(0.0)
    }
}

/// The bass's linked dynamic cut of the low-mid masking. The band is
/// judged against the middle (`REFERENCE_HZ`), not against the whole
/// sound, which the bass dominates. Its power is weighed by its Q, so pink
/// noise reads 0 dB.
const MASK_HZ: f32 = 250.0;
const MASK_Q: f32 = 0.8;
const REFERENCE_HZ: f32 = 1250.0;
const REFERENCE_Q: f32 = 0.7;
/// The level over the middle (dB) where the cut starts, and how many dB
/// more take it whole (`MASK_CUT_DB`, times the bass's amount). Measured
/// with `--bench enhancer` on ten tracks from pop to metal and orchestral:
/// over the middle, 250 Hz has a median of +3 to +5 dB (+9 orchestral);
/// the cut starts about where the loudest tenth of an ordinary mix begins.
const MASK_START_DB: f32 = 5.0;
const MASK_SPAN_DB: f32 = 6.0;
const MASK_CUT_DB: f32 = 1.5;
/// Band powers under this (about -100 dBFS) are not judged.
const TONE_FLOOR: f32 = 1e-10;
/// Brilliance: the treble (7 to 13 kHz) judged against the
/// middle as the masking band is, pink noise reading 0 dB, and a shelf at
/// `BRIGHT_SHELF_HZ` raised by `BRIGHT_RATIO` of how far it is under
/// `BRIGHT_TARGET_DB`, at most `BRIGHT_MOST_DB`, times the control's
/// amount. PowerFX (Winamp), measured with tilted pink noise: its treble
/// over 8 kHz raised by 0.6 of how far it was under pink + 6 dB, 14 dB at
/// 18 dB under, nothing at 6 dB over; on MP3s 8 to 14 dB at 8 to 12 kHz.
/// The target here is half PowerFX's, pink + 3 dB: music falls faster than
/// pink, so an ordinary mix still gets several dB, a dull one more, a
/// bright one none. At 10, Beast In Black's Blind And Frozen (60-90 s, its
/// treble 12.7 dB under pink) gets a shelf of a mean 8.7 dB: +5.7 dB at
/// 10 kHz, +7.1 at 12.5 kHz (PowerFX: +10 and +12); with the target at
/// pink itself, +4.6 at 10 kHz. The shelf drops where cymbals come, so the
/// spectrum's mean raise is under the shelf's mean.
/// The treble band: two high-passes at `BRIGHT_FROM_HZ` (24 dB an octave)
/// and a low-pass at `BRIGHT_TO_HZ`. A band-pass of the second order let
/// so much of the middle through its skirts that a treble
/// low-passed at 3 kHz read only 11 dB under pink.
const BRIGHT_FROM_HZ: f32 = 7000.0;
const BRIGHT_TO_HZ: f32 = 13000.0;
/// The band's power over the middle's for pink noise, dB: taken off, so
/// pink reads 0 (`brilliance_reads_pink_noise_as_balanced`).
const BRIGHT_PINK_DB: f32 = -3.6;
const BRIGHT_SHELF_HZ: f32 = 8000.0;
const BRIGHT_TARGET_DB: f32 = 3.0;
const BRIGHT_RATIO: f32 = 0.6;
pub const BRIGHT_MOST_DB: f32 = 12.0;
/// The treble and the middle are followed over this (power, one pole):
/// longer than the masking cut's 20 ms, so a hi-hat's stroke does not make the
/// shelf breathe between strokes.
const BAND_MEAN_S: f32 = 0.06;
/// The shelf comes up slowly and goes down fast when the treble is back
/// (PowerFX: 10 dB in about 250 ms up, 11 dB in 60 to 100 ms down).
struct ShelfGain {
    db: f32,
    rise: f32,
    fall: f32,
}
impl ShelfGain {
    fn new(rate: u32) -> Self {
        let per_step = |seconds: f32| 1.0 - (-(SUB as f32) / (rate as f32 * seconds)).exp();
        Self {
            db: 0.0,
            rise: per_step(0.3),
            fall: per_step(0.04),
        }
    }

    fn update(&mut self, wanted: f32, activity: Activity) {
        let target = activity.target(wanted, self.db);
        let k = if target > self.db {
            self.rise
        } else {
            self.fall
        };
        self.db += (target - self.db) * k;
        if target == 0.0 && self.db < 1e-4 {
            self.db = 0.0;
        }
    }
}
/// Under this rate the treble band and shelf would sit at the edge: no
/// brilliance.
const BRIGHT_LEAST_RATE: u32 = 32000;
struct ToneChannel {
    detect: Biquad,
    reference: Biquad,
    filter: Biquad,
    bright_detect: [Biquad; 3],
    bright: Biquad,
}
struct Tone {
    channels: Vec<ToneChannel>,
    power: f32,
    reference: f32,
    /// The masking band's level over the middle, dB (for `--bench
    /// enhancer`), and its cut now.
    excess: f32,
    db: f32,
    mean: f32,
    attack: f32,
    release: f32,
    /// Brilliance's treble and middle powers, its treble over the middle
    /// (dB, as `excess`) and its shelf now.
    bright_power: f32,
    bright_reference: f32,
    bright_excess: f32,
    bright_gain: ShelfGain,
    bright_mean: f32,
    bright_able: bool,
}
impl Tone {
    fn new(rate: u32, channels: usize) -> Self {
        Self {
            channels: (0..channels)
                .map(|_| ToneChannel {
                    detect: Biquad::new(Coeffs::band_pass(rate, MASK_HZ, MASK_Q)),
                    reference: Biquad::new(Coeffs::band_pass(rate, REFERENCE_HZ, REFERENCE_Q)),
                    filter: Biquad::default(),
                    bright_detect: [
                        Biquad::new(Coeffs::high_pass(rate, BRIGHT_FROM_HZ, FRAC_1_SQRT_2)),
                        Biquad::new(Coeffs::high_pass(rate, BRIGHT_FROM_HZ, FRAC_1_SQRT_2)),
                        Biquad::new(Coeffs::low_pass(rate, BRIGHT_TO_HZ, FRAC_1_SQRT_2)),
                    ],
                    bright: Biquad::default(),
                })
                .collect(),
            power: 0.0,
            reference: 0.0,
            excess: -100.0,
            db: 0.0,
            mean: release(rate, 0.02),
            attack: release(rate, 0.01),
            release: release(rate, 0.12),
            bright_power: 0.0,
            bright_reference: 0.0,
            // Balanced until heard: a track's first moments are not raised
            // for a treble not yet measured.
            bright_excess: BRIGHT_TARGET_DB,
            // (the target itself: nothing to raise)
            bright_gain: ShelfGain::new(rate),
            bright_mean: release(rate, BAND_MEAN_S),
            bright_able: rate >= BRIGHT_LEAST_RATE,
        }
    }
    /// Brilliance's shelf for the treble heard: `amount` 0 to 1.
    fn brilliance(&mut self, rate: u32, amount: f32, activity: Activity) {
        let reference = self.bright_reference * REFERENCE_Q;
        if reference >= TONE_FLOOR {
            let band = self.bright_power;
            self.bright_excess = if band < TONE_FLOOR {
                -100.0
            } else {
                10.0 * (band / reference).log10() - BRIGHT_PINK_DB
            };
        }
        let target = if self.bright_able && reference >= TONE_FLOOR {
            // Fade the boost out when the measured treble itself approaches
            // the noise floor (full correction above -80 dBFS RMS/channel).
            let confidence = (self.bright_power / (1e-8 * self.channels.len() as f32)).min(1.0);
            confidence
                * amount
                * (BRIGHT_RATIO * (BRIGHT_TARGET_DB - self.bright_excess))
                    .clamp(0.0, BRIGHT_MOST_DB)
        } else {
            0.0
        };
        self.bright_gain.update(target, activity);
        let c = if self.bright_gain.db == 0.0 {
            Coeffs::default()
        } else {
            Coeffs::high_shelf(rate, BRIGHT_SHELF_HZ, self.bright_gain.db)
        };
        for ch in &mut self.channels {
            ch.bright.set(c);
        }
    }
    /// The masking cut for the band heard: `bass` 0 to 1.
    fn coefficients(&mut self, rate: u32, bass: f32) {
        let reference = (self.reference * REFERENCE_Q).max(TONE_FLOOR);
        let band = self.power * MASK_Q;
        self.excess = if band < TONE_FLOOR {
            -100.0
        } else {
            10.0 * (band / reference).log10()
        };
        let target =
            -MASK_CUT_DB * bass * ((self.excess - MASK_START_DB) / MASK_SPAN_DB).clamp(0.0, 1.0);
        let k = if target < self.db {
            self.attack
        } else {
            self.release
        };
        // coefficients change once per SUB frames, so use the
        // equivalent time constant instead of slowing it by SUB.
        self.db += (target - self.db) * (1.0 - (1.0 - k).powi(SUB as i32));
        if target == 0.0 && self.db.abs() < 1e-4 {
            self.db = 0.0;
        }
        let c = if self.db == 0.0 {
            Coeffs::default()
        } else {
            Coeffs::peaking(rate, MASK_HZ, MASK_Q, self.db)
        };
        for ch in &mut self.channels {
            ch.filter.set(c);
        }
    }
    fn run(&mut self, frame: &mut [f32]) {
        let mut reference = 0.0;
        let mut power = 0.0;
        let mut treble = 0.0;
        for (x, ch) in frame.iter_mut().zip(&mut self.channels) {
            let middle = ch.reference.run(*x);
            reference += middle * middle;
            let band = ch.detect.run(*x);
            power += band * band;
            let band = ch.bright_detect.iter_mut().fold(*x, |y, f| f.run(y));
            treble += band * band;
            *x = ch.bright.run(ch.filter.run(*x));
        }
        self.reference += (reference - self.reference) * self.mean;
        self.power += (power - self.power) * self.mean;
        self.bright_reference += (reference - self.bright_reference) * self.bright_mean;
        self.bright_power += (treble - self.bright_power) * self.bright_mean;
    }
}

pub struct Enhancer {
    rate: u32,
    channels: usize,
    target: Amounts,
    now: Amounts,
    slide: f32,
    /// The width the side shelf was set for.
    width_for: f32,
    activity: Detector,
    bass: Bass,
    tone: Tone,
    side_shelf: Biquad,
    sub: usize,
    /// The peaks coming in, and going out as they would without the
    /// reserve; `peak_release` their coefficient.
    in_peak: f32,
    out_peak: f32,
    peak_release: f32,
    ceiling: f32,
    /// The reserve now (dB, 0 or more), and its gain at the last step and
    /// at this one, between which each frame's gain is interpolated.
    reserve_db: f32,
    reserve_from: f32,
    reserve_to: f32,
}
impl Enhancer {
    pub fn new(rate: u32, channels: usize, a: Amounts) -> Self {
        let rate = rate.max(1);
        let channels = channels.max(1);
        let a = a.clamped();
        let mut e = Self {
            rate,
            channels,
            target: a,
            now: a,
            slide: 1.0 - (-(SUB as f32) / (rate as f32 * SLIDE_S)).exp(),
            width_for: -1.0,
            activity: Detector::new(rate),
            bass: Bass::new(rate, channels),
            tone: Tone::new(rate, channels),
            side_shelf: Biquad::default(),
            sub: 0,
            in_peak: 0.0,
            out_peak: 0.0,
            peak_release: release(rate, PEAK_RELEASE_S),
            ceiling: db_to_gain(CEILING_DB),
            reserve_db: 0.0,
            reserve_from: 1.0,
            reserve_to: 1.0,
        };
        e.set_width();
        e
    }
    pub fn configure(&mut self, a: Amounts) {
        self.target = a.clamped();
    }
    /// Also true while controls are fading out after bypass. The final
    /// limiter must keep protecting boosts until this transition ends.
    pub fn active(&self) -> bool {
        self.now != Amounts::default()
            || self.bass.adaptive.db > 0.0
            || self.tone.bright_gain.db > 0.0
    }
    fn set_width(&mut self) {
        if self.now.width != self.width_for {
            self.side_shelf.set(if self.now.width == 0.0 {
                Coeffs::default()
            } else {
                Coeffs::high_shelf(self.rate, WIDTH_HZ, WIDTH_DB * self.now.width)
            });
            self.width_for = self.now.width;
        }
    }
    fn step(&mut self, activity: Activity) {
        let slid = |now: f32, to: f32| {
            if (to - now).abs() < 1e-5 {
                to
            } else {
                now + (to - now) * self.slide
            }
        };
        let (was, to) = (self.now, self.target);
        self.now = Amounts {
            bass: slid(was.bass, to.bass),
            adaptive_bass: slid(was.adaptive_bass, to.adaptive_bass),
            brilliance: slid(was.brilliance, to.brilliance),
            width: slid(was.width, to.width),
        };
        self.set_width();
        self.bass.set_shelf(self.rate, self.now, activity);
        // What the boosts add to the peaks over both the ceiling and the
        // peaks the sound had: a master already over the ceiling is the
        // limiter's as it was without the enhancer.
        let reference = self.in_peak.max(self.ceiling);
        self.bass.limit = reference;
        let wanted = if self.out_peak > reference {
            gain_to_db(self.out_peak / reference).min(self.now.most_reserve_db(self.channels))
        } else {
            0.0
        };
        let dt = SUB as f32 / self.rate as f32;
        self.reserve_db = if wanted > self.reserve_db {
            (self.reserve_db + RESERVE_ATTACK_DB_S * dt).min(wanted)
        } else {
            (self.reserve_db - RESERVE_RELEASE_DB_S * dt).max(wanted)
        };
        self.reserve_from = self.reserve_to;
        self.reserve_to = if self.reserve_db == 0.0 {
            1.0
        } else {
            db_to_gain(-self.reserve_db)
        };
        self.tone.coefficients(self.rate, self.now.bass);
        self.tone
            .brilliance(self.rate, self.now.brilliance, activity);
    }
    /// No delay, and an exact wire at zero. State keeps running while a
    /// stage is off, avoiding stale tails and allocations when a control
    /// moves.
    pub fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(self.channels) {
            self.process_frame(frame);
        }
    }

    pub(super) fn process_frame(&mut self, frame: &mut [f32]) -> Activity {
        let activity = self.activity.observe(frame);
        if self.sub == 0 {
            self.step(activity);
        }
        let reserve = self.reserve_from
            + (self.reserve_to - self.reserve_from) * (self.sub + 1) as f32 / SUB as f32;
        self.sub = (self.sub + 1) % SUB;
        let a = self.now;
        let peak = frame.iter().fold(0f32, |m, x| m.max(x.abs()));
        self.in_peak = follow(self.in_peak, peak, self.peak_release);
        if reserve != 1.0 {
            for x in frame.iter_mut() {
                *x *= reserve;
            }
        }
        let held = self.bass.run(frame);
        self.tone.run(frame);
        if self.channels == 2 {
            let (l, r) = (frame[0], frame[1]);
            let mid = (l + r) / 2.0;
            let side = (l - r) / 2.0;
            // A monotonic side shelf avoids the LF cancellation of
            // adding a phase-shifted high-pass to the original side.
            // The mid is untouched: the width cancels in a mono sum.
            let side = self.side_shelf.run(side);
            if a.width > 0.0 {
                frame[0] = mid + side;
                frame[1] = mid - side;
            }
        }
        // As they would be without the reserve and with the whole
        // bass boost, so the reserve makes room for what was held back.
        let peak = (frame.iter().fold(0f32, |m, x| m.max(x.abs())) + held) / reserve;
        self.out_peak = follow(self.out_peak, peak, self.peak_release);
        activity
    }

    /// The reserve taken now, dB.
    pub fn reserve_db(&self) -> f32 {
        self.reserve_db
    }

    /// The share of the bass's boost its guard lets through now (0 to 1),
    /// for `--bench enhancer`.
    pub fn bass_given(&self) -> f32 {
        self.bass.given
    }

    /// Gain requested of the shared bass shelf, before its peak guard.
    pub fn bass_shelf_db(&self) -> f32 {
        self.bass.shelf_db
    }

    /// The reserve carried into a chain made anew (another track, a seek),
    /// so a track of the same loud album does not start without it; its
    /// peaks are measured afresh.
    pub fn keep_reserve(&mut self, db: f32) {
        self.reserve_db = db.max(0.0);
        let gain = if self.reserve_db == 0.0 {
            1.0
        } else {
            db_to_gain(-self.reserve_db)
        };
        self.reserve_from = gain;
        self.reserve_to = gain;
    }

    /// The masking band (250 Hz) over the middle and its cut now, dB, for
    /// `--bench enhancer`.
    pub fn masking(&self) -> (f32, f32) {
        (self.tone.excess, self.tone.db)
    }

    /// Brilliance's treble over the middle and its shelf now, dB, for
    /// `--bench enhancer`.
    pub fn brilliance(&self) -> (f32, f32) {
        (self.tone.bright_excess, self.tone.bright_gain.db)
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

    /// The amplitude of `f` in the left channel of the last second of
    /// interleaved stereo `s` (a whole number of its periods, `f` in Hz).
    fn level(s: &[f32], rate: u32, f: f32) -> f32 {
        let left: Vec<f32> = s[s.len() - 2 * rate as usize..]
            .iter()
            .step_by(2)
            .copied()
            .collect();
        let (mut c, mut si) = (0f64, 0f64);
        for (i, &x) in left.iter().enumerate() {
            let w = 2.0 * std::f64::consts::PI * f64::from(f) * i as f64 / f64::from(rate);
            c += f64::from(x) * w.cos();
            si += f64::from(x) * w.sin();
        }
        (2.0 * (c * c + si * si).sqrt() / left.len() as f64) as f32
    }

    fn only(f: impl FnOnce(&mut Amounts)) -> Amounts {
        let mut a = Amounts::default();
        f(&mut a);
        a
    }

    #[test]
    fn zero_amounts_are_a_wire() {
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, Amounts::default());
        let original = sine(rate, 440.0, 0.5, 0.5);
        let mut s = original.clone();
        e.process(&mut s);
        assert_eq!(s, original);
    }

    /// `original` through an enhancer of `a` at 48 kHz. The test signals
    /// stay under the ceiling, so no reserve is taken.
    fn matched(a: Amounts, original: &[f32]) -> Vec<f32> {
        let mut s = original.to_vec();
        let mut e = Enhancer::new(48000, 2, a);
        e.process(&mut s);
        assert_eq!(e.reserve_db(), 0.0);
        s
    }
    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0f32, |m, x| m.max(x.abs()))
    }
    /// Two tones, `low` Hz and 1 kHz, of `amplitude` each.
    fn two_tones(low: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        (0..(48000.0 * seconds) as usize)
            .flat_map(|i| {
                let t = 2.0 * PI * i as f32 / 48000.0;
                let x = amplitude * ((low * t).sin() + (1000.0 * t).sin());
                [x, x]
            })
            .collect()
    }
    #[test]
    fn a_boost_with_headroom_turns_nothing_down() {
        // Peaks at 0.4, as after loudness normalisation: the bass is
        // raised and the middle keeps its level exactly.
        let original = two_tones(60.0, 0.2, 2.0);
        let s = matched(only(|a| a.bass = 1.0), &original);
        let middle = 20.0 * (level(&s, 48000, 1000.0) / 0.2).log10();
        assert!(middle.abs() < 0.05, "{middle} dB");
        let bass = 20.0 * (level(&s, 48000, 60.0) / 0.2).log10();
        assert!(bass > 5.0, "{bass} dB");
    }
    #[test]
    fn a_boost_over_the_ceiling_takes_only_what_it_adds() {
        // Peaks at 0.9, at the ceiling: the bass would push them to about
        // 1.3, so the reserve takes about that (3 dB), not the whole 6 dB.
        let original = two_tones(60.0, 0.45, 4.0);
        let dry = peak(&original);
        let mut e = Enhancer::new(48000, 2, only(|a| a.bass = 1.0));
        let mut s = original.clone();
        e.process(&mut s);
        let reserve = e.reserve_db();
        assert!((1.0..5.0).contains(&reserve), "{reserve}");
        let last = peak(&s[s.len() - 48000..]);
        assert!(last < dry.max(db_to_gain(CEILING_DB)) * 1.03, "{last}");
        // Turned off, the reserve goes back to exact unity.
        e.configure(Amounts::default());
        let mut rest = original[..2 * 48000 * 3].to_vec();
        e.process(&mut rest);
        assert_eq!(e.reserve_db(), 0.0);
        assert_eq!(rest[rest.len() - 1], original[2 * 48000 * 3 - 1]);
    }
    #[test]
    fn a_reserve_is_carried_into_a_chain_made_anew() {
        // The width leaves a sound without a side alone; the reserve, not
        // needed, goes out at 3 dB a second: 2.9 dB are left after 30 ms.
        let mut e = Enhancer::new(48000, 2, only(|a| a.width = 1.0));
        e.keep_reserve(3.0);
        let mut s = sine(48000, 1000.0, 0.1, 0.03);
        e.process(&mut s);
        let out = peak(&s[s.len() - 2 * 480..]);
        assert!((20.0 * (out / 0.1).log10() + 2.9).abs() < 0.1, "{out}");
    }
    #[test]
    fn bass_depth_does_not_synthesize_harmonics() {
        let original = sine(48000, 60.0, 0.1, 2.0);
        let s = matched(only(|a| a.bass = 1.0), &original);
        assert!(level(&s, 48000, 60.0) > 0.18);
        assert!(level(&s, 48000, 120.0) < 1e-4);
    }
    #[test]
    fn bass_cuts_the_low_mids_only_when_they_mask_the_middle() {
        // 250 Hz well over 1 kHz: the whole masking cut at Bass 10; level
        // with it: none.
        let with = |low_mid: f32| -> Vec<f32> {
            (0..96000)
                .flat_map(|i| {
                    let t = 2.0 * PI * i as f32 / 48000.0;
                    let x = low_mid * (250.0 * t).sin() + 0.05 * (1000.0 * t).sin();
                    [x, x]
                })
                .collect()
        };
        for (low_mid, cut) in [(0.5, -MASK_CUT_DB), (0.05, 0.0)] {
            let mut e = Enhancer::new(48000, 2, only(|a| a.bass = 1.0));
            e.process(&mut with(low_mid));
            let (_, db) = e.masking();
            assert!((db - cut).abs() < 0.05, "{low_mid}: {db} dB");
        }
    }
    #[test]
    fn a_midrange_transient_cannot_duck_the_bass() {
        let mut original = sine(48000, 60.0, 0.1, 3.0);
        for (i, frame) in original.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            // Loud, but under the ceiling with the bass raised: the
            // reserve (a level for everything) is not what is tested.
            if (48000..48480).contains(&i) {
                let x = 0.6 * (2.0 * PI * 2000.0 * i as f32 / 48000.0).sin();
                for y in frame {
                    *y += x;
                }
            }
        }
        let s = matched(only(|a| a.bass = 1.0), &original);
        // Whole 60 Hz cycles before and after the 10 ms midrange burst.
        let amplitude = |start: usize| {
            let part = s[start * 2..(start + 4800) * 2].to_vec();
            rms(&part) * 2f32.sqrt()
        };
        let (before, after) = (amplitude(33600), amplitude(50400));
        assert!(
            (20.0 * (after / before).log10()).abs() < 0.15,
            "{before} -> {after}"
        );
    }
    #[test]
    fn loud_bass_is_raised_against_the_rest() {
        // A loud bass line under a quiet middle: the bass stands out by
        // nearly the whole shelf, the reserve making room, and the peaks
        // end at the ceiling. An absolute guard at 0.7 gave the bass at
        // 0.5 only +2.7 dB and at 0.75 nothing.
        for amplitude in [0.2, 0.5, 0.75] {
            let original: Vec<f32> = (0..3 * 48000)
                .flat_map(|i| {
                    let t = 2.0 * PI * i as f32 / 48000.0;
                    let x = amplitude * (60.0 * t).sin() + 0.05 * (1000.0 * t).sin();
                    [x, x]
                })
                .collect();
            let mut e = Enhancer::new(48000, 2, only(|a| a.bass = 1.0));
            let mut s = original.clone();
            e.process(&mut s);
            let ratio = |s: &[f32]| level(s, 48000, 60.0) / level(s, 48000, 1000.0);
            let raised = 20.0 * (ratio(&s) / ratio(&original)).log10();
            assert!(raised > 5.0, "{amplitude}: {raised} dB");
            let last = peak(&s[s.len() - 48000..]);
            let ceiling = db_to_gain(CEILING_DB);
            assert!(
                last < peak(&original).max(ceiling) * 1.02,
                "{amplitude}: {last}"
            );
        }
    }
    #[test]
    fn bass_limiter_responds_to_low_frequency_overload() {
        // A loud bass comes in at once: the guard holds it while the
        // reserve, at 20 dB a second, is still coming in, then lets the
        // boost through.
        let mut e = Enhancer::new(48000, 2, only(|a| a.bass = 1.0));
        let loud = sine(48000, 60.0, 0.8, 2.1);
        let mut s = loud[..2 * 4800].to_vec();
        e.process(&mut s);
        assert!(e.bass.given < 0.5, "{}", e.bass.given);
        // After its first 20 ms: the onset's first quarter period passes
        // before the envelopes have it (the final limiter's).
        let held = peak(&s[2 * 960..]);
        assert!(held < 0.95, "{held}");
        let mut rest = loud[2 * 4800..].to_vec();
        e.process(&mut rest);
        assert!(e.bass.given > 0.95, "{}", e.bass.given);
        let reserve = e.reserve_db();
        assert!((4.0..6.0).contains(&reserve), "{reserve}");
        let mut over = sine(48000, 60.0, 1.5, 2.0);
        e.process(&mut over);
        assert!(over.iter().all(|x| x.is_finite()));
    }
    #[test]
    fn width_has_no_low_frequency_dip_and_keeps_the_mono_sum() {
        let a = only(|a| a.width = 1.0);
        // An isolated side signal exposes cancellation that a mono
        // sine cannot: dry + a Butterworth HP lost ~1 dB near 100 Hz.
        for hz in [40.0, 80.0, 100.0, 160.0, 250.0, 600.0, 2000.0, 10000.0] {
            let mut original = sine(48000, hz, 0.1, 2.0);
            for frame in original.as_chunks_mut::<2>().0 {
                frame[1] = -frame[0];
            }
            let s = matched(a, &original);
            let db = 20.0 * (level(&s, 48000, hz) / 0.1).log10();
            assert!((-0.03..=6.03).contains(&db), "{hz} Hz: {db} dB");
            if hz <= 100.0 {
                assert!(db < 0.03, "LF stereo changed: {hz} Hz, {db} dB");
            }
            if hz >= 10000.0 {
                assert!(db > 5.9, "HF widening missing: {db} dB");
            }
        }
        let original: Vec<f32> = (0..48000)
            .flat_map(|i| {
                let t = 2.0 * PI * i as f32 / 48000.0;
                [(431.0 * t).sin() * 0.2, (977.0 * t).cos() * 0.1]
            })
            .collect();
        let s = matched(a, &original);
        for (out, dry) in s.as_chunks::<2>().0.iter().zip(original.as_chunks::<2>().0) {
            assert!(((out[0] + out[1]) - (dry[0] + dry[1])).abs() < 2e-7);
        }
    }
    #[test]
    fn width_cannot_turn_down_mono_or_multichannel_sound() {
        for channels in [1, 6] {
            let a = only(|a| a.width = 1.0);
            assert_eq!(a.most_reserve_db(channels), 0.0);
            let original: Vec<f32> = (0..10000).map(|i| (i as f32 * 0.1).sin() * 0.1).collect();
            let mut processed = original.clone();
            Enhancer::new(48000, channels, a).process(&mut processed);
            assert_eq!(processed, original);
        }
    }
    #[test]
    fn processing_is_independent_of_block_boundaries() {
        let a = Amounts {
            bass: 0.8,
            adaptive_bass: 0.6,
            brilliance: 0.5,
            width: 0.2,
        };
        let original = sine(48000, 63.0, 0.3, 0.2);
        let mut whole = original.clone();
        let mut split = original;
        let mut one = Enhancer::new(48000, 2, Amounts::default());
        one.configure(a);
        one.process(&mut whole);
        let mut many = Enhancer::new(48000, 2, Amounts::default());
        many.configure(a);
        for block in split.chunks_mut(74) {
            many.process(block);
        }
        assert_eq!(whole, split);
    }
    #[test]
    fn zero_controls_and_a_live_transition_stay_continuous() {
        let mut e = Enhancer::new(48000, 2, Amounts::default());
        let mut s = sine(48000, 60.0, 0.3, 2.0);
        e.process(&mut s[..96000]);
        e.configure(Amounts {
            bass: 1.0,
            brilliance: 1.0,
            ..Default::default()
        });
        e.process(&mut s[96000..]);
        let jump = s
            .windows(3)
            .step_by(2)
            .map(|w| (w[2] - w[0]).abs())
            .fold(0f32, f32::max);
        assert!(jump < 0.01, "{jump}");
    }
    /// Pink noise, the same in both channels, about 0.1 RMS (Paul Kellet's
    /// filter: within 0.05 dB of 3 dB an octave from 10 Hz at 44.1 kHz),
    /// through `shelf` (none: as it is).
    fn pink(rate: u32, seconds: f32, shelf: Option<Coeffs>) -> Vec<f32> {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut b = [0f32; 7];
        let mut f = shelf.map(Biquad::new);
        (0..(rate as f32 * seconds) as usize)
            .flat_map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let white = (seed >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;
                b[0] = 0.99886 * b[0] + white * 0.055_517_9;
                b[1] = 0.99332 * b[1] + white * 0.075_075_9;
                b[2] = 0.969 * b[2] + white * 0.153_852;
                b[3] = 0.8665 * b[3] + white * 0.310_485_6;
                b[4] = 0.55 * b[4] + white * 0.532_952_2;
                b[5] = -0.7616 * b[5] - white * 0.016_898;
                let mut x = (b.iter().sum::<f32>() + white * 0.5362) * 0.03;
                b[6] = white * 0.115_926;
                if let Some(f) = &mut f {
                    x = f.run(x);
                }
                [x, x]
            })
            .collect()
    }

    /// Brilliance's treble over the middle and its shelf after `s`, dB.
    fn brilliance_after(rate: u32, s: &[f32]) -> (f32, f32) {
        let mut e = Enhancer::new(rate, 2, only(|a| a.brilliance = 1.0));
        e.process(&mut s.to_vec());
        e.brilliance()
    }

    #[test]
    fn adaptive_bass_follows_the_input_balance_and_shares_the_manual_shelf() {
        let rate = 48000;
        let a = only(|a| a.adaptive_bass = 1.0);
        // The same pink noise throughout. Judge a full second so the
        // outcome does not depend on a single bass note at the end.
        let measure = |mut s: Vec<f32>, amounts: Amounts| {
            let mut e = Enhancer::new(rate, 2, amounts);
            let mut db = Vec::new();
            for block in s.chunks_mut(2 * 4800) {
                e.process(block);
                db.push(e.bass.shelf_db);
            }
            db[db.len() - 10..].iter().sum::<f32>() / 10.0
        };
        for (tilt, range) in [
            (-24.0, 12.0..13.01),
            (-6.0, 6.8..9.0),
            (0.0, 3.0..5.0),
            (9.0, 0.0..0.6),
        ] {
            let s = pink(rate, 3.0, Some(Coeffs::low_shelf(rate, 280.0, tilt)));
            let actual = measure(s, a);
            assert!(range.contains(&actual), "tilt {tilt}: {actual} dB");
        }
        let both = Amounts { bass: 1.0, ..a };
        let db = measure(pink(rate, 3.0, None), both);
        assert!((db - BASS_SHELF_DB).abs() < 0.1, "{db} dB");
    }

    #[test]
    fn brilliance_reads_pink_noise_as_balanced() {
        for rate in [44100, 48000, 96000] {
            let (excess, db) = brilliance_after(rate, &pink(rate, 3.0, None));
            assert!(excess.abs() < 1.0, "{rate}: {excess} dB");
            // Pink is raised towards pink + `BRIGHT_TARGET_DB`.
            let wanted = BRIGHT_RATIO * (BRIGHT_TARGET_DB - excess);
            assert!((db - wanted).abs() < 0.3, "{rate}: {db} dB");
        }
    }

    #[test]
    fn brilliance_raises_dull_treble_and_leaves_bright_treble_alone() {
        let rate = 44100;
        // The treble over 4 kHz 12 dB under pink: raised by 0.6 of how far
        // it is under the target.
        let dull = pink(rate, 3.0, Some(Coeffs::high_shelf(rate, 4000.0, -12.0)));
        let (excess, db) = brilliance_after(rate, &dull);
        assert!((-12.5..-10.5).contains(&excess), "{excess} dB");
        let wanted = BRIGHT_RATIO * (BRIGHT_TARGET_DB - excess);
        assert!((db - wanted).abs() < 0.5, "{excess} dB: {db} dB");
        // At most `BRIGHT_MOST_DB`, however dull.
        let dark = pink(
            rate,
            3.0,
            Some(Coeffs::low_pass(rate, 3000.0, FRAC_1_SQRT_2)),
        );
        let (_, db) = brilliance_after(rate, &dark);
        assert!(
            (BRIGHT_MOST_DB - 0.1..=BRIGHT_MOST_DB).contains(&db),
            "{db} dB"
        );
        // 6 dB over pink: nothing at all, the shelf an exact wire.
        let bright = pink(rate, 3.0, Some(Coeffs::high_shelf(rate, 4000.0, 6.0)));
        let mut s = bright.clone();
        let mut e = Enhancer::new(rate, 2, only(|a| a.brilliance = 1.0));
        e.process(&mut s);
        assert_eq!(e.brilliance().1, 0.0);
        assert_eq!(s, bright);
    }

    #[test]
    fn brilliance_comes_up_slowly_and_goes_down_fast() {
        let rate = 44100;
        let dull = pink(rate, 3.0, Some(Coeffs::high_shelf(rate, 4000.0, -12.0)));
        // Over the target: nothing wanted once the treble is back.
        let bright = pink(rate, 1.0, Some(Coeffs::high_shelf(rate, 4000.0, 6.0)));
        let mut e = Enhancer::new(rate, 2, only(|a| a.brilliance = 1.0));
        let tenth = 2 * rate as usize / 10;
        let mut s = dull[..tenth].to_vec();
        e.process(&mut s);
        let early = e.brilliance().1;
        let mut s = dull[tenth..].to_vec();
        e.process(&mut s);
        let raised = e.brilliance().1;
        assert!(raised > 7.0, "{raised} dB");
        // About 0.3 s to come up: under half of it after 0.1 s.
        assert!(
            (0.1..0.5).contains(&(early / raised)),
            "after 0.1 s: {early} of {raised} dB"
        );
        let mut s = bright[..tenth].to_vec();
        e.process(&mut s);
        let back = e.brilliance().1;
        assert!(
            back < 0.35 * raised,
            "0.1 s after the treble came back: {back} of {raised} dB"
        );
        let mut s = bright[tenth..2 * tenth].to_vec();
        e.process(&mut s);
        let gone = e.brilliance().1;
        assert!(gone < 0.1 * raised, "0.2 s after: {gone} of {raised} dB");
    }

    #[test]
    fn brilliance_raises_the_treble_it_measures() {
        // A dull mix: 1 kHz at 0.1, 10 kHz at 0.01 (pink would hold the
        // band 10 kHz as loud as 1 kHz's).
        let rate = 48000;
        let original: Vec<f32> = (0..3 * rate as usize)
            .flat_map(|i| {
                let t = 2.0 * PI * i as f32 / rate as f32;
                let x = 0.1 * (1000.0 * t).sin() + 0.01 * (10000.0 * t).sin();
                [x, x]
            })
            .collect();
        let s = matched(only(|a| a.brilliance = 1.0), &original);
        let treble = 20.0 * (level(&s, rate, 10000.0) / 0.01).log10();
        let middle = 20.0 * (level(&s, rate, 1000.0) / 0.1).log10();
        assert!(treble > 5.0, "{treble} dB");
        assert!(middle.abs() < 0.2, "{middle} dB");
    }

    #[test]
    fn all_controls_stay_finite_at_supported_rates_and_layouts() {
        for rate in [8000, 22050, 44100, 48000, 96000, 192000] {
            for channels in [1, 2, 6] {
                let a = Amounts {
                    bass: 1.0,
                    adaptive_bass: 1.0,
                    brilliance: 1.0,
                    width: 1.0,
                };
                let mut e = Enhancer::new(rate, channels, a);
                let mut s: Vec<f32> = (0..rate as usize / 5)
                    .flat_map(|i| {
                        std::iter::repeat_n(if i % 17 < 8 { 1.0 } else { -1.0 }, channels)
                    })
                    .collect();
                s.extend(std::iter::repeat_n(0.0, rate as usize / 10 * channels));
                e.process(&mut s);
                assert!(
                    s.iter().all(|x| x.is_finite() && x.abs() < 4.0),
                    "{rate}, {channels}"
                );
            }
        }
    }
}
