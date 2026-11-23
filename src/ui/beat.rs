//! What the newer scenes share: the beats of the analyzer's
//! levels (the bass rising over its recent mean, as lightning's and the
//! vortex's), and a small random generator (xorshift, no dependency);
//! the soft lights the scenes paint with (`glow`, `ribbon`, `ellipse`,
//! `ring`).

use egui::{Color32, Mesh, Pos2, Vec2, vec2};
use std::f32::consts::TAU;

/// A beat: the bass this far over its mean of the last `MEAN_S`, and at
/// least `LEAST`, `GAP_S` after the last; a kick fades in `KICK_S`.
const RISE: f32 = 0.06;
const LEAST: f32 = 0.3;
const GAP_S: f32 = 0.12;
const MEAN_S: f32 = 0.4;
const KICK_S: f32 = 0.3;
/// The levels shown fall the whole way in this long.
const FALL_S: f32 = 0.3;

#[derive(Default)]
pub struct Beat {
    /// The bass and the overall level shown, falling at `FALL_S`.
    pub bass: f32,
    pub level: f32,
    /// The last beat's height, fading to 0 in `KICK_S`.
    pub kick: f32,
    mean: f32,
    since: f32,
}

impl Beat {
    /// Moves on by `dt` seconds with the levels heard now (`None`:
    /// nothing): the strength of a beat struck now (0.4–1), if one is.
    pub fn feed(&mut self, levels: Option<&[f32]>, dt: f32) -> Option<f32> {
        let (bass, level) = levels.filter(|l| !l.is_empty()).map_or((0.0, 0.0), |l| {
            let low = (l.len() / 6).max(1);
            let mean = |s: &[f32]| s.iter().sum::<f32>() / s.len() as f32;
            (mean(&l[..low]), mean(l))
        });
        self.bass = bass.max(self.bass - dt / FALL_S).max(0.0);
        self.level = level.max(self.level - dt / FALL_S).max(0.0);
        self.kick = (self.kick - dt / KICK_S).max(0.0);
        self.since += dt;
        let rise = bass - self.mean;
        self.mean += (bass - self.mean) * (dt / MEAN_S).min(1.0);
        if levels.is_some() && rise > RISE && bass > LEAST && self.since > GAP_S {
            self.since = 0.0;
            let strength = (0.5 + (rise - RISE) * 8.0).clamp(0.4, 1.0);
            self.kick = strength;
            return Some(strength);
        }
        None
    }
}

/// A note starting in a band: the band over `ONSET_FROM` and
/// `ONSET_RISE` over its mean of the last `ONSET_MEAN_S`, `ONSET_GAP_S`
/// after the band's last; the mean then takes the level, so a note held
/// starts only once, and only a further rise starts another.
pub const ONSET_FROM: f32 = 0.35;
const ONSET_RISE: f32 = 0.07;
const ONSET_MEAN_S: f32 = 0.3;
const ONSET_GAP_S: f32 = 0.12;

/// The notes starting in each band of the analyzer's levels (the harp's
/// plucks, the ripples' drops, the fireworks' rockets).
#[derive(Default)]
pub struct Onsets {
    mean: Vec<f32>,
    since: Vec<f32>,
}

impl Onsets {
    /// Moves on by `seconds` with the `n` bands' levels heard now (`None`:
    /// nothing, the means fall): the strength (0.4–1) of a note starting
    /// now in each band, 0 where none does. The first levels, or a count
    /// of bands other than the last, only set the means, so not every
    /// band starts a note at once.
    pub fn feed(&mut self, levels: Option<&[f32]>, n: usize, seconds: f32) -> Vec<f32> {
        let level = |k: usize| levels.and_then(|l| l.get(k)).copied().unwrap_or(0.0);
        if self.mean.len() != n {
            self.mean = (0..n).map(level).collect();
            self.since = vec![ONSET_GAP_S; n];
            return vec![0.0; n];
        }
        let follow = (seconds / ONSET_MEAN_S).min(1.0);
        (0..n)
            .map(|k| {
                let raw = level(k);
                let rise = raw - self.mean[k];
                self.mean[k] += rise * follow;
                self.since[k] += seconds;
                if levels.is_some()
                    && raw > ONSET_FROM
                    && rise > ONSET_RISE
                    && self.since[k] > ONSET_GAP_S
                {
                    self.since[k] = 0.0;
                    self.mean[k] = raw;
                    (0.4 + (rise - ONSET_RISE) * 6.0).clamp(0.4, 1.0)
                } else {
                    0.0
                }
            })
            .collect()
    }
}

/// Xorshift64: enough for flames and sparks.
pub struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        Rng(0x9e37_79b9_7f4a_7c15)
    }
}

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number in 0..1.
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A number in `from..to`.
    pub fn between(&mut self, from: f32, to: f32) -> f32 {
        from + (to - from) * self.unit()
    }
}

/// `levels` read at `place` (0: the first, 1: the last), between the two
/// about it.
pub fn level_at(levels: &[f32], place: f32) -> f32 {
    match levels.len() {
        0 => 0.0,
        1 => levels[0],
        n => {
            let f = place.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (f as usize).min(n - 2);
            let t = f - i as f32;
            levels[i] * (1.0 - t) + levels[i + 1] * t
        }
    }
}

/// A colour added to what is under it (premultiplied with no coverage):
/// light on a dark ground. On a light one, laid over it as usual.
pub fn glow(colour: egui::Color32, alpha: f32, dark: bool) -> egui::Color32 {
    let c = colour.gamma_multiply(alpha.clamp(0.0, 1.0));
    if dark {
        egui::Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), 0)
    } else {
        c
    }
}

/// A line through `points`, `half(i)` of its width either side of point
/// `i`, `colour(i, p)` along its middle: fading to nothing at its sides
/// (`soft`), else solid with an edge of `FEATHER` fading out.
pub fn ribbon(
    mesh: &mut Mesh,
    points: &[Pos2],
    half: impl Fn(usize) -> f32,
    colour: impl Fn(usize, Pos2) -> Color32,
    soft: bool,
) {
    const FEATHER: f32 = 0.7;
    let n = points.len();
    if n < 2 {
        return;
    }
    let across = if soft { 3 } else { 4 };
    let base = mesh.vertices.len() as u32;
    for (i, &p) in points.iter().enumerate() {
        let way = points[(i + 1).min(n - 1)] - points[i.saturating_sub(1)];
        let normal = if way.length() > 1e-3 {
            way.normalized().rot90()
        } else {
            vec2(0.0, 1.0)
        };
        let h = half(i).max(0.05);
        let c = colour(i, p);
        if soft {
            mesh.colored_vertex(p - normal * h, Color32::TRANSPARENT);
            mesh.colored_vertex(p, c);
            mesh.colored_vertex(p + normal * h, Color32::TRANSPARENT);
        } else {
            mesh.colored_vertex(p - normal * (h + FEATHER), Color32::TRANSPARENT);
            mesh.colored_vertex(p - normal * h, c);
            mesh.colored_vertex(p + normal * h, c);
            mesh.colored_vertex(p + normal * (h + FEATHER), Color32::TRANSPARENT);
        }
    }
    for i in 0..n as u32 - 1 {
        let a = base + across * i;
        let b = a + across;
        for k in 0..across - 1 {
            mesh.add_triangle(a + k, a + k + 1, b + k + 1);
            mesh.add_triangle(a + k, b + k + 1, b + k);
        }
    }
}

/// A soft light at `centre`, `colour` there fading to nothing at `radii`.
pub fn ellipse(mesh: &mut Mesh, centre: Pos2, radii: Vec2, colour: Color32) {
    const SIDES: u32 = 24;
    if radii.min_elem() < 0.5 || colour == Color32::TRANSPARENT {
        return;
    }
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(centre, colour);
    for i in 0..SIDES {
        let a = TAU * i as f32 / SIDES as f32;
        mesh.colored_vertex(
            centre + vec2(a.cos() * radii.x, a.sin() * radii.y),
            Color32::TRANSPARENT,
        );
        mesh.add_triangle(base, base + 1 + i, base + 1 + (i + 1) % SIDES);
    }
}

/// A soft ring about `centre`, `radius` to its middle, `colour` there
/// fading to nothing `half` inside and outside it.
pub fn ring(mesh: &mut Mesh, centre: Pos2, radius: f32, half: f32, colour: Color32) {
    const SIDES: u32 = 48;
    if radius < 0.5 || half < 0.1 || colour == Color32::TRANSPARENT {
        return;
    }
    let base = mesh.vertices.len() as u32;
    for i in 0..SIDES {
        let a = TAU * i as f32 / SIDES as f32;
        let way = vec2(a.cos(), a.sin());
        mesh.colored_vertex(centre + way * (radius - half).max(0.0), Color32::TRANSPARENT);
        mesh.colored_vertex(centre + way * radius, colour);
        mesh.colored_vertex(centre + way * (radius + half), Color32::TRANSPARENT);
    }
    for i in 0..SIDES {
        let a = base + 3 * i;
        let b = base + 3 * ((i + 1) % SIDES);
        for k in 0..2 {
            mesh.add_triangle(a + k, a + k + 1, b + k + 1);
            mesh.add_triangle(a + k, b + k + 1, b + k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beat_is_the_bass_rising() {
        let mut beat = Beat::default();
        let quiet = vec![0.1; 24];
        for _ in 0..30 {
            assert_eq!(beat.feed(Some(&quiet), 0.02), None);
        }
        let mut loud = vec![0.3; 24];
        loud[..4].fill(0.9);
        assert!(beat.feed(Some(&loud), 0.02).is_some());
        assert!(beat.kick > 0.0);
        // Not again at once.
        assert_eq!(beat.feed(Some(&loud), 0.02), None);
        // Nothing heard: no beats, the levels fall.
        for _ in 0..30 {
            assert_eq!(beat.feed(None, 0.02), None);
        }
        assert_eq!((beat.bass, beat.level, beat.kick), (0.0, 0.0, 0.0));
    }

    #[test]
    fn a_note_starts_once_where_its_band_rises() {
        let mut o = Onsets::default();
        let quiet = vec![0.5; 8];
        // The first levels only set the means.
        assert!(o.feed(Some(&quiet), 8, 0.02).iter().all(|&s| s == 0.0));
        for _ in 0..20 {
            assert!(o.feed(Some(&quiet), 8, 0.02).iter().all(|&s| s == 0.0));
        }
        let mut note = quiet.clone();
        note[3] = 0.9;
        let starts = o.feed(Some(&note), 8, 0.02);
        assert!(starts[3] >= 0.4);
        assert!(starts.iter().enumerate().all(|(k, &s)| k == 3 || s == 0.0));
        // Held: not again.
        for _ in 0..50 {
            assert_eq!(o.feed(Some(&note), 8, 0.02)[3], 0.0);
        }
        // Nothing heard: nothing starts.
        assert!(o.feed(None, 8, 0.02).iter().all(|&s| s == 0.0));
    }

    #[test]
    fn levels_between_bands() {
        assert_eq!(level_at(&[], 0.5), 0.0);
        assert_eq!(level_at(&[0.0, 1.0], 0.25), 0.25);
        assert_eq!(level_at(&[0.0, 1.0, 0.0], 1.0), 0.0);
        assert_eq!(level_at(&[0.2, 0.4], 7.0), 0.4);
        let mut rng = Rng::default();
        assert!((0..1000).all(|_| (0.0..1.0).contains(&rng.unit())));
    }
}
