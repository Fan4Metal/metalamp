//! Lightning: a glowing core in the middle of the display that swells
//! with the bass; faint arcs flickering from it to both ends while there
//! is sound, as strong as it is loud, drawn anew every `ARC_S`; and on
//! each beat (the bass rising over its recent mean) bright bolts striking
//! from it to the display's edges, with branches, flashing and fading
//! over a quarter to half a second; a loud passage without beats strikes
//! now and then too. A bolt is a path of midpoint displacement: each
//! span's middle moved across it by a random share of its length, `DEPTH`
//! times over. Fed the analyzer's levels (`spectrum::Analyzer::levels`).

use egui::epaint::PathStroke;
use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Vec2, pos2, vec2};

use crate::palette;

/// The bands of the levels fed.
pub const BANDS: usize = 32;
/// The arcs are drawn anew this often; under this overall level there
/// are none.
const ARC_S: f32 = 0.07;
const ARC_FROM: f32 = 0.15;
/// A beat: the bass this far over its mean of the last `MEAN_S`, and at
/// least `STRIKE_LEAST`, `STRIKE_GAP_S` after the last strike; a passage
/// louder than `LOUD` strikes every `LOUD_S` without beats.
const STRIKE_RISE: f32 = 0.06;
const STRIKE_LEAST: f32 = 0.3;
const STRIKE_GAP_S: f32 = 0.12;
const MEAN_S: f32 = 0.4;
const LOUD: f32 = 0.55;
const LOUD_S: f32 = 0.7;
/// The core's and the arcs' levels fall the whole way in this long.
const FALL_S: f32 = 0.3;
/// How many times a bolt's spans are halved, and how far a span's middle
/// moves across it, as a share of its length.
const DEPTH: u32 = 5;
const ROUGHNESS: f32 = 0.22;
/// A strike's flash over the whole display lasts this long.
const FLASH_S: f32 = 0.08;

struct Bolt {
    /// The main path, then its branches, in points from the display's top
    /// left.
    paths: Vec<Vec<Pos2>>,
    age: f32,
    life: f32,
    strength: f32,
    /// An arc: thinner, no flash.
    arc: bool,
}

pub struct Lightning {
    rng: u64,
    bolts: Vec<Bolt>,
    /// The bass and the overall level shown (falling at `FALL_S`).
    bass: f32,
    level: f32,
    /// The bass's recent mean.
    mean: f32,
    since_strike: f32,
    since_arcs: f32,
    /// The side the next bolt strikes: the right one when true.
    right: bool,
}

impl Default for Lightning {
    fn default() -> Self {
        Self::new()
    }
}

impl Lightning {
    pub fn new() -> Lightning {
        Lightning {
            rng: 0x9e37_79b9_7f4a_7c15,
            bolts: Vec::new(),
            bass: 0.0,
            level: 0.0,
            mean: 0.0,
            since_strike: 0.0,
            since_arcs: 0.0,
            right: true,
        }
    }

    /// Moves on by `dt` seconds with the levels heard now (`None`:
    /// nothing) in a display of `size`.
    pub fn feed(&mut self, levels: Option<&[f32]>, dt: f32, size: Vec2) {
        for b in &mut self.bolts {
            b.age += dt;
        }
        self.bolts.retain(|b| b.age < b.life);
        self.since_strike += dt;
        self.since_arcs += dt;
        let (bass, level) = levels.filter(|l| !l.is_empty()).map_or((0.0, 0.0), |l| {
            let low = (l.len() / 6).max(1);
            let mean = |s: &[f32]| s.iter().sum::<f32>() / s.len() as f32;
            (mean(&l[..low]), mean(l))
        });
        self.bass = bass.max(self.bass - dt / FALL_S).max(0.0);
        self.level = level.max(self.level - dt / FALL_S).max(0.0);
        let rise = bass - self.mean;
        self.mean += (bass - self.mean) * (dt / MEAN_S).min(1.0);
        if levels.is_none() || size.x < 8.0 || size.y < 8.0 {
            return;
        }
        let beat = rise > STRIKE_RISE && bass > STRIKE_LEAST && self.since_strike > STRIKE_GAP_S;
        let loud = level > LOUD && self.since_strike > LOUD_S;
        if beat || loud {
            let strength = if beat {
                (0.5 + (rise - STRIKE_RISE) * 8.0).clamp(0.4, 1.0)
            } else {
                0.4
            };
            self.strike(size, strength);
            self.since_strike = 0.0;
        }
        if self.since_arcs >= ARC_S {
            self.since_arcs = 0.0;
            self.bolts.retain(|b| !b.arc);
            if level > ARC_FROM {
                for right in [false, true] {
                    self.arc(size, right, level);
                }
            }
        }
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        !self.bolts.is_empty() || self.bass > 0.0 || self.level > 0.0
    }

    /// Bolts from the middle to the edges, more of them the stronger.
    fn strike(&mut self, size: Vec2, strength: f32) {
        let centre = pos2(size.x / 2.0, size.y / 2.0);
        for _ in 0..1 + (strength * 2.0) as usize {
            self.right = !self.right;
            let side = if self.right { 1.0 } else { -1.0 };
            // Mostly to the end of the display, else to its top or bottom
            // edge far from the middle.
            let target = if self.random() < 0.6 {
                pos2(
                    centre.x + side * size.x / 2.0,
                    size.y * (0.1 + 0.8 * self.random()),
                )
            } else {
                let y = if self.random() < 0.5 { 0.0 } else { size.y };
                pos2(centre.x + side * size.x * (0.25 + 0.23 * self.random()), y)
            };
            let main = self.path(centre, target, DEPTH, ROUGHNESS);
            let mut paths = vec![main];
            for _ in 0..1 + (self.random() * 2.0) as usize {
                let main = &paths[0];
                let at = (main.len() as f32 * (0.25 + 0.45 * self.random())) as usize;
                let from = main[at.min(main.len() - 1)];
                let way = target - from;
                let turn = (20.0 + 30.0 * self.random()).to_radians()
                    * if self.random() < 0.5 { -1.0 } else { 1.0 };
                let (sin, cos) = turn.sin_cos();
                let length = 0.3 + 0.2 * self.random();
                let to = from + vec2(way.x * cos - way.y * sin, way.x * sin + way.y * cos) * length;
                let branch = self.path(from, to, DEPTH - 1, ROUGHNESS);
                paths.push(branch);
            }
            self.bolts.push(Bolt {
                paths,
                age: 0.0,
                life: 0.25 + 0.25 * strength,
                strength,
                arc: false,
            });
        }
    }

    /// A faint arc from the middle to one end.
    fn arc(&mut self, size: Vec2, right: bool, level: f32) {
        let centre = pos2(size.x / 2.0, size.y / 2.0);
        let x = if right { size.x } else { 0.0 };
        let target = pos2(x, centre.y + size.y * 0.6 * (self.random() - 0.5));
        let path = self.path(centre, target, DEPTH, ROUGHNESS * 0.8);
        self.bolts.push(Bolt {
            paths: vec![path],
            age: 0.0,
            life: ARC_S * 1.6,
            strength: (level * 0.7).min(0.6),
            arc: true,
        });
    }

    /// A jagged path from `a` to `b`: its spans halved `depth` times,
    /// each middle moved across its span by up to `roughness` of it.
    fn path(&mut self, a: Pos2, b: Pos2, depth: u32, roughness: f32) -> Vec<Pos2> {
        let mut points = vec![a, b];
        for _ in 0..depth {
            let mut next = Vec::with_capacity(points.len() * 2);
            for w in points.windows(2) {
                let (p, q) = (w[0], w[1]);
                let span = q - p;
                let across = vec2(-span.y, span.x);
                let shift = (self.random() * 2.0 - 1.0) * roughness;
                next.push(p);
                next.push(p + span / 2.0 + across * shift);
            }
            next.push(b);
            points = next;
        }
        points
    }

    /// 0 to 1 (xorshift).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual();
        let centre = rect.center();
        let place = move |p: Pos2| ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);

        // The glow across the display, brighter with the bass, and a
        // flash of a fresh strike over it.
        let flash = self
            .bolts
            .iter()
            .filter(|b| !b.arc && b.age < FLASH_S)
            .map(|b| b.strength * (1.0 - b.age / FLASH_S))
            .fold(0.0, f32::max);
        let glow = colours
            .at(1.0, 0.5)
            .gamma_multiply(0.03 + 0.15 * self.bass + 0.12 * flash);
        let clear = Color32::TRANSPARENT;
        let mut mesh = Mesh::default();
        for (x0, x1, c0, c1) in [
            (rect.left(), centre.x, clear, glow),
            (centre.x, rect.right(), glow, clear),
        ] {
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(pos2(x0, rect.top()), c0);
            mesh.colored_vertex(pos2(x1, rect.top()), c1);
            mesh.colored_vertex(pos2(x1, rect.bottom()), c1);
            mesh.colored_vertex(pos2(x0, rect.bottom()), c0);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        painter.add(mesh);

        // The bolts: a wide faint glow under a thin bright core, the
        // core's colour drawn towards the brightest text.
        let bright = palette::text_bright();
        for b in &self.bolts {
            let fade = 1.0 - b.age / b.life;
            let alpha = if b.arc {
                b.strength * fade
            } else if b.age < FLASH_S / 2.0 {
                b.strength
            } else {
                b.strength * fade.powf(1.5)
            };
            for (k, path) in b.paths.iter().enumerate() {
                let width = if b.arc || k > 0 { 0.6 } else { 1.0 };
                let points: Vec<Pos2> = path.iter().map(|p| rect.min + p.to_vec2()).collect();
                painter.add(Shape::line(
                    points.clone(),
                    PathStroke::new_uv(6.0 * width, move |_, p| {
                        colours.at(1.0, place(p)).gamma_multiply(0.25 * alpha)
                    }),
                ));
                painter.add(Shape::line(
                    points,
                    PathStroke::new_uv(1.6 * width, move |_, p| {
                        colours
                            .at(1.0, place(p))
                            .lerp_to_gamma(bright, 0.35)
                            .gamma_multiply(alpha)
                    }),
                ));
            }
        }

        // The core: a glow fading from its middle, a bright heart in it.
        let r = 4.0 + 10.0 * self.bass + 3.0 * flash;
        let heart = colours.at(1.0, 0.5).lerp_to_gamma(bright, 0.6);
        let mut mesh = Mesh::default();
        mesh.colored_vertex(
            centre,
            heart.gamma_multiply(0.5 + 0.5 * self.level.max(self.bass)),
        );
        const STEPS: u32 = 32;
        for i in 0..=STEPS {
            let a = i as f32 / STEPS as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(centre + vec2(a.cos(), a.sin()) * r * 2.2, clear);
            if i > 0 {
                mesh.add_triangle(0, i, i + 1);
            }
        }
        painter.add(mesh);
        if self.level.max(self.bass) > 0.0 {
            painter.circle_filled(centre, (r * 0.35).max(1.5), heart);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beat_strikes_and_silence_fades_out() {
        let mut l = Lightning::new();
        let size = vec2(600.0, 90.0);
        // Too quiet for arcs, and no beat.
        let quiet = vec![0.1; BANDS];
        for _ in 0..20 {
            l.feed(Some(&quiet), 0.02, size);
        }
        assert!(l.bolts.is_empty());
        // The bass jumps: bolts from the middle to an edge.
        let mut beat = vec![0.3; BANDS];
        beat[..BANDS / 6].fill(0.9);
        l.feed(Some(&beat), 0.02, size);
        let strikes: Vec<&Bolt> = l.bolts.iter().filter(|b| !b.arc).collect();
        assert!(!strikes.is_empty());
        for b in strikes {
            let main = &b.paths[0];
            assert_eq!(main.len(), (1 << DEPTH) + 1);
            assert_eq!(main[0], pos2(300.0, 45.0));
            let end = main[main.len() - 1];
            assert!(
                end.x == 0.0 || end.x == 600.0 || end.y == 0.0 || end.y == 90.0,
                "{end:?}"
            );
            assert!(b.paths.len() >= 2);
        }
        // Loud enough for arcs to both ends once they are drawn anew.
        for _ in 0..4 {
            l.feed(Some(&beat), 0.02, size);
        }
        assert_eq!(l.bolts.iter().filter(|b| b.arc).count(), 2);
        // Nothing heard for a second: all gone.
        for _ in 0..50 {
            l.feed(None, 0.02, size);
        }
        assert!(!l.active());
    }
}
