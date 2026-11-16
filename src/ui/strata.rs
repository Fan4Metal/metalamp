//! Strata: a string per band of the analyzer, the bass in front at the
//! bottom and the higher bands behind it, higher and fainter, all held
//! at a nut on the left, where they gather, and fanning out to the right.
//! Each column of levels comes in at the nut and runs right along the
//! strings, as a wave sent down a rope: the loud moments travel away as
//! swells, each string vibrating under its swell (a sine running right,
//! long and slow on the bass, short and quick on the treble). A beat
//! plucks the strings: a bump runs from the nut to the free end, the
//! bass strings most, and they flash. The strings fade and thin out
//! towards the right, as a sound dies away; each hides what is behind it
//! (layered ranges). On a dark theme the strings glow. The columns move
//! by the sound heard (a share of a column at a time, so they glide);
//! paused, all stands; stopped, silence comes in until the strings are
//! still, then they go.

use std::collections::VecDeque;
use std::f32::consts::TAU;

use egui::epaint::PathStroke;
use egui::{Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2};

use super::beat::{Beat, glow};
use super::visual::smoothed;
use crate::palette;
use crate::settings::Visual;

/// A column of levels comes in every `COLUMN_S` and moves `PITCH`
/// points along the strings.
const COLUMN_S: f32 = 0.02;
const PITCH: f32 = 2.0;
/// A string every `SPACING` points at the free end, in the lower `ROOM`
/// of the display (`FEWEST`–`MOST` strings).
const SPACING: f32 = 5.0;
const ROOM: f32 = 0.7;
const FEWEST: usize = 6;
const MOST: usize = 32;
/// At the nut the strings take `FAN` of the room they take at the free
/// end; they spread over the first `SPREAD` of the width.
const FAN: f32 = 0.22;
const SPREAD: f32 = 0.4;
/// A string is still at the nut and swings fully from `ANCHOR` of the
/// width on.
const ANCHOR: f32 = 0.1;
/// A swell at full level rises `RISE` spacings (levels raised to
/// `POWER`), `LIFT` of it a rise and the rest a swing either way.
const RISE: f32 = 5.0;
const POWER: f32 = 2.0;
const LIFT: f32 = 0.35;
/// The vibration: its wave `WAVE_LONG` points long on the bass down to
/// `WAVE_SHORT` on the treble, running `HZ_LOW` to `HZ_HIGH` times a
/// second.
const WAVE_LONG: f32 = 150.0;
const WAVE_SHORT: f32 = 34.0;
const HZ_LOW: f32 = 1.4;
const HZ_HIGH: f32 = 3.6;
/// A pluck runs `PLUCK_SPEED` widths a second, `PLUCK_WIDTH` of the width
/// wide, `PLUCK` spacings high at full strength on the lowest string,
/// and goes once past `PLUCK_END` of the width.
const PLUCK_SPEED: f32 = 1.5;
const PLUCK_WIDTH: f32 = 0.05;
const PLUCK: f32 = 3.0;
const PLUCK_END: f32 = 1.3;
/// The strings fade to `1 - FADE` of their light at the free end; the
/// one furthest back is `FAINTER` fainter.
const FADE: f32 = 0.85;
const FAINTER: f32 = 0.55;
/// The string's width, and how much wider a kick makes the bass's.
const WIDTH: f32 = 1.3;
const KICK_WIDTH: f32 = 1.0;

pub struct Strata {
    /// The columns, newest first (at the nut): the bands' levels, the
    /// lowest band first.
    columns: VecDeque<Vec<f32>>,
    /// The share of the next column already in: the columns stand that
    /// far right of their places.
    due: f32,
    /// The sound heard, in seconds: the vibration's clock.
    time: f32,
    beat: Beat,
    /// The plucks running: their place (a share of the width from the
    /// nut) and strength.
    plucks: Vec<(f32, f32)>,
}

impl Default for Strata {
    fn default() -> Self {
        Self::new()
    }
}

impl Strata {
    pub fn new() -> Strata {
        Strata {
            columns: VecDeque::new(),
            due: 0.0,
            time: 0.0,
            beat: Beat::default(),
            plucks: Vec::new(),
        }
    }

    /// How many strings a display `height` points high shows: the bands
    /// of the levels fed.
    pub fn lines(height: f32) -> usize {
        ((height * ROOM / SPACING) as usize).clamp(FEWEST, MOST)
    }

    /// Moves on with `heard`: the levels heard now (`lines` bands) and the
    /// seconds of sound heard since the frame before (0: paused, all
    /// stands); `None`: nothing is heard, and silence comes in for `dt`
    /// seconds. `size` is the display's.
    pub fn feed(&mut self, heard: Option<(&[f32], f32)>, dt: f32, size: Vec2) {
        let bands = Self::lines(size.y);
        let cap = ((size.x / PITCH) as usize).clamp(8, 2048) + 2;
        if self.columns.front().is_some_and(|c| c.len() != bands) {
            self.columns.clear();
        }
        let (column, seconds) = match heard {
            Some((levels, seconds)) => {
                if self.columns.is_empty() {
                    // Still strings across from the start.
                    self.columns = VecDeque::from(vec![vec![0.0; bands]; cap]);
                }
                let mut column = levels.to_vec();
                column.resize(bands, 0.0);
                (column, seconds)
            }
            None if !self.still() => (vec![0.0; bands], dt),
            None => {
                *self = Strata::new();
                return;
            }
        };
        if seconds <= 0.0 {
            return;
        }
        self.time += seconds;
        let struck = self.beat.feed(heard.map(|_| column.as_slice()), seconds);
        for pluck in &mut self.plucks {
            pluck.0 += PLUCK_SPEED * seconds;
        }
        self.plucks.retain(|p| p.0 < PLUCK_END);
        if let Some(strength) = struck {
            self.plucks.push((0.0, strength));
        }
        self.due += seconds / COLUMN_S;
        let new = self.due as usize;
        self.due -= new as f32;
        for _ in 0..new.min(cap) {
            self.columns.push_front(column.clone());
        }
        while self.columns.len() > cap {
            self.columns.pop_back();
        }
    }

    /// Nothing moves the strings any more.
    fn still(&self) -> bool {
        self.plucks.is_empty() && self.columns.iter().all(|c| c.iter().all(|&l| l <= 0.0))
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        !self.columns.is_empty()
    }

    /// How far string `band` of `bands` stands off its base line at `x`
    /// points from the nut in a display `width` wide, at `level` (0–1)
    /// there: the swell's rise and swing, and the plucks passing.
    fn offset(&self, band: usize, bands: usize, x: f32, width: f32, level: f32) -> f32 {
        let depth = band as f32 / (bands - 1).max(1) as f32;
        let u = x / width.max(1.0);
        let hold = smoothstep(u / ANCHOR);
        let h = level.clamp(0.0, 1.0).powf(POWER);
        let wave = lerp(WAVE_LONG, WAVE_SHORT, depth);
        let hz = lerp(HZ_LOW, HZ_HIGH, depth);
        let swing = (TAU * (x / wave - hz * self.time) + band as f32 * 1.7).sin();
        let swell = RISE * SPACING * h * (LIFT + (1.0 - LIFT) * swing);
        let pluck = PLUCK * SPACING * (1.0 - depth).powf(1.5) * self.pluck_at(u);
        hold * (swell + pluck)
    }

    /// The plucks' height at `u` (a share of the width from the nut),
    /// each fading as it runs.
    fn pluck_at(&self, u: f32) -> f32 {
        self.plucks
            .iter()
            .map(|&(at, strength)| {
                let d = (u - at) / PLUCK_WIDTH;
                strength * (-d * d).exp() * (1.0 - 0.6 * (at / PLUCK_END))
            })
            .sum()
    }

    /// The strings, the highest band first: each fills the room down to
    /// its base line with the display's ground, hiding the strings behind
    /// it, then draws its glow and itself. The nut is a faint bar.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let Some(bands) = self.columns.front().map(Vec::len) else {
            return;
        };
        if bands == 0 || rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let ground = palette::list_bg();
        let colours = palette::visual_for(Visual::Strata);
        let dark = colours.is_dark();
        let bright = palette::text_bright();
        let width = rect.width();
        let left = rect.left();
        let bottom = rect.bottom() - 1.0;
        let full = SPACING * (bands - 1) as f32;
        let middle = bottom - full / 2.0;
        let rise = RISE * SPACING;
        let kick = self.beat.kick;
        // The places along the strings: the nut, then each column's,
        // a share of a column further right than its own.
        let places: Vec<f32> = std::iter::once(0.0)
            .chain(
                (0..self.columns.len())
                    .map(|i| (i as f32 + self.due) * PITCH)
                    .filter(|&x| x > 0.0),
            )
            .take_while(|&x| x <= width + PITCH)
            .collect();
        let spread = |u: f32| {
            let t = 1.0 - (u / SPREAD).min(1.0);
            FAN + (1.0 - FAN) * (1.0 - t * t)
        };
        for band in (0..bands).rev() {
            let depth = band as f32 / (bands - 1).max(1) as f32;
            let own = bottom - SPACING * band as f32;
            let levels: Vec<f32> = self.columns.iter().map(|c| c[band]).collect();
            let levels = smoothed(&levels);
            let mut bases = Vec::with_capacity(places.len());
            let mut points = Vec::with_capacity(places.len());
            for (j, &x) in places.iter().enumerate() {
                // The nut takes the newest column's level, and each
                // column after it its own.
                let level = levels[j.saturating_sub(1).min(levels.len() - 1)];
                let base = middle + (own - middle) * spread(x / width);
                bases.push(base);
                points.push(pos2(
                    left + x,
                    base - self.offset(band, bands, x, width, level),
                ));
            }
            // The ground under the string, a strip of the string's points
            // and their base's.
            let mut mesh = Mesh::default();
            for (k, (p, base)) in points.iter().zip(&bases).enumerate() {
                mesh.colored_vertex(*p, ground);
                mesh.colored_vertex(pos2(p.x, base + 1.0), ground);
                if k > 0 {
                    let i = 2 * k as u32;
                    mesh.add_triangle(i - 2, i, i + 1);
                    mesh.add_triangle(i - 2, i + 1, i - 1);
                }
            }
            painter.add(mesh);

            // Each point's colour from how high it stands over its base
            // (`base_at` finds the base from the place across), fading
            // to the right, the bass strings flashing on a kick.
            let flash = kick * (1.0 - depth).powf(1.5);
            let back = 1.0 - FAINTER * depth;
            let base_at = move |x: f32| {
                let u = ((x - left) / width).clamp(0.0, 1.0);
                (u, middle + (own - middle) * spread(u))
            };
            let colour_at = move |p: Pos2| {
                let (u, base) = base_at(p.x);
                let level = ((base - p.y) / rise).clamp(0.0, 1.0).sqrt();
                let fade = back * (1.0 - FADE * u.powf(1.3));
                colours
                    .at(level.max(0.3), depth)
                    .lerp_to_gamma(bright, (flash * 0.6 + level * 0.15).min(1.0))
                    .gamma_multiply(fade)
            };
            if dark {
                // Blurred anyway: every other point does.
                let mut sparse: Vec<Pos2> = points.iter().copied().step_by(2).collect();
                if points.len() % 2 == 0 {
                    sparse.extend(points.last());
                }
                painter.add(Shape::line(
                    sparse,
                    PathStroke::new_uv(5.0 + 4.0 * flash, move |_, p| {
                        glow(colour_at(p), 0.22 + 0.3 * flash, true)
                    }),
                ));
            }
            painter.add(Shape::line(
                points,
                PathStroke::new_uv(WIDTH + KICK_WIDTH * flash, move |_, p| colour_at(p)),
            ));
        }
        // The nut: the strings gather at it.
        let half = full * FAN / 2.0 + 3.0;
        painter.vline(
            left + 0.5,
            (middle - half)..=(middle + half),
            Stroke::new(2.0, palette::text_weak().gamma_multiply(0.6)),
        );
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 0 below 0, 1 above 1, an S between.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Vec2 = Vec2::new(400.0, 90.0);

    fn loud() -> Vec<f32> {
        vec![0.9; Strata::lines(SIZE.y)]
    }

    #[test]
    fn a_column_every_20_ms_gliding_between() {
        let mut s = Strata::new();
        let levels = loud();
        s.feed(Some((&levels, 0.0)), 0.02, SIZE);
        // Still strings across from the start.
        let cap = s.columns.len();
        assert_eq!(cap, (400.0 / PITCH) as usize + 2);
        assert!(s.columns.iter().all(|c| c.iter().all(|&l| l == 0.0)));
        // 50 ms: two columns and half of the next.
        s.feed(Some((&levels, 0.05)), 0.05, SIZE);
        assert_eq!(s.columns.len(), cap);
        assert!(s.columns[0][0] > 0.5 && s.columns[1][0] > 0.5);
        assert_eq!(s.columns[2][0], 0.0);
        assert!((s.due - 0.5).abs() < 1e-4, "{}", s.due);
        // Paused: nothing moves.
        let time = s.time;
        s.feed(Some((&levels, 0.0)), 0.5, SIZE);
        assert!((s.due - 0.5).abs() < 1e-4);
        assert_eq!(s.time, time);
        // Another height: other bands, started anew.
        s.feed(Some((&loud(), 0.02)), 0.02, Vec2::new(400.0, 200.0));
        assert_eq!(s.columns[0].len(), Strata::lines(200.0));
    }

    #[test]
    fn the_strings_are_held_at_the_nut() {
        let mut s = Strata::new();
        let levels = loud();
        for _ in 0..50 {
            s.feed(Some((&levels, 0.02)), 0.02, SIZE);
        }
        let bands = Strata::lines(SIZE.y);
        for band in 0..bands {
            assert_eq!(s.offset(band, bands, 0.0, 400.0, 1.0), 0.0);
        }
        // Further along a loud string swings, now up, now down.
        let along: Vec<f32> = (40..400)
            .map(|x| s.offset(0, bands, x as f32, 400.0, 1.0))
            .collect();
        assert!(along.iter().any(|&o| o > SPACING * 2.0));
        assert!(along.iter().any(|&o| o < 0.0));
        // A quiet one does not, with no pluck passing (the loud start
        // struck beats).
        s.plucks.clear();
        assert!(s.offset(0, bands, 200.0, 400.0, 0.0).abs() < 1e-3);
    }

    #[test]
    fn a_beat_plucks_and_the_pluck_runs_off() {
        let mut s = Strata::new();
        let bands = Strata::lines(SIZE.y);
        let quiet = vec![0.1; bands];
        for _ in 0..30 {
            s.feed(Some((&quiet, 0.02)), 0.02, SIZE);
        }
        assert!(s.plucks.is_empty());
        let mut kick = vec![0.3; bands];
        kick[..bands / 6 + 1].fill(0.95);
        s.feed(Some((&kick, 0.02)), 0.02, SIZE);
        assert_eq!(s.plucks.len(), 1);
        // It runs right: the bass string stands up where it is.
        s.feed(Some((&quiet, 0.2)), 0.2, SIZE);
        let at = s.plucks[0].0;
        assert!(at > 0.25, "{at}");
        assert!(s.offset(0, bands, at * 400.0, 400.0, 0.0) > SPACING);
        // And goes past the end.
        for _ in 0..60 {
            s.feed(Some((&quiet, 0.02)), 0.02, SIZE);
        }
        assert!(s.plucks.is_empty());
    }

    #[test]
    fn stopped_the_strings_still_and_go() {
        let mut s = Strata::new();
        let levels = loud();
        for _ in 0..20 {
            s.feed(Some((&levels, 0.02)), 0.02, SIZE);
        }
        assert!(s.active());
        for _ in 0..400 {
            s.feed(None, 0.05, SIZE);
        }
        assert!(!s.active());
    }
}
