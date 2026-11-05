//! Goniometer: the stereo field as a studio's meter draws it. Each frame
//! heard is a point: up by its mid (the channels added), across by its
//! side (their difference), the left channel alone along the upper left
//! diagonal and the right along the upper right; a mono sound is a line
//! straight up, a wide one a cloud, and channels out of phase lie across.
//! The points are joined and laid as light into a square picture that
//! fades as a phosphor does, so the trace glows and trails off. The
//! picture is scaled to the loudness heard of late (up to `MOST_GAIN`
//! times), so quiet music fills it too. Beside it, two meters: the
//! balance of the channels and their correlation (+1: one sound, 0: two
//! unrelated, -1: out of phase). Paused, it stands; stopped, it fades.

use egui::{
    Align2, Color32, ColorImage, FontId, Painter, Rect, Stroke, TextureHandle, TextureOptions,
    Vec2, pos2, vec2,
};

use crate::palette;

/// The picture's side in pixels at most (a pixel a point up to it).
const MOST_SIDE: f32 = 200.0;
/// The light halves this often.
const HALF_S: f32 = 0.07;
/// The light a frame's trace leaves at most per pixel of its length.
const LIGHT: f32 = 0.5;
/// The loudness followed falls by half in `PEAK_HALF_S`; the picture is
/// scaled up to `MOST_GAIN` times to fill `FILL` of it.
const PEAK_HALF_S: f32 = 1.5;
const MOST_GAIN: f32 = 6.0;
const FILL: f32 = 0.85;
/// The meters follow in about this long.
const METER_S: f32 = 0.3;
/// The meters' width at most, and the room they need at least.
const METER_WIDTH: f32 = 150.0;
const METER_LEAST: f32 = 70.0;

pub struct Goniometer {
    /// Premultiplied colour and light, row by row from the top.
    pixels: Vec<[f32; 4]>,
    side: usize,
    peak: f32,
    /// The correlation (-1 to 1) and the balance (-1: left, 1: right).
    correlation: f32,
    balance: f32,
    lit: bool,
    texture: Option<TextureHandle>,
    changed: bool,
    ground: Color32,
}

impl Default for Goniometer {
    fn default() -> Self {
        Self::new()
    }
}

impl Goniometer {
    pub fn new() -> Goniometer {
        Goniometer {
            pixels: Vec::new(),
            side: 0,
            peak: 0.0,
            correlation: 0.0,
            balance: 0.0,
            lit: false,
            texture: None,
            changed: false,
            ground: Color32::TRANSPARENT,
        }
    }

    /// Moves on by `dt` seconds with the frames heard since the frame
    /// before (their mid and side; `None`: nothing heard; `moved` false:
    /// paused, it stands) for a display of `size`.
    pub fn feed(&mut self, heard: Option<(&[f32], &[f32])>, moved: bool, dt: f32, size: Vec2) {
        let side = side_for(size);
        if side != self.side {
            self.side = side;
            self.pixels = vec![[0.0; 4]; side * side];
            self.lit = false;
            self.changed = true;
        }
        if side < 8 || (heard.is_some() && !moved) {
            return;
        }
        let dim = 0.5f32.powf(dt / HALF_S);
        if self.lit {
            let mut lit = false;
            for p in &mut self.pixels {
                if p[3] > 0.0 {
                    let left = p[3] * dim - 0.01;
                    if left <= 0.0 {
                        *p = [0.0; 4];
                    } else {
                        let f = left / p[3];
                        p.iter_mut().for_each(|c| *c *= f);
                        lit = true;
                    }
                }
            }
            self.lit = lit;
            self.changed = true;
        }
        self.peak *= 0.5f32.powf(dt / PEAK_HALF_S);
        let follow = (dt / METER_S).min(1.0);
        let Some((mid, wide)) = heard.filter(|(m, s)| !m.is_empty() && m.len() == s.len()) else {
            self.correlation -= self.correlation * follow;
            self.balance -= self.balance * follow;
            return;
        };
        let loudest = mid
            .iter()
            .zip(wide)
            .map(|(m, s)| m.abs() + s.abs())
            .fold(0.0, f32::max);
        self.peak = self.peak.max(loudest);
        let (mut lr, mut ll, mut rr) = (0.0f32, 0.0f32, 0.0f32);
        for (&m, &s) in mid.iter().zip(wide) {
            let (l, r) = (m + s, m - s);
            lr += l * r;
            ll += l * l;
            rr += r * r;
        }
        if ll + rr > 1e-6 {
            let correlation = if ll * rr > 1e-12 {
                lr / (ll * rr).sqrt()
            } else {
                // One channel silent: unrelated.
                0.0
            };
            self.correlation += (correlation.clamp(-1.0, 1.0) - self.correlation) * follow;
            self.balance += ((rr - ll) / (ll + rr) - self.balance) * follow;
        }
        self.draw(mid, wide);
    }

    /// The frames' trace into the picture, each joined to the one before.
    fn draw(&mut self, mid: &[f32], wide: &[f32]) {
        let n = self.side;
        let half = n as f32 / 2.0;
        let gain = (FILL / self.peak.max(1e-4)).clamp(1.0, MOST_GAIN) * half;
        let colours = palette::visual();
        let weight = (LIGHT * n as f32 * 4.0 / mid.len() as f32).min(LIGHT);
        let mut last: Option<(f32, f32)> = None;
        for (&m, &s) in mid.iter().zip(wide) {
            // The left channel alone up the left diagonal.
            let x = half - s * gain;
            let y = half - m * gain;
            let reach = ((s.abs() + m.abs()) * gain / half).min(1.0);
            let place = (x / n as f32).clamp(0.0, 1.0);
            let c = colours.at(0.4 + 0.6 * reach, place);
            let rgb = [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
            let (fx, fy) = last.unwrap_or((x, y));
            let steps = ((x - fx).abs().max((y - fy).abs()).ceil() as usize).clamp(1, n);
            let k = weight / steps as f32 * steps.min(3) as f32;
            for j in 1..=steps {
                let t = j as f32 / steps as f32;
                self.splat(fx + (x - fx) * t, fy + (y - fy) * t, rgb, k);
            }
            last = Some((x, y));
        }
        self.lit = true;
        self.changed = true;
    }

    /// `k` of `rgb` at (`x`, `y`), shared among the four pixels about it.
    fn splat(&mut self, x: f32, y: f32, rgb: [f32; 3], k: f32) {
        let n = self.side as isize;
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        for (dx, dy, share) in [
            (0, 0, (1.0 - tx) * (1.0 - ty)),
            (1, 0, tx * (1.0 - ty)),
            (0, 1, (1.0 - tx) * ty),
            (1, 1, tx * ty),
        ] {
            let (px, py) = (x0 as isize + dx, y0 as isize + dy);
            if px < 0 || py < 0 || px >= n || py >= n || share <= 0.0 {
                continue;
            }
            let p = &mut self.pixels[(py * n + px) as usize];
            let k = k * share;
            p[0] += rgb[0] * k;
            p[1] += rgb[1] * k;
            p[2] += rgb[2] * k;
            p[3] += k;
        }
    }

    /// Something still glows: keep repainting.
    pub fn active(&self) -> bool {
        self.lit || self.correlation.abs() > 0.01 || self.balance.abs() > 0.01
    }

    pub fn paint(&mut self, painter: &Painter, rect: Rect) {
        let n = self.side;
        if n == 0 {
            return;
        }
        let colours = palette::visual();
        let ground = palette::list_bg();
        let line = palette::text_weak().gamma_multiply(0.35);
        let d = rect.height().min(rect.width());
        let square = Rect::from_center_size(rect.center(), vec2(d, d));
        if self.changed || self.texture.is_none() || self.ground != ground {
            let mut image = ColorImage::filled([n, n], ground);
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
                        "goniometer",
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
            painter.image(t.id(), square, uv, Color32::WHITE);
        }
        // The guides: the diamond of full scale, the mid's line up, the
        // channels' diagonals, named at their ends.
        let c = square.center();
        let r = d / 2.0 - 1.0;
        let (top, bottom) = (c - vec2(0.0, r), c + vec2(0.0, r));
        let (left, right) = (c - vec2(r, 0.0), c + vec2(r, 0.0));
        let stroke = Stroke::new(1.0, line);
        for (a, b) in [(top, right), (right, bottom), (bottom, left), (left, top)] {
            painter.line_segment([a, b], stroke);
        }
        painter.line_segment([top, bottom], stroke);
        painter.line_segment([c - vec2(r, r) / 2.0, c + vec2(r, r) / 2.0], stroke);
        painter.line_segment([c + vec2(r, -r) / 2.0, c + vec2(-r, r) / 2.0], stroke);
        let font = FontId::proportional(10.0);
        let weak = palette::text_weak();
        if d >= 40.0 {
            painter.text(
                c - vec2(r, r) / 2.0 - vec2(3.0, 3.0),
                Align2::RIGHT_BOTTOM,
                "L",
                font.clone(),
                weak,
            );
            painter.text(
                c + vec2(r, -r) / 2.0 + vec2(3.0, -3.0),
                Align2::LEFT_BOTTOM,
                "R",
                font.clone(),
                weak,
            );
        }

        // The meters either side, as room allows.
        let room = (rect.width() - d) / 2.0 - 24.0;
        if room < METER_LEAST {
            return;
        }
        let width = room.min(METER_WIDTH);
        let y = rect.center().y;
        let meters = [
            (
                square.left() - 16.0 - width,
                tr!("Balance", "Баланс"),
                ("L", "R"),
                self.balance,
            ),
            (
                square.right() + 16.0,
                tr!("Correlation", "Корреляция"),
                ("−1", "+1"),
                self.correlation,
            ),
        ];
        for (x, name, (from, to), value) in meters {
            let track = Rect::from_min_size(pos2(x, y - 2.0), vec2(width, 4.0));
            painter.rect_filled(track, 2.0, palette::track());
            let mid = track.center().x;
            painter.line_segment(
                [pos2(mid, y - 6.0), pos2(mid, y + 6.0)],
                Stroke::new(1.0, weak),
            );
            let at = mid + value.clamp(-1.0, 1.0) * width / 2.0;
            let place = (at - track.left()) / width;
            let colour = colours.at(0.5 + 0.5 * value.abs(), place);
            painter.rect_filled(
                Rect::from_min_max(pos2(mid.min(at), y - 2.0), pos2(mid.max(at), y + 2.0)),
                2.0,
                colour.gamma_multiply(0.6),
            );
            painter.circle_filled(pos2(at, y), 4.0, colour);
            painter.text(
                pos2(mid, y - 10.0),
                Align2::CENTER_BOTTOM,
                name,
                font.clone(),
                weak,
            );
            painter.text(
                pos2(track.left(), y + 8.0),
                Align2::LEFT_TOP,
                from,
                font.clone(),
                weak,
            );
            painter.text(
                pos2(track.right(), y + 8.0),
                Align2::RIGHT_TOP,
                to,
                font.clone(),
                weak,
            );
        }
    }
}

/// The picture's side for a display of `size`: its height (or width),
/// a pixel a point up to `MOST_SIDE`.
fn side_for(size: Vec2) -> usize {
    size.x.min(size.y).clamp(0.0, MOST_SIDE) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, left: f32, right: f32) -> (Vec<f32>, Vec<f32>) {
        (0..n)
            .map(|i| {
                let s = (i as f32 * 0.05).sin() * 0.5;
                let (l, r) = (s * left, s * right);
                ((l + r) / 2.0, (l - r) / 2.0)
            })
            .unzip()
    }

    /// How the lit pixels lie: the mean of their offsets across times
    /// down from the middle (positive: along the upper left diagonal),
    /// and the furthest across.
    fn lit_across(g: &Goniometer) -> (f32, f32) {
        let n = g.side;
        let mid = n as f32 / 2.0;
        let (mut sum, mut weight, mut far) = (0.0, 0.0, 0.0f32);
        for (i, p) in g.pixels.iter().enumerate() {
            if p[3] > 0.0 {
                let x = (i % n) as f32 + 0.5 - mid;
                let y = (i / n) as f32 + 0.5 - mid;
                sum += x * y * p[3];
                weight += p[3];
                far = far.max(x.abs());
            }
        }
        (sum / weight.max(1e-9), far)
    }

    #[test]
    fn mono_stands_up_and_the_left_leans_left() {
        let size = vec2(400.0, 100.0);
        let mut g = Goniometer::new();
        let (m, s) = tone(900, 1.0, 1.0);
        for _ in 0..50 {
            g.feed(Some((&m, &s)), true, 0.02, size);
        }
        let (_, far) = lit_across(&g);
        assert!(far <= 1.5, "mono spreads {far}");
        assert!(g.correlation > 0.9, "{}", g.correlation);
        assert!(g.balance.abs() < 0.05);

        let mut g = Goniometer::new();
        let (m, s) = tone(900, 1.0, 0.0);
        for _ in 0..50 {
            g.feed(Some((&m, &s)), true, 0.02, size);
        }
        // The left alone: on the left diagonal, the balance to the left.
        let (lean, far) = lit_across(&g);
        assert!(far > 10.0, "{far}");
        assert!(lean > 25.0, "{lean}");
        assert!(g.balance < -0.5, "{}", g.balance);
        assert!(g.correlation.abs() < 0.1, "{}", g.correlation);

        let mut g = Goniometer::new();
        let (m, s) = tone(900, 1.0, -1.0);
        for _ in 0..50 {
            g.feed(Some((&m, &s)), true, 0.02, size);
        }
        assert!(g.correlation < -0.9, "{}", g.correlation);
    }

    #[test]
    fn it_stands_paused_and_fades_stopped() {
        let size = vec2(400.0, 100.0);
        let mut g = Goniometer::new();
        let (m, s) = tone(900, 1.0, 0.5);
        g.feed(Some((&m, &s)), true, 0.02, size);
        assert!(g.active());
        let before = g.pixels.clone();
        g.feed(Some((&m, &s)), false, 0.02, size);
        assert_eq!(g.pixels, before);
        for _ in 0..100 {
            g.feed(None, true, 0.02, size);
        }
        assert!(!g.active());
    }
}
