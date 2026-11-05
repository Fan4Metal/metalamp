//! Sparks: thrown up from the display's bottom as from a grinder, each
//! band of the analyzer throwing them at its place across (the bass on
//! the left), more and higher the louder it is; every beat throws a
//! sheaf of them in a fan from a place or two. They fly as thrown things
//! do, falling back, bouncing once or twice off the bottom, and cool from
//! white through the visualization's colour into nothing. Each is drawn
//! as a short streak along its flight with a faint glow under it, the
//! glow added to the ground on a dark theme. Paused, they stand; stopped,
//! the last of them fall and go out.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Vec2, pos2, vec2};

use super::beat::{Beat, Rng, glow};
use crate::palette;

/// The bands of the levels fed.
pub const BANDS: usize = 24;
/// A band at full level throws this many sparks a second; under `FROM`
/// none.
const RATE: f32 = 70.0;
const FROM: f32 = 0.3;
/// A spark thrown at full level rises to the display's top in `RISE_S`.
const RISE_S: f32 = 0.5;
/// A beat throws this many sparks at full strength from each of its
/// places, faster than the bands' by `BURST_SPEED`.
const BURST: f32 = 45.0;
const BURST_SPEED: f32 = 1.2;
/// The sparks kept at most (the oldest go).
const MOST: usize = 1500;
/// A spark lives this long at most, a bounce taking this share of the
/// height of its fall, and of its speed across.
const LIFE_S: [f32; 2] = [0.5, 1.3];
const BOUNCE: f32 = 0.35;
const BOUNCE_ACROSS: f32 = 0.6;
/// A spark's streak is as long as its flight in this long, at most
/// `STREAK_MOST` points.
const STREAK_S: f32 = 0.03;
const STREAK_MOST: f32 = 14.0;
/// The time step of the flight at most (a slow frame takes several).
const STEP_S: f32 = 0.02;

struct Spark {
    /// From the display's top left, in points; points a second.
    at: Pos2,
    speed: Vec2,
    age: f32,
    life: f32,
    /// Its place across where it was thrown, for its colour.
    place: f32,
}

pub struct Sparks {
    sparks: Vec<Spark>,
    /// Sparks owed to each band (fractions carried on).
    owed: Vec<f32>,
    beat: Beat,
    rng: Rng,
}

impl Default for Sparks {
    fn default() -> Self {
        Self::new()
    }
}

impl Sparks {
    pub fn new() -> Sparks {
        Sparks {
            sparks: Vec::new(),
            owed: vec![0.0; BANDS],
            beat: Beat::default(),
            rng: Rng::default(),
        }
    }

    /// Moves on by `dt` seconds with the levels heard now (`None`:
    /// nothing; `moved` false: paused, they stand) in a display of `size`.
    pub fn feed(&mut self, levels: Option<&[f32]>, moved: bool, dt: f32, size: Vec2) {
        if size.x < 8.0 || size.y < 8.0 {
            self.sparks.clear();
            return;
        }
        if levels.is_some() && !moved {
            return;
        }
        // Thrown up to the top in `RISE_S` at full level.
        let gravity = 2.0 * size.y / (RISE_S * RISE_S);
        let speed = gravity * RISE_S;
        let mut left = dt;
        while left > 0.0 {
            let step = left.min(STEP_S);
            left -= step;
            for s in &mut self.sparks {
                s.speed.y += gravity * step;
                s.at += s.speed * step;
                s.age += step;
                if s.at.y > size.y && s.speed.y > 0.0 {
                    s.at.y = size.y;
                    s.speed.y *= -BOUNCE;
                    s.speed.x *= BOUNCE_ACROSS;
                }
            }
        }
        self.sparks
            .retain(|s| s.age < s.life && s.at.x > -STREAK_MOST && s.at.x < size.x + STREAK_MOST);

        let struck = self.beat.feed(levels, dt);
        if let Some(levels) = levels {
            let n = levels.len().min(BANDS);
            for (band, &level) in levels.iter().take(n).enumerate() {
                let l = ((level - FROM) / (1.0 - FROM)).clamp(0.0, 1.0);
                self.owed[band] += RATE * l * l * dt;
                while self.owed[band] >= 1.0 {
                    self.owed[band] -= 1.0;
                    let place = (band as f32 + self.rng.unit()) / n as f32;
                    let up = speed * (0.55 + 0.45 * l) * self.rng.between(0.7, 1.05);
                    let across = speed * self.rng.between(-0.3, 0.3);
                    self.throw(pos2(place * size.x, size.y), vec2(across, -up), place);
                }
            }
        }
        if let Some(strength) = struck {
            let places = 1 + (self.rng.next() & 1) as usize;
            for _ in 0..places {
                let place = self.rng.between(0.1, 0.9);
                for _ in 0..(BURST * strength) as usize {
                    let angle = self.rng.between(-2.6, -0.55);
                    let v = speed * BURST_SPEED * strength * self.rng.between(0.5, 1.0);
                    self.throw(
                        pos2(place * size.x, size.y),
                        vec2(angle.cos(), angle.sin()) * v,
                        place,
                    );
                }
            }
        }
        if self.sparks.len() > MOST {
            let over = self.sparks.len() - MOST;
            self.sparks.drain(..over);
        }
    }

    fn throw(&mut self, at: Pos2, speed: Vec2, place: f32) {
        let life = self.rng.between(LIFE_S[0], LIFE_S[1]);
        self.sparks.push(Spark {
            at,
            speed,
            age: 0.0,
            life,
            place,
        });
    }

    /// Some still fly: keep repainting.
    pub fn active(&self) -> bool {
        !self.sparks.is_empty() || self.beat.level > 0.0
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual();
        let dark = colours.is_dark();
        let hot = Color32::from_rgb(255, 247, 225);
        let mut mesh = Mesh::default();
        for s in &self.sparks {
            let t = s.age / s.life;
            let colour = colours.at(1.0 - 0.6 * t, s.place);
            let colour = if dark && t < 0.35 {
                hot.lerp_to_gamma(colour, t / 0.35)
            } else {
                colour
            };
            let fade = (1.0 - t).powf(1.5);
            let head = rect.min + s.at.to_vec2();
            let tail_by = s.speed * STREAK_S;
            let tail = head - tail_by * (STREAK_MOST / tail_by.length().max(STREAK_MOST)).min(1.0);
            segment(&mut mesh, tail, head, 3.5, glow(colour, 0.22 * fade, dark));
            segment(&mut mesh, tail, head, 1.3, glow(colour, fade, dark));
        }
        painter.add(mesh);
    }
}

/// A line from `a` to `b` `width` wide as two triangles.
fn segment(mesh: &mut Mesh, a: Pos2, b: Pos2, width: f32, colour: Color32) {
    let along = b - a;
    let length = along.length();
    let way = if length > 0.01 {
        along / length
    } else {
        vec2(1.0, 0.0)
    };
    // A streak too short to see is a dot.
    let (a, b) = if length < width {
        let mid = a + along / 2.0;
        (mid - way * (width / 2.0), mid + way * (width / 2.0))
    } else {
        (a, b)
    };
    let normal = way.rot90() * (width / 2.0);
    let i = mesh.vertices.len() as u32;
    for p in [a + normal, b + normal, b - normal, a - normal] {
        mesh.colored_vertex(p, colour);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparks_fly_up_fall_back_and_go_out() {
        let mut s = Sparks::new();
        let size = vec2(400.0, 100.0);
        let loud = vec![0.9; BANDS];
        for _ in 0..25 {
            s.feed(Some(&loud), true, 0.02, size);
        }
        assert!(s.sparks.len() > 50, "{}", s.sparks.len());
        // Up into the display, none past its bottom.
        assert!(s.sparks.iter().any(|p| p.at.y < size.y * 0.5));
        assert!(s.sparks.iter().all(|p| p.at.y <= size.y + 0.01));
        // Paused, they stand.
        let before: Vec<Pos2> = s.sparks.iter().map(|p| p.at).collect();
        s.feed(Some(&loud), false, 0.02, size);
        assert_eq!(before, s.sparks.iter().map(|p| p.at).collect::<Vec<_>>());
        // Quiet bands throw none; stopped, they all go out.
        for _ in 0..100 {
            s.feed(None, true, 0.02, size);
        }
        assert!(!s.active());
        let quiet = vec![0.25; BANDS];
        for _ in 0..25 {
            s.feed(Some(&quiet), true, 0.02, size);
        }
        assert!(s.sparks.is_empty());
    }

    #[test]
    fn a_beat_throws_a_sheaf() {
        let mut s = Sparks::new();
        let size = vec2(400.0, 100.0);
        let quiet = vec![0.1; BANDS];
        for _ in 0..30 {
            s.feed(Some(&quiet), true, 0.02, size);
        }
        assert!(s.sparks.is_empty());
        let mut beat = vec![0.1; BANDS];
        beat[..BANDS / 6].fill(0.9);
        s.feed(Some(&beat), true, 0.02, size);
        assert!(s.sparks.len() >= 20, "{}", s.sparks.len());
    }

    #[test]
    fn the_sparks_kept_are_bounded() {
        let mut s = Sparks::new();
        let size = vec2(2000.0, 100.0);
        let loud = vec![1.0; BANDS];
        for _ in 0..400 {
            s.feed(Some(&loud), true, 0.05, size);
        }
        assert!(s.sparks.len() <= MOST);
    }
}
