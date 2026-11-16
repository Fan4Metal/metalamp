//! Cloth: a sheet stretched in a frame, seen in perspective from above
//! its near edge, its threads drawn as a net over a faintly lit surface.
//! The sheet is a wave equation on a grid (each knot pulled by its four
//! neighbours, damped), so whatever moves it sends waves across that
//! reflect from the frame and pass through one another. Three things
//! move it: the spectrum pushes the near edge (the bass in the middle,
//! the treble at the sides, swelling slowly), so each change of the
//! sound sends a front away from the viewer; a beat drops a stone
//! somewhere on it, rings running out from there; the treble rains
//! small drops on it. The higher a knot stands, the brighter its colour;
//! the far rows fade. Paused, all stands; stopped, the waves die down
//! and the sheet goes.

use std::f32::consts::TAU;

use egui::epaint::PathStroke;
use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2};

use super::beat::{Beat, Rng, glow, level_at};
use crate::palette;
use crate::settings::Visual;

/// The bands of the levels fed.
pub const BANDS: usize = 32;
/// The knots across and from the far edge to the near one.
const COLS: usize = 72;
const ROWS: usize = 26;
/// The sheet moves in steps of `STEP_S`, at most `MOST_STEPS` a frame.
const STEP_S: f32 = 1.0 / 120.0;
const MOST_STEPS: usize = 16;
/// Waves run `SPEED` knots a second and die down by `DAMP` a second; a
/// knot goes no further than `LIMIT` either way.
const SPEED: f32 = 30.0;
const DAMP: f32 = 0.9;
const LIMIT: f32 = 2.5;
/// The near edge stands as high as the level at its place (over `FROM`,
/// raised to `POWER`), swelling by `SWELL` at `SWELL_HZ`, and follows it
/// within about `FOLLOW_S`.
const FROM: f32 = 0.3;
const POWER: f32 = 1.5;
const SWELL: f32 = 0.35;
const SWELL_HZ: f32 = 0.6;
const FOLLOW_S: f32 = 0.03;
/// A stone: a blow of `STONE` (at full strength) over `STONE_RADIUS`
/// knots. Rain: up to `RAIN` drops a second as the treble goes from
/// `RAIN_FROM` to full, each a blow of `DROP` on a knot.
const STONE: f32 = 40.0;
const STONE_RADIUS: f32 = 1.8;
const RAIN: f32 = 24.0;
const RAIN_FROM: f32 = 0.4;
const DROP: f32 = 14.0;
/// The far edge lies `HORIZON` of the height under the top; the rows are
/// at depths from `NEAR` to `FAR`, the near one `SPAN` display widths
/// wide; a knot at 1 stands `HEIGHT` of the floor's height over it.
const HORIZON: f32 = 0.12;
const NEAR: f32 = 1.0;
const FAR: f32 = 3.2;
const SPAN: f32 = 1.3;
const HEIGHT: f32 = 0.26;
/// Under this the sheet is still.
const STILL: f32 = 0.004;

pub struct Cloth {
    /// The knots' heights and speeds, a row after another from the far
    /// edge.
    u: Vec<f32>,
    v: Vec<f32>,
    levels: Vec<f32>,
    /// The sound heard, in seconds: the swell's clock.
    time: f32,
    /// Steps and drops owed (fractions carried on).
    owed: f32,
    rain: f32,
    beat: Beat,
    rng: Rng,
    /// Something was heard since the sheet last went still.
    shown: bool,
}

impl Default for Cloth {
    fn default() -> Self {
        Self::new()
    }
}

impl Cloth {
    pub fn new() -> Cloth {
        Cloth {
            u: vec![0.0; COLS * ROWS],
            v: vec![0.0; COLS * ROWS],
            levels: vec![0.0; BANDS],
            time: 0.0,
            owed: 0.0,
            rain: 0.0,
            beat: Beat::default(),
            rng: Rng::default(),
            shown: false,
        }
    }

    /// Moves on with `heard`: the levels heard now (`BANDS`) and the
    /// seconds of sound heard since the frame before (0: paused, all
    /// stands); `None`: nothing is heard, and the waves die down for `dt`
    /// seconds.
    pub fn feed(&mut self, heard: Option<(&[f32], f32)>, dt: f32) {
        match heard {
            Some((levels, seconds)) => {
                if seconds <= 0.0 {
                    return;
                }
                self.shown = true;
                self.levels.clear();
                self.levels.extend_from_slice(levels);
                self.run(true, seconds);
            }
            None if self.shown => {
                self.levels.fill(0.0);
                self.run(false, dt);
                if self.still() {
                    *self = Cloth::new();
                }
            }
            None => {}
        }
    }

    fn run(&mut self, heard: bool, seconds: f32) {
        self.time += seconds;
        let levels = heard.then_some(self.levels.as_slice());
        if let Some(strength) = self.beat.feed(levels, seconds) {
            let i = self.rng.between(0.15, 0.85) * (COLS - 1) as f32;
            let j = self.rng.between(0.3, 0.85) * (ROWS - 1) as f32;
            self.blow(i, j, STONE * strength, STONE_RADIUS);
        }
        let n = self.levels.len();
        let treble = if n > 0 {
            let top = &self.levels[n - n / 3..];
            top.iter().sum::<f32>() / top.len().max(1) as f32
        } else {
            0.0
        };
        self.rain += RAIN * ((treble - RAIN_FROM) / (1.0 - RAIN_FROM)).clamp(0.0, 1.0) * seconds;
        while self.rain >= 1.0 {
            self.rain -= 1.0;
            let i = self.rng.between(0.05, 0.95) * (COLS - 1) as f32;
            let j = self.rng.between(0.1, 0.9) * (ROWS - 1) as f32;
            self.blow(i, j, DROP, 0.7);
        }
        self.owed += seconds / STEP_S;
        let steps = self.owed as usize;
        self.owed -= steps as f32;
        for _ in 0..steps.min(MOST_STEPS) {
            self.step();
        }
    }

    /// A blow down at knot (`i`, `j`) of `strength`, spread over `radius`
    /// knots.
    fn blow(&mut self, i: f32, j: f32, strength: f32, radius: f32) {
        let reach = (radius * 2.5).ceil() as isize;
        let (ci, cj) = (i.round() as isize, j.round() as isize);
        for dj in -reach..=reach {
            for di in -reach..=reach {
                let (x, y) = (ci + di, cj + dj);
                if x < 1 || y < 1 || x >= COLS as isize - 1 || y >= ROWS as isize - 1 {
                    continue;
                }
                let d2 = ((x as f32 - i).powi(2) + (y as f32 - j).powi(2)) / (radius * radius);
                self.v[y as usize * COLS + x as usize] -= strength * (-d2).exp();
            }
        }
    }

    /// The near edge's height at knot `i`: the level at its place, the
    /// bass in the middle, swelling.
    fn edge(&self, i: usize) -> f32 {
        let place = (i as f32 / (COLS - 1) as f32 * 2.0 - 1.0).abs();
        let level = ((level_at(&self.levels, place) - FROM) / (1.0 - FROM)).clamp(0.0, 1.0);
        let swell = 1.0 + SWELL * (TAU * (SWELL_HZ * self.time + place * 1.3)).sin();
        level.powf(POWER) * swell
    }

    fn step(&mut self) {
        let near = (ROWS - 1) * COLS;
        let follow = 1.0 - (-STEP_S / FOLLOW_S).exp();
        for i in 0..COLS {
            let wanted = self.edge(i);
            self.u[near + i] += (wanted - self.u[near + i]) * follow;
        }
        let pull = SPEED * SPEED * STEP_S;
        let damp = 1.0 - DAMP * STEP_S;
        for j in 1..ROWS - 1 {
            for i in 1..COLS - 1 {
                let k = j * COLS + i;
                let u = &self.u;
                let lap = u[k - 1] + u[k + 1] + u[k - COLS] + u[k + COLS] - 4.0 * u[k];
                self.v[k] = (self.v[k] + pull * lap) * damp;
            }
        }
        for j in 1..ROWS - 1 {
            for i in 1..COLS - 1 {
                let k = j * COLS + i;
                self.u[k] = (self.u[k] + self.v[k] * STEP_S).clamp(-LIMIT, LIMIT);
            }
        }
    }

    fn still(&self) -> bool {
        self.u.iter().all(|u| u.abs() < STILL) && self.v.iter().all(|v| (v * STEP_S).abs() < STILL)
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.shown
    }

    /// The sheet from the far row to the near one: the surface between a
    /// row and the one before (hiding what is behind), the threads across
    /// it, then the row's thread, glowing on a dark theme.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        if !self.shown || rect.width() < 16.0 || rect.height() < 16.0 {
            return;
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual_for(Visual::Cloth);
        let dark = colours.is_dark();
        let ground = palette::list_bg();
        let bright = palette::text_bright();
        let kick = self.beat.kick;
        let top = rect.top() + rect.height() * HORIZON;
        let floor = rect.bottom() - 1.0 - top;
        let half = rect.width() * SPAN / 2.0;
        let cx = rect.center().x;
        let far_scale = NEAR / FAR;

        // Each row's scale (1 at the near edge) and its light.
        let scale = |j: usize| {
            let z = NEAR + (FAR - NEAR) * (ROWS - 1 - j) as f32 / (ROWS - 1) as f32;
            NEAR / z
        };
        let nearness = |s: f32| (s - far_scale) / (1.0 - far_scale);
        let mut previous: Option<(Vec<Pos2>, Vec<Color32>)> = None;
        for j in 0..ROWS {
            let s = scale(j);
            let fog = 0.2 + 0.8 * nearness(s).powf(0.7);
            let base = top + floor * nearness(s);
            let mut points = Vec::with_capacity(COLS);
            let mut shades = Vec::with_capacity(COLS);
            for i in 0..COLS {
                let u = self.u[j * COLS + i];
                let across = i as f32 / (COLS - 1) as f32 * 2.0 - 1.0;
                points.push(pos2(cx + across * half * s, base - u * HEIGHT * floor * s));
                let level = (u.abs() * 0.9).min(1.0).sqrt();
                let colour = colours
                    .at(level.max(0.25), across.abs())
                    .lerp_to_gamma(bright, kick * 0.3 * level);
                shades.push(colour.lerp_to_gamma(ground, 1.0 - fog));
            }
            if let Some((back, back_shades)) = &previous {
                // The surface between the row before and this one.
                let mut mesh = Mesh::default();
                for i in 0..COLS {
                    let lit = |c: Color32, u: f32| {
                        ground.lerp_to_gamma(c, 0.08 + 0.3 * (u.abs() * 0.9).min(1.0))
                    };
                    mesh.colored_vertex(back[i], lit(back_shades[i], self.u[(j - 1) * COLS + i]));
                    mesh.colored_vertex(points[i], lit(shades[i], self.u[j * COLS + i]));
                    if i > 0 {
                        let k = 2 * i as u32;
                        mesh.add_triangle(k - 2, k - 1, k);
                        mesh.add_triangle(k - 1, k + 1, k);
                    }
                }
                painter.add(mesh);
                // The threads from the row before to this one.
                let width = 0.5 + 0.4 * s;
                for i in 0..COLS {
                    let colour = back_shades[i]
                        .lerp_to_gamma(shades[i], 0.5)
                        .gamma_multiply(0.55);
                    painter.line_segment([back[i], points[i]], Stroke::new(width, colour));
                }
            }
            let (x0, x1) = (points[0].x, points[COLS - 1].x);
            let along = shades.clone();
            let colour_at = move |p: Pos2| {
                let t = ((p.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0) * (COLS - 1) as f32;
                let k = (t as usize).min(COLS - 2);
                along[k].lerp_to_gamma(along[k + 1], t - k as f32)
            };
            if dark {
                let glow_at = colour_at.clone();
                painter.add(Shape::line(
                    points.clone(),
                    PathStroke::new_uv(1.5 + 3.5 * s, move |_, p| {
                        glow(glow_at(p), 0.18 + 0.2 * kick, true)
                    }),
                ));
            }
            painter.add(Shape::line(
                points.clone(),
                PathStroke::new_uv(0.6 + 0.8 * s, move |_, p| colour_at(p)),
            ));
            previous = Some((points, shades));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn most(c: &Cloth) -> f32 {
        c.u.iter().fold(0.0f32, |m, u| m.max(u.abs()))
    }

    #[test]
    fn a_stone_sends_rings_that_die_down() {
        let mut c = Cloth::new();
        c.shown = true;
        c.blow(36.0, 13.0, STONE, STONE_RADIUS);
        for _ in 0..30 {
            c.feed(Some((&[0.0; BANDS], 1.0 / 60.0)), 1.0 / 60.0);
        }
        // Half a second: the ring has gone out and left the middle.
        let at = |c: &Cloth, i: usize, j: usize| c.u[j * COLS + i];
        assert!(most(&c) > 0.15, "{}", most(&c));
        assert!(at(&c, 36 + 12, 13).abs() > 0.01 || at(&c, 36 - 12, 13).abs() > 0.01);
        // Stopped: it dies down and goes.
        for _ in 0..2000 {
            c.feed(None, 0.05);
        }
        assert!(!c.active());
    }

    #[test]
    fn the_near_edge_stands_with_the_bass_in_the_middle() {
        let mut c = Cloth::new();
        let mut levels = vec![0.2; BANDS];
        levels[..4].fill(1.0);
        for _ in 0..30 {
            c.feed(Some((&levels, 0.02)), 0.02);
        }
        let near = (ROWS - 1) * COLS;
        assert!(c.u[near + COLS / 2] > 0.5, "{}", c.u[near + COLS / 2]);
        assert!(c.u[near] < 0.05);
        // Paused: all stands.
        let before = c.u.clone();
        c.feed(Some((&levels, 0.0)), 0.5);
        assert_eq!(c.u, before);
    }

    #[test]
    fn loud_music_keeps_it_within_bounds() {
        let mut c = Cloth::new();
        let mut rng = Rng::default();
        for k in 0..3000 {
            // Beats every half second, noise between.
            let mut levels: Vec<f32> = (0..BANDS).map(|_| rng.between(0.4, 0.95)).collect();
            if k % 25 == 0 {
                levels[..6].fill(1.0);
            } else {
                levels[..6].fill(0.4);
            }
            c.feed(Some((&levels, 0.02)), 0.02);
            assert!(c.u.iter().chain(&c.v).all(|x| x.is_finite()));
        }
        assert!(most(&c) < LIMIT, "{}", most(&c));
        assert!(most(&c) > 0.1);
    }
}
