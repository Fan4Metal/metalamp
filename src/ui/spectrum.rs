//! The spectrum analyzer on the right of the player bar, as Winamp's:
//! bars over log-spaced bands with peaks that hold and fall. The samples
//! heard now (`audio::vis::Tap`) go through a Hann window and a radix-2
//! FFT of `N` points; a band's level is the loudest bin in it, in
//! decibels of full scale, tilted up towards the high end (music is
//! pink: without the tilt the right half would lie flat), over
//! `RANGE_DB`.

use std::f32::consts::PI;
use std::time::Instant;

use egui::{Color32, Mesh, Painter, Rect, pos2};

use crate::palette;

/// The window: 2048 frames, 46 ms at 44.1 kHz; bins of 21.5 Hz.
pub const N: usize = 2048;
/// The band edges, in hertz.
const F_LO: f32 = 40.0;
const F_HI: f32 = 16000.0;
/// The floor of the bars in decibels of full scale, and the level of a
/// full bar (music is mastered a few dB under full scale).
const RANGE_DB: f32 = 60.0;
const TOP_DB: f32 = -6.0;
/// Decibels added per octave above `TILT_REF_HZ` (taken away below).
const TILT_DB_PER_OCTAVE: f32 = 3.0;
const TILT_REF_HZ: f32 = 500.0;
/// A bar falls the whole height in this many seconds.
const FALL_S: f32 = 0.45;
/// A peak stays this long, then falls with a growing speed.
const PEAK_HOLD_S: f32 = 0.5;
const PEAK_GRAVITY: f32 = 6.0;
/// Points per bar, and the gap within them.
pub const BAR_PITCH: f32 = 7.0;
const BAR_GAP: f32 = 2.0;
const PEAK_HEIGHT: f32 = 2.0;

pub struct Analyzer {
    window: Vec<f32>,
    /// The twiddles: `(cos, sin)` of `-2π k / N`.
    twiddles: Vec<(f32, f32)>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// The bars' heights, 0 to 1.
    bars: Vec<f32>,
    /// Each peak's height and how long it has been falling (negative:
    /// still holding).
    peaks: Vec<(f32, f32)>,
    last: Instant,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Analyzer {
        let window = (0..N)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / N as f32).cos())
            .collect();
        let twiddles = (0..N / 2)
            .map(|k| {
                let a = -2.0 * PI * k as f32 / N as f32;
                (a.cos(), a.sin())
            })
            .collect();
        Analyzer {
            window,
            twiddles,
            re: vec![0.0; N],
            im: vec![0.0; N],
            bars: Vec::new(),
            peaks: Vec::new(),
            last: Instant::now(),
        }
    }

    /// How many bars fit in `width` points.
    pub fn bands_for(width: f32) -> usize {
        ((width / BAR_PITCH) as usize).clamp(8, 160)
    }

    /// Moves the bars on with the samples heard now (`N` of them at
    /// `rate`; `None`: silence) for `bands` bars.
    pub fn feed(&mut self, samples: Option<&[f32]>, rate: u32, bands: usize) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        if self.bars.len() != bands {
            self.bars = vec![0.0; bands];
            self.peaks = vec![(0.0, 0.0); bands];
        }
        let levels = match samples {
            Some(s) if s.len() == N && rate > 0 => self.levels(s, rate, bands),
            _ => vec![0.0; bands],
        };
        for ((bar, (peak, age)), level) in self.bars.iter_mut().zip(&mut self.peaks).zip(levels) {
            let fallen = (*bar - dt / FALL_S).max(0.0);
            *bar = level.max(fallen);
            if *bar >= *peak {
                *peak = *bar;
                *age = -PEAK_HOLD_S;
            } else {
                *age += dt;
                if *age > 0.0 {
                    *peak = (*peak - PEAK_GRAVITY * *age * dt).max(*bar);
                }
            }
        }
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.bars.iter().any(|&b| b > 0.0) || self.peaks.iter().any(|&(p, _)| p > 0.0)
    }

    /// The bands' levels, 0 to 1, of `samples` at `rate`.
    fn levels(&mut self, samples: &[f32], rate: u32, bands: usize) -> Vec<f32> {
        for ((re, im), (s, w)) in self
            .re
            .iter_mut()
            .zip(&mut self.im)
            .zip(samples.iter().zip(&self.window))
        {
            *re = s * w;
            *im = 0.0;
        }
        fft(&mut self.re, &mut self.im, &self.twiddles);
        let bin_hz = rate as f32 / N as f32;
        let f_hi = F_HI.min(rate as f32 / 2.0 * 0.9);
        let ratio = f_hi / F_LO;
        let magnitude = |k: usize| (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt();
        (0..bands)
            .map(|b| {
                let lo = F_LO * ratio.powf(b as f32 / bands as f32);
                let hi = F_LO * ratio.powf((b + 1) as f32 / bands as f32);
                let (k0, k1) = (lo / bin_hz, hi / bin_hz);
                let mag = if k1 - k0 < 1.0 {
                    // Narrower than a bin: between the two bins around its
                    // centre, so neighbouring bands do not step.
                    let c = (k0 + k1) / 2.0;
                    let k = (c as usize).min(N / 2 - 2);
                    let t = c - k as f32;
                    magnitude(k) * (1.0 - t) + magnitude(k + 1) * t
                } else {
                    let first = (k0.ceil() as usize).min(N / 2 - 1);
                    let last = (k1.floor() as usize).clamp(first, N / 2 - 1);
                    (first..=last).map(magnitude).fold(0.0, f32::max)
                };
                // A Hann window halves a sine's peak; the FFT gives N/2 of
                // it: a full-scale sine is 0 dB.
                let amplitude = mag * 4.0 / N as f32;
                let db = 20.0 * amplitude.max(1e-9).log10();
                let tilt = TILT_DB_PER_OCTAVE * ((lo * hi).sqrt() / TILT_REF_HZ).log2();
                ((db + tilt - TOP_DB + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0)
            })
            .collect()
    }

    /// Paints the bars in `rect`.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let bands = self.bars.len();
        if bands == 0 || rect.height() <= PEAK_HEIGHT {
            return;
        }
        let pitch = rect.width() / bands as f32;
        let bar_w = (pitch - BAR_GAP).max(1.0);
        let top_room = rect.height() - PEAK_HEIGHT;
        let mut mesh = Mesh::default();
        for (b, (&h, &(peak, _))) in self.bars.iter().zip(&self.peaks).enumerate() {
            let x0 = rect.left() + pitch * b as f32 + (pitch - bar_w) / 2.0;
            let x1 = x0 + bar_w;
            if h > 0.0 {
                let y1 = rect.bottom();
                let y0 = y1 - top_room * h;
                let top = lerp(palette::spectrum_low(), palette::spectrum_high(), h);
                let i = mesh.vertices.len() as u32;
                mesh.colored_vertex(pos2(x0, y0), top);
                mesh.colored_vertex(pos2(x1, y0), top);
                mesh.colored_vertex(pos2(x1, y1), palette::spectrum_low());
                mesh.colored_vertex(pos2(x0, y1), palette::spectrum_low());
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            if peak > 0.0 {
                let y = rect.bottom() - top_room * peak - PEAK_HEIGHT;
                let i = mesh.vertices.len() as u32;
                mesh.colored_vertex(pos2(x0, y), palette::spectrum_peak());
                mesh.colored_vertex(pos2(x1, y), palette::spectrum_peak());
                mesh.colored_vertex(pos2(x1, y + PEAK_HEIGHT), palette::spectrum_peak());
                mesh.colored_vertex(pos2(x0, y + PEAK_HEIGHT), palette::spectrum_peak());
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
        }
        painter.add(mesh);
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let c = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
}

/// An in-place radix-2 FFT of `re.len()` points (a power of two), with
/// `twiddles[k]` = `(cos, sin)` of `-2π k / n` for `k < n / 2`.
fn fft(re: &mut [f32], im: &mut [f32], twiddles: &[(f32, f32)]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n && twiddles.len() == n / 2);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (wr, wi) = twiddles[k * step];
                let (a, b) = (start + k, start + k + half);
                let (xr, xi) = (re[b], im[b]);
                let (vr, vi) = (xr * wr - xi * wi, xr * wi + xi * wr);
                re[b] = re[a] - vr;
                im[b] = im[a] - vi;
                re[a] += vr;
                im[a] += vi;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_finds_a_sine() {
        let a = Analyzer::new();
        let bin = 100;
        let mut re: Vec<f32> = (0..N)
            .map(|i| 0.5 * (2.0 * PI * bin as f32 * i as f32 / N as f32).cos())
            .collect();
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im, &a.twiddles);
        let mags: Vec<f32> = (0..N / 2)
            .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt())
            .collect();
        let (peak, &m) = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        assert_eq!(peak, bin);
        // A cosine of amplitude A gives A·N/2 in its bin.
        assert!((m - 0.5 * N as f32 / 2.0).abs() < 1.0, "{m}");
        assert!(mags[bin + 10] < 0.01 * m);
    }

    #[test]
    fn a_full_scale_sine_fills_its_band_and_silence_none() {
        let mut a = Analyzer::new();
        let rate = 44100;
        let f = 1000.0;
        let sine: Vec<f32> = (0..N)
            .map(|i| (2.0 * PI * f * i as f32 / rate as f32).sin())
            .collect();
        let levels = a.levels(&sine, rate, 32);
        let (loudest, &l) = levels
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        // 1 kHz among 32 bands of 40 Hz to 16 kHz: band 17.
        let ratio: f32 = F_HI / F_LO;
        let expected = ((f / F_LO).ln() / ratio.ln() * 32.0) as usize;
        assert_eq!(loudest, expected);
        assert!(l > 0.95, "{l}");
        assert!(levels[0] < 0.2 && levels[31] < 0.2, "{levels:?}");
        let silence = a.levels(&vec![0.0; N], rate, 32);
        assert!(silence.iter().all(|&l| l == 0.0));
        a.feed(None, 0, 32);
        assert!(!a.active());
    }

    #[test]
    fn bars_fall_and_peaks_hold() {
        let mut a = Analyzer::new();
        a.feed(None, 0, 4);
        a.bars = vec![1.0; 4];
        a.peaks = vec![(1.0, -PEAK_HOLD_S); 4];
        // A frame of 0.1 s of silence: the bars fall, the peaks hold.
        a.last = Instant::now() - std::time::Duration::from_millis(100);
        a.feed(None, 0, 4);
        assert!(a.bars[0] < 0.8 && a.bars[0] > 0.7, "{}", a.bars[0]);
        assert_eq!(a.peaks[0].0, 1.0);
        assert!(a.active());
    }
}
