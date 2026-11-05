//! Aurora: curtains of light over the display, three of them one before
//! the other, their lower edges swaying slowly and the light rising from
//! each as high as the analyzer's level at its place across (the bass on
//! the left), streaked by rays drifting along. The light is brightest at
//! the edge and fades upwards, in the visualization's colours: the edge
//! a colour's top, the light above it a colour further across. On a dark
//! theme it is added to the ground, over a few faint stars. The levels
//! rise quickly and fall slowly, so the curtains breathe with the music
//! rather than flicker. Paused, it stands; stopped, it fades.

use std::f32::consts::TAU;

use egui::{Color32, Mesh, Painter, Rect, pos2};

use super::beat::{glow, level_at};
use crate::palette;

/// The bands of the levels fed.
pub const BANDS: usize = 40;
/// The levels rise in about `RISE_S` and fall in about `FALL_S`.
const RISE_S: f32 = 0.08;
const FALL_S: f32 = 0.7;
/// A level under this lights nothing.
const FROM: f32 = 0.2;
/// A column of the curtains every so many points.
const PITCH: f32 = 3.0;
/// The curtains, the furthest first: the height of their edge (a share
/// of the display's from the top), their light, how much further across
/// the colours their light takes.
const CURTAINS: [(f32, f32, f32); 3] = [(0.58, 0.45, 0.35), (0.68, 0.65, 0.2), (0.8, 0.9, 0.1)];
/// The stars: how many, and their light at most.
const STARS: usize = 48;
const STAR_LIGHT: f32 = 0.35;

pub struct Aurora {
    levels: Vec<f32>,
    time: f32,
}

impl Default for Aurora {
    fn default() -> Self {
        Self::new()
    }
}

impl Aurora {
    pub fn new() -> Aurora {
        Aurora {
            levels: vec![0.0; BANDS],
            time: 0.0,
        }
    }

    /// Moves on by `dt` seconds with the levels heard now (`None`:
    /// nothing; `moved` false: paused, it stands).
    pub fn feed(&mut self, levels: Option<&[f32]>, moved: bool, dt: f32) {
        if levels.is_some() && !moved {
            return;
        }
        self.time += dt;
        for (i, shown) in self.levels.iter_mut().enumerate() {
            let wanted = levels.map_or(0.0, |l| {
                let place = i as f32 / (BANDS - 1) as f32;
                ((level_at(l, place) - FROM) / (1.0 - FROM)).clamp(0.0, 1.0)
            });
            let speed = if wanted > *shown { RISE_S } else { FALL_S };
            *shown += (wanted - *shown) * (1.0 - (-dt / speed).exp());
            if *shown < 0.002 {
                *shown = 0.0;
            }
        }
    }

    /// Some light still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.levels.iter().any(|&l| l > 0.0)
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual();
        let dark = colours.is_dark();
        let t = self.time;
        let (w, h) = (rect.width(), rect.height());
        if w < 4.0 || h < 4.0 {
            return;
        }
        let overall = self.levels.iter().sum::<f32>() / BANDS as f32;

        if dark {
            // Stars at fixed places, twinkling, dimmed under the light.
            let mut mesh = Mesh::default();
            let mut seed = 0x2545_f491_4f6c_dd1du64;
            let star = palette::text_bright();
            for k in 0..STARS {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let x = (seed & 0xffff) as f32 / 65535.0;
                let y = ((seed >> 16) & 0xffff) as f32 / 65535.0 * 0.7;
                let twinkle = 0.5 + 0.5 * (t * (1.3 + (k % 5) as f32 * 0.4) + k as f32).sin();
                let light = STAR_LIGHT * (0.3 + 0.7 * twinkle) * (1.0 - overall).max(0.3);
                let at = pos2(rect.left() + x * w, rect.top() + y * h);
                let r = if k % 7 == 0 { 0.9 } else { 0.6 };
                mesh.add_colored_rect(
                    Rect::from_center_size(at, egui::vec2(r, r) * 2.0),
                    glow(star, light, dark),
                );
            }
            painter.add(mesh);
        }

        let columns = ((w / PITCH) as usize).clamp(8, 1024);
        for (k, &(edge, light, further)) in CURTAINS.iter().enumerate() {
            let k = k as f32;
            let mut mesh = Mesh::default();
            for c in 0..=columns {
                let u = c as f32 / columns as f32;
                let x = rect.left() + u * w;
                let level = level_at(&self.levels, (u + 0.035 * k).clamp(0.0, 1.0));
                // The edge sways: two slow waves running either way.
                let sway = 0.1 * (TAU * (u * 1.3 + 0.07 * k) + t * 0.35 * (1.0 + 0.3 * k)).sin()
                    + 0.05 * (TAU * u * 3.1 - t * 0.6 + k).sin();
                // Folds in the hem, with finer detail than the slow
                // sway. The layers drift separately, giving them depth.
                let fold = (TAU * u * (5.0 + k) + t * 0.45 + k * 1.7).sin();
                let y = rect.top() + h * (edge + sway + fold * 0.018);
                let tall = h * (0.2 + 0.65 * level) * (0.85 + 0.15 * (TAU * u * 2.0 + t).sin());
                // Rays drifting along.
                let rays = 0.45
                    + 0.55
                        * (0.5 + 0.5 * (TAU * u * 23.0 + t * 1.7 + 2.0 * k).sin())
                        * (0.5 + 0.5 * (TAU * u * 9.3 - t * 0.9 + k).sin());
                let edge_fade = (u * 18.0).min((1.0 - u) * 18.0).min(1.0);
                let bright = level.powf(0.65) * rays * light * edge_fade;
                let place = (u * 0.8 + 0.1).min(1.0);
                let low = colours.at(0.6 + 0.4 * level, place);
                let low = if dark {
                    low
                } else {
                    low.lerp_to_gamma(colours.peak(place), 0.3)
                };
                let rim = low.lerp_to_gamma(colours.peak(place), 0.35);
                let high_place = (place + further).min(1.0);
                let high = colours.at(0.8, high_place);
                let high = if dark {
                    high
                } else {
                    high.lerp_to_gamma(colours.peak(high_place), 0.3)
                };
                let clear = Color32::TRANSPARENT;
                let base = mesh.vertices.len() as u32;
                mesh.colored_vertex(pos2(x, y + h * 0.035), clear);
                mesh.colored_vertex(pos2(x, y + 1.0), glow(low, bright * 0.3, dark));
                mesh.colored_vertex(pos2(x, y - 0.5), glow(rim, bright * 1.4, dark));
                mesh.colored_vertex(pos2(x, y - tall * 0.12), glow(low, bright * 0.65, dark));
                mesh.colored_vertex(pos2(x, y - tall * 0.4), glow(high, bright * 0.3, dark));
                mesh.colored_vertex(pos2(x, y - tall), clear);
                if c > 0 {
                    for j in 0..5 {
                        let (a, b) = (base - 6 + j, base + j);
                        mesh.add_triangle(a, b, a + 1);
                        mesh.add_triangle(a + 1, b, b + 1);
                    }
                }
            }
            painter.add(mesh);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rises_quickly_falls_slowly_and_fades() {
        let mut a = Aurora::new();
        let loud = vec![1.0; BANDS];
        for _ in 0..10 {
            a.feed(Some(&loud), true, 0.02);
        }
        assert!(a.levels[0] > 0.9);
        a.feed(None, true, 0.02);
        assert!(a.levels[0] > 0.85, "{}", a.levels[0]);
        // Paused, it stands.
        let before = a.levels.clone();
        a.feed(Some(&loud), false, 0.02);
        assert_eq!(a.levels, before);
        for _ in 0..400 {
            a.feed(None, true, 0.02);
        }
        assert!(!a.active());
        // Quiet bands light nothing.
        let quiet = vec![0.15; BANDS];
        a.feed(Some(&quiet), true, 0.02);
        assert!(!a.active());
    }

    #[test]
    fn the_curtains_breathe_at_the_same_speed_at_different_frame_rates() {
        let mut slow = Aurora::new();
        let mut fast = Aurora::new();
        for _ in 0..5 {
            slow.feed(Some(&[0.8; BANDS]), true, 0.04);
        }
        for _ in 0..20 {
            fast.feed(Some(&[0.8; BANDS]), true, 0.01);
        }
        assert!((slow.levels[0] - fast.levels[0]).abs() < 1e-5);
        slow.feed(None, true, 0.04);
        for _ in 0..4 {
            fast.feed(None, true, 0.01);
        }
        assert!((slow.levels[0] - fast.levels[0]).abs() < 1e-5);
    }
}
