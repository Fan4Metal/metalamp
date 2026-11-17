//! Six independent controls: bass depth, bass harmonics, clarity, air,
//! stereo width and warmth. The bass detector only sees low frequencies;
//! treble transients cannot duck its gain. Clarity uses linked dynamic EQ
//! instead of synthesizing high-frequency sum tones. The boosts turn the
//! whole sound down only as far as they would push its peaks over the
//! ceiling and over the peaks it had (an adaptive reserve, at most the
//! largest boost): with headroom, as after loudness normalisation, a boost
//! is heard as a boost and nothing else gets quieter.
//! Warmth's oversampled residual and the dry signal share a fixed delay.

use super::CEILING_DB;
use super::biquad::{Biquad, Coeffs, db_to_gain, gain_to_db};
use std::f32::consts::FRAC_1_SQRT_2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Amounts {
    /// 0 to 1 each.
    pub bass: f32,
    pub bass_harmonics: f32,
    pub clarity: f32,
    pub air: f32,
    pub width: f32,
    pub warmth: f32,
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
            bass_harmonics: finite(self.bass_harmonics),
            clarity: finite(self.clarity),
            air: finite(self.air),
            width: finite(self.width),
            warmth: finite(self.warmth),
        }
    }
    /// The most the reserve may take (dB): the largest boost. The reserve
    /// itself is only what the peaks need (`Enhancer::step`); all-zero
    /// controls keep exact unity.
    pub fn most_reserve_db(self, channels: usize) -> f32 {
        (BASS_SHELF_DB * self.bass + 2.0 * self.bass_harmonics)
            .max(AIR_SHELF_DB * self.air)
            .max(if channels == 2 {
                WIDTH_DB * self.width
            } else {
                0.0
            })
            .min(6.0)
    }
}
const SLIDE_S: f32 = 0.05;
const SUB: usize = 32;
pub const BASS_SHELF_DB: f32 = 6.0;
pub const BASS_SHELF_HZ: f32 = 120.0;
const BASS_THIRD: f32 = 0.3;
const BASS_RELEASE_S: f32 = 0.2;
pub const AIR_SHELF_DB: f32 = 3.0;
pub const AIR_SHELF_HZ: f32 = 11000.0;
const WIDTH_DB: f32 = 6.0;
const WIDTH_HZ: f32 = 600.0;
const WARMTH_DRIVE: f32 = 2.5;
const HALF_TAPS: usize = 16;
// Three 2x stages, up and down: 7/4 of each half-band's delay.
// The middle decimator's late phase removes the remaining quarter frame.
pub const LATENCY: usize = (7 * (2 * HALF_TAPS - 1) - 1) / 4;
const KAISER_BETA: f64 = 7.0;
const DRY_RING: usize = 128;
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
    band: Biquad,
    env: f32,
    release: f32,
    harm_hp: Biquad,
    harm_lp: Biquad,
    shelf: Vec<Biquad>,
    low: Vec<Biquad>,
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
            band: Biquad::new(Coeffs::low_pass(rate, 120.0, FRAC_1_SQRT_2)),
            env: 0.0,
            release: release(rate, BASS_RELEASE_S),
            harm_hp: Biquad::new(Coeffs::high_pass(rate, 90.0, FRAC_1_SQRT_2)),
            harm_lp: Biquad::new(Coeffs::low_pass(rate, 350.0, FRAC_1_SQRT_2)),
            shelf: vec![Biquad::default(); channels],
            low: vec![Biquad::new(Coeffs::low_pass(rate, 120.0, FRAC_1_SQRT_2)); channels],
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
    fn set_shelf(&mut self, rate: u32, amount: f32) {
        let c = if amount == 0.0 {
            Coeffs::default()
        } else {
            Coeffs::low_shelf(rate, BASS_SHELF_HZ, BASS_SHELF_DB * amount)
        };
        for f in &mut self.shelf {
            f.set(c);
        }
    }
    fn harmonics(&mut self, mono: f32) -> f32 {
        let low = self.band.run(mono);
        self.env = follow(self.env, low.abs(), self.release);
        let n = low / self.env.max(1e-6);
        let even = low.abs();
        let third = BASS_THIRD * 4.0 * low * n * n;
        self.harm_lp.run(self.harm_hp.run(even + third))
    }
    /// Adds the boost as far as `limit` lets it; returns how much higher
    /// the frame's peak would be with the whole boost, which the reserve
    /// makes room for.
    fn run(&mut self, frame: &mut [f32], harmonics: f32) -> f32 {
        let front = frame.len().min(2);
        let mono = frame[..front].iter().sum::<f32>() / front as f32;
        let harm = harmonics * self.harmonics(mono);
        let mut low = 0f32;
        let mut boosted = 0f32;
        for (ch, &x) in frame.iter().enumerate() {
            let lows = self.low[ch].run(x);
            low = low.max(lows.abs());
            let part = self.shelf[ch].run(x) - x + if ch < front { harm } else { 0.0 };
            self.added[ch] = part;
            // Through the same low-pass, so its phase is the low end's.
            boosted = boosted.max((lows + self.low_part[ch].run(part)).abs());
        }
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

/// Linked dynamic cuts: low-mid masking, harsh presence, sibilance.
/// Each band is judged against the middle (`REFERENCE_HZ`), not against
/// the whole sound: the bass holds most of music's power, and against it
/// the presence never stood out (a cut of 0.0 dB on dense rock). A
/// band's power is weighed by its Q, so pink noise reads 0 dB in every
/// band; music falls faster than pink, so the presence normally sits a
/// few dB under the middle and the sibilance further.
const TONE_HZ: [f32; 3] = [250.0, 4000.0, 7000.0];
const TONE_Q: [f32; 3] = [0.8, 0.9, 1.2];
const REFERENCE_HZ: f32 = 1250.0;
const REFERENCE_Q: f32 = 0.7;
/// The level over the middle (dB) where each cut starts, and how many dB
/// more take it whole (`TONE_CUT_DB`, times the control's amount). Measured
/// with `--bench enhancer` on ten tracks from pop to metal and orchestral:
/// over the middle, 250 Hz has a median of +3 to +5 dB (+9 orchestral),
/// 4 kHz -5 dB (about -1 at the 90th percentile; bright metal +1), 7 kHz
/// -8 dB (-3), its sibilants +5 to +12; each starts about where the
/// loudest tenth of an ordinary mix begins.
const TONE_START_DB: [f32; 3] = [5.0, -1.0, -2.0];
const TONE_SPAN_DB: [f32; 3] = [6.0, 6.0, 6.0];
const TONE_CUT_DB: [f32; 3] = [1.5, 3.0, 3.0];
/// Band powers under this (about -100 dBFS) are not judged.
const TONE_FLOOR: f32 = 1e-10;
struct ToneChannel {
    detect: [Biquad; 3],
    reference: Biquad,
    filters: [Biquad; 3],
}
struct Tone {
    channels: Vec<ToneChannel>,
    power: [f32; 3],
    reference: f32,
    /// Each band's level over the middle, dB (for `--bench enhancer`).
    excess: [f32; 3],
    db: [f32; 3],
    mean: f32,
    attack: f32,
    release: f32,
}
impl Tone {
    fn new(rate: u32, channels: usize) -> Self {
        Self {
            channels: (0..channels)
                .map(|_| ToneChannel {
                    detect: std::array::from_fn(|i| {
                        Biquad::new(Coeffs::band_pass(rate, TONE_HZ[i], TONE_Q[i]))
                    }),
                    reference: Biquad::new(Coeffs::band_pass(rate, REFERENCE_HZ, REFERENCE_Q)),
                    filters: [Biquad::default(); 3],
                })
                .collect(),
            power: [0.0; 3],
            reference: 0.0,
            excess: [-100.0; 3],
            db: [0.0; 3],
            mean: release(rate, 0.02),
            attack: release(rate, 0.01),
            release: release(rate, 0.12),
        }
    }
    fn coefficients(&mut self, rate: u32, bass: f32, clarity: f32) {
        let reference = (self.reference * REFERENCE_Q).max(TONE_FLOOR);
        for (i, amount) in [bass, clarity, clarity].into_iter().enumerate() {
            let band = self.power[i] * TONE_Q[i];
            self.excess[i] = if band < TONE_FLOOR {
                -100.0
            } else {
                10.0 * (band / reference).log10()
            };
            let target = -TONE_CUT_DB[i]
                * amount
                * ((self.excess[i] - TONE_START_DB[i]) / TONE_SPAN_DB[i]).clamp(0.0, 1.0);
            let k = if target < self.db[i] {
                self.attack
            } else {
                self.release
            };
            // coefficients change once per SUB frames, so use the
            // equivalent time constant instead of slowing it by SUB.
            self.db[i] += (target - self.db[i]) * (1.0 - (1.0 - k).powi(SUB as i32));
            if target == 0.0 && self.db[i].abs() < 1e-4 {
                self.db[i] = 0.0;
            }
            let c = if self.db[i] == 0.0 {
                Coeffs::default()
            } else {
                Coeffs::peaking(rate, TONE_HZ[i], TONE_Q[i], self.db[i])
            };
            for ch in &mut self.channels {
                ch.filters[i].set(c);
            }
        }
    }
    fn run(&mut self, frame: &mut [f32]) {
        let mut reference = 0.0;
        let mut power = [0.0; 3];
        for (x, ch) in frame.iter_mut().zip(&mut self.channels) {
            let middle = ch.reference.run(*x);
            reference += middle * middle;
            for (i, detector) in ch.detect.iter_mut().enumerate() {
                let band = detector.run(*x);
                power[i] += band * band;
            }
            for filter in &mut ch.filters {
                *x = filter.run(*x);
            }
        }
        self.reference += (reference - self.reference) * self.mean;
        for (env, p) in self.power.iter_mut().zip(power) {
            *env += (p - *env) * self.mean;
        }
    }
}

/// The modified Bessel function of the first kind, order 0 (Kaiser's
/// window).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..50 {
        term *= (x / 2.0) / k as f64;
        sum += term * term;
        if term * term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// The half-band filter's side taps `s[i]`, at `±(2i + 1)` from the
/// middle tap (0.5); scaled so that they add up to 0.25 on each side, so
/// the filter passes DC whole.
fn half_band() -> [f32; HALF_TAPS] {
    let edge = (2 * HALF_TAPS) as f64;
    let mut s = [0f64; HALF_TAPS];
    for (i, tap) in s.iter_mut().enumerate() {
        let j = (2 * i + 1) as f64;
        let ideal = (std::f64::consts::FRAC_PI_2 * j).sin() / (std::f64::consts::PI * j);
        let r = j / edge;
        let window = bessel_i0(KAISER_BETA * (1.0 - r * r).sqrt()) / bessel_i0(KAISER_BETA);
        *tap = ideal * window;
    }
    let total: f64 = s.iter().sum();
    s.map(|t| (t * 0.25 / total) as f32)
}

/// The nonzero polyphases of a 2x half-band interpolator. Keeping only
/// input-rate history avoids multiplying zero-stuffed samples.
#[derive(Clone, Default)]
struct Interpolator {
    history: [f32; 2 * HALF_TAPS],
    at: usize,
}
impl Interpolator {
    fn pair(&mut self, x: f32, taps: &[f32; HALF_TAPS]) -> [f32; 2] {
        self.history[self.at] = x;
        let back = |n: usize| self.history[self.at.wrapping_sub(n) % self.history.len()];
        let mut even = 0.0;
        for (i, &tap) in taps.iter().enumerate() {
            even += tap * (back(HALF_TAPS - 1 - i) + back(HALF_TAPS + i));
        }
        let odd = back(HALF_TAPS - 1);
        self.at = (self.at + 1) % self.history.len();
        [2.0 * even, odd]
    }
}

/// Both input phases are retained, but the filter is evaluated only for
/// the phase which is emitted. `late` chooses the second sample's time.
#[derive(Clone, Default)]
struct Decimator {
    even: [f32; 2 * HALF_TAPS],
    odd: [f32; 2 * HALF_TAPS],
    at: usize,
}
impl Decimator {
    fn sample(&mut self, pair: [f32; 2], late: bool, taps: &[f32; HALF_TAPS]) -> f32 {
        self.even[self.at] = pair[0];
        self.odd[self.at] = pair[1];
        let back = |h: &[f32; 2 * HALF_TAPS], n: usize| h[self.at.wrapping_sub(n) % h.len()];
        let mut y = 0.5
            * if late {
                back(&self.even, HALF_TAPS - 1)
            } else {
                back(&self.odd, HALF_TAPS)
            };
        let side = if late { &self.odd } else { &self.even };
        for (i, &tap) in taps.iter().enumerate() {
            y += tap * (back(side, HALF_TAPS - 1 - i) + back(side, HALF_TAPS + i));
        }
        self.at = (self.at + 1) % self.even.len();
        y
    }
}

/// Warmth at eight times the rate and the aligned dry delay, per channel.
#[derive(Clone)]
struct WarmthChannel {
    dry: [f32; DRY_RING],
    up: [Interpolator; 3],
    down: [Decimator; 3],
}

struct Warmth {
    taps: [f32; HALF_TAPS],
    channels: Vec<WarmthChannel>,
    /// The frames gone through.
    n: usize,
    /// `tanh`'s gain making it unity at -12 dBFS.
    gain: f32,
}

impl Warmth {
    fn new(channels: usize) -> Warmth {
        Warmth {
            taps: half_band(),
            channels: vec![
                WarmthChannel {
                    dry: [0.0; DRY_RING],
                    up: std::array::from_fn(|_| Interpolator::default()),
                    down: std::array::from_fn(|_| Decimator::default()),
                };
                channels
            ],
            n: 0,
            gain: 0.25 * WARMTH_DRIVE / (0.25 * WARMTH_DRIVE).tanh() / WARMTH_DRIVE,
        }
    }

    /// Drop inactive resampler history before a new fade in. The dry
    /// delay always runs, including while warmth is at exact zero.
    fn clear_wet(&mut self) {
        for ch in &mut self.channels {
            ch.up = std::array::from_fn(|_| Interpolator::default());
            ch.down = std::array::from_fn(|_| Decimator::default());
        }
    }

    /// One frame: each sample `LATENCY` frames back, with `amount` of the
    /// saturation's part blended in.
    fn run(&mut self, frame: &mut [f32], amount: f32) {
        let n = self.n;
        self.n = n.wrapping_add(1);
        let s = &self.taps;
        for (x, ch) in frame.iter_mut().zip(&mut self.channels) {
            ch.dry[n % DRY_RING] = *x;
            let dry = |back: usize| ch.dry[n.wrapping_sub(back) % DRY_RING];
            let out = dry(LATENCY);
            if amount <= 0.0 {
                *x = out;
                continue;
            }
            // Saturate at 8x: at 2x the fifth harmonic of a 15 kHz tone
            // folds directly into 13.2 kHz at 44.1 kHz. Filtering after
            // that fold cannot remove it. Process only the nonlinear
            // residual, so the original sound never traverses these FIRs.
            let twice = ch.up[0].pair(*x, s);
            let mut four = [0.0; 4];
            for (x, pair) in twice.into_iter().zip(four.as_chunks_mut::<2>().0) {
                pair.copy_from_slice(&ch.up[1].pair(x, s));
            }
            let part = |u: f32| (WARMTH_DRIVE * u).tanh() * self.gain - u;
            for x in &mut four {
                let pair = ch.up[2].pair(*x, s).map(part);
                *x = ch.down[0].sample(pair, false, s);
            }
            let twice = std::array::from_fn::<_, 2, _>(|i| {
                ch.down[1].sample([four[2 * i], four[2 * i + 1]], true, s)
            });
            let down = ch.down[2].sample(twice, false, s);
            *x = out + amount * down;
        }
    }
}

pub struct Enhancer {
    rate: u32,
    channels: usize,
    target: Amounts,
    now: Amounts,
    slide: f32,
    shelves_for: (f32, f32, f32),
    bass: Bass,
    air: Vec<Biquad>,
    tone: Tone,
    side_shelf: Biquad,
    warmth: Warmth,
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
            shelves_for: (-1.0, -1.0, -1.0),
            bass: Bass::new(rate, channels),
            air: vec![Biquad::default(); channels],
            tone: Tone::new(rate, channels),
            side_shelf: Biquad::default(),
            warmth: Warmth::new(channels),
            sub: 0,
            in_peak: 0.0,
            out_peak: 0.0,
            peak_release: release(rate, PEAK_RELEASE_S),
            ceiling: db_to_gain(CEILING_DB),
            reserve_db: 0.0,
            reserve_from: 1.0,
            reserve_to: 1.0,
        };
        e.set_shelves();
        e
    }
    pub fn configure(&mut self, a: Amounts) {
        self.target = a.clamped();
    }
    /// Also true while controls are fading out after bypass. The final
    /// limiter must keep protecting boosts until this transition ends.
    pub fn active(&self) -> bool {
        self.now != Amounts::default()
    }
    fn set_shelves(&mut self) {
        if self.now.bass != self.shelves_for.0 {
            self.bass.set_shelf(self.rate, self.now.bass);
        }
        if self.now.air != self.shelves_for.1 {
            let c = if self.now.air == 0.0 {
                Coeffs::default()
            } else {
                Coeffs::high_shelf(self.rate, AIR_SHELF_HZ, AIR_SHELF_DB * self.now.air)
            };
            for f in &mut self.air {
                f.set(c);
            }
        }
        if self.now.width != self.shelves_for.2 {
            self.side_shelf.set(if self.now.width == 0.0 {
                Coeffs::default()
            } else {
                Coeffs::high_shelf(self.rate, WIDTH_HZ, WIDTH_DB * self.now.width)
            });
        }
        self.shelves_for = (self.now.bass, self.now.air, self.now.width);
    }
    fn step(&mut self) {
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
            bass_harmonics: slid(was.bass_harmonics, to.bass_harmonics),
            clarity: slid(was.clarity, to.clarity),
            air: slid(was.air, to.air),
            width: slid(was.width, to.width),
            warmth: slid(was.warmth, to.warmth),
        };
        if was.warmth <= 0.0 && self.now.warmth > 0.0 {
            self.warmth.clear_wet();
        }
        self.set_shelves();
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
        self.tone
            .coefficients(self.rate, self.now.bass, self.now.clarity);
    }
    /// Fixed latency even at zero. State keeps running while a stage is
    /// off, avoiding stale tails and allocations when a control moves.
    pub fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(self.channels) {
            if self.sub == 0 {
                self.step();
            }
            let reserve = self.reserve_from
                + (self.reserve_to - self.reserve_from) * (self.sub + 1) as f32 / SUB as f32;
            self.sub = (self.sub + 1) % SUB;
            let a = self.now;
            let peak = frame.iter().fold(0f32, |m, x| m.max(x.abs()));
            self.in_peak = follow(self.in_peak, peak, self.peak_release);
            self.warmth.run(frame, a.warmth);
            if reserve != 1.0 {
                for x in frame.iter_mut() {
                    *x *= reserve;
                }
            }
            let held = self.bass.run(frame, a.bass_harmonics);
            self.tone.run(frame);
            for (x, filter) in frame.iter_mut().zip(&mut self.air) {
                *x = filter.run(*x);
            }
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
        }
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

    /// Each dynamic cut's band over the middle and its cut now, dB (250 Hz,
    /// 4 kHz, 7 kHz), for `--bench enhancer`.
    pub fn tone(&self) -> ([f32; 3], [f32; 3]) {
        (self.tone.excess, self.tone.db)
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
    fn zero_amounts_only_delay() {
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, Amounts::default());
        let original = sine(rate, 440.0, 0.5, 0.5);
        let mut s = original.clone();
        e.process(&mut s);
        assert!(s[..2 * LATENCY].iter().all(|&x| x == 0.0));
        assert_eq!(s[2 * LATENCY..], original[..original.len() - 2 * LATENCY]);
    }

    #[test]
    fn half_band_passes_dc_whole() {
        let s = half_band();
        assert!((0.5 + 2.0 * s.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!((2.0 * 2.0 * s.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn warmth_lines_up_with_the_sound_it_comes_from() {
        // Quiet tones: warmth is its small-signal gain then, so
        // the output is the input `LATENCY` back times that gain; the
        // saturation's part a frame off would be out by about its whole
        // size (0.006).
        for rate in [44100, 48000] {
            for frequency in [60.0, 1000.0, 10000.0, 19000.0] {
                let mut e = Enhancer::new(rate, 2, only(|a| a.warmth = 1.0));
                let original = sine(rate, frequency, 0.05, 0.2);
                let mut s = original.clone();
                e.process(&mut s);
                let d = 0.25 * WARMTH_DRIVE;
                let g = d / d.tanh();
                let error = s[2 * (LATENCY + 128)..]
                    .iter()
                    .zip(&original[2 * 128..])
                    .map(|(&y, &x)| (y - g * x).abs())
                    .fold(0.0f32, f32::max);
                assert!(error < 0.0015, "{rate}, {frequency}: {error}");
            }
        }
    }

    #[test]
    fn warmth_folds_nothing_back() {
        // At 2x the third harmonic was suppressed, but the fifth and
        // seventh folded back as loud as -26 dBc. Check every odd order,
        // including those which already alias within the oversampler.
        for rate in [44100, 48000] {
            for frequency in [10000.0, 15000.0, 19000.0] {
                let mut e = Enhancer::new(rate, 2, only(|a| a.warmth = 1.0));
                // Double-precision phase keeps the generator's own noise
                // below the alias rejection being measured.
                let mut s: Vec<f32> = (0..2 * rate)
                    .flat_map(|i| {
                        let phase = std::f64::consts::TAU * f64::from(frequency) * f64::from(i)
                            / f64::from(rate);
                        let x = (0.9 * phase.sin()) as f32;
                        [x, x]
                    })
                    .collect();
                e.process(&mut s);
                let fundamental = level(&s, rate, frequency);
                for harmonic in (3..=31).step_by(2) {
                    let alias = (frequency * harmonic as f32) % rate as f32;
                    let alias = alias.min(rate as f32 - alias);
                    if alias == 0.0 || (alias - frequency).abs() < 1.0 {
                        continue;
                    }
                    let folded = level(&s, rate, alias);
                    assert!(
                        folded < fundamental * 0.001,
                        "{rate}, {frequency}, order {harmonic}, alias {alias}: {} dBc",
                        20.0 * (folded / fundamental).log10()
                    );
                }
            }
        }
    }

    #[test]
    fn warmth_rounds_peaks_and_keeps_quiet_levels() {
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, only(|a| a.warmth = 1.0));
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
        assert_eq!(rest[rest.len() - 1], original[2 * 48000 * 3 - 1 - 2 * LATENCY]);
    }
    #[test]
    fn a_reserve_is_carried_into_a_chain_made_anew() {
        // The air's shelf leaves 1 kHz alone; the reserve, not needed,
        // goes out at 3 dB a second: 2.9 dB are left after 30 ms.
        let mut e = Enhancer::new(48000, 2, only(|a| a.air = 1.0));
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
        let harmonics = matched(only(|a| a.bass_harmonics = 1.0), &original);
        assert!(level(&harmonics, 48000, 120.0) > 0.01);
        assert!(level(&harmonics, 48000, 180.0) > 0.005);
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
            assert!(last < peak(&original).max(ceiling) * 1.02, "{amplitude}: {last}");
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
    fn air_has_no_sum_tones() {
        let original: Vec<f32> = (0..96000)
            .flat_map(|i| {
                let t = 2.0 * PI * i as f32 / 48000.0;
                let x = 0.15 * (5000.0 * t).sin() + 0.15 * (6500.0 * t).sin();
                [x, x]
            })
            .collect();
        let s = matched(only(|a| a.air = 1.0), &original);
        assert!(level(&s, 48000, 11500.0) < 1e-4);
        assert!(level(&s, 48000, 10000.0) < 1e-4);
        let bright = sine(48000, 16000.0, 0.1, 2.0);
        let s = matched(only(|a| a.air = 1.0), &bright);
        assert!(level(&s, 48000, 16000.0) > 0.12);
    }
    #[test]
    fn clarity_tames_presence_and_preserves_the_stereo_ratio() {
        let mut original = sine(48000, 4000.0, 0.1, 2.0);
        for frame in original.as_chunks_mut::<2>().0 {
            frame[1] *= 0.5;
        }
        let s = matched(only(|a| a.clarity = 1.0), &original);
        assert!(level(&s, 48000, 4000.0) < 0.085);
        for f in s.as_chunks::<2>().0 {
            assert!((f[0] * 0.5 - f[1]).abs() < 1e-6);
        }
        let low = sine(48000, 1000.0, 0.1, 2.0);
        let s = matched(only(|a| a.clarity = 1.0), &low);
        assert!((level(&s, 48000, 1000.0) / 0.1 - 1.0).abs() < 0.02);
    }
    #[test]
    fn clarity_judges_the_presence_against_the_middle() {
        // The presence 6 dB under the middle is left alone, whatever the
        // bass under them; 3.5 dB over it is cut by about 2 dB.
        let with = |presence: f32, bass: f32| -> Vec<f32> {
            (0..96000)
                .flat_map(|i| {
                    let t = 2.0 * PI * i as f32 / 48000.0;
                    let x = bass * (50.0 * t).sin()
                        + 0.1 * (1000.0 * t).sin()
                        + presence * (4000.0 * t).sin();
                    [x, x]
                })
                .collect()
        };
        for bass in [0.0, 0.4] {
            let s = matched(only(|a| a.clarity = 1.0), &with(0.05, bass));
            let db = 20.0 * (level(&s, 48000, 4000.0) / 0.05).log10();
            assert!(db.abs() < 0.05, "{bass}: {db} dB");
            let s = matched(only(|a| a.clarity = 1.0), &with(0.15, bass));
            let db = 20.0 * (level(&s, 48000, 4000.0) / 0.15).log10();
            assert!((-3.0..-1.5).contains(&db), "{bass}: {db} dB");
        }
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
        for (out, dry) in s[2 * LATENCY..]
            .as_chunks::<2>()
            .0
            .iter()
            .zip(original.as_chunks::<2>().0)
        {
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
            let n = processed.len() / channels * channels;
            assert_eq!(
                processed[channels * LATENCY..n],
                original[..n - channels * LATENCY]
            );
        }
    }
    #[test]
    fn processing_is_independent_of_block_boundaries() {
        let a = Amounts {
            bass: 0.8,
            bass_harmonics: 0.4,
            clarity: 0.7,
            air: 0.6,
            width: 0.2,
            warmth: 0.2,
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
            air: 1.0,
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
    #[test]
    fn all_controls_stay_finite_at_supported_rates_and_layouts() {
        for rate in [8000, 22050, 44100, 48000, 96000, 192000] {
            for channels in [1, 2, 6] {
                let a = Amounts {
                    bass: 1.0,
                    bass_harmonics: 1.0,
                    clarity: 1.0,
                    air: 1.0,
                    width: 1.0,
                    warmth: 1.0,
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
