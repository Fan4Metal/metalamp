//! Sparks: a fountain of them from the middle of the display's bottom, as
//! from a grinder. Each band of the analyzer throws its sparks its own
//! way, the bass to the left and the treble to the right, more and
//! further the louder it is, so the fountain's shape is the spectrum's;
//! every beat throws a sheaf of them all ways at once, faster. They fly
//! as thrown things do, falling back, bouncing once or twice off the
//! bottom, and cool from white through the visualization's colour into
//! nothing. Each is drawn as a streak along its flight with a faint glow
//! under it and a bright head, the light added to the ground on a dark
//! theme. Paused, they stand; stopped, the last of them fall and go out.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Vec2, pos2, vec2};

use super::beat::{Beat, Rng, glow};
use crate::palette;

/// The bands of the levels fed.
pub const BANDS: usize = 24;
/// A band at full level throws this many sparks a second; under `FROM`
/// none.
const RATE: f32 = 28.0;
const FROM: f32 = 0.3;
/// A spark thrown at full level rises to the display's top in `RISE_S`
/// and, the bass's or the treble's, lands near its end.
const RISE_S: f32 = 0.5;
/// A beat throws this many sparks at full strength, faster than the
/// bands' by `BURST_SPEED`.
const BURST: f32 = 70.0;
const BURST_SPEED: f32 = 1.2;
/// The sparks kept at most (the oldest go).
const MOST: usize = 1500;
/// A spark lives this long at most, a bounce taking this share of the
/// height of its fall, and of its speed across.
const LIFE_S: [f32; 2] = [0.7, 1.6];
const BOUNCE: f32 = 0.35;
const BOUNCE_ACROSS: f32 = 0.6;
/// A spark's streak is as long as its flight in this long, at most
/// `STREAK_MOST` points.
const STREAK_S: f32 = 0.05;
const STREAK_MOST: f32 = 24.0;
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
    size: Vec2,
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
            size: Vec2::ZERO,
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
            self.size = Vec2::ZERO;
            return;
        }
        if self.size != size && self.size.x > 0.0 && self.size.y > 0.0 {
            let scale = size / self.size;
            for s in &mut self.sparks {
                s.at = (s.at.to_vec2() * scale).to_pos2();
                s.speed = s.speed * scale;
            }
        }
        self.size = size;
        if levels.is_some() && !moved {
            return;
        }
        // Thrown up to the top in `RISE_S` at full level.
        let gravity = 2.0 * size.y / (RISE_S * RISE_S);
        let speed = gravity * RISE_S;
        let struck = self.beat.feed(levels, dt);
        // Across: the furthest bands land at the ends at three quarters of
        // full level, after twice the rise.
        let across = size.x / 2.0 / (2.0 * RISE_S) / 0.75;
        let from = pos2(size.x / 2.0, size.y);
        if let Some(strength) = struck {
            for _ in 0..(BURST * strength) as usize {
                let place = self.rng.unit();
                let v = BURST_SPEED * strength * self.rng.between(0.6, 1.0);
                let way = vec2((2.0 * place - 1.0) * across, -speed);
                self.throw(from, way * v, place);
            }
        }
        // Emit throughout a slow frame as well as integrating it in
        // small steps. Otherwise each frame launches one visible clump.
        let mut left = dt;
        while left > 0.0 {
            let step = left.min(STEP_S);
            left -= step;
            if let Some(levels) = levels {
                let n = levels.len().min(BANDS);
                for (band, &level) in levels.iter().take(n).enumerate() {
                    let l = ((level - FROM) / (1.0 - FROM)).clamp(0.0, 1.0);
                    self.owed[band] += RATE * l * l * step;
                    while self.owed[band] >= 1.0 {
                        self.owed[band] -= 1.0;
                        let place = (band as f32 + self.rng.unit()) / n as f32;
                        let strength = (0.5 + 0.5 * l) * self.rng.between(0.75, 1.05);
                        let way = vec2((2.0 * place - 1.0) * across, -speed);
                        let at = from + vec2(self.rng.between(-4.0, 4.0), 0.0);
                        self.throw(at, way * strength, place);
                    }
                }
            }
            for s in &mut self.sparks {
                s.at += s.speed * step + vec2(0.0, gravity * step * step * 0.5);
                s.speed.y += gravity * step;
                s.age += step;
                if s.at.y > size.y && s.speed.y > 0.0 {
                    s.at.y = size.y;
                    s.speed.y *= -BOUNCE;
                    s.speed.x *= BOUNCE_ACROSS;
                }
            }
            self.sparks.retain(|s| {
                s.age < s.life && s.at.x > -STREAK_MOST && s.at.x < size.x + STREAK_MOST
            });
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
        // The glowing source anchors the fountain; its light follows
        // the music and dies with the last sparks.
        let source_light = self.beat.level * 0.4 + self.beat.kick * 0.15;
        if source_light > 0.0 {
            let center = pos2(rect.center().x, rect.bottom());
            let radius = vec2((rect.width() * 0.035).clamp(6.0, 24.0), 6.0);
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(center, glow(colours.peak(0.5), source_light, dark));
            for i in 0..=12 {
                let a = std::f32::consts::PI * i as f32 / 12.0;
                mesh.colored_vertex(
                    center + vec2(a.cos(), -a.sin()) * radius,
                    Color32::TRANSPARENT,
                );
                if i > 0 {
                    mesh.add_triangle(base, base + i, base + i + 1);
                }
            }
        }
        for s in &self.sparks {
            let t = s.age / s.life;
            let colour = colours.at(1.0 - 0.25 * t, s.place);
            let colour = if dark && t < 0.3 {
                hot.lerp_to_gamma(colour, t / 0.3)
            } else if !dark {
                colour.lerp_to_gamma(colours.peak(s.place), 0.3)
            } else {
                colour
            };
            let fade = (1.0 - t).powf(1.5);
            let head = rect.min + s.at.to_vec2();
            let tail_by = s.speed * STREAK_S;
            let tail = head - tail_by * (STREAK_MOST / tail_by.length().max(STREAK_MOST)).min(1.0);
            segment(&mut mesh, tail, head, 4.0, glow(colour, 0.2 * fade, dark));
            segment(&mut mesh, tail, head, 1.4, glow(colour, 0.95 * fade, dark));
            let head_colour = colour;
            mesh.add_colored_rect(
                Rect::from_center_size(head, vec2(2.2, 2.2)),
                glow(head_colour, fade, dark),
            );
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
    for (p, c) in [
        (a + normal * 0.25, Color32::TRANSPARENT),
        (b + normal, colour),
        (b - normal, colour),
        (a - normal * 0.25, Color32::TRANSPARENT),
    ] {
        mesh.colored_vertex(p, c);
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

    #[test]
    fn a_slow_frame_spreads_out_emission() {
        let mut s = Sparks::new();
        s.feed(Some(&[1.0; BANDS]), true, 0.1, vec2(400.0, 100.0));
        let oldest = s.sparks.iter().map(|p| p.age).fold(0.0, f32::max);
        let youngest = s.sparks.iter().map(|p| p.age).fold(1.0, f32::min);
        assert!(oldest > 0.05 && oldest - youngest > 0.03);
    }

    #[test]
    fn resizing_keeps_the_fountain_in_the_display() {
        let mut s = Sparks::new();
        s.feed(None, true, 0.0, vec2(800.0, 100.0));
        s.throw(pos2(400.0, 40.0), vec2(80.0, -50.0), 0.5);
        // Resizing a paused scene changes only its scale, not its age.
        s.feed(Some(&[0.0; BANDS]), false, 0.02, vec2(300.0, 50.0));
        assert_eq!(s.sparks[0].at, pos2(150.0, 20.0));
        assert_eq!(s.sparks[0].speed, vec2(30.0, -25.0));
        assert_eq!(s.sparks[0].age, 0.0);
    }
}
