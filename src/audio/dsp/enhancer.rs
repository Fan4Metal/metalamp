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
    pub fn headroom_db(self) -> f32 {
        (BASS_SHELF_DB * self.bass + 2.0 * self.bass_harmonics)
            .max(AIR_SHELF_DB * self.air)
            .max(6.0 * self.width)
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
const WARMTH_DRIVE: f32 = 2.5;
const HALF_TAPS: usize = 8;
pub const LATENCY: usize = 2 * HALF_TAPS - 1;
const KAISER_BETA: f64 = 7.0;
const DRY_RING: usize = 32;
const WET_RING: usize = 64;
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
        let c = Coeffs::low_shelf(rate, BASS_SHELF_HZ, BASS_SHELF_DB * amount);
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
            let c = Coeffs::peaking(rate, TONE_HZ[i], TONE_Q[i], self.db[i]);
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

/// Warmth at twice the rate, and the delay of the dry sound, per channel.
#[derive(Clone)]
struct WarmthChannel {
    /// The input, by frame.
    dry: [f32; DRY_RING],
    /// The saturation's part at twice the rate, by its time.
    wet: [f32; WET_RING],
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
                    wet: [0.0; WET_RING],
                };
                channels
            ],
            n: 0,
            gain: 0.25 * WARMTH_DRIVE / (0.25 * WARMTH_DRIVE).tanh() / WARMTH_DRIVE,
        }
    }

    /// The saturation's part dropped (it is of the sound long before).
    fn clear_wet(&mut self) {
        for ch in &mut self.channels {
            ch.wet = [0.0; WET_RING];
        }
    }

    /// One frame: each sample `LATENCY` frames back, with `amount` of the
    /// saturation's part blended in.
    fn run(&mut self, frame: &mut [f32], amount: f32) {
        let n = self.n;
        self.n = n.wrapping_add(1);
        let m = HALF_TAPS - 1;
        let s = &self.taps;
        for (x, ch) in frame.iter_mut().zip(&mut self.channels) {
            ch.dry[n % DRY_RING] = *x;
            let dry = |back: usize| ch.dry[n.wrapping_sub(back) % DRY_RING];
            let out = dry(LATENCY);
            if amount <= 0.0 {
                *x = out;
                continue;
            }
            // Up: the zero-stuffed input through the half-band filter (times
            // two). The odd phase is the input `m` frames back; the even one
            // lies half a frame before it.
            let mut even = 0.0;
            for (i, &tap) in s.iter().enumerate() {
                even += tap * (dry(m - i) + dry(m + 1 + i));
            }
            let even = 2.0 * even;
            let odd = dry(m);
            let part = |u: f32| (WARMTH_DRIVE * u).tanh() * self.gain - u;
            let t = 2 * n;
            ch.wet[t % WET_RING] = part(even);
            ch.wet[(t + 1) % WET_RING] = part(odd);
            // Down: the half-band filter at time `t`, which is the frame
            // `LATENCY` back; its middle tap falls on an odd time, the side
            // taps on even ones.
            let wet = |at: usize| ch.wet[at % WET_RING];
            let middle = t + WET_RING - LATENCY;
            let mut down = 0.5 * wet(middle);
            for (i, &tap) in s.iter().enumerate() {
                down += tap * (wet(middle + 2 * i + 1) + wet(middle - 2 * i - 1));
            }
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
    shelves_for: (f32, f32),
    bass: Bass,
    air: Vec<Biquad>,
    tone: Tone,
    side_hp: Biquad,
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
            shelves_for: (-1.0, -1.0),
            bass: Bass::new(rate, channels),
            air: vec![Biquad::default(); channels],
            tone: Tone::new(rate, channels),
            side_hp: Biquad::new(Coeffs::high_pass(rate, 250.0, FRAC_1_SQRT_2)),
            warmth: Warmth::new(channels),
            sub: 0,
            reserve: db_to_gain(-a.headroom_db()),
        };
        e.set_shelves();
        e
    }
    pub fn configure(&mut self, a: Amounts) {
        self.target = a.clamped();
    }
    fn set_shelves(&mut self) {
        if (self.now.bass - self.shelves_for.0).abs() > 1e-4 {
            self.bass.set_shelf(self.rate, self.now.bass);
        }
        if (self.now.air - self.shelves_for.1).abs() > 1e-4 {
            let c = Coeffs::high_shelf(self.rate, AIR_SHELF_HZ, AIR_SHELF_DB * self.now.air);
            for f in &mut self.air {
                f.set(c);
            }
        }
        self.shelves_for = (self.now.bass, self.now.air);
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
        self.reserve = db_to_gain(-self.now.headroom_db());
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
            if a.width > 0.0 && self.channels == 2 {
                let (l, r) = (frame[0], frame[1]);
                let mid = (l + r) / 2.0;
                let side = (l - r) / 2.0;
                let side = side + a.width * self.side_hp.run(side);
                frame[0] = mid + side;
                frame[1] = mid - side;
            } else if self.channels == 2 {
                self.side_hp.run((frame[0] - frame[1]) / 2.0);
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
        // A quiet 10 kHz tone: warmth is its small-signal gain then, so
        // the output is the input `LATENCY` back times that gain; the
        // saturation's part a frame off would be out by about its whole
        // size (0.006).
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, only(|a| a.warmth = 1.0));
        let original = sine(rate, 10000.0, 0.05, 0.2);
        let mut s = original.clone();
        e.process(&mut s);
        let d = 0.25 * WARMTH_DRIVE;
        let g = d / d.tanh();
        let error = s[2 * (LATENCY + 64)..]
            .iter()
            .zip(&original[2 * 64..])
            .map(|(&y, &x)| (y - g * x).abs())
            .fold(0.0f32, f32::max);
        assert!(error < 0.0015, "{error}");
    }

    #[test]
    fn warmth_folds_nothing_back() {
        // 15 kHz at 44.1 kHz: its third harmonic, 45 kHz, came back at
        // 900 Hz without the twice the rate.
        let rate = 44100;
        let mut e = Enhancer::new(rate, 2, only(|a| a.warmth = 1.0));
        let mut s = sine(rate, 15000.0, 0.9, 2.0);
        e.process(&mut s);
        let folded = level(&s, rate, 900.0);
        // Under -60 dB of the tone.
        assert!(folded < 0.9 * 0.001, "{folded}");
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
            *x *= db_to_gain(a.headroom_db());
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
