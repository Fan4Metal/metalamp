//! Six independent controls: bass depth, bass harmonics, clarity, air,
//! stereo width and warmth. The bass detector only sees low frequencies;
//! treble transients cannot duck its gain. Clarity uses linked dynamic EQ
//! instead of synthesizing high-frequency sum tones. Enhancement reserves
//! up to 6 dB before the boosts; comparisons need matched listening levels.
//! Warmth's oversampled residual and the dry signal share a fixed delay.

use super::biquad::{Biquad, Coeffs, db_to_gain};
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
    /// Processing headroom, not output makeup gain. The largest boost
    /// determines the reserve; all-zero controls keep exact unity.
    pub fn headroom_db(self, channels: usize) -> f32 {
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
    added: Vec<f32>,
    low_env: f32,
    added_env: f32,
    given: f32,
    attack: f32,
    give_release: f32,
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
            added: vec![0.0; channels],
            low_env: 0.0,
            added_env: 0.0,
            given: 1.0,
            attack: release(rate, 0.005),
            give_release: release(rate, 0.18),
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
    fn run(&mut self, frame: &mut [f32], harmonics: f32) {
        let front = frame.len().min(2);
        let mono = frame[..front].iter().sum::<f32>() / front as f32;
        let harm = harmonics * self.harmonics(mono);
        let mut low = 0f32;
        let mut added = 0f32;
        for (ch, &x) in frame.iter().enumerate() {
            low = low.max(self.low[ch].run(x).abs());
            let part = self.shelf[ch].run(x) - x + if ch < front { harm } else { 0.0 };
            self.added[ch] = part;
            added = added.max(part.abs());
        }
        self.low_env = follow(self.low_env, low, self.release);
        self.added_env = follow(self.added_env, added, self.release);
        // Only the LF envelope controls bass gain. The final limiter
        // catches residual mixed-signal peaks, after the headroom reserve.
        let target = ((0.7 - self.low_env) / self.added_env.max(1e-6)).clamp(0.0, 1.0);
        let k = if target < self.given {
            self.attack
        } else {
            self.give_release
        };
        self.given += (target - self.given) * k;
        for (x, part) in frame.iter_mut().zip(&self.added) {
            *x += self.given * part;
        }
    }
}

/// Linked dynamic cuts: low-mid masking, harsh presence, sibilance.
/// Relative band energy makes the decisions independent of input gain.
const TONE_HZ: [f32; 3] = [250.0, 4000.0, 7000.0];
const TONE_Q: [f32; 3] = [0.8, 0.9, 1.2];
struct ToneChannel {
    detect: [Biquad; 3],
    filters: [Biquad; 3],
}
struct Tone {
    channels: Vec<ToneChannel>,
    power: [f32; 3],
    wide: f32,
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
                    filters: [Biquad::default(); 3],
                })
                .collect(),
            power: [0.0; 3],
            wide: 0.0,
            db: [0.0; 3],
            mean: release(rate, 0.02),
            attack: release(rate, 0.01),
            release: release(rate, 0.12),
        }
    }
    fn coefficients(&mut self, rate: u32, bass: f32, clarity: f32) {
        for (i, amount) in [bass, clarity, clarity].into_iter().enumerate() {
            let threshold = [0.35, 0.22, 0.12][i];
            let ratio = if self.wide > 1e-10 {
                self.power[i] / self.wide
            } else {
                0.0
            };
            let target = -[1.5, 3.0, 3.0][i]
                * amount
                * ((ratio - threshold) / (1.0 - threshold)).clamp(0.0, 1.0);
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
        let mut wide = 0.0;
        let mut power = [0.0; 3];
        for (x, ch) in frame.iter_mut().zip(&mut self.channels) {
            wide += *x * *x;
            for (i, detector) in ch.detect.iter_mut().enumerate() {
                let band = detector.run(*x);
                power[i] += band * band;
            }
            for filter in &mut ch.filters {
                *x = filter.run(*x);
            }
        }
        self.wide += (wide - self.wide) * self.mean;
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
    reserve: f32,
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
            reserve: db_to_gain(-a.headroom_db(channels)),
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
        self.reserve = db_to_gain(-self.now.headroom_db(self.channels));
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
            self.sub = (self.sub + 1) % SUB;
            let a = self.now;
            self.warmth.run(frame, a.warmth);
            let reserve = self.reserve;
            for x in frame.iter_mut() {
                *x *= reserve;
            }
            self.bass.run(frame, a.bass_harmonics);
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
        }
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

    /// Remove the intentional reserve for spectral comparisons only.
    fn matched(a: Amounts, original: &[f32]) -> Vec<f32> {
        let mut s = original.to_vec();
        Enhancer::new(48000, 2, a).process(&mut s);
        for x in &mut s {
            *x *= db_to_gain(a.headroom_db(2));
        }
        s
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
            if (48000..48480).contains(&i) {
                let x = 0.85 * (2.0 * PI * 2000.0 * i as f32 / 48000.0).sin();
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
    fn bass_limiter_responds_to_low_frequency_overload() {
        let mut e = Enhancer::new(48000, 2, only(|a| a.bass = 1.0));
        let mut s = sine(48000, 60.0, 1.5, 2.0);
        e.process(&mut s);
        assert!(e.bass.given < 0.2, "{}", e.bass.given);
        assert!(s.iter().all(|x| x.is_finite()));
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
            assert_eq!(a.headroom_db(channels), 0.0);
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
