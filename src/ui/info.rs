//! The file info window, which edits the tags too: a click on the player
//! bar's cover opens it for the track heard, I, Alt+3 or the context menus
//! of the tracks and the playlist for the track selected; Ctrl+E and "Edit
//! tags…" open it editing the files selected. The cover, the title, the
//! artist and the album on top with the audio properties as chips; the
//! main fields in the usual order of taggers (the short ones two a line,
//! in boxes as while editing), the tag's other fields under them (only
//! those it has), the lyrics, the file's path and the rest; the text can be
//! selected and copied. Several files are shown one at a time (‹ ›) or all
//! at once: the main fields with the value the files share or
//! "(different)", and the files' names.
//!
//! Editing ("Edit" or Ctrl+E, `Form`): the main fields and the other fields
//! the tag has become text fields, read with `library::edit` on a thread;
//! a field the files differ in is left alone unless typed in; Save (Enter
//! with no field focused) writes only the fields typed in, on a thread; a
//! file that cannot be written (in use, a format lofty does not write) is
//! named under the fields and the others are written; Cancel or Esc gives
//! up. Going to another file saves first. The library reads the files
//! written again (`App`).
//!
//! The cross, Esc, Enter or a click outside closes it; while editing only
//! the cross (typing is not lost by a slip). When the program's window
//! cannot hold it (the player and playlist layout), it is a window of its
//! own, as the settings are. The file is read on a thread of its own
//! (`library::details`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Key, RichText, TextureHandle, Ui, vec2};
use lofty::picture::PictureType;
use lofty::prelude::ItemKey;

use crate::format;
use crate::keymap::{Action, Cmd, Keymap};
use crate::library::details::{self, Details};
use crate::library::edit::{self, Changes, Values};
use crate::palette;

type Read = (PathBuf, Result<Details, String>);

const WIDTH: f32 = 720.0;
/// The cover's side in the window, in points.
const COVER: f32 = 150.0;

/// The main fields in the order taggers show them; the short ones two a
/// line.
enum Line {
    One(edit::Key),
    Two(edit::Key, edit::Key),
}
const MAIN: [Line; 8] = [
    Line::One(edit::Key::Title),
    Line::One(edit::Key::Artist),
    Line::One(edit::Key::Album),
    Line::One(edit::Key::AlbumArtist),
    Line::One(edit::Key::Composer),
    Line::Two(edit::Key::Year, edit::Key::Genre),
    Line::Two(edit::Key::Track, edit::Key::Disc),
    Line::One(edit::Key::Comment),
];
/// The width of the first value of a line of two, in points.
const SHORT: f32 = 150.0;
/// The names' column at most, in points; a longer name is cut.
const MAX_NAME: f32 = 190.0;
/// The row of Cancel and Save under the fields while editing, in points.
const FOOTER: f32 = 44.0;
/// The files that could not be read or written, at most, in points.
const ERRORS: f32 = 70.0;

/// The window of its own: its title (by which it is found) and height.
fn window_title() -> &'static str {
    tr!("File info", "Сведения о файле")
}
const WINDOW_HEIGHT: f32 = 640.0;

#[derive(Default)]
pub struct InfoView {
    pub open: bool,
    /// Opened this frame: the key that opened it must not close it.
    fresh: bool,
    /// The files opened (the selection), at least one while open.
    paths: Vec<PathBuf>,
    /// The one shown, or none: all of them at once (several only).
    at: Option<usize>,
    /// The file whose details are asked for: the one shown (with all of
    /// them, the first, for its cover).
    wanted: Option<PathBuf>,
    /// What was read of it (or of the file before, until it is read).
    shown: Option<Read>,
    cover: Option<(PathBuf, TextureHandle)>,
    /// To the reading thread, started with the first file asked for.
    requests: Option<Sender<PathBuf>>,
    results: Option<Receiver<Read>>,
    /// The fields as `edit` reads and writes them: of all the files shown
    /// at once, or of the one edited.
    form: Option<Form>,
    /// Forms still writing when another file was opened (the main window
    /// stays interactive beside a window of its own): polled for what
    /// they wrote, then dropped.
    finishing: Vec<Form>,
    editing: bool,
    /// Where to go once the fields typed in are written (`go`).
    then: Option<Option<usize>>,
    /// In a window of its own (decided as it opens), and its handle once
    /// found.
    windowed: bool,
    window: super::OwnWindow,
}

/// What the window was asked to do this frame.
enum Act {
    Close,
    Edit,
    Cancel,
    Save,
    /// To a file, or to all of them.
    Go(Option<usize>),
}

#[derive(Default, PartialEq)]
enum State {
    #[default]
    Reading,
    Ready,
    Saving,
}

enum Msg {
    Read(Vec<Result<Values, String>>),
    /// The files written, and the errors.
    Written(usize, Vec<String>),
}

#[derive(Default)]
struct Field {
    value: String,
    /// The files differ in it; left alone unless typed in.
    mixed: bool,
    /// The value shown at first (empty when mixed).
    shown: String,
    /// Typed in and not as it was shown: written.
    touched: bool,
}

impl Field {
    /// The field of files whose values are `values`.
    fn of<'a>(mut values: impl Iterator<Item = &'a String>) -> Field {
        let first = values.next().cloned().unwrap_or_default();
        let mixed = values.any(|v| *v != first);
        let value = if mixed { String::new() } else { first };
        Field {
            shown: value.clone(),
            value,
            mixed,
            touched: false,
        }
    }
}

/// The tags of the files shown as `edit` has them: read on a thread,
/// changed in the text fields, written on a thread.
struct Form {
    paths: Vec<PathBuf>,
    state: State,
    rx: Receiver<Msg>,
    /// In `edit::Key::ALL`'s order.
    main: Vec<Field>,
    /// The other fields of one file (all the files at once: none).
    other: Vec<(ItemKey, Field)>,
    /// The files that could not be read or written, with why.
    errors: Vec<String>,
    /// The fields came while editing: the first one takes the keys.
    focus_first: bool,
}

/// What came to a form.
enum Polled {
    Nothing,
    /// The files were written (`any`: some of them), without an error
    /// (`ok`).
    Written {
        any: bool,
        ok: bool,
    },
}

impl Form {
    fn read(ctx: &egui::Context, paths: Vec<PathBuf>) -> Form {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let to_read = paths.clone();
        std::thread::spawn(move || {
            let read = to_read
                .iter()
                .map(|p| {
                    crate::guard::catch(p, || edit::read(p))
                        .unwrap_or_else(|| {
                            Err(tr!("cannot be read", "не удалось прочитать").to_owned())
                        })
                        .map_err(|e| format!("{}: {e}", p.display()))
                })
                .collect();
            let _ = tx.send(Msg::Read(read));
            ctx.request_repaint();
        });
        Form {
            paths,
            state: State::Reading,
            rx,
            main: Vec::new(),
            other: Vec::new(),
            errors: Vec::new(),
            focus_first: false,
        }
    }

    fn poll(&mut self) -> Polled {
        let mut out = Polled::Nothing;
        for msg in self.rx.try_iter().collect::<Vec<_>>() {
            match msg {
                Msg::Read(read) => {
                    let ok: Vec<&Values> = read.iter().filter_map(|r| r.as_ref().ok()).collect();
                    self.errors = read.iter().filter_map(|r| r.clone().err()).collect();
                    self.main = (0..edit::MAIN)
                        .map(|i| Field::of(ok.iter().map(|v| &v.main[i])))
                        .collect();
                    self.other = match ok.as_slice() {
                        [one] if self.paths.len() == 1 => one
                            .other
                            .iter()
                            .map(|(k, v)| (*k, Field::of(std::iter::once(v))))
                            .collect(),
                        _ => Vec::new(),
                    };
                    self.state = State::Ready;
                }
                Msg::Written(written, errors) => {
                    self.state = State::Ready;
                    out = Polled::Written {
                        any: written > 0,
                        ok: errors.is_empty(),
                    };
                    self.errors = errors;
                }
            }
        }
        out
    }

    fn fields(&self) -> impl Iterator<Item = &Field> {
        self.main.iter().chain(self.other.iter().map(|(_, f)| f))
    }

    fn touched(&self) -> bool {
        self.fields().any(|f| f.touched)
    }

    /// Every number typed in is one.
    fn valid(&self) -> bool {
        edit::Key::ALL
            .iter()
            .zip(&self.main)
            .all(|(k, f)| !f.touched || k.takes(&f.value))
    }

    /// The fields as they were read.
    fn reset(&mut self) {
        let other = self.other.iter_mut().map(|(_, f)| f);
        for f in self.main.iter_mut().chain(other) {
            f.value = f.shown.clone();
            f.touched = false;
        }
        self.errors.clear();
    }

    /// The fields typed in.
    fn changes(&self) -> Changes {
        let mut changes = Changes::default();
        for (c, f) in changes.main.iter_mut().zip(&self.main) {
            if f.touched {
                *c = Some(f.value.trim().to_owned());
            }
        }
        changes.other = self
            .other
            .iter()
            .filter(|(_, f)| f.touched)
            .map(|(k, f)| (*k, f.value.trim().to_owned()))
            .collect();
        changes
    }

    fn save(&mut self, ctx: &egui::Context) {
        let changes = self.changes();
        let paths = self.paths.clone();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut written = 0;
            let mut errors = Vec::new();
            for p in &paths {
                match crate::guard::catch(p, || edit::write(p, &changes))
                    .unwrap_or_else(|| {
                        Err(tr!("cannot be written", "не удалось записать").to_owned())
                    })
                {
                    Ok(()) => written += 1,
                    Err(e) => errors.push(format!("{}: {e}", p.display())),
                }
            }
            let _ = tx.send(Msg::Written(written, errors));
            ctx.request_repaint();
        });
        self.rx = rx;
        self.errors.clear();
        self.state = State::Saving;
    }

    /// The value every file has, else empty.
    fn common(&self, key: edit::Key) -> &str {
        self.main
            .get(key as usize)
            .filter(|f| !f.mixed)
            .map_or("", |f| f.shown.as_str())
    }
}

impl InfoView {
    /// Opens the window for `paths` (the first of them, or, editing
    /// several, all of them at once), editing when `edit`; the files are
    /// read again (the tags may have been changed since).
    pub fn open(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, edit: bool) {
        if paths.is_empty() {
            return;
        }
        self.open = true;
        self.fresh = true;
        self.editing = edit;
        self.then = None;
        let to = if edit && paths.len() > 1 {
            None
        } else {
            Some(0)
        };
        self.paths = paths;
        // Another file's details are not shown while these are read.
        self.shown = None;
        self.cover = None;
        if let Some(form) = self.form.take_if(|f| f.state == State::Saving) {
            self.finishing.push(form);
        }
        self.switch(ctx, to);
    }

    /// Shows the file `to` (none: all of them): its details asked for, the
    /// fields read for all of them or while editing.
    fn switch(&mut self, ctx: &egui::Context, to: Option<usize>) {
        self.at = to;
        let path = self.paths[to.unwrap_or(0)].clone();
        if to.is_some() && !edit::writable(&path) {
            self.editing = false;
        }
        self.form = match to {
            None => Some(Form::read(ctx, self.paths.clone())),
            Some(_) if self.editing => Some(Form::read(ctx, vec![path.clone()])),
            Some(_) => None,
        };
        if let Some(form) = &mut self.form {
            form.focus_first = self.editing;
        }
        self.request(ctx, &path);
    }

    /// Asks the "info" thread (started with the first file) for `path`.
    fn request(&mut self, ctx: &egui::Context, path: &Path) {
        self.wanted = Some(path.to_path_buf());
        if self.requests.is_none() {
            let (req_tx, req_rx) = channel::<PathBuf>();
            let (res_tx, res_rx) = channel();
            let ctx = ctx.clone();
            let started = std::thread::Builder::new()
                .name("info".into())
                .spawn(move || {
                    while let Ok(mut path) = req_rx.recv() {
                        // Only the last of the files asked for meanwhile.
                        while let Ok(next) = req_rx.try_recv() {
                            path = next;
                        }
                        let read = crate::guard::catch(&path, || details::read(&path))
                            .unwrap_or_else(|| Err("the file could not be read".into()));
                        if res_tx.send((path, read)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                });
            if let Err(e) = started {
                log::warn!("no info thread: {e}");
                return;
            }
            self.requests = Some(req_tx);
            self.results = Some(res_rx);
        }
        if let Some(tx) = &self.requests {
            let _ = tx.send(path.to_path_buf());
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.results else { return };
        while let Ok(mut read) = rx.try_recv() {
            if self.wanted.as_deref() != Some(read.0.as_path()) {
                continue;
            }
            self.cover = match &mut read.1 {
                Ok(d) => d.cover.take().map(|image| {
                    let texture = ctx.load_texture("file_info_cover", image, Default::default());
                    (read.0.clone(), texture)
                }),
                Err(_) => None,
            };
            self.shown = Some(read);
        }
    }

    /// What came to the form; whether files were written. Written without
    /// an error, the window goes where it was going, or stops editing; the
    /// files are read again either way.
    fn poll_form(&mut self, ctx: &egui::Context) -> bool {
        let mut written = false;
        self.finishing.retain_mut(|f| match f.poll() {
            Polled::Written { any, .. } => {
                written |= any;
                false
            }
            Polled::Nothing => true,
        });
        let Some(form) = &mut self.form else {
            return written;
        };
        let Polled::Written { any, ok } = form.poll() else {
            return written;
        };
        if ok {
            let to = match self.then.take() {
                Some(to) => to,
                None => {
                    self.editing = false;
                    self.at
                }
            };
            if self.open {
                self.switch(ctx, to);
            } else {
                self.form = None;
            }
        } else {
            self.then = None;
        }
        written || any
    }

    /// Goes to the file `to` (or all of them), saving the fields typed in
    /// first; not while one of them cannot be written (`stuck`).
    fn go(&mut self, ctx: &egui::Context, to: Option<usize>) {
        if self.stuck() {
            return;
        }
        if self.editing && self.form.as_ref().is_some_and(Form::touched) {
            self.save(ctx, Some(to));
        } else {
            self.switch(ctx, to);
        }
    }

    /// A field typed in holds what cannot be written (a number that is
    /// not one, red): nothing is saved and no other file is gone to.
    fn stuck(&self) -> bool {
        self.editing
            && self
                .form
                .as_ref()
                .is_some_and(|f| f.touched() && !f.valid())
    }

    fn save(&mut self, ctx: &egui::Context, then: Option<Option<usize>>) {
        let Some(form) = &mut self.form else { return };
        if self.editing && form.state == State::Ready && form.touched() && form.valid() {
            form.save(ctx);
            self.then = then;
        }
    }

    fn start_editing(&mut self, ctx: &egui::Context) {
        if self.editing {
            return;
        }
        match self.at {
            Some(i) => {
                let path = self.paths[i].clone();
                if !edit::writable(&path) {
                    return;
                }
                let mut form = Form::read(ctx, vec![path]);
                form.focus_first = true;
                self.form = Some(form);
            }
            None => {
                let Some(form) = &mut self.form else { return };
                if form.state != State::Ready {
                    return;
                }
                form.reset();
                form.focus_first = true;
            }
        }
        self.editing = true;
    }

    fn stop_editing(&mut self) {
        if self.form.as_ref().is_some_and(|f| f.state == State::Saving) {
            return;
        }
        self.editing = false;
        self.then = None;
        match self.at {
            Some(_) => self.form = None,
            None => {
                if let Some(form) = &mut self.form {
                    form.reset();
                }
            }
        }
    }

    fn act(&mut self, ctx: &egui::Context, act: Act) {
        match act {
            Act::Close => {
                self.open = false;
                self.editing = false;
                self.then = None;
                // A write under way goes on and is reported.
                if self
                    .form
                    .as_ref()
                    .is_some_and(|f| f.state != State::Saving)
                {
                    self.form = None;
                }
            }
            Act::Edit => self.start_editing(ctx),
            Act::Cancel => self.stop_editing(),
            Act::Save => self.save(ctx, None),
            Act::Go(to) => self.go(ctx, to),
        }
    }

    /// The window's keys: Esc or Enter closes it, the keys of "Edit
    /// tags" edit; editing, Esc gives up and Enter (no field holding it)
    /// saves.
    fn keys(&self, ctx: &egui::Context, keys: &Keymap, acts: &mut Vec<Act>) {
        if self.editing {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                acts.push(Act::Cancel);
            }
            if ctx.memory(|m| m.focused().is_none())
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
            {
                acts.push(Act::Save);
            }
        } else {
            if ctx.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter)) {
                acts.push(Act::Close);
            }
            let chords = keys.chords(Action::EditTags);
            if crate::keymap::take(ctx, keys, |c| chords.contains(c)).contains(&Cmd::EditTags) {
                acts.push(Act::Edit);
            }
        }
    }

    /// Draws the window while it is open; whether files were written (the
    /// library reads them again).
    pub fn show(&mut self, ctx: &egui::Context, owner: Option<isize>, keys: &Keymap) -> bool {
        self.poll(ctx);
        let written = self.poll_form(ctx);
        if !self.open {
            return written;
        }
        let fresh = std::mem::take(&mut self.fresh);
        if fresh {
            let room = ctx.content_rect().size();
            self.windowed = room.x < WIDTH + 64.0 || room.y < 420.0;
            self.window.reset();
        }
        let mut acts = Vec::new();
        if self.windowed {
            self.window(ctx, owner, fresh, keys, &mut acts);
        } else {
            let modal = egui::Modal::new(egui::Id::new("file_info"))
                .frame(
                    egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
                        .fill(palette::panel_bg())
                        .inner_margin(egui::Margin::same(20))
                        .corner_radius(10.0),
                )
                .show(ctx, |ui| {
                    let height = ctx.content_rect().height();
                    self.content(ui, Some(height), &mut acts);
                });
            if !fresh {
                self.keys(ctx, keys, &mut acts);
                if !self.editing && modal.should_close() {
                    acts.push(Act::Close);
                }
            }
        }
        for act in acts {
            self.act(ctx, act);
        }
        written
    }

    /// The info in a window of its own (the settings' way): frameless,
    /// owned by the program's window, centred over it; moved by its top,
    /// closed by the cross, Esc or Enter.
    fn window(
        &mut self,
        ctx: &egui::Context,
        owner: Option<isize>,
        fresh: bool,
        keys: &Keymap,
        acts: &mut Vec<Act>,
    ) {
        let size = vec2(WIDTH + 40.0, WINDOW_HEIGHT);
        let centre = ctx
            .input(|i| i.viewport().outer_rect)
            .unwrap_or(ctx.content_rect())
            .center();
        let builder = super::OwnWindow::builder(egui::ViewportBuilder::default())
            .with_title(window_title())
            .with_decorations(false)
            .with_resizable(true)
            .with_inner_size(size)
            .with_min_inner_size(vec2(WIDTH + 40.0, 360.0))
            .with_position(centre - size / 2.0)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            .with_icon(crate::window_icon());
        let id = egui::ViewportId::from_hash_of("file_info_window");
        let dark = palette::current().dark;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            self.window.follow(&ctx, window_title(), owner, dark);
            if ctx.input(|i| i.viewport().close_requested()) {
                acts.push(Act::Close);
            }
            crate::frameless::resize_edges(ui);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(palette::panel_bg())
                        .inner_margin(egui::Margin::same(20)),
                )
                .show(ui, |ui| {
                    // Its top moves it; the widgets on it come later and
                    // take their clicks first.
                    let top = egui::Rect::from_min_size(
                        ctx.content_rect().min,
                        vec2(ctx.content_rect().width(), 20.0 + COVER),
                    );
                    crate::frameless::drag_area(
                        ui,
                        top,
                        ui.id().with("info_drag"),
                        egui::Sense::hover(),
                    );
                    self.content(ui, None, acts);
                });
            if !fresh {
                self.keys(&ctx, keys, acts);
            }
        });
    }

    /// The window's content: the hero and the body in a scroll area for
    /// `height` (the program's window), or the rest of a window of its own
    /// (none); editing, Cancel and Save under it.
    fn content(&mut self, ui: &mut Ui, height: Option<f32>, acts: &mut Vec<Act>) {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
        crate::ui::settings::round_widgets(ui);
        let wanted = self.wanted.clone().unwrap_or_default();
        // Going to another file, the one before stays until it is read: a
        // window shrunk to "Reading..." for a moment and grown back blinked.
        let shown = self.shown.as_ref().map(|(_, d)| d);
        let cover = self.cover.as_ref().map(|(_, t)| t.clone());
        let saving = self
            .form
            .as_ref()
            .is_some_and(|f| f.state == State::Saving);
        let nav = Nav {
            at: self.at,
            count: self.paths.len(),
            enabled: !saving && !self.stuck(),
        };
        let top = match (self.at, shown) {
            (None, _) => Top::all(self.paths.len(), self.form.as_ref()),
            (Some(_), Some(Ok(d))) => Top::of(d),
            (Some(_), _) => Top::named(&wanted),
        };
        // All the files: their cover when they are of one folder.
        let one_folder = self.paths.windows(2).all(|w| w[0].parent() == w[1].parent());
        let cover = cover.filter(|_| self.at.is_some() || one_folder);
        hero(ui, &top, cover.as_ref(), &nav, acts);

        let errors = self.form.as_ref().is_some_and(|f| !f.errors.is_empty());
        let footer = match (self.editing, errors) {
            (false, _) => 0.0,
            (true, false) => FOOTER,
            (true, true) => FOOTER + ERRORS + 6.0,
        };
        // A long tag (lyrics, many fields) scrolls within the screen
        // instead of pushing the window off it. The height is forced
        // (`min_scrolled_height`), not left to the room below: a centred
        // modal is placed by its size of the frame before, so an area that
        // only took the room left froze at the height the window had while
        // the file was read. In a window of its own (`height` none) the
        // area takes the rest of it.
        let room = height.map_or(ui.available_height() - footer, |h| {
            (h - 2.0 * 20.0 - COVER - 60.0 - footer).max(120.0)
        });
        let editing = self.editing;
        let reading = |ui: &mut Ui| {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(tr!("Reading…", "Чтение…"));
            });
        };
        match (self.at, shown) {
            (None, _) => match &mut self.form {
                Some(form) => {
                    let paths = &self.paths;
                    scrolled(ui, room, |ui| {
                        ui.add_enabled_ui(!saving, |ui| body_all(ui, form, paths, editing, acts));
                    });
                }
                None => reading(ui),
            },
            (Some(_), None) => reading(ui),
            (Some(_), Some(Err(e))) => {
                ui.add_space(16.0);
                ui.colored_label(
                    palette::error(),
                    tr!(
                        format!("Cannot read the file: {e}"),
                        format!("Не удалось прочитать файл: {e}")
                    ),
                );
            }
            (Some(_), Some(Ok(d))) => {
                // Gone to another file, the one before stays shown until
                // it is read: its fields are shown as they are, not in
                // the other file's form.
                let form = self
                    .form
                    .as_mut()
                    .filter(|f| editing && f.paths.first() == Some(&d.path));
                scrolled(ui, room, |ui| {
                    ui.add_enabled_ui(!saving, |ui| body_one(ui, d, form, acts));
                });
            }
        }
        if editing {
            // Its right edge where the fields end, left of the scroll bar.
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    right: 14,
                    ..egui::Margin::ZERO
                })
                .show(ui, |ui| footer_row(ui, self.form.as_ref(), acts));
        }
    }
}

/// The body in a scroll area of `room` points, with room for the scroll
/// bar at the right edge (`set_max_width` here also shrank the area's
/// height to its minimum).
fn scrolled(ui: &mut Ui, room: f32, add: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .id_salt("file_info_body")
        .min_scrolled_height(room)
        .max_height(room)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    right: 14,
                    ..egui::Margin::ZERO
                })
                .show(ui, add);
        });
}

/// What the top of the window says: the title, the artist, the album with
/// its year and genre, the chips.
struct Top {
    title: String,
    artist: String,
    album: String,
    chips: Vec<String>,
}

impl Top {
    fn of(d: &Details) -> Top {
        let title = match value(d, &[ItemKey::TrackTitle]) {
            "" => stem(&d.path),
            t => t.to_owned(),
        };
        let album = [
            value(d, edit::Key::Album.items()),
            value(d, edit::Key::Year.items()),
            value(d, edit::Key::Genre.items()),
        ];
        Top {
            title,
            artist: value(d, &[ItemKey::TrackArtist, ItemKey::AlbumArtist]).to_owned(),
            album: joined(&album),
            chips: chips(d),
        }
    }

    /// While the file is read, or when it cannot be: its name alone.
    fn named(path: &Path) -> Top {
        Top {
            title: stem(path),
            artist: String::new(),
            album: String::new(),
            chips: Vec::new(),
        }
    }

    /// All the files: their number, and what they share.
    fn all(count: usize, form: Option<&Form>) -> Top {
        let title =
            crate::i18n::count(count as u64, ["file", "files"], ["файл", "файла", "файлов"]);
        let Some(form) = form.filter(|f| f.state != State::Reading) else {
            return Top {
                title,
                ..Top::named(Path::new(""))
            };
        };
        let artist = match form.common(edit::Key::Artist) {
            "" => form.common(edit::Key::AlbumArtist),
            a => a,
        };
        let album = [
            form.common(edit::Key::Album),
            form.common(edit::Key::Year),
            form.common(edit::Key::Genre),
        ];
        Top {
            title,
            artist: artist.to_owned(),
            album: joined(&album),
            chips: Vec::new(),
        }
    }
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The parts that are not empty, with dots between them.
fn joined(parts: &[&str]) -> String {
    let parts: Vec<&str> = parts.iter().copied().filter(|s| !s.is_empty()).collect();
    parts.join("  ·  ")
}

/// Going through several files: the one shown (none: all of them), how
/// many, and whether it can be done now (not while writing).
struct Nav {
    at: Option<usize>,
    count: usize,
    enabled: bool,
}

/// The top of the window: the cover, the title, the artist, the album
/// line and the chips; in the corner the cross and, for several files,
/// "All" and the arrows.
fn hero(ui: &mut Ui, top: &Top, cover: Option<&TextureHandle>, nav: &Nav, acts: &mut Vec<Act>) {
    ui.horizontal_top(|ui| {
        cover_box(ui, cover, COVER);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if super::close_cross(ui) {
                        acts.push(Act::Close);
                    }
                    if nav.count > 1 {
                        ui.add_space(6.0);
                        ui.add_enabled_ui(nav.enabled, |ui| nav_buttons(ui, nav, acts));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&top.title)
                                    .size(22.0)
                                    .strong()
                                    .color(palette::text_bright()),
                            )
                            .truncate(),
                        );
                    });
                });
            });
            if !top.artist.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(&top.artist).size(16.0).color(palette::text()))
                        .truncate(),
                );
            }
            if !top.album.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(&top.album)
                            .size(13.5)
                            .color(palette::text_weak()),
                    )
                    .truncate(),
                );
            }
            if !top.chips.is_empty() {
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for text in &top.chips {
                        chip(ui, text);
                    }
                });
            }
        });
    });
}

/// "All N", the previous file, "3 / 12", the next one (laid out from the
/// right).
fn nav_buttons(ui: &mut Ui, nav: &Nav, acts: &mut Vec<Act>) {
    use crate::ui::icons::Icon;
    let n = nav.count;
    let next = match nav.at {
        None => Some(0),
        Some(i) if i + 1 < n => Some(i + 1),
        Some(_) => None,
    };
    let previous = nav.at.and_then(|i| i.checked_sub(1));
    let tip = tr!("Next file", "Следующий файл");
    if ui
        .add_enabled_ui(next.is_some(), |ui| super::icon_button(ui, Icon::Next, false, tip))
        .inner
        .clicked()
    {
        acts.push(Act::Go(next));
    }
    if let Some(i) = nav.at {
        ui.label(RichText::new(format!("{} / {n}", i + 1)).color(palette::text_weak()));
    }
    let tip = tr!("Previous file", "Предыдущий файл");
    if ui
        .add_enabled_ui(previous.is_some(), |ui| {
            super::icon_button(ui, Icon::Prev, false, tip)
        })
        .inner
        .clicked()
    {
        acts.push(Act::Go(previous));
    }
    ui.add_space(4.0);
    let all = nav.at.is_none();
    if ui
        .add(egui::Button::new(tr!(format!("All {n}"), format!("Все {n}"))).selected(all))
        .on_hover_text(tr!(
            "The main fields of all the files at once",
            "Основные поля всех файлов сразу"
        ))
        .clicked()
        && !all
    {
        acts.push(Act::Go(None));
    }
}

/// How the main fields are shown: as the file's details have them, as
/// all the files share them, or in text fields.
enum Mode<'a> {
    Read(&'a Details),
    All(&'a Form),
    Edit(&'a mut Form),
}

/// One file's sections: the main fields, the tag's other fields (only
/// those it has), the lyrics, then the file. The names of every section
/// in one column. Editing (`form`), the fields of `edit` in text fields.
fn body_one(ui: &mut Ui, d: &Details, form: Option<&mut Form>, acts: &mut Vec<Act>) {
    ui.add_space(18.0);
    ui.spacing_mut().item_spacing.y = 6.0;
    let editing = form.is_some();
    // Editing: the fields `edit` read (in the order shown), the others as
    // they are.
    let form = form.filter(|f| f.state != State::Reading);
    let mut more: Vec<Other> = d
        .fields
        .iter()
        .filter(|(k, _)| edit::is_other(*k))
        .map(|(k, v)| {
            let edited = form
                .as_ref()
                .and_then(|f| f.other.iter().position(|(o, _)| o == k));
            match edited {
                Some(i) => Other::Edited(*k, i),
                None => Other::Shown(*k, v.as_str()),
            }
        })
        .collect();
    if let Some(f) = &form {
        for (i, (k, _)) in f.other.iter().enumerate() {
            if !more.iter().any(|o| o.key() == *k) {
                more.push(Other::Edited(*k, i));
            }
        }
    }
    let file = file_lines(d);
    let names = main_names()
        .chain(more.iter().map(|o| field_name(o.key())))
        .chain(file.iter().map(|(n, _)| n.clone()));
    let width = names_width(ui, names);
    let pad = EDIT_PAD;

    let tags = d.tags.join(" · ");
    section(ui, tr!("Main", "Основное"), |ui| {
        if !editing {
            let why_not = (!edit::writable(&d.path)).then_some(tr!(
                "Tags of this format cannot be written",
                "Теги этого формата записать нельзя"
            ));
            edit_button(ui, why_not, acts);
            ui.add_space(8.0);
        }
        ui.label(RichText::new(tags).size(11.0).color(palette::text_weak()));
    });
    let mut form = form;
    match form.as_mut() {
        Some(f) => main_lines(ui, width, pad, &mut Mode::Edit(f)),
        // Editing, until the fields are read: as shown.
        None => main_lines(ui, width, pad, &mut Mode::Read(d)),
    }

    if !more.is_empty() {
        ui.add_space(14.0);
        section(ui, tr!("Additional", "Дополнительно"), |_| {});
        for o in &more {
            let n = field_name(o.key());
            match (o, &mut form) {
                (Other::Edited(_, i), Some(f)) => line(ui, width, &n, pad, |ui| {
                    if let Some((_, field)) = f.other.get_mut(*i) {
                        text_field(ui, field, false, true, None, false);
                    }
                }),
                (Other::Shown(_, text), _) => line(ui, width, &n, pad, |ui| {
                    boxed(ui, text, "", Some(1), None);
                }),
                (Other::Edited(..), None) => {}
            }
        }
    }

    // The lyrics apart: many lines, which the fields' grid would stretch.
    let lyrics = value(d, &[ItemKey::UnsyncLyrics, ItemKey::Lyrics]);
    if !lyrics.is_empty() {
        ui.add_space(14.0);
        section(ui, tr!("Lyrics", "Текст"), |_| {});
        ui.add(egui::Label::new(RichText::new(lyrics).color(palette::text())).wrap());
    }

    ui.add_space(14.0);
    section(ui, tr!("File", "Файл"), |ui| {
        if ui
            .small_button(tr!("Show in folder", "Показать в папке"))
            .clicked()
        {
            crate::win::show_in_folder(&d.path);
        }
    });
    for (n, text) in &file {
        line(ui, width, n, 0.0, |ui| shown_value(ui, text, true));
    }
}

/// A field of the section "Additional": in a text field (its place in
/// `Form::other`) or as it is.
enum Other<'a> {
    Edited(ItemKey, usize),
    Shown(ItemKey, &'a str),
}

impl Other<'_> {
    fn key(&self) -> ItemKey {
        match self {
            Other::Edited(k, _) | Other::Shown(k, _) => *k,
        }
    }
}

/// All the files at once: the main fields (the value they share, or
/// "(different)"), the files that could not be read, the files' names
/// (a click shows that file).
fn body_all(ui: &mut Ui, form: &mut Form, paths: &[PathBuf], editing: bool, acts: &mut Vec<Act>) {
    ui.add_space(18.0);
    ui.spacing_mut().item_spacing.y = 6.0;
    let width = names_width(ui, main_names());
    section(ui, tr!("Main", "Основное"), |ui| {
        if !editing {
            let reading = form.state != State::Ready;
            edit_button(ui, reading.then_some(tr!("Reading…", "Чтение…")), acts);
        }
    });
    // Until the files are read, empty boxes of the same size.
    if editing && form.state != State::Reading {
        main_lines(ui, width, EDIT_PAD, &mut Mode::Edit(form));
    } else {
        main_lines(ui, width, EDIT_PAD, &mut Mode::All(form));
        errors(ui, &form.errors);
    }
    ui.add_space(14.0);
    section(ui, tr!("Files", "Файлы"), |_| {});
    for (i, path) in paths.iter().enumerate() {
        let text = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let resp = ui
            .add(
                egui::Label::new(RichText::new(text).color(palette::text()))
                    .truncate()
                    .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(path.display().to_string());
        if resp.clicked() {
            acts.push(Act::Go(Some(i)));
        }
    }
}

/// The names of the main fields, in `MAIN`'s order.
fn main_names() -> impl Iterator<Item = String> {
    MAIN.iter().map(|l| match l {
        Line::One(k) | Line::Two(k, _) => main_name(*k),
    })
}

fn main_name(key: edit::Key) -> String {
    field_name(key.items()[0])
}

/// A name beside a text field is lowered to the text in it, in points.
const EDIT_PAD: f32 = 3.0;

/// The main fields, `pad` the names lowered.
fn main_lines(ui: &mut Ui, width: f32, pad: f32, mode: &mut Mode) {
    let second = names_width(
        ui,
        MAIN.iter().filter_map(|l| match l {
            Line::Two(_, k) => Some(main_name(*k)),
            Line::One(_) => None,
        }),
    );
    for l in &MAIN {
        match l {
            Line::One(k) => line(ui, width, &main_name(*k), pad, |ui| {
                main_cell(ui, mode, *k, None);
            }),
            Line::Two(first, then) => line(ui, width, &main_name(*first), pad, |ui| {
                cell(ui, SHORT, |ui| main_cell(ui, mode, *first, Some(SHORT - 12.0)));
                cell(ui, second, |ui| {
                    ui.vertical(|ui| {
                        ui.add_space(pad);
                        name(ui, &main_name(*then));
                    });
                });
                main_cell(ui, mode, *then, None);
            }),
        }
    }
}

/// A main field's value, `width` points wide in a text field (else the
/// room left).
fn main_cell(ui: &mut Ui, mode: &mut Mode, key: edit::Key, width: Option<f32>) {
    let comment = key == edit::Key::Comment;
    let rows = comment.then_some(2);
    match mode {
        Mode::Read(d) => boxed(ui, value(d, key.items()), "", rows, width),
        Mode::All(form) => match form.main.get(key as usize) {
            Some(f) if f.mixed => boxed(ui, "", tr!("(different)", "(разные)"), rows, width),
            Some(f) => boxed(ui, &f.shown, "", rows, width),
            None => boxed(ui, "", "", rows, width),
        },
        Mode::Edit(form) => {
            // The first field takes the keys once the fields came.
            let focus = key == edit::Key::Title && std::mem::take(&mut form.focus_first);
            if let Some(f) = form.main.get_mut(key as usize) {
                let valid = !f.touched || key.takes(&f.value);
                text_field(ui, f, comment, valid, width, focus);
            }
        }
    }
}

/// A field in a text field: "(different)" as a hint while the files differ
/// in it and nothing was typed; red while not what the field takes.
fn text_field(
    ui: &mut Ui,
    f: &mut Field,
    multiline: bool,
    valid: bool,
    width: Option<f32>,
    focus: bool,
) {
    let hint = if f.mixed && !f.touched {
        tr!("(different)", "(разные)")
    } else {
        ""
    };
    let edit = if multiline {
        egui::TextEdit::multiline(&mut f.value).desired_rows(2)
    } else {
        egui::TextEdit::singleline(&mut f.value)
    };
    let mut edit = edit
        .hint_text(hint)
        .desired_width(width.unwrap_or(ui.available_width()));
    if !valid {
        edit = edit.text_color(palette::error());
    }
    let resp = ui.add(edit);
    // Typed back to what it was (a mixed field emptied again): left
    // alone, not written empty to every file.
    if resp.changed() {
        f.touched = f.value != f.shown;
    }
    if focus {
        resp.request_focus();
    }
    super::take_leaving_key(ui, &resp);
}

/// A value shown in a box as the text fields' while editing, but not
/// editable (its text can be selected and copied); `rows`: on as many
/// lines at least, a longer value wrapped, else on one line.
fn boxed(ui: &mut Ui, text: &str, hint: &str, rows: Option<usize>, width: Option<f32>) {
    let mut text = text;
    let edit = match rows {
        Some(rows) => egui::TextEdit::multiline(&mut text).desired_rows(rows),
        None => egui::TextEdit::singleline(&mut text),
    };
    // egui draws a field that cannot be edited without its ground: the
    // editable one's, painted under it.
    let ground = ui.painter().add(egui::Shape::Noop);
    let resp = ui.add(
        edit.hint_text(hint)
            .desired_width(width.unwrap_or(ui.available_width())),
    );
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = ui.visuals().extreme_bg_color;
    ui.painter()
        .set(ground, egui::epaint::RectShape::filled(resp.rect, radius, fill));
}

/// "Edit" in a section's heading, disabled with `why_not`.
fn edit_button(ui: &mut Ui, why_not: Option<&str>, acts: &mut Vec<Act>) {
    let resp = ui
        .add_enabled(
            why_not.is_none(),
            egui::Button::new(tr!("Edit", "Изменить")).small(),
        )
        .on_hover_text(crate::keymap::with_key(
            tr!("Edit the tags", "Изменить теги"),
            Cmd::EditTags,
        ))
        .on_disabled_hover_text(why_not.unwrap_or_default());
    if resp.clicked() {
        acts.push(Act::Edit);
    }
}

/// The files that could not be read or written, in red.
fn errors(ui: &mut Ui, errors: &[String]) {
    if errors.is_empty() {
        return;
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt("file_info_errors")
        .max_height(ERRORS)
        .show(ui, |ui| {
            for e in errors {
                ui.label(RichText::new(e).size(12.5).color(palette::error()));
            }
        });
}

/// Under the fields while editing: what could not be read or written,
/// "Writing…", Cancel and Save.
fn footer_row(ui: &mut Ui, form: Option<&Form>, acts: &mut Vec<Act>) {
    ui.add_space(6.0);
    if let Some(f) = form {
        errors(ui, &f.errors);
    }
    ui.add_space(6.0);
    let state = form.map(|f| &f.state);
    let can_save = form.is_some_and(|f| f.state == State::Ready && f.touched() && f.valid());
    ui.horizontal(|ui| {
        if state == Some(&State::Saving) {
            ui.spinner();
            ui.weak(tr!("Writing…", "Запись…"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(
                    can_save,
                    egui::Button::new(tr!("Save", "Сохранить")).min_size(vec2(100.0, 0.0)),
                )
                .clicked()
            {
                acts.push(Act::Save);
            }
            if ui
                .add_enabled(
                    state != Some(&State::Saving),
                    egui::Button::new(tr!("Cancel", "Отмена")).min_size(vec2(90.0, 0.0)),
                )
                .clicked()
            {
                acts.push(Act::Cancel);
            }
        });
    });
}

/// The lines of the section "File": the path, an MP3's LAME tag, the
/// pictures.
fn file_lines(d: &Details) -> Vec<(String, String)> {
    let mut out = vec![(tr!("Path", "Путь").to_owned(), d.path.display().to_string())];
    if let Some((delay, padding)) = d.gapless {
        out.push((
            tr!("LAME tag", "Тег LAME").to_owned(),
            tr!(
                format!("encoder delay {delay}, padding {padding} samples"),
                format!("задержка кодировщика {delay}, добивка {padding} сэмплов")
            ),
        ));
    }
    for p in &d.pictures {
        let mut text = p.format.clone();
        if let Some((w, h)) = p.pixels {
            text += &format!(" {w}×{h}");
        }
        text += &format!(", {}", format::size(p.bytes as u64));
        out.push((picture_name(p.kind), text));
    }
    out
}

/// A section's heading: small capitals, `right` (a note or a button) at
/// the right end, a line under them.
fn section(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .size(11.0)
                .strong()
                .color(palette::text_weak()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.left()..=r.right(),
        r.top() + 2.0,
        egui::Stroke::new(1.0, palette::line()),
    );
    ui.add_space(10.0);
}

/// The names' column: as wide as the widest of `names`, at most
/// `MAX_NAME` (measured, not left to a grid, which sizes its columns by
/// the frame before).
fn names_width(ui: &Ui, names: impl Iterator<Item = String>) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    names
        .map(|n| {
            ui.painter()
                .layout_no_wrap(n, font.clone(), palette::text())
                .size()
                .x
        })
        .fold(0.0, f32::max)
        .min(MAX_NAME)
        .ceil()
}

/// A line of a section: `label` in the names' column (`width`, lowered by
/// `pad` beside a text field), then `value` in the room right of it.
fn line(ui: &mut Ui, width: f32, label: &str, pad: f32, value: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        cell(ui, width, |ui| {
            ui.vertical(|ui| {
                ui.add_space(pad);
                name(ui, label);
            });
        });
        value(ui);
    });
}

/// A cell `width` points wide whatever it holds.
fn cell(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui)) {
    let size = vec2(width, ui.spacing().interact_size.y);
    let layout = egui::Layout::left_to_right(egui::Align::Min);
    ui.allocate_ui_with_layout(size, layout, |ui| {
        ui.set_width(width);
        add(ui);
    });
}

/// A field's name, in the weak colour; cut (whole in a tooltip) when
/// longer than its cell.
fn name(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(palette::text_weak())).truncate());
}

/// A field's value, a dash when empty; `wrap`: a long value takes more
/// lines, else it is cut.
fn shown_value(ui: &mut Ui, text: &str, wrap: bool) {
    if text.is_empty() {
        ui.label(RichText::new("—").color(palette::line()));
    } else {
        let label = egui::Label::new(RichText::new(text).color(palette::text()));
        ui.add(if wrap { label.wrap() } else { label.truncate() });
    }
}

/// The cover at `side` points with rounded corners, or a placeholder.
fn cover_box(ui: &mut Ui, cover: Option<&TextureHandle>, side: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), egui::Sense::hover());
    let painter = ui.painter();
    match cover {
        Some(t) => {
            let [w, h] = t.size();
            let scale = side / w.max(h) as f32;
            let r = egui::Rect::from_center_size(
                rect.center(),
                vec2(w as f32 * scale, h as f32 * scale),
            );
            painter.rect_filled(rect, 8.0, palette::list_bg());
            egui::Image::new(egui::load::SizedTexture::new(t.id(), r.size()))
                .corner_radius(8.0)
                .paint_at(ui, r);
        }
        None => crate::ui::no_cover(painter, rect, 8.0),
    }
}

/// A small rounded label with a fact (`320 kbps`).
fn chip(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(12.0),
        palette::text(),
    );
    let size = galley.size() + vec2(18.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, size.y / 2.0, palette::card_bg());
    ui.painter()
        .galley(rect.min + vec2(9.0, 4.0), galley, palette::text());
}

/// The chips of the hero: the format, the length, the bit rate, the
/// sample rate and depth, the channels, the size.
fn chips(d: &Details) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ext) = d.path.extension() {
        out.push(ext.to_string_lossy().to_uppercase());
    }
    out.push(format::time(d.duration_ms));
    if d.bitrate > 0 {
        out.push(format::bitrate(d.bitrate));
    }
    if d.sample_rate > 0 {
        out.push(format::sample_rate(d.sample_rate));
    }
    if let Some(bits) = d.bit_depth {
        out.push(tr!(format!("{bits} bit"), format!("{bits} бит")));
    }
    if let Some(c) = format::channels(d.channels) {
        out.push(c);
    }
    out.push(format::size(d.size));
    out
}

/// The value of the first of `keys` the tag has, else empty.
fn value<'a>(d: &'a Details, keys: &[ItemKey]) -> &'a str {
    keys.iter()
        .find_map(|k| d.fields.iter().find(|(f, _)| f == k))
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

/// A field's name as players and taggers show it; the others by
/// lofty's name for them, in words.
fn field_name(key: ItemKey) -> String {
    let name = match key {
        ItemKey::TrackNumber => tr!("Track #", "Трек №"),
        ItemKey::DiscNumber => tr!("Disc #", "Диск №"),
        ItemKey::TrackTitle => tr!("Title", "Название"),
        ItemKey::TrackArtist => tr!("Artist", "Исполнитель"),
        ItemKey::AlbumTitle => tr!("Album", "Альбом"),
        // ID3v2.3's TYER is read as ID3v2.4's TDRC.
        ItemKey::Year | ItemKey::RecordingDate => tr!("Year", "Год"),
        ItemKey::Genre => tr!("Genre", "Жанр"),
        ItemKey::Comment => tr!("Comment", "Комментарий"),
        ItemKey::AlbumArtist => tr!("Album artist", "Исп. альбома"),
        ItemKey::Composer => tr!("Composer", "Композитор"),
        ItemKey::Publisher | ItemKey::Label => tr!("Publisher", "Издатель"),
        ItemKey::OriginalArtist => tr!("Orig. artist", "Ориг. исп."),
        ItemKey::CopyrightMessage => tr!("Copyright", "Копирайт"),
        ItemKey::AudioFileUrl => "URL",
        ItemKey::EncodedBy => tr!("Encoded by", "Кодировщик"),
        ItemKey::EncoderSoftware => tr!("Encoder", "Программа"),
        ItemKey::Bpm | ItemKey::IntegerBpm => "BPM",
        ItemKey::ReplayGainTrackGain => tr!("Track gain", "Усил. трека"),
        ItemKey::ReplayGainAlbumGain => tr!("Album gain", "Усил. альбома"),
        ItemKey::ReplayGainTrackPeak => tr!("Track peak", "Пик трека"),
        ItemKey::ReplayGainAlbumPeak => tr!("Album peak", "Пик альбома"),
        ItemKey::Lyricist => tr!("Lyricist", "Автор текста"),
        ItemKey::Conductor => tr!("Conductor", "Дирижёр"),
        ItemKey::Lyrics | ItemKey::UnsyncLyrics => tr!("Lyrics", "Текст"),
        ItemKey::CommercialInformationUrl => tr!("Shop URL", "URL магазина"),
        ItemKey::CopyrightUrl => tr!("Copyright URL", "URL копирайта"),
        ItemKey::TrackArtistUrl => tr!("Artist URL", "URL исполнителя"),
        ItemKey::PublisherUrl => tr!("Publisher URL", "URL издателя"),
        ItemKey::AudioSourceUrl => tr!("Source URL", "URL источника"),
        ItemKey::RadioStationUrl => tr!("Station URL", "URL станции"),
        ItemKey::PaymentUrl => tr!("Payment URL", "URL оплаты"),
        ItemKey::EncoderSettings => tr!("Encoder settings", "Настройки кодировщика"),
        ItemKey::EncodingTime => tr!("Encoded on", "Дата кодирования"),
        ItemKey::TaggingTime => tr!("Tagged on", "Дата тега"),
        ItemKey::OriginalReleaseDate => tr!("Original release", "Первый выпуск"),
        ItemKey::ReleaseDate => tr!("Released", "Выпущен"),
        ItemKey::Isrc => "ISRC",
        ItemKey::CatalogNumber => tr!("Catalogue number", "Номер в каталоге"),
        ItemKey::InitialKey => tr!("Key", "Тональность"),
        ItemKey::Mood => tr!("Mood", "Настроение"),
        ItemKey::FlagCompilation => tr!("Compilation", "Сборник"),
        ItemKey::TrackSubtitle => tr!("Subtitle", "Подзаголовок"),
        ItemKey::SetSubtitle => tr!("Disc subtitle", "Название диска"),
        ItemKey::ContentGroup => tr!("Grouping", "Группа"),
        ItemKey::OriginalAlbumTitle => tr!("Original album", "Ориг. альбом"),
        ItemKey::OriginalLyricist => tr!("Original lyricist", "Ориг. автор текста"),
        ItemKey::Arranger => tr!("Arranger", "Аранжировщик"),
        ItemKey::Writer => tr!("Writer", "Автор"),
        ItemKey::Director => tr!("Director", "Режиссёр"),
        ItemKey::Engineer => tr!("Engineer", "Звукоинженер"),
        ItemKey::MixDj => tr!("Mix DJ", "Диджей сведения"),
        ItemKey::MixEngineer => tr!("Mix engineer", "Звукорежиссёр сведения"),
        ItemKey::Performer => tr!("Performer", "Исполнители"),
        ItemKey::Producer => tr!("Producer", "Продюсер"),
        ItemKey::Remixer => tr!("Remixer", "Ремикс"),
        ItemKey::Language => tr!("Language", "Язык"),
        ItemKey::Script => tr!("Script", "Письменность"),
        ItemKey::Barcode => tr!("Barcode", "Штрихкод"),
        ItemKey::Description => tr!("Description", "Описание"),
        ItemKey::AlbumTitleSortOrder => tr!("Album sort", "Сорт. альбома"),
        ItemKey::AlbumArtistSortOrder => tr!("Album artist sort", "Сорт. исп. альбома"),
        ItemKey::TrackTitleSortOrder => tr!("Title sort", "Сорт. названия"),
        ItemKey::TrackArtistSortOrder => tr!("Artist sort", "Сорт. исполнителя"),
        ItemKey::ComposerSortOrder => tr!("Composer sort", "Сорт. композитора"),
        ItemKey::Popularimeter => tr!("Rating", "Рейтинг"),
        // MusicBrainz, AcoustID and the like: names of services, the same
        // in either language.
        other => return words(&format!("{other:?}")),
    };
    name.to_owned()
}

/// `MusicBrainzTrackId` as `MusicBrainz Track Id`: a word before each
/// capital, except within the names of services.
fn words(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out.replace("Music Brainz", "MusicBrainz")
        .replace("Acoust Id", "AcoustID")
        .replace(" Id", " ID")
        .replace(" Url", " URL")
}

fn picture_name(kind: PictureType) -> String {
    let name = match kind {
        PictureType::CoverFront => tr!("Front cover", "Обложка"),
        PictureType::CoverBack => tr!("Back cover", "Задняя обложка"),
        PictureType::Leaflet => tr!("Leaflet", "Буклет"),
        PictureType::Media => tr!("Disc", "Диск"),
        PictureType::Artist | PictureType::LeadArtist => tr!("Artist", "Исполнитель"),
        PictureType::Band => tr!("Band", "Группа"),
        PictureType::Icon => tr!("Icon", "Значок"),
        PictureType::OtherIcon => tr!("Other icon", "Другой значок"),
        PictureType::Conductor => tr!("Conductor", "Дирижёр"),
        PictureType::Composer => tr!("Composer", "Композитор"),
        PictureType::Lyricist => tr!("Lyricist", "Автор текста"),
        PictureType::RecordingLocation => tr!("Recording location", "Место записи"),
        PictureType::DuringRecording => tr!("During recording", "Во время записи"),
        PictureType::DuringPerformance => tr!("During performance", "Во время выступления"),
        PictureType::ScreenCapture => tr!("Screen capture", "Кадр из видео"),
        PictureType::BrightFish => tr!("Bright fish", "Яркая рыбка"),
        PictureType::Illustration => tr!("Illustration", "Иллюстрация"),
        PictureType::BandLogo => tr!("Band logo", "Логотип группы"),
        PictureType::PublisherLogo => tr!("Publisher logo", "Логотип издателя"),
        _ => tr!("Picture", "Картинка"),
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_words() {
        assert_eq!(words("MusicBrainzTrackId"), "MusicBrainz Track ID");
        assert_eq!(words("AcoustIdFingerprint"), "AcoustID Fingerprint");
        assert_eq!(words("Barcode"), "Barcode");
    }
}
