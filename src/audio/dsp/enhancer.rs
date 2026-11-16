//! The enhancer: what consumer "sound improvers" (DFX, SRS WOW,
//! AudioBurst PowerFX) do, in four amounts of 0 to 10, each 0 doing
//! nothing. In the order they run:
//!
//! - **Warmth**: a blend with a soft saturation of the whole, which
//!   rounds the peaks as tape does; first, so the bass raised after it
//!   does not drive the rest into it. The saturation's own part (what it
//!   takes from the signal and the harmonics it adds) is worked out at
//!   twice the rate and brought back through a half-band filter, so the
//!   harmonics over the Nyquist frequency do not fold back into the
//!   music (the third of a 15 kHz tone came back at 900 Hz at 44.1 kHz);
//!   the signal itself only waits `LATENCY` frames for it, always, so
//!   turning warmth on or off does not shift the sound.
//! - **Bass**: a low shelf under `BASS_SHELF_HZ`, and harmonics of the band under
//!   120 Hz, which small speakers that cannot play the fundamental still
//!   make heard as bass (the missing fundamental): the band rectified
//!   gives the even ones (2f, 4f, …), with no fundamental; the cube of
//!   the band levelled by its envelope the third (3f), with some of the
//!   fundamental in phase on the peaks, which the shelf's bass keeps; both
//!   are needed, the even ones alone are heard an octave up. Kept between
//!   90 and 500 Hz, and put into the front pair only. What the bass adds
//!   gives way to the limiter's ceiling (`Bass::give`), so a loud record
//!   gets less of it rather than the limiter turning the voice down with
//!   each beat.
//! - **Clarity**: the second harmonics (and sum tones) of the band from
//!   4.5 to 8 kHz, squared, so nothing of the band itself is added (a
//!   saturator's near-linear output, high-passed twice, cancelled the
//!   music around 5 kHz), its output over 10 kHz, and a high shelf over
//!   10 kHz: the air an MP3's low-pass took away. Kept over 10 kHz, since
//!   the sum tones of a distorted guitar's dense 3 to 5 kHz, let in at 6
//!   to 10 kHz, are grit there, not air. The band's upper edge
//!   keeps the doubled tones under the Nyquist frequency, so nothing
//!   folds back as an inharmonic whistle; their level follows the
//!   envelope of all that is over 4.5 kHz, so a tone leaking in from over
//!   the band stays as weak as it came in (and music, whose peaks the
//!   envelope holds, gets some 12 dB less of them than a sine: `AIR_MIX`).
//! - **Width**: the side signal over 250 Hz raised (the bass stays in
//!   the middle), up to twice; stereo only.
//!
//! The amounts slide to new values over `SLIDE_S`, and a stage that
//! starts again starts with empty filters (their state is of the sound
//! long before), so moving a slider or turning a stage on does not click.

use std::f32::consts::FRAC_1_SQRT_2;

use super::biquad::{Biquad, Coeffs};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Amounts {
    /// 0 to 1 each.
    pub bass: f32,
    pub clarity: f32,
    pub width: f32,
    pub warmth: f32,
}

impl Amounts {
    fn clamped(self) -> Amounts {
        Amounts {
            bass: self.bass.clamp(0.0, 1.0),
            clarity: self.clarity.clamp(0.0, 1.0),
            width: self.width.clamp(0.0, 1.0),
            warmth: self.warmth.clamp(0.0, 1.0),
        }
    }
}

/// How long a change of the amounts takes, seconds.
const SLIDE_S: f32 = 0.05;
/// The frames between two steps of the slide (and of the shelves'
/// coefficients).
const SUB: usize = 32;

/// The low shelf's gain at a full bass, dB.
pub const BASS_SHELF_DB: f32 = 6.0;
/// The low shelf's corner, where it gives half its gain: the weight of a
/// distorted guitar is at 120 to 200 Hz, and a shelf at 100 Hz raised what
/// a rock record hardly has (measured: +3 dB under 80 Hz, +0.4 dB in the
/// record's loudest band, 159 Hz).
pub const BASS_SHELF_HZ: f32 = 120.0;
/// The third harmonic's share against the rectified band's.
const BASS_THIRD: f32 = 0.3;
/// The harmonics' gain at a full bass.
const BASS_MIX: f32 = 1.0;
/// The bass envelope's fall, seconds.
const BASS_RELEASE_S: f32 = 0.2;
/// The added bass, given way to the ceiling, comes back over this, seconds.
const GIVE_RELEASE_S: f32 = 0.3;

/// The high shelf's gain at a full clarity, dB.
pub const AIR_SHELF_DB: f32 = 4.0;
/// The high shelf's corner.
pub const AIR_SHELF_HZ: f32 = 10000.0;
/// The band doubled: from here …
const AIR_FROM: f32 = 4500.0;
/// … to here (or less at low rates, see `Air::new`).
const AIR_TO: f32 = 8000.0;
/// The harmonics kept from here up.
const AIR_OUT: f32 = 10000.0;
/// The harmonics' gain at a full clarity.
const AIR_MIX: f32 = 3.0;
/// The envelope over `AIR_FROM`: its fall, seconds.
const AIR_RELEASE_S: f32 = 0.1;

/// Warmth's saturation drive.
const WARMTH_DRIVE: f32 = 2.5;

/// The half-band filter of warmth's twice the rate: its side taps on
/// either side of the middle one (`HALF_TAPS` × 4 − 1 taps in all, every
/// other one 0).
const HALF_TAPS: usize = 8;
/// The frames the enhancer delays the sound by (the half-band filter's
/// delay going up and coming down): 15, 0.34 ms at 44.1 kHz.
pub const LATENCY: usize = 2 * HALF_TAPS - 1;
/// The Kaiser window's β: about 70 dB of stopband.
const KAISER_BETA: f64 = 7.0;
/// The rings' lengths, powers of two over what is looked back on.
const DRY_RING: usize = 32;
const WET_RING: usize = 64;

fn release(rate: u32, seconds: f32) -> f32 {
    1.0 - (-1.0 / (rate as f32 * seconds)).exp()
}

/// An envelope up at once and down over its release.
fn follow(env: f32, mag: f32, release: f32) -> f32 {
    if mag > env {
        mag
    } else {
        env + (mag - env) * release
    }
}

struct Bass {
    /// The band under 120 Hz of the front pair's mean.
    band: Biquad,
    env: f32,
    release: f32,
    harm_hp: Biquad,
    harm_lp: Biquad,
    shelf: Vec<Biquad>,
    /// What the bass adds to each channel of the frame in hand.
    added: Vec<f32>,
    /// The share of it let through now (`give`): 1 while it fits under
    /// the ceiling.
    given: f32,
    give_release: f32,
}

/// The share of `added` that keeps `dry + added` within `ceiling`: 1 when
/// it fits, 0 when the dry sound alone is over it.
fn fit(dry: f32, added: f32, ceiling: f32) -> f32 {
    if (dry + added).abs() <= ceiling {
        1.0
    } else {
        ((ceiling - dry.abs()) / added.abs().max(1e-9)).clamp(0.0, 1.0)
    }
}

impl Bass {
    fn new(rate: u32, channels: usize) -> Bass {
        Bass {
            band: Biquad::new(Coeffs::low_pass(rate, 120.0, FRAC_1_SQRT_2)),
            env: 0.0,
            release: release(rate, BASS_RELEASE_S),
            harm_hp: Biquad::new(Coeffs::high_pass(rate, 90.0, FRAC_1_SQRT_2)),
            harm_lp: Biquad::new(Coeffs::low_pass(rate, 500.0, FRAC_1_SQRT_2)),
            shelf: vec![Biquad::default(); channels],
            added: vec![0.0; channels],
            given: 1.0,
            give_release: release(rate, GIVE_RELEASE_S),
        }
    }

    /// The share of the added bass let through, following `fits` (what
    /// this frame has room for): down at once, back up over
    /// `GIVE_RELEASE_S`, so on a loud passage it settles and the bass added
    /// is only less, not chopped. The limiter after the chain turns the
    /// whole sound down for a peak, the voice with it: with a bass of 6 a
    /// loud passage's peaks came to +3.8 dBFS and the voice's bands lost
    /// 0.7 to 1 dB on average (far more at each beat), heard as the voice
    /// going quiet when the enhancer was turned on. So the bass gives way
    /// first; what the limiter still takes is what the record's own peaks
    /// ask for (measured: 0.2 dB with or without the bass). With 0.5 ms of
    /// attack the peaks still passed (+1.1 dBFS, 0.4 dB taken).
    fn give(&mut self, fits: f32) -> f32 {
        if fits < self.given {
            self.given = fits;
        } else {
            self.given += (fits - self.given) * self.give_release;
        }
        self.given
    }

    fn set_shelf(&mut self, rate: u32, amount: f32) {
        let c = Coeffs::low_shelf(rate, BASS_SHELF_HZ, BASS_SHELF_DB * amount);
        for f in &mut self.shelf {
            f.set(c);
        }
    }

    /// The harmonics of `mono`'s bass.
    fn harmonics(&mut self, mono: f32) -> f32 {
        let low = self.band.run(mono);
        self.env = follow(self.env, low.abs(), self.release);
        // |n| <= 1, since the envelope rises at once.
        let n = low / self.env.max(1e-6);
        // The rectified band: 2f, 4f, … (its mean goes in the high-pass).
        let even = low.abs();
        // The cube: a tone at the envelope's level gives its third
        // harmonic at `BASS_THIRD` and its fundamental, in phase, three
        // times that. Not T3(n) = 4n³ - 3n, the third alone at that level:
        // music stays under the envelope, which holds its peaks, and there
        // -3n was the band turned over, cancelling the shelf.
        let third = BASS_THIRD * 4.0 * low * n * n;
        self.harm_lp.run(self.harm_hp.run(even + third))
    }
}

struct Air {
    /// Everything over `AIR_FROM`, whose envelope levels the band.
    hp: Biquad,
    /// Three sections: the band's upper edge steep enough that little
    /// over it is doubled past the Nyquist frequency.
    lp: [Biquad; 3],
    env: f32,
    release: f32,
    out: [Biquad; 2],
    shelf: Biquad,
}

impl Air {
    fn new(rate: u32) -> Air {
        // Doubled, the band's top stays under the Nyquist frequency.
        let top = AIR_TO.min(rate as f32 * 0.23);
        let lp = Biquad::new(Coeffs::low_pass(rate, top, FRAC_1_SQRT_2));
        let out = Biquad::new(Coeffs::high_pass(rate, AIR_OUT, FRAC_1_SQRT_2));
        Air {
            hp: Biquad::new(Coeffs::high_pass(rate, AIR_FROM, FRAC_1_SQRT_2)),
            lp: [lp; 3],
            env: 0.0,
            release: release(rate, AIR_RELEASE_S),
            out: [out; 2],
            shelf: Biquad::default(),
        }
    }

    fn run(&mut self, x: f32, mix: f32) -> f32 {
        let wide = self.hp.run(x);
        self.env = follow(self.env, wide.abs(), self.release);
        let band = self.lp.iter_mut().fold(wide, |s, f| f.run(s));
        // band² / env: the band's level, not its square, so a quiet
        // passage gets its air too; the ratio kept within 2 for a band
        // ringing over what it came from.
        let ratio = (band / self.env.max(1e-6)).clamp(-2.0, 2.0);
        let doubled = band * ratio;
        let air = self.out.iter_mut().fold(doubled, |s, f| f.run(s));
        self.shelf.run(x) + mix * AIR_MIX * air
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
    /// The amounts asked for, and those applied now, sliding to them.
    target: Amounts,
    now: Amounts,
    slide: f32,
    /// The amounts the shelves were made for.
    shelves_for: (f32, f32),
    bass: Bass,
    air: Vec<Air>,
    side_hp: Biquad,
    warmth: Warmth,
    /// The limiter's ceiling, which the added bass gives way to.
    ceiling: f32,
}

impl Enhancer {
    pub fn new(rate: u32, channels: usize, a: Amounts) -> Enhancer {
        let a = a.clamped();
        let mut e = Enhancer {
            rate,
            channels,
            target: a,
            now: a,
            slide: 1.0 - (-(SUB as f32) / (rate as f32 * SLIDE_S)).exp(),
            shelves_for: (-1.0, -1.0),
            bass: Bass::new(rate, channels),
            air: (0..channels).map(|_| Air::new(rate)).collect(),
            side_hp: Biquad::new(Coeffs::high_pass(rate, 250.0, FRAC_1_SQRT_2)),
            warmth: Warmth::new(channels),
            ceiling: super::biquad::db_to_gain(super::CEILING_DB),
        };
        e.set_shelves();
        e
    }

    /// New amounts, slid to; the filters' state is kept.
    pub fn configure(&mut self, a: Amounts) {
        self.target = a.clamped();
    }

    fn set_shelves(&mut self) {
        let (bass, clarity) = (self.now.bass, self.now.clarity);
        let (was_bass, was_clarity) = self.shelves_for;
        if (bass - was_bass).abs() > 1e-4 {
            self.bass.set_shelf(self.rate, bass);
        }
        if (clarity - was_clarity).abs() > 1e-4 {
            let c = Coeffs::high_shelf(self.rate, AIR_SHELF_HZ, AIR_SHELF_DB * clarity);
            for air in &mut self.air {
                air.shelf.set(c);
            }
        }
        self.shelves_for = (bass, clarity);
    }

    /// One step of the slide; a stage starting from 0 starts with empty
    /// filters.
    fn step(&mut self) {
        let k = self.slide;
        let slid = |now: f32, to: f32| {
            if (to - now).abs() < 1e-3 {
                to
            } else {
                now + (to - now) * k
            }
        };
        let (was, to) = (self.now, self.target);
        self.now = Amounts {
            bass: slid(was.bass, to.bass),
            clarity: slid(was.clarity, to.clarity),
            width: slid(was.width, to.width),
            warmth: slid(was.warmth, to.warmth),
        };
        if was.bass <= 0.0 && self.now.bass > 0.0 {
            self.bass = Bass::new(self.rate, self.channels);
            self.shelves_for.0 = -1.0;
        }
        if was.clarity <= 0.0 && self.now.clarity > 0.0 {
            self.air = (0..self.channels).map(|_| Air::new(self.rate)).collect();
            self.shelves_for.1 = -1.0;
        }
        if was.width <= 0.0 && self.now.width > 0.0 {
            self.side_hp = Biquad::new(Coeffs::high_pass(self.rate, 250.0, FRAC_1_SQRT_2));
        }
        if was.warmth <= 0.0 && self.now.warmth > 0.0 {
            self.warmth.clear_wet();
        }
        self.set_shelves();
    }

    /// Processes interleaved `samples`, delayed by `LATENCY` frames
    /// whatever the amounts (all 0: the delay alone), so the chain's delay
    /// stays the same while they change.
    pub fn process(&mut self, samples: &mut [f32]) {
        let n = self.channels;
        // The front pair (or the one channel) gets the bass harmonics.
        let front = n.min(2);
        for block in samples.chunks_mut(n * SUB) {
            if self.now != self.target {
                self.step();
            }
            let a = self.now;
            for frame in block.chunks_exact_mut(n) {
                self.warmth.run(frame, a.warmth);
                if a.bass > 0.0 {
                    let mono = frame[..front].iter().sum::<f32>() / front as f32;
                    let harm = BASS_MIX * a.bass * self.bass.harmonics(mono);
                    // What the bass adds to each channel, and how much of
                    // it fits under the ceiling with the sound as it was.
                    let mut fits = 1f32;
                    for (ch, &x) in frame.iter().enumerate() {
                        let mut added = self.bass.shelf[ch].run(x) - x;
                        if ch < front {
                            added += harm;
                        }
                        self.bass.added[ch] = added;
                        fits = fits.min(fit(x, added, self.ceiling));
                    }
                    let give = self.bass.give(fits);
                    for (x, added) in frame.iter_mut().zip(&self.bass.added) {
                        *x += give * added;
                    }
                }
                if a.clarity > 0.0 {
                    for (x, air) in frame.iter_mut().zip(&mut self.air) {
                        *x = air.run(*x, a.clarity);
                    }
                }
                if a.width > 0.0 && n == 2 {
                    let (l, r) = (frame[0], frame[1]);
                    let mid = (l + r) / 2.0;
                    let side = (l - r) / 2.0;
                    let side = side + a.width * self.side_hp.run(side);
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

    /// `s` through one shelf per channel (the enhancer's shelves alone).
    fn shelved(s: &[f32], c: Coeffs) -> Vec<f32> {
        let mut f = [Biquad::new(c), Biquad::new(c)];
        s.iter()
            .enumerate()
            .map(|(i, &x)| f[i % 2].run(x))
            .collect()
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
    fn bass_adds_the_second_and_third_harmonics_over_the_shelf() {
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, only(|a| a.bass = 1.0));
        let original = sine(rate, 60.0, 0.3, 2.0);
        let mut s = original.clone();
        e.process(&mut s);
        let shelf = shelved(&original, Coeffs::low_shelf(rate, BASS_SHELF_HZ, BASS_SHELF_DB));
        // The fundamental is the shelf's and a little of the cube's, in
        // phase: the harmonics never take from it.
        let (got, want) = (level(&s, rate, 60.0), level(&shelf, rate, 60.0));
        assert!(got >= want && got < 1.3 * want, "{got} against {want}");
        // 2f and 3f, which the shelf alone does not make.
        for f in [120.0, 180.0] {
            assert!(level(&shelf, rate, f) < 1e-3);
            assert!(level(&s, rate, f) > 0.05, "{f} Hz: {}", level(&s, rate, f));
        }
    }

    #[test]
    fn bass_under_its_envelope_is_not_turned_over() {
        // A 100 Hz tone (a guitar's low strings; at 60 Hz the harmonics'
        // high-pass turns what it adds by 120° and hides this) loud for
        // 100 ms, then quiet for 300: the envelope holds the loud level
        // into the quiet part, as it holds music's peaks. There the bass
        // must still be the shelf's or more (T3's -3n turned it over and
        // cancelled the shelf: on a real song the octave 62-125 Hz got
        // +0.4 dB at a bass of 6 where the shelf alone gives +1.5).
        let rate = 48000;
        let period = rate as usize * 2 / 5;
        let loud = rate as usize / 10;
        let mut e = Enhancer::new(rate, 2, only(|a| a.bass = 1.0));
        let original: Vec<f32> = (0..2 * rate as usize)
            .flat_map(|i| {
                let amp = if i % period < loud { 0.3 } else { 0.05 };
                let x = amp * (2.0 * PI * 100.0 * i as f32 / rate as f32).sin();
                [x, x]
            })
            .collect();
        let mut s = original.clone();
        e.process(&mut s);
        let shelf = shelved(&original, Coeffs::low_shelf(rate, BASS_SHELF_HZ, BASS_SHELF_DB));
        // The quiet parts of the second second, from 50 ms into them.
        let quiet = |v: &[f32]| -> Vec<f32> {
            v.chunks(2)
                .enumerate()
                .skip(rate as usize)
                .filter(|(i, _)| i % period >= loud + rate as usize / 20)
                .map(|(_, f)| f[0])
                .collect()
        };
        let (got, want) = (rms(&quiet(&s)), rms(&quiet(&shelf)));
        assert!(got >= want, "{got} against {want}");
    }

    #[test]
    fn clarity_adds_harmonics_without_cutting_the_band() {
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, only(|a| a.clarity = 1.0));
        let original = sine(rate, 5000.0, 0.3, 2.0);
        let mut s = original.clone();
        e.process(&mut s);
        let shelf = shelved(&original, Coeffs::high_shelf(rate, AIR_SHELF_HZ, AIR_SHELF_DB));
        let (got, want) = (level(&s, rate, 5000.0), level(&shelf, rate, 5000.0));
        assert!((got / want - 1.0).abs() < 0.02, "{got} against {want}");
        assert!(level(&shelf, rate, 10000.0) < 1e-3);
        let second = level(&s, rate, 10000.0);
        assert!(second > 0.02, "{second}");
    }

    #[test]
    fn clarity_folds_nothing_back() {
        // 12 kHz at 44.1 kHz: its second harmonic, 24 kHz, would come back
        // at 20.1 kHz.
        let rate = 44100;
        let mut e = Enhancer::new(rate, 2, only(|a| a.clarity = 1.0));
        let mut s = sine(rate, 12000.0, 0.5, 2.0);
        e.process(&mut s);
        let folded = level(&s, rate, 20100.0);
        // Under -48 dB of the tone.
        assert!(folded < 0.5 * 0.004, "{folded}");
    }

    #[test]
    fn width_widens_and_keeps_the_middle() {
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, only(|a| a.width = 1.0));
        // Left only at 1 kHz: side and mid equal; the side doubles.
        let mut s: Vec<f32> = (0..rate as usize)
            .flat_map(|i| {
                [
                    0.5 * (2.0 * PI * 1000.0 * i as f32 / rate as f32).sin(),
                    0.0,
                ]
            })
            .collect();
        e.process(&mut s);
        let tail = &s[rate as usize / 2 * 2..];
        let mid: Vec<f32> = tail.chunks(2).map(|f| (f[0] + f[1]) / 2.0).collect();
        let side: Vec<f32> = tail.chunks(2).map(|f| (f[0] - f[1]) / 2.0).collect();
        assert!((rms(&mid) - 0.25 / 2f32.sqrt()).abs() < 0.01);
        assert!(
            (rms(&side) / rms(&mid) - 2.0).abs() < 0.1,
            "{}",
            rms(&side) / rms(&mid)
        );
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

    #[test]
    fn a_stage_turned_on_slides_in() {
        // A 60 Hz tone the bass's shelf raises by about 4 dB: turned on at
        // once, the step in level would be a click of about 0.3.
        let rate = 48000;
        let mut e = Enhancer::new(rate, 2, Amounts::default());
        let mut s = sine(rate, 60.0, 0.5, 2.0);
        let half = s.len() / 2;
        let (before, after) = s.split_at_mut(half);
        e.process(before);
        e.configure(only(|a| a.bass = 1.0));
        e.process(after);
        let jump = s
            .windows(3)
            .step_by(2)
            .map(|w| (w[2] - w[0]).abs())
            .fold(0.0f32, f32::max);
        // The tone's own steps are under 0.01 even at its end level.
        assert!(jump < 0.02, "{jump}");
        // And it got there (the last second, the slide's 50 ms in it).
        let raised = level(&s, rate, 60.0);
        assert!(raised > 0.7, "{raised}");
    }

    #[test]
    fn surround_gets_the_bass_harmonics_in_the_front_pair_only() {
        let rate = 48000;
        let all = Amounts {
            bass: 1.0,
            clarity: 1.0,
            width: 1.0,
            warmth: 1.0,
        };
        let mut e = Enhancer::new(rate, 6, all);
        let mut s: Vec<f32> = (0..rate as usize)
            .flat_map(|i| {
                let x = 0.5 * (2.0 * PI * 60.0 * i as f32 / rate as f32).sin();
                [x, x, 0.0, 0.0, 0.0, 0.0]
            })
            .collect();
        e.process(&mut s);
        assert!(s.chunks(6).all(|f| f[2..].iter().all(|&x| x == 0.0)));
        assert!(rms(&s) > 0.1);
    }

    #[test]
    fn the_bass_gives_way_to_the_ceiling() {
        // A 60 Hz tone right under the ceiling with a quiet voice (1 kHz)
        // on it: the bass has no room, so it adds nothing over the
        // ceiling, and the voice is as it was. With room, the bass is
        // raised as ever (the other tests).
        let rate = 48000;
        let ceiling = crate::audio::dsp::biquad::db_to_gain(crate::audio::dsp::CEILING_DB);
        let mut e = Enhancer::new(rate, 2, only(|a| a.bass = 1.0));
        let original: Vec<f32> = (0..2 * rate as usize)
            .flat_map(|i| {
                let t = 2.0 * PI * i as f32 / rate as f32;
                let x = 0.8 * (60.0 * t).sin() + 0.05 * (1000.0 * t).sin();
                [x, x]
            })
            .collect();
        let mut s = original.clone();
        e.process(&mut s);
        let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak <= ceiling + 1e-4, "{peak}");
        let (voice, was) = (level(&s, rate, 1000.0), level(&original, rate, 1000.0));
        assert!((voice / was - 1.0).abs() < 0.02, "{voice} against {was}");
        // The bass is not turned down under what it was.
        assert!(level(&s, rate, 60.0) >= 0.99 * level(&original, rate, 60.0));
    }

    #[test]
    fn extremes_stay_finite_and_bounded() {
        let rate = 44100;
        let all = Amounts {
            bass: 1.0,
            clarity: 1.0,
            width: 1.0,
            warmth: 1.0,
        };
        let mut e = Enhancer::new(rate, 2, all);
        // Full-scale noise, then silence, then a full-scale square.
        let mut seed = 0x2545_f491u32;
        let mut s: Vec<f32> = (0..rate as usize * 2)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as f32 / u32::MAX as f32 * 2.0 - 1.0
            })
            .collect();
        s.extend(std::iter::repeat_n(0.0, rate as usize));
        s.extend((0..rate as usize * 2).map(|i| if (i / 2 / 50) % 2 == 0 { 1.0 } else { -1.0 }));
        e.process(&mut s);
        let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(s.iter().all(|x| x.is_finite()));
        assert!(peak < 4.0, "{peak}");
    }
}
