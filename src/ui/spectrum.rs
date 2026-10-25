//! The spectrum analyzer on the right of the player bar:
//! bars over log-spaced bands with peaks that hold and fall. The samples
//! heard now (`audio::vis::Tap`) go through a Hann window and a radix-2
//! FFT of `N` points; a band's level is the loudest bin in it, in
//! decibels of full scale, tilted up towards the high end (music is
//! pink: without the tilt the right half would lie flat), over
//! `RANGE_DB`.

use std::f32::consts::{PI, TAU};
use std::time::Instant;

use egui::epaint::PathStroke;
use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, TextureHandle, pos2};

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
/// The LED spectrum's bars and their segments: a segment and the gap
/// under it, an unlit one's share of its colour.
pub const LED_PITCH: f32 = 9.0;
const LED_SEGMENT: f32 = 3.0;
const LED_GAP: f32 = 1.0;
const LED_UNLIT: f32 = 0.09;
/// Points per band of the curve.
pub const CURVE_PITCH: f32 = 4.0;
/// The radial spectrum: the bars' start as a share of the radius, their
/// share of their angle, and how much the bass swells the cover.
const RADIAL_INNER: f32 = 0.4;
const RADIAL_FILL: f32 = 0.6;
const RADIAL_SWELL: f32 = 0.1;

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

    /// How many bars of `pitch` points fit in `width` points.
    pub fn bands_for(width: f32, pitch: f32) -> usize {
        ((width / pitch) as usize).clamp(8, 256)
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
    pub fn levels(&mut self, samples: &[f32], rate: u32, bands: usize) -> Vec<f32> {
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
        let colours = palette::visual();
        let mut mesh = Mesh::default();
        for (b, (&h, &(peak, _))) in self.bars.iter().zip(&self.peaks).enumerate() {
            let x0 = rect.left() + pitch * b as f32 + (pitch - bar_w) / 2.0;
            let x1 = x0 + bar_w;
            // The band's place among them, for a rainbow.
            let place = b as f32 / (bands - 1).max(1) as f32;
            if h > 0.0 {
                let y1 = rect.bottom();
                let y0 = y1 - top_room * h;
                let (top, foot) = (colours.at(h, place), colours.at(0.0, place));
                let i = mesh.vertices.len() as u32;
                mesh.colored_vertex(pos2(x0, y0), top);
                mesh.colored_vertex(pos2(x1, y0), top);
                mesh.colored_vertex(pos2(x1, y1), foot);
                mesh.colored_vertex(pos2(x0, y1), foot);
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            if peak > 0.0 {
                let y = rect.bottom() - top_room * peak - PEAK_HEIGHT;
                let colour = colours.peak(place);
                let i = mesh.vertices.len() as u32;
                mesh.colored_vertex(pos2(x0, y), colour);
                mesh.colored_vertex(pos2(x1, y), colour);
                mesh.colored_vertex(pos2(x1, y + PEAK_HEIGHT), colour);
                mesh.colored_vertex(pos2(x0, y + PEAK_HEIGHT), colour);
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
        }
        painter.add(mesh);
    }

    /// Paints the bars as columns of segments in `rect`: lit up to the
    /// bar, each in the colour of its height, the peak's segment lit in the
    /// peaks' colour, the rest faint.
    pub fn paint_led(&self, painter: &Painter, rect: Rect) {
        let bands = self.bars.len();
        if bands == 0 || rect.height() < LED_SEGMENT {
            return;
        }
        let pitch = rect.width() / bands as f32;
        let bar_w = (pitch - BAR_GAP).max(1.0);
        let count = ((rect.height() + LED_GAP) / (LED_SEGMENT + LED_GAP)).max(1.0) as usize;
        // The segments stretched to fill the height.
        let step = (rect.height() + LED_GAP) / count as f32;
        let colours = palette::visual();
        let mut mesh = Mesh::default();
        for (b, (&h, &(peak, _))) in self.bars.iter().zip(&self.peaks).enumerate() {
            let x0 = rect.left() + pitch * b as f32 + (pitch - bar_w) / 2.0;
            let place = b as f32 / (bands - 1).max(1) as f32;
            let lit = (h * count as f32).round() as usize;
            let peak_at =
                (peak > 0.0).then(|| ((peak * count as f32).ceil() as usize).clamp(1, count) - 1);
            for i in 0..count {
                let y1 = rect.bottom() - step * i as f32;
                let colour = if i < lit {
                    colours.at((i as f32 + 0.5) / count as f32, place)
                } else if peak_at == Some(i) {
                    colours.peak(place)
                } else {
                    colours
                        .at((i as f32 + 0.5) / count as f32, place)
                        .gamma_multiply(LED_UNLIT)
                };
                mesh.add_colored_rect(
                    Rect::from_min_max(pos2(x0, y1 - step + LED_GAP), pos2(x0 + bar_w, y1)),
                    colour,
                );
            }
        }
        painter.add(mesh);
    }

    /// Paints the bars up and down from the middle line of `rect`, each
    /// half as the plain spectrum's bar, the peaks on both sides.
    pub fn paint_mirror(&self, painter: &Painter, rect: Rect) {
        let bands = self.bars.len();
        if bands == 0 || rect.height() <= 2.0 * PEAK_HEIGHT + 2.0 {
            return;
        }
        let pitch = rect.width() / bands as f32;
        let bar_w = (pitch - BAR_GAP).max(1.0);
        let cy = rect.center().y;
        let room = rect.height() / 2.0 - PEAK_HEIGHT - 0.5;
        let colours = palette::visual();
        let mut mesh = Mesh::default();
        for (b, (&h, &(peak, _))) in self.bars.iter().zip(&self.peaks).enumerate() {
            let x0 = rect.left() + pitch * b as f32 + (pitch - bar_w) / 2.0;
            let x1 = x0 + bar_w;
            let place = b as f32 / (bands - 1).max(1) as f32;
            for side in [-1.0, 1.0] {
                // From the middle (half a point off it) outwards.
                let foot = cy + side * 0.5;
                if h > 0.0 {
                    let tip = foot + side * room * h;
                    let (top, base) = (colours.at(h, place), colours.at(0.0, place));
                    quad(
                        &mut mesh,
                        [pos2(x0, tip), pos2(x1, tip), pos2(x1, foot), pos2(x0, foot)],
                        [top, top, base, base],
                    );
                }
                if peak > 0.0 {
                    let y = foot + side * room * peak;
                    mesh.add_colored_rect(
                        Rect::from_two_pos(pos2(x0, y), pos2(x1, y + side * PEAK_HEIGHT)),
                        colours.peak(place),
                    );
                }
            }
        }
        painter.add(mesh);
    }

    /// Paints the bars' tops as a smooth line over a fill fading to the
    /// floor's colour, and the peaks as a faint line over it.
    pub fn paint_curve(&self, painter: &Painter, rect: Rect) {
        let bands = self.bars.len();
        if bands < 2 || rect.height() <= 2.0 {
            return;
        }
        let pitch = rect.width() / bands as f32;
        let room = rect.height() - 1.0;
        let at = |b: usize, h: f32| {
            pos2(
                rect.left() + pitch * (b as f32 + 0.5),
                rect.bottom() - room * h,
            )
        };
        let mut tops: Vec<Pos2> = self
            .bars
            .iter()
            .enumerate()
            .map(|(b, &h)| at(b, h))
            .collect();
        // Out to the edges, level with the outer bands.
        tops.insert(0, pos2(rect.left(), tops[0].y));
        tops.push(pos2(rect.right(), tops[tops.len() - 1].y));
        let line = smooth(&tops, rect.top(), rect.bottom());
        let colours = palette::visual();
        let place = move |p: Pos2| ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let level = move |p: Pos2| ((rect.bottom() - p.y) / room).clamp(0.0, 1.0);
        let mut mesh = Mesh::default();
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (fa, fb) = (pos2(a.x, rect.bottom()), pos2(b.x, rect.bottom()));
            quad(
                &mut mesh,
                [a, b, fb, fa],
                [
                    colours.at(level(a), place(a)).gamma_multiply(0.75),
                    colours.at(level(b), place(b)).gamma_multiply(0.75),
                    colours.at(0.0, place(b)).gamma_multiply(0.35),
                    colours.at(0.0, place(a)).gamma_multiply(0.35),
                ],
            );
        }
        painter.add(mesh);
        let peaks: Vec<Pos2> = self
            .peaks
            .iter()
            .enumerate()
            .filter(|(_, (p, _))| *p > 0.0)
            .map(|(b, &(p, _))| at(b, p) - egui::vec2(0.0, PEAK_HEIGHT))
            .collect();
        if peaks.len() > 1 {
            painter.add(Shape::line(
                peaks,
                PathStroke::new_uv(1.0, move |_, p| colours.peak(place(p)).gamma_multiply(0.6)),
            ));
        }
        painter.add(Shape::line(
            line,
            PathStroke::new_uv(1.6, move |_, p| colours.at(level(p).max(0.6), place(p))),
        ));
    }

    /// How loud the bass is now, 0 to 1: the lowest sixth of the bars.
    fn bass(&self) -> f32 {
        let n = (self.bars.len() / 6).max(1).min(self.bars.len());
        if n == 0 {
            return 0.0;
        }
        self.bars[..n].iter().sum::<f32>() / n as f32
    }

    /// Paints the bars around a circle in the middle of `rect`, the bass
    /// at the top and the treble down both sides, `cover` (else a disc of
    /// the colours) in it swelling with the bass, over a glow of it.
    pub fn paint_radial(&self, painter: &Painter, rect: Rect, cover: Option<&TextureHandle>) {
        let bands = self.bars.len();
        if bands == 0 {
            return;
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let centre = rect.center();
        let radius = (rect.height().min(rect.width()) / 2.0 - 1.0).max(4.0);
        let inner = radius * RADIAL_INNER;
        let room = (radius - inner - PEAK_HEIGHT - 1.0).max(1.0);
        let colours = palette::visual();
        let bass = self.bass();
        let polar = |angle: f32, r: f32| centre + egui::vec2(angle.cos(), angle.sin()) * r;

        // The glow, brighter with the bass: from behind the circle out to
        // the display's ends, as the logo's.
        let glow = colours.at(1.0, 0.0).gamma_multiply(0.04 + 0.16 * bass);
        let clear = Color32::TRANSPARENT;
        let mut mesh = Mesh::default();
        let (left, right) = (rect.left(), rect.right());
        for (x0, x1, c0, c1) in [
            (left, centre.x, clear, glow),
            (centre.x, right, glow, clear),
        ] {
            quad(
                &mut mesh,
                [
                    pos2(x0, rect.top()),
                    pos2(x1, rect.top()),
                    pos2(x1, rect.bottom()),
                    pos2(x0, rect.bottom()),
                ],
                [c0, c1, c1, c0],
            );
        }
        painter.add(mesh);
        const STEPS: u32 = 48;

        let pitch = PI / bands as f32;
        let half = pitch * RADIAL_FILL / 2.0;
        let mut mesh = Mesh::default();
        for (b, (&h, &(peak, _))) in self.bars.iter().zip(&self.peaks).enumerate() {
            let place = b as f32 / (bands - 1).max(1) as f32;
            let along = (b as f32 + 0.5) * pitch;
            for side in [1.0, -1.0] {
                let angle = -PI / 2.0 + side * along;
                let (a0, a1) = (angle - half, angle + half);
                if h > 0.0 {
                    let tip = inner + room * h;
                    let (top, base) = (colours.at(h, place), colours.at(0.0, place));
                    quad(
                        &mut mesh,
                        [
                            polar(a0, tip),
                            polar(a1, tip),
                            polar(a1, inner),
                            polar(a0, inner),
                        ],
                        [top, top, base, base],
                    );
                }
                if peak > 0.0 {
                    let (r0, r1) = (
                        inner + room * peak + 1.0,
                        inner + room * peak + 1.0 + PEAK_HEIGHT,
                    );
                    let c = colours.peak(place);
                    quad(
                        &mut mesh,
                        [polar(a0, r1), polar(a1, r1), polar(a1, r0), polar(a0, r0)],
                        [c; 4],
                    );
                }
            }
        }
        painter.add(mesh);

        let r = ((inner - 3.0) * (1.0 - RADIAL_SWELL + RADIAL_SWELL * bass)).max(1.0);
        match cover {
            Some(texture) => {
                // The middle square of a cover not square.
                let [w, h] = texture.size().map(|s| s.max(1) as f32);
                let (su, sv) = (h.min(w) / w, h.min(w) / h);
                let mut mesh = Mesh::with_texture(texture.id());
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: centre,
                    uv: pos2(0.5, 0.5),
                    color: Color32::WHITE,
                });
                for i in 0..=STEPS {
                    let a = i as f32 / STEPS as f32 * TAU;
                    let (cos, sin) = (a.cos(), a.sin());
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: centre + egui::vec2(cos, sin) * r,
                        uv: pos2(0.5 + 0.5 * cos * su, 0.5 + 0.5 * sin * sv),
                        color: Color32::WHITE,
                    });
                    if i > 0 {
                        mesh.add_triangle(0, i, i + 1);
                    }
                }
                painter.add(mesh);
            }
            None => {
                painter.circle_filled(centre, r, colours.at(0.0, 0.0).gamma_multiply(0.35));
            }
        }
        painter.circle_stroke(
            centre,
            r + 0.5,
            egui::Stroke::new(1.5, colours.at(bass, 0.0)),
        );
    }
}

/// How many bars each half of the radial spectrum has in a display of
/// `size`: about 5 points of its outer edge each.
pub fn radial_bands(size: egui::Vec2) -> usize {
    let radius = size.x.min(size.y) / 2.0;
    ((PI * radius / 5.0) as usize).clamp(12, 64)
}

/// A quad of `corners` (clockwise) in `colours`.
fn quad(mesh: &mut Mesh, corners: [Pos2; 4], colours: [Color32; 4]) {
    let i = mesh.vertices.len() as u32;
    for (p, c) in corners.into_iter().zip(colours) {
        mesh.colored_vertex(p, c);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

/// A Catmull-Rom spline through `points` (left to right), four points a
/// span, kept between `top` and `bottom`.
fn smooth(points: &[Pos2], top: f32, bottom: f32) -> Vec<Pos2> {
    const STEPS: usize = 4;
    let n = points.len();
    let mut out = Vec::with_capacity(n * STEPS);
    for i in 0..n.saturating_sub(1) {
        let p0 = points[i.saturating_sub(1)];
        let (p1, p2) = (points[i], points[i + 1]);
        let p3 = points[(i + 2).min(n - 1)];
        for s in 0..STEPS {
            let t = s as f32 / STEPS as f32;
            let (t2, t3) = (t * t, t * t * t);
            let y = 0.5
                * (2.0 * p1.y
                    + (p2.y - p0.y) * t
                    + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2
                    + (3.0 * p1.y - p0.y - 3.0 * p2.y + p3.y) * t3);
            out.push(pos2(p1.x + (p2.x - p1.x) * t, y.clamp(top, bottom)));
        }
    }
    if let Some(&last) = points.last() {
        out.push(last);
    }
    out
}

/// An in-place radix-2 FFT of `re.len()` points (a power of two), with
/// `twiddles[k]` = `(cos, sin)` of `-2π k / n` for `k < n / 2`.
pub(super) fn fft(re: &mut [f32], im: &mut [f32], twiddles: &[(f32, f32)]) {
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
