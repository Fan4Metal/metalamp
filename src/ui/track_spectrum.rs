//! The spectrum of a whole track in a window of its own, as Spek draws it
//! (the "Spectrum" chip of the file info window): time across, the
//! frequency up from 0 Hz to half the rate in a straight line, the level
//! in SoX's colours from black at −120 dB through blue, purple, red and
//! yellow to white at 0 dB, with the scales of the three around it. Worked
//! out by `audio::track_spectrum` on a thread, the picture filling from
//! the left as the file is decoded; drawn at the window's own pixels,
//! made again when it is resized. The pointer over it reads the time, the
//! frequency and the level there. Once worked out, it is saved as a PNG
//! (the button in the heading or Ctrl+S): the heading, the facts, the
//! picture and its scales drawn by hand (`ui::raster`) at `SAVE_SIZE` on a
//! thread of its own. Esc or the cross closes it; the window closed, the
//! work is given up.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::{Align2, Color32, FontId, Rect, TextureHandle, Ui, pos2, vec2};

use crate::audio::track_spectrum::{BINS, COLUMNS, Grid, Job, Worker, db_of};
use crate::palette;

const SIZE: egui::Vec2 = vec2(1000.0, 580.0);
const MIN_SIZE: egui::Vec2 = vec2(560.0, 340.0);
/// Room for the frequencies on the left, the colour scale on the right and
/// the times under the picture, in points.
const LEFT: f32 = 60.0;
const RIGHT: f32 = 82.0;
const BOTTOM: f32 = 26.0;
/// The picture is made again at most this often while columns come, in
/// seconds.
const REBUILD_S: f64 = 0.1;
/// The picture saved: its size in points and its pixels a point (1800 ×
/// 1020 pixels).
const SAVE_SIZE: egui::Vec2 = vec2(1200.0, 680.0);
const SAVE_SCALE: f32 = 1.5;
/// The margin round the window's content, in points.
const MARGIN: f32 = 16.0;

/// The window's title, by which it is found.
fn title() -> &'static str {
    tr!("Spectrum", "Спектр")
}

#[derive(Default)]
pub struct SpectrumWindow {
    open: bool,
    /// Opened this frame: the key that opened it must not close it.
    fresh: bool,
    path: PathBuf,
    /// "Artist – Title" and the facts (format, bit rate, rate…).
    heading: String,
    facts: String,
    job: Option<Arc<Job>>,
    worker: Worker,
    window: super::OwnWindow,
    picture: Option<Picture>,
    /// Save was asked for: the app shows Windows' dialog (on a thread of
    /// its own, `dialog.rs`) and gives the file back through
    /// [`SpectrumWindow::save_to`].
    pub ask_save: bool,
    /// The file chosen, written from the next frame.
    save_path: Option<PathBuf>,
    /// The PNG being written on its thread.
    saving: Option<Receiver<Result<PathBuf, String>>>,
    /// What came of the last save: the file written, or why not.
    saved: Option<Result<PathBuf, String>>,
}

/// The picture as last made: its texture, pixels and the grid's revision.
struct Picture {
    texture: TextureHandle,
    size: [usize; 2],
    revision: u64,
    made: f64,
}

impl SpectrumWindow {
    /// Shows `path`'s spectrum, worked out unless it is already (the same
    /// file opened again).
    pub fn open(&mut self, ctx: &egui::Context, path: &Path, heading: String, facts: String) {
        let kept = self.path == path
            && self
                .job
                .as_ref()
                .is_some_and(|j| !j.cancelled() && j.grid().error.is_none());
        if !kept {
            if let Some(job) = self.job.take() {
                job.cancel();
            }
            self.job = Some(self.worker.start(ctx, path));
            self.picture = None;
            self.path = path.to_path_buf();
            self.saved = None;
        }
        self.heading = heading;
        self.facts = facts;
        if self.open {
            ctx.send_viewport_cmd_to(viewport(), egui::ViewportCommand::Focus);
        } else {
            self.open = true;
            self.fresh = true;
        }
    }

    /// Closed: a spectrum not yet worked out is given up (the file let go).
    fn close(&mut self) {
        self.open = false;
        self.picture = None;
        if let Some(job) = self.job.take_if(|j| !j.grid().done) {
            job.cancel();
        }
    }

    /// The window, for the save dialog to be owned by.
    pub fn hwnd(&self) -> Option<isize> {
        self.window.hwnd()
    }

    /// The name the save dialog offers: the track's file's, as a PNG.
    pub fn file_name(&self) -> String {
        let stem = self
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        tr!(format!("{stem} - spectrum.png"), format!("{stem} - спектр.png"))
    }

    /// The file chosen in the save dialog: written from the next frame.
    pub fn save_to(&mut self, path: PathBuf) {
        self.save_path = Some(path);
    }

    /// The PNG written on a thread of its own once a file was chosen, and
    /// what came of it taken.
    fn follow_saving(&mut self, ctx: &egui::Context) {
        if let (Some(path), Some(job)) = (self.save_path.take(), self.job.clone()) {
            let (heading, facts) = (self.heading.clone(), self.facts.clone());
            let (send, receive) = channel();
            let wake = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("spectrum png".into())
                .spawn(move || {
                    let result = save_png(&path, &heading, &facts, &job.grid()).map(|()| path);
                    if let Err(e) = &result {
                        log::warn!("cannot save the spectrum: {e}");
                    }
                    let _ = send.send(result);
                    wake.request_repaint();
                });
            match spawned {
                Ok(_) => {
                    self.saving = Some(receive);
                    self.saved = None;
                }
                Err(e) => self.saved = Some(Err(e.to_string())),
            }
        }
        if let Some(receive) = &self.saving {
            match receive.try_recv() {
                Ok(result) => {
                    self.saved = Some(result);
                    self.saving = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.saving = None,
            }
        }
    }

    /// Draws the window while it is open, owned by `owner`.
    pub fn show(&mut self, ctx: &egui::Context, owner: Option<isize>) {
        if !self.open {
            return;
        }
        self.follow_saving(ctx);
        let fresh = std::mem::take(&mut self.fresh);
        if fresh {
            self.window.reset();
        }
        let centre = ctx
            .input(|i| i.viewport().outer_rect)
            .unwrap_or(ctx.content_rect())
            .center();
        let builder = super::OwnWindow::builder(egui::ViewportBuilder::default())
            .with_title(title())
            .with_decorations(false)
            .with_resizable(true)
            .with_inner_size(SIZE)
            .with_min_inner_size(MIN_SIZE)
            .with_position(centre - SIZE / 2.0)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            .with_icon(crate::window_icon());
        let dark = palette::current().dark;
        let mut close = false;
        ctx.show_viewport_immediate(viewport(), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            self.window.follow(&ctx, title(), owner, dark);
            close |= ctx.input(|i| i.viewport().close_requested());
            if !fresh && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                close = true;
            }
            crate::frameless::resize_edges(ui, true);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(palette::panel_bg())
                        .inner_margin(egui::Margin::same(MARGIN as i8)),
                )
                .show(ui, |ui| close |= self.content(ui));
        });
        if close {
            self.close();
        }
    }

    /// The heading, then the picture with its scales; whether to close.
    fn content(&mut self, ui: &mut Ui) -> bool {
        let Some(job) = self.job.clone() else {
            return true;
        };
        let ctx = ui.ctx().clone();
        // The heading moves the window; the widgets on it come later and
        // take their clicks first.
        let top = Rect::from_min_size(
            ctx.content_rect().min,
            vec2(ctx.content_rect().width(), 64.0),
        );
        crate::frameless::drag_area(ui, top, ui.id().with("spectrum_drag"), egui::Sense::hover());
        let mut close = false;
        let (done, error, columns) = {
            let grid = job.grid();
            (grid.done, grid.error.clone(), grid.columns())
        };
        let can_save = done && error.is_none() && self.saving.is_none();
        if can_save
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
        {
            self.ask_save = true;
        }
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = super::close_cross(ui);
                ui.add_space(8.0);
                if done && error.is_none() {
                    let button = egui::Button::new(tr!("Save as PNG…", "Сохранить в PNG…"));
                    if ui
                        .add_enabled(can_save, button)
                        .on_hover_text(tr!(
                            "The picture with its scales as a PNG file (Ctrl+S)",
                            "Картинка со шкалами в файл PNG (Ctrl+S)"
                        ))
                        .clicked()
                    {
                        self.ask_save = true;
                    }
                    if self.saving.is_some() {
                        ui.spinner();
                    }
                    self.saved_note(ui);
                }
                if !done {
                    let share = columns.min(COLUMNS) * 100 / COLUMNS;
                    ui.label(
                        egui::RichText::new(tr!(
                            format!("Analysing… {share} %"),
                            format!("Анализ… {share} %")
                        ))
                        .color(palette::text_weak()),
                    );
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&self.heading)
                                .size(17.0)
                                .strong()
                                .color(palette::text_bright()),
                        )
                        .truncate(),
                    )
                    .on_hover_text(self.path.display().to_string());
                });
            });
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new(&self.facts)
                    .size(12.0)
                    .color(palette::text_weak()),
            )
            .truncate(),
        );
        ui.add_space(10.0);

        let room = ui.available_rect_before_wrap();
        let plot = plot_in(room);
        ui.allocate_rect(room, egui::Sense::hover());
        if plot.width() < 40.0 || plot.height() < 40.0 {
            return close;
        }
        let painter = ui.painter_at(room);
        painter.rect_filled(plot, 0.0, Color32::BLACK);
        if let Some(e) = error {
            painter.text(
                plot.center(),
                Align2::CENTER_CENTER,
                tr!(
                    format!("Cannot decode the file: {e}"),
                    format!("Не удалось декодировать файл: {e}")
                ),
                FontId::proportional(14.0),
                palette::error(),
            );
            return close;
        }
        self.make_picture(&ctx, &job, plot, done);
        if let Some(p) = &self.picture {
            painter.image(
                p.texture.id(),
                plot,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let grid = job.grid();
        scales(&painter, plot, &grid);
        if let Some(at) = ui.ctx().pointer_hover_pos().filter(|p| plot.contains(*p)) {
            readout(&painter, plot, at, &grid);
        }
        close
    }

    /// What came of the last save: "Saved", a click showing the file in
    /// its folder, or why not, in red.
    fn saved_note(&self, ui: &mut Ui) {
        match &self.saved {
            Some(Ok(path)) => {
                let note = ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(tr!("Saved", "Сохранено"))
                                .color(palette::text_weak()),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(tr!(
                        format!("{}\nA click shows it in its folder", path.display()),
                        format!("{}\nЩелчок покажет его в папке", path.display())
                    ));
                if note.clicked() {
                    crate::win::show_in_folder(path);
                }
            }
            Some(Err(e)) => {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(tr!("Not saved", "Не сохранено"))
                            .color(palette::error()),
                    ),
                )
                .on_hover_text(e);
            }
            None => {}
        }
    }

    /// The picture made again for `plot`'s pixels when the grid or the
    /// size changed (while columns come, at most every `REBUILD_S`).
    fn make_picture(&mut self, ctx: &egui::Context, job: &Job, plot: Rect, done: bool) {
        let ppp = ctx.pixels_per_point();
        let size = [
            (plot.width() * ppp).round().max(1.0) as usize,
            (plot.height() * ppp).round().max(1.0) as usize,
        ];
        let revision = job.revision();
        let now = ctx.input(|i| i.time);
        let fresh = self.picture.as_ref().is_some_and(|p| {
            p.size == size && (p.revision == revision || (!done && now - p.made < REBUILD_S))
        });
        if fresh {
            if self
                .picture
                .as_ref()
                .is_some_and(|p| p.revision != revision)
            {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(REBUILD_S));
            }
            return;
        }
        let image = picture(&job.grid(), size);
        let options = egui::TextureOptions::LINEAR;
        match &mut self.picture {
            Some(p) => {
                p.texture.set(image, options);
                p.size = size;
                p.revision = revision;
                p.made = now;
            }
            None => {
                self.picture = Some(Picture {
                    texture: ctx.load_texture("track_spectrum", image, options),
                    size,
                    revision,
                    made: now,
                });
            }
        }
    }
}

/// The heading and the facts of `path` as the library (or the file heard
/// outside it) knows it: "Artist – Title", and the format, length, bit
/// rate, sample rate, channels and size; else the file's name and format.
pub fn about(path: &Path, track: Option<&crate::library::Track>) -> (String, String) {
    use crate::format;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut facts = Vec::new();
    if let Some(ext) = path.extension() {
        facts.push(ext.to_string_lossy().to_uppercase());
    }
    let Some(t) = track else {
        return (name, facts.join("  ·  "));
    };
    let title = if t.title.is_empty() {
        name
    } else {
        t.title.clone()
    };
    let heading = match t.artist.as_str() {
        "" => title,
        artist => format!("{artist} – {title}"),
    };
    if t.duration_ms > 0 {
        facts.push(format::time(u64::from(t.duration_ms)));
    }
    if t.bitrate > 0 {
        facts.push(format::bitrate(t.bitrate));
    }
    if t.sample_rate > 0 {
        facts.push(format::sample_rate(t.sample_rate));
    }
    if let Some(c) = format::channels(t.channels) {
        facts.push(c);
    }
    if t.size > 0 {
        facts.push(format::size(t.size));
    }
    (heading, facts.join("  ·  "))
}

/// The picture's place in `room`, the room under the heading: the scales
/// round it.
fn plot_in(room: Rect) -> Rect {
    Rect::from_min_max(
        pos2(room.left() + LEFT, room.top() + 8.0),
        pos2(room.right() - RIGHT, room.bottom() - BOTTOM),
    )
}

/// The spectrum as the window shows it, drawn by hand at `SAVE_SIZE` and
/// written to `path` as a PNG: the heading and the facts over the picture
/// and its scales, on the window's ground.
fn save_png(path: &Path, heading: &str, facts: &str, grid: &Grid) -> Result<(), String> {
    let mut renderer = super::raster::Renderer::new(SAVE_SCALE);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, SAVE_SIZE);
    let inner = rect.shrink(MARGIN);
    // The heading's row and the facts' line, as the window lays them out.
    let room = Rect::from_min_max(pos2(inner.left(), inner.top() + 54.0), inner.max);
    let plot = plot_in(room);
    let image = picture(grid, renderer.pixels(plot.size()));
    let pixels = renderer.frame(rect, palette::panel_bg(), |painter| {
        let line = |text: &str, size: f32, colour: Color32, top: f32| {
            let mut job = egui::text::LayoutJob::simple_singleline(
                text.to_owned(),
                FontId::proportional(size),
                colour,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(inner.width());
            let galley = painter.layout_job(job);
            painter.galley(pos2(inner.left(), top), galley, colour);
        };
        line(heading, 17.0, palette::text_bright(), inner.top() + 2.0);
        line(facts, 12.0, palette::text_weak(), inner.top() + 30.0);
        painter.rect_filled(plot, 0.0, Color32::BLACK);
        let texture =
            painter
                .ctx()
                .load_texture("spectrum_saved", image, egui::TextureOptions::NEAREST);
        painter.image(
            texture.id(),
            plot,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        scales(painter, plot, grid);
    });
    let [w, h] = renderer.pixels(SAVE_SIZE);
    image::save_buffer(
        path,
        &super::raster::rgb8(&pixels),
        w as u32,
        h as u32,
        image::ColorType::Rgb8,
    )
    .map_err(|e| e.to_string())
}

fn viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("track_spectrum_window")
}

/// The columns done drawn into `[w, h]` pixels, 0 Hz at the bottom: each
/// pixel the loudest of the columns and bins it covers; the columns not
/// yet done black.
fn picture(grid: &Grid, [w, h]: [usize; 2]) -> egui::ColorImage {
    let lut = colours();
    let done = grid.columns().min(COLUMNS);
    let mut pixels = vec![Color32::BLACK; w * h];
    let rows: Vec<(usize, usize)> = (0..h)
        .map(|y| {
            let r = h - 1 - y;
            let a = r * BINS / h;
            (a, ((r + 1) * BINS / h).max(a + 1))
        })
        .collect();
    let mut column = vec![0u8; BINS];
    for x in 0..w {
        let a = x * COLUMNS / w;
        if a >= done {
            break;
        }
        let b = ((x + 1) * COLUMNS / w).clamp(a + 1, done);
        column.copy_from_slice(&grid.levels[a * BINS..(a + 1) * BINS]);
        for c in a + 1..b {
            for (m, &l) in column
                .iter_mut()
                .zip(&grid.levels[c * BINS..(c + 1) * BINS])
            {
                *m = (*m).max(l);
            }
        }
        for (y, &(r0, r1)) in rows.iter().enumerate() {
            let level = column[r0..r1].iter().copied().max().unwrap_or(0);
            pixels[y * w + x] = lut[usize::from(level)];
        }
    }
    egui::ColorImage::new([w, h], pixels)
}

/// The 256 levels' colours.
fn colours() -> [Color32; 256] {
    std::array::from_fn(|i| colour(i as f32 / 255.0))
}

/// SoX's spectrogram colours (Spek's default): black, a dark blue, purple,
/// red, orange, yellow and white as `level` goes from 0 to 1.
fn colour(level: f32) -> Color32 {
    use std::f32::consts::{FRAC_PI_2, PI};
    let r = if level < 0.13 {
        0.0
    } else if level < 0.73 {
        ((level - 0.13) / 0.60 * FRAC_PI_2).sin()
    } else {
        1.0
    };
    let g = if level < 0.6 {
        0.0
    } else if level < 0.91 {
        ((level - 0.6) / 0.31 * FRAC_PI_2).sin()
    } else {
        1.0
    };
    let b = if level < 0.6 {
        0.5 * (level / 0.6 * PI).sin()
    } else if level < 0.78 {
        0.0
    } else {
        (level - 0.78) / 0.22
    };
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(byte(r), byte(g), byte(b))
}

/// A frequency as the scale names it: `5 kHz`, `2.5 kHz`.
fn khz(hz: f32) -> String {
    let k = hz / 1000.0;
    let number = if (k - k.round()).abs() < 1e-3 {
        format!("{}", k.round() as i64)
    } else {
        let text = format!("{k:.1}");
        match crate::i18n::lang() {
            crate::i18n::Lang::Ru => text.replace('.', ","),
            crate::i18n::Lang::En => text,
        }
    };
    tr!(format!("{number} kHz"), format!("{number} кГц"))
}

/// The smallest of `steps` whose marks are at least `least` points apart
/// when `whole` units take `length` points.
fn step_of(steps: &[f32], whole: f32, length: f32, least: f32) -> f32 {
    steps
        .iter()
        .copied()
        .find(|s| s / whole * length >= least)
        .unwrap_or(*steps.last().unwrap_or(&1.0))
}

/// The frequencies on the left, the times under the picture, the colour
/// scale on the right.
fn scales(painter: &egui::Painter, plot: Rect, grid: &Grid) {
    let font = FontId::proportional(11.5);
    let text = palette::text_weak();
    let tick = egui::Stroke::new(1.0, palette::line());
    if grid.rate > 0 {
        let top = grid.rate as f32 / 2.0;
        let steps = [500.0, 1000.0, 2000.0, 5000.0, 10_000.0, 20_000.0, 50_000.0];
        let step = step_of(&steps, top, plot.height(), 30.0);
        let mut hz = 0.0;
        while hz <= top {
            let y = plot.bottom() - hz / top * plot.height();
            painter.hline(plot.left() - 5.0..=plot.left(), y, tick);
            let at = y.clamp(plot.top() + 6.0, plot.bottom() - 6.0);
            painter.text(
                pos2(plot.left() - 8.0, at),
                Align2::RIGHT_CENTER,
                khz(hz),
                font.clone(),
                text,
            );
            hz += step;
        }
    }
    if grid.seconds > 0.0 {
        let whole = grid.seconds as f32;
        let steps = [
            1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0,
        ];
        let step = step_of(&steps, whole, plot.width(), 70.0);
        let x_of = |s: f32| plot.left() + s / whole * plot.width();
        let mut s = 0.0;
        let mut last = f32::NEG_INFINITY;
        while s <= whole {
            let x = x_of(s);
            painter.vline(x, plot.bottom()..=plot.bottom() + 5.0, tick);
            let align = if s == 0.0 {
                Align2::LEFT_TOP
            } else {
                Align2::CENTER_TOP
            };
            painter.text(
                pos2(x, plot.bottom() + 7.0),
                align,
                crate::format::time((s * 1000.0) as u64),
                font.clone(),
                text,
            );
            last = x;
            s += step;
        }
        // The end, as Spek marks it, unless a mark is near.
        let end = plot.right();
        if end - last > 56.0 {
            painter.vline(end, plot.bottom()..=plot.bottom() + 5.0, tick);
            painter.text(
                pos2(end, plot.bottom() + 7.0),
                Align2::RIGHT_TOP,
                crate::format::time((grid.seconds * 1000.0) as u64),
                font.clone(),
                text,
            );
        }
    }
    // The colour scale: 0 dB at the top.
    let bar = Rect::from_min_max(
        pos2(plot.right() + 14.0, plot.top()),
        pos2(plot.right() + 26.0, plot.bottom()),
    );
    let strips = 96;
    for i in 0..strips {
        let a = i as f32 / strips as f32;
        let b = (i + 1) as f32 / strips as f32;
        let strip = Rect::from_min_max(
            pos2(bar.left(), bar.bottom() - b * bar.height()),
            pos2(bar.right(), bar.bottom() - a * bar.height() + 0.5),
        );
        painter.rect_filled(strip, 0.0, colour((a + b) / 2.0));
    }
    let floor = crate::audio::track_spectrum::FLOOR_DB;
    let mut db = 0.0;
    while db >= floor {
        let y = bar.top() + db / floor * bar.height();
        painter.hline(bar.right()..=bar.right() + 4.0, y, tick);
        let at = y.clamp(bar.top() + 6.0, bar.bottom() - 6.0);
        painter.text(
            pos2(bar.right() + 7.0, at),
            Align2::LEFT_CENTER,
            tr!(format!("{db} dB"), format!("{db} дБ")),
            font.clone(),
            text,
        );
        db -= 20.0;
    }
}

/// A cross over the picture at `at` and a label with the time, the
/// frequency and the level there.
fn readout(painter: &egui::Painter, plot: Rect, at: egui::Pos2, grid: &Grid) {
    let line = egui::Stroke::new(1.0, Color32::from_white_alpha(70));
    painter.vline(at.x, plot.y_range(), line);
    painter.hline(plot.x_range(), at.y, line);
    let across = ((at.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let up = ((plot.bottom() - at.y) / plot.height()).clamp(0.0, 1.0);
    let mut parts = Vec::new();
    if grid.seconds > 0.0 {
        let ms = (f64::from(across) * grid.seconds * 1000.0) as u64;
        parts.push(format!("{}.{}", crate::format::time(ms), ms % 1000 / 100));
    }
    if grid.rate > 0 {
        let hz = up * grid.rate as f32 / 2.0;
        parts.push(tr!(format!("{hz:.0} Hz"), format!("{hz:.0} Гц")));
    }
    let column = ((across * COLUMNS as f32) as usize).min(COLUMNS - 1);
    let bin = ((up * BINS as f32) as usize).min(BINS - 1);
    if let Some(level) = grid.at(column, bin) {
        let db = db_of(level);
        parts.push(tr!(format!("{db:.0} dB"), format!("{db:.0} дБ")));
    }
    let galley = painter.layout_no_wrap(
        parts.join("  ·  "),
        FontId::proportional(12.5),
        Color32::WHITE,
    );
    let size = galley.size() + vec2(14.0, 8.0);
    let mut min = at + vec2(14.0, 14.0);
    if min.x + size.x > plot.right() {
        min.x = at.x - 14.0 - size.x;
    }
    if min.y + size.y > plot.bottom() {
        min.y = at.y - 14.0 - size.y;
    }
    let rect = Rect::from_min_size(min, size);
    painter.rect_filled(rect, 5.0, Color32::from_black_alpha(200));
    painter.galley(rect.min + vec2(7.0, 4.0), galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_run_from_black_to_white() {
        assert_eq!(colour(0.0), Color32::BLACK);
        assert_eq!(colour(1.0), Color32::WHITE);
        // Blue at the low levels, red over it, yellow near the top.
        let low = colour(0.25);
        assert!(low.b() > low.r() && low.g() == 0);
        let mid = colour(0.62);
        assert!(mid.r() > 200 && mid.g() < 100);
        let high = colour(0.88);
        assert!(high.r() == 255 && high.g() > 200);
    }

    #[test]
    fn a_picture_shows_the_columns_done() {
        let mut grid = Grid {
            rate: 44100,
            seconds: 10.0,
            ..Grid::default()
        };
        // Half the columns: loud at the top bin, silent elsewhere.
        for _ in 0..COLUMNS / 2 {
            let mut column = vec![0u8; BINS];
            column[BINS - 1] = 255;
            grid.levels.extend_from_slice(&column);
        }
        let image = picture(&grid, [100, 50]);
        assert_eq!(image.pixels[10], Color32::WHITE);
        assert_eq!(image.pixels[49 * 100 + 10], Color32::BLACK);
        assert_eq!(image.pixels[80], Color32::BLACK);
    }

    #[test]
    fn steps_keep_their_marks_apart() {
        assert_eq!(step_of(&[1.0, 2.0, 5.0], 10.0, 100.0, 15.0), 2.0);
        assert_eq!(step_of(&[1.0, 2.0, 5.0], 10.0, 10.0, 15.0), 5.0);
    }

    #[test]
    fn the_spectrum_is_saved_with_its_scales() {
        let mut grid = Grid {
            rate: 44100,
            seconds: 200.0,
            done: true,
            ..Grid::default()
        };
        // Loud at the bottom half, silent over it.
        for _ in 0..COLUMNS {
            let mut column = vec![0u8; BINS];
            column[..BINS / 2].fill(255);
            grid.levels.extend_from_slice(&column);
        }
        let dir = std::env::temp_dir().join(format!("metalamp_spectrum_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saved.png");
        save_png(&path, "Artist – Title", "MP3  ·  3:20", &grid).unwrap();
        let image = image::open(&path).unwrap().to_rgb8();
        std::fs::remove_dir_all(&dir).unwrap();
        let [w, h] = [
            (SAVE_SIZE.x * SAVE_SCALE) as u32,
            (SAVE_SIZE.y * SAVE_SCALE) as u32,
        ];
        assert_eq!(image.dimensions(), (w, h));
        // The plot: white (0 dB) low in it, black high in it.
        let plot = plot_in(Rect::from_min_max(
            pos2(MARGIN, MARGIN + 54.0),
            (SAVE_SIZE - vec2(MARGIN, MARGIN)).to_pos2(),
        ));
        let px = |p: egui::Pos2| {
            image.get_pixel((p.x * SAVE_SCALE) as u32, (p.y * SAVE_SCALE) as u32).0
        };
        let low = plot.left_bottom() + vec2(plot.width() / 2.0, -plot.height() / 4.0);
        let high = plot.left_top() + vec2(plot.width() / 2.0, plot.height() / 4.0);
        assert_eq!(px(low), [255, 255, 255]);
        assert_eq!(px(high), [0, 0, 0]);
        // Text was drawn: the heading's row is not only the ground.
        let ground = palette::panel_bg();
        let ground = [ground.r(), ground.g(), ground.b()];
        let y = ((MARGIN + 12.0) * SAVE_SCALE) as u32;
        assert!((0..w / 3).any(|x| image.get_pixel(x, y).0 != ground));
    }
}
