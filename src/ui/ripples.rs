//! Ripples: a still pond seen from above its near shore, drops falling
//! on it as notes start in the analyzer's bands (`beat::Onsets`): the
//! bass on the left, its rings wide, slow and long-lived, the treble on
//! the right, small and quick. Each drop jumps up as a splash and sends
//! out a few crests, ellipses flattened by the view, smaller further
//! back; on a dark theme their light adds up, so where the rings of two
//! drops cross they meet in bright knots, as waves do. A beat sends one
//! great wave in from under the near shore over the whole pond, and the
//! faint streaks of the water's surface sway with it. Paused, all
//! stands; stopped, the rings run out and the pond goes.

use std::f32::consts::TAU;

use egui::epaint::PathStroke;
use egui::{Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use super::beat::{Beat, Onsets, Rng, glow};
use crate::palette;
use crate::settings::Visual;

/// The bands of the levels fed: the places drops fall at.
pub const BANDS: usize = 16;
/// The ellipses' height as a share of their width.
const SQUASH: f32 = 0.32;
/// A drop at the back is `BACK` as large and slow as one at the front.
const BACK: f32 = 0.55;
/// Drops fall between these shares of the height from the top.
const NEAREST: f32 = 0.9;
const FARTHEST: f32 = 0.3;
/// A drop's rings run `SPEED_BASS` to `SPEED_TREBLE` points a second,
/// `WAVE_BASS` to `WAVE_TREBLE` points apart, and fade in `LIFE_BASS`
/// to `LIFE_TREBLE` seconds (a third of their light left at that age).
const SPEED_BASS: f32 = 55.0;
const SPEED_TREBLE: f32 = 80.0;
const WAVE_BASS: f32 = 15.0;
const WAVE_TREBLE: f32 = 6.0;
const LIFE_BASS: f32 = 2.2;
const LIFE_TREBLE: f32 = 0.8;
/// The crests of a drop.
const CRESTS: usize = 3;
/// A beat's wave: from `GREAT_FROM` heights under the top (under the
/// near shore), `GREAT_SPEED` points a second, its crests `GREAT_WAVE`
/// apart, fading in `GREAT_LIFE`.
const GREAT_FROM: f32 = 1.35;
const GREAT_SPEED: f32 = 280.0;
const GREAT_WAVE: f32 = 24.0;
const GREAT_LIFE: f32 = 1.2;
/// A splash stands `SPLASH` points high at full strength and falls in
/// `SPLASH_S`.
const SPLASH: f32 = 9.0;
const SPLASH_S: f32 = 0.28;
/// At most `MOST` drops at once (the oldest go).
const MOST: usize = 48;
/// The surface's streaks.
const STREAKS: usize = 7;

#[derive(Clone, Copy)]
struct Drop {
    /// Where it fell: shares of the width and the height.
    x: f32,
    y: f32,
    /// Its band's place (0: the bass), its strength (0.4–1), its age.
    place: f32,
    strength: f32,
    age: f32,
    /// A beat's great wave.
    great: bool,
}

impl Drop {
    fn life(&self) -> f32 {
        if self.great {
            GREAT_LIFE
        } else {
            lerp(LIFE_BASS, LIFE_TREBLE, self.place)
        }
    }

    /// How large it is where it fell: 1 at the front.
    fn scale(&self) -> f32 {
        if self.great {
            1.0
        } else {
            lerp(BACK, 1.0, (self.y - FARTHEST) / (NEAREST - FARTHEST))
        }
    }

    /// The radius of crest `k` (across, in points), and its light (0–1
    /// and over, where crests add up).
    fn crest(&self, k: usize) -> (f32, f32) {
        let (speed, wave) = if self.great {
            (GREAT_SPEED, GREAT_WAVE)
        } else {
            (
                lerp(SPEED_BASS, SPEED_TREBLE, self.place),
                lerp(WAVE_BASS, WAVE_TREBLE, self.place),
            )
        };
        let s = self.scale();
        let r = (speed * self.age - k as f32 * wave) * s;
        if r <= 0.5 {
            return (r, 0.0);
        }
        let light = self.strength
            * (-self.age / self.life()).exp()
            * (1.0 - 0.3 * k as f32)
            * (1.0 + r / 40.0).powf(-0.5)
            * (r / 3.0).min(1.0);
        (r, light)
    }

    fn gone(&self) -> bool {
        self.age > self.life() * 3.0
    }
}

pub struct Ripples {
    drops: Vec<Drop>,
    onsets: Onsets,
    beat: Beat,
    rng: Rng,
    /// The sound heard, in seconds: the streaks' clock.
    time: f32,
    /// Something was heard since the pond last went still.
    shown: bool,
}

impl Default for Ripples {
    fn default() -> Self {
        Self::new()
    }
}

impl Ripples {
    pub fn new() -> Ripples {
        Ripples {
            drops: Vec::new(),
            onsets: Onsets::default(),
            beat: Beat::default(),
            rng: Rng::default(),
            time: 0.0,
            shown: false,
        }
    }

    /// Moves on with `heard`: the levels heard now (`BANDS`) and the
    /// seconds of sound heard since the frame before (0: paused, all
    /// stands); `None`: nothing is heard, and the rings run out for `dt`
    /// seconds.
    pub fn feed(&mut self, heard: Option<(&[f32], f32)>, dt: f32) {
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
        self.time += seconds;
        for d in &mut self.drops {
            d.age += seconds;
        }
        self.drops.retain(|d| !d.gone());
        let starts = self.onsets.feed(levels, BANDS, seconds);
        for (band, &strength) in starts.iter().enumerate() {
            if strength > 0.0 {
                let place = band as f32 / (BANDS - 1) as f32;
                let x = (band as f32 + 0.5 + self.rng.between(-0.35, 0.35)) / BANDS as f32;
                let y = self.rng.between(FARTHEST, NEAREST);
                self.add(Drop {
                    x,
                    y,
                    place,
                    strength,
                    age: 0.0,
                    great: false,
                });
            }
        }
        if let Some(strength) = self.beat.feed(levels, seconds) {
            let x = self.rng.between(0.35, 0.65);
            self.add(Drop {
                x,
                y: GREAT_FROM,
                place: 0.0,
                strength,
                age: 0.0,
                great: true,
            });
        }
        if levels.is_none() && self.drops.is_empty() {
            *self = Ripples::new();
        }
    }

    fn add(&mut self, drop: Drop) {
        if self.drops.len() >= MOST {
            self.drops.remove(0);
        }
        self.drops.push(drop);
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.shown
    }

    /// The surface's streaks, then each drop's crests (the ones further
    /// back first) and splashes.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        if !self.shown || rect.width() < 16.0 || rect.height() < 16.0 {
            return;
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual_for(Visual::Ripples);
        let dark = colours.is_dark();
        let (w, h) = (rect.width(), rect.height());
        let kick = self.beat.kick;

        // The surface: faint streaks, closer together further back,
        // swaying more as a beat's wave passes.
        for k in 0..STREAKS {
            let depth = k as f32 / (STREAKS - 1) as f32;
            let y = rect.top() + h * (0.18 + 0.8 * depth.powf(1.4));
            let sway = 0.6 + 2.5 * kick;
            let points: Vec<Pos2> = (0..=40)
                .map(|i| {
                    let u = i as f32 / 40.0;
                    let x = rect.left() + u * w;
                    let wobble = (TAU * (u * (2.0 + depth) + 0.15 * self.time) + k as f32).sin();
                    pos2(x, y + wobble * sway * (0.4 + depth))
                })
                .collect();
            let colour = colours.at(0.3, 0.5);
            let light = (0.07 + 0.1 * kick) * (0.4 + 0.6 * depth);
            let colour = if dark {
                glow(colour, light, true)
            } else {
                colour.gamma_multiply(light)
            };
            painter.add(Shape::line(points, Stroke::new(1.0, colour)));
        }

        let mut order: Vec<&Drop> = self.drops.iter().collect();
        order.sort_by(|a, b| a.y.total_cmp(&b.y));
        for d in order {
            let centre = pos2(rect.left() + d.x * w, rect.top() + d.y * h);
            for k in 0..CRESTS {
                let (r, light) = d.crest(k);
                if light < 0.015 {
                    continue;
                }
                let colour = if d.great {
                    colours.peak(0.5).lerp_to_gamma(colours.at(1.0, 0.5), 0.5)
                } else {
                    colours
                        .at(0.45 + 0.55 * light.min(1.0), d.place)
                        .lerp_to_gamma(
                            colours.peak(d.place),
                            if k == 0 { 0.3 * light } else { 0.0 },
                        )
                };
                let width = if d.great {
                    2.2
                } else {
                    1.0 + 0.7 * light.min(1.0)
                };
                let points = ellipse(centre, r, r * SQUASH);
                if dark {
                    painter.add(Shape::closed_line(
                        points.clone(),
                        PathStroke::new(width * 3.5, glow(colour, 0.25 * light, true)),
                    ));
                    // Added to what is under it: crossing crests brighten.
                    painter.add(Shape::closed_line(
                        points,
                        Stroke::new(width, glow(colour, (0.35 + light).min(1.0), true)),
                    ));
                } else {
                    painter.add(Shape::closed_line(
                        points,
                        Stroke::new(width, colour.gamma_multiply((0.25 + light).min(1.0))),
                    ));
                }
            }
            // The splash: a drop jumping up where it fell.
            if !d.great && d.age < SPLASH_S {
                let t = d.age / SPLASH_S;
                let tall = SPLASH * d.strength * d.scale() * (1.0 - t) * (t * 4.0).min(1.0);
                let spark = colours.peak(d.place);
                let light = 1.0 - t;
                painter.line_segment(
                    [centre, centre - vec2(0.0, tall)],
                    Stroke::new(1.5, spark.gamma_multiply(light)),
                );
                painter.circle_filled(
                    centre - vec2(0.0, tall),
                    1.3 + d.strength,
                    spark.gamma_multiply(light),
                );
                if dark {
                    painter.circle_filled(
                        centre,
                        4.0 + 6.0 * d.strength * (1.0 - t),
                        glow(spark, 0.3 * light, true),
                    );
                }
            }
        }
    }
}

/// The points of an ellipse about `centre`, `rx` across and `ry` up and
/// down, closer together on a large one.
fn ellipse(centre: Pos2, rx: f32, ry: f32) -> Vec<Pos2> {
    let n = ((rx * 0.5) as usize).clamp(16, 120);
    (0..n)
        .map(|i| {
            let a = TAU * i as f32 / n as f32;
            pos2(centre.x + rx * a.cos(), centre.y + ry * a.sin())
        })
        .collect()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Under a beat's least bass: no great wave.
    fn quiet() -> Vec<f32> {
        vec![0.2; BANDS]
    }

    #[test]
    fn a_note_drops_where_its_band_lies() {
        let mut r = Ripples::new();
        for _ in 0..20 {
            r.feed(Some((&quiet(), 0.02)), 0.02);
        }
        assert!(r.drops.is_empty());
        let mut notes = quiet();
        // Above the bands a beat listens to.
        notes[3] = 0.9;
        notes[BANDS - 2] = 0.9;
        r.feed(Some((&notes, 0.02)), 0.02);
        assert_eq!(r.drops.len(), 2);
        let (bass, treble) = (r.drops[0], r.drops[1]);
        assert!(bass.x < 0.25 && treble.x > 0.8);
        assert!((FARTHEST..=NEAREST).contains(&bass.y));
        // The bass's rings spread wider and live longer.
        r.feed(Some((&notes, 0.5)), 0.5);
        let gap = |d: &Drop| (d.crest(0).0 - d.crest(1).0) / d.scale();
        assert!(gap(&r.drops[0]) > gap(&r.drops[1]));
        assert!(r.drops[0].life() > r.drops[1].life());
        // Held: no more drops; paused: all stands.
        assert_eq!(r.drops.len(), 2);
        let age = r.drops[0].age;
        r.feed(Some((&notes, 0.0)), 0.5);
        assert_eq!(r.drops[0].age, age);
    }

    #[test]
    fn a_beat_sends_a_great_wave() {
        let mut r = Ripples::new();
        for _ in 0..30 {
            r.feed(Some((&[0.1; BANDS], 0.02)), 0.02);
        }
        let mut kick = vec![0.3; BANDS];
        kick[..3].fill(0.95);
        r.feed(Some((&kick, 0.02)), 0.02);
        assert!(r.drops.iter().any(|d| d.great));
    }

    #[test]
    fn crests_follow_one_another_and_fade() {
        let d = Drop {
            x: 0.5,
            y: NEAREST,
            place: 0.0,
            strength: 1.0,
            age: 0.5,
            great: false,
        };
        let (r0, l0) = d.crest(0);
        let (r1, l1) = d.crest(1);
        assert!(r0 > r1 && l0 > l1 && l1 > 0.0);
        let old = Drop { age: 3.0, ..d };
        assert!(old.crest(0).1 < l0 * 0.3);
        assert!(Drop { age: 7.0, ..d }.gone());
    }

    #[test]
    fn stopped_the_rings_run_out_and_the_pond_goes() {
        let mut r = Ripples::new();
        r.feed(Some((&quiet(), 0.02)), 0.02);
        r.feed(Some((&[1.0; BANDS], 0.02)), 0.02);
        assert!(!r.drops.is_empty());
        for _ in 0..200 {
            r.feed(None, 0.05);
        }
        assert!(!r.active());
    }
}
