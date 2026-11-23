//! Fire: flames rising from the spectrum along the display's bottom, with
//! fireballs, explosions and sparks. The picture is a fine grid of heat (a
//! pixel `POINTS` points wide); at least `STEP_HZ` times a second each
//! pixel takes a mean of the pixel under it (the most), its two neighbours
//! and the one under that (less: sideways they blurred the flames), less
//! a cooling read from a map of soft blobs that rises with the flames
//! (`cooling_map`; Hugo Elias's fire, with a finer octave for ragged
//! edges and wisps), more the higher: where a blob passes, a tongue thins
//! and breaks off, so the flames lick upwards smoothly rather than as
//! Doom's coarse grains (the user found those low in resolution). Each row
//! is first moved aside by a gust, and each pixel further by a swirl that
//! rises slower than the flames (`shifted`, `swirl_map`), so the tongues
//! sway and each bends and curls on its own. Every destination is written,
//! leaving no old heat
//! floating between the flames. The bottom row is the source: each column
//! the analyzer's level at its place across (the bass on the left), with
//! hot spots and travelling ripples that break it into tongues, going from
//! the last frame's level to this one's over the frame's steps. Beats
//! speed up the rise and reduce cooling, throwing the flames higher.
//!
//! Each beat sends a fireball up out of the flames (`Ball`): a blob that
//! puts its heat into the grid wherever it is, so flames trail from it as
//! they would from something burning, under a glow of its own. A strong
//! beat's ball bursts at the top of its climb (`Blast`): a flash over the
//! display, a shock ring, a burst of heat that billows up as flames,
//! burning fragments thrown out in arcs (small balls, their trails
//! flames too) and sparks. Embers rise from the flames all the while, as
//! many as the sound is loud, swaying, cooling from white to red.
//!
//! Shown as a texture over the display, the heat in the visualization's
//! colours, the hottest drawn towards white; on a dark theme a blurred
//! glow of it added over it (`bloom_image`), the flames lighting the air;
//! the glows, rings and sparks over it in one mesh, added to the ground on
//! a dark theme. Paused, it stands; stopped, it burns out.

use egui::{
    Color32, ColorImage, Mesh, Painter, Pos2, Rect, TextureHandle, TextureOptions, Vec2, pos2,
    vec2,
};
use std::f32::consts::TAU;

use super::beat::{Beat, Rng, ellipse, glow, level_at, ribbon, ring};
use crate::palette;

/// The bands of the levels fed.
pub const BANDS: usize = 48;
/// Points a pixel of the fire, and its pixels at most across.
const POINTS: f32 = 1.0;
const MAX_WIDTH: usize = 1200;
/// The heat rises a pixel at least this many times a second, faster
/// with the sound and especially on beats.
const STEP_HZ: f32 = 210.0;
/// The cooling map's blobs: their radius across and down in pixels (three
/// box blurs of it), and how much more the strongest cool than the mean
/// (the blurred noise raised to `COOL_CONTRAST`).
const COOL_ACROSS: usize = 3;
const COOL_DOWN: usize = 6;
const COOL_CONTRAST: f32 = 2.0;
/// A finer octave of it, `FINE_SHARE` of the cooling: ragged edges and
/// thin wisps breaking off the tongues.
const FINE_ACROSS: usize = 1;
const FINE_DOWN: usize = 2;
const FINE_SHARE: f32 = 0.35;
/// The swirl: a field of sideways pushes (blobs `SWIRL_ACROSS` by
/// `SWIRL_DOWN` pixels), up to `SWIRL` pixels a step at the top and a
/// quarter of it at the base, rising `SWIRL_RISE` of a row a step (slower
/// than the flames, so each tongue bends and curls on its own).
const SWIRL_ACROSS: usize = 6;
const SWIRL_DOWN: usize = 10;
const SWIRL: f32 = 0.7;
const SWIRL_RISE: f32 = 0.55;
/// The glow about the flames on a dark ground: the heat in pixels of
/// `BLOOM` by `BLOOM` blurred, added at `BLOOM_LIGHT`.
const BLOOM: usize = 4;
const BLOOM_LIGHT: f32 = 0.35;
/// The places across the colours are worked out for, once per palette.
const PLACES: usize = 32;
/// On a light ground the colours go no higher than this level.
const LIGHT_TOP: f32 = 0.6;
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

/// A fireball climbs for `BALL_RISE_S` (the least, and the most for the
/// strongest beat) to `BALL_APEX` of the height under the top (from the
/// highest to the lowest), its radius `BALL_RADIUS` of the height plus
/// as much again for the strongest beat, growing by `BALL_GROW` over the
/// climb. A ball not bursting dies down over `FIZZLE_S` after the top.
/// At most `MOST_BALLS` at once.
const BALL_RISE_S: (f32, f32) = (0.42, 0.62);
const BALL_APEX: (f32, f32) = (0.1, 0.42);
const BALL_RADIUS: f32 = 0.055;
const BALL_GROW: f32 = 0.6;
const FIZZLE_S: f32 = 0.3;
const MOST_BALLS: usize = 10;
/// A beat this strong bursts its ball; one over `TWIN` sends two.
const BURST: f32 = 0.6;
const TWIN: f32 = 0.92;
/// A burst: the flash and the shock ring last `BLAST_S`; the ring runs
/// out to `RING_REACH` of the height (times 0.6–1 by the strength); the
/// heat put in reaches `BLAST_HEAT_RADIUS` times the ball's radius.
const BLAST_S: f32 = 0.35;
const RING_REACH: f32 = 0.65;
const BLAST_HEAT_RADIUS: f32 = 2.4;
/// Burning fragments thrown by a burst: up to `FRAGMENTS` more for the
/// strongest, at `FRAGMENT_SPEED` heights a second, falling at
/// `FRAGMENT_FALL` heights a second².
const FRAGMENTS: (f32, f32) = (5.0, 5.0);
const FRAGMENT_SPEED: (f32, f32) = (0.9, 1.8);
const FRAGMENT_FALL: f32 = 2.4;
/// Sparks thrown by a burst, as many again for the strongest.
const BURST_SPARKS: f32 = 16.0;
/// Embers rising from the flames: `EMBERS` a second with the whole source
/// at full heat,
/// `BEAT_EMBERS` more on a beat; at most `MOST_SPARKS` sparks at once.
const EMBERS: f32 = 45.0;
const BEAT_EMBERS: f32 = 10.0;
const MOST_SPARKS: usize = 420;

/// A ball of fire in the grid's pixels, flying up (or out of a burst).
struct Ball {
    at: Pos2,
    /// Where it was the frame before: the steps between put its heat
    /// along the way (stamped only where it is, its trail was beads).
    was: Pos2,
    speed: Vec2,
    /// Down, pixels a second².
    fall: f32,
    radius: f32,
    heat: f32,
    age: f32,
    /// When it reaches the top of its climb: it bursts, or dies down.
    top: f32,
    bursts: bool,
    strength: f32,
    /// A burst's fragment: no glow of its own beyond a small one.
    fragment: bool,
    /// Its own sway.
    seed: f32,
}

impl Ball {
    fn life(&self) -> f32 {
        self.top + if self.bursts { 0.0 } else { FIZZLE_S }
    }

    /// Its heat now, dying down after the top unless it bursts there.
    fn burning(&self) -> f32 {
        let over = (self.age - self.top).max(0.0);
        self.heat * (1.0 - over / FIZZLE_S).clamp(0.0, 1.0)
    }

    fn radius_now(&self) -> f32 {
        let climb = (self.age / self.top).min(1.0);
        self.radius * (1.0 + BALL_GROW * climb)
    }
}

/// A ball bursting: its flash and shock ring, in the grid's pixels.
struct Blast {
    at: Pos2,
    age: f32,
    strength: f32,
    radius: f32,
}

struct Spark {
    at: Pos2,
    speed: Vec2,
    age: f32,
    life: f32,
    /// Down, pixels a second² (negative: it rises).
    fall: f32,
    /// Slowed this much a second.
    drag: f32,
    /// How much it sways, pixels a second; and its own phase.
    sway: f32,
    seed: f32,
    size: f32,
}

pub struct Fire {
    /// The heat, 0 to 1, row by row from the top; the next step's.
    heat: Vec<f32>,
    next: Vec<f32>,
    /// Each row moved aside by its gust, a pixel more either side.
    shifted: Vec<f32>,
    /// The cooling map (`width` × `cool_rows`, a mean of 1), and how far
    /// it has risen.
    cooling: Vec<f32>,
    cool_rows: usize,
    scroll: usize,
    /// The swirl (`width` × `cool_rows`, a mean of 0 and a spread of
    /// 1), and how far it has risen.
    swirl: Vec<f32>,
    swirl_scroll: f32,
    width: usize,
    height: usize,
    /// The colours by place and heat, and the palette and ground they are
    /// for.
    lut: Vec<Color32>,
    /// The bottom row's heat before its flicker, and as it was the frame
    /// before: the steps of a frame go from one to the other (set once a
    /// frame, it rose as a band every frame's steps high).
    source: Vec<f32>,
    was: Vec<f32>,
    /// Steps owed (fractions carried on).
    owed: f32,
    /// The simulation clock, frozen along with the flame when paused.
    time: f32,
    beat: Beat,
    rng: Rng,
    lit: bool,
    balls: Vec<Ball>,
    blasts: Vec<Blast>,
    sparks: Vec<Spark>,
    /// Embers owed (fractions carried on).
    embers_owed: f32,
    texture: Option<TextureHandle>,
    /// The glow's texture (dark grounds only).
    bloom: Option<TextureHandle>,
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
            next: Vec::new(),
            shifted: Vec::new(),
            cooling: Vec::new(),
            cool_rows: 0,
            scroll: 0,
            swirl: Vec::new(),
            swirl_scroll: 0.0,
            width: 0,
            height: 0,
            lut: Vec::new(),
            source: Vec::new(),
            was: Vec::new(),
            owed: 0.0,
            time: 0.0,
            beat: Beat::default(),
            rng: Rng::default(),
            lit: false,
            balls: Vec::new(),
            blasts: Vec::new(),
            sparks: Vec::new(),
            embers_owed: 0.0,
            texture: None,
            bloom: None,
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
            self.next = vec![0.0; w * h];
            self.shifted = vec![0.0; (w + 2) * h];
            self.cool_rows = (2 * h).max(64);
            self.cooling = cooling_map(&mut self.rng, w, self.cool_rows);
            self.scroll = 0;
            self.swirl = swirl_map(&mut self.rng, w, self.cool_rows);
            self.swirl_scroll = 0.0;
            self.source = vec![0.0; w];
            self.was = vec![0.0; w];
            self.lit = false;
            self.changed = true;
            self.balls.clear();
            self.blasts.clear();
            self.sparks.clear();
        }
        if w < 4 || h < 4 || (levels.is_some() && !moved) {
            return;
        }
        let struck = self.beat.feed(levels, dt);
        let kick = self.beat.kick * KICK_HEAT;
        self.was.clone_from(&self.source);
        for (x, s) in self.source.iter_mut().enumerate() {
            let wanted = levels.map_or(0.0, |l| {
                let level = level_at(l, x as f32 / (w - 1) as f32);
                let heat = ((level - FROM) / (1.0 - FROM)).clamp(0.0, 1.0).powf(POWER);
                (heat + kick * heat.sqrt()).min(1.0)
            });
            *s = wanted.max(*s - dt / FALL_S).max(0.0);
        }
        self.fly(dt);
        if let Some(strength) = struck.filter(|_| self.source.iter().any(|&s| s > 0.05)) {
            self.launch(strength);
            if strength > TWIN {
                self.launch(strength * 0.8);
            }
            self.throw_embers(BEAT_EMBERS * strength);
        }
        let burning = self.source.iter().sum::<f32>() / w as f32;
        self.embers_owed += EMBERS * burning * dt;
        let embers = self.embers_owed.floor();
        self.embers_owed -= embers;
        self.throw_embers(embers);
        if !self.lit && self.balls.is_empty() && self.source.iter().all(|&s| s <= 0.0) {
            return;
        }
        let hz = STEP_HZ * (1.0 + KICK_SPEED * self.beat.kick + LEVEL_SPEED * self.beat.level);
        self.owed += dt * hz;
        // The UI caps dt at 0.1 s: enough steps for a beat on most
        // a slow frame, instead of slowing the flame down to four rows.
        let steps = (self.owed as usize).min(40);
        self.owed -= self.owed.floor();
        for i in 0..steps {
            self.step(1.0 / hz, (i + 1) as f32 / steps as f32);
        }
        self.changed |= steps > 0;
    }

    /// The balls, the blasts and the sparks moved on by `dt`; the balls at
    /// the top of their climb burst.
    fn fly(&mut self, dt: f32) {
        let h = self.height as f32;
        let mut bursting = Vec::new();
        for b in &mut self.balls {
            b.age += dt;
            b.was = b.at;
            b.speed.y += b.fall * dt;
            let sway = if b.fragment {
                0.0
            } else {
                (b.age * 7.0 + b.seed).sin() * 0.25 * h
            };
            b.at += (b.speed + vec2(sway, 0.0)) * dt;
            if b.bursts && b.age >= b.top {
                bursting.push((b.at, b.radius_now(), b.strength));
            }
        }
        self.balls.retain(|b| b.age < b.life());
        for (at, radius, strength) in bursting {
            self.burst(at, radius, strength);
        }
        for b in &mut self.blasts {
            b.age += dt;
        }
        self.blasts.retain(|b| b.age < BLAST_S);
        for s in &mut self.sparks {
            s.age += dt;
            s.speed.y += s.fall * dt;
            s.speed *= (1.0 - s.drag * dt).max(0.0);
            let sway = (s.age * 5.0 + s.seed).sin() * s.sway;
            s.at += (s.speed + vec2(sway, 0.0)) * dt;
        }
        let (w, h) = (self.width as f32, self.height as f32);
        self.sparks.retain(|s| {
            s.age < s.life && s.at.y > -0.2 * h && s.at.x > -0.1 * w && s.at.x < 1.1 * w
        });
    }

    /// A random column, the hotter ones of the source more often.
    fn hot_column(&mut self) -> f32 {
        let w = self.width;
        let mut best = (0.0, -1.0);
        for _ in 0..4 {
            let x = self.rng.unit() * w as f32;
            let s = self.source[(x as usize).min(w - 1)] + 0.15 * self.rng.unit();
            if s > best.1 {
                best = (x, s);
            }
        }
        best.0
    }

    /// A fireball sent up from a hot column.
    fn launch(&mut self, strength: f32) {
        if self.balls.iter().filter(|b| !b.fragment).count() >= MOST_BALLS {
            return;
        }
        let h = self.height as f32;
        let x = self.hot_column();
        let from = pos2(x, h - 1.0);
        // Higher for the stronger beats.
        let apex = h * (BALL_APEX.1 - (BALL_APEX.1 - BALL_APEX.0) * strength)
            + self.rng.between(-0.05, 0.05) * h;
        let rise = self.rng.between(BALL_RISE_S.0, BALL_RISE_S.1);
        // Slowing to a stop at the top: up at 2d/T, falling at 2d/T².
        let climb = (from.y - apex).max(1.0);
        self.balls.push(Ball {
            at: from,
            was: from,
            speed: vec2(self.rng.between(-0.3, 0.3) * h, -2.0 * climb / rise),
            fall: 2.0 * climb / (rise * rise),
            radius: h * BALL_RADIUS * (1.0 + strength),
            heat: 1.0,
            age: 0.0,
            top: rise,
            bursts: strength >= BURST,
            strength,
            fragment: false,
            seed: self.rng.unit() * TAU,
        });
    }

    /// A ball bursting at `at`: the flash and the ring, a burst of heat,
    /// burning fragments and sparks.
    fn burst(&mut self, at: Pos2, radius: f32, strength: f32) {
        let h = self.height as f32;
        self.blasts.push(Blast {
            at,
            age: 0.0,
            strength,
            radius,
        });
        stamp(
            &mut self.heat,
            self.width,
            self.height,
            at,
            radius * BLAST_HEAT_RADIUS,
            1.0,
        );
        self.changed = true;
        let n = (FRAGMENTS.0 + FRAGMENTS.1 * strength) as usize;
        for i in 0..n {
            let a = TAU * (i as f32 + self.rng.unit() * 0.8) / n as f32;
            let speed = self.rng.between(FRAGMENT_SPEED.0, FRAGMENT_SPEED.1) * h;
            self.balls.push(Ball {
                at,
                was: at,
                speed: vec2(a.cos() * 1.6, a.sin()) * speed - vec2(0.0, 0.4 * h),
                fall: FRAGMENT_FALL * h,
                radius: h * self.rng.between(0.025, 0.045),
                heat: self.rng.between(0.75, 1.0),
                age: 0.0,
                top: self.rng.between(0.3, 0.6),
                bursts: false,
                strength,
                fragment: true,
                seed: 0.0,
            });
        }
        for _ in 0..(BURST_SPARKS * (1.0 + strength)) as usize {
            let a = self.rng.unit() * TAU;
            let speed = self.rng.between(1.2, 3.6) * h;
            self.sparks.push(Spark {
                at,
                speed: vec2(a.cos() * 1.5, a.sin()) * speed,
                age: 0.0,
                life: self.rng.between(0.35, 0.8),
                fall: 1.4 * h,
                drag: 2.5,
                sway: 0.0,
                seed: 0.0,
                size: self.rng.between(0.8, 1.3),
            });
        }
        self.cap_sparks();
    }

    /// `n` embers rising from hot columns.
    fn throw_embers(&mut self, n: f32) {
        let h = self.height as f32;
        for _ in 0..n as usize {
            let x = self.hot_column();
            let heat = self.source[(x as usize).min(self.width - 1)];
            self.sparks.push(Spark {
                at: pos2(x, h * (1.0 - self.rng.between(0.05, 0.35) * (0.5 + heat))),
                speed: vec2(
                    self.rng.between(-0.3, 0.3) * h,
                    -self.rng.between(0.6, 1.5) * h,
                ),
                age: 0.0,
                life: self.rng.between(0.7, 1.6),
                fall: -0.3 * h,
                drag: 0.8,
                sway: self.rng.between(0.2, 0.6) * h,
                seed: self.rng.unit() * TAU,
                size: self.rng.between(0.5, 1.0),
            });
        }
        self.cap_sparks();
    }

    fn cap_sparks(&mut self) {
        if self.sparks.len() > MOST_SPARKS {
            let over = self.sparks.len() - MOST_SPARKS;
            self.sparks.drain(..over);
        }
    }

    /// The heat risen a pixel: each pixel the mean of the one under it, its
    /// neighbours and the one under that, less the cooling map's there
    /// (which rises with it). Every destination is written, from the rows
    /// under it only, so no old heat is left floating. Then the source and
    /// the balls put theirs in.
    fn step(&mut self, dt: f32, along: f32) {
        self.time += dt;
        let (w, h) = (self.width, self.height);
        let pw = w + 2;
        // A mean of 1 a pixel lost on each row: full heat reaches `REACH`
        // of the height.
        let cooling = 1.0 / (REACH * h as f32) * (1.0 - KICK_REACH * self.beat.kick);
        // Each row moved aside by a gust shared along it, so whole tongues
        // sway, more the higher they reach, and each pixel further by the
        // swirl there, so each tongue bends on its own; between pixels,
        // smoothly.
        let rows = self.cool_rows;
        self.scroll = (self.scroll + 1) % rows;
        self.swirl_scroll = (self.swirl_scroll + SWIRL_RISE) % rows as f32;
        let swirl_at = self.swirl_scroll as usize;
        let last = w as isize - 1;
        for y in 0..h {
            let rise = 1.0 - y as f32 / h as f32;
            let wind = (self.time * 1.3).sin() * 0.22
                + (self.time * 3.1 + rise * 4.0).sin() * 0.3 * rise
                + (self.time * 0.7).sin() * 0.12;
            let push = SWIRL * (0.25 + 0.75 * rise);
            let swirl = &self.swirl[((y + swirl_at) % rows) * w..][..w];
            let row = &self.heat[y * w..(y + 1) * w];
            let out = &mut self.shifted[y * pw..(y + 1) * pw];
            // Kept positive by `PAD`, so a cast floors it (the hot loop:
            // `floor` and two clamps doubled the step's cost).
            const PAD: f32 = 64.0;
            let start = wind - 1.0 + PAD;
            for (i, o) in out.iter_mut().enumerate() {
                let along = swirl[(i.saturating_sub(1)).min(w - 1)];
                let from = (i as f32 + start + along * push).max(0.0);
                let whole = from as usize;
                let blend = from - whole as f32;
                let x = (whole as isize - PAD as isize).clamp(0, last) as usize;
                let next = (x + 1).min(w - 1);
                *o = row[x] + (row[next] - row[x]) * blend;
            }
        }
        for y in 0..h - 1 {
            let here = &self.shifted[(y + 1) * pw..(y + 2) * pw];
            let under = (y + 2).min(h - 1);
            let below = &self.shifted[under * pw..(under + 1) * pw];
            let cool = &self.cooling[((y + 1 + self.scroll) % rows) * w..][..w];
            let out = &mut self.next[y * w..(y + 1) * w];
            // Cooling more the higher: a hot base, the tongues thinning to
            // their tips.
            let rise = 1.0 - (y + 1) as f32 / h as f32;
            let cooling = cooling * (0.4 + 1.2 * rise);
            for x in 0..w {
                let v = 0.5 * here[x + 1] + 0.15 * (here[x] + here[x + 2]) + 0.2 * below[x + 1];
                out[x] = (v - cool[x] * cooling).max(0.0);
            }
        }
        std::mem::swap(&mut self.heat, &mut self.next);
        let bottom = (h - 1) * w;
        for x in 0..w {
            // Coherent patches of hot and cool fuel travel across the
            // base. Even sustained notes produce growing and shrinking
            // tongues, with fine flicker only over that larger motion.
            // In units of 3 points, the size the ripples were made for.
            let place = x as f32 * POINTS / 3.0;
            // Two ripples of lengths that do not divide, the second the
            // other way: one alone set the tongues out evenly, a fence.
            let broad = 0.6 * (place * 0.28 - self.time * 6.0).sin()
                + 0.4 * (place * 0.113 + self.time * 2.3).sin();
            let fine = (place * 0.72 + self.time * 13.0 + broad * 1.4).sin();
            // Hot spots in the fuel, pixel by pixel: the base is not one
            // even band, nor is a beat's heat.
            let flicker = (0.8 + 0.3 * broad + 0.18 * fine) * self.rng.between(0.6, 1.2);
            let fuel = self.was[x] + (self.source[x] - self.was[x]) * along;
            self.heat[bottom + x] = (fuel * flicker).min(1.0);
        }
        for b in &self.balls {
            let at = b.was + (b.at - b.was) * along;
            stamp(&mut self.heat, w, h, at, b.radius_now(), b.burning());
        }
        self.lit = self.heat.iter().any(|&v| v > 0.003);
    }

    /// Something still burns or flies: keep repainting.
    pub fn active(&self) -> bool {
        self.lit
            || self.source.iter().any(|&s| s > 0.0)
            || !self.balls.is_empty()
            || !self.blasts.is_empty()
            || !self.sparks.is_empty()
    }

    pub fn paint(&mut self, painter: &Painter, rect: Rect) {
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            return;
        }
        let colours = palette::visual_for(crate::settings::Visual::Fire);
        let ground = palette::list_bg();
        let dark = colours.is_dark();
        let bright = if dark {
            Color32::from_rgb(255, 246, 220)
        } else {
            colours.peak(0.5)
        };
        if self.drawn != Some((colours, ground)) {
            self.lut = colour_table(colours, ground, bright);
        }
        if self.changed || self.texture.is_none() || self.drawn != Some((colours, ground)) {
            let mut image = ColorImage::filled([w, h], ground);
            let across = (PLACES - 1) as f32 / (w - 1).max(1) as f32;
            for (i, out) in image.pixels.iter_mut().enumerate() {
                let v = self.heat[i];
                if v <= 0.01 {
                    continue;
                }
                let place = ((i % w) as f32 * across).round() as usize;
                *out = self.lut[place * 256 + (v * 255.0).min(255.0) as usize];
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
            if dark {
                let image = bloom_image(&self.heat, w, h, colours);
                match &mut self.bloom {
                    Some(t) => t.set(image, TextureOptions::LINEAR),
                    None => {
                        self.bloom = Some(painter.ctx().load_texture(
                            "fire glow",
                            image,
                            TextureOptions::LINEAR,
                        ));
                    }
                }
            } else {
                self.bloom = None;
            }
            self.changed = false;
            self.drawn = Some((colours, ground));
        }
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let mut mesh = Mesh::default();
        // Glows lie over a light ground rather than adding to it: softer.
        let haze = if dark { 1.0 } else { 0.5 };
        let hottest = if dark { 1.0 } else { LIGHT_TOP };
        let (sx, sy) = (rect.width() / w as f32, rect.height() / h as f32);
        let to_screen = |p: Pos2| rect.min + vec2((p.x + 0.5) * sx, (p.y + 0.5) * sy);
        let place = |p: Pos2| (p.x / w as f32).clamp(0.0, 1.0);
        // Towards white on a dark ground; the flames' yellow on a light one
        // (its top and peak are greys that looked dirty on the balls).
        let hot = |p: f32, heat: f32| {
            if dark {
                colours.at(1.0, p).lerp_to_gamma(bright, heat)
            } else {
                colours.at(LIGHT_TOP, p)
            }
        };

        // The flames' light on the air above them, and a burst's over the
        // whole display.
        let burning = self.source.iter().sum::<f32>() / w as f32;
        let flash = self
            .blasts
            .iter()
            .map(|b| b.strength * (1.0 - b.age / (0.5 * BLAST_S)).max(0.0))
            .fold(0.0, f32::max);
        let warm = glow(colours.at(0.8, 0.5), (0.12 * burning + 0.1 * flash) * haze, dark);
        let i = mesh.vertices.len() as u32;
        let top = rect.top() + rect.height() * 0.3 * (1.0 - flash);
        mesh.colored_vertex(pos2(rect.left(), top), Color32::TRANSPARENT);
        mesh.colored_vertex(pos2(rect.right(), top), Color32::TRANSPARENT);
        mesh.colored_vertex(rect.right_bottom(), warm);
        mesh.colored_vertex(rect.left_bottom(), warm);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);

        // The bursts: a flash about the place, the shock ring running out.
        for b in &self.blasts {
            let t = b.age / BLAST_S;
            let at = to_screen(b.at);
            let fade = 1.0 - t;
            let r = b.radius * sy;
            ellipse(
                &mut mesh,
                at,
                Vec2::splat(r * (2.0 + 4.0 * t)),
                glow(hot(place(b.at), 0.6), 0.55 * fade * fade * haze, dark),
            );
            let reach = RING_REACH * rect.height() * (0.6 + 0.4 * b.strength);
            let out = 1.0 - (1.0 - t) * (1.0 - t);
            // A heat wave: hot at first, then in the flames' colours,
            // thinning and gone well before it would hang about.
            let wave = colours
                .at(1.0 - 0.4 * t, place(b.at))
                .lerp_to_gamma(bright, 0.5 * fade);
            ring(
                &mut mesh,
                at,
                r + reach * out,
                (1.5 + 3.0 * t) * sy / POINTS,
                glow(wave, 0.7 * fade * fade * fade * b.strength, dark),
            );
        }

        // The fireballs: a glow about each, a hot heart.
        for b in &self.balls {
            let heat = b.burning();
            if heat <= 0.01 {
                continue;
            }
            let at = to_screen(b.at);
            let r = b.radius_now() * sy;
            let p = place(b.at);
            let (outer, inner) = if b.fragment { (3.0, 0.8) } else { (3.2, 1.0) };
            ellipse(
                &mut mesh,
                at,
                Vec2::splat(r * outer),
                glow(colours.at(0.85f32.min(hottest), p), 0.28 * heat * haze, dark),
            );
            ellipse(
                &mut mesh,
                at,
                Vec2::splat(r * inner),
                glow(hot(p, 0.7), 0.9 * heat, dark),
            );
        }

        // The sparks: short streaks along their flight, cooling from white
        // through the colours to their floor, a glow about the hot ones.
        for s in &self.sparks {
            let t = s.age / s.life;
            let light = (1.0 - t * t) * (1.0 - t).sqrt();
            if light <= 0.01 {
                continue;
            }
            let p = place(s.at);
            let colour = if t < 0.25 {
                hot(p, 1.0 - t / 0.25 * 0.6)
            } else {
                colours.at((1.0 - (t - 0.25) * 0.8).min(hottest), p)
            };
            let head = to_screen(s.at);
            let tail = head - vec2(s.speed.x * sx, s.speed.y * sy) * 0.035;
            let width = s.size * sy / POINTS;
            ribbon(
                &mut mesh,
                &[head, tail],
                |i| if i == 0 { 0.6 * width } else { 0.15 },
                |i, _| glow(colour, if i == 0 { light } else { 0.0 }, dark),
                false,
            );
            if t < 0.5 {
                ellipse(
                    &mut mesh,
                    head,
                    Vec2::splat(3.5 * width),
                    glow(colour, 0.25 * light * haze, dark),
                );
            }
        }
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        if let Some(t) = &self.texture {
            painter.image(t.id(), rect, uv, Color32::WHITE);
        }
        // The flames' light on the air about them, added over them.
        if let Some(t) = &self.bloom {
            painter.image(t.id(), rect, uv, Color32::WHITE);
        }
        painter.add(mesh);
    }
}

/// The flames' colour for each of `PLACES` places across and 256 heats:
/// the colours' floor for the cooler half, their top for the hot core,
/// then towards `bright` on a dark ground (on a light one the scheme's
/// peak is a grey that looked dirty in the flames), fading into the
/// ground where it is cool.
fn colour_table(colours: palette::VisualPalette, ground: Color32, bright: Color32) -> Vec<Color32> {
    let dark = colours.is_dark();
    let mut lut = Vec::with_capacity(PLACES * 256);
    for p in 0..PLACES {
        let place = p as f32 / (PLACES - 1) as f32;
        for i in 0..256 {
            let v = i as f32 / 255.0;
            // On a light ground no higher than the yellow: the white-hot top,
            // made readable there, is a grey.
            let level = if dark { v.powf(0.65) } else { v.powf(0.65).min(LIGHT_TOP) };
            let mut c = colours.at(level, place);
            if dark && v > 0.7 {
                c = c.lerp_to_gamma(bright, (v - 0.7) / 0.3 * 0.7);
            }
            // Fading into the ground over a narrow band of heat: crisp
            // edges to the tongues.
            let a = ((v - 0.03) / 0.22).clamp(0.0, 1.0);
            lut.push(ground.lerp_to_gamma(c, a * a * (3.0 - 2.0 * a)));
        }
    }
    lut
}

/// The cooling map: noise gathered into soft blobs taller than wide, the
/// contrast raised, with a finer octave of it over them; a mean of 1.
fn cooling_map(rng: &mut Rng, w: usize, rows: usize) -> Vec<f32> {
    let coarse = cooling_blobs(rng, w, rows, COOL_ACROSS, COOL_DOWN);
    let fine = cooling_blobs(rng, w, rows, FINE_ACROSS, FINE_DOWN);
    coarse
        .iter()
        .zip(&fine)
        .map(|(c, f)| c * (1.0 - FINE_SHARE) + f * FINE_SHARE)
        .collect()
}

/// Noise gathered into blobs `across` by `down` pixels (three box blurs,
/// wrapping both ways, so it tiles as it rises), spread to 0–1 and raised
/// to `COOL_CONTRAST`: a few strong blobs breaking the tongues, gentle
/// cooling between them; a mean of 1.
fn cooling_blobs(rng: &mut Rng, w: usize, rows: usize, across: usize, down: usize) -> Vec<f32> {
    let mut map = blobs(rng, w, rows, across, down);
    let (lo, hi) = map
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    for v in &mut map {
        let t = if hi > lo { (*v - lo) / (hi - lo) } else { 0.5 };
        *v = t.powf(COOL_CONTRAST);
    }
    let mean = map.iter().sum::<f32>() / map.len() as f32;
    if mean > 0.0 {
        for v in &mut map {
            *v /= mean;
        }
    }
    map
}

/// The swirl: noise gathered into wide soft blobs, a mean of 0 and a
/// spread (root mean square) of 1.
fn swirl_map(rng: &mut Rng, w: usize, rows: usize) -> Vec<f32> {
    let mut map = blobs(rng, w, rows, SWIRL_ACROSS, SWIRL_DOWN);
    let mean = map.iter().sum::<f32>() / map.len() as f32;
    let spread = (map.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / map.len() as f32)
        .sqrt();
    for v in &mut map {
        *v = if spread > 0.0 { (*v - mean) / spread } else { 0.0 };
    }
    map
}

/// White noise blurred three times by boxes `across` by `down` pixels,
/// wrapping both ways.
fn blobs(rng: &mut Rng, w: usize, rows: usize, across: usize, down: usize) -> Vec<f32> {
    let mut map: Vec<f32> = (0..w * rows).map(|_| rng.unit()).collect();
    let mut tmp = vec![0.0; w * rows];
    for _ in 0..3 {
        blur(&map, &mut tmp, w, rows, across, true);
        blur(&tmp, &mut map, w, rows, down, false);
    }
    map
}

/// The glow about the flames: the heat in blocks of `BLOOM` pixels,
/// spread three times to their neighbours (not wrapping: the hot base would
/// light the top), in the colours' upper part, premultiplied with no
/// coverage so it is added to what is under it.
fn bloom_image(heat: &[f32], w: usize, h: usize, colours: palette::VisualPalette) -> ColorImage {
    let (bw, bh) = (w.div_ceil(BLOOM), h.div_ceil(BLOOM));
    let mut blocks = vec![0.0f32; bw * bh];
    for y in 0..h {
        for x in 0..w {
            blocks[(y / BLOOM) * bw + x / BLOOM] += heat[y * w + x];
        }
    }
    let per = (BLOOM * BLOOM) as f32;
    for v in &mut blocks {
        *v /= per;
    }
    let mut tmp = vec![0.0; bw * bh];
    for _ in 0..3 {
        for y in 0..bh {
            for x in 0..bw {
                let at = |x: usize| blocks[y * bw + x];
                let (l, r) = (at(x.saturating_sub(1)), at((x + 1).min(bw - 1)));
                tmp[y * bw + x] = 0.25 * l + 0.5 * at(x) + 0.25 * r;
            }
        }
        for y in 0..bh {
            for x in 0..bw {
                let at = |y: usize| tmp[y * bw + x];
                let (u, d) = (at(y.saturating_sub(1)), at((y + 1).min(bh - 1)));
                blocks[y * bw + x] = 0.25 * u + 0.5 * at(y) + 0.25 * d;
            }
        }
    }
    let mut image = ColorImage::filled([bw, bh], Color32::TRANSPARENT);
    let across = 1.0 / (bw - 1).max(1) as f32;
    for (i, out) in image.pixels.iter_mut().enumerate() {
        let v = blocks[i];
        if v > 0.005 {
            let c = colours.at(0.75, (i % bw) as f32 * across);
            *out = glow(c, BLOOM_LIGHT * v.sqrt(), true);
        }
    }
    image
}

/// A box blur of `from` into `to`, `radius` pixels either way across or
/// down, wrapping.
fn blur(from: &[f32], to: &mut [f32], w: usize, rows: usize, radius: usize, across: bool) {
    let count = (2 * radius + 1) as f32;
    for y in 0..rows {
        for x in 0..w {
            let mut sum = 0.0;
            for k in 0..=2 * radius {
                let (xx, yy) = if across {
                    ((x + w * radius + k - radius) % w, y)
                } else {
                    (x, (y + rows * radius + k - radius) % rows)
                };
                sum += from[yy * w + xx];
            }
            to[y * w + x] = sum / count;
        }
    }
}

/// Heat put into the grid about `at`, `heat` in the middle falling to
/// nothing at `radius`, where there is less.
fn stamp(grid: &mut [f32], w: usize, h: usize, at: Pos2, radius: f32, heat: f32) {
    if heat <= 0.0 || radius <= 0.0 {
        return;
    }
    let (x0, x1) = ((at.x - radius).floor().max(0.0), (at.x + radius).ceil());
    let (y0, y1) = ((at.y - radius).floor().max(0.0), (at.y + radius).ceil());
    for y in y0 as usize..(y1 as usize + 1).min(h) {
        for x in x0 as usize..(x1 as usize + 1).min(w) {
            let d = (pos2(x as f32, y as f32) - at).length() / radius;
            if d < 1.0 {
                let v = heat * (1.0 - d * d);
                let cell = &mut grid[y * w + x];
                *cell = cell.max(v);
            }
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
        // Paused, it stands, the sparks too.
        let before = f.heat.clone();
        let time = f.time;
        let sparks: Vec<Pos2> = f.sparks.iter().map(|s| s.at).collect();
        assert!(!sparks.is_empty());
        f.feed(Some(&loud), false, 0.02, size);
        assert_eq!(f.heat, before);
        assert_eq!(f.time, time);
        assert_eq!(f.sparks.iter().map(|s| s.at).collect::<Vec<_>>(), sparks);
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
        // The bass rising from nothing is a beat: its ball's fragments,
        // thrown either way, burn out first.
        for _ in 0..120 {
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
        assert_eq!(pixels_for(vec2(300.0, 90.0)), (300, 90));
        assert_eq!(pixels_for(vec2(4000.0, 90.0)).0, MAX_WIDTH);
        assert_eq!(pixels_for(vec2(0.0, 90.0)), (0, 0));
    }

    #[test]
    fn no_old_heat_is_left_between_the_flames() {
        let mut f = Fire::new();
        f.feed(None, true, 0.0, vec2(300.0, 90.0));
        f.heat.fill(1.0);
        for _ in 0..=f.height {
            f.step(1.0 / STEP_HZ, 1.0);
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

    #[test]
    fn a_strong_beat_sends_a_fireball_up_that_bursts() {
        let size = vec2(300.0, 90.0);
        let mut f = Fire::new();
        let steady = [0.5; BANDS];
        // The sound starting is a beat: its ball goes first.
        for _ in 0..120 {
            f.feed(Some(&steady), true, 0.02, size);
        }
        assert!(f.balls.is_empty());
        let mut pulse = steady;
        pulse[..BANDS / 6].fill(1.0);
        f.feed(Some(&pulse), true, 0.02, size);
        let ball = f.balls.iter().find(|b| !b.fragment).expect("a fireball");
        assert!(ball.bursts);
        let (start, top) = (ball.at.y, ball.top);
        // It climbs, putting its heat in the grid where it is.
        for _ in 0..10 {
            f.feed(Some(&steady), true, 0.02, size);
        }
        let ball = f.balls.iter().find(|b| !b.fragment).unwrap();
        assert!(ball.at.y < start - 3.0, "{} {start}", ball.at.y);
        let (x, y) = (ball.at.x as usize, ball.at.y as usize);
        assert!(f.heat[y * f.width + x.min(f.width - 1)] > 0.5);
        // At the top it bursts: a blast, fragments, sparks.
        let mut burst = false;
        for _ in 0..((top / 0.02) as usize + 2) {
            f.feed(Some(&steady), true, 0.02, size);
            burst |= !f.blasts.is_empty();
        }
        assert!(burst);
        assert!(f.balls.iter().any(|b| b.fragment));
        assert!(f.sparks.len() > 20);
        // Stopped, all goes.
        for _ in 0..150 {
            f.feed(None, true, 0.02, size);
        }
        assert!(!f.active());
    }

    #[test]
    fn heat_is_stamped_round_and_within_the_grid() {
        let (w, h) = (20, 10);
        let mut grid = vec![0.0; w * h];
        stamp(&mut grid, w, h, pos2(0.0, 9.0), 4.0, 1.0);
        assert_eq!(grid[9 * w], 1.0);
        assert!(grid[9 * w + 3] > 0.0 && grid[9 * w + 3] < 1.0);
        assert_eq!(grid[9 * w + 4], 0.0);
        assert!(grid.iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
