//! Lightning: a glowing core in the middle of the display that swells
//! with the bass, crackling with short tendrils as many as the treble is
//! loud (a plasma globe's); faint arcs flickering from it to both ends
//! while there is sound, drawn anew every `ARC_S`; and on each beat (the
//! bass rising over its recent mean, `beat::Beat`) bolts striking from it
//! to the display's edges, as real lightning does: a leader grows from the
//! core to the edge in `LEADER_S` with a bright tip, its branches growing
//! from it as it passes (and branches of branches), then the return
//! stroke lights the whole channel white-hot, lights the sky about it,
//! throws sparks where it hits the edge, and flickers once or twice more
//! along the same channel (`Bolt::pulses`) before its glow fades; a loud
//! passage without beats strikes now and then too. A bolt is a path of
//! midpoint displacement: each span's middle moved across it by a random
//! share of its length, `DEPTH` times over. Painted as ribbons narrowing
//! to their tips: a wide soft halo, a coloured glow, a hot core, added to
//! the ground on a dark theme (`beat::glow`), so where they cross they
//! brighten. Fed the analyzer's levels (`spectrum::Analyzer::levels`).

use egui::{Color32, Mesh, Painter, Pos2, Rect, Vec2, pos2, vec2};
use std::f32::consts::TAU;

use crate::palette;
use crate::ui::beat::{Beat, Rng, glow};

/// The bands of the levels fed.
pub const BANDS: usize = 32;
/// The arcs are drawn anew this often; under this overall level there
/// are none.
const ARC_S: f32 = 0.07;
const ARC_FROM: f32 = 0.15;
/// The core's tendrils are drawn anew this often, at most `TENDRILS` of
/// them with the treble loud, none under `TENDRIL_FROM`.
const TENDRIL_S: f32 = 0.05;
const TENDRILS: f32 = 7.0;
const TENDRIL_FROM: f32 = 0.12;
/// A passage louder than `LOUD` strikes every `LOUD_S` without beats.
const LOUD: f32 = 0.55;
const LOUD_S: f32 = 0.7;
/// How many times a bolt's spans are halved, and how far a span's middle
/// moves across it, as a share of its length.
const DEPTH: u32 = 5;
const ROUGHNESS: f32 = 0.22;
/// The leader reaches the edge in this long; then the return stroke.
const LEADER_S: f32 = 0.04;
/// A stroke's white-hot light halves in about `PULSE_S` × 0.7.
const PULSE_S: f32 = 0.045;
/// The sky's light about a stroke, as a share of its own.
const SKY: f32 = 0.3;
/// Sparks thrown where a bolt hits: at most `SPARKS` for the strongest,
/// falling at `GRAVITY` points a second² (at the reference height),
/// slowed by `DRAG` a second; never more than `MOST_SPARKS` at once.
const SPARKS: f32 = 18.0;
const GRAVITY: f32 = 420.0;
const DRAG: f32 = 2.5;
const MOST_SPARKS: usize = 240;
/// The full bar's display height, which the widths and speeds are for.
const REFERENCE_HEIGHT: f32 = 118.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Strike,
    Arc,
    Tendril,
}

struct Path {
    /// In points from the display's top left.
    points: Vec<Pos2>,
    /// When the leader starts down it, seconds after the strike.
    start: f32,
    /// Its width where it starts, a share of the main path's.
    width: f32,
}

struct Bolt {
    kind: Kind,
    /// The main path first, then its branches.
    paths: Vec<Path>,
    /// The leader's time from one point to the next.
    step: f32,
    age: f32,
    life: f32,
    strength: f32,
    /// The strokes along the channel: seconds after the return stroke,
    /// and their light (the first at 0, at 1).
    pulses: Vec<(f32, f32)>,
    /// Where the main path meets the edge and the way into the display:
    /// sparks are thrown there at the return stroke.
    hit: Option<(Pos2, Vec2)>,
    struck: bool,
}

impl Bolt {
    /// Seconds since the return stroke (none: the leader still grows).
    fn after(&self) -> Option<f32> {
        match self.kind {
            Kind::Strike => (self.age >= LEADER_S).then_some(self.age - LEADER_S),
            _ => Some(self.age),
        }
    }

    /// The white-hot light of the channel now (0–1).
    fn hot(&self) -> f32 {
        match (self.kind, self.after()) {
            (Kind::Strike, None) => 0.35,
            (Kind::Strike, Some(t)) => self
                .pulses
                .iter()
                .filter(|(at, _)| t >= *at)
                .map(|(at, h)| h * (-(t - at) / PULSE_S).exp())
                .fold(0.0, f32::max),
            _ => 1.0 - self.age / self.life,
        }
    }

    /// The channel's afterglow, in its colour, fading over the bolt's
    /// life (0–1; none while the leader grows).
    fn rest(&self) -> f32 {
        match (self.kind, self.after()) {
            (Kind::Strike, Some(t)) => {
                let rest = (1.0 - t / (self.life - LEADER_S)).max(0.0);
                rest * rest
            }
            _ => 0.0,
        }
    }
}

struct Spark {
    at: Pos2,
    speed: Vec2,
    age: f32,
    life: f32,
    /// Its place across the display, for its colour.
    place: f32,
}

pub struct Lightning {
    rng: Rng,
    beat: Beat,
    bolts: Vec<Bolt>,
    sparks: Vec<Spark>,
    /// The treble shown (falling as the beat's levels do).
    treble: f32,
    since_strike: f32,
    since_arcs: f32,
    since_tendrils: f32,
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
            rng: Rng::default(),
            beat: Beat::default(),
            bolts: Vec::new(),
            sparks: Vec::new(),
            treble: 0.0,
            since_strike: 0.0,
            since_arcs: 0.0,
            since_tendrils: 0.0,
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
        let scale = (size.y / REFERENCE_HEIGHT).clamp(0.6, 1.5);
        for s in &mut self.sparks {
            s.age += dt;
            s.at += s.speed * dt;
            s.speed.y += GRAVITY * scale * dt;
            s.speed *= (1.0 - DRAG * dt).max(0.0);
        }
        self.sparks.retain(|s| s.age < s.life);
        // The return strokes struck since the frame before throw sparks.
        for i in 0..self.bolts.len() {
            let b = &self.bolts[i];
            if b.kind == Kind::Strike && !b.struck && b.age >= LEADER_S {
                let (hit, strength) = (b.hit, b.strength);
                self.bolts[i].struck = true;
                if let Some((at, inward)) = hit {
                    self.throw_sparks(at, inward, strength, size, scale);
                }
            }
        }
        self.since_strike += dt;
        self.since_arcs += dt;
        self.since_tendrils += dt;
        let struck = self.beat.feed(levels, dt);
        let treble = levels.filter(|l| !l.is_empty()).map_or(0.0, |l| {
            let top = &l[l.len() * 2 / 3..];
            top.iter().sum::<f32>() / top.len() as f32
        });
        self.treble = treble.max(self.treble - dt / 0.3).max(0.0);
        let level = levels.filter(|l| !l.is_empty()).map_or(0.0, |l| {
            l.iter().sum::<f32>() / l.len() as f32
        });
        if levels.is_none() || size.x < 8.0 || size.y < 8.0 {
            return;
        }
        let loud = level > LOUD && self.since_strike > LOUD_S;
        if let Some(strength) = struck.or(loud.then_some(0.4)) {
            self.strike(size, strength);
            self.since_strike = 0.0;
        }
        if self.since_arcs >= ARC_S {
            self.since_arcs = 0.0;
            self.bolts.retain(|b| b.kind != Kind::Arc);
            if level > ARC_FROM {
                for right in [false, true] {
                    self.arc(size, right, level);
                }
            }
        }
        if self.since_tendrils >= TENDRIL_S {
            self.since_tendrils = 0.0;
            self.bolts.retain(|b| b.kind != Kind::Tendril);
            if treble > TENDRIL_FROM && level > ARC_FROM {
                let n = (treble * TENDRILS).round() as usize;
                for _ in 0..n.max(1) {
                    self.tendril(size, treble);
                }
            }
        }
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        !self.bolts.is_empty()
            || !self.sparks.is_empty()
            || self.beat.bass > 0.0
            || self.beat.level > 0.0
    }

    /// Bolts from the middle to the edges, more of them the stronger.
    fn strike(&mut self, size: Vec2, strength: f32) {
        let centre = pos2(size.x / 2.0, size.y / 2.0);
        for _ in 0..1 + (strength * 2.0) as usize {
            self.right = !self.right;
            let side = if self.right { 1.0 } else { -1.0 };
            // Mostly to the end of the display, else to its top or bottom
            // edge far from the middle.
            let (target, inward) = if self.rng.unit() < 0.6 {
                (
                    pos2(
                        centre.x + side * size.x / 2.0,
                        size.y * self.rng.between(0.1, 0.9),
                    ),
                    vec2(-side, 0.0),
                )
            } else {
                let top = self.rng.unit() < 0.5;
                (
                    pos2(
                        centre.x + side * size.x * self.rng.between(0.25, 0.48),
                        if top { 0.0 } else { size.y },
                    ),
                    vec2(0.0, if top { 1.0 } else { -1.0 }),
                )
            };
            let main = self.path(centre, target, DEPTH, ROUGHNESS);
            let step = LEADER_S / (main.len() - 1) as f32;
            let mut paths = vec![Path {
                points: main,
                start: 0.0,
                width: 1.0,
            }];
            for _ in 0..1 + (strength * 2.0 + self.rng.unit() * 1.5) as usize {
                let (at, branch) = self.branch(&paths[0].points, 0.15, 0.75, 0.3, 0.55, DEPTH - 1);
                let start = at as f32 * step;
                // A branch of the branch, now and then.
                if self.rng.unit() < 0.55 {
                    let (sub_at, sub) = self.branch(&branch, 0.25, 0.7, 0.35, 0.7, DEPTH - 2);
                    paths.push(Path {
                        points: sub,
                        start: start + sub_at as f32 * step,
                        width: 0.3,
                    });
                }
                paths.push(Path {
                    points: branch,
                    start,
                    width: 0.55,
                });
            }
            // The strokes along the channel: the return stroke, then
            // once or twice more for the stronger ones.
            let life = 0.22 + 0.24 * strength;
            let mut pulses = vec![(0.0, 1.0)];
            if self.rng.unit() < 0.35 + 0.55 * strength {
                pulses.push((self.rng.between(0.07, 0.14), self.rng.between(0.6, 0.9)));
                if self.rng.unit() < 0.5 * strength {
                    pulses.push((self.rng.between(0.17, 0.24), self.rng.between(0.45, 0.7)));
                }
            }
            self.bolts.push(Bolt {
                kind: Kind::Strike,
                paths,
                step,
                age: 0.0,
                life: LEADER_S + life,
                strength,
                pulses,
                hit: Some((target, inward)),
                struck: false,
            });
        }
    }

    /// A branch off `path`: from a point `from`–`to` of the way along,
    /// turned 15–50° from the way to its end, `least`–`most` of that long.
    /// Its first point's index on `path`, and its points.
    fn branch(
        &mut self,
        path: &[Pos2],
        from: f32,
        to: f32,
        least: f32,
        most: f32,
        depth: u32,
    ) -> (usize, Vec<Pos2>) {
        let n = path.len();
        let at = ((n as f32 * self.rng.between(from, to)) as usize).min(n - 2);
        let start = path[at];
        let way = path[n - 1] - start;
        let turn = self.rng.between(15.0, 50.0).to_radians()
            * if self.rng.unit() < 0.5 { -1.0 } else { 1.0 };
        let (sin, cos) = turn.sin_cos();
        let length = self.rng.between(least, most);
        let end = start + vec2(way.x * cos - way.y * sin, way.x * sin + way.y * cos) * length;
        (at, self.path(start, end, depth, ROUGHNESS * 1.1))
    }

    /// A faint arc from the middle to one end.
    fn arc(&mut self, size: Vec2, right: bool, level: f32) {
        let centre = pos2(size.x / 2.0, size.y / 2.0);
        let x = if right { size.x } else { 0.0 };
        let target = pos2(x, centre.y + size.y * 0.6 * (self.rng.unit() - 0.5));
        let path = self.path(centre, target, DEPTH, ROUGHNESS * 0.8);
        self.bolts.push(Bolt {
            kind: Kind::Arc,
            paths: vec![Path {
                points: path,
                start: 0.0,
                width: 0.45,
            }],
            step: 0.0,
            age: 0.0,
            life: ARC_S * 1.6,
            strength: (level * 0.6).min(0.5),
            pulses: Vec::new(),
            hit: None,
            struck: true,
        });
    }

    /// A short crackling tendril from the core, a plasma globe's.
    fn tendril(&mut self, size: Vec2, treble: f32) {
        let centre = pos2(size.x / 2.0, size.y / 2.0);
        let angle = self.rng.unit() * TAU;
        let reach = size.y.min(size.x) * self.rng.between(0.2, 0.42) * (0.6 + 0.6 * treble);
        let end = centre + vec2(angle.cos() * 1.4, angle.sin()) * reach;
        let path = self.path(centre, end, 3, 0.3);
        self.bolts.push(Bolt {
            kind: Kind::Tendril,
            paths: vec![Path {
                points: path,
                start: 0.0,
                width: 0.4,
            }],
            step: 0.0,
            age: 0.0,
            life: TENDRIL_S * self.rng.between(1.0, 1.8),
            strength: (0.25 + 0.6 * treble).min(0.7),
            pulses: Vec::new(),
            hit: None,
            struck: true,
        });
    }

    /// Sparks thrown into the display from `at`, where a bolt hit.
    fn throw_sparks(&mut self, at: Pos2, inward: Vec2, strength: f32, size: Vec2, scale: f32) {
        let n = (SPARKS * strength) as usize + 3;
        let along = inward.rot90();
        let place = (at.x / size.x).clamp(0.0, 1.0);
        for _ in 0..n {
            let speed = (inward * self.rng.between(40.0, 220.0)
                + along * self.rng.between(-160.0, 160.0)
                + vec2(0.0, -self.rng.between(0.0, 120.0)))
                * scale;
            self.sparks.push(Spark {
                at,
                speed,
                age: 0.0,
                life: self.rng.between(0.25, 0.6),
                place,
            });
        }
        if self.sparks.len() > MOST_SPARKS {
            let over = self.sparks.len() - MOST_SPARKS;
            self.sparks.drain(..over);
        }
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
                let shift = (self.rng.unit() * 2.0 - 1.0) * roughness;
                next.push(p);
                next.push(p + span / 2.0 + across * shift);
            }
            next.push(b);
            points = next;
        }
        points
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual_for(crate::settings::Visual::Lightning);
        let dark = colours.is_dark();
        // Glows lie over a light ground rather than adding to it: softer.
        let haze = if dark { 1.0 } else { 0.55 };
        let thick = (rect.height() / REFERENCE_HEIGHT).clamp(0.7, 1.4);
        let centre = rect.center();
        let offset = rect.min.to_vec2();
        let place = move |p: Pos2| ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        // White-hot on a dark ground; the scheme's deepest on a light one.
        let hot = move |p: f32, heat: f32| {
            if dark {
                colours.at(1.0, p).lerp_to_gamma(Color32::WHITE, 0.15 + 0.8 * heat)
            } else {
                colours.peak(p)
            }
        };
        let mut mesh = Mesh::default();

        // The glow across the display, brighter with the bass and the
        // strokes.
        let flash = self
            .bolts
            .iter()
            .filter(|b| b.kind == Kind::Strike && b.after().is_some())
            .map(|b| b.strength * b.hot())
            .fold(0.0, f32::max);
        let wash = colours.at(1.0, 0.5);
        let a = (0.03 + 0.14 * self.beat.bass + 0.1 * flash) * haze;
        let wash = glow(wash, a, dark);
        for (x0, x1, c0, c1) in [
            (rect.left(), centre.x, Color32::TRANSPARENT, wash),
            (centre.x, rect.right(), wash, Color32::TRANSPARENT),
        ] {
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(pos2(x0, rect.top()), c0);
            mesh.colored_vertex(pos2(x1, rect.top()), c1);
            mesh.colored_vertex(pos2(x1, rect.bottom()), c1);
            mesh.colored_vertex(pos2(x0, rect.bottom()), c0);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }

        // The sky lit about each stroke, and the edge where it hit.
        for b in self.bolts.iter().filter(|b| b.kind == Kind::Strike) {
            let Some(t) = b.after() else {
                continue;
            };
            let light = b.hot() * b.strength;
            let main = &b.paths[0].points;
            let (from, to) = (main[0] + offset, main[main.len() - 1] + offset);
            let middle = from + (to - from) * 0.55;
            let reach = (to - from).length();
            ellipse(
                &mut mesh,
                middle,
                vec2(reach * 0.6, rect.height() * 0.9),
                glow(colours.at(0.8, place(middle)), SKY * light * haze, dark),
            );
            let impact = (1.0 - t / 0.3).max(0.0);
            if impact > 0.0 {
                let r = (10.0 + 22.0 * b.strength) * thick * (1.0 + 1.5 * t);
                ellipse(
                    &mut mesh,
                    to,
                    vec2(r, r),
                    glow(hot(place(to), 0.5), 0.5 * impact * impact * haze, dark),
                );
            }
        }

        // The channels: a wide soft halo, a coloured glow, a hot core,
        // each narrowing to the tips.
        let mut shown = Vec::with_capacity(1 << (DEPTH + 2));
        for b in &self.bolts {
            let leading = b.after().is_none();
            let strike = b.kind == Kind::Strike;
            // The halo's, the glow's and the core's light, and how
            // white-hot the core is: a stroke's white light cools into
            // the channel's colour, the afterglow fading.
            let (h, r, s) = (b.hot(), b.rest(), b.strength);
            let (halo_light, glow_light, core, heat) = match b.kind {
                Kind::Strike if leading => (0.1, 0.4, 0.55, 0.4),
                Kind::Strike => {
                    let k = 0.6 + 0.4 * s;
                    (
                        0.25 * k * (h + 0.35 * r),
                        k * (0.8 * h + 0.4 * r),
                        k * (1.3 * h + 0.1 * r).min(1.0),
                        h,
                    )
                }
                Kind::Arc => (0.12 * s * h, 0.55 * s * h, 0.45 * s * h, 0.0),
                Kind::Tendril => (0.12 * s * h, 0.55 * s * h, 0.8 * s * h, 0.3),
            };
            let hot_light = if strike { h } else { 0.0 };
            let base = if strike { 1.0 + 0.8 * b.strength } else { 1.0 } * thick;
            for path in &b.paths {
                shown.clear();
                let n = path.points.len();
                if leading {
                    let k = (b.age - path.start) / b.step;
                    if k <= 0.0 {
                        continue;
                    }
                    let whole = k as usize;
                    if whole + 1 >= n {
                        shown.extend(path.points.iter().map(|p| *p + offset));
                    } else {
                        shown.extend(path.points[..=whole].iter().map(|p| *p + offset));
                        let (p, q) = (path.points[whole], path.points[whole + 1]);
                        shown.push(p + (q - p) * k.fract() + offset);
                    }
                } else {
                    shown.extend(path.points.iter().map(|p| *p + offset));
                }
                if shown.len() < 2 {
                    continue;
                }
                let last = (n - 1) as f32;
                let taper = |i: usize| 1.0 - 0.65 * (i as f32 / last).min(1.0);
                let w = base * path.width;
                if halo_light > 0.005 {
                    ribbon(
                        &mut mesh,
                        &shown,
                        |i| 8.0 * w * taper(i),
                        |_, p| glow(colours.at(0.8, place(p)), halo_light * haze, dark),
                        true,
                    );
                }
                ribbon(
                    &mut mesh,
                    &shown,
                    |i| 2.8 * w * taper(i),
                    |_, p| glow(colours.at(1.0, place(p)), glow_light, dark),
                    true,
                );
                if core > 0.01 {
                    ribbon(
                        &mut mesh,
                        &shown,
                        |i| 0.55 * w * taper(i) * (0.7 + 0.6 * hot_light),
                        |_, p| glow(hot(place(p), heat), core, dark),
                        false,
                    );
                }
                // The leader's tip, brighter than the channel behind it.
                if leading && let Some(&tip) = shown.last() {
                    ellipse(
                        &mut mesh,
                        tip,
                        Vec2::splat(5.0 * w),
                        glow(colours.at(1.0, place(tip)), 0.55 * haze, dark),
                    );
                    ellipse(
                        &mut mesh,
                        tip,
                        Vec2::splat(1.6 * w),
                        glow(hot(place(tip), 1.0), 1.0, dark),
                    );
                }
            }
        }

        // The sparks: short streaks along their flight, hot heads.
        for s in &self.sparks {
            let t = s.age / s.life;
            let light = (1.0 - t * t) * (1.0 - t).sqrt();
            let head = s.at + offset;
            let tail = head - s.speed * 0.06;
            let c = colours.at(1.0, s.place);
            ribbon(
                &mut mesh,
                &[head, tail],
                |i| if i == 0 { 0.5 * thick } else { 0.1 },
                |i, _| {
                    let c = if i == 0 { hot(s.place, 1.0 - t) } else { c };
                    glow(c, if i == 0 { light } else { 0.0 }, dark)
                },
                false,
            );
            ellipse(
                &mut mesh,
                head,
                vec2(4.0, 4.0) * thick,
                glow(c, 0.3 * light * haze, dark),
            );
        }

        // The core: a glow fading from its middle, a bright heart in it.
        let lit = self.beat.level.max(self.beat.bass);
        let r = (4.0 + 10.0 * self.beat.bass + 5.0 * flash) * thick;
        let heart = hot(0.5, flash);
        ellipse(
            &mut mesh,
            centre,
            vec2(r, r) * 2.4,
            glow(colours.at(1.0, 0.5), (0.35 + 0.4 * lit + 0.4 * flash) * haze, dark),
        );
        ellipse(
            &mut mesh,
            centre,
            vec2(r, r) * 0.9,
            glow(heart, 0.5 + 0.5 * lit.max(flash), dark),
        );
        if lit > 0.0 {
            ellipse(
                &mut mesh,
                centre,
                Vec2::splat((r * 0.45).max(2.0)),
                glow(heart, 1.0, dark),
            );
        }
        painter.add(mesh);
    }
}

/// A line through `points`, `half(i)` of its width either side of point
/// `i`, `colour(i, p)` along its middle: fading to nothing at its sides
/// (`soft`), else solid with an edge of `FEATHER` fading out.
fn ribbon(
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
fn ellipse(mesh: &mut Mesh, centre: Pos2, radii: Vec2, colour: Color32) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn strikes(l: &Lightning) -> impl Iterator<Item = &Bolt> {
        l.bolts.iter().filter(|b| b.kind == Kind::Strike)
    }

    #[test]
    fn a_beat_strikes_and_silence_fades_out() {
        let mut l = Lightning::new();
        let size = vec2(600.0, 90.0);
        // Too quiet for arcs or tendrils, and no beat.
        let quiet = vec![0.1; BANDS];
        for _ in 0..20 {
            l.feed(Some(&quiet), 0.02, size);
        }
        assert!(l.bolts.is_empty());
        // The bass jumps: bolts from the middle to an edge.
        let mut beat = vec![0.3; BANDS];
        beat[..BANDS / 6].fill(0.9);
        l.feed(Some(&beat), 0.02, size);
        assert!(strikes(&l).count() > 0);
        for b in strikes(&l) {
            let main = &b.paths[0].points;
            assert_eq!(main.len(), (1 << DEPTH) + 1);
            assert_eq!(main[0], pos2(300.0, 45.0));
            let end = main[main.len() - 1];
            assert!(
                end.x == 0.0 || end.x == 600.0 || end.y == 0.0 || end.y == 90.0,
                "{end:?}"
            );
            assert!(b.paths.len() >= 2);
            assert_eq!(b.hit.map(|h| h.0), Some(end));
            // Each branch starts on the path it grows from.
            for p in &b.paths[1..] {
                assert!(b.paths.iter().any(|q| q.points.contains(&p.points[0])));
            }
        }
        // Loud enough for arcs to both ends, and tendrils for the treble,
        // once they are drawn anew.
        for _ in 0..4 {
            l.feed(Some(&beat), 0.02, size);
        }
        assert_eq!(l.bolts.iter().filter(|b| b.kind == Kind::Arc).count(), 2);
        assert!(l.bolts.iter().any(|b| b.kind == Kind::Tendril));
        // Nothing heard for a second and a half: all gone, sparks too.
        for _ in 0..75 {
            l.feed(None, 0.02, size);
        }
        assert!(!l.active());
    }

    #[test]
    fn the_leader_grows_then_the_stroke_throws_sparks_and_flickers() {
        let mut l = Lightning::new();
        let size = vec2(600.0, 118.0);
        let quiet = vec![0.0; BANDS];
        for _ in 0..20 {
            l.feed(Some(&quiet), 0.02, size);
        }
        let mut beat = vec![0.0; BANDS];
        beat[..BANDS / 6].fill(1.0);
        l.feed(Some(&beat), 0.01, size);
        let b = strikes(&l).next().unwrap();
        // The leader: not yet struck, dimmer than the stroke, no sparks.
        assert!(b.after().is_none() && !b.struck);
        assert!(b.hot() < 0.5);
        assert!(l.sparks.is_empty());
        // The stroke: white-hot, the sparks thrown into the display.
        l.feed(Some(&quiet), LEADER_S, size);
        let b = strikes(&l).next().unwrap();
        assert!(b.struck);
        assert!(b.hot() > 0.7, "{}", b.hot());
        assert!(!l.sparks.is_empty());
        let (at, inward) = b.hit.unwrap();
        for s in &l.sparks {
            assert!((s.at - at).dot(inward) >= -1e-3);
        }
        // A strong beat's channel strokes again, brighter than just before.
        let mut l = Lightning::new();
        let b = Bolt {
            kind: Kind::Strike,
            paths: Vec::new(),
            step: 0.0,
            age: LEADER_S + 0.099,
            life: LEADER_S + 0.6,
            strength: 1.0,
            pulses: vec![(0.0, 1.0), (0.1, 0.8)],
            hit: None,
            struck: true,
        };
        let before = b.hot();
        l.bolts.push(b);
        l.feed(None, 0.002, size);
        assert!(l.bolts[0].hot() > before + 0.4);
        // The sparks fall.
        let mut l = Lightning::new();
        l.throw_sparks(pos2(300.0, 0.0), vec2(0.0, 1.0), 1.0, size, 1.0);
        let start: f32 = l.sparks.iter().map(|s| s.speed.y).sum();
        l.feed(None, 0.05, size);
        let later: f32 = l.sparks.iter().map(|s| s.speed.y).sum();
        assert!(later > start * (1.0 - DRAG * 0.05));
    }
}
