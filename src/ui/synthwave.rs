//! Synthwave: a flight over a grid receding to the horizon, as the
//! genre's pictures draw it. A row of the analyzer's levels comes in at
//! the horizon every `ROW_S` and moves towards the viewer, row by row,
//! smoothly between them (the share of the next row due); the rows are
//! drawn in perspective (a row at depth `z` lies `1/z` of the floor's
//! height under the horizon, its width `1/z` of the world's), the bands
//! mirrored from the middle outwards, a flat road in the middle and
//! mountains on both sides as high as their bands are loud. The rows are
//! drawn from the furthest: each fills the room under its mountains with
//! the display's ground, hiding what is behind, then the lines to the row
//! behind it and its own line; further rows fade into the dark. Over the
//! horizon a striped sun, behind the mountains, and a glow. Paused, it
//! stands; stopped, flat rows come in until the mountains are gone.

use std::collections::VecDeque;

use egui::epaint::PathStroke;
use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2};

use crate::palette;

/// The bands of a row.
pub const BANDS: usize = 24;
/// A row comes in this often; this many are shown.
const ROW_S: f32 = 0.06;
const ROWS: usize = 28;
/// The nearest row's depth as it leaves, the furthest's as it comes.
const NEAR: f32 = 0.9;
const FAR: f32 = 12.0;
/// The horizon, as a share of the display's height from its top.
const HORIZON: f32 = 0.48;
/// The world's width at depth 1, in display widths.
const WIDTH: f32 = 5.0;
/// A full band's mountain, in floor heights at depth 1; levels raised to
/// `POWER`; the road's half width, and where the mountains are whole, as
/// shares of the half width.
const MOUNTAIN: f32 = 4.0;
const POWER: f32 = 1.6;
const ROAD: f32 = 0.06;
const SLOPES: f32 = 0.25;
/// The sun's radius, as a share of the sky's height; its stripes' pitch
/// as a share of its radius.
const SUN: f32 = 0.85;
const STRIPES: f32 = 0.16;

pub struct Synthwave {
    /// The rows, the nearest (oldest) first: the bands' levels.
    rows: VecDeque<Vec<f32>>,
    /// Rows owed (the fraction: how far towards the next one).
    due: f32,
}

impl Default for Synthwave {
    fn default() -> Self {
        Self::new()
    }
}

impl Synthwave {
    pub fn new() -> Synthwave {
        Synthwave {
            rows: VecDeque::new(),
            due: 0.0,
        }
    }

    /// Moves on with `heard`: the levels heard now and the seconds of
    /// sound heard since the frame before; `None`: nothing is heard, and
    /// flat rows come in for `dt` seconds.
    pub fn feed(&mut self, heard: Option<(Vec<f32>, f32)>, dt: f32) {
        let (row, seconds) = match heard {
            Some((levels, seconds)) => {
                if self.rows.is_empty() {
                    // A flat field to fly over from the start.
                    self.rows = VecDeque::from(vec![vec![0.0; levels.len()]; ROWS]);
                }
                (levels, seconds)
            }
            None if self.rows.iter().any(|r| r.iter().any(|&l| l > 0.0)) => {
                let width = self.rows.back().map_or(BANDS, Vec::len);
                (vec![0.0; width], dt)
            }
            None => {
                self.rows.clear();
                self.due = 0.0;
                return;
            }
        };
        self.due += seconds / ROW_S;
        let new = self.due as usize;
        self.due -= new as f32;
        for _ in 0..new.min(ROWS) {
            self.rows.push_back(row.clone());
        }
        while self.rows.len() > ROWS {
            self.rows.pop_front();
        }
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        !self.rows.is_empty()
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let n = self.rows.len();
        if n < 2 || rect.height() < 8.0 {
            return;
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual();
        let ground = palette::list_bg();
        let horizon = rect.top() + rect.height() * HORIZON;
        let floor = rect.bottom() - horizon;
        let cx = rect.center().x;
        let world = WIDTH * rect.width();
        let project = |u: f32, z: f32, h: f32| {
            pos2(
                cx + (u - 0.5) * world / z,
                horizon + floor * (1.0 - h * MOUNTAIN) / z,
            )
        };
        let place = move |p: Pos2| ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        // The furthest row's bass swells the sun.
        let bass = self.rows.back().map_or(0.0, |r| {
            let low = (r.len() / 6).max(1).min(r.len());
            r[..low].iter().sum::<f32>() / low.max(1) as f32
        });

        // The sky's glow over the horizon.
        let glow = colours.at(1.0, 0.5).gamma_multiply(0.22);
        let mut mesh = Mesh::default();
        let sky = Rect::from_min_max(pos2(rect.left(), rect.top()), pos2(rect.right(), horizon));
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(sky.left_top(), Color32::TRANSPARENT);
        mesh.colored_vertex(sky.right_top(), Color32::TRANSPARENT);
        mesh.colored_vertex(sky.right_bottom(), glow);
        mesh.colored_vertex(sky.left_bottom(), glow);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);

        // The sun: slices of a disc over the horizon, its lower half cut
        // by stripes that widen towards the horizon.
        let r = ((horizon - rect.top()) * SUN * (1.0 + 0.06 * bass)).min(rect.width() / 3.0);
        let (top, foot) = (colours.at(1.0, 0.85), colours.at(0.45, 0.0));
        let pitch = (r * STRIPES).max(3.0);
        let mut y = horizon - r;
        while y < horizon {
            let y1 = (y + 1.0).min(horizon);
            let t = (y - (horizon - r)) / r;
            let lower = t - 0.5;
            let cut = lower > 0.0 && {
                let into = (horizon - y) % pitch;
                into < pitch * (0.1 + 0.9 * lower)
            };
            if !cut {
                let dy = horizon - (y + y1) / 2.0;
                let half = (r * r - dy * dy).max(0.0).sqrt();
                mesh.add_colored_rect(
                    Rect::from_min_max(pos2(cx - half, y), pos2(cx + half, y1)),
                    top.lerp_to_gamma(foot, t),
                );
            }
            y = y1;
        }
        painter.add(mesh);
        painter.hline(
            rect.x_range(),
            horizon,
            Stroke::new(1.0, colours.at(1.0, 0.5).gamma_multiply(0.8)),
        );

        // The rows, each at its depth, the nearest leaving as the next
        // comes in at the horizon.
        let step = (FAR - NEAR) / ROWS as f32;
        let depth = |j: usize| NEAR + (j as f32 + 1.0 - self.due) * step;
        let fog =
            |z: f32| 0.08 + 0.92 * (1.0 - (z - NEAR) / (FAR - NEAR)).clamp(0.0, 1.0).powf(0.7);
        let rows: Vec<(f32, Vec<Pos2>, Vec<Pos2>)> = self
            .rows
            .iter()
            .enumerate()
            .map(|(j, row)| {
                let z = depth(j);
                let columns = 2 * row.len().max(2) - 1;
                let (mut line, mut flat) =
                    (Vec::with_capacity(columns), Vec::with_capacity(columns));
                for c in 0..columns {
                    let u = c as f32 / (columns - 1) as f32;
                    let d = (u - 0.5).abs() * 2.0;
                    let h = mountain(row, d);
                    line.push(project(u, z, h));
                    flat.push(project(u, z, 0.0));
                }
                (z, line, flat)
            })
            .collect();
        for j in (0..n).rev() {
            let (z, line, flat) = &rows[j];
            if *z < 0.3 {
                continue;
            }
            let shade = fog(*z);
            // Under the mountains, the ground hides what is behind.
            let mut mesh = Mesh::default();
            for c in 0..line.len() - 1 {
                if line[c].y >= flat[c].y && line[c + 1].y >= flat[c + 1].y {
                    continue;
                }
                let i = mesh.vertices.len() as u32;
                for p in [line[c], line[c + 1], flat[c + 1], flat[c]] {
                    mesh.colored_vertex(p, ground);
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            painter.add(mesh);
            // The lines to the row behind.
            if let Some((behind, back, _)) = rows.get(j + 1) {
                let shade = fog((z + behind) / 2.0);
                for (a, b) in line.iter().zip(back) {
                    painter.line_segment(
                        [*a, *b],
                        Stroke::new(1.0, colours.at(0.85, place(*a)).gamma_multiply(shade)),
                    );
                }
            }
            painter.add(Shape::line(
                line.clone(),
                PathStroke::new_uv(1.0, move |_, p| {
                    colours.at(0.85, place(p)).gamma_multiply(shade)
                }),
            ));
        }
    }
}

/// A row's height `d` from the middle (0) to the side (1): its bands
/// mirrored, the lowest in the middle, raised to `POWER`, flat on the road
/// and rising over the slopes beside it.
fn mountain(row: &[f32], d: f32) -> f32 {
    if row.is_empty() {
        return 0.0;
    }
    let at = d.clamp(0.0, 1.0) * (row.len() - 1) as f32;
    let i = (at as usize).min(row.len() - 1);
    let next = (i + 1).min(row.len() - 1);
    let level = row[i] + (row[next] - row[i]) * (at - i as f32);
    let t = ((d - ROAD) / (SLOPES - ROAD)).clamp(0.0, 1.0);
    level.clamp(0.0, 1.0).powf(POWER) * t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_come_in_and_fly_out() {
        let mut s = Synthwave::new();
        let loud = vec![0.8; BANDS];
        // The first sound lays a flat field.
        s.feed(Some((loud.clone(), 0.0)), 0.02);
        assert_eq!(s.rows.len(), ROWS);
        assert!(s.rows.iter().all(|r| r.iter().all(|&l| l == 0.0)));
        // 150 ms: two rows in at the horizon, half the way to a third.
        s.feed(Some((loud.clone(), 0.15)), 0.15);
        assert_eq!(s.rows.len(), ROWS);
        assert_eq!(s.rows[ROWS - 1], loud);
        assert_eq!(s.rows[ROWS - 3], vec![0.0; BANDS]);
        assert!((s.due - 0.5).abs() < 1e-4, "{}", s.due);
        // Paused: nothing moves.
        s.feed(Some((loud, 0.0)), 0.5);
        assert!((s.due - 0.5).abs() < 1e-4);
        // Stopped: flat rows push the mountains out, then all is gone.
        for _ in 0..40 {
            s.feed(None, 0.06);
        }
        assert!(!s.active());
    }

    #[test]
    fn a_road_in_the_middle_mountains_beside_it() {
        let row = vec![1.0; BANDS];
        assert_eq!(mountain(&row, 0.0), 0.0);
        assert_eq!(mountain(&row, ROAD), 0.0);
        assert_eq!(mountain(&row, 0.9), 1.0);
        assert!(mountain(&row, (ROAD + SLOPES) / 2.0) > 0.2);
    }
}
