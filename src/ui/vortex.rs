//! Vortex: a picture with feedback, as Winamp's MilkDrop and Geiss drew
//! theirs. Each frame the picture before is drawn again moved as the scene
//! in play says (grown from the middle, faster towards the edges or the
//! middle, stretched, turned steadily, drifted, rippled with the bass,
//! stirred by running sines), and dimmed; then the
//! sound heard now is drawn in the scene's shape (`Shape`: the wave on a
//! carrier across the middle, two of them, a ring, MilkDrop's XY spiral,
//! the wave against itself a moment later, rays from the middle) in
//! the display's colours, brighter the louder. So every wave leaves trails
//! that fly, swirl and fade. The picture shown may have an echo of itself
//! mixed in, zoomed and mirrored (MilkDrop's video echo), which makes it
//! symmetric. The scenes (`SCENES`) follow one another: the first is the
//! vortex as it was; after `SCENE_LEAST` of sound another comes on a beat,
//! after `SCENE_MOST` anyway, in a random order of its own each time (a
//! shuffled round of the others: each shown before any comes again, never
//! the same one twice running; the clock's seed, so not the same order at
//! every start, which looked like the scenes going in turn), blended
//! over `BLEND_S` (every number of the two mixed, both shapes drawn, both
//! echoes shown). On each beat (the bass rising over its recent mean) the
//! picture is thrown outwards faster and the wave flares, drawn towards
//! white. The move is worked out at the corners of cells of `CELL` pixels
//! and spread between them, as MilkDrop's mesh. The picture is a buffer of
//! at most `PIXELS` pixels (premultiplied colour and coverage, floats so
//! the dimming does not stall at a dark grey as it would in bytes), shown
//! as a texture stretched over the display with linear filtering. Paused,
//! it stands; stopped, it moves on and fades until nothing is left.

use std::borrow::Cow;
use std::f32::consts::{PI, TAU};

use egui::{Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions, Vec2, pos2};

use crate::palette::{self, VisualPalette};
use crate::ui::beat::Rng;

/// The bands of the levels fed (the bass and the overall level).
pub const BANDS: usize = 32;
/// The wave drawn each frame: the last so many seconds heard.
pub const WAVE_S: f32 = 0.025;
/// The picture's pixels at most, and its width at most (a pixel a point
/// up to them: stretched, it blurs).
const PIXELS: f32 = 120_000.0;
const MAX_WIDTH: f32 = 1024.0;
/// The move is worked out every so many pixels across and down.
const CELL: usize = 8;
/// `CUT` a second is taken off the light besides its halving, so faint
/// light goes soon (the blur of the sampling gathers in it) and the last
/// of it with `active`.
const CUT: f32 = 0.2;
/// A kick grows the picture this much more a second at its height, a
/// surge forwards fading in `KICK_S` (it turned the picture too, each
/// kick the other way: the user found it rocking).
const KICK_ZOOM: f32 = 1.2;
const KICK_S: f32 = 0.5;
/// The ripple's waves per half height, and per second.
const RIPPLE_K: f32 = 5.0;
const RIPPLE_HZ: f32 = 1.0;
/// The stir's waves per half height.
const STIR_K: f32 = 2.2;
/// The line's carrier: this high (a share of the half height), so many
/// waves across the display, running so many a second to the right.
const CARRIER: f32 = 0.25;
const CARRIER_WAVES: f32 = 1.5;
const CARRIER_HZ: f32 = 0.8;
/// The line's swing as a share of the half height, and the light it
/// leaves at full level.
const WAVE_HEIGHT: f32 = 0.4;
const GAIN: f32 = 0.8;
/// The ring's radius and swing, in half heights.
const RING: f32 = 0.55;
const RING_SWING: f32 = 0.3;
/// The shapes drawn from the samples one after another: at most so many
/// points, the later sample this share of the wave on, and the light a
/// pixel of the way leaves (of a step of up to `RUN` pixels; a longer
/// leap leaves no more, spread along it, as a beam's).
const POINTS: usize = 600;
const LATER: f32 = 0.03;
const STEP_LIGHT: f32 = 0.3;
const RUN: f32 = 3.0;
/// The loop: its half size, its orbit's half width and height (half
/// heights), and its turn and orbit's in radians a second (in the middle
/// and still, a slow bass drew it over itself into a blot). The spiral
/// leaves half the light of the other shapes, drawn over itself more.
const LOOP_SIZE: f32 = 0.45;
const LOOP_ORBIT: Vec2 = Vec2::new(1.3, 0.45);
const LOOP_TURN: f32 = 0.8;
/// The rays: so many, from so far out (half heights) and so long at the
/// peak (the square root of a share of it: between the kicks the peaks
/// are small), and the light a pixel of them leaves.
const RAYS: usize = 40;
const RAY_FROM: f32 = 0.12;
const RAY_LONG: f32 = 0.9;
const RAY_LIGHT: f32 = 0.6;
/// The samples are drawn as shares of their peak, which falls by e in
/// `PEAK_S`, from `PEAK_LEAST` up.
const PEAK_S: f32 = 1.0;
const PEAK_LEAST: f32 = 0.05;
/// A scene plays at least so many seconds of sound before a beat brings
/// the next, at most so many, and blends into the next over `BLEND_S`.
const SCENE_LEAST: f32 = 14.0;
const SCENE_MOST: f32 = 30.0;
const BLEND_S: f32 = 2.5;
/// A beat: the bass this far over its mean of the last `MEAN_S`, and at
/// least `KICK_LEAST`, `KICK_GAP_S` after the last.
const KICK_RISE: f32 = 0.06;
const KICK_LEAST: f32 = 0.3;
const KICK_GAP_S: f32 = 0.12;
const MEAN_S: f32 = 0.4;
/// The levels shown fall the whole way in this long.
const FALL_S: f32 = 0.3;

/// What a scene draws the sound as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    /// The wave on a carrier across the middle.
    Line,
    /// The left channel's wave over the middle, the right's under it, on
    /// carriers mirrored.
    Lines,
    /// A ring about the middle, its radius swinging with the wave.
    Ring,
    /// MilkDrop's XY spiral: the radius from a sample of the right, the
    /// angle from a later one of the left.
    Spiral,
    /// The left against the right a moment later: loops, as an
    /// oscilloscope's XY mode draws.
    Loop,
    /// Rays from the middle, each as long as the peak of its stretch of
    /// the wave, turning slowly.
    Rays,
}

/// How the echo is turned over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mirror {
    None,
    /// Left for right.
    Across,
    /// Top for bottom.
    Down,
    /// Both: half a turn.
    Both,
}

/// How the picture moves and what is drawn on it. The middle stays put
/// and every scene flies forwards, turning steadily if at all: the middle
/// wandering and the turn swinging one way and the other, the user found
/// the picture rocking.
#[derive(Clone, Copy, Debug)]
struct Scene {
    shape: Shape,
    /// The picture grows this share a second...
    zoom: f32,
    /// ...times this at the edge and its inverse in the middle (MilkDrop's
    /// zoom exponent: over 1 a tunnel, under 1 a bulge).
    edge: f32,
    /// More growth a second across and down.
    stretch: Vec2,
    /// The turn, in radians a second (anticlockwise on the screen when
    /// over 0).
    swirl: f32,
    /// The picture moves so many half heights a second.
    drift: Vec2,
    /// The ripple and the stir, in half heights a second.
    ripple: f32,
    stir: f32,
    /// The light halves this often.
    half_s: f32,
    /// The echo mixed into the picture shown, its zoom (at least 1: a
    /// smaller one shows the picture's edges as a frame) and its turn.
    echo: f32,
    echo_zoom: f32,
    mirror: Mirror,
    /// The colours' places run on so many times a second.
    hue: f32,
}

/// The vortex as it was before the scenes, held steady.
const CLASSIC: Scene = Scene {
    shape: Shape::Line,
    zoom: 0.7,
    edge: 1.0,
    stretch: Vec2::ZERO,
    swirl: 0.12,
    drift: Vec2::ZERO,
    ripple: 0.3,
    stir: 0.0,
    half_s: 0.3,
    echo: 0.0,
    echo_zoom: 1.0,
    mirror: Mirror::None,
    hue: 0.0,
};

const SCENES: [Scene; 8] = [
    CLASSIC,
    // A tunnel: rings flying out, faster at the edges.
    Scene {
        shape: Shape::Ring,
        zoom: 1.0,
        edge: 1.5,
        swirl: 0.1,
        ripple: 0.0,
        half_s: 0.3,
        hue: 0.03,
        ..CLASSIC
    },
    // A spiral turning one way as it flies out, its top and bottom alike.
    Scene {
        shape: Shape::Spiral,
        zoom: 0.6,
        edge: 1.3,
        swirl: 0.5,
        ripple: 0.0,
        half_s: 0.35,
        echo: 0.5,
        mirror: Mirror::Down,
        ..CLASSIC
    },
    // Smoke: two waves flying out wide, rising a little and stirred.
    Scene {
        shape: Shape::Lines,
        zoom: 0.5,
        stretch: Vec2::new(0.3, 0.0),
        swirl: 0.0,
        drift: Vec2::new(0.0, -0.15),
        ripple: 0.0,
        stir: 0.25,
        half_s: 0.45,
        ..CLASSIC
    },
    // A kaleidoscope: loops bulging out of the middle, left and right
    // alike.
    Scene {
        shape: Shape::Loop,
        zoom: 0.6,
        edge: 0.8,
        swirl: -0.25,
        ripple: 0.0,
        half_s: 0.35,
        echo: 0.5,
        mirror: Mirror::Across,
        ..CLASSIC
    },
    // A starburst, with a larger echo of it about it.
    Scene {
        shape: Shape::Rays,
        zoom: 0.9,
        edge: 1.6,
        swirl: 0.1,
        ripple: 0.0,
        stir: 0.1,
        half_s: 0.25,
        echo: 0.3,
        echo_zoom: 1.5,
        hue: 0.05,
        ..CLASSIC
    },
    // A whirlpool: rings flying out and turning, half a turn of it over
    // itself.
    Scene {
        shape: Shape::Ring,
        zoom: 0.7,
        edge: 1.5,
        swirl: -0.8,
        ripple: 0.0,
        half_s: 0.45,
        echo: 0.35,
        echo_zoom: 1.25,
        mirror: Mirror::Both,
        ..CLASSIC
    },
    // A corridor: the two waves flying out to both sides, faster across
    // than down, the top and the bottom alike.
    Scene {
        shape: Shape::Lines,
        zoom: 0.3,
        stretch: Vec2::new(0.6, 0.0),
        swirl: 0.0,
        ripple: 0.0,
        half_s: 0.4,
        echo: 0.4,
        mirror: Mirror::Down,
        ..CLASSIC
    },
];

impl Scene {
    /// `self` and `other` mixed, `t` of the way to `other` (the shape and
    /// the mirror are `self`'s: both are drawn and shown apart).
    fn mix(&self, other: &Scene, t: f32) -> Scene {
        let m = |a: f32, b: f32| a + (b - a) * t;
        let v = |a: Vec2, b: Vec2| a + (b - a) * t;
        Scene {
            shape: self.shape,
            zoom: m(self.zoom, other.zoom),
            edge: m(self.edge, other.edge),
            stretch: v(self.stretch, other.stretch),
            swirl: m(self.swirl, other.swirl),
            drift: v(self.drift, other.drift),
            ripple: m(self.ripple, other.ripple),
            stir: m(self.stir, other.stir),
            half_s: m(self.half_s, other.half_s),
            echo: m(self.echo, other.echo),
            echo_zoom: m(self.echo_zoom, other.echo_zoom),
            mirror: self.mirror,
            hue: m(self.hue, other.hue),
        }
    }
}

/// The sound heard now: the last `WAVE_S` of it (mono), their side when
/// known, and the levels of `BANDS` bands.
#[derive(Clone, Copy)]
pub struct Sound<'a> {
    pub wave: &'a [f32],
    pub side: Option<&'a [f32]>,
    pub levels: &'a [f32],
}

/// An echo shown: its share, zoom and turn.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Echo {
    share: f32,
    zoom: f32,
    mirror: Mirror,
}

/// What a wave is drawn with.
#[derive(Clone, Copy)]
struct Ink {
    colours: VisualPalette,
    bright: Color32,
    /// Towards `bright` by a kick.
    flare: f32,
    /// The level, 0 to 1, and the light a point leaves.
    light: f32,
    weight: f32,
    /// How far the colours' places have run.
    hue: f32,
}

impl Ink {
    /// The colour of a point at `place` (0 to 1 across the display), whose
    /// sample swings `swing`.
    fn rgb(&self, swing: f32, place: f32) -> [f32; 3] {
        let level = if self.colours.by_level() {
            swing.abs().min(1.0)
        } else {
            self.light
        };
        // Run on and back, so the colours never jump.
        let x = (place * 0.5 + self.hue).rem_euclid(1.0);
        let place = 1.0 - (1.0 - 2.0 * x).abs();
        let c = self
            .colours
            .at(level, place)
            .lerp_to_gamma(self.bright, self.flare);
        [c.r(), c.g(), c.b()].map(|c| f32::from(c) / 255.0)
    }
}

pub struct Vortex {
    /// The picture, row by row from the top: premultiplied colour and
    /// coverage; and the room the next one is made in.
    pixels: Vec<[f32; 4]>,
    scratch: Vec<[f32; 4]>,
    /// Where each corner of the cells takes its colour from, and a row of
    /// them between two rows of corners.
    mesh: Vec<[f32; 2]>,
    row: Vec<[f32; 2]>,
    width: usize,
    height: usize,
    /// Something is lit.
    lit: bool,
    time: f32,
    /// How far the colours' places have run.
    hue: f32,
    /// The bass and the overall level shown (falling at `FALL_S`), the
    /// bass's recent mean, a kick's height and the time since the last.
    bass: f32,
    level: f32,
    mean: f32,
    kick: f32,
    since_kick: f32,
    /// The samples' peak followed.
    peak: f32,
    /// The scene playing, the one it blends from, how far (1: done), and
    /// the seconds of sound it has played.
    scene: usize,
    from: usize,
    blend: f32,
    played: f32,
    rng: Rng,
    /// The scenes still to come in this round, the next last.
    round: Vec<usize>,
    /// The echoes shown, of the two scenes.
    echoes: [Echo; 2],
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
        let none = Echo {
            share: 0.0,
            zoom: 1.0,
            mirror: Mirror::None,
        };
        Vortex {
            pixels: Vec::new(),
            scratch: Vec::new(),
            mesh: Vec::new(),
            row: Vec::new(),
            width: 0,
            height: 0,
            lit: false,
            time: 0.0,
            hue: 0.0,
            bass: 0.0,
            level: 0.0,
            mean: 0.0,
            kick: 0.0,
            since_kick: 0.0,
            peak: PEAK_LEAST,
            scene: 0,
            from: 0,
            blend: 1.0,
            played: 0.0,
            rng: Rng::from_clock(),
            round: Vec::new(),
            echoes: [none; 2],
            texture: None,
            changed: false,
            ground: Color32::TRANSPARENT,
        }
    }

    /// Moves on by `dt` seconds with the sound heard now (`None`: nothing
    /// heard; `moved` false: paused, nothing new heard) in a display of
    /// `size` points.
    pub fn feed(&mut self, heard: Option<Sound<'_>>, moved: bool, dt: f32, size: Vec2) {
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
                .map(|s| s.levels)
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
        let kicked = heard.is_some()
            && rise > KICK_RISE
            && bass > KICK_LEAST
            && self.since_kick > KICK_GAP_S;
        if kicked {
            self.kick = (0.5 + (rise - KICK_RISE) * 8.0).clamp(0.4, 1.0);
            self.since_kick = 0.0;
        }
        if heard.is_none() && !self.lit {
            return;
        }
        if heard.is_some() {
            self.played += dt;
            if self.blend >= 1.0
                && (self.played > SCENE_MOST || (kicked && self.played > SCENE_LEAST))
            {
                self.next_scene();
            }
        }
        self.blend = (self.blend + dt / BLEND_S).min(1.0);
        let t = smooth(self.blend);
        let (from, to) = (SCENES[self.from], SCENES[self.scene]);
        let look = from.mix(&to, t);
        self.time += dt;
        self.hue += look.hue * dt;
        self.echoes = [(from, 1.0 - t), (to, t)].map(|(s, k)| Echo {
            share: s.echo * k,
            zoom: s.echo_zoom,
            mirror: s.mirror,
        });
        self.warp(dt, &look);
        if let Some(sound) = heard {
            let peak = sound.wave.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            self.peak = peak.max(self.peak * (-dt / PEAK_S).exp()).max(PEAK_LEAST);
            if t < 1.0 {
                self.draw(from.shape, sound, 1.0 - t);
            }
            if t > 0.0 {
                self.draw(to.shape, sound, t);
            }
        }
        self.changed = true;
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        self.lit
    }

    /// The scenes there are, and the `i`th played from now on, blended
    /// into from none (frames to look at).
    #[cfg(test)]
    pub const SCENES: usize = SCENES.len();
    #[cfg(test)]
    pub fn play_scene(&mut self, i: usize) {
        self.scene = i;
        self.from = i;
        self.blend = 1.0;
        self.played = f32::MIN;
    }

    /// Another scene than the one playing, blended into from it: the next
    /// of a round of the others in a random order, a new round once it is
    /// done (picked at random each time, two scenes came back in turn).
    fn next_scene(&mut self) {
        if self.round.is_empty() {
            self.round = (0..SCENES.len()).filter(|&i| i != self.scene).collect();
            self.rng.shuffle(&mut self.round);
        }
        let Some(next) = self.round.pop() else {
            return;
        };
        self.from = self.scene;
        self.scene = next;
        self.blend = 0.0;
        self.played = 0.0;
    }

    /// The middle the picture moves about and the shapes are drawn round,
    /// in pixels.
    fn middle(&self) -> (f32, f32) {
        (self.width as f32 / 2.0, self.height as f32 / 2.0)
    }

    /// The picture moved by `dt` seconds' worth of `look` and dimmed: each
    /// pixel of the new one takes the old one's colour where it came from
    /// (sampled between four pixels), that place worked out at the cells'
    /// corners and spread between them.
    fn warp(&mut self, dt: f32, look: &Scene) {
        let (w, h) = (self.width, self.height);
        let half = h as f32 / 2.0;
        let t = self.time;
        let (cx, cy) = self.middle();
        let reach = (w as f32 / h as f32).sqrt();
        let zoom = look.zoom + KICK_ZOOM * self.kick;
        let ln_edge = look.edge.max(0.01).ln();
        let turn = -look.swirl * dt;
        let (sn, cs) = turn.sin_cos();
        let ripple = look.ripple * (0.3 + 0.7 * self.bass) * dt;
        let stir = look.stir * dt;
        let drift = look.drift * dt;
        let (nx, ny) = ((w - 1) / CELL + 2, (h - 1) / CELL + 2);
        self.mesh.clear();
        for gy in 0..ny {
            let py = ((gy * CELL) as f32 + 0.5 - cy) / half;
            for gx in 0..nx {
                let px = ((gx * CELL) as f32 + 0.5 - cx) / half;
                let rx = px + ripple * (py * RIPPLE_K + t * RIPPLE_HZ * TAU).sin();
                let ry = py + ripple * (px * RIPPLE_K * 0.7 + t * RIPPLE_HZ * 1.3 * TAU).sin();
                let r = ((px * px + py * py).sqrt() / reach).min(1.0);
                let grow = zoom * (ln_edge * (2.0 * r - 1.0)).exp();
                let mut sx = (rx * cs - ry * sn) * (-(grow + look.stretch.x) * dt).exp();
                let mut sy = (rx * sn + ry * cs) * (-(grow + look.stretch.y) * dt).exp();
                if stir > 0.0 {
                    let (ux, uy) = stirred(px, py, t);
                    sx += stir * ux;
                    sy += stir * uy;
                }
                self.mesh.push([
                    (sx - drift.x) * half + cx - 0.5,
                    (sy - drift.y) * half + cy - 0.5,
                ]);
            }
        }
        let dim = 0.5f32.powf(dt / look.half_s.max(0.01));
        let cut = CUT * dt;
        let mut lit = false;
        self.row.resize(nx, [0.0; 2]);
        let cell = CELL as f32;
        for (y, out_row) in self.scratch.chunks_mut(w).enumerate() {
            let (gy, fy) = (y / CELL, (y % CELL) as f32 / cell);
            let (above, below) = (
                &self.mesh[gy * nx..][..nx],
                &self.mesh[(gy + 1) * nx..][..nx],
            );
            for ((r, a), b) in self.row.iter_mut().zip(above).zip(below) {
                *r = [a[0] + (b[0] - a[0]) * fy, a[1] + (b[1] - a[1]) * fy];
            }
            for (x, out) in out_row.iter_mut().enumerate() {
                let (gx, fx) = (x / CELL, (x % CELL) as f32 / cell);
                let (a, b) = (self.row[gx], self.row[gx + 1]);
                let mut p = sample(
                    &self.pixels,
                    w,
                    h,
                    a[0] + (b[0] - a[0]) * fx,
                    a[1] + (b[1] - a[1]) * fx,
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

    /// The sound as `shape`, its light `share` of the whole (two shapes
    /// share it while the scenes blend).
    fn draw(&mut self, shape: Shape, sound: Sound<'_>, share: f32) {
        let (w, h) = (self.width, self.height);
        let wave = sound.wave;
        if wave.is_empty() || w < 2 {
            return;
        }
        let light = ((self.level - 0.1) / 0.8).clamp(0.0, 1.0);
        let ink = Ink {
            colours: palette::visual_for(crate::settings::Visual::Vortex),
            bright: palette::text_bright(),
            flare: self.kick * 0.6,
            light,
            weight: GAIN * (0.25 + 0.75 * light) * share,
            hue: self.hue,
        };
        let mid = h as f32 / 2.0;
        // The channels, as shares of their peak.
        let norm = 1.0 / self.peak;
        let side = sound.side.filter(|s| s.len() == wave.len());
        let left = |i: usize| (wave[i] + side.map_or(0.0, |s| s[i])) * norm;
        let right = |i: usize| (wave[i] - side.map_or(0.0, |s| s[i])) * norm;
        let (cx, cy) = self.middle();
        let half = mid;
        let n = wave.len();
        let later = ((n as f32 * LATER) as usize).clamp(1, n - 1);
        match shape {
            Shape::Line => {
                self.line(|i| wave[i], n, mid, 1.0, WAVE_HEIGHT * mid, &ink);
            }
            Shape::Lines => {
                let ink = Ink {
                    weight: ink.weight * 0.7,
                    ..ink
                };
                let swing = 0.3 * mid * self.peak;
                self.line(|i| left(i) * 0.5, n, mid * 0.6, 1.0, swing, &ink);
                self.line(|i| right(i) * 0.5, n, mid * 1.4, -1.0, swing, &ink);
            }
            Shape::Ring => {
                let points = ((TAU * RING * half) as usize).clamp(48, 720);
                // A tenth more samples than points: the first tenth blends
                // into the samples past the end, so the ring closes.
                let span = points + points / 10;
                let at =
                    |i: usize| left(i * (n - 1) / span) * 0.5 + right(i * (n - 1) / span) * 0.5;
                let spin = self.time * 0.2;
                let ring: Vec<_> = (0..=points)
                    .map(|i| {
                        let i = i % points;
                        let mut s = at(i);
                        if i < points / 10 {
                            let k = 0.5 - 0.5 * (PI * i as f32 / (points / 10) as f32).cos();
                            s = at(i + points) * (1.0 - k) + s * k;
                        }
                        let r = RING + RING_SWING * s;
                        let a = TAU * i as f32 / points as f32 + spin;
                        (cx + r * a.cos() * half, cy + r * a.sin() * half, s)
                    })
                    .collect();
                self.stroke(&ring, &ink, 1.0);
            }
            Shape::Rays => {
                // Each ray a stretch of the wave, as long as its peak.
                let spin = self.time * 0.3;
                let stretch = (n / RAYS).max(1);
                for k in 0..RAYS {
                    let from = (k * n / RAYS).min(n - 1);
                    let peak = (from..(from + stretch).min(n))
                        .map(|i| left(i).abs().max(right(i).abs()))
                        .fold(0.0f32, f32::max)
                        .min(1.5);
                    let a = TAU * k as f32 / RAYS as f32 + spin;
                    let (sn, cs) = a.sin_cos();
                    let (r0, r1) = (RAY_FROM, RAY_FROM + RAY_LONG * peak.sqrt());
                    let steps = (((r1 - r0) * half / 2.0) as usize).max(1);
                    let ray: Vec<_> = (0..=steps)
                        .map(|j| {
                            let r = r0 + (r1 - r0) * j as f32 / steps as f32;
                            (cx + r * cs * half, cy + r * sn * half, peak)
                        })
                        .collect();
                    self.stroke(&ray, &ink, RAY_LIGHT);
                }
            }
            Shape::Spiral | Shape::Loop => {
                let count = (n - later).min(POINTS);
                let spin = self.time * 2.3;
                let turn = (self.time * LOOP_TURN).sin_cos();
                let points: Vec<_> = (0..count)
                    .map(|k| {
                        let i = k * (n - later) / count;
                        let (l, r, ll, rl) = (left(i), right(i), left(i + later), right(i + later));
                        let (x, y) = match shape {
                            Shape::Spiral => {
                                let radius = 0.53 + 0.43 * r * 0.5;
                                let a = ll * 1.57 + spin;
                                (radius * a.cos(), radius * a.sin())
                            }
                            _ => {
                                // Turning on an orbit round the middle.
                                let (x, y) = (l * LOOP_SIZE, rl * LOOP_SIZE);
                                (
                                    x * turn.1 - y * turn.0 + LOOP_ORBIT.x * turn.1,
                                    x * turn.0 + y * turn.1 + LOOP_ORBIT.y * turn.0,
                                )
                            }
                        };
                        (cx + x * half, cy + y * half, l * 0.5)
                    })
                    .collect();
                let light = if shape == Shape::Spiral {
                    STEP_LIGHT / 2.0
                } else {
                    STEP_LIGHT
                };
                self.stroke(&points, &ink, light);
            }
        }
        self.lit = true;
    }

    /// A wave on a carrier across, a column a pixel: from the column
    /// before's height to this one's, the light spread along the run, a
    /// pixel wide. `at(i)` is the `i`th of `n` samples; the carrier runs
    /// about `base`, turned over by `turn` (1 or -1).
    fn line(
        &mut self,
        at: impl Fn(usize) -> f32,
        n: usize,
        base: f32,
        turn: f32,
        swing: f32,
        ink: &Ink,
    ) {
        let (w, h) = (self.width, self.height);
        let mid = h as f32 / 2.0;
        let mut last = None;
        for x in 0..w {
            let s = at(x * n / w).clamp(-1.0, 1.0);
            let place = x as f32 / (w - 1) as f32;
            let carrier = (TAU * (CARRIER_WAVES * place - CARRIER_HZ * self.time)).sin();
            let y = base + turn * CARRIER * mid * carrier - s * swing;
            let rgb = ink.rgb(s, place);
            let from = last.unwrap_or(y);
            let steps = ((y - from).abs().ceil() as usize).max(1);
            let k = ink.weight / steps as f32;
            for j in 0..steps {
                let yy = from + (y - from) * (j as f32 + 0.5) / steps as f32;
                splat(&mut self.pixels, w, h, x as f32, yy - 0.5, rgb, k);
            }
            last = Some(y);
        }
    }

    /// Lines through `points` (x and y in pixels, the sample's swing, the
    /// colours by the place across), a pixel of them leaving `light` times the
    /// ink's weight, a step longer than `RUN` no more than one of it.
    fn stroke(&mut self, points: &[(f32, f32, f32)], ink: &Ink, light: f32) {
        let (w, h) = (self.width, self.height);
        let most = (w + h) as f32;
        for pair in points.windows(2) {
            let (x0, y0, _) = pair[0];
            let (x1, y1, s) = pair[1];
            let rgb = ink.rgb(s, (x1 / w as f32).clamp(0.0, 1.0));
            let span = (x1 - x0).abs().max((y1 - y0).abs()).min(most);
            let steps = (span.ceil() as usize).max(1);
            let k = ink.weight * light * span.min(RUN) / steps as f32;
            for j in 0..steps {
                let f = (j as f32 + 0.5) / steps as f32;
                splat(
                    &mut self.pixels,
                    w,
                    h,
                    x0 + (x1 - x0) * f - 0.5,
                    y0 + (y1 - y0) * f - 0.5,
                    rgb,
                    k,
                );
            }
        }
    }

    /// The picture as shown: with the echoes mixed in.
    fn shown(&self) -> Cow<'_, [[f32; 4]]> {
        let (w, h) = (self.width, self.height);
        let echoes: Vec<Echo> = self
            .echoes
            .iter()
            .copied()
            .filter(|e| e.share > 0.001)
            .collect();
        if echoes.is_empty() {
            return Cow::Borrowed(&self.pixels);
        }
        let keep = 1.0 - echoes.iter().map(|e| e.share).sum::<f32>();
        let (mx, my) = (w as f32 / 2.0, h as f32 / 2.0);
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut p = self.pixels[y * w + x].map(|c| c * keep);
                for e in &echoes {
                    let (mut fx, mut fy) = (x as f32, y as f32);
                    if matches!(e.mirror, Mirror::Across | Mirror::Both) {
                        fx = (w - 1) as f32 - fx;
                    }
                    if matches!(e.mirror, Mirror::Down | Mirror::Both) {
                        fy = (h - 1) as f32 - fy;
                    }
                    let ex = (fx + 0.5 - mx) / e.zoom + mx - 0.5;
                    let ey = (fy + 0.5 - my) / e.zoom + my - 0.5;
                    let q = sample(&self.pixels, w, h, ex, ey);
                    for (c, q) in p.iter_mut().zip(q) {
                        *c += q * e.share;
                    }
                }
                out.push(p);
            }
        }
        Cow::Owned(out)
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
            for (out, p) in image.pixels.iter_mut().zip(self.shown().iter()) {
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

/// 0 to 1 eased in and out.
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The stir at (`px`, `py`) (half heights from the middle) at `t`:
/// MilkDrop's warp, four sines running across at their own speeds.
fn stirred(px: f32, py: f32, t: f32) -> (f32, f32) {
    let k = STIR_K;
    let x = (t * 0.333 + k * (px - py * 0.7)).sin()
        + (t * 0.753 - k * 1.4 * (px * 0.8 - py * 1.2)).cos();
    let y = (t * 0.375 - k * (px * 0.9 + py * 1.1)).cos()
        + (t * 0.825 + k * 1.4 * (px * 1.3 + py * 0.6)).sin();
    (x, y)
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
    if !(fx > -1.0 && fy > -1.0 && fx < w as f32 && fy < h as f32) {
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

/// `k` of `rgb` added at (`x`, `y`), shared between the four pixels about
/// it.
fn splat(pixels: &mut [[f32; 4]], w: usize, h: usize, x: f32, y: f32, rgb: [f32; 3], k: f32) {
    if !(x > -1.0 && y > -1.0 && x < w as f32 && y < h as f32) {
        return;
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let (x0, y0) = (x0 as isize, y0 as isize);
    for (dx, sx) in [(0, 1.0 - tx), (1, tx)] {
        for (dy, sy) in [(0, 1.0 - ty), (1, ty)] {
            let (px, py) = (x0 + dx, y0 + dy);
            let share = sx * sy;
            if px < 0 || py < 0 || px >= w as isize || py >= h as isize || share <= 0.0 {
                continue;
            }
            let p = &mut pixels[py as usize * w + px as usize];
            let k = k * share;
            p[0] += rgb[0] * k;
            p[1] += rgb[1] * k;
            p[2] += rgb[2] * k;
            p[3] += k;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn sound<'a>(wave: &'a [f32], levels: &'a [f32]) -> Option<Sound<'a>> {
        Some(Sound {
            wave,
            side: None,
            levels,
        })
    }

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
        v.feed(sound(&flat, &levels), false, 0.02, size);
        assert!(!v.active());
        // A flat wave lights a thin line within the carrier's reach of the
        // middle, and nothing else.
        v.feed(sound(&flat, &levels), true, 0.02, size);
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
            v.feed(sound(&flat, &levels), true, 0.02, size);
        }
        let rows = lit_rows(&v, 100);
        assert!(rows.len() >= 8, "{rows:?}");
        // Paused, the picture stands.
        let before = v.pixels.clone();
        v.feed(sound(&flat, &levels), false, 0.02, size);
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
            v.feed(sound(&flat, &quiet), true, 0.02, size);
        }
        assert_eq!(v.kick, 0.0);
        let mut beat = vec![0.3; BANDS];
        beat[..BANDS / 6].fill(0.9);
        v.feed(sound(&flat, &beat), true, 0.02, size);
        assert!(v.kick > 0.0);
    }

    /// A sine of `hz` cycles in the wave, at `amp`.
    fn tone(n: usize, hz: f32, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (TAU * hz * i as f32 / n as f32).sin())
            .collect()
    }

    #[test]
    fn every_scene_draws_and_fades() {
        let size = vec2(300.0, 80.0);
        let levels = vec![0.6; BANDS];
        let wave = tone(1200, 7.0, 0.5);
        let side = tone(1200, 3.0, 0.1);
        for (i, scene) in SCENES.iter().enumerate() {
            // A smaller echo would frame the picture with its edges.
            assert!(scene.echo_zoom >= 1.0, "scene {i}");
            assert!((0.0..=0.5).contains(&scene.echo), "scene {i}");
            let mut v = Vortex::new();
            v.scene = i;
            v.from = i;
            for _ in 0..50 {
                let s = Sound {
                    wave: &wave,
                    side: Some(&side),
                    levels: &levels,
                };
                v.feed(Some(s), true, 0.02, size);
            }
            let lit = v.pixels.iter().filter(|p| p[3] > 0.01).count();
            assert!(lit > 300, "scene {i} ({:?}) lit {lit}", scene.shape);
            assert!(
                v.pixels.iter().flatten().all(|c| c.is_finite()),
                "scene {i}"
            );
            for _ in 0..600 {
                v.feed(None, true, 0.02, size);
            }
            assert!(!v.active(), "scene {i} still lit");
        }
    }

    #[test]
    fn scenes_follow_one_another_on_the_beats() {
        let mut v = Vortex::new();
        let size = vec2(200.0, 60.0);
        let wave = tone(1000, 5.0, 0.4);
        let quiet = vec![0.1; BANDS];
        let mut beat = vec![0.3; BANDS];
        beat[..BANDS / 6].fill(0.9);
        let mut seen = vec![0];
        let mut changes = Vec::new();
        // Two minutes at 50 frames a second, a beat every half second.
        for frame in 0..6000 {
            let levels = if frame % 25 == 0 { &beat } else { &quiet };
            let before = v.scene;
            v.feed(sound(&wave, levels), true, 0.02, size);
            if v.scene != before {
                assert_ne!(v.scene, before);
                assert_eq!(v.from, before);
                changes.push(frame as f32 * 0.02);
                seen.push(v.scene);
            }
        }
        // Not before `SCENE_LEAST`, and on a beat after it.
        assert!(changes.len() >= 4, "{changes:?}");
        let mut last = 0.0;
        for &at in &changes {
            assert!(at - last >= SCENE_LEAST - 0.05, "{changes:?}");
            assert!(at - last <= SCENE_LEAST + 0.6, "{changes:?}");
            last = at;
        }
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() >= 3, "{seen:?}");
        // Paused, no time passes for the scene.
        let played = v.played;
        v.feed(sound(&wave, &quiet), false, 0.02, size);
        assert_eq!(v.played, played);
    }

    /// The scenes come in rounds of the others in a random order: each
    /// once in a round, never the same twice running.
    #[test]
    fn the_scenes_come_in_rounds() {
        let mut v = Vortex::new();
        let others = SCENES.len() - 1;
        let mut order = Vec::new();
        for _ in 0..3 * others {
            let before = v.scene;
            v.next_scene();
            assert_ne!(v.scene, before);
            assert_eq!(v.from, before);
            order.push(v.scene);
        }
        for round in order.chunks(others) {
            let mut seen = round.to_vec();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), others, "{order:?}");
        }
    }

    #[test]
    fn without_beats_a_scene_lasts_its_most() {
        let mut v = Vortex::new();
        let size = vec2(200.0, 60.0);
        let wave = tone(1000, 5.0, 0.4);
        let quiet = vec![0.1; BANDS];
        let mut at = None;
        for frame in 0..2000 {
            v.feed(sound(&wave, &quiet), true, 0.02, size);
            if v.scene != 0 {
                at = Some(frame as f32 * 0.02);
                break;
            }
        }
        let at = at.expect("no change");
        assert!((at - SCENE_MOST).abs() < 0.1, "{at}");
    }

    #[test]
    fn a_blend_moves_from_one_scene_to_the_next() {
        let a = SCENES[0];
        let b = SCENES[1];
        let m = a.mix(&b, 0.0);
        assert_eq!((m.zoom, m.edge, m.half_s), (a.zoom, a.edge, a.half_s));
        let m = a.mix(&b, 1.0);
        assert_eq!((m.zoom, m.edge, m.half_s), (b.zoom, b.edge, b.half_s));
        let m = a.mix(&b, 0.5);
        assert!((m.zoom - (a.zoom + b.zoom) / 2.0).abs() < 1e-6);
        assert_eq!(smooth(0.0), 0.0);
        assert_eq!(smooth(1.0), 1.0);
        assert!((smooth(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_mirrored_echo_makes_the_picture_symmetric() {
        let mut v = Vortex::new();
        let size = vec2(120.0, 40.0);
        let levels = vec![0.5; BANDS];
        let wave = tone(1000, 4.0, 0.5);
        v.feed(sound(&wave, &levels), true, 0.02, size);
        // Something only on the left.
        let (w, h) = (v.width, v.height);
        v.pixels.fill([0.0; 4]);
        v.pixels[10 * w + 5] = [1.0, 0.5, 0.2, 1.0];
        v.echoes[0] = Echo {
            share: 0.5,
            zoom: 1.0,
            mirror: Mirror::Across,
        };
        let shown = v.shown();
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (shown[y * w + x], shown[y * w + w - 1 - x]);
                for i in 0..4 {
                    assert!((a[i] - b[i]).abs() < 1e-5, "({x}, {y}): {a:?} {b:?}");
                }
            }
        }
        assert_eq!(shown[10 * w + 5][3], 0.5);
        // Half a turn: the point shows across the middle too.
        v.echoes[0].mirror = Mirror::Both;
        let shown = v.shown();
        assert_eq!(shown[(h - 1 - 10) * w + w - 1 - 5][3], 0.5);
        // Without an echo, the picture as it is.
        v.echoes[0].share = 0.0;
        assert_eq!(*v.shown(), *v.pixels);
    }
}
