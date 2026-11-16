//! Harp: a string per band of the analyzer across the display, the bass
//! on the left, each held at both ends between a soundboard rising to
//! the right and a curved neck, so the bass strings are long and the
//! treble ones short. A string is plucked when its band rises sharply
//! over its recent mean (a note starting, not a level held), at a place
//! of its own, and rings as a string does: its first three modes, each
//! as the pluck's place excites it, the long strings slower and ringing
//! longer; a band held loud keeps its string shimmering. A spark marks
//! where it was plucked. On a dark theme the strings ringing glow.
//! Paused, all stands; stopped, the strings ring out and the harp goes.

use std::f32::consts::{PI, TAU};

use egui::epaint::PathStroke;
use egui::{Painter, Pos2, Rect, Shape, Stroke, pos2};

use super::beat::{ONSET_FROM, Onsets, Rng, glow};
use crate::palette;
use crate::settings::Visual;

/// A string every `PITCH` points, `FEWEST` to `MOST` of them.
const PITCH: f32 = 13.0;
const FEWEST: usize = 12;
const MOST: usize = 48;
/// A string swings `HZ_LONG` times a second on the bass to `HZ_SHORT`
/// on the treble and rings out in `RING_LONG` to `RING_SHORT` seconds.
const HZ_LONG: f32 = 2.6;
const HZ_SHORT: f32 = 8.0;
const RING_LONG: f32 = 1.6;
const RING_SHORT: f32 = 0.55;
/// A pluck at full strength swings `SWING` spacings; a band held at full
/// level shimmers `SHIMMER` of that.
const SWING: f32 = 1.6;
const SHIMMER: f32 = 0.15;
/// The modes rung.
const MODES: usize = 3;
/// A spark fades in `SPARK_S`.
const SPARK_S: f32 = 0.3;
/// Points along a string.
const SEGMENTS: usize = 28;
/// Under this a string is still.
const STILL: f32 = 0.01;

#[derive(Clone, Copy, Default)]
struct Wire {
    /// The band's level now (over `ONSET_FROM`, 0–1).
    level: f32,
    /// Seconds since the last pluck, its place along the string (0: the
    /// soundboard) and the swing left of it.
    age: f32,
    at: f32,
    swing: f32,
    spark: f32,
}

pub struct Harp {
    wires: Vec<Wire>,
    onsets: Onsets,
    rng: Rng,
    /// Something was heard since the strings last went still.
    shown: bool,
}

impl Default for Harp {
    fn default() -> Self {
        Self::new()
    }
}

impl Harp {
    pub fn new() -> Harp {
        Harp {
            wires: Vec::new(),
            onsets: Onsets::default(),
            rng: Rng::default(),
            shown: false,
        }
    }

    /// How many strings a display `width` points wide holds: the bands of
    /// the levels fed.
    pub fn strings(width: f32) -> usize {
        ((width / PITCH) as usize).clamp(FEWEST, MOST)
    }

    /// Moves on with `heard`: the levels heard now (one a string) and the
    /// seconds of sound heard since the frame before (0: paused, all
    /// stands); `None`: nothing is heard, and the strings ring out for
    /// `dt` seconds. `width` is the display's.
    pub fn feed(&mut self, heard: Option<(&[f32], f32)>, dt: f32, width: f32) {
        let n = Self::strings(width);
        if self.wires.len() != n {
            self.wires = vec![Wire::default(); n];
        }
        let (levels, seconds) = match heard {
            Some((levels, seconds)) => {
                if seconds <= 0.0 {
                    return;
                }
                self.shown = true;
                (Some(levels), seconds)
            }
            None if self.shown => (None, dt),
            None => return,
        };
        let plucks = self.onsets.feed(levels, n, seconds);
        for (k, &strength) in plucks.iter().enumerate() {
            let place = k as f32 / (n - 1) as f32;
            let w = &mut self.wires[k];
            w.age += seconds;
            w.swing *= (-seconds / lerp(RING_LONG, RING_SHORT, place)).exp();
            w.spark = (w.spark - seconds / SPARK_S).max(0.0);
            let raw = levels.and_then(|l| l.get(k)).copied().unwrap_or(0.0);
            w.level = ((raw - ONSET_FROM) / (1.0 - ONSET_FROM)).clamp(0.0, 1.0);
            if strength > 0.0 {
                w.swing = w.swing.max(strength);
                w.age = 0.0;
                w.at = self.rng.between(0.2, 0.8);
                w.spark = strength;
            }
        }
        if levels.is_none() && self.wires.iter().all(|w| w.swing < STILL && w.spark <= 0.0) {
            *self = Harp::new();
        }
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.shown
    }

    /// How far string `k` stands off its rest at `s` along it (0: the
    /// soundboard, 1: the neck), in spacings: its modes as its last pluck
    /// excited them, ringing; the shimmer of a band held loud.
    fn offset(&self, k: usize, s: f32) -> f32 {
        let w = &self.wires[k];
        let place = k as f32 / (self.wires.len() - 1).max(1) as f32;
        let hz = lerp(HZ_LONG, HZ_SHORT, place);
        let excite = |m: f32| (m * PI * w.at).sin() / (m * m);
        let norm = (1..=MODES)
            .map(|m| excite(m as f32) * (m as f32 * PI * w.at).sin())
            .sum::<f32>()
            .max(0.2);
        let shape: f32 = (1..=MODES)
            .map(|m| {
                let m = m as f32;
                excite(m) * (m * PI * s).sin() * (TAU * m * hz * w.age).cos()
            })
            .sum::<f32>()
            / norm;
        let amount = w.swing + SHIMMER * w.level * w.level;
        SWING * amount * shape
    }

    /// The frame (the pillar on the left, the soundboard, the neck), then
    /// each string from its foot to its head, glowing as it rings, and
    /// the sparks where they were plucked.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let n = self.wires.len();
        if !self.shown || n < 2 || rect.width() < 32.0 || rect.height() < 24.0 {
            return;
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual_for(Visual::Harp);
        let dark = colours.is_dark();
        let frame = palette::text_weak().gamma_multiply(0.45);
        let h = rect.height();
        let left = rect.left() + 10.0;
        let span = rect.right() - 6.0 - left;
        let spacing = span / n as f32;
        // The foot and the head of a string at `place` across.
        let foot = |place: f32| rect.bottom() - 3.0 - 0.14 * h * place;
        let head =
            |place: f32| rect.top() + 3.0 + h * (0.04 + 0.42 * place + 0.06 * (TAU * place).sin());
        let x_of = |k: usize| left + spacing * (k as f32 + 0.5);
        let places: Vec<f32> = (0..n).map(|k| k as f32 / (n - 1) as f32).collect();

        // The frame.
        let line = |a: f32, b: f32, f: &dyn Fn(f32) -> f32| -> Vec<Pos2> {
            (0..=32)
                .map(|i| {
                    let t = a + (b - a) * i as f32 / 32.0;
                    pos2(left + spacing * (0.5 + t * (n - 1) as f32), f(t))
                })
                .collect()
        };
        painter.add(Shape::line(
            line(-0.02, 1.02, &foot),
            Stroke::new(2.5, frame),
        ));
        painter.add(Shape::line(
            line(-0.02, 1.02, &head),
            Stroke::new(3.0, frame),
        ));
        painter.line_segment(
            [
                pos2(left - 4.0, head(0.0) - 2.0),
                pos2(left - 4.0, foot(0.0) + 2.0),
            ],
            Stroke::new(4.0, frame),
        );

        for (k, &place) in places.iter().enumerate() {
            let w = &self.wires[k];
            let (y0, y1) = (foot(place), head(place));
            let x = x_of(k);
            let ringing = (w.swing + SHIMMER * w.level * w.level).min(1.0);
            let points: Vec<Pos2> = (0..=SEGMENTS)
                .map(|i| {
                    let s = i as f32 / SEGMENTS as f32;
                    pos2(x + self.offset(k, s) * spacing, y0 + (y1 - y0) * s)
                })
                .collect();
            let colour = colours
                .at(0.35 + 0.65 * ringing, place)
                .lerp_to_gamma(colours.peak(place), w.spark * 0.6);
            if dark && ringing > 0.02 {
                painter.add(Shape::line(
                    points.clone(),
                    PathStroke::new(4.0 + 3.0 * ringing, glow(colour, 0.3 * ringing, true)),
                ));
            }
            let width = lerp(1.8, 0.9, place) + 0.6 * ringing;
            painter.add(Shape::line(points, Stroke::new(width, colour)));
            if w.spark > 0.0 {
                let at = pos2(x + self.offset(k, w.at) * spacing, y0 + (y1 - y0) * w.at);
                let spark = colours.peak(place);
                let r = 2.0 + 5.0 * w.spark;
                if dark {
                    painter.circle_filled(at, r * 1.8, glow(spark, 0.35 * w.spark, true));
                }
                painter.circle_filled(at, r * 0.45, spark.gamma_multiply(w.spark.sqrt()));
            }
        }
        // The pegs on the neck.
        for (k, &place) in places.iter().enumerate() {
            painter.circle_filled(pos2(x_of(k), head(place)), 1.4, frame);
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f32 = 400.0;

    fn quiet() -> Vec<f32> {
        vec![0.4; Harp::strings(WIDTH)]
    }

    #[test]
    fn a_band_rising_plucks_its_string_once() {
        let mut h = Harp::new();
        for _ in 0..30 {
            h.feed(Some((&quiet(), 0.02)), 0.02, WIDTH);
        }
        assert!(h.wires.iter().all(|w| w.swing < STILL));
        let mut note = quiet();
        note[5] = 0.9;
        h.feed(Some((&note, 0.02)), 0.02, WIDTH);
        assert!(h.wires[5].swing > 0.4);
        assert!(
            h.wires
                .iter()
                .enumerate()
                .all(|(k, w)| k == 5 || w.swing < STILL)
        );
        // Held: not plucked again, it rings out.
        let first = h.wires[5].swing;
        for _ in 0..50 {
            h.feed(Some((&note, 0.02)), 0.02, WIDTH);
        }
        assert!(h.wires[5].swing < first * 0.6, "{}", h.wires[5].swing);
        // Paused: all stands.
        let age = h.wires[5].age;
        h.feed(Some((&note, 0.0)), 0.5, WIDTH);
        assert_eq!(h.wires[5].age, age);
    }

    #[test]
    fn a_string_is_held_at_both_ends_and_swings_between() {
        let mut h = Harp::new();
        let mut note = quiet();
        h.feed(Some((&note, 0.02)), 0.02, WIDTH);
        note[0] = 1.0;
        h.feed(Some((&note, 0.02)), 0.02, WIDTH);
        assert_eq!(h.wires[0].age, 0.0);
        // As plucked: out the most at the pluck's place.
        let w = h.wires[0];
        let full = SWING * (w.swing + SHIMMER * w.level * w.level);
        assert!((h.offset(0, w.at) - full).abs() < 1e-3);
        for s in [0.0, 1.0] {
            assert!(h.offset(0, s).abs() < 1e-4);
        }
        // Half a swing later it is on the other side.
        h.feed(Some((&note, 0.5 / HZ_LONG)), 0.0, WIDTH);
        assert!(h.offset(0, 0.5) < 0.0, "{}", h.offset(0, 0.5));
    }

    #[test]
    fn stopped_the_strings_ring_out_and_the_harp_goes() {
        let mut h = Harp::new();
        let mut note = quiet();
        h.feed(Some((&note, 0.02)), 0.02, WIDTH);
        note.fill(1.0);
        h.feed(Some((&note, 0.02)), 0.02, WIDTH);
        assert!(h.active());
        for _ in 0..200 {
            h.feed(None, 0.05, WIDTH);
        }
        assert!(!h.active());
    }
}
