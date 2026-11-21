//! Fireworks (in the sparks' place: a fountain of streaks the user found
//! dull). Rockets go up as notes start in the bands (`beat::Onsets`, the
//! bass on the left, the treble on the right) and burst at the top of
//! their climb as small shells; a beat bursts the rocket highest up at
//! once as a large shell, or one out of the dark with its trail from the
//! ground when none is far enough up, and lights the sky. The shells are
//! of six kinds (`Shell`): a peony's sphere of stars, a chrysanthemum's
//! trailing ones turning to white glitter, a willow's long drooping gold,
//! a tilted ring, a crackle's stars splitting into flickers, a palm's few
//! heavy arms; a peony or a chrysanthemum may have a core of a second
//! colour. The stars are slowed by the air (`drag`), so a shell opens fast
//! and hangs, and fall; a burst seen from the ground is a sphere's, so
//! they crowd at its rim. Drawn as tapering trails with a glow and a head,
//! white-hot at first on a dark theme, the light added to the ground; a
//! burst leaves a flash and a faint glow of smoke. Paused, they stand;
//! stopped, the rockets up burst and the last stars go out.

use std::f32::consts::TAU;

use egui::{Color32, Mesh, Painter, Pos2, Rect, Vec2, pos2, vec2};

use super::beat::{Beat, Onsets, Rng, glow};
use crate::palette;
use crate::settings::Visual;

/// The bands of the levels fed.
pub const BANDS: usize = 24;
/// The bands gathered into this many for the notes that send rockets up,
/// each from its own stretch of the ground.
const GROUPS: usize = 8;
/// A group sends a rocket up at most this often; at most `MOST_ROCKETS`
/// climb at once.
const LAUNCH_GAP_S: f32 = 0.35;
const MOST_ROCKETS: usize = 6;
/// With nothing sent up for `IDLE_S` while the level is over `IDLE_FROM`,
/// a rocket goes up anyway (a passage without notes starting).
const IDLE_S: f32 = 1.4;
const IDLE_FROM: f32 = 0.3;
/// A rocket climbs this long to the top of its climb, which is this far
/// under the display's top (shares of its height; the stronger the note,
/// the higher), throwing `EMBERS` embers a second.
const RISE_S: [f32; 2] = [0.75, 1.05];
const APEX: [f32; 2] = [0.28, 0.45];
const EMBERS: f32 = 60.0;
/// A beat bursts a rocket that has climbed this share of its way.
const RISEN: f32 = 0.55;
/// The stars fall at this many heights a second squared, times their
/// weight, slowed by their drag.
const GRAVITY: f32 = 0.6;
/// A shell's radius, of the height: the smallest and the largest; at
/// most `SHELL_WIDE` of the width.
const SHELL: [f32; 2] = [0.12, 0.33];
const SHELL_WIDE: f32 = 0.2;
/// The stars kept at most (the oldest go).
const MOST: usize = 1500;
/// The time step of the flight at most (a slow frame takes several), and
/// the most of a frame taken (after a stall the scene does not jump).
const STEP_S: f32 = 1.0 / 60.0;
const MOST_DT: f32 = 0.25;
/// A star is white-hot for this long on a dark theme.
const HOT_S: f32 = 0.12;
/// A glittering star is lit or not this many times a second.
const TWINKLE_HZ: f32 = 24.0;
/// A chrysanthemum's stars turn to glitter at this share of their life.
const TURN: f32 = 0.6;
/// A burst's flash fades in `FLASH_S`, its smoke in `SMOKE_S`, the sky
/// lit by a beat's in `SKY_S`.
const FLASH_S: f32 = 0.3;
const SMOKE_S: f32 = 1.6;
const SKY_S: f32 = 0.25;
/// A trailing star keeps this many places behind it.
const TRAIL: usize = 10;
/// A star without a trail is a streak as long as its flight in
/// `STREAK_S`, at most `STREAK_MOST` points.
const STREAK_S: f32 = 0.045;
const STREAK_MOST: f32 = 20.0;
/// A willow's and a rocket's embers on a dark theme, whatever the colours.
const GOLD: Color32 = Color32::from_rgb(255, 190, 100);
/// The widths are for a display this high (the full bar's), thinner in a
/// lower one.
const REFERENCE_HEIGHT: f32 = 118.0;

/// How a shell bursts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shell {
    Peony,
    Chrysanthemum,
    Willow,
    Ring,
    Crackle,
    Palm,
}

/// How a star is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// A streak along its flight.
    Streak,
    /// A trail through the places it has been.
    Trail,
    /// A rocket's ember, dim.
    Ember,
    /// A crackle's flicker, glittering.
    Flicker,
}

#[derive(Clone)]
struct Spark {
    /// From the display's top left, in points; points a second.
    at: Pos2,
    speed: Vec2,
    /// The places behind it (`Look::Trail`), the newest first, `kept` of
    /// them, one every `every` seconds.
    trail: [Pos2; TRAIL],
    kept: u8,
    every: f32,
    since: f32,
    age: f32,
    life: f32,
    /// Its speed falls by this share a second (exponentially); its fall
    /// is `weight` times gravity's.
    drag: f32,
    weight: f32,
    /// Its place in the colours across, its light (1: full), its width in
    /// points at the reference height.
    place: f32,
    light: f32,
    width: f32,
    look: Look,
    /// A willow's gold, between the colour and the peaks'.
    gold: bool,
    /// Turns to glitter at `TURN` of its life (a chrysanthemum's).
    turns: bool,
    /// Splits into flickers at its end (a crackle's).
    splits: bool,
    seed: u32,
}

struct Rocket {
    at: Pos2,
    speed: Vec2,
    /// Its own: it reaches the top of its climb in its rise time.
    gravity: f32,
    from_y: f32,
    apex_y: f32,
    place: f32,
    strength: f32,
    owed: f32,
}

impl Rocket {
    /// The share of its climb done.
    fn risen(&self) -> f32 {
        (self.from_y - self.at.y) / (self.from_y - self.apex_y).max(1.0)
    }
}

/// A burst's flash and smoke.
struct Flash {
    at: Pos2,
    place: f32,
    radius: f32,
    strength: f32,
    age: f32,
    /// Out of the dark: its rocket's trail from the ground is drawn.
    from_ground: bool,
}

pub struct Fireworks {
    sparks: Vec<Spark>,
    rockets: Vec<Rocket>,
    flashes: Vec<Flash>,
    size: Vec2,
    beat: Beat,
    onsets: Onsets,
    /// Since each group last sent a rocket up, and since any did.
    since: [f32; GROUPS],
    idle: f32,
    /// The last bursts' places across (shares of the width), so the next
    /// out of the dark goes elsewhere.
    recent: [f32; 3],
    /// The sky lit by a beat, and its colour's place.
    sky: f32,
    sky_place: f32,
    rng: Rng,
}

impl Default for Fireworks {
    fn default() -> Self {
        Self::new()
    }
}

impl Fireworks {
    pub fn new() -> Fireworks {
        Fireworks {
            sparks: Vec::new(),
            rockets: Vec::new(),
            flashes: Vec::new(),
            size: Vec2::ZERO,
            beat: Beat::default(),
            onsets: Onsets::default(),
            since: [LAUNCH_GAP_S; GROUPS],
            idle: 0.0,
            recent: [0.5; 3],
            sky: 0.0,
            sky_place: 0.5,
            rng: Rng::default(),
        }
    }

    /// Moves on by `dt` seconds with the levels heard now (`None`:
    /// nothing; `moved` false: paused, they stand) in a display of `size`.
    pub fn feed(&mut self, levels: Option<&[f32]>, moved: bool, dt: f32, size: Vec2) {
        if size.x < 8.0 || size.y < 8.0 {
            self.sparks.clear();
            self.rockets.clear();
            self.flashes.clear();
            self.size = Vec2::ZERO;
            return;
        }
        if self.size != size && self.size.x > 0.0 && self.size.y > 0.0 {
            self.rescale(size / self.size);
        }
        self.size = size;
        if levels.is_some() && !moved {
            return;
        }
        let dt = dt.min(MOST_DT);
        let struck = self.beat.feed(levels, dt);
        let groups = levels.map(grouped);
        let starts = self.onsets.feed(groups.as_deref(), GROUPS, dt);
        for s in &mut self.since {
            *s += dt;
        }
        self.idle += dt;
        if levels.is_some() {
            for (g, &strength) in starts.iter().enumerate() {
                if strength > 0.0
                    && self.since[g] > LAUNCH_GAP_S
                    && self.rockets.len() < MOST_ROCKETS
                {
                    self.since[g] = 0.0;
                    let place = (g as f32 + self.rng.between(0.15, 0.85)) / GROUPS as f32;
                    self.launch(place, strength);
                }
            }
            if self.idle > IDLE_S && self.beat.level > IDLE_FROM && self.rockets.is_empty() {
                let place = self.rng.unit();
                self.launch(place, 0.5);
            }
        }
        if let Some(strength) = struck {
            let treble = levels.map_or(0.0, treble_share);
            self.strike(strength, treble);
        }
        let mut left = dt;
        while left > 0.0 {
            let step = left.min(STEP_S);
            left -= step;
            self.step(step);
        }
        for f in &mut self.flashes {
            f.age += dt;
        }
        self.flashes.retain(|f| f.age < SMOKE_S);
        self.sky = (self.sky - dt / SKY_S).max(0.0);
        if self.sparks.len() > MOST {
            let over = self.sparks.len() - MOST;
            self.sparks.drain(..over);
        }
    }

    /// Everything stretched by `scale` (the display resized).
    fn rescale(&mut self, scale: Vec2) {
        let at = |p: Pos2| (p.to_vec2() * scale).to_pos2();
        for s in &mut self.sparks {
            s.at = at(s.at);
            s.speed = s.speed * scale;
            for p in &mut s.trail {
                *p = at(*p);
            }
        }
        for r in &mut self.rockets {
            r.at = at(r.at);
            r.speed = r.speed * scale;
            r.gravity *= scale.y;
            r.from_y *= scale.y;
            r.apex_y *= scale.y;
        }
        for f in &mut self.flashes {
            f.at = at(f.at);
            f.radius *= scale.y;
        }
    }

    /// A rocket up from `place` across, higher the stronger the note.
    fn launch(&mut self, place: f32, strength: f32) {
        let (w, h) = (self.size.x, self.size.y);
        let x = (0.06 + 0.88 * place) * w;
        let high = 0.5 * strength + 0.5 * self.rng.unit();
        let apex_y = h * (APEX[1] - (APEX[1] - APEX[0]) * high);
        let rise = self.rng.between(RISE_S[0], RISE_S[1]);
        let gravity = 2.0 * (h - apex_y) / (rise * rise);
        let speed = vec2(self.rng.between(-0.06, 0.06) * h, -gravity * rise);
        self.rockets.push(Rocket {
            at: pos2(x, h),
            speed,
            gravity,
            from_y: h,
            apex_y,
            place,
            strength,
            owed: 0.0,
        });
        self.idle = 0.0;
    }

    /// A beat of `strength`: the rocket highest up bursts as a large
    /// shell, or one bursts out of the dark; the sky is lit.
    fn strike(&mut self, strength: f32, treble: f32) {
        let size = 0.55 + 0.45 * strength;
        let shell = self.large_shell(strength, treble);
        let highest = self
            .rockets
            .iter()
            .enumerate()
            .filter(|(_, r)| r.risen() > RISEN)
            .min_by(|a, b| a.1.at.y.total_cmp(&b.1.at.y))
            .map(|(i, _)| i);
        let place = match highest {
            Some(i) => {
                let r = self.rockets.swap_remove(i);
                self.burst(r.at, r.place, size, shell);
                r.place
            }
            None => {
                let (w, h) = (self.size.x, self.size.y);
                let x = self.free_place() * w;
                let y = h * self.rng.between(APEX[0], APEX[1]);
                let place = self.rng.unit();
                self.burst(pos2(x, y), place, size, shell);
                if let Some(f) = self.flashes.last_mut() {
                    f.from_ground = true;
                }
                place
            }
        };
        if strength >= self.sky {
            self.sky_place = place;
        }
        self.sky = self.sky.max(strength);
    }

    /// A place across away from the last bursts: the best of a few tried.
    fn free_place(&mut self) -> f32 {
        let mut best = (0.5, -1.0);
        for _ in 0..5 {
            let x = self.rng.between(0.1, 0.9);
            let gap = self
                .recent
                .iter()
                .map(|r| (r - x).abs())
                .fold(f32::MAX, f32::min);
            if gap > best.1 {
                best = (x, gap);
            }
        }
        best.0
    }

    /// A beat's shell: a strong beat may be a willow; with much treble, a
    /// crackle.
    fn large_shell(&mut self, strength: f32, treble: f32) -> Shell {
        let r = self.rng.unit();
        if treble > 0.45 && r < 0.3 {
            return Shell::Crackle;
        }
        let r = self.rng.unit();
        match r {
            _ if strength > 0.7 && r < 0.22 => Shell::Willow,
            _ if r < 0.42 => Shell::Chrysanthemum,
            _ if r < 0.66 => Shell::Peony,
            _ if r < 0.84 => Shell::Ring,
            _ => Shell::Palm,
        }
    }

    /// A rocket's shell at the top of its climb, by its place: rings and
    /// peonies at the bass, chrysanthemums and palms in the middle,
    /// crackles at the treble.
    fn small_shell(&mut self, place: f32) -> Shell {
        let r = self.rng.unit();
        if place < 0.3 {
            if r < 0.4 { Shell::Ring } else { Shell::Peony }
        } else if place < 0.7 {
            match r {
                _ if r < 0.45 => Shell::Chrysanthemum,
                _ if r < 0.8 => Shell::Peony,
                _ => Shell::Palm,
            }
        } else if r < 0.7 {
            Shell::Crackle
        } else {
            Shell::Chrysanthemum
        }
    }

    /// A shell of `size` (0–1) bursting at `at`.
    fn burst(&mut self, at: Pos2, place: f32, size: f32, shell: Shell) {
        let (w, h) = (self.size.x, self.size.y);
        let radius = (h * (SHELL[0] + (SHELL[1] - SHELL[0]) * size)).min(w * SHELL_WIDE);
        let count = |base: f32| (base * (0.6 + 0.8 * size)) as usize;
        match shell {
            Shell::Ring => {
                let n = count(36.0);
                let tilt = self.rng.between(0.25, 1.0);
                let turn = self.rng.unit() * TAU;
                let (sin, cos) = turn.sin_cos();
                for k in 0..n {
                    let a = TAU * k as f32 / n as f32;
                    let v = vec2(a.cos(), a.sin() * tilt);
                    let v = vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
                    let v = v * self.rng.between(0.97, 1.0);
                    self.star(shell, at, v, radius, place);
                }
            }
            Shell::Palm => {
                let n = 6 + self.rng.between(0.0, 3.99) as usize;
                self.sphere(shell, at, radius, n, place);
            }
            Shell::Willow => self.sphere(shell, at, radius, count(44.0), place),
            Shell::Crackle => self.sphere(shell, at, radius, count(40.0), place),
            Shell::Peony | Shell::Chrysanthemum => {
                self.sphere(shell, at, radius, count(56.0), place);
                if self.rng.unit() < 0.4 {
                    // A core of another colour, further on along the colours.
                    let other = (place + self.rng.between(0.3, 0.6)).fract();
                    self.sphere(Shell::Peony, at, radius * 0.45, count(18.0), other);
                }
            }
        }
        self.flashes.push(Flash {
            at,
            place,
            radius,
            strength: size,
            age: 0.0,
            from_ground: false,
        });
        self.recent.rotate_right(1);
        self.recent[0] = at.x / w;
    }

    /// `n` stars all round at `at`, as a sphere's seen from the ground:
    /// crowding at the rim.
    fn sphere(&mut self, shell: Shell, at: Pos2, radius: f32, n: usize, place: f32) {
        let turn = self.rng.unit() * TAU;
        for k in 0..n {
            let a = turn + TAU * (k as f32 + self.rng.between(-0.3, 0.3)) / n as f32;
            let across = self.rng.between(-1.0, 1.0);
            let reach = (1.0 - across * across).sqrt() * self.rng.between(0.92, 1.0);
            self.star(shell, at, vec2(a.cos(), a.sin()) * reach, radius, place);
        }
    }

    /// A star of `shell` flying `way` (1: to the shell's radius).
    fn star(&mut self, shell: Shell, at: Pos2, way: Vec2, radius: f32, place: f32) {
        let mut s = Spark {
            at,
            speed: Vec2::ZERO,
            trail: [at; TRAIL],
            kept: 0,
            every: 0.03,
            since: 0.0,
            age: 0.0,
            life: 1.0,
            drag: 2.4,
            weight: 1.0,
            place,
            light: 1.0,
            width: 1.3,
            look: Look::Streak,
            gold: false,
            turns: false,
            splits: false,
            seed: self.rng.next() as u32,
        };
        let mut reach = 1.0;
        match shell {
            Shell::Peony => s.life = self.rng.between(1.0, 1.45),
            Shell::Chrysanthemum => {
                s.look = Look::Trail;
                s.every = 0.028;
                s.life = self.rng.between(1.1, 1.5);
                s.width = 1.2;
                s.turns = true;
            }
            Shell::Willow => {
                s.look = Look::Trail;
                s.every = 0.05;
                s.life = self.rng.between(2.2, 2.9);
                s.drag = 3.4;
                s.weight = 0.8;
                s.width = 1.0;
                s.light = 0.85;
                s.gold = true;
                reach = 0.9;
            }
            Shell::Ring => {
                s.life = self.rng.between(0.9, 1.3);
                s.drag = 2.2;
                s.width = 1.4;
            }
            Shell::Crackle => {
                s.life = self.rng.between(0.45, 0.75);
                s.drag = 2.8;
                s.width = 1.1;
                s.splits = true;
                reach = 0.7;
            }
            Shell::Palm => {
                s.look = Look::Trail;
                s.every = 0.035;
                s.life = self.rng.between(1.3, 1.7);
                s.drag = 1.9;
                s.weight = 1.2;
                s.width = 2.1;
            }
        }
        // Slowed by its drag, a star goes `speed / drag` at most.
        s.speed = way * radius * reach * s.drag;
        self.sparks.push(s);
    }

    /// The flight moved on by `step` seconds.
    fn step(&mut self, step: f32) {
        let (w, h) = (self.size.x, self.size.y);
        let gravity = GRAVITY * h;
        let mut embers = Vec::new();
        for r in &mut self.rockets {
            r.at += r.speed * step;
            r.speed.y += r.gravity * step;
            r.owed += EMBERS * step;
            while r.owed >= 1.0 {
                r.owed -= 1.0;
                embers.push((r.at, r.speed, r.place));
            }
        }
        let mut i = 0;
        while i < self.rockets.len() {
            if self.rockets[i].speed.y >= 0.0 {
                let r = self.rockets.swap_remove(i);
                let shell = self.small_shell(r.place);
                self.burst(r.at, r.place, 0.2 + 0.4 * r.strength, shell);
            } else {
                i += 1;
            }
        }
        for (at, speed, place) in embers {
            let spread = vec2(self.rng.between(-0.08, 0.08), self.rng.between(0.02, 0.12)) * h;
            self.sparks.push(Spark {
                at: at + vec2(self.rng.between(-0.6, 0.6), 0.0),
                speed: speed * 0.1 + spread,
                trail: [at; TRAIL],
                kept: 0,
                every: 0.03,
                since: 0.0,
                age: 0.0,
                life: self.rng.between(0.25, 0.5),
                drag: 3.0,
                weight: 0.6,
                place,
                light: 0.55,
                width: 0.9,
                look: Look::Ember,
                gold: true,
                turns: false,
                splits: false,
                seed: self.rng.next() as u32,
            });
        }
        for s in &mut self.sparks {
            let was = s.at;
            s.speed *= (-s.drag * step).exp();
            s.speed.y += gravity * s.weight * step;
            s.at += s.speed * step;
            s.age += step;
            if s.look == Look::Trail {
                s.since += step;
                if s.since >= s.every {
                    s.since = 0.0;
                    s.trail.rotate_right(1);
                    s.trail[0] = was;
                    s.kept = (s.kept + 1).min(TRAIL as u8);
                }
            }
        }
        let split: Vec<(Pos2, f32)> = self
            .sparks
            .iter()
            .filter(|s| s.splits && s.age >= s.life)
            .map(|s| (s.at, s.place))
            .collect();
        self.sparks
            .retain(|s| s.age < s.life && s.at.y < h + 4.0 && s.at.x > -40.0 && s.at.x < w + 40.0);
        for (at, place) in split {
            let n = 2 + (self.rng.unit() * 2.0) as usize;
            for _ in 0..n {
                let a = self.rng.unit() * TAU;
                let v = vec2(a.cos(), a.sin()) * self.rng.between(0.1, 0.25) * h;
                self.sparks.push(Spark {
                    at,
                    speed: v,
                    trail: [at; TRAIL],
                    kept: 0,
                    every: 0.03,
                    since: 0.0,
                    age: 0.0,
                    life: self.rng.between(0.15, 0.35),
                    drag: 3.0,
                    weight: 0.5,
                    place,
                    light: 1.2,
                    width: 0.9,
                    look: Look::Flicker,
                    gold: false,
                    turns: false,
                    splits: false,
                    seed: self.rng.next() as u32,
                });
            }
        }
    }

    /// Something still flies or glows: keep repainting.
    pub fn active(&self) -> bool {
        !self.sparks.is_empty()
            || !self.rockets.is_empty()
            || !self.flashes.is_empty()
            || self.beat.level > 0.0
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let colours = palette::visual_for(Visual::Fireworks);
        let dark = colours.is_dark();
        let hot = Color32::from_rgb(255, 248, 232);
        // Glows lie over a light ground rather than adding to it: softer.
        let haze = if dark { 1.0 } else { 0.5 };
        let thin = (rect.height() / REFERENCE_HEIGHT).clamp(0.6, 1.3);
        let offset = rect.min.to_vec2();
        let mut mesh = Mesh::default();
        if self.sky > 0.0 {
            let c = colours.at(0.6, self.sky_place);
            mesh.add_colored_rect(rect, glow(c, 0.09 * self.sky * self.sky * haze, dark));
        }
        for f in &self.flashes {
            let at = f.at + offset;
            let c = colours.at(0.8, f.place);
            let flash = (1.0 - f.age / FLASH_S).max(0.0);
            let smoke = 1.0 - f.age / SMOKE_S;
            let lit = if dark { hot.lerp_to_gamma(c, 0.5) } else { c };
            if flash > 0.0 {
                let a = 0.3 * f.strength * flash * flash * haze;
                disc(&mut mesh, at, f.radius * 1.3, glow(lit, a, dark));
                if f.from_ground {
                    let ground = pos2(at.x, rect.bottom());
                    let a = 0.7 * flash;
                    strip(&mut mesh, &[at, ground], 1.3 * thin, glow(lit, a, dark));
                }
            }
            let a = 0.07 * f.strength * smoke * smoke * haze;
            disc(
                &mut mesh,
                at,
                f.radius * (1.1 + 0.25 * f.age),
                glow(c, a, dark),
            );
        }
        for r in &self.rockets {
            let head = r.at + offset;
            let tail = head - r.speed * 0.05;
            let c = colours.peak(r.place);
            strip(&mut mesh, &[head, tail], 1.6 * thin, glow(c, 0.9, dark));
            disc(&mut mesh, head, 4.0 * thin, glow(c, 0.35 * haze, dark));
            dot(
                &mut mesh,
                head,
                1.3 * thin,
                glow(if dark { hot } else { c }, 1.0, dark),
            );
        }
        let mut points = Vec::with_capacity(TRAIL + 1);
        for s in &self.sparks {
            let t = s.age / s.life;
            let mut light = s.light * (1.0 - t * t * t) * (1.0 - t).sqrt();
            let mut colour = if s.gold && dark {
                // Burning charcoal's gold, tinged with the colour.
                colours.at(0.9, s.place).lerp_to_gamma(GOLD, 0.6)
            } else if s.gold {
                colours.at(0.9, s.place)
            } else {
                colours.at(1.0 - 0.3 * t, s.place)
            };
            let glitters = s.look == Look::Flicker || (s.turns && t > TURN);
            if s.turns && t > TURN {
                let turned = ((t - TURN) / 0.15).min(1.0);
                colour = colour.lerp_to_gamma(colours.peak(s.place), turned);
            }
            if glitters && !lit(s.seed, s.age) {
                light *= 0.12;
            }
            if dark && s.age < HOT_S {
                colour = hot.lerp_to_gamma(colour, s.age / HOT_S);
            } else if !dark {
                colour = colour.lerp_to_gamma(colours.peak(s.place), 0.25);
            }
            if light <= 0.003 {
                continue;
            }
            let head = s.at + offset;
            points.clear();
            points.push(head);
            if s.look == Look::Trail {
                points.extend(s.trail[..s.kept as usize].iter().map(|p| *p + offset));
            } else {
                let by = s.speed * STREAK_S;
                let most = STREAK_MOST * thin;
                points.push(head - by * (most / by.length().max(most)));
            }
            let width = s.width * thin;
            strip(
                &mut mesh,
                &points,
                width * 3.5,
                glow(colour, 0.16 * light, dark),
            );
            strip(&mut mesh, &points, width, glow(colour, 0.9 * light, dark));
            if s.look != Look::Ember {
                disc(
                    &mut mesh,
                    head,
                    width * 3.0,
                    glow(colour, 0.22 * light * haze, dark),
                );
            }
            let core = if dark {
                colour.lerp_to_gamma(Color32::WHITE, 0.35)
            } else {
                colour
            };
            dot(
                &mut mesh,
                head,
                width * 0.75,
                glow(core, light.min(1.0), dark),
            );
        }
        painter.add(mesh);
    }
}

/// The levels gathered into `GROUPS`, each the loudest of its bands.
fn grouped(levels: &[f32]) -> Vec<f32> {
    let n = levels.len();
    (0..GROUPS)
        .map(|g| {
            if n == 0 {
                return 0.0;
            }
            let a = (g * n / GROUPS).min(n - 1);
            let b = ((g + 1) * n / GROUPS).clamp(a + 1, n);
            levels[a..b].iter().copied().fold(0.0, f32::max)
        })
        .collect()
}

/// The upper third's share of the levels (0–1; a third when even).
fn treble_share(levels: &[f32]) -> f32 {
    let all: f32 = levels.iter().sum();
    let top: f32 = levels[levels.len() * 2 / 3..].iter().sum();
    if all > 0.0 { top / all } else { 0.0 }
}

/// A glittering star lit at `age` (five times in eight).
fn lit(seed: u32, age: f32) -> bool {
    let n = (age * TWINKLE_HZ) as u32;
    let x = (seed ^ n.wrapping_mul(0x9e37_79b9)).wrapping_mul(0x85eb_ca6b);
    (x >> 29) >= 3
}

/// A line through `points` from the head (`points[0]`, `width` wide in
/// `colour`) narrowing and fading to nothing at the last.
fn strip(mesh: &mut Mesh, points: &[Pos2], width: f32, colour: Color32) {
    let n = points.len();
    if n < 2 {
        return;
    }
    let base = mesh.vertices.len() as u32;
    for i in 0..n {
        let way = points[i.saturating_sub(1)] - points[(i + 1).min(n - 1)];
        let way = if way.length() > 1e-3 {
            way.normalized()
        } else {
            vec2(0.0, -1.0)
        };
        let k = 1.0 - i as f32 / (n - 1) as f32;
        let normal = way.rot90() * (width * 0.5 * (0.3 + 0.7 * k));
        let c = colour.gamma_multiply(k * k.sqrt());
        mesh.colored_vertex(points[i] + normal, c);
        mesh.colored_vertex(points[i] - normal, c);
    }
    for i in 0..n as u32 - 1 {
        let a = base + 2 * i;
        mesh.add_triangle(a, a + 1, a + 3);
        mesh.add_triangle(a, a + 3, a + 2);
    }
}

/// A soft round light at `centre`, `colour` there fading to nothing at
/// `radius`.
fn disc(mesh: &mut Mesh, centre: Pos2, radius: f32, colour: Color32) {
    const SIDES: u32 = 10;
    if radius < 0.5 || colour == Color32::TRANSPARENT {
        return;
    }
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(centre, colour);
    for i in 0..SIDES {
        let a = TAU * i as f32 / SIDES as f32;
        mesh.colored_vertex(
            centre + vec2(a.cos(), a.sin()) * radius,
            Color32::TRANSPARENT,
        );
        mesh.add_triangle(base, base + 1 + i, base + 1 + (i + 1) % SIDES);
    }
}

/// A small solid diamond at `centre`.
fn dot(mesh: &mut Mesh, centre: Pos2, radius: f32, colour: Color32) {
    let radius = radius.max(0.6);
    let base = mesh.vertices.len() as u32;
    for v in [
        vec2(0.0, -1.0),
        vec2(1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(-1.0, 0.0),
    ] {
        mesh.colored_vertex(centre + v * radius, colour);
    }
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Vec2 = vec2(760.0, 118.0);

    /// A display fed `quiet` levels for a second, so the means settle.
    fn settled(quiet: f32) -> Fireworks {
        let mut f = Fireworks::new();
        let levels = vec![quiet; BANDS];
        for _ in 0..50 {
            f.feed(Some(&levels), true, 0.02, SIZE);
        }
        f
    }

    #[test]
    fn notes_send_rockets_up_that_burst_at_the_top() {
        let mut f = settled(0.1);
        assert!(f.rockets.is_empty() && f.sparks.is_empty());
        // A note starts in the middle (not the bass: no beat).
        let mut note = vec![0.1; BANDS];
        note[12..15].fill(0.9);
        f.feed(Some(&note), true, 0.02, SIZE);
        assert_eq!(f.rockets.len(), 1);
        let place = f.rockets[0].place;
        assert!((0.5..0.625).contains(&place), "{place}");
        // It climbs, throwing embers, and bursts within its rise time.
        for _ in 0..20 {
            f.feed(Some(&note), true, 0.02, SIZE);
        }
        assert!(f.rockets.len() == 1 && f.rockets[0].at.y < SIZE.y * 0.8);
        assert!(f.sparks.iter().all(|s| s.look == Look::Ember));
        for _ in 0..40 {
            f.feed(Some(&note), true, 0.02, SIZE);
        }
        assert!(f.rockets.is_empty());
        assert!(f.sparks.iter().filter(|s| s.look != Look::Ember).count() >= 15);
        assert_eq!(f.flashes.len(), 1);
        // Paused, they stand.
        let before: Vec<Pos2> = f.sparks.iter().map(|s| s.at).collect();
        f.feed(Some(&note), false, 0.02, SIZE);
        assert_eq!(before, f.sparks.iter().map(|s| s.at).collect::<Vec<_>>());
        // Stopped, they go out.
        for _ in 0..200 {
            f.feed(None, true, 0.02, SIZE);
        }
        assert!(!f.active());
    }

    #[test]
    fn a_beat_bursts_the_rocket_highest_up_or_one_out_of_the_dark() {
        let mut f = settled(0.1);
        let mut beat = vec![0.1; BANDS];
        beat[..BANDS / 6].fill(0.9);
        f.feed(Some(&beat), true, 0.02, SIZE);
        // None up: one bursts out of the dark. The bass's notes send
        // rockets up from the left, for the next beat.
        assert!(f.sparks.len() >= 30, "{}", f.sparks.len());
        assert!(f.flashes[0].from_ground && f.sky > 0.0);
        assert!(!f.rockets.is_empty());
        assert!(f.rockets.iter().all(|r| r.place < 0.25 && r.risen() < 0.1));

        let mut f = settled(0.1);
        f.launch(0.3, 1.0);
        f.launch(0.7, 0.4);
        for _ in 0..30 {
            f.feed(Some(&[0.1; BANDS]), true, 0.02, SIZE);
        }
        let highest = f.rockets.iter().map(|r| r.at.y).fold(f32::MAX, f32::min);
        f.feed(Some(&beat), true, 0.02, SIZE);
        let up: Vec<&Rocket> = f.rockets.iter().filter(|r| r.risen() > 0.1).collect();
        assert_eq!(up.len(), 1);
        assert!(up[0].at.y > highest);
        assert!(!f.flashes[0].from_ground);
    }

    #[test]
    fn a_shell_opens_to_its_radius_crowding_at_the_rim() {
        let mut f = settled(0.0);
        let at = pos2(380.0, 40.0);
        f.burst(at, 0.5, 1.0, Shell::Peony);
        let radius = f.flashes[0].radius;
        assert!((radius - SIZE.y * SHELL[1]).abs() < 0.01);
        for _ in 0..50 {
            f.step(0.02);
        }
        // The shell's own stars (not a core of another colour).
        let stars: Vec<Pos2> = f
            .sparks
            .iter()
            .filter(|s| s.place == 0.5)
            .map(|s| s.at)
            .collect();
        // Fallen a little, not far.
        let middle = stars.iter().fold(Vec2::ZERO, |m, p| m + p.to_vec2()) / stars.len() as f32;
        assert!(
            middle.y > at.y && middle.y < at.y + 0.4 * SIZE.y,
            "{middle:?}"
        );
        let out: Vec<f32> = stars
            .iter()
            .map(|p| (*p - middle.to_pos2()).length())
            .collect();
        let widest = out.iter().copied().fold(0.0, f32::max);
        assert!(
            widest > 0.85 * radius && widest < 1.05 * radius,
            "{widest} {radius}"
        );
        let outer = out.iter().filter(|&&d| d > 0.5 * radius).count();
        assert!(outer * 4 > out.len() * 3, "{outer} of {}", out.len());
    }

    #[test]
    fn a_crackle_splits_into_flickers() {
        let mut f = settled(0.0);
        f.burst(pos2(380.0, 40.0), 0.5, 0.5, Shell::Crackle);
        for _ in 0..40 {
            f.step(0.02);
        }
        assert!(f.sparks.iter().any(|s| s.look == Look::Flicker));
        let flickers = f.sparks.iter().filter(|s| s.look == Look::Flicker);
        assert!(flickers.clone().any(|s| lit(s.seed, s.age)));
    }

    #[test]
    fn the_stars_kept_are_bounded() {
        let mut f = Fireworks::new();
        let mut loud = vec![1.0; BANDS];
        for i in 0..2000 {
            // Beats and notes all the time.
            let v = if i % 6 < 3 { 1.0 } else { 0.2 };
            loud.fill(v);
            f.feed(Some(&loud), true, 0.05, vec2(2000.0, 118.0));
            assert!(f.sparks.len() <= MOST && f.rockets.len() <= MOST_ROCKETS);
        }
    }

    #[test]
    fn resizing_keeps_them_in_the_display() {
        let mut f = Fireworks::new();
        f.feed(None, true, 0.0, vec2(800.0, 100.0));
        f.star(Shell::Peony, pos2(400.0, 40.0), vec2(0.5, 0.0), 40.0, 0.5);
        let speed = f.sparks[0].speed;
        // Resizing a paused scene changes only its scale, not its age.
        f.feed(Some(&[0.0; BANDS]), false, 0.02, vec2(400.0, 50.0));
        assert_eq!(f.sparks[0].at, pos2(200.0, 20.0));
        assert_eq!(f.sparks[0].speed, speed * vec2(0.5, 0.5));
        assert_eq!(f.sparks[0].age, 0.0);
    }

    #[test]
    fn groups_are_their_loudest_bands() {
        let mut levels = vec![0.0; BANDS];
        levels[4] = 0.5;
        levels[23] = 0.9;
        let g = grouped(&levels);
        assert_eq!(g.len(), GROUPS);
        assert_eq!((g[1], g[7], g[0]), (0.5, 0.9, 0.0));
        assert_eq!(grouped(&[]), vec![0.0; GROUPS]);
        assert_eq!(grouped(&[0.3; 3]).len(), GROUPS);
    }

    /// Frames of a few seconds of a beat with notes, drawn by hand into a
    /// PNG sheet to look at: `METALAMP_FRAMES=<file.png> cargo test
    /// fireworks_frames -- --ignored` (`METALAMP_THEME=light` for the
    /// light theme).
    #[test]
    #[ignore]
    fn fireworks_frames() {
        let Ok(out) = std::env::var("METALAMP_FRAMES") else {
            panic!("METALAMP_FRAMES=<file.png>");
        };
        if std::env::var("METALAMP_THEME").is_ok_and(|t| t == "light") {
            palette::set(palette::Look {
                theme: palette::Theme::Light,
                ..Default::default()
            });
        }
        let rect = Rect::from_min_size(Pos2::ZERO, SIZE);
        let mut f = Fireworks::new();
        let mut rng = Rng::default();
        let ctx = egui::Context::default();
        let scale = 2.0;
        let (w, h) = ((SIZE.x * scale) as usize, (SIZE.y * scale) as usize);
        let shots: Vec<usize> = (0..8).map(|k| 150 + k * 37).collect();
        let mut sheet = vec![0u8; w * h * shots.len() * 3];
        let mut notes = [0.0f32; BANDS];
        for frame in 0..=*shots.last().unwrap() {
            let t = frame as f32 * 0.02;
            // A kick every half second, notes now and then elsewhere.
            let kick = (1.0 - (t % 0.5) / 0.12).max(0.0);
            for n in &mut notes {
                *n = (*n - 0.02 / 0.25).max(0.0);
            }
            if rng.unit() < 0.12 {
                let at = 4 + (rng.unit() * (BANDS - 4) as f32) as usize;
                notes[at.min(BANDS - 1)] = rng.between(0.5, 0.95);
            }
            let levels: Vec<f32> = (0..BANDS)
                .map(|b| {
                    let bass = if b < BANDS / 6 {
                        0.35 + 0.6 * kick
                    } else {
                        0.3
                    };
                    bass.max(notes[b])
                })
                .collect();
            f.feed(Some(&levels), true, 0.02, SIZE);
            let Some(k) = shots.iter().position(|&s| s == frame) else {
                continue;
            };
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f.paint(ui.painter(), rect));
            out.textures_delta.clear();
            let prims = ctx.tessellate(out.shapes, 1.0);
            let vertices: usize = prims
                .iter()
                .map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                    _ => 0,
                })
                .sum();
            println!("{} stars, {vertices} vertices", f.sparks.len());
            let ground = palette::list_bg().to_array().map(|c| c as f32 / 255.0);
            let mut img = vec![[ground[0], ground[1], ground[2]]; w * h];
            for p in &prims {
                if let egui::epaint::Primitive::Mesh(m) = &p.primitive {
                    raster(&mut img, w, h, scale, m);
                }
            }
            for (i, c) in img.iter().enumerate() {
                let o = (k * w * h + i) * 3;
                for j in 0..3 {
                    sheet[o + j] = (c[j].clamp(0.0, 1.0) * 255.0) as u8;
                }
            }
        }
        image::save_buffer(
            &out,
            &sheet,
            w as u32,
            (h * shots.len()) as u32,
            image::ColorType::Rgb8,
        )
        .unwrap();
    }

    /// A mesh's triangles blended into `img` as premultiplied colours.
    fn raster(img: &mut [[f32; 3]], w: usize, h: usize, scale: f32, m: &Mesh) {
        for tri in m.indices.chunks(3) {
            let v = [0, 1, 2].map(|i| &m.vertices[tri[i] as usize]);
            let p = v.map(|v| v.pos.to_vec2() * scale);
            let area = (p[1] - p[0]).x * (p[2] - p[0]).y - (p[1] - p[0]).y * (p[2] - p[0]).x;
            if area.abs() < 1e-6 {
                continue;
            }
            let lo = p.iter().fold(vec2(f32::MAX, f32::MAX), |a, b| a.min(*b));
            let hi = p.iter().fold(vec2(f32::MIN, f32::MIN), |a, b| a.max(*b));
            let (x0, x1) = (lo.x.max(0.0) as usize, (hi.x.ceil() as usize).min(w));
            let (y0, y1) = (lo.y.max(0.0) as usize, (hi.y.ceil() as usize).min(h));
            for y in y0..y1 {
                for x in x0..x1 {
                    let q = vec2(x as f32 + 0.5, y as f32 + 0.5);
                    let edge = |a: Vec2, b: Vec2| (b - a).x * (q - a).y - (b - a).y * (q - a).x;
                    let l =
                        [edge(p[1], p[2]), edge(p[2], p[0]), edge(p[0], p[1])].map(|e| e / area);
                    if l.iter().any(|&l| l < 0.0) {
                        continue;
                    }
                    let c = |i: usize| -> [f32; 4] {
                        let c = v[i].color.to_array();
                        c.map(|x| x as f32 / 255.0)
                    };
                    let (a, b, cc) = (c(0), c(1), c(2));
                    let s: [f32; 4] =
                        std::array::from_fn(|j| a[j] * l[0] + b[j] * l[1] + cc[j] * l[2]);
                    let d = &mut img[y * w + x];
                    for j in 0..3 {
                        d[j] = s[j] + d[j] * (1.0 - s[3]);
                    }
                }
            }
        }
    }
}
