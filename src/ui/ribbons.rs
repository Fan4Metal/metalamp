//! Ribbons: three ribbons of light, the bass, the middle and the treble,
//! flowing across the display and winding through one another. Each
//! bends along a wave running right and turns about its length, showing
//! its face, then its edge, then its back (darker), its face catching a
//! sheen as silk does. Its band's level comes in on the left and flows
//! right along it (as the strata's), widening it, swinging it further
//! and lighting it. A beat sends a sharp bend running from the left
//! along all three, the bass's most. On a dark theme their light adds
//! up where they cross. Paused, all stands; stopped, they fade away.

use std::collections::VecDeque;
use std::f32::consts::TAU;

use egui::epaint::PathStroke;
use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Vec2, pos2, vec2};

use super::beat::{Beat, glow};
use super::visual::smoothed;
use crate::palette;
use crate::settings::Visual;

/// The bands of the levels fed; the ribbons' shares of them.
pub const BANDS: usize = 24;
const SHARES: [(usize, usize); 3] = [(0, 4), (5, 13), (14, 24)];
/// A band's level counts from `FROM`, raised to `POWER`.
const FROM: f32 = 0.3;
const POWER: f32 = 1.2;
/// A column of levels comes in every `COLUMN_S` and flows `PITCH` points
/// right; a point of the ribbons every `STEP` points.
const COLUMN_S: f32 = 0.02;
const PITCH: f32 = 3.0;
const STEP: f32 = 3.0;
/// Each ribbon, the bass first: its wave's length (display widths) and
/// speed (widths a second), its turn's length and speed (turns a
/// second), its swing at full level (a share of the height), and how
/// much a beat's bend moves it.
const RIBBONS: [Ribbon; 3] = [
    Ribbon {
        wave: 1.3,
        speed: 0.12,
        turn: 0.9,
        turn_hz: 0.25,
        swing: 0.3,
        bend: 1.0,
    },
    Ribbon {
        wave: 0.75,
        speed: 0.2,
        turn: 0.55,
        turn_hz: 0.4,
        swing: 0.24,
        bend: 0.65,
    },
    Ribbon {
        wave: 0.45,
        speed: 0.32,
        turn: 0.35,
        turn_hz: 0.6,
        swing: 0.18,
        bend: 0.4,
    },
];
/// A ribbon's half width: `THIN` of the height, and `WIDE` more at full
/// level; it swings `CALM` of its swing at no level.
const THIN: f32 = 0.025;
const WIDE: f32 = 0.085;
const CALM: f32 = 0.35;
/// A beat's bend: `BEND` of the height at full strength, `BEND_WIDTH` of
/// the width wide, running `BEND_SPEED` widths a second, gone past
/// `BEND_END`.
const BEND: f32 = 0.28;
const BEND_WIDTH: f32 = 0.07;
const BEND_SPEED: f32 = 1.1;
const BEND_END: f32 = 1.3;
/// The ribbons come in over `APPEAR_S` and fade over `FADE_S` once
/// nothing is heard.
const APPEAR_S: f32 = 0.4;
const FADE_S: f32 = 1.0;

struct Ribbon {
    wave: f32,
    speed: f32,
    turn: f32,
    turn_hz: f32,
    swing: f32,
    bend: f32,
}

pub struct Ribbons {
    /// The ribbons' levels, newest first (at the left edge).
    columns: VecDeque<[f32; 3]>,
    /// The share of the next column already in.
    due: f32,
    /// The sound heard, in seconds: the waves' clock.
    time: f32,
    beat: Beat,
    /// The bends running: their place (a share of the width), strength
    /// and way (up or down).
    bends: Vec<(f32, f32, f32)>,
    /// How much the ribbons show, 0–1.
    presence: f32,
}

impl Default for Ribbons {
    fn default() -> Self {
        Self::new()
    }
}

impl Ribbons {
    pub fn new() -> Ribbons {
        Ribbons {
            columns: VecDeque::new(),
            due: 0.0,
            time: 0.0,
            beat: Beat::default(),
            bends: Vec::new(),
            presence: 0.0,
        }
    }

    /// Moves on with `heard`: the levels heard now (`BANDS`) and the
    /// seconds of sound heard since the frame before (0: paused, all
    /// stands); `None`: nothing is heard, and the ribbons fade for `dt`
    /// seconds. `width` is the display's.
    pub fn feed(&mut self, heard: Option<(&[f32], f32)>, dt: f32, width: f32) {
        let cap = ((width / PITCH) as usize).clamp(8, 2048) + 2;
        let (column, seconds) = match heard {
            Some((levels, seconds)) => {
                if seconds <= 0.0 {
                    return;
                }
                self.presence = (self.presence + seconds / APPEAR_S).min(1.0);
                (shares(levels), seconds)
            }
            None if self.presence > 0.0 => {
                self.presence = (self.presence - dt / FADE_S).max(0.0);
                if self.presence <= 0.0 {
                    *self = Ribbons::new();
                    return;
                }
                ([0.0; 3], dt)
            }
            None => return,
        };
        if self.columns.is_empty() {
            self.columns = VecDeque::from(vec![[0.0; 3]; cap]);
        }
        self.time += seconds;
        for bend in &mut self.bends {
            bend.0 += BEND_SPEED * seconds;
        }
        self.bends.retain(|b| b.0 < BEND_END);
        let levels = heard.map(|(l, _)| l);
        if let Some(strength) = self.beat.feed(levels, seconds) {
            // Up and down by turns.
            let way = if self.bends.len().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            self.bends.push((0.0, strength, way));
        }
        self.due += seconds / COLUMN_S;
        let new = self.due as usize;
        self.due -= new as f32;
        for _ in 0..new.min(cap) {
            self.columns.push_front(column);
        }
        while self.columns.len() > cap {
            self.columns.pop_back();
        }
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.presence > 0.0
    }

    /// Ribbon `r`'s middle line's height off the display's middle (in
    /// heights, up) at `u` across (a share of the width), at `level`
    /// there: its wave and the bends passing.
    fn rise(&self, r: usize, u: f32, level: f32) -> f32 {
        let ribbon = &RIBBONS[r];
        let phase = r as f32 * 2.1;
        let wave = 0.7 * (TAU * (u / ribbon.wave - ribbon.speed * self.time) + phase).sin()
            + 0.3
                * (TAU * (u * 2.3 / ribbon.wave + ribbon.speed * 1.7 * self.time) + phase * 1.6)
                    .sin();
        let bends: f32 = self
            .bends
            .iter()
            .map(|&(at, strength, way)| {
                let d = (u - at) / BEND_WIDTH;
                way * strength * (-d * d).exp() * (1.0 - 0.5 * at / BEND_END)
            })
            .sum();
        ribbon.swing * (CALM + (1.0 - CALM) * level) * wave + BEND * ribbon.bend * bends
    }

    /// How ribbon `r` is turned at `u`: 1 its face, 0 its edge, −1 its
    /// back.
    fn facing(&self, r: usize, u: f32) -> f32 {
        let ribbon = &RIBBONS[r];
        (TAU * (u / ribbon.turn - ribbon.turn_hz * self.time) + r as f32).cos()
    }

    /// The ribbons, the treble at the back: each a strip of its edges and
    /// its middle (lit as a rounded sheen), its edges as thin lines.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        if self.presence <= 0.0 || self.columns.is_empty() || rect.width() < 16.0 {
            return;
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual_for(Visual::Ribbons);
        let dark = colours.is_dark();
        let (w, h) = (rect.width(), rect.height());
        let middle = rect.center().y;
        let kick = self.beat.kick;
        let count = (w / STEP) as usize + 1;
        for r in (0..3).rev() {
            let place = r as f32 / 2.0;
            let levels: Vec<f32> = smoothed(&self.columns.iter().map(|c| c[r]).collect::<Vec<_>>());
            // The level at `x` points from the left: the columns flow
            // right, a share of a column past their places.
            let level_at = |x: f32| {
                let i = (x / PITCH - self.due).max(0.0);
                let k = (i as usize).min(levels.len() - 1);
                let next = (k + 1).min(levels.len() - 1);
                levels[k] + (levels[next] - levels[k]) * (i - k as f32).min(1.0)
            };
            let samples: Vec<(Pos2, f32, f32)> = (0..count)
                .map(|i| {
                    let x = (i as f32 * STEP).min(w);
                    let u = x / w;
                    let level = level_at(x);
                    let y = middle - self.rise(r, u, level) * h;
                    (pos2(rect.left() + x, y), level, self.facing(r, u))
                })
                .collect();
            let mut mesh = Mesh::default();
            let mut tops = Vec::with_capacity(count);
            let mut bottoms = Vec::with_capacity(count);
            let mut shades = Vec::with_capacity(count);
            for (i, &(p, level, facing)) in samples.iter().enumerate() {
                // The normal from the neighbours' slope.
                let before = samples[i.saturating_sub(1)].0;
                let after = samples[(i + 1).min(count - 1)].0;
                let along = (after - before).normalized();
                let normal = if along == Vec2::ZERO {
                    vec2(0.0, -1.0)
                } else {
                    vec2(along.y, -along.x)
                };
                let half = h * (THIN + WIDE * level) * facing;
                let (top, bottom) = (p + normal * half, p - normal * half);
                let face = facing.abs();
                let mut shade = (0.3 + 0.7 * face) * if facing < 0.0 { 0.55 } else { 1.0 };
                shade = (shade + kick * 0.3 * RIBBONS[r].bend).min(1.2) * self.presence;
                let colour = colours.at(0.4 + 0.6 * level, place).lerp_to_gamma(
                    colours.peak(place),
                    if facing > 0.0 {
                        face.powi(8) * 0.45
                    } else {
                        0.0
                    },
                );
                let paint = |light: f32| {
                    if dark {
                        glow(colour, 0.55 * light, true)
                    } else {
                        colour.gamma_multiply((0.7 * light).min(1.0))
                    }
                };
                mesh.colored_vertex(top, paint(shade * 0.55));
                mesh.colored_vertex(p, paint(shade));
                mesh.colored_vertex(bottom, paint(shade * 0.55));
                if i > 0 {
                    let k = 3 * i as u32;
                    for (a, b) in [(k - 3, k), (k - 2, k + 1)] {
                        mesh.add_triangle(a, a + 1, b);
                        mesh.add_triangle(a + 1, b + 1, b);
                    }
                }
                tops.push(top);
                bottoms.push(bottom);
                shades.push(paint(shade.min(1.0) * 1.3));
            }
            painter.add(mesh);
            // The edges, coloured along as the strip is.
            let (x0, x1) = (rect.left(), rect.left() + (count - 1) as f32 * STEP);
            let along = std::sync::Arc::new(shades);
            for edge in [tops, bottoms] {
                let along = along.clone();
                let colour_at = move |p: Pos2| -> Color32 {
                    let t = ((p.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0)
                        * (along.len() - 1) as f32;
                    let k = (t as usize).min(along.len().saturating_sub(2));
                    along[k].lerp_to_gamma(along[(k + 1).min(along.len() - 1)], t - k as f32)
                };
                painter.add(Shape::line(
                    edge,
                    PathStroke::new_uv(0.9, move |_, p| colour_at(p)),
                ));
            }
        }
    }
}

/// The three ribbons' levels: the mean of each one's bands, over `FROM`,
/// raised to `POWER`.
fn shares(levels: &[f32]) -> [f32; 3] {
    SHARES.map(|(from, to)| {
        let to = to.min(levels.len());
        if from >= to {
            return 0.0;
        }
        let mean = levels[from..to].iter().sum::<f32>() / (to - from) as f32;
        ((mean - FROM) / (1.0 - FROM)).clamp(0.0, 1.0).powf(POWER)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f32 = 400.0;

    #[test]
    fn the_levels_flow_in_from_the_left() {
        let mut r = Ribbons::new();
        let mut loud = [0.2; BANDS];
        loud[..4].fill(1.0);
        r.feed(Some((&loud, 0.1)), 0.1, WIDTH);
        assert!(r.columns[0][0] > 0.9 && r.columns[0][2] == 0.0);
        // A second later the loud columns have flowed 150 points right.
        let quiet = [0.2; BANDS];
        r.feed(Some((&quiet, 1.0)), 1.0, WIDTH);
        assert_eq!(r.columns[0][0], 0.0);
        assert!(r.columns[50][0] > 0.9);
        // Paused: all stands.
        let time = r.time;
        r.feed(Some((&quiet, 0.0)), 0.5, WIDTH);
        assert_eq!(r.time, time);
    }

    #[test]
    fn a_loud_ribbon_swings_further() {
        let r = Ribbons::new();
        let most = |level: f32| {
            (0..100)
                .map(|i| r.rise(0, i as f32 / 100.0, level).abs())
                .fold(0.0f32, f32::max)
        };
        assert!(most(1.0) > most(0.0) * 2.0);
        // It turns: face, edge, back along it.
        let facings: Vec<f32> = (0..100).map(|i| r.facing(0, i as f32 / 100.0)).collect();
        assert!(facings.iter().any(|&f| f > 0.9) && facings.iter().any(|&f| f < -0.9));
    }

    #[test]
    fn a_beat_bends_them_from_the_left_and_the_bend_runs_off() {
        let mut r = Ribbons::new();
        for _ in 0..30 {
            r.feed(Some((&[0.1; BANDS], 0.02)), 0.02, WIDTH);
        }
        let mut kick = [0.3; BANDS];
        kick[..4].fill(0.95);
        r.feed(Some((&kick, 0.02)), 0.02, WIDTH);
        assert_eq!(r.bends.len(), 1);
        r.feed(Some((&[0.1; BANDS], 0.3)), 0.3, WIDTH);
        let at = r.bends[0].0;
        let calm = Ribbons {
            time: r.time,
            ..Ribbons::new()
        };
        assert!((r.rise(0, at, 0.0) - calm.rise(0, at, 0.0)).abs() > 0.15);
        for _ in 0..60 {
            r.feed(Some((&[0.1; BANDS], 0.02)), 0.02, WIDTH);
        }
        assert!(r.bends.is_empty());
    }

    #[test]
    fn stopped_they_fade_away() {
        let mut r = Ribbons::new();
        for _ in 0..30 {
            r.feed(Some((&[0.8; BANDS], 0.02)), 0.02, WIDTH);
        }
        assert!(r.active());
        for _ in 0..30 {
            r.feed(None, 0.05, WIDTH);
        }
        assert!(!r.active());
        assert!(r.columns.is_empty());
    }
}
