//! The player bar's display: one of the visualizations (`Visual`, a
//! setting), chosen by the right button on it, the next one by a click.
//! All of them are fed the samples heard at this moment (`audio::vis::Tap`,
//! mono): the spectrum analyzer (`spectrum`) painted as bars, as LED
//! segments, as a smooth curve or mirrored about the middle line; ridges
//! (the analyzer's levels across the display every
//! 50 ms, the newest line in front at the bottom and the older ones
//! behind it, higher, narrower and fainter, each hiding what is behind
//! it, as the cover of "Unknown Pleasures"); a waveform rolling from the
//! right as DJ software draws a track (`waveform`: the bass, the middle
//! and the treble as layers of their own colours), an oscilloscope (`scope`:
//! 25 ms of the wave as a tube's trace with an afterglow, held still on a
//! tone), a spectrogram (the analyzer's bands as a column of colours
//! every 20 ms, the loud ones bright, rolling left out of the live
//! spectrum drawn as bars on the right; brighter on the beats), three
//! scenes: a
//! synthwave flight (`synthwave`) and lightning striking on the beats
//! (`lightning`) of the analyzer's levels, a vortex of the waves heard
//! (`vortex`, a picture with feedback as MilkDrop's), strata (`strata`:
//! a string per band held at a nut on the left, the levels running right
//! along them as vibrating swells, plucked by the beats), and
//! the notes (`notes`): the twelve pitch classes as bars (a chromagram)
//! or a piano's keys lit by their notes with a bar over each. The
//! waveform, the ridges, the strata and the spectrogram count the frames
//! heard since the frame before (the tap's `end`), so they roll with the
//! sound, whatever the frame rate; paused, they stand; stopped, they roll
//! silence until they are empty. None ("No visualization"), the display shows the app icon
//! and the name, and the app repaints only as the time needs; switched
//! off in the settings, nothing is painted (`player_bar::visual_display`).

use std::collections::VecDeque;
use std::time::Instant;

use egui::epaint::PathStroke;
use egui::{
    Align2, Color32, ColorImage, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, TextureOptions,
    Vec2, pos2,
};

use crate::audio::vis::Heard;
use crate::palette;
use crate::settings::Visual;
use crate::ui::aurora::{self, Aurora};
use crate::ui::beat::Beat;
use crate::ui::cloth::{self, Cloth};
use crate::ui::fire::{self, Fire};
use crate::ui::goniometer::Goniometer;
use crate::ui::harp::Harp;
use crate::ui::lightning::{self, Lightning};
use crate::ui::notes::{self, Notes};
use crate::ui::ribbons::{self, Ribbons};
use crate::ui::ripples::{self, Ripples};
use crate::ui::scope::Scope;
use crate::ui::fireworks::{self, Fireworks};
use crate::ui::spectrum::{self, Analyzer};
use crate::ui::strata::Strata;
use crate::ui::strings::Strings;
use crate::ui::synthwave::{self, Synthwave};
use crate::ui::vortex::{self, Vortex};
use crate::ui::waveform::{self, Waveform};

/// The frames asked of the tap each frame: the analyzer's window, and
/// what is heard between two frames at 50 frames a second up to 192 kHz.
pub const FETCH: usize = 8192;
/// The time of a column of the spectrogram, and its width in points.
const COLUMN_S: f32 = 0.02;
const GRAM_PITCH: f32 = 2.0;
/// The spectrogram's colours: a level's place among the ground, the
/// analyzer's foot, its top and its peaks.
const GRAM_STOPS: [f32; 4] = [0.0, 0.4, 0.75, 1.0];
const GRAM_CONTRAST: f32 = 1.8;
/// The spectrogram's live spectrum on the right (the bands now, as bars
/// from the history's edge, which it flows out of): its share of the
/// width (within `GRAM_EMITTER`), rows a bar (one of them a gap), the
/// bars falling the whole way in `GRAM_FALL_S`, their peaks held
/// `GRAM_HOLD_S` and falling in `GRAM_PEAK_FALL_S`.
const GRAM_EMITTER_SHARE: f32 = 0.09;
const GRAM_EMITTER: [f32; 2] = [20.0, 64.0];
const GRAM_BAR: usize = 3;
const GRAM_FALL_S: f32 = 0.5;
const GRAM_HOLD_S: f32 = 0.4;
const GRAM_PEAK_FALL_S: f32 = 0.9;
/// A beat (`beat::Beat`) brightens the columns written while its kick
/// lasts by this share, and lights the live spectrum.
const GRAM_KICK: f32 = 0.25;
/// The share of the history's width on the left over which it fades.
const GRAM_FADE: f32 = 0.2;
/// The ridges: a line every `RIDGE_S`, points across every `RIDGE_PITCH`,
/// a line every `RIDGE_SPACING` points of height; the front line's height
/// as a share of the display's, the levels raised to `RIDGE_POWER` (music
/// fills the upper half of the analyzer's range), the share of the width
/// at each end that slopes down to the base line, and how much narrower,
/// lower and fainter the line furthest back is.
const RIDGE_S: f32 = 0.05;
const RIDGE_PITCH: f32 = 4.0;
const RIDGE_SPACING: f32 = 4.5;
const RIDGE_HEIGHT: f32 = 0.5;
const RIDGE_POWER: f32 = 2.0;
const RIDGE_EDGE: f32 = 0.12;
const RIDGE_NARROWER: f32 = 0.3;
const RIDGE_LOWER: f32 = 0.35;
const RIDGE_FAINTER: f32 = 0.75;
/// The piano's white keys are at least this wide; the keys take this
/// share of the height, under the bars; a black key's width and length
/// as shares of a white one's; a key's level from which it starts to
/// light, and over which share of the scale it lights fully.
const PIANO_WHITE: f32 = 9.0;
const PIANO_KEYS: f32 = 0.5;
const PIANO_BLACK_WIDTH: f32 = 0.6;
const PIANO_BLACK_LENGTH: f32 = 0.62;
const PIANO_LIGHT_FROM: f32 = 0.35;
const PIANO_LIGHT_SPAN: f32 = 0.4;

pub struct Visualizer {
    analyzer: Analyzer,
    notes: Notes,
    lightning: Lightning,
    synthwave: Synthwave,
    vortex: Vortex,
    fire: Fire,
    fireworks: Fireworks,
    aurora: Aurora,
    cloth: Cloth,
    ripples: Ripples,
    ribbons: Ribbons,
    goniometer: Goniometer,
    strings: Strings,
    harp: Harp,
    /// The ridges' lines, oldest first: the levels across, the lowest
    /// band first.
    ridges: VecDeque<Vec<f32>>,
    /// The ridges' line heard now (the front one, not kept yet), and the
    /// share of the way the lines have moved on to their next places.
    ridge_live: Vec<f32>,
    ridge_moved: f32,
    strata: Strata,
    /// Columns (or lines) owed to the history shown (fractions carried
    /// on).
    owed: f32,
    waveform: Waveform,
    /// The spectrogram's columns, oldest first: the bands' levels, the
    /// lowest band first.
    gram: VecDeque<Vec<f32>>,
    gram_texture: Option<egui::TextureHandle>,
    /// What the texture shows: changed columns, colours or width make it
    /// anew.
    gram_drawn: Option<(u64, palette::VisualPalette, Color32, usize)>,
    gram_count: u64,
    /// The spectrogram's beat, its live spectrum and the live peaks (level,
    /// time left held).
    gram_beat: Beat,
    gram_live: Vec<f32>,
    gram_peaks: Vec<(f32, f32)>,
    scope: Scope,
    /// The tap's end at the frame before (`None`: nothing heard then).
    end: Option<u64>,
    /// Silence owed while nothing is heard, in seconds.
    idle: f32,
    last: Instant,
    shown: Visual,
    /// The app icon shown while off.
    icon: Option<egui::TextureHandle>,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Visualizer {
    pub fn new() -> Visualizer {
        Visualizer {
            analyzer: Analyzer::new(),
            notes: Notes::new(),
            lightning: Lightning::new(),
            synthwave: Synthwave::new(),
            vortex: Vortex::new(),
            fire: Fire::new(),
            fireworks: Fireworks::new(),
            aurora: Aurora::new(),
            cloth: Cloth::new(),
            ripples: Ripples::new(),
            ribbons: Ribbons::new(),
            goniometer: Goniometer::new(),
            strings: Strings::new(),
            harp: Harp::new(),
            ridges: VecDeque::new(),
            ridge_live: Vec::new(),
            ridge_moved: 0.0,
            strata: Strata::new(),
            owed: 0.0,
            waveform: Waveform::default(),
            gram: VecDeque::new(),
            gram_texture: None,
            gram_drawn: None,
            gram_count: 0,
            gram_beat: Beat::default(),
            gram_live: Vec::new(),
            gram_peaks: Vec::new(),
            scope: Scope::default(),
            end: None,
            idle: 0.0,
            last: Instant::now(),
            shown: Visual::default(),
            icon: None,
        }
    }

    /// Moves `visual` on with the `FETCH` frames heard now (`None`:
    /// nothing is heard) and, for the goniometer, their side, for a
    /// display of `size` (`None`: none shown).
    pub fn feed(
        &mut self,
        visual: Visual,
        heard: Option<(&[f32], Heard)>,
        side: Option<&[f32]>,
        size: Option<Vec2>,
    ) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        if visual != self.shown {
            // Another one: the last one's state goes.
            *self = Visualizer {
                gram_texture: self.gram_texture.take(),
                icon: self.icon.take(),
                shown: visual,
                last: now,
                ..Visualizer::new()
            };
        }
        let heard = heard.filter(|(s, h)| s.len() >= spectrum::N && h.rate > 0);
        // The frames new since the frame before.
        let new = match (heard, self.end) {
            (Some((s, h)), Some(end)) if h.end >= end => ((h.end - end) as usize).min(s.len()),
            _ => 0,
        };
        self.end = heard.map(|(_, h)| h.end);
        let Some(size) = size else {
            return;
        };
        let rate = heard.map_or(0, |(_, h)| h.rate);
        match visual {
            Visual::Spectrum | Visual::Led | Visual::Curve | Visual::Mirror => {
                let window = heard.map(|(s, _)| &s[s.len() - spectrum::N..]);
                let bands = match visual {
                    Visual::Led => Analyzer::bands_for(size.x, spectrum::LED_PITCH),
                    Visual::Curve => Analyzer::bands_for(size.x, spectrum::CURVE_PITCH),
                    _ => Analyzer::bands_for(size.x, spectrum::BAR_PITCH),
                };
                self.analyzer.feed(window, rate, bands);
            }
            Visual::Lightning => {
                let levels = heard.map(|(s, h)| {
                    self.analyzer
                        .levels(&s[s.len() - spectrum::N..], h.rate, lightning::BANDS)
                });
                self.lightning.feed(levels.as_deref(), dt, size);
            }
            Visual::Synthwave => {
                let row = heard.map(|(s, h)| {
                    let levels =
                        self.analyzer
                            .levels(&s[s.len() - spectrum::N..], h.rate, synthwave::BANDS);
                    (smoothed(&levels), new as f32 / h.rate as f32)
                });
                self.synthwave.feed(row, dt);
            }
            Visual::Vortex => {
                let heard = heard.map(|(s, h)| {
                    let levels =
                        self.analyzer
                            .levels(&s[s.len() - spectrum::N..], h.rate, vortex::BANDS);
                    let span = ((h.rate as f32 * vortex::WAVE_S) as usize).clamp(16, s.len());
                    (&s[s.len() - span..], levels)
                });
                let heard = heard
                    .as_ref()
                    .map(|(wave, levels)| (*wave, levels.as_slice()));
                self.vortex.feed(heard, new > 0, dt, size);
            }
            Visual::Chroma | Visual::Piano | Visual::Strings => {
                let window = heard
                    .filter(|(s, _)| s.len() >= notes::N)
                    .map(|(s, _)| &s[s.len() - notes::N..]);
                self.notes.feed(window, rate);
                if visual == Visual::Strings {
                    self.strings
                        .feed(&self.notes.keys, heard.is_none() || new > 0, dt, size.x);
                }
            }
            Visual::Fire | Visual::Fireworks | Visual::Aurora => {
                let bands = match visual {
                    Visual::Fire => fire::BANDS,
                    Visual::Fireworks => fireworks::BANDS,
                    _ => aurora::BANDS,
                };
                let levels = heard.map(|(s, h)| {
                    self.analyzer
                        .levels(&s[s.len() - spectrum::N..], h.rate, bands)
                });
                let levels = levels.as_deref();
                let moved = new > 0;
                match visual {
                    Visual::Fire => self.fire.feed(levels, moved, dt, size),
                    Visual::Fireworks => self.fireworks.feed(levels, moved, dt, size),
                    _ => self.aurora.feed(levels.map(smoothed).as_deref(), moved, dt),
                }
            }
            Visual::Goniometer => {
                let both = heard
                    .zip(side)
                    .filter(|((s, _), side)| s.len() == side.len());
                let fresh = both.map(|((s, _), side)| {
                    let n = new.min(s.len());
                    (&s[s.len() - n..], &side[side.len() - n..])
                });
                self.goniometer.feed(fresh, new > 0, dt, size);
            }
            Visual::Ridges => {
                let cap = ridge_lines(size.y);
                let points = ((size.x / RIDGE_PITCH) as usize).clamp(24, 160);
                if self.ridges.front().is_some_and(|l| l.len() != points) {
                    self.ridges.clear();
                }
                let shown = self.ridges.iter().any(|l| l.iter().any(|&v| v > 0.0));
                // The line heard now, every frame: the front one, coming
                // up into its place until it is kept.
                let line = match heard {
                    Some((s, h)) => smoothed(&self.analyzer.levels(
                        &s[s.len() - spectrum::N..],
                        h.rate,
                        points,
                    )),
                    None => vec![0.0; points],
                };
                match self.due(rate, new, RIDGE_S, dt, shown) {
                    Some(due) => {
                        for _ in 0..due.min(cap) {
                            self.ridges.push_back(line.clone());
                        }
                        while self.ridges.len() > cap {
                            self.ridges.pop_front();
                        }
                        // How far the lines have moved on to their next
                        // places.
                        self.ridge_moved = if rate > 0 { self.owed } else { self.idle };
                        self.ridge_live = line;
                    }
                    None => {
                        self.ridges.clear();
                        self.ridge_live.clear();
                        self.ridge_moved = 0.0;
                    }
                }
            }
            Visual::Harp => {
                let bands = Harp::strings(size.x);
                let levels = heard.map(|(s, h)| {
                    let levels = self
                        .analyzer
                        .levels(&s[s.len() - spectrum::N..], h.rate, bands);
                    (levels, new as f32 / h.rate as f32)
                });
                let heard = levels.as_ref().map(|(l, seconds)| (l.as_slice(), *seconds));
                self.harp.feed(heard, dt, size.x);
            }
            Visual::Ribbons => {
                let levels = heard.map(|(s, h)| {
                    let levels =
                        self.analyzer
                            .levels(&s[s.len() - spectrum::N..], h.rate, ribbons::BANDS);
                    (levels, new as f32 / h.rate as f32)
                });
                let heard = levels.as_ref().map(|(l, seconds)| (l.as_slice(), *seconds));
                self.ribbons.feed(heard, dt, size.x);
            }
            Visual::Ripples => {
                let levels = heard.map(|(s, h)| {
                    let levels =
                        self.analyzer
                            .levels(&s[s.len() - spectrum::N..], h.rate, ripples::BANDS);
                    (levels, new as f32 / h.rate as f32)
                });
                let heard = levels.as_ref().map(|(l, seconds)| (l.as_slice(), *seconds));
                self.ripples.feed(heard, dt);
            }
            Visual::Cloth => {
                let levels = heard.map(|(s, h)| {
                    let levels =
                        self.analyzer
                            .levels(&s[s.len() - spectrum::N..], h.rate, cloth::BANDS);
                    (smoothed(&levels), new as f32 / h.rate as f32)
                });
                let heard = levels.as_ref().map(|(l, seconds)| (l.as_slice(), *seconds));
                self.cloth.feed(heard, dt);
            }
            Visual::Strata => {
                let bands = Strata::lines(size.y);
                let levels = heard.map(|(s, h)| {
                    let levels = self
                        .analyzer
                        .levels(&s[s.len() - spectrum::N..], h.rate, bands);
                    (levels, new as f32 / h.rate as f32)
                });
                let heard = levels.as_ref().map(|(l, seconds)| (l.as_slice(), *seconds));
                self.strata.feed(heard, dt, size);
            }
            Visual::Waveform => {
                let cap = columns(size.x, waveform::PITCH);
                let fresh = heard.map(|(s, h)| (&s[s.len() - new..], h.rate));
                self.waveform.feed(fresh, dt, cap);
            }
            Visual::Spectrogram => {
                let cap = columns(size.x - gram_emitter(size.x), GRAM_PITCH);
                let rows = (size.y.round() as usize).clamp(16, 256);
                if self.gram.front().is_some_and(|c| c.len() != rows) {
                    self.gram.clear();
                }
                // The live spectrum falls, then takes the column heard.
                self.gram_live.resize(rows, 0.0);
                self.gram_peaks.resize(rows, (0.0, 0.0));
                for l in &mut self.gram_live {
                    *l = (*l - dt / GRAM_FALL_S).max(0.0);
                }
                let shown = self.gram.iter().any(|c| c.iter().any(|&l| l > 0.0));
                match self.due(rate, new, COLUMN_S, dt, shown) {
                    Some(due) => {
                        if due > 0 {
                            // One column of the window heard now, as many
                            // times as are due (more than one only after a
                            // slow frame).
                            let column = match heard {
                                Some((s, h)) => {
                                    self.analyzer
                                        .levels(&s[s.len() - spectrum::N..], h.rate, rows)
                                }
                                None => vec![0.0; rows],
                            };
                            let heard_now = heard.is_some().then_some(column.as_slice());
                            self.gram_beat.feed(heard_now, due as f32 * COLUMN_S);
                            for (l, &c) in self.gram_live.iter_mut().zip(&column) {
                                *l = l.max(c);
                            }
                            // Written brighter while a kick lasts.
                            let boost = 1.0 + GRAM_KICK * self.gram_beat.kick;
                            let lit: Vec<f32> =
                                column.iter().map(|&l| (l * boost).min(1.0)).collect();
                            for _ in 0..due.min(cap) {
                                self.gram.push_back(lit.clone());
                                self.gram_count += 1;
                            }
                        }
                        while self.gram.len() > cap {
                            self.gram.pop_front();
                        }
                    }
                    None => self.gram.clear(),
                }
                for (p, &l) in self.gram_peaks.iter_mut().zip(&self.gram_live) {
                    if l >= p.0 {
                        *p = (l, GRAM_HOLD_S);
                    } else if p.1 > 0.0 {
                        p.1 -= dt;
                    } else {
                        p.0 = (p.0 - dt / GRAM_PEAK_FALL_S).max(0.0);
                    }
                }
            }
            Visual::Off => {}
            Visual::Scope => {
                let heard = heard.map(|(s, h)| (s, h.rate));
                self.scope.feed(heard, new > 0, dt, size.x);
            }
        }
    }

    /// The columns (or lines) of `period` seconds due to a history now:
    /// those of the `new` frames heard at `rate`, the fractions carried
    /// on; nothing heard, silent ones by the clock while something still
    /// `shown`, else `None`: the history goes.
    fn due(&mut self, rate: u32, new: usize, period: f32, dt: f32, shown: bool) -> Option<usize> {
        if rate > 0 {
            self.idle = 0.0;
            self.owed += new as f32 / (rate as f32 * period);
            let due = self.owed as usize;
            self.owed -= due as f32;
            Some(due)
        } else if shown {
            Some(self.idle_lines(dt, period))
        } else {
            None
        }
    }

    /// Silent columns (or lines) of `period` seconds owed for `dt` seconds
    /// of nothing heard.
    fn idle_lines(&mut self, dt: f32, period: f32) -> usize {
        self.idle += dt / period;
        let due = self.idle as usize;
        self.idle -= due as f32;
        due
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        match self.shown {
            Visual::Spectrum | Visual::Led | Visual::Curve | Visual::Mirror => {
                self.analyzer.active()
            }
            Visual::Lightning => self.lightning.active(),
            Visual::Synthwave => self.synthwave.active(),
            Visual::Vortex => self.vortex.active(),
            Visual::Ridges => !self.ridges.is_empty(),
            Visual::Strata => self.strata.active(),
            Visual::Chroma | Visual::Piano => self.notes.active(),
            Visual::Strings => self.strings.active(),
            Visual::Harp => self.harp.active(),
            Visual::Fire => self.fire.active(),
            Visual::Fireworks => self.fireworks.active(),
            Visual::Aurora => self.aurora.active(),
            Visual::Cloth => self.cloth.active(),
            Visual::Ripples => self.ripples.active(),
            Visual::Ribbons => self.ribbons.active(),
            Visual::Goniometer => self.goniometer.active(),
            Visual::Waveform => self.waveform.active(),
            Visual::Spectrogram => {
                !self.gram.is_empty() || self.gram_live.iter().any(|&l| l > 0.0)
            }
            Visual::Scope => self.scope.active(),
            Visual::Off => false,
        }
    }

    /// Paints the visualization in `rect`.
    pub fn paint(&mut self, painter: &Painter, rect: Rect) {
        match self.shown {
            Visual::Spectrum => self.analyzer.paint(painter, rect),
            Visual::Led => self.analyzer.paint_led(painter, rect),
            Visual::Curve => self.analyzer.paint_curve(painter, rect),
            Visual::Mirror => self.analyzer.paint_mirror(painter, rect),
            Visual::Lightning => self.lightning.paint(painter, rect),
            Visual::Synthwave => self.synthwave.paint(painter, rect),
            Visual::Vortex => self.vortex.paint(painter, rect),
            Visual::Fire => self.fire.paint(painter, rect),
            Visual::Fireworks => self.fireworks.paint(painter, rect),
            Visual::Aurora => self.aurora.paint(painter, rect),
            Visual::Cloth => self.cloth.paint(painter, rect),
            Visual::Ripples => self.ripples.paint(painter, rect),
            Visual::Ribbons => self.ribbons.paint(painter, rect),
            Visual::Goniometer => self.goniometer.paint(painter, rect),
            Visual::Strings => self.strings.paint(painter, rect),
            Visual::Harp => self.harp.paint(painter, rect),
            Visual::Ridges => self.paint_ridges(painter, rect),
            Visual::Strata => self.strata.paint(painter, rect),
            Visual::Chroma => self.paint_chroma(painter, rect),
            Visual::Piano => self.paint_piano(painter, rect),
            Visual::Waveform => self.waveform.paint(painter, rect),
            Visual::Spectrogram => self.paint_gram(painter, rect),
            Visual::Scope => self.scope.paint(painter, rect),
            Visual::Off => self.paint_logo(painter, rect),
        }
    }

    /// The ridges, the oldest line first: each fills the room down to its
    /// base line with the display's ground, hiding the lines behind it,
    /// then draws its line.
    fn paint_ridges(&self, painter: &Painter, rect: Rect) {
        let Some(points) = self.ridges.back().map(Vec::len) else {
            return;
        };
        if points < 2 {
            return;
        }
        let lines = ridge_lines(rect.height());
        let n = self.ridges.len();
        let height = rect.height() * RIDGE_HEIGHT;
        let spacing = (rect.height() - height - 2.0) / (lines - 1).max(1) as f32;
        let ground = palette::list_bg();
        let colours = palette::visual_for(Visual::Ridges);
        let cx = rect.center().x;
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        // Each line a share of the way on to its next place, so they glide
        // up between the lines kept; in front the line heard now, coming
        // up from under the bottom into the front place.
        let moved = self.ridge_moved;
        let live = Some(&self.ridge_live).filter(|l| l.len() == points);
        let all = self
            .ridges
            .iter()
            .enumerate()
            .map(|(i, line)| ((n - 1 - i) as f32 + moved, line))
            .chain(live.map(|line| (moved - 1.0, line)));
        for (age, line) in all {
            let depth = (age / (lines - 1).max(1) as f32).clamp(0.0, 1.0);
            let base = rect.bottom() - 1.0 - spacing * age;
            let width = rect.width() * (1.0 - RIDGE_NARROWER * depth);
            let tall = height * (1.0 - RIDGE_LOWER * depth);
            // The oldest fades as it goes.
            let leaving = (lines as f32 - age).clamp(0.0, 1.0);
            let shown: Vec<Pos2> = line
                .iter()
                .enumerate()
                .map(|(j, &level)| {
                    let u = j as f32 / (points - 1) as f32;
                    let edge = (u.min(1.0 - u) / RIDGE_EDGE).min(1.0);
                    let taper = edge * edge * (3.0 - 2.0 * edge);
                    let h = level.clamp(0.0, 1.0).powf(RIDGE_POWER) * taper;
                    pos2(cx + (u - 0.5) * width, base - tall * h)
                })
                .collect();
            let mut mesh = Mesh::default();
            for w in shown.windows(2) {
                let (a, b) = (w[0], w[1]);
                let i = mesh.vertices.len() as u32;
                // A point under the base line: the line behind does not
                // show through the seam.
                for p in [a, b, pos2(b.x, base + 1.0), pos2(a.x, base + 1.0)] {
                    mesh.colored_vertex(p, ground);
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            painter.add(mesh);
            let fade = (1.0 - RIDGE_FAINTER * depth) * leaving;
            let left = cx - width / 2.0;
            painter.add(Shape::line(
                shown,
                PathStroke::new_uv(1.2, move |_, p| {
                    let level = ((base - p.y) / tall.max(1.0)).clamp(0.0, 1.0).sqrt();
                    let place = ((p.x - left) / width).clamp(0.0, 1.0);
                    colours.at(level.max(0.3), place).gamma_multiply(fade)
                }),
            ));
        }
    }

    /// The twelve pitch classes as bars, C on the left, each named under
    /// it, the loudest one's name bright.
    fn paint_chroma(&self, painter: &Painter, rect: Rect) {
        let labels = rect.height() >= 40.0;
        let font = FontId::proportional(10.5);
        let label_h = if labels { 14.0 } else { 0.0 };
        let floor = rect.bottom() - label_h;
        let room = floor - rect.top() - 1.0;
        let pitch = rect.width() / 12.0;
        let bar_w = (pitch * 0.62).clamp(2.0, 36.0);
        let colours = palette::visual_for(Visual::Chroma);
        let loudest = self
            .notes
            .chroma
            .iter()
            .enumerate()
            .filter(|(_, l)| **l > 0.0)
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(c, _)| c);
        let mut mesh = Mesh::default();
        for (c, &level) in self.notes.chroma.iter().enumerate() {
            let x = rect.left() + pitch * (c as f32 + 0.5);
            let place = c as f32 / 11.0;
            if level > 0.0 {
                let top = floor - room * level;
                let (tip, foot) = (colours.at(level, place), colours.at(0.0, place));
                let i = mesh.vertices.len() as u32;
                mesh.colored_vertex(pos2(x - bar_w / 2.0, top), tip);
                mesh.colored_vertex(pos2(x + bar_w / 2.0, top), tip);
                mesh.colored_vertex(pos2(x + bar_w / 2.0, floor), foot);
                mesh.colored_vertex(pos2(x - bar_w / 2.0, floor), foot);
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
        }
        painter.add(mesh);
        if labels {
            for (c, name) in notes::NAMES.iter().enumerate() {
                let x = rect.left() + pitch * (c as f32 + 0.5);
                let colour = if loudest == Some(c) {
                    palette::text_bright()
                } else if notes::is_black(c) {
                    palette::text_weak().gamma_multiply(0.7)
                } else {
                    palette::text_weak()
                };
                painter.text(
                    pos2(x, rect.bottom() - label_h / 2.0),
                    Align2::CENTER_CENTER,
                    *name,
                    font.clone(),
                    colour,
                );
            }
        }
    }

    /// A piano's keys at the bottom, lit by their notes' levels, and a bar
    /// over each key; as many octaves about middle C as fit.
    fn paint_piano(&self, painter: &Painter, rect: Rect) {
        let octaves = ((rect.width() / (7.0 * PIANO_WHITE)) as u8).clamp(2, 7);
        let lowest = 60 - 12 * (octaves / 2);
        let highest = lowest + 12 * octaves;
        let whites = 7 * usize::from(octaves) + 1;
        let white_w = rect.width() / whites as f32;
        let keys_top = rect.bottom() - (rect.height() * PIANO_KEYS).max(8.0);
        let floor = keys_top - 2.0;
        let room = (floor - rect.top()).max(1.0);
        let colours = palette::visual_for(Visual::Piano);
        let level = |m: u8| {
            self.notes
                .keys
                .get(usize::from(m.saturating_sub(notes::LOWEST)))
                .copied()
                .unwrap_or(0.0)
        };
        let place = |m: u8| f32::from(m - lowest) / f32::from(highest - lowest);
        let light = |l: f32| {
            ((l - PIANO_LIGHT_FROM) / PIANO_LIGHT_SPAN)
                .clamp(0.0, 1.0)
                .powf(1.5)
        };
        let white_key = palette::list_bg().lerp_to_gamma(palette::text_weak(), 0.28);
        let black_key = palette::list_bg().lerp_to_gamma(Color32::BLACK, 0.35);
        // Each note's key: its rect and whether it is black.
        let keys: Vec<(u8, Rect, bool)> = {
            let mut white = 0usize;
            (lowest..=highest)
                .map(|m| {
                    let black = notes::is_black(usize::from(m % 12));
                    let r = if black {
                        let x = rect.left() + white_w * white as f32;
                        let w = white_w * PIANO_BLACK_WIDTH;
                        let bottom = keys_top + (rect.bottom() - keys_top) * PIANO_BLACK_LENGTH;
                        Rect::from_min_max(pos2(x - w / 2.0, keys_top), pos2(x + w / 2.0, bottom))
                    } else {
                        let x = rect.left() + white_w * white as f32;
                        white += 1;
                        Rect::from_min_max(
                            pos2(x + 0.5, keys_top),
                            pos2(x + white_w - 0.5, rect.bottom()),
                        )
                    };
                    (m, r, black)
                })
                .collect()
        };
        let mut mesh = Mesh::default();
        // The bars, at the keys' middles.
        let bar_w = (white_w * 0.4).max(1.0);
        for &(m, r, _) in &keys {
            let l = level(m);
            if l <= 0.0 {
                continue;
            }
            let x = r.center().x;
            let top = floor - room * l;
            let (tip, foot) = (colours.at(l, place(m)), colours.at(0.0, place(m)));
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(pos2(x - bar_w / 2.0, top), tip);
            mesh.colored_vertex(pos2(x + bar_w / 2.0, top), tip);
            mesh.colored_vertex(pos2(x + bar_w / 2.0, floor), foot);
            mesh.colored_vertex(pos2(x - bar_w / 2.0, floor), foot);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        // The white keys, then the black ones over them.
        for black in [false, true] {
            for &(m, r, is_black) in keys.iter().filter(|k| k.2 == black) {
                let base = if is_black { black_key } else { white_key };
                let lit = colours.at(1.0, place(m));
                mesh.add_colored_rect(r, base.lerp_to_gamma(lit, light(level(m))));
            }
        }
        painter.add(mesh);
    }

    /// Off: the app icon and the name in the middle of the display,
    /// "AMP" in the accent, over a faint glow of it; the icon alone when
    /// the name does not fit.
    fn paint_logo(&mut self, painter: &Painter, rect: Rect) {
        let side = (rect.height() * 0.62).clamp(16.0, 64.0);
        let size = (side * 0.52).max(12.0);
        let mut job = egui::text::LayoutJob::default();
        let (name, amp) = crate::APP_NAME.split_at(crate::APP_NAME.len() - 3);
        for (text, colour) in [(name, palette::text_bright()), (amp, palette::accent())] {
            job.append(
                text,
                0.0,
                egui::TextFormat::simple(egui::FontId::proportional(size), colour),
            );
        }
        let galley = painter.layout_job(job);
        let gap = side * 0.22;
        let with_name = side + gap + galley.size().x <= rect.width() - 16.0;
        let width = if with_name {
            side + gap + galley.size().x
        } else {
            side
        };
        let left = rect.center().x - width / 2.0;
        // The glow: the accent fading out from behind the logo to the
        // display's ends.
        let glow = palette::accent().gamma_multiply(0.10);
        let clear = glow.gamma_multiply(0.0);
        let mut mesh = Mesh::default();
        let (cx, half) = (rect.center().x, (width * 0.9).max(rect.height()));
        for (x0, x1, c0, c1) in [(cx - half, cx, clear, glow), (cx, cx + half, glow, clear)] {
            let (x0, x1) = (x0.max(rect.left()), x1.min(rect.right()));
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(pos2(x0, rect.top()), c0);
            mesh.colored_vertex(pos2(x1, rect.top()), c1);
            mesh.colored_vertex(pos2(x1, rect.bottom()), c1);
            mesh.colored_vertex(pos2(x0, rect.bottom()), c0);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        painter.add(mesh);
        let icon = crate::ui::app_icon(painter.ctx(), side, &mut self.icon);
        let icon_rect = Rect::from_min_size(
            pos2(left, rect.center().y - side / 2.0),
            egui::vec2(side, side),
        );
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(icon.id(), icon_rect, uv, Color32::WHITE);
        if with_name {
            let pos = pos2(
                icon_rect.right() + gap,
                rect.center().y - galley.size().y / 2.0,
            );
            painter.galley(pos, galley, palette::text_bright());
        }
    }

    /// The spectrogram: the history rolling left out of the live spectrum
    /// on the right (the bands now as bars from the history's edge, with
    /// their peaks), moving smoothly between columns, fading out on the
    /// left; the columns heard on a beat brighter, the live spectrum lit.
    fn paint_gram(&mut self, painter: &Painter, rect: Rect) {
        let Some(rows) = self.gram.front().map(Vec::len).or_else(|| {
            let n = self.gram_live.len();
            (n > 0).then_some(n)
        }) else {
            return;
        };
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let emitter = gram_emitter(rect.width());
        let edge = rect.right() - emitter;
        let cap = columns(rect.width() - emitter, GRAM_PITCH);
        let visual = palette::visual_for(Visual::Spectrogram);
        let ground = palette::list_bg();
        // Each band's colours, from the ground up: the floor's, the top's
        // and the peaks' at its place (a rainbow from the bass up), or
        // along the level's when the colours tell it.
        let colours: Vec<[Color32; 4]> = (0..rows)
            .map(|band| {
                let place = band as f32 / (rows - 1).max(1) as f32;
                if visual.by_level() {
                    [
                        ground,
                        visual.at(0.0, place),
                        visual.at(0.6, place),
                        visual.at(1.0, place),
                    ]
                } else {
                    [
                        ground,
                        visual.at(0.0, place),
                        visual.at(1.0, place),
                        visual.peak(place),
                    ]
                }
            })
            .collect();
        if !self.gram.is_empty() {
            // Also the width: resized while paused, it is drawn anew.
            let drawn = (self.gram_count, visual, ground, cap);
            if self.gram_drawn != Some(drawn) || self.gram_texture.is_none() {
                // Right-aligned: the newest column at the right edge, the
                // top row the highest band.
                let mut image = ColorImage::filled([cap, rows], ground);
                let skip = cap.saturating_sub(self.gram.len());
                for (i, column) in self.gram.iter().enumerate().take(cap) {
                    for (band, &level) in column.iter().enumerate().take(rows) {
                        let y = rows - 1 - band;
                        image.pixels[y * cap + skip + i] = gram_colour(level, &colours[band]);
                    }
                }
                match &mut self.gram_texture {
                    Some(t) => t.set(image, TextureOptions::LINEAR),
                    None => {
                        self.gram_texture = Some(painter.ctx().load_texture(
                            "spectrogram",
                            image,
                            TextureOptions::LINEAR,
                        ));
                    }
                }
                self.gram_drawn = Some(drawn);
            }
            if let Some(t) = &self.gram_texture {
                // Moved left by the share of the next column due, so it
                // rolls smoothly rather than a column at a time.
                let part = if self.end.is_some() { self.owed } else { self.idle };
                let right = edge - part.clamp(0.0, 1.0) * GRAM_PITCH;
                let image = Rect::from_min_max(
                    pos2(right - cap as f32 * GRAM_PITCH, rect.top()),
                    pos2(right, rect.bottom()),
                );
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter
                    .with_clip_rect(painter.clip_rect().intersect(Rect::from_min_max(
                        rect.min,
                        pos2(edge, rect.bottom()),
                    )))
                    .image(t.id(), image, uv, Color32::WHITE);
            }
            // The oldest fade into the ground on the left.
            let fade = Rect::from_min_max(
                rect.min,
                pos2(rect.left() + (edge - rect.left()) * GRAM_FADE, rect.bottom()),
            );
            let mut mesh = Mesh::default();
            mesh.colored_vertex(fade.left_top(), ground);
            mesh.colored_vertex(fade.right_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(fade.right_bottom(), Color32::TRANSPARENT);
            mesh.colored_vertex(fade.left_bottom(), ground);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
        }
        // The live spectrum: a bar a few rows, from the edge to the right.
        let kick = self.gram_beat.kick;
        let dark = visual.is_dark();
        let row_h = rect.height() / rows as f32;
        let mut mesh = Mesh::default();
        if kick > 0.0 {
            let lit = visual.peak(0.5).gamma_multiply(0.14 * kick);
            let lit = if dark {
                Color32::from_rgba_premultiplied(lit.r(), lit.g(), lit.b(), 0)
            } else {
                lit
            };
            let glow = Rect::from_min_max(pos2(edge, rect.top()), rect.max);
            mesh.colored_vertex(glow.left_top(), lit);
            mesh.colored_vertex(glow.right_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(glow.right_bottom(), Color32::TRANSPARENT);
            mesh.colored_vertex(glow.left_bottom(), lit);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
        }
        let length = emitter - 3.0;
        for first in (0..rows).step_by(GRAM_BAR) {
            let last = (first + GRAM_BAR - 1).min(rows);
            let band = first + (last - first) / 2;
            let level = self.gram_live[first..last.max(first + 1)]
                .iter()
                .copied()
                .fold(0.0, f32::max);
            let peak = self.gram_peaks[first..last.max(first + 1)]
                .iter()
                .map(|p| p.0)
                .fold(0.0, f32::max);
            // The rows from the top: the highest band first.
            let y1 = rect.bottom() - first as f32 * row_h;
            let y0 = rect.bottom() - last as f32 * row_h;
            let shown = level.powf(GRAM_CONTRAST);
            if shown > 0.01 {
                let colour = gram_colour(level, &colours[band]);
                let tip = colour.lerp_to_gamma(colours[band][3], 0.35 + 0.4 * kick);
                let bar = Rect::from_min_max(pos2(edge, y0), pos2(edge + shown * length, y1));
                let i = mesh.vertices.len() as u32;
                mesh.colored_vertex(bar.left_top(), colour);
                mesh.colored_vertex(bar.right_top(), tip);
                mesh.colored_vertex(bar.right_bottom(), tip);
                mesh.colored_vertex(bar.left_bottom(), colour);
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            let peak_shown = peak.powf(GRAM_CONTRAST);
            if peak_shown > 0.05 {
                let x = edge + peak_shown * length;
                mesh.add_colored_rect(
                    Rect::from_min_max(pos2(x, y0), pos2(x + 1.5, y1)),
                    colours[band][3].gamma_multiply(0.85),
                );
            }
        }
        painter.add(mesh);
        // The seam the history flows out of.
        painter.vline(
            edge,
            rect.y_range(),
            Stroke::new(1.0, visual.at(0.5, 0.5).gamma_multiply(0.6 + 0.4 * kick)),
        );
    }
}

/// The width of the spectrogram's live spectrum in a display `width`
/// points wide.
fn gram_emitter(width: f32) -> f32 {
    (width * GRAM_EMITTER_SHARE).clamp(GRAM_EMITTER[0], GRAM_EMITTER[1])
}

/// `levels` each averaged with its neighbours (1 2 3 2 1), so a ridge
/// is a range of hills, not a saw.
pub(super) fn smoothed(levels: &[f32]) -> Vec<f32> {
    const KERNEL: [f32; 5] = [1.0, 2.0, 3.0, 2.0, 1.0];
    let n = levels.len() as isize;
    (0..n)
        .map(|i| {
            let (mut sum, mut weight) = (0.0, 0.0);
            for (k, w) in KERNEL.iter().enumerate() {
                let j = i + k as isize - 2;
                if (0..n).contains(&j) {
                    sum += levels[j as usize] * w;
                    weight += w;
                }
            }
            sum / weight
        })
        .collect()
}

/// How many ridges a display `height` points high shows.
fn ridge_lines(height: f32) -> usize {
    ((height / RIDGE_SPACING) as usize).clamp(6, 32)
}

/// How many columns of `pitch` points fit in `width`.
fn columns(width: f32, pitch: f32) -> usize {
    ((width / pitch) as usize).clamp(8, 2048)
}

/// A level's colour among `colours` at `GRAM_STOPS`; raised to
/// `GRAM_CONTRAST` first, so only the loud bands are bright (most of
/// music's bands sit in the upper half of the analyzer's range).
fn gram_colour(level: f32, colours: &[Color32; 4]) -> Color32 {
    let level = level.clamp(0.0, 1.0).powf(GRAM_CONTRAST);
    let i = GRAM_STOPS
        .windows(2)
        .position(|w| level <= w[1])
        .unwrap_or(GRAM_STOPS.len() - 2);
    let t = (level - GRAM_STOPS[i]) / (GRAM_STOPS[i + 1] - GRAM_STOPS[i]);
    let c = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    let (a, b) = (colours[i], colours[i + 1]);
    Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(f: f32, rate: u32, n: usize, offset: usize) -> Vec<f32> {
        (offset..offset + n)
            .map(|i| 0.5 * (2.0 * PI * f * i as f32 / rate as f32).sin())
            .collect()
    }

    /// What a frame of each visualization costs in a full bar's display,
    /// filled: the feed, the painting and egui's tessellation. Run with
    /// `cargo test --release paint_costs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn paint_costs() {
        use std::time::{Duration, Instant};
        let rate = 48000;
        let rect = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(760.0, 118.0));
        // Many bands lit: sines across the range, a little noise.
        let mut seed = 1u32;
        let sound = |offset: usize, seed: &mut u32| -> Vec<f32> {
            (offset..offset + FETCH)
                .map(|i| {
                    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (*seed >> 8) as f32 / (1 << 24) as f32 - 0.5;
                    let t = i as f32 / rate as f32;
                    [55.0, 110.0, 220.0, 440.0, 880.0, 1760.0, 3520.0, 7040.0]
                        .iter()
                        .map(|f| 0.08 * (2.0 * PI * f * t).sin())
                        .sum::<f32>()
                        + 0.05 * noise
                })
                .collect()
        };
        for visual in [
            Visual::Ridges,
            Visual::Strata,
            Visual::Vortex,
            Visual::Spectrum,
            Visual::Fire,
            Visual::Fireworks,
            Visual::Aurora,
            Visual::Cloth,
            Visual::Ripples,
            Visual::Ribbons,
            Visual::Goniometer,
            Visual::Strings,
            Visual::Harp,
            Visual::Waveform,
            Visual::Spectrogram,
            Visual::Scope,
        ] {
            let mut v = Visualizer::new();
            let ctx = egui::Context::default();
            let mut end = 10_000u64;
            // Filled: a few seconds of frames at 50 a second.
            for _ in 0..300 {
                end += 960;
                let s = sound(end as usize, &mut seed);
                // The scene must advance by 20 ms even in this tight
                // loop; otherwise fire and fireworks stay nearly empty.
                v.last = Instant::now() - Duration::from_millis(20);
                v.feed(
                    visual,
                    Some((&s, Heard { rate, end })),
                    Some(&s),
                    Some(rect.size()),
                );
            }
            let (mut feed, mut paint, mut tess) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
            let (mut vertices, frames) = (0, 100);
            for _ in 0..frames {
                end += 960;
                let s = sound(end as usize, &mut seed);
                v.last = Instant::now() - Duration::from_millis(20);
                let t = Instant::now();
                v.feed(
                    visual,
                    Some((&s, Heard { rate, end })),
                    Some(&s),
                    Some(rect.size()),
                );
                feed += t.elapsed();
                let mut painted = Duration::ZERO;
                let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let t = Instant::now();
                    v.paint(ui.painter(), rect);
                    painted = t.elapsed();
                });
                paint += painted;
                let t = Instant::now();
                let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
                tess += t.elapsed();
                vertices = prims
                    .iter()
                    .map(|p| match &p.primitive {
                        egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                        _ => 0,
                    })
                    .sum();
            }
            let ms = |d: Duration| d.as_secs_f64() * 1000.0 / frames as f64;
            println!(
                "{:<12} feed {:.3} ms, paint {:.3} ms, tessellate {:.3} ms, {} vertices",
                format!("{visual:?}"),
                ms(feed),
                ms(paint),
                ms(tess),
                vertices
            );
        }
    }

    #[test]
    fn the_waveform_rolls_with_the_frames_heard() {
        let mut v = Visualizer::new();
        let rate = 48000;
        let size = Some(Vec2::new(200.0, 80.0));
        let s = sine(1000.0, rate, FETCH, 0);
        v.feed(
            Visual::Waveform,
            Some((&s, Heard { rate, end: 10_000 })),
            None,
            size,
        );
        // The first frame only learns where the sound is.
        assert!(v.waveform.columns.is_empty());
        // 100 ms later: five columns of 20 ms, each peaking at 0.5 with
        // the RMS of a sine.
        v.feed(
            Visual::Waveform,
            Some((&s, Heard { rate, end: 14_800 })),
            None,
            size,
        );
        assert_eq!(v.waveform.columns.len(), 5);
        for c in &v.waveform.columns {
            assert!((c.peak - 0.5).abs() < 0.01, "{}", c.peak);
            assert!((c.rms - 0.5 / 2f32.sqrt()).abs() < 0.01, "{}", c.rms);
        }
        // Paused: the same end, nothing new.
        v.feed(
            Visual::Waveform,
            Some((&s, Heard { rate, end: 14_800 })),
            None,
            size,
        );
        assert_eq!(v.waveform.columns.len(), 5);
        assert!(v.active());
        // Another visualization starts afresh.
        v.feed(Visual::Spectrum, None, None, size);
        assert!(v.waveform.columns.is_empty());
    }

    #[test]
    fn the_ridges_add_a_line_every_50_ms_and_roll_out_when_stopped() {
        let mut v = Visualizer::new();
        let rate = 48000;
        let size = Some(Vec2::new(400.0, 90.0));
        let s = sine(1000.0, rate, FETCH, 0);
        v.feed(
            Visual::Ridges,
            Some((&s, Heard { rate, end: 10_000 })),
            None,
            size,
        );
        assert!(v.ridges.is_empty());
        // 100 ms: two lines of 100 points, the band of 1 kHz loudest (its
        // neighbours averaged in).
        v.feed(
            Visual::Ridges,
            Some((&s, Heard { rate, end: 14_800 })),
            None,
            size,
        );
        assert_eq!(v.ridges.len(), 2);
        assert_eq!(v.ridges[0].len(), 100);
        assert!(v.ridges[0].iter().any(|&l| l > 0.5));
        // 25 ms more: half the way on to the next places, the line heard
        // now coming up in front.
        v.feed(
            Visual::Ridges,
            Some((&s, Heard { rate, end: 16_000 })),
            None,
            size,
        );
        assert_eq!(v.ridges.len(), 2);
        assert!((v.ridge_moved - 0.5).abs() < 0.01, "{}", v.ridge_moved);
        assert_eq!(v.ridge_live.len(), 100);
        // Never more than the display holds.
        for i in 1..20 {
            let end = 14_800 + i * 8000;
            v.feed(Visual::Ridges, Some((&s, Heard { rate, end })), None, size);
        }
        assert_eq!(v.ridges.len(), ridge_lines(90.0));
        // Stopped: silent lines push the sound out, then nothing is left.
        for _ in 0..ridge_lines(90.0) + 2 {
            v.idle = 1.0;
            v.feed(Visual::Ridges, None, None, size);
        }
        assert!(!v.active());
    }

    #[test]
    fn the_scope_shows_a_sweep_then_its_afterglow_fades() {
        let mut v = Visualizer::new();
        let rate = 48000;
        let s = sine(1000.0, rate, FETCH, 7);
        // The first frame only learns where the sound is; the next one,
        // with frames new, sweeps.
        for end in [FETCH as u64, FETCH as u64 + 960] {
            v.feed(
                Visual::Scope,
                Some((&s, Heard { rate, end })),
                None,
                Some(Vec2::new(300.0, 80.0)),
            );
        }
        assert!(v.active());
        // Stopped: the afterglow fades.
        for _ in 0..20 {
            v.last = Instant::now() - std::time::Duration::from_millis(20);
            v.feed(Visual::Scope, None, None, Some(Vec2::new(300.0, 80.0)));
        }
        assert!(!v.active());
    }

    #[test]
    fn the_spectrogram_adds_a_column_every_20_ms() {
        let mut v = Visualizer::new();
        let rate = 44100;
        let size = Some(Vec2::new(100.0, 40.0));
        let s = sine(1000.0, rate, FETCH, 0);
        v.feed(
            Visual::Spectrogram,
            Some((&s, Heard { rate, end: 50_000 })),
            None,
            size,
        );
        v.feed(
            Visual::Spectrogram,
            Some((
                &s,
                Heard {
                    rate,
                    end: 50_000 + 2205,
                },
            )),
            None,
            size,
        );
        assert_eq!(v.gram.len(), 2);
        let column = &v.gram[0];
        assert_eq!(column.len(), 40);
        // The loudest row is the band of 1 kHz, about 3/5 of the way up
        // the bands of 40 Hz to 16 kHz.
        let (loudest, _) = column
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        assert!((20..28).contains(&loudest), "{loudest}");
        assert_eq!(
            gram_colour(
                0.0,
                &[Color32::BLACK, Color32::RED, Color32::GREEN, Color32::WHITE]
            ),
            Color32::BLACK
        );
        assert_eq!(
            gram_colour(
                1.0,
                &[Color32::BLACK, Color32::RED, Color32::GREEN, Color32::WHITE]
            ),
            Color32::WHITE
        );
    }

    /// A few bars of made-up music at 48 kHz, sample `i`: a kick on every
    /// beat (125 a minute), a snare on two and four, hi-hats on the eighths,
    /// a bass line and a chord changing every bar.
    fn music(i: usize) -> f32 {
        let rate = 48_000.0;
        let t = i as f32 / rate;
        let beat = 60.0 / 125.0;
        let in_beat = t % beat;
        let beat_no = (t / beat) as usize;
        let bar = beat_no / 4;
        let noise = music_noise(i);
        let kick = {
            let f = 50.0 + 90.0 * (-in_beat * 30.0).exp();
            (2.0 * PI * f * in_beat).sin() * (-in_beat * 7.0).exp() * 0.7
        };
        let snare = if beat_no % 2 == 1 {
            (noise * 0.5 + (2.0 * PI * 190.0 * in_beat).sin() * 0.3) * (-in_beat * 14.0).exp()
        } else {
            0.0
        };
        let in_eighth = t % (beat / 2.0);
        // Noise less its neighbour's: mostly treble.
        let hat = (noise - music_noise(i + 1)) * (-in_eighth * 45.0).exp() * 0.18;
        let roots = [55.0, 73.4, 61.7, 49.0];
        let root = roots[bar % 4];
        let bass = ((t * root).fract() * 2.0 - 1.0) * 0.18 * (0.6 + 0.4 * (-in_beat * 4.0).exp());
        let chord: f32 = [4.0, 5.0, 6.0]
            .iter()
            .map(|k| (2.0 * PI * root * k * t).sin() * 0.06)
            .sum();
        kick + snare + hat + bass + chord
    }

    fn music_noise(i: usize) -> f32 {
        let mut x = (i as u32).wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
        x ^= x >> 15;
        x = x.wrapping_mul(0x2c1b_3c6d);
        x ^= x >> 12;
        (x >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
    }

    /// Every visualization fed music, paused and stopped in displays from a
    /// point to a wide window's, at a low rate and the usual one, painted
    /// and tessellated: nothing panics (a display squeezed by the narrow
    /// bar, the mini player's least window, a strip being resized).
    #[test]
    fn every_visualization_takes_any_size() {
        use std::time::Duration;
        let sizes = [
            Vec2::new(1.0, 1.0),
            Vec2::new(7.0, 3.0),
            Vec2::new(40.0, 12.0),
            Vec2::new(176.0, 58.0),
            Vec2::new(30.0, 300.0),
            Vec2::new(1600.0, 160.0),
        ];
        const FRAMES: usize = 36;
        let ctx = egui::Context::default();
        for rate in [8000u32, 48_000] {
            let step = rate as usize / 50;
            // Playing for 20 frames, then paused (the same frames), then
            // stopped (none).
            let sound: Vec<f32> = (0..FETCH + 20 * step)
                .map(|i| music(i * 48_000 / rate as usize))
                .collect();
            for visual in Visual::ALL {
                for size in sizes {
                    let rect = Rect::from_min_size(pos2(5.0, 5.0), size);
                    let mut v = Visualizer::new();
                    for frame in 0..FRAMES {
                        let end = FETCH + frame.min(20) * step;
                        let s = &sound[end - FETCH..end];
                        let heard = (frame < 28).then_some((
                            s,
                            Heard {
                                rate,
                                end: end as u64,
                            },
                        ));
                        v.last = Instant::now() - Duration::from_millis(20);
                        v.feed(visual, heard, heard.map(|h| h.0), Some(size));
                        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                            v.paint(ui.painter(), rect);
                        });
                        out.textures_delta.clear();
                        ctx.tessellate(out.shapes, out.pixels_per_point);
                    }
                }
            }
        }
    }

    /// Frames of a visualization fed `music`, drawn by hand into a PNG
    /// sheet to look at: `METALAMP_FRAMES=<file.png>
    /// METALAMP_VISUAL=<settings name> cargo test --release visual_frames --
    /// --ignored` (`METALAMP_THEME=light` for the light theme).
    #[test]
    #[ignore]
    fn visual_frames() {
        use std::time::Duration;
        let out = std::env::var("METALAMP_FRAMES").expect("METALAMP_FRAMES=<file.png>");
        let name = std::env::var("METALAMP_VISUAL").unwrap_or_else(|_| "waveform".into());
        let visual: Visual = serde_json::from_str(&format!("\"{name}\"")).unwrap();
        if std::env::var("METALAMP_THEME").is_ok_and(|t| t == "light") {
            palette::set(palette::Look {
                theme: palette::Theme::Light,
                ..Default::default()
            });
        }
        let rate = 48_000;
        let size = Vec2::new(760.0, 118.0);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), size);
        let mut v = Visualizer::new();
        let mut renderer = crate::ui::test_render::Renderer::new(2.0);
        let mut frames = Vec::new();
        let mut end = FETCH as u64;
        for frame in 0..=400 {
            end += 960;
            let s: Vec<f32> = (end as usize - FETCH..end as usize).map(music).collect();
            v.last = Instant::now() - Duration::from_millis(20);
            v.feed(visual, Some((&s, Heard { rate, end })), Some(&s), Some(size));
            if frame >= 250 && frame % 25 == 0 {
                frames.push(renderer.frame(rect, palette::list_bg(), |p| v.paint(p, rect)));
            }
        }
        crate::ui::test_render::save(&out, &frames, renderer.pixels(size));
    }
}
