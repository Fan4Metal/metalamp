//! Vortex: a picture with feedback, as Winamp's MilkDrop and Geiss drew
//! theirs. Each frame the picture before is drawn again grown a little
//! from the display's middle, turned a little (the turn swinging slowly
//! one way and the other), rippled, and dimmed; then the wave heard now
//! is drawn across the middle in the display's colours, brighter the
//! louder. So every wave leaves a trail flying outwards and swirling
//! until it fades. On each beat (the bass rising over its recent mean)
//! the picture is thrown outwards faster and the wave flares, drawn
//! towards white. The picture is a buffer of at most `PIXELS` pixels
//! (premultiplied colour and coverage, floats so the dimming does not
//! stall at a dark grey as it would in bytes), shown as a texture
//! stretched over the display with linear filtering. Paused, it stands;
//! stopped, it flies out and fades until nothing is left.

use std::f32::consts::TAU;

use egui::{Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions, Vec2, pos2};

use crate::palette;

/// The bands of the levels fed (the bass and the overall level).
pub const BANDS: usize = 32;
/// The wave drawn each frame: the last so many seconds heard.
pub const WAVE_S: f32 = 0.025;
/// The picture's pixels at most, and its width at most (a pixel a point
/// up to them: stretched, it blurs).
const PIXELS: f32 = 120_000.0;
const MAX_WIDTH: f32 = 1024.0;
/// The light halves this often; `CUT` a second is taken off besides, so
/// faint light goes soon (the blur of the sampling gathers in it) and the
/// last of it with `active`.
const HALF_S: f32 = 0.3;
const CUT: f32 = 0.2;
/// The picture grows this share a second, and this much more at a kick's
/// height; a kick fades in `KICK_S`.
const ZOOM: f32 = 0.6;
const KICK_ZOOM: f32 = 2.5;
const KICK_S: f32 = 0.3;
/// The turn, in radians a second at most, swinging one way and the other
/// over `SWING_S`; a kick turns it `KICK_SWIRL` more at its height, the
/// way changing with each kick.
const SWIRL: f32 = 0.7;
const SWING_S: f32 = 9.0;
const KICK_SWIRL: f32 = 2.0;
/// The middle the picture grows from wanders this far (shares of the half
/// width and the half height), round in these times.
const WANDER: Vec2 = Vec2::new(0.35, 0.3);
const WANDER_S: Vec2 = Vec2::new(7.0, 5.3);
/// The ripple: its amplitude in half heights with the bass at its height,
/// its waves per half height, and per second.
const RIPPLE: f32 = 0.02;
const RIPPLE_K: f32 = 5.0;
const RIPPLE_HZ: f32 = 1.0;
/// The wave rides a carrier: this high (a share of the half height), so
/// many waves across the display, running so many a second to the right.
const CARRIER: f32 = 0.25;
const CARRIER_WAVES: f32 = 1.5;
const CARRIER_HZ: f32 = 0.8;
/// The wave's swing as a share of the half height, and the light it
/// leaves at full level.
const WAVE_HEIGHT: f32 = 0.4;
const GAIN: f32 = 0.8;
/// A beat: the bass this far over its mean of the last `MEAN_S`, and at
/// least `KICK_LEAST`, `KICK_GAP_S` after the last.
const KICK_RISE: f32 = 0.06;
const KICK_LEAST: f32 = 0.3;
const KICK_GAP_S: f32 = 0.12;
const MEAN_S: f32 = 0.4;
/// The levels shown fall the whole way in this long.
const FALL_S: f32 = 0.3;

pub struct Vortex {
    /// The picture, row by row from the top: premultiplied colour and
    /// coverage; and the room the next one is made in.
    pixels: Vec<[f32; 4]>,
    scratch: Vec<[f32; 4]>,
    width: usize,
    height: usize,
    /// Something is lit.
    lit: bool,
    time: f32,
    /// The bass and the overall level shown (falling at `FALL_S`), the
    /// bass's recent mean, a kick's height, the time since the last and
    /// the way it turns the picture (1 or -1).
    bass: f32,
    level: f32,
    mean: f32,
    kick: f32,
    since_kick: f32,
    way: f32,
    texture: Option<TextureHandle>,
    /// The picture changed since the texture was set, and the ground it
    /// was set over.
    changed: bool,
    ground: Color32,
}

impl Default for Vortex {
    fn default() -> Self {
        Self::new()
    }
}

impl Vortex {
    pub fn new() -> Vortex {
        Vortex {
            pixels: Vec::new(),
            scratch: Vec::new(),
            width: 0,
            height: 0,
            lit: false,
            time: 0.0,
            bass: 0.0,
            level: 0.0,
            mean: 0.0,
            kick: 0.0,
            since_kick: 0.0,
            way: 1.0,
            texture: None,
            changed: false,
            ground: Color32::TRANSPARENT,
        }
    }

    /// Moves on by `dt` seconds with the wave and the levels heard now
    /// (`None`: nothing heard; `moved` false: paused, nothing new heard)
    /// in a display of `size` points.
    pub fn feed(&mut self, heard: Option<(&[f32], &[f32])>, moved: bool, dt: f32, size: Vec2) {
        let (w, h) = pixels_for(size);
        if (w, h) != (self.width, self.height) {
            self.width = w;
            self.height = h;
            self.pixels = vec![[0.0; 4]; w * h];
            self.scratch = self.pixels.clone();
            self.lit = false;
            self.changed = true;
        }
        if w < 8 || h < 8 || (heard.is_some() && !moved) {
            return;
        }
        let (bass, level) =
            heard
                .map(|(_, l)| l)
                .filter(|l| !l.is_empty())
                .map_or((0.0, 0.0), |l| {
                    let low = (l.len() / 6).max(1);
                    let mean = |s: &[f32]| s.iter().sum::<f32>() / s.len() as f32;
                    (mean(&l[..low]), mean(l))
                });
        self.bass = bass.max(self.bass - dt / FALL_S).max(0.0);
        self.level = level.max(self.level - dt / FALL_S).max(0.0);
        let rise = bass - self.mean;
        self.mean += (bass - self.mean) * (dt / MEAN_S).min(1.0);
        self.since_kick += dt;
        self.kick = (self.kick - dt / KICK_S).max(0.0);
        if heard.is_some() && rise > KICK_RISE && bass > KICK_LEAST && self.since_kick > KICK_GAP_S
        {
            self.kick = (0.5 + (rise - KICK_RISE) * 8.0).clamp(0.4, 1.0);
            self.since_kick = 0.0;
            self.way = -self.way;
        }
        if heard.is_none() && !self.lit {
            return;
        }
        self.time += dt;
        self.warp(dt);
        if let Some((wave, _)) = heard {
            self.draw(wave);
        }
        self.changed = true;
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.lit
    }

    /// The picture grown from its wandering middle, turned, rippled and
    /// dimmed by `dt` seconds' worth: each pixel of the new one takes the
    /// old one's colour where it came from (sampled between four pixels).
    fn warp(&mut self, dt: f32) {
        let (w, h) = (self.width, self.height);
        let half = h as f32 / 2.0;
        let t = self.time;
        let cx = w as f32 / 2.0 * (1.0 + WANDER.x * (TAU * t / WANDER_S.x).sin());
        let cy = half * (1.0 + WANDER.y * (TAU * t / WANDER_S.y + 1.0).sin());
        let scale = 1.0 / (1.0 + (ZOOM + KICK_ZOOM * self.kick) * dt);
        let turn = (-SWIRL * (TAU * t / SWING_S).sin() + KICK_SWIRL * self.kick * self.way) * dt;
        let (sn, cs) = turn.sin_cos();
        let dim = 0.5f32.powf(dt / HALF_S);
        let cut = CUT * dt;
        let ripple = RIPPLE * (0.3 + 0.7 * self.bass);
        // The ripple's shift of each row (along it) and column (down it).
        let row_shift: Vec<f32> = (0..h)
            .map(|y| {
                let py = (y as f32 + 0.5 - cy) / half;
                ripple * (py * RIPPLE_K + t * RIPPLE_HZ * TAU).sin()
            })
            .collect();
        let column_shift: Vec<f32> = (0..w)
            .map(|x| {
                let px = (x as f32 + 0.5 - cx) / half;
                ripple * (px * RIPPLE_K * 0.7 + t * RIPPLE_HZ * 1.3 * TAU).sin()
            })
            .collect();
        let mut lit = false;
        for (y, (row, &along)) in self.scratch.chunks_mut(w).zip(&row_shift).enumerate() {
            let py = (y as f32 + 0.5 - cy) / half;
            for (x, (out, &down)) in row.iter_mut().zip(&column_shift).enumerate() {
                let px = (x as f32 + 0.5 - cx) / half;
                let rx = px + along;
                let ry = py + down;
                let sx = (rx * cs - ry * sn) * scale;
                let sy = (rx * sn + ry * cs) * scale;
                let mut p = sample(
                    &self.pixels,
                    w,
                    h,
                    sx * half + cx - 0.5,
                    sy * half + cy - 0.5,
                );
                if p[3] > 0.0 {
                    // Dimmed, the colour kept.
                    let left = (p[3] * dim - cut).max(0.0);
                    let f = left / p[3];
                    for c in &mut p {
                        *c *= f;
                    }
                    lit |= left > 0.0;
                }
                *out = p;
            }
        }
        std::mem::swap(&mut self.pixels, &mut self.scratch);
        self.lit = lit;
    }

    /// The wave on its carrier across the middle, a column a pixel: from
    /// the column before's height to this one's, the light spread along
    /// the run, a pixel wide.
    fn draw(&mut self, wave: &[f32]) {
        let (w, h) = (self.width, self.height);
        if wave.is_empty() || w < 2 {
            return;
        }
        let colours = palette::visual();
        let bright = palette::text_bright();
        let light = ((self.level - 0.1) / 0.8).clamp(0.0, 1.0);
        let weight = GAIN * (0.25 + 0.75 * light);
        let mid = h as f32 / 2.0;
        let swing = WAVE_HEIGHT * mid;
        let flare = self.kick * 0.6;
        let mut last = None;
        for x in 0..w {
            let s = wave[x * wave.len() / w].clamp(-1.0, 1.0);
            let place = x as f32 / (w - 1) as f32;
            let carrier = (TAU * (CARRIER_WAVES * place - CARRIER_HZ * self.time)).sin();
            let y = mid + CARRIER * mid * carrier - s * swing;
            let level = if colours.by_level() { s.abs() } else { light };
            let colour = colours.at(level, place).lerp_to_gamma(bright, flare);
            let rgb = [colour.r(), colour.g(), colour.b()].map(|c| f32::from(c) / 255.0);
            let from = last.unwrap_or(y);
            let n = ((y - from).abs().ceil() as usize).max(1);
            let k = weight / n as f32;
            for j in 0..n {
                let yy = from + (y - from) * (j as f32 + 0.5) / n as f32;
                splat(&mut self.pixels, w, h, x, yy - 0.5, rgb, k);
            }
            last = Some(y);
        }
        self.lit = true;
    }

    pub fn paint(&mut self, painter: &Painter, rect: Rect) {
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            return;
        }
        let ground = palette::list_bg();
        if self.changed || self.texture.is_none() || self.ground != ground {
            // Over the ground; light past full coverage keeps its colour.
            let mut image = ColorImage::filled([w, h], ground);
            for (out, p) in image.pixels.iter_mut().zip(&self.pixels) {
                if p[3] <= 0.0 {
                    continue;
                }
                let f = 1.0 / p[3].max(1.0);
                let a = p[3].min(1.0);
                let c = |i: usize, g: u8| {
                    (f32::from(g) * (1.0 - a) + p[i] * f * 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8
                };
                *out = Color32::from_rgb(c(0, ground.r()), c(1, ground.g()), c(2, ground.b()));
            }
            match &mut self.texture {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => {
                    self.texture = Some(painter.ctx().load_texture(
                        "vortex",
                        image,
                        TextureOptions::LINEAR,
                    ));
                }
            }
            self.changed = false;
            self.ground = ground;
        }
        if let Some(t) = &self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(t.id(), rect, uv, Color32::WHITE);
        }
    }
}

/// The picture's pixels for a display of `size` points: the display's
/// shape, within `PIXELS` and `MAX_WIDTH`.
fn pixels_for(size: Vec2) -> (usize, usize) {
    if size.x < 1.0 || size.y < 1.0 {
        return (0, 0);
    }
    let scale = (PIXELS / (size.x * size.y))
        .sqrt()
        .min(MAX_WIDTH / size.x)
        .min(1.0);
    (
        (size.x * scale).round().max(1.0) as usize,
        (size.y * scale).round().max(1.0) as usize,
    )
}

/// The picture at (`fx`, `fy`), between the four pixels round it; nothing
/// outside it.
fn sample(pixels: &[[f32; 4]], w: usize, h: usize, fx: f32, fy: f32) -> [f32; 4] {
    if fx <= -1.0 || fy <= -1.0 || fx >= w as f32 || fy >= h as f32 {
        return [0.0; 4];
    }
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (x0, y0) = (x0 as isize, y0 as isize);
    let at = |x: isize, y: isize| {
        if x < 0 || y < 0 || x >= w as isize || y >= h as isize {
            [0.0; 4]
        } else {
            pixels[y as usize * w + x as usize]
        }
    };
    let (a, b, c, d) = (
        at(x0, y0),
        at(x0 + 1, y0),
        at(x0, y0 + 1),
        at(x0 + 1, y0 + 1),
    );
    let mut out = [0.0; 4];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty;
    }
    out
}

/// `k` of `rgb` added at column `x`, height `y`, shared between the two
/// rows about it.
fn splat(pixels: &mut [[f32; 4]], w: usize, h: usize, x: usize, y: f32, rgb: [f32; 3], k: f32) {
    let y0 = y.floor();
    let t = y - y0;
    let y0 = y0 as isize;
    for (row, share) in [(y0, 1.0 - t), (y0 + 1, t)] {
        if row < 0 || row >= h as isize || share <= 0.0 {
            continue;
        }
        let p = &mut pixels[row as usize * w + x];
        let k = k * share;
        p[0] += rgb[0] * k;
        p[1] += rgb[1] * k;
        p[2] += rgb[2] * k;
        p[3] += k;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn the_picture_fits_the_display() {
        assert_eq!(pixels_for(vec2(200.0, 60.0)), (200, 60));
        let (w, h) = pixels_for(vec2(700.0, 118.0));
        assert!((w * h) as f32 <= PIXELS * 1.01, "{w}x{h}");
        assert!((w as f32 / h as f32 - 700.0 / 118.0).abs() < 0.1);
        let (w, _) = pixels_for(vec2(4000.0, 20.0));
        assert!(w as f32 <= MAX_WIDTH);
        assert_eq!(pixels_for(vec2(0.0, 50.0)), (0, 0));
    }

    #[test]
    fn a_wave_leaves_a_trail_that_fades_out() {
        let mut v = Vortex::new();
        let size = vec2(200.0, 60.0);
        let levels = vec![0.5; BANDS];
        let flat = vec![0.0; 1000];
        // Nothing new heard: nothing drawn.
        v.feed(Some((&flat, &levels)), false, 0.02, size);
        assert!(!v.active());
        // A flat wave lights a thin line within the carrier's reach of the
        // middle, and nothing else.
        v.feed(Some((&flat, &levels)), true, 0.02, size);
        assert!(v.active());
        let lit_rows = |v: &Vortex, x: usize| -> Vec<usize> {
            (0..v.height)
                .filter(|&y| v.pixels[y * v.width + x][3] > 0.0)
                .collect()
        };
        let rows = lit_rows(&v, 100);
        assert!(!rows.is_empty() && rows.len() <= 3, "{rows:?}");
        let reach = (CARRIER * 30.0) as usize + 2;
        assert!(rows.iter().all(|&y| y.abs_diff(30) <= reach), "{rows:?}");
        // Half a second on: the line has run on its carrier and its trail
        // flown outwards: many rows lit.
        for _ in 0..25 {
            v.feed(Some((&flat, &levels)), true, 0.02, size);
        }
        let rows = lit_rows(&v, 100);
        assert!(rows.len() >= 8, "{rows:?}");
        // Paused, the picture stands.
        let before = v.pixels.clone();
        v.feed(Some((&flat, &levels)), false, 0.02, size);
        assert_eq!(v.pixels, before);
        // Stopped, it fades until nothing is left.
        let light = |v: &Vortex| v.pixels.iter().map(|p| p[3]).sum::<f32>();
        let lit = light(&v);
        for _ in 0..50 {
            v.feed(None, true, 0.02, size);
        }
        assert!(light(&v) < lit / 2.0);
        for _ in 0..400 {
            v.feed(None, true, 0.02, size);
        }
        assert!(!v.active());
    }

    #[test]
    fn a_beat_kicks() {
        let mut v = Vortex::new();
        let size = vec2(200.0, 60.0);
        let flat = vec![0.0; 1000];
        let quiet = vec![0.1; BANDS];
        for _ in 0..20 {
            v.feed(Some((&flat, &quiet)), true, 0.02, size);
        }
        assert_eq!(v.kick, 0.0);
        let mut beat = vec![0.3; BANDS];
        beat[..BANDS / 6].fill(0.9);
        v.feed(Some((&flat, &beat)), true, 0.02, size);
        assert!(v.kick > 0.0);
        // It turns the picture the other way than the next one will.
        assert_eq!(v.way, -1.0);
    }
}
