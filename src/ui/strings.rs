//! Guitar strings: the notes heard (`notes::Notes`) played on a guitar's
//! neck, seen from above, the nut on the left, the high E string on top
//! as in a tablature. The loudest notes (at most one a string) are given
//! to strings as a hand would play them: each to the free string whose
//! fret is nearest the hand's place, and the hand moves along the neck
//! towards the frets it plays. A string played rings: the part from its
//! fret to the bridge swings as a standing wave (its first two modes,
//! faster the higher the note), as wide as the note is loud, and the
//! finger's place on it glows. The frets are set as a real neck's, each
//! narrower than the one before by the twelfth root of two, as many as
//! fit (`LEAST_FRET` points the narrowest). Paused, the strings stand;
//! stopped, they ring out.

use std::f32::consts::{PI, TAU};

use egui::epaint::PathStroke;
use egui::{Painter, Pos2, Rect, Shape, Stroke, pos2};

use super::beat::glow;
use super::notes;
use crate::palette;

/// The open strings, the low E first (standard tuning: E2 A2 D3 G3 B3 E4).
pub const OPEN: [u8; 6] = [40, 45, 50, 55, 59, 64];
/// The strings' widths in points, the low E first.
const WIDTHS: [f32; 6] = [2.2, 1.9, 1.6, 1.3, 1.0, 0.8];
/// The frets: at least `FEWEST`, at most `MOST`, the narrowest at least
/// `LEAST_FRET` points wide; the neck takes `NECK` of the width.
const FEWEST: u8 = 5;
const MOST: u8 = 19;
const LEAST_FRET: f32 = 11.0;
const NECK: f32 = 0.84;
/// A note lights from this level, fully over `SPAN` more.
const FROM: f32 = 0.35;
const SPAN: f32 = 0.4;
/// A string rings out in this long.
const RING_S: f32 = 0.6;
/// The hand moves to the frets played in about this long.
const HAND_S: f32 = 0.6;
/// The swing at full level, as a share of the strings' spacing; the
/// swing's speed for the low E, faster by an octave each two.
const SWING: f32 = 0.42;
const SWING_HZ: f32 = 5.0;
/// The dots of the neck's inlays, by fret (12 and 24 doubled).
const INLAYS: [u8; 9] = [3, 5, 7, 9, 12, 15, 17, 19, 21];

/// A string's note: its fret and how loud it rings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Ringing {
    fret: u8,
    level: f32,
}

pub struct Strings {
    ringing: [Ringing; 6],
    /// The hand's place on the neck, in frets.
    hand: f32,
    frets: u8,
    time: f32,
}

impl Default for Strings {
    fn default() -> Self {
        Self::new()
    }
}

impl Strings {
    pub fn new() -> Strings {
        Strings {
            ringing: [Ringing::default(); 6],
            hand: 0.0,
            frets: 12,
            time: 0.0,
        }
    }

    /// Moves on by `dt` seconds with the notes' levels heard now (from
    /// `notes::LOWEST`; `moved` false: paused, they stand) for a display
    /// `width` points wide.
    pub fn feed(&mut self, keys: &[f32], moved: bool, dt: f32, width: f32) {
        self.frets = frets_for(width);
        if !moved {
            return;
        }
        self.time += dt;
        for r in &mut self.ringing {
            r.level = (r.level - dt / RING_S).max(0.0);
        }
        // The notes a guitar can play, the loudest first.
        let highest = OPEN[5] + self.frets;
        let mut heard: Vec<(u8, f32)> = (OPEN[0]..=highest)
            .filter_map(|m| {
                let level = keys
                    .get(usize::from(m.checked_sub(notes::LOWEST)?))
                    .copied()?;
                let light = ((level - FROM) / SPAN).clamp(0.0, 1.0);
                (light > 0.05).then_some((m, light))
            })
            .collect();
        heard.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut taken = [false; 6];
        let mut frets_played = (0.0, 0.0);
        for (m, light) in heard {
            let best = (0..6)
                .filter(|&s| !taken[s] && m >= OPEN[s] && m - OPEN[s] <= self.frets)
                .min_by(|&a, &b| {
                    let off = |s: usize| (f32::from(m - OPEN[s]) - self.hand).abs();
                    off(a).total_cmp(&off(b))
                });
            let Some(s) = best else { continue };
            taken[s] = true;
            let fret = m - OPEN[s];
            let r = &mut self.ringing[s];
            if light >= r.level || r.fret == fret {
                *r = Ringing {
                    fret,
                    level: light.max(r.level),
                };
            }
            if fret > 0 {
                frets_played.0 += f32::from(fret) * light;
                frets_played.1 += light;
            }
            if taken.iter().all(|&t| t) {
                break;
            }
        }
        if frets_played.1 > 0.0 {
            // The hand's place: a little below the frets it plays.
            let wanted = (frets_played.0 / frets_played.1 - 1.5).max(0.0);
            self.hand += (wanted - self.hand) * (dt / HAND_S).min(1.0);
        }
    }

    /// Some string still rings: keep repainting.
    pub fn active(&self) -> bool {
        self.ringing.iter().any(|r| r.level > 0.0)
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual();
        let dark = colours.is_dark();
        let weak = palette::text_weak();
        let (w, h) = (rect.width(), rect.height());
        if w < 40.0 || h < 16.0 {
            return;
        }
        let margin = (h * 0.14).max(4.0);
        let spacing = (h - 2.0 * margin) / 5.0;
        let y_of = |s: usize| rect.bottom() - margin - spacing * s as f32;
        let nut = rect.left() + 8.0;
        let neck_end = rect.left() + w * NECK;
        let bridge = rect.right() - 4.0;
        let x_of = fret_places(nut, neck_end, self.frets);

        // The fretboard, its frets and inlays.
        let board = Rect::from_min_max(
            pos2(nut, y_of(5) - spacing * 0.55),
            pos2(neck_end, y_of(0) + spacing * 0.55),
        );
        painter.rect_filled(board, 2.0, weak.gamma_multiply(0.07));
        for (f, &x) in x_of.iter().enumerate() {
            let (width, alpha) = if f == 0 { (2.5, 0.7) } else { (1.0, 0.35) };
            painter.line_segment(
                [pos2(x, board.top()), pos2(x, board.bottom())],
                Stroke::new(width, weak.gamma_multiply(alpha)),
            );
        }
        let inlay = weak.gamma_multiply(0.22);
        let dot = (spacing * 0.18).clamp(1.5, 3.5);
        for &f in INLAYS.iter().filter(|&&f| f <= self.frets) {
            let x = (x_of[usize::from(f) - 1] + x_of[usize::from(f)]) / 2.0;
            if f % 12 == 0 {
                for between in [(1, 2), (3, 4)] {
                    let y = (y_of(between.0) + y_of(between.1)) / 2.0;
                    painter.circle_filled(pos2(x, y), dot, inlay);
                }
            } else {
                painter.circle_filled(pos2(x, (y_of(2) + y_of(3)) / 2.0), dot, inlay);
            }
        }
        // The bridge's saddle.
        painter.line_segment(
            [
                pos2(bridge, y_of(5) - spacing * 0.4),
                pos2(bridge, y_of(0) + spacing * 0.4),
            ],
            Stroke::new(2.0, weak.gamma_multiply(0.5)),
        );

        for (s, r) in self.ringing.iter().enumerate() {
            let y = y_of(s);
            let place = s as f32 / 5.0;
            let rest = weak.gamma_multiply(0.75);
            let width = WIDTHS[s];
            if r.level <= 0.0 {
                painter.line_segment([pos2(nut, y), pos2(bridge, y)], Stroke::new(width, rest));
                continue;
            }
            let fret = usize::from(r.fret.min(self.frets));
            let from = x_of[fret];
            if fret > 0 {
                painter.line_segment([pos2(nut, y), pos2(from, y)], Stroke::new(width, rest));
            }
            // The two first modes of the part from the fret to the
            // bridge, the second a third as wide and twice as fast.
            let swing = SWING * spacing * r.level;
            let note = OPEN[s] + r.fret;
            let hz = (SWING_HZ * 2f32.powf(f32::from(note - OPEN[0]) / 24.0)).min(18.0);
            let phase = TAU * hz * self.time + s as f32;
            let (a1, a2) = (phase.sin(), (2.0 * phase + 1.0).sin() * 0.35);
            let steps = (((bridge - from) / 3.0) as usize).max(8);
            let points: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let u = i as f32 / steps as f32;
                    let off = swing * ((PI * u).sin() * a1 + (TAU * u).sin() * a2);
                    pos2(from + (bridge - from) * u, y + off)
                })
                .collect();
            let colour = colours.at(0.4 + 0.6 * r.level, place);
            painter.add(Shape::line(
                points.clone(),
                PathStroke::new(width * 3.5 + 2.0, glow(colour, 0.25 * r.level, dark)),
            ));
            painter.add(Shape::line(
                points,
                PathStroke::new(width.max(1.2), rest.lerp_to_gamma(colour, r.level.sqrt())),
            ));
            // The finger, between the fret and the one before; an open
            // string glows at the nut.
            let at = if fret == 0 {
                pos2(nut, y)
            } else {
                pos2((x_of[fret - 1] + x_of[fret]) / 2.0, y)
            };
            let radius = (spacing * 0.32).clamp(2.5, 7.0);
            painter.circle_filled(at, radius * 1.8, glow(colour, 0.25 * r.level, dark));
            painter.circle_filled(at, radius, glow(colour, 0.5 + 0.5 * r.level, dark));
        }
    }
}

/// How many frets fit a display `width` points wide.
fn frets_for(width: f32) -> u8 {
    let neck = (width * NECK - 8.0).max(1.0);
    (FEWEST..=MOST)
        .rev()
        .find(|&n| {
            let x = fret_places(0.0, neck, n);
            x[usize::from(n)] - x[usize::from(n) - 1] >= LEAST_FRET
        })
        .unwrap_or(FEWEST)
}

/// The places of the nut (0) and of each fret up to `frets`, the last at
/// `end`: a real neck's, each fret the twelfth root of two narrower.
fn fret_places(nut: f32, end: f32, frets: u8) -> Vec<f32> {
    let reach = |n: u8| 1.0 - 2f32.powf(-f32::from(n) / 12.0);
    let scale = (end - nut) / reach(frets);
    (0..=frets).map(|n| nut + scale * reach(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys' levels with these notes at `level`.
    fn keys(notes: &[(u8, f32)]) -> Vec<f32> {
        let mut k = vec![0.0; usize::from(notes::HIGHEST - notes::LOWEST) + 1];
        for &(m, level) in notes {
            k[usize::from(m - notes::LOWEST)] = level;
        }
        k
    }

    #[test]
    fn the_frets_narrow_as_a_necks() {
        let x = fret_places(0.0, 100.0, 12);
        assert!((x[12] - 100.0).abs() < 1e-3);
        // The twelfth fret is halfway to the bridge: the first six frets
        // take more room than the next six.
        assert!(x[6] > 50.0);
        assert!(x[1] - x[0] > x[12] - x[11]);
        assert_eq!(frets_for(100.0), FEWEST);
        assert_eq!(frets_for(5000.0), MOST);
        let f = frets_for(700.0);
        assert!((FEWEST..=MOST).contains(&f), "{f}");
    }

    #[test]
    fn a_chord_rings_one_note_a_string() {
        let mut s = Strings::new();
        // An open E major chord: E2 B2 E3 G#3 B3 E4.
        let chord = keys(&[
            (40, 1.0),
            (47, 0.9),
            (52, 0.9),
            (56, 0.9),
            (59, 0.9),
            (64, 0.9),
        ]);
        s.feed(&chord, true, 0.02, 700.0);
        let frets: Vec<u8> = s.ringing.iter().map(|r| r.fret).collect();
        assert!(s.ringing.iter().all(|r| r.level > 0.5), "{:?}", s.ringing);
        // Each note on a string of its own, within reach of the nut.
        let played: Vec<u8> = (0..6).map(|i| OPEN[i] + frets[i]).collect();
        let mut sorted = played.clone();
        sorted.sort();
        assert_eq!(sorted, [40, 47, 52, 56, 59, 64], "{frets:?}");
        assert!(frets.iter().all(|&f| f <= 5), "{frets:?}");
        // Paused, they stand; then they ring out.
        let before = s.ringing;
        s.feed(&keys(&[]), false, 0.02, 700.0);
        assert_eq!(s.ringing, before);
        for _ in 0..50 {
            s.feed(&keys(&[]), true, 0.02, 700.0);
        }
        assert!(!s.active());
    }

    #[test]
    fn the_hand_moves_up_the_neck() {
        let mut s = Strings::new();
        // A high A5 again and again: played high on the neck, the hand
        // follows it.
        let high = keys(&[(81, 1.0)]);
        for _ in 0..100 {
            s.feed(&high, true, 0.02, 700.0);
        }
        assert!(s.hand > 8.0, "{}", s.hand);
        // A note quieter than the notes' floor plays nothing.
        let mut s = Strings::new();
        s.feed(&keys(&[(45, 0.3)]), true, 0.02, 700.0);
        assert!(!s.active());
    }
}
