//! The waveform (`Visual::Waveform`) as DJ software draws a track (the
//! user found the plain one dull): the sound rolling in from the right in
//! columns of `COLUMN_S`, mirrored about the middle line, in three layers
//! of their own colours, the bass at the back, the middle and the treble
//! in front, each as high as its band's peak in the column (the bands
//! split by two-pole filters as the samples come), over a faint outline of
//! the whole sound's peak. A column where the bass jumps (a kick) lights
//! up, the light fading over the columns after it; the oldest columns fade
//! out on the left, and the newest moves in smoothly as its frames come.
//! Scaled to the loudest column in sight (at most `GAIN` times), so a
//! quiet track fills the display too. Paused, it stands; stopped, silence
//! rolls in until it is empty.

use std::collections::VecDeque;

use egui::{Color32, Mesh, Painter, Rect, Stroke, pos2};

use crate::palette;
use crate::settings::Visual;

/// The time of a column, and its width in points.
pub const COLUMN_S: f32 = 0.02;
pub const PITCH: f32 = 2.0;
/// The bass is under `LOW_HZ`, the treble over `HIGH_HZ`, the middle
/// between them, each band through two poles a side (subtracting filtered
/// sound from the whole leaked half a kick into the middle: the filters
/// shift the phase).
const LOW_HZ: f32 = 250.0;
const HIGH_HZ: f32 = 2500.0;
/// The loudest column in sight fills this share of the half height, the
/// sound raised at most `GAIN` times for it; each band's loudest in sight
/// fills its share of `BAND_FILL`, raised at most `BAND_GAIN` times, so
/// the layers stand one inside another whatever the music (a fixed gain
/// per band let the treble of one track cover the rest).
const FILL: f32 = 0.92;
const GAIN: f32 = 3.0;
const BAND_FILL: [f32; 3] = [0.92, 0.7, 0.45];
const BAND_GAIN: f32 = 8.0;
/// The share of the width on the left over which the oldest columns fade.
const FADE: f32 = 0.22;
/// A kick: the bass's peak `KICK_RISE` over its mean (following by
/// `MEAN` a column), `KICK_GAP` columns after the last; its light fades
/// by `KICK_FADE` a column and mixes the colours this far to the peaks'.
const KICK_RISE: f32 = 0.1;
const KICK_GAP: usize = 6;
const MEAN: f32 = 0.08;
const KICK_FADE: f32 = 0.7;
const KICK_LIGHT: f32 = 0.7;

/// A column: the whole sound's peak and RMS, the bands' peaks (the bass,
/// the middle, the treble), all 0 to 1, and a kick's strength.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Column {
    pub peak: f32,
    pub rms: f32,
    pub bands: [f32; 3],
    pub kick: f32,
}

#[derive(Default)]
pub struct Waveform {
    /// Oldest first.
    pub columns: VecDeque<Column>,
    /// The column being gathered (`rms` its sum of squares) and its
    /// frames.
    gathering: Column,
    frames: usize,
    /// The filters' one-pole low-passes: the bass's two; the middle's two
    /// taken away (a high-pass each) and two; the treble's two taken away.
    poles: [f32; 8],
    /// The bass's mean, and the columns since the last kick.
    mean: f32,
    since: usize,
    /// The share of the next column gathered (or of silence owed).
    part: f32,
    idle: f32,
}

impl Waveform {
    /// Moves on with the `new` samples heard at a rate (`None`: nothing
    /// is heard: silence rolls in by the clock, `dt` seconds, while
    /// anything shows, then it goes), keeping `cap` columns.
    pub fn feed(&mut self, heard: Option<(&[f32], u32)>, dt: f32, cap: usize) {
        match heard {
            Some((new, rate)) => {
                self.idle = 0.0;
                let per = ((rate as f32 * COLUMN_S) as usize).max(1);
                let pole = |hz: f32| 1.0 - (-std::f32::consts::TAU * hz / rate as f32).exp();
                let (low, high) = (pole(LOW_HZ), pole(HIGH_HZ));
                for &x in new {
                    let p = &mut self.poles;
                    // A one-pole low-pass of `input` with state `s` and
                    // coefficient `a`; a high-pass is the input less it.
                    let lp = |s: &mut f32, a: f32, input: f32| {
                        *s += a * (input - *s);
                        *s
                    };
                    let bass = {
                        let once = lp(&mut p[0], low, x);
                        lp(&mut p[1], low, once)
                    };
                    let middle = {
                        let once = x - lp(&mut p[2], low, x);
                        let twice = once - lp(&mut p[3], low, once);
                        let cut = lp(&mut p[4], high, twice);
                        lp(&mut p[5], high, cut)
                    };
                    let treble = {
                        let once = x - lp(&mut p[6], high, x);
                        once - lp(&mut p[7], high, once)
                    };
                    let bands = [bass, middle, treble];
                    let g = &mut self.gathering;
                    g.peak = g.peak.max(x.abs());
                    g.rms += x * x;
                    for (b, v) in g.bands.iter_mut().zip(bands) {
                        *b = b.max(v.abs());
                    }
                    self.frames += 1;
                    if self.frames >= per {
                        self.close();
                    }
                }
                self.part = self.frames as f32 / per as f32;
            }
            None if self.columns.iter().any(|c| c.peak > 0.0) => {
                self.idle += dt / COLUMN_S;
                while self.idle >= 1.0 {
                    self.idle -= 1.0;
                    self.columns.push_back(Column::default());
                }
                self.part = self.idle;
            }
            None => {
                self.columns.clear();
                self.part = 0.0;
            }
        }
        while self.columns.len() > cap {
            self.columns.pop_front();
        }
    }

    /// The column gathered is done: its RMS, whether the bass kicked.
    fn close(&mut self) {
        let mut c = std::mem::take(&mut self.gathering);
        c.rms = (c.rms / self.frames.max(1) as f32).sqrt().min(1.0);
        c.peak = c.peak.min(1.0);
        for b in &mut c.bands {
            *b = b.min(1.0);
        }
        let rise = c.bands[0] - self.mean;
        self.mean += rise * MEAN;
        self.since += 1;
        if rise > KICK_RISE && self.since >= KICK_GAP {
            self.since = 0;
            c.kick = (0.5 + (rise - KICK_RISE) * 3.0).min(1.0);
        }
        self.columns.push_back(c);
        self.frames = 0;
    }

    pub fn active(&self) -> bool {
        !self.columns.is_empty()
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let cy = rect.center().y;
        let half = rect.height() / 2.0 - 1.0;
        painter.hline(rect.x_range(), cy, Stroke::new(1.0, palette::line()));
        let n = self.columns.len();
        if n == 0 {
            return;
        }
        let colours = palette::visual_for(Visual::Waveform);
        let most = self.columns.iter().map(|c| c.peak).fold(0.0, f32::max);
        let gain = if most > 0.0 {
            (FILL / most).min(GAIN)
        } else {
            1.0
        };
        let gains: [f32; 3] = std::array::from_fn(|b| {
            let most = self.columns.iter().map(|c| c.bands[b]).fold(0.0, f32::max);
            if most > 0.0 {
                (BAND_FILL[b] / most).min(BAND_GAIN)
            } else {
                1.0
            }
        });
        let shift = self.part * PITCH;
        let x_of = |i: usize| rect.right() - shift - PITCH * (n - 1 - i) as f32;
        let fade = |x: f32| ((x - rect.left()) / (rect.width() * FADE)).clamp(0.0, 1.0);
        // A kick's light, fading over the columns after it.
        let mut light = 0.0f32;
        let lights: Vec<f32> = self
            .columns
            .iter()
            .map(|c| {
                light = (light * KICK_FADE).max(c.kick);
                light
            })
            .collect();
        // The layers from the back: the whole sound's outline, the bass,
        // the middle, the treble.
        for layer in 0..4 {
            let place = [0.5, 0.0, 0.5, 1.0][layer];
            let mut mesh = Mesh::default();
            let mut first = true;
            for (i, c) in self.columns.iter().enumerate() {
                let x = x_of(i);
                if x < rect.left() - PITCH {
                    continue;
                }
                let level = match layer {
                    0 => c.peak * gain,
                    k => c.bands[k - 1] * gains[k - 1],
                };
                let level = level.min(1.0);
                let h = (level * half).max(0.5);
                let (mut edge, mut middle) = if layer == 0 {
                    // A faint haze of the bass's colour.
                    let c = colours.at(0.5, 0.0).gamma_multiply(0.3);
                    (c, c)
                } else if colours.by_level() {
                    let c = colours.at(level, place);
                    (c, c.lerp_to_gamma(colours.peak(place), 0.25))
                } else {
                    (colours.at(0.45, place), colours.at(1.0, place))
                };
                if layer > 0 && lights[i] > 0.0 {
                    let to = colours.peak(place);
                    edge = edge.lerp_to_gamma(to, lights[i] * KICK_LIGHT * 0.6);
                    middle = middle.lerp_to_gamma(to, lights[i] * KICK_LIGHT);
                }
                let alpha = fade(x);
                let (edge, middle) = (edge.gamma_multiply(alpha), middle.gamma_multiply(alpha));
                let base = mesh.vertices.len() as u32;
                mesh.colored_vertex(pos2(x, cy - h), edge);
                mesh.colored_vertex(pos2(x, cy), middle);
                mesh.colored_vertex(pos2(x, cy + h), edge);
                if !first {
                    let p = base - 3;
                    mesh.add_triangle(p, p + 1, base);
                    mesh.add_triangle(p + 1, base + 1, base);
                    mesh.add_triangle(p + 1, p + 2, base + 1);
                    mesh.add_triangle(p + 2, base + 2, base + 1);
                }
                first = false;
            }
            painter.add(mesh);
        }
        // The newest column's edge: a soft light where the sound comes in.
        if let Some(c) = self.columns.back() {
            let x = x_of(n - 1);
            let h = ((c.peak * gain).min(1.0) * half).max(2.0);
            let colour = colours.peak(0.5);
            let mut mesh = Mesh::default();
            let glow = Rect::from_min_max(pos2(x - 10.0, cy - h), pos2(x, cy + h));
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(glow.left_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(glow.right_top(), colour.gamma_multiply(0.35));
            mesh.colored_vertex(glow.right_bottom(), colour.gamma_multiply(0.35));
            mesh.colored_vertex(glow.left_bottom(), Color32::TRANSPARENT);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
            painter.add(mesh);
            painter.vline(x, glow.y_range(), Stroke::new(1.0, colour.gamma_multiply(0.8)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn tone(hz: f32, amplitude: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amplitude * (TAU * hz * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn the_bands_take_their_tones() {
        for (hz, band) in [(60.0, 0), (800.0, 1), (8000.0, 2)] {
            let mut w = Waveform::default();
            w.feed(Some((&tone(hz, 0.5, 48_000), 48_000)), 0.0, 1000);
            let c = w.columns[w.columns.len() - 1];
            let loudest = (0..3).max_by(|&a, &b| c.bands[a].total_cmp(&c.bands[b])).unwrap();
            assert_eq!(loudest, band, "{hz} Hz: {:?}", c.bands);
        }
    }

    #[test]
    fn a_kick_lights_its_column() {
        let mut w = Waveform::default();
        // Quiet treble, then a bass thump.
        w.feed(Some((&tone(6000.0, 0.1, 24_000), 48_000)), 0.0, 1000);
        assert!(w.columns.iter().all(|c| c.kick == 0.0));
        w.feed(Some((&tone(60.0, 0.8, 4800), 48_000)), 0.0, 1000);
        assert!(w.columns.iter().rev().take(5).any(|c| c.kick >= 0.5));
    }

    #[test]
    fn stopped_it_rolls_silence_then_goes() {
        let mut w = Waveform::default();
        w.feed(Some((&tone(440.0, 0.5, 9600), 48_000)), 0.0, 100);
        assert_eq!(w.columns.len(), 10);
        // Nothing heard: silence rolls in, the sound rolls out.
        for _ in 0..60 {
            w.feed(None, 0.05, 100);
        }
        assert!(w.columns.iter().all(|c| c.peak == 0.0));
        w.feed(None, 0.05, 100);
        assert!(!w.active());
    }
}
