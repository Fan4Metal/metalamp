//! The oscilloscope (`Visual::Scope`): `SCOPE_S` of the wave heard, as a
//! tube's trace. Each frame's sweep starts at a rising zero crossing, the
//! one whose sweep matches the last sweep best (`trigger`), so a tone
//! stands still and a chord does not jump between its crossings; the
//! sweeps before it stay a moment, fading as a phosphor's afterglow
//! (`PERSIST_S`). The beam is bright and thick where it moves slowly and
//! thin and faint where it leaps (a noise is a haze, the wave's shape
//! stands out), over a faint graticule. The height follows the loudness
//! (the RMS) slowly (`FOLLOW_S`, at most `MOST_GAIN` times), so a quiet
//! passage fills the screen and the wave between the kicks is not a flat
//! line under them (following the peaks, it was), and a peak over the
//! knee is pressed softly to the edge (`soft`), not cut. The wave is
//! drawn smoothed (`smoothed`, a Gaussian without a shift in time, half
//! its height at `SMOOTH_HZ`): heavy music's distorted guitars and cymbals
//! drew a fuzz of teeth over the riff. A sweep before the newest fades
//! the faster the more the newest departs from it (`agree`): on such music
//! they did not line up and stood as ghost traces. The user liked the
//! plain one; this keeps its look. Paused, it stands; stopped, the
//! afterglow fades.

use std::collections::VecDeque;

use egui::{Color32, Mesh, Painter, Pos2, Rect, Stroke, pos2, vec2};

use super::beat::glow;
use crate::palette;
use crate::settings::Visual;

/// The sweep's time, and how far back from the end its start is looked
/// for.
pub const SCOPE_S: f32 = 0.025;
const TRIGGER_S: f32 = 0.02;
/// The crossings tried at most, nearest the end first; every `MATCH_STEP`
/// sample of a sweep compared with the last one.
const CANDIDATES: usize = 48;
const MATCH_STEP: usize = 4;
/// A sweep fades by e every `PERSIST_S` and goes after `GONE_S`; the ones
/// before the newest are at most `AFTERGLOW` as bright (at 0.06 s and
/// 0.22 s, as bright as the newest, the user found the sweeps running
/// together).
const PERSIST_S: f32 = 0.03;
const GONE_S: f32 = 0.11;
const AFTERGLOW: f32 = 0.55;
/// The sweeps' RMS, followed over `FOLLOW_S`, stands at `FILL` of the half
/// height, the gain at most `MOST_GAIN` (silence's noise is not blown up);
/// over `KNEE` the trace is pressed softly towards the edge.
const FILL: f32 = 0.3;
const FOLLOW_S: f32 = 0.8;
const MOST_GAIN: f32 = 8.0;
const KNEE: f32 = 0.75;
/// The wave drawn smoothed to about this: its power halved here, a
/// quarter left at twice it.
const SMOOTH_HZ: f32 = 1200.0;
/// The graticule's divisions across and up.
const DIVISIONS: (usize, usize) = (10, 4);

/// A sweep: its points (x 0 to 1 across, y −1 to 1 up), its age, and how
/// much of its light is left by the sweeps after it departing from it.
struct Sweep {
    points: Vec<Pos2>,
    age: f32,
    light: f32,
}

#[derive(Default)]
pub struct Scope {
    /// Oldest first.
    sweeps: VecDeque<Sweep>,
    /// The last sweep's samples, for the next one to match.
    last: Vec<f32>,
    /// The loudness followed.
    level: f32,
}

impl Scope {
    /// Moves on by `dt` seconds with the `FETCH` samples heard now at a
    /// rate (`None`: nothing is heard, the afterglow fades; `moved` false:
    /// paused, it stands) for a display `width` points wide.
    pub fn feed(&mut self, heard: Option<(&[f32], u32)>, moved: bool, dt: f32, width: f32) {
        if heard.is_some() && !moved {
            return;
        }
        for s in &mut self.sweeps {
            s.age += dt;
        }
        while self.sweeps.front().is_some_and(|s| s.age > GONE_S) {
            self.sweeps.pop_front();
        }
        let Some((samples, rate)) = heard else {
            self.last.clear();
            return;
        };
        let span = ((rate as f32 * SCOPE_S) as usize).clamp(16, samples.len() / 2);
        let reach = (rate as f32 * TRIGGER_S) as usize;
        let smooth = smoothed(samples, rate, span + reach);
        let samples = smooth.as_slice();
        let start = self.trigger(samples, rate, span);
        let sweep = &samples[start..start + span];
        let rms = (sweep.iter().map(|x| x * x).sum::<f32>() / span as f32).sqrt();
        self.level = if self.level == 0.0 {
            rms
        } else {
            self.level + (rms - self.level) * (dt / FOLLOW_S).min(1.0)
        };
        let gain = (FILL / self.level.max(FILL / MOST_GAIN)).min(MOST_GAIN);
        // About a point a point across.
        let n = (width.round() as usize).clamp(32, span);
        let points: Vec<Pos2> = (0..n)
            .map(|j| {
                let at = j as f32 * (span - 1) as f32 / (n - 1) as f32;
                let i = (at as usize).min(span - 2);
                let t = at - i as f32;
                let y = sweep[i] * (1.0 - t) + sweep[i + 1] * t;
                pos2(j as f32 / (n - 1) as f32, soft(y * gain))
            })
            .collect();
        for old in &mut self.sweeps {
            old.light *= agree(&old.points, &points);
        }
        self.sweeps.push_back(Sweep {
            points,
            age: 0.0,
            light: 1.0,
        });
        self.last.clear();
        self.last.extend_from_slice(sweep);
    }

    /// Where the sweep starts in `samples`: among the rising zero
    /// crossings within `TRIGGER_S` of the end that leave a sweep after
    /// them, the one whose sweep is nearest the last one, else the one
    /// nearest the end, else the last sweep's place.
    fn trigger(&self, samples: &[f32], rate: u32, span: usize) -> usize {
        let latest = samples.len() - span;
        let reach = ((rate as f32 * TRIGGER_S) as usize).min(latest);
        let crossings: Vec<usize> = (latest - reach + 1..=latest)
            .rev()
            .filter(|&i| samples[i - 1] < 0.0 && samples[i] >= 0.0)
            .take(CANDIDATES)
            .collect();
        match crossings.as_slice() {
            [] => latest,
            [only] => *only,
            _ if self.last.len() != span => crossings[0],
            _ => *crossings
                .iter()
                .min_by(|&&a, &&b| {
                    let off = |c: usize| -> f32 {
                        (0..span)
                            .step_by(MATCH_STEP)
                            .map(|k| (samples[c + k] - self.last[k]).powi(2))
                            .sum()
                    };
                    off(a).total_cmp(&off(b))
                })
                .unwrap(),
        }
    }

    pub fn active(&self) -> bool {
        !self.sweeps.is_empty()
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let cy = rect.center().y;
        let half = rect.height() / 2.0 - 2.0;
        let colours = palette::visual_for(Visual::Scope);
        let dark = colours.is_dark();
        // The graticule: faint divisions, the middle line, its ticks.
        let line = palette::line();
        let faint = Stroke::new(1.0, line.gamma_multiply(0.45));
        for k in 1..DIVISIONS.0 {
            let x = rect.left() + rect.width() * k as f32 / DIVISIONS.0 as f32;
            painter.vline(x, rect.y_range(), faint);
        }
        for k in 1..DIVISIONS.1 {
            let y = rect.top() + rect.height() * k as f32 / DIVISIONS.1 as f32;
            if (y - cy).abs() > 1.0 {
                painter.hline(rect.x_range(), y, faint);
            }
        }
        painter.hline(rect.x_range(), cy, Stroke::new(1.0, line));
        let ticks = DIVISIONS.0 * 5;
        for k in 1..ticks {
            let x = rect.left() + rect.width() * k as f32 / ticks as f32;
            painter.vline(x, cy - 1.5..=cy + 1.5, Stroke::new(1.0, line));
        }
        // Each point coloured by its place (a rainbow along the line) or,
        // when the colours tell the level, by how far it swings.
        let colour_at = |p: Pos2, top: bool| {
            let place = p.x;
            let level = if colours.by_level() {
                p.y.abs().sqrt()
            } else if top {
                1.0
            } else {
                0.0
            };
            colours.at(level, place)
        };
        let n = self.sweeps.len();
        let mut mesh = Mesh::default();
        for (k, sweep) in self.sweeps.iter().enumerate() {
            let newest = k + 1 == n;
            let fade = (-sweep.age / PERSIST_S).exp()
                * if newest { 1.0 } else { AFTERGLOW * sweep.light };
            if fade < 0.01 {
                continue;
            }
            let points: Vec<Pos2> = sweep
                .points
                .iter()
                .map(|p| pos2(rect.left() + p.x * rect.width(), cy - p.y * half))
                .collect();
            // The beam's speed: points of height a point across, from
            // the neighbours on both sides.
            let step = rect.width() / (points.len() - 1).max(1) as f32;
            let light: Vec<f32> = (0..points.len())
                .map(|i| {
                    let a = points[i.saturating_sub(1)];
                    let b = points[(i + 1).min(points.len() - 1)];
                    let speed = (b.y - a.y).abs() / (b.x - a.x).max(step);
                    (1.0 / (1.0 + 0.6 * speed)).max(0.08)
                })
                .collect();
            if dark && (newest || fade > 0.4) {
                let widths: Vec<f32> = light.iter().map(|l| 2.5 + 4.0 * l).collect();
                let tints: Vec<Color32> = sweep
                    .points
                    .iter()
                    .zip(&light)
                    .map(|(p, l)| glow(colour_at(*p, false), 0.22 * l * fade, dark))
                    .collect();
                beam(&mut mesh, &points, &widths, &tints);
            }
            let widths: Vec<f32> = light.iter().map(|l| 0.8 + 1.2 * l).collect();
            let tints: Vec<Color32> = sweep
                .points
                .iter()
                .zip(&light)
                .map(|(p, l)| glow(colour_at(*p, true), (0.15 + 0.85 * l) * fade, dark))
                .collect();
            beam(&mut mesh, &points, &widths, &tints);
        }
        painter.add(mesh);
    }
}

/// The last `len` of `samples` (or all of them) smoothed by a Gaussian of
/// the width that halves the power at `SMOOTH_HZ`, the same both ways, so
/// nothing moves in time; at the ends, of the samples there are.
fn smoothed(samples: &[f32], rate: u32, len: usize) -> Vec<f32> {
    // A Gaussian's response at f: exp(−(2πfσ)²/2); half the power where
    // (2πfσ)² = ln 2.
    let sigma = std::f32::consts::LN_2.sqrt() / (std::f32::consts::TAU * SMOOTH_HZ)
        * rate as f32;
    let radius = (3.0 * sigma).ceil() as usize;
    let weights: Vec<f32> = (0..=radius)
        .map(|k| (-0.5 * (k as f32 / sigma).powi(2)).exp())
        .collect();
    // The samples before the part kept are read too, so its start is
    // smoothed as its middle is.
    let len = len.min(samples.len());
    let from = samples.len() - len;
    (from..samples.len())
        .map(|i| {
            let (mut sum, mut weight) = (samples[i], 1.0);
            for (k, w) in weights.iter().enumerate().skip(1) {
                for j in [i.checked_sub(k), Some(i + k).filter(|&j| j < samples.len())]
                    .into_iter()
                    .flatten()
                {
                    sum += samples[j] * w;
                    weight += w;
                }
            }
            sum / weight
        })
        .collect()
}

/// How much of an older sweep's light is kept as `new` comes: 1 where
/// they are alike, less the further apart they are against `new`'s own
/// swing, nothing when as far apart as the swing.
fn agree(old: &[Pos2], new: &[Pos2]) -> f32 {
    if old.len() != new.len() || new.is_empty() {
        return 0.0;
    }
    let apart: f32 = old.iter().zip(new).map(|(a, b)| (a.y - b.y).abs()).sum();
    let swing: f32 = new.iter().map(|p| p.y.abs()).sum();
    (1.0 - apart / swing.max(1e-3)).clamp(0.0, 1.0)
}

/// `y` (1: the edge) as is up to `KNEE`, over it pressed softly towards
/// the edge.
fn soft(y: f32) -> f32 {
    let a = y.abs();
    let pressed = if a <= KNEE {
        a
    } else {
        KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh()
    };
    pressed.copysign(y)
}

/// A line through `points`, each `widths` wide in `colours`.
fn beam(mesh: &mut Mesh, points: &[Pos2], widths: &[f32], colours: &[Color32]) {
    let n = points.len();
    if n < 2 {
        return;
    }
    let base = mesh.vertices.len() as u32;
    for i in 0..n {
        let way = points[(i + 1).min(n - 1)] - points[i.saturating_sub(1)];
        let way = if way.length() > 1e-4 {
            way.normalized()
        } else {
            vec2(1.0, 0.0)
        };
        let normal = way.rot90() * (widths[i] / 2.0);
        mesh.colored_vertex(points[i] + normal, colours[i]);
        mesh.colored_vertex(points[i] - normal, colours[i]);
    }
    for i in 0..n as u32 - 1 {
        let a = base + 2 * i;
        mesh.add_triangle(a, a + 1, a + 3);
        mesh.add_triangle(a, a + 3, a + 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn wave(n: usize, offset: usize, f: impl Fn(f32) -> f32) -> Vec<f32> {
        (offset..offset + n).map(|i| f(i as f32 / 48_000.0)).collect()
    }

    #[test]
    fn a_sweep_starts_at_a_rising_zero_crossing() {
        let mut s = Scope::default();
        // 1 kHz: a period of 48 frames; started off its zero.
        let tone = wave(8192, 7, |t| 0.5 * (TAU * 1000.0 * t).sin());
        s.feed(Some((&tone, 48_000)), true, 0.02, 300.0);
        let p = &s.sweeps.back().unwrap().points;
        assert!(p[0].y >= 0.0 && p[0].y < 0.15, "{}", p[0].y);
        assert!(p[1].y > p[0].y);
    }

    #[test]
    fn a_tone_stands_still() {
        let mut s = Scope::default();
        // A chord of 220 and 330 Hz: several rising crossings a period.
        let chord = |t: f32| 0.3 * (TAU * 220.0 * t).sin() + 0.25 * (TAU * 330.0 * t).sin();
        let mut first = None;
        for frame in 0..20 {
            let samples = wave(8192, frame * 937, chord);
            s.feed(Some((&samples, 48_000)), true, 0.02, 300.0);
            let points = &s.sweeps.back().unwrap().points;
            match &first {
                None => first = Some(points.clone()),
                // After the gain settles, the sweep is where it was.
                Some(f) if frame > 10 => {
                    let off: f32 = f
                        .iter()
                        .zip(points)
                        .map(|(a, b)| (a.y - b.y).abs())
                        .sum::<f32>()
                        / f.len() as f32;
                    assert!(off < 0.05, "frame {frame}: {off}");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_quiet_passage_fills_the_screen_and_a_loud_one_is_pressed() {
        let mut s = Scope::default();
        let quiet = wave(8192, 0, |t| 0.05 * (TAU * 440.0 * t).sin());
        for _ in 0..200 {
            s.feed(Some((&quiet, 48_000)), true, 0.02, 300.0);
        }
        let top = |s: &Scope| {
            s.sweeps
                .back()
                .unwrap()
                .points
                .iter()
                .fold(0.0f32, |m, p| m.max(p.y.abs()))
        };
        assert!(top(&s) > 0.25, "{}", top(&s));
        // Suddenly loud: pressed under the edge at once.
        let loud = wave(8192, 0, |t| 0.9 * (TAU * 440.0 * t).sin());
        s.feed(Some((&loud, 48_000)), true, 0.02, 300.0);
        assert!(top(&s) <= 1.0);
        for _ in 0..200 {
            s.feed(Some((&loud, 48_000)), true, 0.02, 300.0);
        }
        // A sine's RMS at FILL: its peaks at √2 of it.
        assert!((top(&s) - FILL * 2f32.sqrt()).abs() < 0.05, "{}", top(&s));
    }

    #[test]
    fn the_afterglow_fades_when_stopped_and_stands_when_paused() {
        let mut s = Scope::default();
        let tone = wave(8192, 0, |t| 0.5 * (TAU * 440.0 * t).sin());
        for _ in 0..5 {
            s.feed(Some((&tone, 48_000)), true, 0.02, 300.0);
        }
        let kept = s.sweeps.len();
        assert!(kept > 1);
        s.feed(Some((&tone, 48_000)), false, 0.02, 300.0);
        assert_eq!(s.sweeps.len(), kept);
        for _ in 0..20 {
            s.feed(None, true, 0.02, 300.0);
        }
        assert!(!s.active());
    }

    /// Treble over a bass note (a distorted guitar's teeth): drawn about
    /// as the note alone.
    #[test]
    fn the_treble_is_smoothed_away() {
        let note = |t: f32| 0.4 * (TAU * 110.0 * t).sin();
        let fuzzy = |t: f32| note(t) + 0.15 * (TAU * 3700.0 * t).sin();
        let drawn = |f: &dyn Fn(f32) -> f32| {
            let mut s = Scope::default();
            for frame in 0..30 {
                let samples = wave(8192, frame * 960, f);
                s.feed(Some((&samples, 48_000)), true, 0.02, 300.0);
            }
            s.sweeps.back().unwrap().points.clone()
        };
        let (clean, fuzz) = (drawn(&note), drawn(&fuzzy));
        let off = clean
            .iter()
            .zip(&fuzz)
            .map(|(a, b)| (a.y - b.y).abs())
            .fold(0.0f32, f32::max);
        assert!(off < 0.04, "{off}");
        // Smoothing moves nothing in time: a crossing stays where it is.
        let tone = wave(4096, 0, |t| (TAU * 200.0 * t).sin());
        let smooth = smoothed(&tone, 48_000, 4096);
        let mid = 2040..2100;
        let crossing = |v: &[f32]| mid.clone().find(|&i| v[i - 1] < 0.0 && v[i] >= 0.0);
        assert_eq!(crossing(&tone), crossing(&smooth));
    }

    #[test]
    fn sweeps_that_depart_from_the_newest_fade_at_once() {
        let mut s = Scope::default();
        let tone = wave(8192, 0, |t| 0.5 * (TAU * 440.0 * t).sin());
        s.feed(Some((&tone, 48_000)), true, 0.02, 300.0);
        s.feed(Some((&tone, 48_000)), true, 0.02, 300.0);
        // The same wave again: the one before keeps its light.
        assert!(s.sweeps[0].light > 0.9, "{}", s.sweeps[0].light);
        // Another one: it goes.
        let other = wave(8192, 0, |t| 0.5 * (TAU * 97.0 * t).sin().signum());
        s.feed(Some((&other, 48_000)), true, 0.02, 300.0);
        assert!(s.sweeps[1].light < 0.3, "{}", s.sweeps[1].light);
    }

    #[test]
    fn soft_presses_over_the_knee() {
        assert_eq!(soft(0.5), 0.5);
        assert_eq!(soft(-0.5), -0.5);
        assert!(soft(3.0) <= 1.0 && soft(3.0) > 0.95);
        assert!(soft(-0.9) > -1.0 && soft(-0.9) < -KNEE);
    }
}
