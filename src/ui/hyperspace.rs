//! Hyperspace: a flight through the stars, the stars coming at the viewer
//! out of a point ahead and stretching into streaks as the flight speeds
//! up. The music drives it: the level sets the cruising speed, a beat
//! surges it (the streaks stretch long, the light ahead swells), and a
//! strong beat is a jump, a flash over the whole display. Each star's
//! band is its way out from the middle (the bass at the bottom, the
//! treble at the top), its light and colour that band's level, so the
//! spectrum shapes the tunnel. The point ahead sways slowly, as a ship
//! steering. On a dark theme the streaks are white-hot at their heads and
//! their light adds up. Paused, all stands; stopped, the flight slows and
//! the stars fade away.

use std::f32::consts::{PI, TAU};

use egui::{Color32, Mesh, Painter, Pos2, Rect, Vec2, vec2};

use super::beat::{Beat, Rng, ellipse, glow, level_at, ribbon};
use crate::palette;
use crate::settings::Visual;

/// The bands of the levels fed.
pub const BANDS: usize = 32;
/// A band's level counts from `FROM`, raised to `POWER`; it falls the
/// whole way in `FALL_S`.
const FROM: f32 = 0.3;
const POWER: f32 = 1.4;
const FALL_S: f32 = 0.3;
/// A star for this many square points of the display, within `STARS`.
const PER_STAR: f32 = 150.0;
const STARS: [usize; 2] = [120, 600];
/// The stars' depths: they come in at `FAR` and are gone by `NEAR` (a
/// star at depth `z` is `1/z` of its way to the display's edge, so at
/// `FAR` it is within the middle third). They fade in over the first
/// `FADE_IN` of the way.
const FAR: f32 = 3.0;
const NEAR: f32 = 0.04;
const FADE_IN: f32 = 0.25;
/// No star comes in within this share of the half size of the point
/// ahead (it would stand there for long).
const CLEAR: f32 = 0.05;
/// The speed in depths a second: cruising, more with the level squared,
/// more with a beat's surge squared (fading in `SURGE_S`); the flight
/// takes up a faster speed in `RISE_S` and slows in `SLOW_S`.
const CRUISE: f32 = 0.35;
const LEVEL_SPEED: f32 = 0.7;
const SURGE_SPEED: f32 = 6.0;
const SURGE_S: f32 = 0.4;
const RISE_S: f32 = 0.08;
const SLOW_S: f32 = 0.7;
/// A streak is where the star was this long ago.
const TRAIL_S: f32 = 0.09;
/// A beat from `JUMP` is a jump: a flash over the display fading in
/// `FLASH_S`.
const JUMP: f32 = 0.8;
const FLASH_S: f32 = 0.35;
/// The stars come in over `APPEAR_S` and fade over `FADE_S` once nothing
/// is heard.
const APPEAR_S: f32 = 0.5;
const FADE_S: f32 = 1.5;
/// The point ahead sways this far (shares of the half size) at these
/// rates (a second).
const DRIFT: Vec2 = vec2(0.05, 0.14);
const DRIFT_HZ: [f32; 2] = [0.045, 0.07];
/// The most of a frame taken (after a stall the stars do not leap).
const MOST_DT: f32 = 0.1;
/// The widths are for a display this high (the full bar's).
const REFERENCE_HEIGHT: f32 = 118.0;

#[derive(Clone, Copy, Debug)]
struct Star {
    /// Its way out from the point ahead at depth 1: shares of the half
    /// width and height.
    u: f32,
    v: f32,
    z: f32,
    /// Its band's place (0: the bass, at the bottom; 1: the treble, at
    /// the top).
    place: f32,
    /// How white it burns, 0–1.
    tone: f32,
}

pub struct Hyperspace {
    stars: Vec<Star>,
    /// The bands' levels shown, shaped and falling.
    levels: Vec<f32>,
    beat: Beat,
    speed: f32,
    surge: f32,
    flash: f32,
    /// The sound heard, in seconds: the sway's clock.
    time: f32,
    /// How much the scene shows, 0–1.
    presence: f32,
    rng: Rng,
}

impl Default for Hyperspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Hyperspace {
    pub fn new() -> Hyperspace {
        Hyperspace {
            stars: Vec::new(),
            levels: vec![0.0; BANDS],
            beat: Beat::default(),
            speed: 0.0,
            surge: 0.0,
            flash: 0.0,
            time: 0.0,
            presence: 0.0,
            rng: Rng::default(),
        }
    }

    /// Moves on by `dt` seconds with the levels heard now (`None`:
    /// nothing, the flight slows and fades; `moved` false: paused, all
    /// stands) in a display of `size`.
    pub fn feed(&mut self, levels: Option<&[f32]>, moved: bool, dt: f32, size: Vec2) {
        if size.x < 8.0 || size.y < 8.0 {
            self.stars.clear();
            return;
        }
        if levels.is_some() && !moved {
            return;
        }
        let dt = dt.min(MOST_DT);
        match levels {
            Some(_) => self.presence = (self.presence + dt / APPEAR_S).min(1.0),
            None if self.presence > 0.0 => {
                self.presence = (self.presence - dt / FADE_S).max(0.0);
                if self.presence <= 0.0 {
                    *self = Hyperspace::new();
                    return;
                }
            }
            None => return,
        }
        for (k, shown) in self.levels.iter_mut().enumerate() {
            let place = k as f32 / (BANDS - 1) as f32;
            let heard = levels.map_or(0.0, |l| shaped(level_at(l, place)));
            *shown = heard.max(*shown - dt / FALL_S);
        }
        if let Some(strength) = self.beat.feed(levels, dt) {
            self.surge = self.surge.max(strength);
            if strength >= JUMP {
                self.flash = self.flash.max(strength);
            }
        }
        self.surge = (self.surge - dt / SURGE_S).max(0.0);
        self.flash = (self.flash - dt / FLASH_S).max(0.0);
        let target = if levels.is_some() {
            let level = shaped(self.beat.level);
            CRUISE + LEVEL_SPEED * level * level + SURGE_SPEED * self.surge * self.surge
        } else {
            0.0
        };
        let follow = if target > self.speed { RISE_S } else { SLOW_S };
        self.speed += (target - self.speed) * (1.0 - (-dt / follow).exp());
        self.time += dt;

        let count = ((size.x * size.y / PER_STAR) as usize).clamp(STARS[0], STARS[1]);
        self.stars.truncate(count);
        while self.stars.len() < count {
            let star = self.spawn(true);
            self.stars.push(star);
        }
        let tail = self.speed * TRAIL_S;
        for i in 0..self.stars.len() {
            let s = &mut self.stars[i];
            s.z -= self.speed * dt;
            // Gone once its tail is past the edge (the sway's room kept).
            let reach = s.u.abs().max(s.v.abs());
            if s.z < NEAR || reach / (s.z + tail) > 1.0 + DRIFT.y {
                self.stars[i] = self.spawn(false);
            }
        }
    }

    /// A star coming in at `FAR`, or (`scattered`) anywhere on its way,
    /// for the first stars.
    fn spawn(&mut self, scattered: bool) -> Star {
        let (u, v) = loop {
            let u = self.rng.between(-1.0, 1.0);
            let v = self.rng.between(-1.0, 1.0);
            if u * u + v * v > CLEAR * CLEAR {
                break (u, v);
            }
        };
        let reach = u.abs().max(v.abs());
        let z = if scattered {
            self.rng.between(reach + 0.05, FAR)
        } else {
            FAR - 0.15 * self.rng.unit()
        };
        Star {
            u,
            v,
            z,
            place: place_of(u, v),
            tone: self.rng.unit(),
        }
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.presence > 0.0
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        if self.presence <= 0.0 || self.stars.is_empty() {
            return;
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual_for(Visual::Hyperspace);
        let dark = colours.is_dark();
        // Glows lie over a light ground rather than adding to it: softer.
        let haze = if dark { 1.0 } else { 0.5 };
        let half = rect.size() * 0.5;
        let h = rect.height();
        let t = self.time;
        let centre = rect.center()
            + vec2(
                DRIFT.x * half.x * (TAU * DRIFT_HZ[0] * t).sin(),
                DRIFT.y * half.y * (TAU * DRIFT_HZ[1] * t + 1.0).sin(),
            );
        let thin = (h / REFERENCE_HEIGHT).clamp(0.6, 1.6);
        let presence = self.presence;
        // How white-hot the heads burn: with the speed.
        let hot = (self.speed / (CRUISE + 0.5 * SURGE_SPEED)).min(1.0);
        let bass = self.levels[..BANDS / 6].iter().sum::<f32>() / (BANDS / 6) as f32;
        let mut mesh = Mesh::default();
        if self.flash > 0.0 {
            let a = 0.2 * self.flash * self.flash * presence * haze;
            mesh.add_colored_rect(rect, glow(colours.at(1.0, 0.25), a, dark));
        }
        // The light ahead: a wide soft glow swelling with the bass and the
        // surge, a small bright core.
        let swell = 0.6 + 0.5 * bass + 0.6 * self.surge;
        ellipse(
            &mut mesh,
            centre,
            vec2(1.5 * h, 0.6 * h) * swell,
            glow(
                colours.at(1.0, 0.5),
                (0.12 + 0.12 * bass + 0.2 * self.surge) * presence * haze,
                dark,
            ),
        );
        ellipse(
            &mut mesh,
            centre,
            vec2(0.3 * h, 0.14 * h) * swell,
            glow(
                colours.at(1.0, 0.5).lerp_to_gamma(colours.peak(0.5), 0.5),
                (0.3 + 0.45 * self.surge) * presence * haze,
                dark,
            ),
        );
        let tail = self.speed * TRAIL_S;
        let at = |u: f32, v: f32, z: f32| centre + vec2(u * half.x, v * half.y) / z;
        for s in &self.stars {
            let level = level_at(&self.levels, s.place);
            let near = 1.0 - s.z / FAR;
            let fade = smoothstep(near / FADE_IN);
            let light = fade
                * (0.2 + 0.8 * near)
                * (0.35 + 0.65 * level + 0.2 * s.tone).min(1.0)
                * presence;
            if light < 0.01 {
                continue;
            }
            let head = at(s.u, s.v, s.z);
            let back = at(s.u, s.v, s.z + tail);
            let width = thin * (0.3 + 1.3 / (1.0 + 2.5 * s.z));
            let colour = colours.at(0.45 + 0.55 * level, s.place);
            let core = if dark {
                colour.lerp_to_gamma(Color32::WHITE, 0.1 + 0.35 * hot * s.tone)
            } else {
                colour
            };
            let points = [head, back];
            if (head - back).length() > 1.0 {
                // A soft glow along it, then the streak narrowing to its
                // tail.
                ribbon(
                    &mut mesh,
                    &points,
                    |i| if i == 0 { 3.0 * width } else { 1.2 * width },
                    |i, _| {
                        if i == 0 {
                            glow(colour, 0.3 * light * haze, dark)
                        } else {
                            Color32::TRANSPARENT
                        }
                    },
                    true,
                );
                ribbon(
                    &mut mesh,
                    &points,
                    |i| if i == 0 { 0.5 * width } else { 0.12 * width },
                    |i, _| {
                        if i == 0 {
                            glow(core, light, dark)
                        } else {
                            glow(colour, 0.08 * light, dark)
                        }
                    },
                    false,
                );
            }
            if near > 0.5 {
                // Near: a halo about the head.
                let halo = 2.0 * near - 1.0;
                ellipse(
                    &mut mesh,
                    head,
                    Vec2::splat(3.5 * width),
                    glow(colour, 0.25 * halo * light * haze, dark),
                );
            }
            dot(&mut mesh, head, 0.7 * width, glow(core, light, dark));
        }
        painter.add(mesh);
    }
}

/// A level over `FROM`, raised to `POWER`.
fn shaped(level: f32) -> f32 {
    ((level - FROM) / (1.0 - FROM)).clamp(0.0, 1.0).powf(POWER)
}

/// The band's place of a star going out along (`u`, `v`) (down is
/// positive): 0 straight down (the bass), 1 straight up (the treble), a
/// half to the sides.
fn place_of(u: f32, v: f32) -> f32 {
    let mut d = (v.atan2(u) - PI / 2.0).abs();
    if d > PI {
        d = TAU - d;
    }
    d / PI
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// A small solid diamond at `centre`.
fn dot(mesh: &mut Mesh, centre: Pos2, radius: f32, colour: Color32) {
    let radius = radius.max(0.6);
    let base = mesh.vertices.len() as u32;
    for d in [
        vec2(0.0, -1.0),
        vec2(1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(-1.0, 0.0),
    ] {
        mesh.colored_vertex(centre + d * radius, colour);
    }
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Vec2 = vec2(760.0, 118.0);

    /// A flight fed `level` in every band for a second.
    fn flying(level: f32) -> Hyperspace {
        let mut h = Hyperspace::new();
        let levels = [level; BANDS];
        for _ in 0..50 {
            h.feed(Some(&levels), true, 0.02, SIZE);
        }
        h
    }

    /// Each star's way out to the edge (1: at it).
    fn reaches(h: &Hyperspace) -> Vec<f32> {
        h.stars
            .iter()
            .map(|s| s.u.abs().max(s.v.abs()) / s.z)
            .collect()
    }

    #[test]
    fn the_stars_fly_out_from_the_point_ahead() {
        let mut h = flying(0.5);
        assert_eq!(h.stars.len(), STARS[1].min((SIZE.x * SIZE.y / PER_STAR) as usize));
        assert!(h.speed > 0.9 * CRUISE);
        let before = reaches(&h);
        h.feed(Some(&[0.5; BANDS]), true, 0.02, SIZE);
        let after = reaches(&h);
        let out = before.iter().zip(&after).filter(|(b, a)| a > b).count();
        // All but the few gone and come in anew.
        assert!(out * 10 > before.len() * 9, "{out} of {}", before.len());
    }

    #[test]
    fn a_beat_surges_the_flight_and_a_strong_one_jumps() {
        let mut h = flying(0.1);
        let cruising = h.speed;
        let mut beat = [0.2; BANDS];
        beat[..BANDS / 6].fill(1.0);
        h.feed(Some(&beat), true, 0.02, SIZE);
        assert!(h.surge > 0.0);
        for _ in 0..5 {
            h.feed(Some(&[0.2; BANDS]), true, 0.02, SIZE);
        }
        assert!(h.speed > 3.0 * cruising, "{} {cruising}", h.speed);
        assert!(h.flash > 0.0);
        // It calms down again.
        for _ in 0..150 {
            h.feed(Some(&[0.1; BANDS]), true, 0.02, SIZE);
        }
        assert!(h.speed < 1.2 * cruising && h.flash == 0.0);
    }

    #[test]
    fn paused_it_stands_stopped_it_slows_and_goes() {
        let mut h = flying(0.6);
        let at: Vec<f32> = h.stars.iter().map(|s| s.z).collect();
        h.feed(Some(&[0.6; BANDS]), false, 0.02, SIZE);
        assert_eq!(at, h.stars.iter().map(|s| s.z).collect::<Vec<_>>());
        let mut slower = h.speed;
        for _ in 0..20 {
            h.feed(None, true, 0.02, SIZE);
            assert!(h.speed <= slower);
            slower = h.speed;
        }
        assert!(h.active());
        for _ in 0..100 {
            h.feed(None, true, 0.02, SIZE);
        }
        assert!(!h.active() && h.stars.is_empty());
    }

    #[test]
    fn the_bass_is_at_the_bottom_the_treble_at_the_top() {
        assert!(place_of(0.0, 1.0) < 1e-6);
        assert!((place_of(0.0, -1.0) - 1.0).abs() < 1e-6);
        assert!((place_of(1.0, 0.0) - 0.5).abs() < 1e-6);
        assert!((place_of(-1.0, 0.0) - 0.5).abs() < 1e-6);
        assert!((place_of(-0.5, 0.5) - 0.25).abs() < 1e-6);
    }
}
