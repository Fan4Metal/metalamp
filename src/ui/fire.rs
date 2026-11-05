//! Fire: flames rising from the spectrum along the display's bottom, as
//! Doom's fire was drawn. The picture is a coarse grid of heat (a pixel
//! about `POINTS` points wide); at least `STEP_HZ` times a second each pixel pulls
//! heat from the row below: one random number of four
//! moves it a pixel aside (mostly to the left, a wind) and takes a step of
//! heat off or not, so the shift and the cooling go together and the heat
//! rises in licking tongues rather than as noise. Each destination is
//! written, leaving no old heat floating between the flames. The bottom
//! row is the source: each column the analyzer's level
//! at its place across (the bass on the left). Travelling ripples in the
//! source break it into tongues, and gusts bend them as they rise. Beats
//! speed up the rise and reduce cooling, throwing the flames higher.
//! Shown as a texture over the display, the heat in the
//! visualization's colours, the hottest drawn towards white. Paused, it
//! stands; stopped, it burns out.

use egui::{Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions, Vec2, pos2};

use super::beat::{Beat, Rng, level_at};
use crate::palette;

/// The bands of the levels fed.
pub const BANDS: usize = 48;
/// Points a pixel of the fire, and its pixels at most across.
const POINTS: f32 = 3.0;
const MAX_WIDTH: usize = 400;
/// The heat rises a pixel at least this many times a second, faster
/// with the sound and especially on beats.
const STEP_HZ: f32 = 110.0;
const KICK_SPEED: f32 = 0.8;
const LEVEL_SPEED: f32 = 0.2;
/// A beat reduces heat loss so its plume reaches higher too.
const KICK_REACH: f32 = 0.4;
/// The source's full heat reaches this share of the height on average.
const REACH: f32 = 0.95;
/// A level under this lights nothing; over it the source's heat grows to
/// full at 1, raised to `POWER`.
const FROM: f32 = 0.25;
const POWER: f32 = 0.65;
/// A beat's height adds this much heat to the whole source.
const KICK_HEAT: f32 = 0.35;
/// The source's heat falls the whole way in this long.
const FALL_S: f32 = 0.15;

pub struct Fire {
    /// The heat, 0 to 1, row by row from the top.
    heat: Vec<f32>,
    width: usize,
    height: usize,
    /// The bottom row's heat before its flicker.
    source: Vec<f32>,
    /// Steps owed (fractions carried on).
    owed: f32,
    /// The simulation clock, frozen along with the flame when paused.
    time: f32,
    beat: Beat,
    rng: Rng,
    lit: bool,
    texture: Option<TextureHandle>,
    /// The heat changed since the texture was set; the colours it was
    /// set in.
    changed: bool,
    drawn: Option<(palette::VisualPalette, Color32)>,
}

impl Default for Fire {
    fn default() -> Self {
        Self::new()
    }
}

impl Fire {
    pub fn new() -> Fire {
        Fire {
            heat: Vec::new(),
            width: 0,
            height: 0,
            source: Vec::new(),
            owed: 0.0,
            time: 0.0,
            beat: Beat::default(),
            rng: Rng::default(),
            lit: false,
            texture: None,
            changed: false,
            drawn: None,
        }
    }

    /// Moves on by `dt` seconds with the levels heard now (`None`:
    /// nothing; `moved` false: paused, it stands) in a display of `size`.
    pub fn feed(&mut self, levels: Option<&[f32]>, moved: bool, dt: f32, size: Vec2) {
        let (w, h) = pixels_for(size);
        if (w, h) != (self.width, self.height) {
            self.width = w;
            self.height = h;
            self.heat = vec![0.0; w * h];
            self.source = vec![0.0; w];
            self.lit = false;
            self.changed = true;
        }
        if w < 4 || h < 4 || (levels.is_some() && !moved) {
            return;
        }
        self.beat.feed(levels, dt);
        let kick = self.beat.kick * KICK_HEAT;
        for (x, s) in self.source.iter_mut().enumerate() {
            let wanted = levels.map_or(0.0, |l| {
                let level = level_at(l, x as f32 / (w - 1) as f32);
                let heat = ((level - FROM) / (1.0 - FROM)).clamp(0.0, 1.0).powf(POWER);
                (heat + kick * heat.sqrt()).min(1.0)
            });
            *s = wanted.max(*s - dt / FALL_S).max(0.0);
        }
        if !self.lit && self.source.iter().all(|&s| s <= 0.0) {
            return;
        }
        let hz = STEP_HZ * (1.0 + KICK_SPEED * self.beat.kick + LEVEL_SPEED * self.beat.level);
        self.owed += dt * hz;
        // The UI caps dt at 0.1 s: enough steps even during a beat on
        // a slow frame, instead of slowing the flame down to four rows.
        let steps = (self.owed as usize).min(24);
        self.owed -= self.owed.floor();
        for _ in 0..steps {
            self.step(1.0 / hz);
        }
        self.changed |= steps > 0;
    }

    /// Pulls heat from the row below, with Doom's coupled drift and
    /// cooling. Every destination is written: scattering left holes
    /// holding old heat, which looked like floating grains.
    fn step(&mut self, dt: f32) {
        self.time += dt;
        let (w, h) = (self.width, self.height);
        // Half a step lost a row on average: full heat reaches `REACH` of
        // the height.
        let cooling = 2.0 / (REACH * h as f32) * (1.0 - KICK_REACH * self.beat.kick);
        for y in 1..h {
            // A shared gust per row bends whole tongues, rather than
            // shaking unrelated pixels. Fractional drift stays smooth.
            let rise = 1.0 - y as f32 / h as f32;
            let wind = (self.time * 2.7).sin() * 0.9
                + (self.time * 5.1 + rise * 5.0).sin() * (0.35 + 0.65 * rise);
            let drift = wind.floor() as isize;
            let blend = wind - wind.floor();
            let mut bits = 0u64;
            for x in 0..w {
                if x % 32 == 0 {
                    bits = self.rng.next();
                }
                let r = (bits & 3) as usize;
                bits >>= 2;
                let from = x as isize + r as isize - 1 + drift;
                let at = |x: isize| self.heat[y * w + x.clamp(0, w as isize - 1) as usize];
                let carried = at(from) * (1.0 - blend) + at(from + 1) * blend;
                let v = 0.85 * carried + 0.15 * self.heat[y * w + x];
                let lost = (r & 1) as f32 * cooling;
                self.heat[(y - 1) * w + x] = (v - lost).max(0.0);
            }
        }
        let bottom = (h - 1) * w;
        let mut lit = false;
        for x in 0..w {
            // Coherent patches of hot and cool fuel travel across the
            // base. Even sustained notes produce growing and shrinking
            // tongues, with fine flicker only over that larger motion.
            let place = x as f32;
            let broad = (place * 0.28 - self.time * 6.0).sin();
            let fine = (place * 0.72 + self.time * 13.0 + broad * 1.4).sin();
            let flicker = (0.8 + 0.25 * broad + 0.18 * fine) * self.rng.between(0.95, 1.0);
            self.heat[bottom + x] = (self.source[x] * flicker).min(1.0);
        }
        for &v in &self.heat {
            if v > 0.003 {
                lit = true;
                break;
            }
        }
        self.lit = lit;
    }

    /// Something still burns: keep repainting.
    pub fn active(&self) -> bool {
        self.lit || self.source.iter().any(|&s| s > 0.0)
    }

    pub fn paint(&mut self, painter: &Painter, rect: Rect) {
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            return;
        }
        let colours = palette::visual();
        let ground = palette::list_bg();
        if self.changed || self.texture.is_none() || self.drawn != Some((colours, ground)) {
            let bright = if colours.is_dark() {
                Color32::from_rgb(255, 246, 220)
            } else {
                colours.peak(0.5)
            };
            let mut image = ColorImage::filled([w, h], ground);
            for (i, out) in image.pixels.iter_mut().enumerate() {
                // Each pixel with its neighbours across (1 2 1): the
                // flames' edges soft, not grains.
                let x = i % w;
                let left = self.heat[i - usize::from(x > 0)];
                let right = self.heat[i + usize::from(x + 1 < w)];
                let v = (left + 2.0 * self.heat[i] + right) / 4.0;
                if v <= 0.01 {
                    continue;
                }
                let place = x as f32 / (w - 1).max(1) as f32;
                // The colour's floor for the cooler half, its top for the
                // hot core, then towards white.
                let mut c = colours.at(v.powf(0.85), place);
                if !colours.is_dark() {
                    c = c.lerp_to_gamma(colours.peak(place), 0.3);
                }
                if v > 0.75 {
                    c = c.lerp_to_gamma(bright, (v - 0.75) / 0.25 * 0.6);
                }
                // Fading into the ground where it is cool.
                let a = (v / 0.28).clamp(0.0, 1.0);
                *out = ground.lerp_to_gamma(c, a * a * (3.0 - 2.0 * a));
            }
            match &mut self.texture {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => {
                    self.texture = Some(painter.ctx().load_texture(
                        "fire",
                        image,
                        TextureOptions::LINEAR,
                    ));
                }
            }
            self.changed = false;
            self.drawn = Some((colours, ground));
        }
        if let Some(t) = &self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(t.id(), rect, uv, Color32::WHITE);
        }
    }
}

/// The fire's pixels for a display of `size` points.
fn pixels_for(size: Vec2) -> (usize, usize) {
    if size.x < 1.0 || size.y < 1.0 {
        return (0, 0);
    }
    (
        ((size.x / POINTS).round() as usize).clamp(1, MAX_WIDTH),
        ((size.y / POINTS).round() as usize).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    /// The heat of each row, the top first.
    fn rows(f: &Fire) -> Vec<f32> {
        f.heat
            .chunks(f.width)
            .map(|r| r.iter().sum::<f32>() / f.width as f32)
            .collect()
    }

    #[test]
    fn flames_rise_with_the_sound_and_burn_out() {
        let mut f = Fire::new();
        let size = vec2(300.0, 90.0);
        let loud = vec![0.9; BANDS];
        for _ in 0..60 {
            f.feed(Some(&loud), true, 0.02, size);
        }
        assert!(f.active());
        let r = rows(&f);
        // Hot at the bottom, cooler going up, reaching past the middle.
        assert!(r[r.len() - 1] > 0.6, "{r:?}");
        assert!(r[r.len() / 2] > 0.05, "{r:?}");
        assert!(r[0] < r[r.len() - 1]);
        // Paused, it stands.
        let before = f.heat.clone();
        let time = f.time;
        f.feed(Some(&loud), false, 0.02, size);
        assert_eq!(f.heat, before);
        assert_eq!(f.time, time);
        // Quiet bands light nothing; stopped, it burns out.
        for _ in 0..200 {
            f.feed(None, true, 0.02, size);
        }
        assert!(!f.active());
        let quiet = vec![0.2; BANDS];
        for _ in 0..30 {
            f.feed(Some(&quiet), true, 0.02, size);
        }
        assert!(!f.active());
    }

    #[test]
    fn the_bass_burns_on_the_left() {
        let mut f = Fire::new();
        let size = vec2(300.0, 90.0);
        let mut bass = vec![0.0; BANDS];
        bass[..BANDS / 4].fill(1.0);
        for _ in 0..40 {
            f.feed(Some(&bass), true, 0.02, size);
        }
        let w = f.width;
        let half = |left: bool| -> f32 {
            f.heat
                .chunks(w)
                .flat_map(|r| if left { &r[..w / 2] } else { &r[w / 2..] })
                .sum()
        };
        assert!(half(true) > 10.0 * half(false).max(0.01));
    }

    #[test]
    fn the_grid_fits() {
        assert_eq!(pixels_for(vec2(300.0, 90.0)), (100, 30));
        assert_eq!(pixels_for(vec2(4000.0, 90.0)).0, MAX_WIDTH);
        assert_eq!(pixels_for(vec2(0.0, 90.0)), (0, 0));
    }

    #[test]
    fn no_old_heat_is_left_between_the_flames() {
        let mut f = Fire::new();
        f.feed(None, true, 0.0, vec2(300.0, 90.0));
        f.heat.fill(1.0);
        for _ in 0..=f.height {
            f.step(1.0 / STEP_HZ);
        }
        assert!(f.heat.iter().all(|&v| v == 0.0));
        assert!(!f.active());
    }

    #[test]
    fn the_flame_reaches_the_middle_within_a_fifth_of_a_second() {
        let mut f = Fire::new();
        let size = vec2(300.0, 90.0);
        for _ in 0..10 {
            f.feed(Some(&[0.9; BANDS]), true, 0.02, size);
        }
        assert!(rows(&f)[f.height / 2] > 0.05);
        assert!(f.heat.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }

    #[test]
    fn a_bass_beat_throws_a_plume_above_the_steady_flame() {
        let size = vec2(300.0, 90.0);
        let steady = [0.5; BANDS];
        let mut pulse = steady;
        pulse[..BANDS / 6].fill(0.95);
        let warm = || {
            let mut f = Fire::new();
            for _ in 0..100 {
                f.feed(Some(&steady), true, 0.02, size);
            }
            f
        };
        let (mut calm, mut beat) = (warm(), warm());
        for _ in 0..10 {
            calm.feed(Some(&steady), true, 0.02, size);
            beat.feed(Some(&pulse), true, 0.02, size);
        }
        let high_heat = |f: &Fire| f.heat[..f.width * (f.height / 2)].iter().sum::<f32>();
        assert!(high_heat(&beat) > high_heat(&calm) + 5.0);
    }
}
