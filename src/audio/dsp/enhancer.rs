//! The enhancer: a multiband automatic gain control fitted on recordings
//! of another player's enhancer, with its controls. Six detectors hear what their frequency regions hold (what
//! each hears and where it acts, as octave probes showed it); three stages
//! follow one another, all on the same detectors:
//!
//! - the core: per detector a gain `offset - slope x level`, between a
//!   floor and a most, over a base curve per band; the bass and treble
//!   types move the most and the base;
//! - fidelity and ambience: upward stages, a gain from 0 up to a most that
//!   grows with the control, each detector hearing the sound with the
//!   stages before it applied (so two raise less than the sum of each);
//! - the levels: a three-band equalizer after the rest.
//!
//! Drive multiplies the whole gain in dB (measured: slopes, maxima and
//! floors scale with it, the thresholds stay). The gains are real, so a
//! channel's phase is kept; the stereo channels share them. The dry sound
//! is delayed by the window and the gain's difference overlap-added onto
//! it, so with the enhancer off the output is the input delayed, exactly.
//! The fitted numbers are in `enhancer_law.rs`, generated.

use serde::{Deserialize, Serialize};

use super::enhancer_law as law;
use super::fft::fft;
use std::f32::consts::TAU;

const K: usize = 6;
const NB: usize = law::CENTRES.len();

/// What a control adds to its stage at a value of it (`at`, -1 to 1 for
/// the types, 0 to 1 for fidelity and ambience); between knots linearly.
pub struct Knot {
    pub at: f32,
    pub most: [f32; K],
    pub base: [f32; NB],
}

/// The enhancer's controls (the settings' `dsp.enhancement`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Enhancement {
    /// Percent, 0 to 140: how much of everything below.
    pub drive: u8,
    /// -100 (deep) to 100 (punchy).
    pub bass_type: i8,
    /// -100 (sharp) to 100 (smooth).
    pub treble_type: i8,
    /// Decibels, -6 to 6.
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    /// 0 to 10.
    pub fidelity: f32,
    pub ambience: f32,
}

impl Enhancement {
    /// The settings recorded (Drive 135 %, punch 30, smooth 50, bass +2,
    /// treble +1.8, fidelity 5.5, ambience 7).
    pub const STANDARD: Enhancement = Enhancement {
        drive: 135,
        bass_type: 30,
        treble_type: 50,
        bass: 2.0,
        mid: 0.0,
        treble: 1.8,
        fidelity: 5.5,
        ambience: 7.0,
    };
    pub const MOST_DRIVE: u8 = 140;
    pub const LEVEL_DB: f32 = 6.0;

    /// Within the controls' ranges (a hand-edited settings file).
    pub fn clamped(self) -> Enhancement {
        let level = |v: f32| {
            if v.is_finite() {
                v.clamp(-Self::LEVEL_DB, Self::LEVEL_DB)
            } else {
                0.0
            }
        };
        let amount = |v: f32| if v.is_finite() { v.clamp(0.0, 10.0) } else { 0.0 };
        Enhancement {
            drive: self.drive.min(Self::MOST_DRIVE),
            bass_type: self.bass_type.clamp(-100, 100),
            treble_type: self.treble_type.clamp(-100, 100),
            bass: level(self.bass),
            mid: level(self.mid),
            treble: level(self.treble),
            fidelity: amount(self.fidelity),
            ambience: amount(self.ambience),
        }
    }
}

impl Default for Enhancement {
    fn default() -> Self {
        Self::STANDARD
    }
}

/// The detectors' frequency regions: lower and upper edges in Hz, and
/// what each of the six detectors hears of each region per unit of mean
/// square signal.
const EDGES: [f32; 21] = [
    0., 40., 56.57, 80., 113.14, 160., 226.27, 320., 452.55, 640., 905.1, 1280., 1810.19, 2560.,
    3620.39, 5120., 7240.77, 10240., 14481.55, 20480., 22050.,
];
const SENSITIVITY: [[f32; K]; 20] = [
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
/// Hz and which detectors a frequency follows (an index between two,
/// their gains blended in amplitude).
const INDEX: [[f32; 2]; 38] = [
    [0., 0.],
    [43., 0.],
    [86., 0.],
    [108., 0.08],
    [129., 0.31],
    [151., 0.76],
    [172., 0.95],
    [194., 1.],
    [215., 1.14],
    [237., 1.45],
    [258., 1.79],
    [280., 1.98],
    [350., 1.99],
    [390., 2.],
    [495., 2.],
    [580., 2.],
    [750., 2.],
    [850., 2.],
    [1000., 2.26],
    [1200., 2.6],
    [1420., 2.91],
    [1660., 3.],
    [1990., 3.],
    [2300., 3.],
    [3150., 3.],
    [3950., 3.],
    [4800., 3.08],
    [5600., 3.23],
    [6300., 3.32],
    [7300., 3.43],
    [8300., 3.51],
    [10000., 3.62],
    [12500., 4.],
    [13900., 4.],
    [15000., 4.45],
    [16000., 5.],
    [19000., 5.],
    [20000., 5.],
];
/// The output against the recorded player's: its peaks went up to 4 dB
/// over full scale (its volume came after it), the final limiter here
/// holds them at -1 dBTP; with 2 dB less the limiter takes about as much
/// off a hit as those peaks passed over full scale.
const OUTPUT_TRIM_DB: f32 = -2.0;
/// The share of the way a changed control's curve and drive go per hop
/// (a quarter window): about 25 ms at 44.1 and 48 kHz.
const GLIDE: f32 = 0.25;

/// Piecewise-linear interpolation of a value given at increasing `xs`.
fn interp(x: f32, xs: &[f32], ys: &[f32]) -> f32 {
    let i = xs.partition_point(|&k| k < x).clamp(1, xs.len() - 1);
    let t = ((x - xs[i - 1]) / (xs[i] - xs[i - 1])).clamp(0., 1.);
    ys[i - 1] + t * (ys[i] - ys[i - 1])
}

/// A knot table's most and base at `x`.
fn knotted(knots: &[Knot; 5], x: f32) -> ([f32; K], [f32; NB]) {
    let x = x.clamp(knots[0].at, knots[4].at);
    let i = knots.partition_point(|k| k.at < x).clamp(1, 4);
    let (a, b) = (&knots[i - 1], &knots[i]);
    let t = ((x - a.at) / (b.at - a.at)).clamp(0., 1.);
    (
        std::array::from_fn(|k| a.most[k] + t * (b.most[k] - a.most[k])),
        std::array::from_fn(|n| a.base[n] + t * (b.base[n] - a.base[n])),
    )
}

/// One stage's settled parameters, and its detectors' state.
#[derive(Clone, Copy, Default)]
struct Stage {
    on: bool,
    offset: [f32; K],
    slope: [f32; K],
    floor: [f32; K],
    most: [f32; K],
    /// The base of each detector's own bands (dB), so the next stage's
    /// detector hears that detector's region with this stage's gain.
    own_base: [f32; K],
    env: [f32; K],
    gain: [f32; K],
}

/// What the controls make of the stages and the bands.
struct Law {
    drive: f32,
    stages: [Stage; 3],
    /// Per band (dB): the stages' base times drive, the levels after it.
    band_db: [f32; NB],
}

fn law_of(e: &Enhancement) -> Law {
    let e = e.clamped();
    let d = f32::from(e.drive) / 100.;
    let (bt_most, bt_base) = knotted(&law::BASS_TYPE, f32::from(e.bass_type) / 100.);
    let (tt_most, tt_base) = knotted(&law::TREBLE_TYPE, f32::from(e.treble_type) / 100.);
    let core_base: [f32; NB] = std::array::from_fn(|n| law::CORE_BASE[n] + bt_base[n] + tt_base[n]);
    let mut stages = [Stage::default(); 3];
    stages[0] = Stage {
        on: true,
        offset: law::CORE_OFFSET,
        slope: law::CORE_SLOPE,
        floor: law::CORE_FLOOR,
        most: std::array::from_fn(|k| law::CORE_MOST[k] + bt_most[k] + tt_most[k]),
        own_base: own(&core_base),
        ..Stage::default()
    };
    let mut band_db = core_base;
    for (i, (amount, knots, offset, slope)) in [
        (e.fidelity, &law::FIDELITY, law::FIDELITY_OFFSET, law::FIDELITY_SLOPE),
        (e.ambience, &law::AMBIENCE, law::AMBIENCE_OFFSET, law::AMBIENCE_SLOPE),
    ]
    .into_iter()
    .enumerate()
    {
        let x = amount / 10.;
        let (most, base) = knotted(knots, x);
        stages[i + 1] = Stage {
            on: x > 0.,
            offset,
            slope,
            floor: [0.; K],
            most: most.map(|m| m.max(0.)),
            own_base: own(&base),
            ..Stage::default()
        };
        if x > 0. {
            for (b, v) in band_db.iter_mut().zip(base) {
                *b += v;
            }
        }
    }
    for (n, b) in band_db.iter_mut().enumerate() {
        *b = d * *b
            + e.bass * law::LEVEL_BASS[n]
            + e.mid * law::LEVEL_MID[n]
            + e.treble * law::LEVEL_TREBLE[n];
    }
    Law {
        drive: d,
        stages,
        band_db,
    }
}

fn own(base: &[f32; NB]) -> [f32; K] {
    std::array::from_fn(|k| (0..NB).map(|n| law::SHARE[n][k] * base[n]).sum())
}

struct Bin {
    detector: [f32; K],
    band: usize,
    blend: f32,
    /// Where the bin stands among the third-octave bands.
    place: (usize, f32),
    audible: f32,
}

impl Bin {
    fn new(hz: f32, step: f32, scale: f32) -> Self {
        let index = interp(hz, &INDEX.map(|r| r[0]), &INDEX.map(|r| r[1]));
        let band = (index as usize).min(K - 2);
        let mut detector = [0.; K];
        for (edges, weights) in EDGES.windows(2).zip(SENSITIVITY) {
            let overlap =
                ((hz + step * 0.5).min(edges[1]) - (hz - step * 0.5).max(edges[0])).clamp(0., step);
            for (d, w) in detector.iter_mut().zip(weights) {
                *d += w * (overlap / step) * scale;
            }
        }
        let lf = hz.max(law::CENTRES[0]).ln();
        let centres = law::CENTRES.map(f32::ln);
        let i = centres.partition_point(|&c| c < lf).clamp(1, NB - 1);
        let t = ((lf - centres[i - 1]) / (centres[i] - centres[i - 1])).clamp(0., 1.);
        Self {
            detector,
            band,
            blend: index - band as f32,
            place: (i, t),
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

pub(crate) struct Enhancer {
    enabled: bool,
    tail: usize,
    pos: usize,
    channels: Vec<Channel>,
    window: Vec<f32>,
    twiddles: Vec<(f32, f32)>,
    bins: Vec<Bin>,
    bin_gain: Vec<f32>,
    /// The law asked for; the curve and the drive glide towards it.
    law: Law,
    band_now: [f32; NB],
    drive_now: f32,
    average: f32,
    attack: f32,
    release: f32,
}

impl Enhancer {
    pub(crate) fn new(rate: u32, channels: usize, enabled: bool, e: &Enhancement) -> Self {
        let rate = rate.max(1) as f32;
        let n = 1usize << (10. + (rate / 44100.).log2()).round().clamp(7., 13.) as u32;
        let angle = TAU / n as f32;
        let window: Vec<f32> = (0..n)
            .map(|i| 0.5 - 0.5 * (i as f32 * angle).cos())
            .collect();
        let scale = 2.
            / (n as f32
                * window.iter().map(|w| w * w).sum::<f32>()
                * (channels - usize::from(channels >= 6)).max(1) as f32);
        let step = rate / n as f32;
        let smooth = |seconds: f32| 1. - (-(n as f32 / 4.) / (rate * seconds)).exp();
        let law = law_of(e);
        let mut fx = Self {
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
            band_now: law.band_db,
            drive_now: law.drive,
            law,
            average: smooth(law::AVERAGE_S),
            attack: smooth(law::ATTACK_S),
            release: smooth(law::RELEASE_S),
        };
        fx.start();
        fx
    }

    /// As after silence: the levels heard nothing, the gains at their
    /// most (the recorded player starts so after a seek; started as after
    /// a reference level, the first hit of a passage was 5-11 dB short).
    fn start(&mut self) {
        for s in &mut self.law.stages {
            s.env = [0.; K];
            s.gain = s.most;
        }
    }

    pub(crate) fn configure(&mut self, enabled: bool, e: &Enhancement) {
        let mut law = law_of(e);
        // The detectors' state carries over; the curve and drive glide.
        for (new, old) in law.stages.iter_mut().zip(&self.law.stages) {
            new.env = old.env;
            new.gain = if old.on { old.gain } else { new.most };
        }
        self.law = law;
        if enabled != self.enabled {
            if enabled {
                self.start();
                self.band_now = self.law.band_db;
                self.drive_now = self.law.drive;
            } else {
                self.tail = self.latency_frames();
            }
            self.enabled = enabled;
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.enabled || self.tail > 0
    }

    pub(crate) fn latency_frames(&self) -> usize {
        self.window.len()
    }

    pub(crate) fn process(&mut self, samples: &mut [f32]) {
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

    /// The detectors' gains now, per stage (dB, before the drive), for
    /// tests and measuring.
    #[cfg(test)]
    fn gains(&self) -> [[f32; K]; 3] {
        self.law.stages.map(|s| s.gain)
    }

    fn synthesize(&mut self) {
        let n = self.window.len();
        let mask = n - 1;
        let mut power = [0.; K];
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
        self.drive_now += GLIDE * (self.law.drive - self.drive_now);
        for (now, &to) in self.band_now.iter_mut().zip(&self.law.band_db) {
            *now += GLIDE * (to - *now);
        }
        let d = self.drive_now;
        // Each stage hears the sound with the stages before it applied.
        let mut heard = power;
        let mut linear = [[1f32; K]; 3];
        for (s, lin) in self.law.stages.iter_mut().zip(&mut linear) {
            if !s.on {
                continue;
            }
            for k in 0..K {
                s.env[k] += self.average * (heard[k] - s.env[k]);
                let level = 10. * s.env[k].max(1e-12).log10();
                let target = (s.offset[k] - s.slope[k] * level).max(s.floor[k]).min(s.most[k]);
                let a = if target < s.gain[k] {
                    self.attack
                } else {
                    self.release
                };
                s.gain[k] += a * (target - s.gain[k]);
                lin[k] = 10f32.powf(s.gain[k] / 20.);
                heard[k] *= 10f32.powf(d * (s.gain[k] + s.own_base[k]) / 10.);
            }
        }
        let on = self.law.stages.map(|s| s.on);
        for (g, b) in self.bin_gain.iter_mut().zip(&self.bins) {
            let mut product = 1.;
            for (lin, _) in linear.iter().zip(on).filter(|(_, on)| *on) {
                product *= lin[b.band] + b.blend * (lin[b.band + 1] - lin[b.band]);
            }
            let (i, t) = b.place;
            let base = self.band_now[i - 1] + t * (self.band_now[i] - self.band_now[i - 1]);
            let db = base + OUTPUT_TRIM_DB + d * 20. * product.max(1e-9).log10();
            *g = (10f32.powf(db / 20.) - 1.) * b.audible;
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

    /// Level in dB of a sine of `hz` in the steady part of `out`.
    fn tone_db(out: &[f32], rate: u32, hz: f64) -> f64 {
        let (mut re, mut im) = (0f64, 0f64);
        for (i, &x) in out.iter().enumerate() {
            let p = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
            re += f64::from(x) * p.cos();
            im += f64::from(x) * p.sin();
        }
        20. * (2. * re.hypot(im) / out.len() as f64).log10()
    }

    fn sine(rate: u32, hz: f32, level_db: f32, seconds: f32) -> Vec<f32> {
        let a = 10f32.powf(level_db / 20.);
        (0..(rate as f32 * seconds) as usize)
            .map(|i| a * (TAU * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    fn gain_db(e: &Enhancement, hz: f32, level_db: f32) -> f64 {
        let rate = 48000;
        let mut fx = Enhancer::new(rate, 1, true, e);
        let mut s = sine(rate, hz, level_db, 3.);
        fx.process(&mut s);
        tone_db(&s[2 * rate as usize..], rate, f64::from(hz)) - f64::from(level_db)
    }

    #[test]
    fn quiet_sound_is_raised_more_than_loud_sound() {
        // The recorded player: a 1 kHz tone at -36 dBFS got about 20 dB
        // more than one at -12 (with the standard settings).
        let e = Enhancement::STANDARD;
        let quiet = gain_db(&e, 1000., -36.);
        let loud = gain_db(&e, 1000., -12.);
        assert!(quiet - loud > 10., "{quiet} {loud}");
        assert!(quiet > 10. && quiet < 35., "{quiet}");
    }

    #[test]
    fn drive_zero_leaves_only_the_levels() {
        // At drive 0 the recorded player only applied the levels: the bass
        // +2 dB under 125 Hz, the middle untouched.
        let e = Enhancement {
            drive: 0,
            ..Enhancement::STANDARD
        };
        let bass = gain_db(&e, 60., -24.) - f64::from(OUTPUT_TRIM_DB);
        let mid = gain_db(&e, 500., -24.) - f64::from(OUTPUT_TRIM_DB);
        assert!((bass - 2.).abs() < 0.3, "{bass}");
        assert!(mid.abs() < 0.3, "{mid}");
    }

    #[test]
    fn drive_scales_the_gain_of_quiet_sound() {
        // Quiet, every detector is at its most: the gain is drive times
        // one curve (louder, the stages compress what drive added, as the
        // recorded player's did).
        let at = |drive| {
            gain_db(
                &Enhancement {
                    drive,
                    bass: 0.,
                    mid: 0.,
                    treble: 0.,
                    ..Enhancement::STANDARD
                },
                1000.,
                -50.,
            ) - f64::from(OUTPUT_TRIM_DB)
        };
        let (half, full) = (at(70), at(140));
        assert!(half > 3. && (full / half - 2.).abs() < 0.1, "{half} {full}");
    }

    #[test]
    fn fidelity_and_ambience_raise_quiet_sound() {
        let none = Enhancement {
            fidelity: 0.,
            ambience: 0.,
            ..Enhancement::STANDARD
        };
        let more = Enhancement {
            fidelity: 10.,
            ambience: 10.,
            ..Enhancement::STANDARD
        };
        let (a, b) = (gain_db(&none, 1000., -40.), gain_db(&more, 1000., -40.));
        assert!(b - a > 8., "{a} {b}");
    }

    #[test]
    fn the_stages_start_at_their_most() {
        let fx = Enhancer::new(48000, 2, true, &Enhancement::STANDARD);
        for (stage, gains) in fx.law.stages.iter().zip(fx.gains()) {
            assert_eq!(stage.most, gains);
        }
    }

    #[test]
    fn linked_gain_preserves_polarity() {
        let rate = 44100;
        let mut fx = Enhancer::new(rate, 2, true, &Enhancement::STANDARD);
        let mut audio: Vec<_> = sine(rate, 185., -26., 1.)
            .into_iter()
            .flat_map(|x| [x, -0.25 * x])
            .collect();
        fx.process(&mut audio);
        assert!(
            audio
                .as_chunks::<2>()
                .0
                .iter()
                .all(|s| (s[0] + 4. * s[1]).abs() < 1e-6)
        );
    }

    #[test]
    fn settings_out_of_range_are_held_in() {
        let wild = Enhancement {
            drive: 250,
            bass_type: -128,
            treble_type: 127,
            bass: f32::NAN,
            mid: -40.,
            treble: 9.,
            fidelity: -1.,
            ambience: f32::INFINITY,
        }
        .clamped();
        assert_eq!(wild.drive, 140);
        assert_eq!((wild.bass_type, wild.treble_type), (-100, 100));
        assert_eq!((wild.bass, wild.mid, wild.treble), (0., -6., 6.));
        assert_eq!((wild.fidelity, wild.ambience), (0., 0.));
    }

    #[test]
    fn switching_across_blocks_drains_to_an_exact_wire() {
        for rate in [22050, 44100, 48000, 96000, 192000] {
            let e = Enhancement::STANDARD;
            let mut whole = Enhancer::new(rate, 1, false, &e);
            let mut split = Enhancer::new(rate, 1, false, &e);
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
                whole.configure(on, &e);
                split.configure(on, &e);
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

    #[test]
    fn a_changed_control_glides() {
        // Drive from 0 to 135 in one step: the output's level moves over
        // tens of milliseconds, not at once (no click).
        let rate = 48000;
        let mut fx = Enhancer::new(
            rate,
            1,
            true,
            &Enhancement {
                drive: 0,
                ..Enhancement::STANDARD
            },
        );
        let mut s = sine(rate, 1000., -30., 1.);
        fx.process(&mut s);
        fx.configure(true, &Enhancement::STANDARD);
        let mut s = sine(rate, 1000., -30., 0.2);
        fx.process(&mut s);
        let block = rate as usize / 100;
        let rms = |b: &[f32]| (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
        let levels: Vec<f32> = s.chunks(block).map(|b| 20. * rms(b).log10()).collect();
        let (first, last) = (levels[0], levels[levels.len() - 1]);
        let most = levels.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        let reached = levels
            .iter()
            .position(|&l| (l - first).abs() >= 0.9 * (last - first).abs())
            .unwrap_or(0);
        assert!(most < 0.6 * (last - first).abs() && reached >= 3, "{levels:?}");
    }
}
