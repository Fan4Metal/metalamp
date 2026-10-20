//! The player bar's display: one of the visualizations (`Visual`, a
//! setting), chosen by the right button on it, the next one by a click.
//! All of them are fed the samples heard at this moment (`audio::vis::Tap`,
//! mono): the spectrum analyzer (`spectrum`), a waveform rolling from the
//! right (each column 20 ms of sound: its peak, and its RMS inside it, as
//! an editor draws a track), an oscilloscope (25 ms of the wave, started
//! at a rising zero crossing so a steady tone stands still) and a
//! spectrogram (the analyzer's bands as a column of colours every 20 ms,
//! rolling from the right: the loud ones bright). The waveform and the
//! spectrogram count the frames heard since the frame before (the tap's
//! `end`), so they roll with the sound, whatever the frame rate; paused,
//! they stand; stopped, they roll silence until they are empty. None
//! ("No visualization"), the display shows the app icon and the name, and
//! the app repaints only as the time needs; switched off in the settings,
//! nothing is painted (`player_bar::visual_display`).

use std::collections::VecDeque;
use std::time::Instant;

use egui::{Color32, ColorImage, Mesh, Painter, Rect, Shape, Stroke, TextureOptions, Vec2, pos2};

use crate::audio::vis::Heard;
use crate::palette;
use crate::settings::Visual;
use crate::ui::spectrum::{self, Analyzer};

/// The frames asked of the tap each frame: the analyzer's window, and
/// what is heard between two frames at 50 frames a second up to 192 kHz.
pub const FETCH: usize = 8192;
/// The time of a column of the waveform and of the spectrogram.
const COLUMN_S: f32 = 0.02;
/// Points per column of the waveform and of the spectrogram.
const WAVE_PITCH: f32 = 2.0;
const GRAM_PITCH: f32 = 2.0;
/// The oscilloscope's span, and how far back it looks for a zero crossing.
const SCOPE_S: f32 = 0.025;
const TRIGGER_S: f32 = 0.02;
/// The spectrogram's colours: a level's place among the ground, the
/// analyzer's foot, its top and its peaks.
const GRAM_STOPS: [f32; 4] = [0.0, 0.4, 0.75, 1.0];
const GRAM_CONTRAST: f32 = 2.2;

pub struct Visualizer {
    analyzer: Analyzer,
    /// The waveform's columns, oldest first: peak and RMS, 0 to 1.
    wave: VecDeque<[f32; 2]>,
    /// The column being gathered: its peak, sum of squares and frames.
    gathering: (f32, f32, usize),
    /// The spectrogram's columns, oldest first: the bands' levels, the
    /// lowest band first.
    gram: VecDeque<Vec<f32>>,
    /// Columns owed to the spectrogram (fractions carried on).
    gram_due: f32,
    gram_texture: Option<egui::TextureHandle>,
    /// What the texture shows: changed columns, colours or width make it
    /// anew.
    gram_drawn: Option<(u64, palette::VisualPalette, Color32, usize)>,
    gram_count: u64,
    /// The oscilloscope's stretch of the wave.
    scope: Vec<f32>,
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
            wave: VecDeque::new(),
            gathering: (0.0, 0.0, 0),
            gram: VecDeque::new(),
            gram_due: 0.0,
            gram_texture: None,
            gram_drawn: None,
            gram_count: 0,
            scope: Vec::new(),
            end: None,
            idle: 0.0,
            last: Instant::now(),
            shown: Visual::default(),
            icon: None,
        }
    }

    /// Moves `visual` on with the `FETCH` frames heard now (`None`:
    /// nothing is heard) for a display of `size` (`None`: none shown).
    pub fn feed(&mut self, visual: Visual, heard: Option<(&[f32], Heard)>, size: Option<Vec2>) {
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
        match visual {
            Visual::Spectrum => {
                let window = heard.map(|(s, _)| &s[s.len() - spectrum::N..]);
                let rate = heard.map_or(0, |(_, h)| h.rate);
                self.analyzer
                    .feed(window, rate, Analyzer::bands_for(size.x));
            }
            Visual::Waveform => {
                let cap = columns(size.x, WAVE_PITCH);
                match heard {
                    Some((s, h)) => {
                        let per_column = ((h.rate as f32 * COLUMN_S) as usize).max(1);
                        for &x in &s[s.len() - new..] {
                            let (peak, squares, n) = &mut self.gathering;
                            *peak = peak.max(x.abs());
                            *squares += x * x;
                            *n += 1;
                            if *n >= per_column {
                                let rms = (*squares / *n as f32).sqrt();
                                self.wave.push_back([peak.min(1.0), rms.min(1.0)]);
                                self.gathering = (0.0, 0.0, 0);
                            }
                        }
                        self.idle = 0.0;
                    }
                    None if self.wave.iter().any(|c| c[0] > 0.0) => {
                        for _ in 0..self.idle_columns(dt) {
                            self.wave.push_back([0.0; 2]);
                        }
                    }
                    None => self.wave.clear(),
                }
                while self.wave.len() > cap {
                    self.wave.pop_front();
                }
            }
            Visual::Spectrogram => {
                let cap = columns(size.x, GRAM_PITCH);
                let rows = (size.y.round() as usize).clamp(16, 256);
                if self.gram.front().is_some_and(|c| c.len() != rows) {
                    self.gram.clear();
                }
                let due = match heard {
                    Some((_, h)) => {
                        self.idle = 0.0;
                        self.gram_due += new as f32 / (h.rate as f32 * COLUMN_S);
                        let due = self.gram_due as usize;
                        self.gram_due -= due as f32;
                        due
                    }
                    None if self.gram.iter().any(|c| c.iter().any(|&l| l > 0.0)) => {
                        self.idle_columns(dt)
                    }
                    None => {
                        self.gram.clear();
                        0
                    }
                };
                if due > 0 {
                    // One column of the window heard now, as many times as
                    // are due (more than one only after a slow frame).
                    let column = match heard {
                        Some((s, h)) => {
                            self.analyzer
                                .levels(&s[s.len() - spectrum::N..], h.rate, rows)
                        }
                        None => vec![0.0; rows],
                    };
                    for _ in 0..due.min(cap) {
                        self.gram.push_back(column.clone());
                        self.gram_count += 1;
                    }
                }
                while self.gram.len() > cap {
                    self.gram.pop_front();
                }
            }
            Visual::Off => {}
            Visual::Scope => match heard {
                Some((s, h)) => {
                    let span = ((h.rate as f32 * SCOPE_S) as usize).clamp(16, s.len() / 2);
                    let reach = ((h.rate as f32 * TRIGGER_S) as usize).min(s.len() - span);
                    // The rising zero crossing nearest the end that leaves
                    // the span after it, else the last span.
                    let latest = s.len() - span;
                    let start = (latest - reach + 1..=latest)
                        .rev()
                        .find(|&i| s[i - 1] < 0.0 && s[i] >= 0.0)
                        .unwrap_or(latest);
                    self.scope.clear();
                    self.scope.extend_from_slice(&s[start..start + span]);
                }
                None => self.scope.clear(),
            },
        }
    }

    /// Silent columns owed for `dt` seconds of nothing heard.
    fn idle_columns(&mut self, dt: f32) -> usize {
        self.idle += dt / COLUMN_S;
        let due = self.idle as usize;
        self.idle -= due as f32;
        due
    }

    /// Something still shows: keep repainting.
    pub fn active(&self) -> bool {
        match self.shown {
            Visual::Spectrum => self.analyzer.active(),
            Visual::Waveform => !self.wave.is_empty(),
            Visual::Spectrogram => !self.gram.is_empty(),
            Visual::Scope => !self.scope.is_empty(),
            Visual::Off => false,
        }
    }

    /// Paints the visualization in `rect`.
    pub fn paint(&mut self, painter: &Painter, rect: Rect) {
        match self.shown {
            Visual::Spectrum => self.analyzer.paint(painter, rect),
            Visual::Waveform => self.paint_wave(painter, rect),
            Visual::Spectrogram => self.paint_gram(painter, rect),
            Visual::Scope => self.paint_scope(painter, rect),
            Visual::Off => self.paint_logo(painter, rect),
        }
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

    fn paint_wave(&self, painter: &Painter, rect: Rect) {
        let cy = rect.center().y;
        let half = rect.height() / 2.0;
        painter.hline(rect.x_range(), cy, Stroke::new(1.0, palette::line()));
        let mut mesh = Mesh::default();
        let n = self.wave.len();
        let colours = palette::visual();
        for (i, &[peak, rms]) in self.wave.iter().enumerate() {
            let x1 = rect.right() - WAVE_PITCH * (n - 1 - i) as f32;
            let x0 = (x1 - WAVE_PITCH).max(rect.left());
            if x1 <= rect.left() {
                continue;
            }
            // The peak's shade around the RMS's: by their levels when the
            // colours tell the level, else the floor's and the top's.
            let place = (x0 - rect.left()) / rect.width();
            let (outer, inner) = if colours.by_level() {
                (colours.at(peak, place), colours.at(rms, place))
            } else {
                (colours.at(0.0, place), colours.at(1.0, place))
            };
            for (level, colour) in [(peak, outer), (rms, inner)] {
                let h = (level * half).max(0.5);
                quad(&mut mesh, x0, cy - h, x1, cy + h, colour);
            }
        }
        painter.add(mesh);
    }

    fn paint_scope(&self, painter: &Painter, rect: Rect) {
        let cy = rect.center().y;
        let half = rect.height() / 2.0 - 1.0;
        painter.hline(rect.x_range(), cy, Stroke::new(1.0, palette::line()));
        if self.scope.len() < 2 {
            return;
        }
        // About two points a pixel: a span at 192 kHz is thinned out.
        let step = (self.scope.len() as f32 / (rect.width() * 2.0))
            .ceil()
            .max(1.0) as usize;
        let last = (self.scope.len() - 1) as f32;
        let points: Vec<_> = (0..self.scope.len())
            .step_by(step)
            .map(|i| {
                let x = rect.left() + rect.width() * i as f32 / last;
                let y = cy - self.scope[i].clamp(-1.0, 1.0) * half;
                pos2(x, y)
            })
            .collect();
        // A glow under the line, as a tube's trace; each point coloured by
        // its place (a rainbow along the line) or, when the colours tell
        // the level, by how far it swings.
        let colours = palette::visual();
        let colour_at = move |p: egui::Pos2, top: bool| {
            let place = (p.x - rect.left()) / rect.width();
            let level = if colours.by_level() {
                ((p.y - cy).abs() / half.max(1.0)).sqrt()
            } else if top {
                1.0
            } else {
                0.0
            };
            colours.at(level, place)
        };
        painter.add(Shape::line(
            points.clone(),
            egui::epaint::PathStroke::new_uv(4.0, move |_, p| {
                colour_at(p, false).gamma_multiply(0.35)
            }),
        ));
        painter.add(Shape::line(
            points,
            egui::epaint::PathStroke::new_uv(1.5, move |_, p| colour_at(p, true)),
        ));
    }

    fn paint_gram(&mut self, painter: &Painter, rect: Rect) {
        let Some(rows) = self.gram.front().map(Vec::len) else {
            return;
        };
        let cap = columns(rect.width(), GRAM_PITCH);
        let visual = palette::visual();
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
        // Also the width: resized while paused, it is drawn anew.
        let drawn = (self.gram_count, visual, ground, cap);
        if self.gram_drawn != Some(drawn) || self.gram_texture.is_none() {
            // Right-aligned: the newest column at the right edge, the top
            // row the highest band.
            let mut image = ColorImage::filled([cap, rows], ground);
            let skip = cap.saturating_sub(self.gram.len());
            for (i, column) in self.gram.iter().enumerate().take(cap) {
                for (band, &level) in column.iter().enumerate() {
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
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(t.id(), rect, uv, Color32::WHITE);
        }
    }
}

/// How many columns of `pitch` points fit in `width`.
fn columns(width: f32, pitch: f32) -> usize {
    ((width / pitch) as usize).clamp(8, 2048)
}

fn quad(mesh: &mut Mesh, x0: f32, y0: f32, x1: f32, y1: f32, colour: Color32) {
    mesh.add_colored_rect(Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)), colour);
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

    #[test]
    fn the_waveform_rolls_with_the_frames_heard() {
        let mut v = Visualizer::new();
        let rate = 48000;
        let size = Some(Vec2::new(200.0, 80.0));
        let s = sine(1000.0, rate, FETCH, 0);
        v.feed(
            Visual::Waveform,
            Some((&s, Heard { rate, end: 10_000 })),
            size,
        );
        // The first frame only learns where the sound is.
        assert!(v.wave.is_empty());
        // 100 ms later: five columns of 20 ms, each peaking at 0.5 with
        // the RMS of a sine.
        v.feed(
            Visual::Waveform,
            Some((&s, Heard { rate, end: 14_800 })),
            size,
        );
        assert_eq!(v.wave.len(), 5);
        for &[peak, rms] in &v.wave {
            assert!((peak - 0.5).abs() < 0.01, "{peak}");
            assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.01, "{rms}");
        }
        // Paused: the same end, nothing new.
        v.feed(
            Visual::Waveform,
            Some((&s, Heard { rate, end: 14_800 })),
            size,
        );
        assert_eq!(v.wave.len(), 5);
        assert!(v.active());
        // Another visualization starts afresh.
        v.feed(Visual::Spectrum, None, size);
        assert!(v.wave.is_empty());
    }

    #[test]
    fn the_scope_starts_at_a_rising_zero_crossing() {
        let mut v = Visualizer::new();
        let rate = 48000;
        // 1 kHz: a period of 48 frames; started off its zero.
        let s = sine(1000.0, rate, FETCH, 7);
        v.feed(
            Visual::Scope,
            Some((
                &s,
                Heard {
                    rate,
                    end: FETCH as u64,
                },
            )),
            Some(Vec2::new(300.0, 80.0)),
        );
        assert_eq!(v.scope.len(), (rate as f32 * SCOPE_S) as usize);
        assert!(v.scope[0] >= 0.0 && v.scope[0] < 0.07, "{}", v.scope[0]);
        assert!(v.scope[1] > v.scope[0]);
        v.feed(Visual::Scope, None, Some(Vec2::new(300.0, 80.0)));
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
}
