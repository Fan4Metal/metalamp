//! The tag editor: the main fields of one or more files (the tracks'
//! table's selection or the playlist's, Ctrl+E or "Edit tags…" in their
//! menus). The files are read on a thread; a field the files share shows
//! its value, one that differs shows "(different)" and is left alone
//! unless typed in. Saving writes only the fields typed in (`library::
//! edit`, on a thread); a file that cannot be written (in use, a format
//! lofty does not write) is named under the fields and the others are
//! written. The library reads the files written again (`App`).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use egui::{Align, Key, Layout, RichText, Ui, vec2};

use crate::library::edit::{self, Changes, Values};
use crate::palette;

const WIDTH: f32 = 560.0;

#[derive(Default)]
pub struct TagEditor {
    pub open: bool,
    /// Opened this frame: the key that opened it must not act in it.
    fresh: bool,
    paths: Vec<PathBuf>,
    fields: Vec<Field>,
    state: State,
    rx: Option<Receiver<Msg>>,
    /// The files that could not be read or written, with why.
    errors: Vec<String>,
    /// The fields came: the first one takes the keys.
    focus_first: bool,
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

/// What the editor did this frame, for the app.
#[derive(Default)]
pub struct Done {
    /// Files were written: the library reads them again.
    pub written: bool,
}

impl TagEditor {
    /// Opens the editor for `paths`, read on a thread.
    pub fn open(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let to_read = paths.clone();
        std::thread::spawn(move || {
            let read = to_read
                .iter()
                .map(|p| {
                    crate::guard::catch(p, || edit::read(p))
                        .unwrap_or_else(|| Err("cannot be read".to_owned()))
                        .map_err(|e| format!("{}: {e}", p.display()))
                })
                .collect();
            let _ = tx.send(Msg::Read(read));
            ctx.request_repaint();
        });
        *self = TagEditor {
            open: true,
            fresh: true,
            paths,
            fields: Vec::new(),
            state: State::Reading,
            rx: Some(rx),
            errors: Vec::new(),
            focus_first: false,
        };
    }

    fn poll(&mut self) -> Done {
        let mut done = Done::default();
        let Some(rx) = &self.rx else { return done };
        for msg in rx.try_iter().collect::<Vec<_>>() {
            match msg {
                Msg::Read(read) => {
                    let ok: Vec<&Values> = read.iter().filter_map(|r| r.as_ref().ok()).collect();
                    self.errors = read.iter().filter_map(|r| r.clone().err()).collect();
                    self.fields = (0..edit::Key::ALL.len())
                        .map(|i| {
                            let first = ok.first().map(|v| v[i].clone()).unwrap_or_default();
                            let mixed = ok.iter().any(|v| v[i] != first);
                            let value = if mixed { String::new() } else { first };
                            Field {
                                shown: value.clone(),
                                value,
                                mixed,
                                touched: false,
                            }
                        })
                        .collect();
                    self.state = State::Ready;
                    self.focus_first = true;
                }
                Msg::Written(written, errors) => {
                    done.written = written > 0;
                    if errors.is_empty() {
                        self.open = false;
                    } else {
                        self.errors = errors;
                        self.state = State::Ready;
                    }
                }
            }
        }
        done
    }

    /// The fields typed in.
    fn changes(&self) -> Changes {
        let mut changes: Changes = Default::default();
        for (c, f) in changes.iter_mut().zip(&self.fields) {
            if f.touched {
                *c = Some(f.value.trim().to_owned());
            }
        }
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
                    .unwrap_or_else(|| Err("cannot be written".to_owned()))
                {
                    Ok(()) => written += 1,
                    Err(e) => errors.push(format!("{}: {e}", p.display())),
                }
            }
            let _ = tx.send(Msg::Written(written, errors));
            ctx.request_repaint();
        });
        self.rx = Some(rx);
        self.errors.clear();
        self.state = State::Saving;
    }

    /// Draws the editor while it is open.
    pub fn show(&mut self, ctx: &egui::Context) -> Done {
        let done = self.poll();
        if !self.open {
            return done;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let (mut close, mut save) = (false, false);
        let any_touched = self.fields.iter().any(|f| f.touched);
        let modal = egui::Modal::new(egui::Id::new("tag_editor"))
            .frame(
                egui::Frame::window(&ctx.style_of(ctx.theme()))
                    .fill(palette::panel_bg())
                    .inner_margin(egui::Margin::same(20))
                    .corner_radius(10.0),
            )
            .show(ctx, |ui| {
                crate::ui::settings::round_widgets(ui);
                ui.set_width(super::modal_size(ctx, vec2(WIDTH, 300.0), 12.0).x);
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr!("Tags", "Теги"))
                            .size(20.0)
                            .strong()
                            .color(palette::text_bright()),
                    );
                    ui.label(RichText::new(self.subtitle()).color(palette::text_weak()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close |= super::close_cross(ui);
                    });
                });
                ui.add_space(8.0);
                match self.state {
                    State::Reading => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.weak(tr!("Reading…", "Чтение…"));
                        });
                    }
                    State::Ready | State::Saving => {
                        let enabled = self.state == State::Ready;
                        ui.add_enabled_ui(enabled, |ui| self.fields_grid(ui));
                    }
                }
                if !self.errors.is_empty() {
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical()
                        .max_height(90.0)
                        .show(ui, |ui| {
                            for e in &self.errors {
                                ui.label(RichText::new(e).size(12.5).color(palette::error()));
                            }
                        });
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if self.state == State::Saving {
                        ui.spinner();
                        ui.weak(tr!("Writing…", "Запись…"));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        save |= ui
                            .add_enabled(
                                self.state == State::Ready && any_touched,
                                egui::Button::new(tr!("Save", "Сохранить"))
                                    .min_size(vec2(100.0, 0.0)),
                            )
                            .clicked();
                        close |= ui
                            .add(
                                egui::Button::new(tr!("Cancel", "Отмена"))
                                    .min_size(vec2(90.0, 0.0)),
                            )
                            .clicked();
                    });
                });
            });
        // Esc gives up; Enter (no field holding it) saves. A click outside
        // does not close: typing is not lost by a slip.
        if !fresh && modal.is_top_modal && !modal.any_popup_open {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                close = true;
            }
            if self.state == State::Ready
                && any_touched
                && ctx.memory(|m| m.focused().is_none())
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
            {
                save = true;
            }
        }
        if save && self.state == State::Ready {
            self.save(ctx);
        }
        // Closing while writing leaves the writing to finish.
        if close {
            self.open = false;
        }
        done
    }

    /// "1 file" or "3 files", for the title row.
    fn subtitle(&self) -> String {
        crate::i18n::count(
            self.paths.len() as u64,
            ["file", "files"],
            ["файл", "файла", "файлов"],
        )
    }

    fn fields_grid(&mut self, ui: &mut Ui) {
        let focus_first = std::mem::take(&mut self.focus_first);
        egui::Grid::new("tag_fields")
            .num_columns(2)
            .spacing(vec2(12.0, 6.0))
            .show(ui, |ui| {
                for (i, key) in edit::Key::ALL.iter().enumerate() {
                    let Some(field) = self.fields.get_mut(i) else {
                        continue;
                    };
                    ui.label(key.label());
                    let hint = if field.mixed && !field.touched {
                        tr!("(different)", "(разные)")
                    } else {
                        ""
                    };
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut field.value)
                            .hint_text(hint)
                            .desired_width(ui.available_width().max(200.0)),
                    );
                    // Typed back to what it was (a mixed field emptied
                    // again): left alone, not written empty to every file.
                    if resp.changed() {
                        field.touched = field.value != field.shown;
                    }
                    if focus_first && i == 0 {
                        resp.request_focus();
                    }
                    super::take_leaving_key(ui, &resp);
                    ui.end_row();
                }
            });
    }
}
