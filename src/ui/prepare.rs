//! The album's preparation window ("Prepare the album…" in an album's
//! menu, "Prepare an album from a folder…" in the folders' menu): the
//! preset in use, and what it would do as a table of each file's fields
//! before and after (`library::prepare::plan`, worked out again at every
//! change of the preset), Apply at the bottom, then what was done. The
//! presets are edited in a window of their own (`PresetEditor`): a list of
//! them and their settings in tabs (tags, cover, names, folder).
//!
//! The files are read and written on threads of their own; the app stops
//! the player first when a file of the album plays and is to be renamed,
//! and afterwards moves the plays, ratings, loudness and playlist entries
//! to the new paths and reads the folders again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};

use egui::{Align, Key, Layout, RichText, Ui, vec2};

use crate::library::edit::{self, Item, TagPart};
use crate::library::prepare::{
    self, CoverFrom, DiscFormat, Extras, Fill, FolderJpg, GenreChoice, Kind, Known, Made, MaskChoice,
    Need, Numbering, Outcome, Plan, Preset, Profile, Source, TagCover, ThisAlbum, TrackFormat, Unify,
};
use crate::palette;
use crate::settings::{CoverSide, Settings};
use crate::ui::settings::{divider, round_widgets, row, section, switch, tall_row};

/// The window's size, when the program's window has room for it.
const SIZE: egui::Vec2 = vec2(980.0, 700.0);
/// The table's first column: the field's name.
const NAME: f32 = 150.0;
/// The preset editor's size.
const EDITOR: egui::Vec2 = vec2(900.0, 640.0);
/// The editor's list of presets.
const LIST: f32 = 200.0;

/// The preset in use: the one of its name, else the first, else the
/// default.
pub fn profile_in_use(settings: &Settings) -> Profile {
    settings
        .prepare_presets
        .iter()
        .find(|p| p.name == settings.prepare_preset)
        .or_else(|| settings.prepare_presets.first())
        .map(|p| p.profile.clone())
        .unwrap_or_default()
}

#[derive(Default)]
pub struct PrepareWindow {
    pub open: bool,
    /// The album's name, for the heading.
    title: String,
    reading: Option<Receiver<Source>>,
    source: Option<Arc<Source>>,
    /// What the library knows, given to the source read.
    known: Known,
    /// The album's genre and masks this time.
    this: ThisAlbum,
    /// The genre typed for this album.
    own_genre: String,
    /// The masks typed for this album.
    own_file_mask: String,
    own_folder_mask: String,
    /// The plan for the profile and the choices it was made of.
    plan: Option<(Profile, ThisAlbum, Arc<Plan>)>,
    /// The covers made ready for the tags, by candidate and side.
    made: HashMap<(usize, Option<u32>), Option<Made>>,
    applying: Option<Applying>,
    outcome: Option<Outcome>,
    /// Apply was clicked: the app starts it (`start`).
    start_asked: bool,
    /// The files that change alone.
    only_changes: bool,
    editor: PresetEditor,
}

struct Applying {
    done: Arc<AtomicUsize>,
    total: usize,
    answer: Receiver<Outcome>,
}

/// What the window did this frame, for the app.
#[derive(Default)]
pub struct Shown {
    /// The presets changed: the settings are saved.
    pub changed: bool,
    /// The preparation ended.
    pub done: Option<Outcome>,
}

impl PrepareWindow {
    /// Opens the window for the files at `paths` (an album's, or a
    /// folder's audio) and reads them; `known` what the library knows.
    pub fn open(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, title: String, known: Known) {
        if paths.is_empty() || self.applying.is_some() {
            return;
        }
        *self = PrepareWindow {
            open: true,
            title,
            known,
            only_changes: self.only_changes,
            ..PrepareWindow::default()
        };
        // `METALAMP_PREPARE_TAB=tags|read|genre|cover|names|folder`: the presets'
        // editor on that tab (GUI checks).
        if let Ok(tab) = std::env::var("METALAMP_PREPARE_TAB") {
            self.editor.open = true;
            self.editor.tab = match tab.as_str() {
                "genre" => Tab::Genre,
                "read" => Tab::Read,
                "cover" => Tab::Cover,
                "names" => Tab::Names,
                "folder" => Tab::Folder,
                _ => Tab::Tags,
            };
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("prepare read".into())
            .spawn(move || {
                let _ = tx.send(prepare::read(paths));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.reading = Some(rx),
            Err(e) => log::error!("cannot start the preparation's thread: {e}"),
        }
    }

    /// Apply was clicked: the plan to be done, for the app to see whether
    /// the player must let go of the files first; asked once.
    pub fn take_start(&mut self) -> Option<Arc<Plan>> {
        if !std::mem::take(&mut self.start_asked) {
            return None;
        }
        self.plan.as_ref().map(|(_, _, plan)| plan.clone())
    }

    /// Does the plan on a thread.
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        profile: Profile,
        how: edit::WriteTags,
        releaser: crate::audio::Releaser,
    ) {
        let Some((_, _, plan)) = self.plan.clone() else {
            return;
        };
        let done = Arc::new(AtomicUsize::new(0));
        let total = plan.files.len() + plan.renames();
        let (tx, rx) = channel();
        let counter = done.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("prepare".into())
            .spawn(move || {
                let outcome = prepare::apply(&plan, &profile, how, Some(&releaser), &counter);
                let _ = tx.send(outcome);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => {
                self.applying = Some(Applying {
                    done,
                    total,
                    answer: rx,
                })
            }
            Err(e) => log::error!("cannot start the preparation's thread: {e}"),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) -> Shown {
        let mut shown = Shown::default();
        if let Some(rx) = &self.reading
            && let Ok(mut source) = rx.try_recv()
        {
            source.known = std::mem::take(&mut self.known);
            self.source = Some(Arc::new(source));
            self.made.clear();
            self.reading = None;
        }
        if let Some(applying) = &self.applying {
            if let Ok(outcome) = applying.answer.try_recv() {
                shown.done = Some(outcome.clone());
                self.outcome = Some(outcome);
                self.applying = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        if !self.open {
            return shown;
        }
        if settings.prepare_presets.is_empty() {
            settings.prepare_presets = prepare::first_presets();
            shown.changed = true;
        }
        // A preset keeping the words by default of before gets today's.
        for preset in &mut settings.prepare_presets {
            if preset.profile.editions == prepare::EDITIONS_BEFORE {
                preset.profile.editions = prepare::EDITIONS.to_owned();
                shown.changed = true;
            }
        }
        if !settings.prepare_presets.iter().any(|p| p.name == settings.prepare_preset) {
            settings.prepare_preset = settings.prepare_presets[0].name.clone();
            shown.changed = true;
        }
        let profile = profile_in_use(settings);
        if let Some(source) = &self.source
            && self.outcome.is_none()
            && self
                .plan
                .as_ref()
                .is_none_or(|(p, t, _)| *p != profile || *t != self.this)
        {
            let made = &mut self.made;
            let mut make = |i: usize, side: Option<u32>| {
                made.entry((i, side))
                    .or_insert_with(|| prepare::make_cover(&source.candidates[i].bytes, side))
                    .clone()
            };
            let plan = prepare::plan(source, &profile, &self.this, &mut make);
            self.plan = Some((profile, self.this.clone(), Arc::new(plan)));
        }

        let busy = self.applying.is_some();
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("prepare_album"))
            .frame(
                egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
                    .fill(palette::panel_bg())
                    .inner_margin(egui::Margin::same(18))
                    .corner_radius(10.0),
            )
            .show(ctx, |ui| {
                round_widgets(ui);
                let size = super::modal_size(ctx, SIZE, 12.0);
                ui.set_width(size.x);
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr!("Prepare the album", "Подготовка альбома"))
                            .size(20.0)
                            .strong()
                            .color(palette::text_bright()),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(&self.title).size(15.0).color(palette::text_weak()))
                            .truncate(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !busy {
                            close |= super::close_cross(ui);
                        }
                    });
                });
                ui.add_space(4.0);
                // Under the heading: the preset's, the album's, the genre's
                // and the two masks' rows.
                let body = (size.y - 250.0).max(200.0);
                if let Some(outcome) = &self.outcome {
                    report(ui, outcome, body);
                } else {
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr!("Preset", "Пресет"));
                            let names: Vec<String> =
                                settings.prepare_presets.iter().map(|p| p.name.clone()).collect();
                            egui::ComboBox::from_id_salt("prepare_preset")
                                .width(220.0)
                                .selected_text(settings.prepare_preset.clone())
                                .show_ui(ui, |ui| {
                                    for name in names {
                                        if ui
                                            .selectable_label(settings.prepare_preset == name, &name)
                                            .clicked()
                                        {
                                            settings.prepare_preset = name;
                                            shown.changed = true;
                                        }
                                    }
                                });
                            if ui.button(tr!("Edit the presets…", "Настроить пресеты…")).clicked() {
                                self.editor.open = true;
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.checkbox(
                                    &mut self.only_changes,
                                    tr!("Only the files that change", "Только изменяемые файлы"),
                                );
                            });
                        });
                    });
                    ui.add_enabled_ui(!busy, |ui| {
                        self.album_row(ui);
                        self.genre_row(ui);
                        self.mask_row(ui, false);
                        self.mask_row(ui, true);
                    });
                    ui.add_space(4.0);
                    match (&self.plan, &self.source) {
                        (Some((_, _, plan)), Some(_)) => {
                            preview(ui, plan, body - 30.0, self.only_changes)
                        }
                        _ => {
                            ui.allocate_ui(vec2(ui.available_width(), body), |ui| {
                                ui.set_min_height(body);
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(tr!("Reading the tags…", "Чтение тегов…"));
                                });
                            });
                        }
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(applying) = &self.applying {
                        ui.spinner();
                        let done = applying.done.load(Ordering::Relaxed).min(applying.total);
                        ui.label(tr!(
                            format!("Working… {done} of {}", applying.total),
                            format!("Выполняется… {done} из {}", applying.total)
                        ));
                    } else if self.outcome.is_none()
                        && let Some((_, _, plan)) = &self.plan
                    {
                        ui.label(RichText::new(summary(plan)).color(palette::text_weak()));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(outcome) = &self.outcome {
                            close |= ui
                                .add(egui::Button::new(tr!("Close", "Закрыть")).min_size(vec2(90.0, 0.0)))
                                .clicked();
                            let folder = outcome
                                .folder
                                .as_ref()
                                .map(|(_, new)| new.clone())
                                .or_else(|| self.plan.as_ref().map(|(_, _, p)| p.folder.clone()));
                            if let Some(folder) = folder
                                && ui.button(tr!("Open the folder", "Открыть папку")).clicked()
                            {
                                crate::win::shell_open(&folder);
                            }
                        } else {
                            let ready = !busy && self.plan.as_ref().is_some_and(|(_, _, p)| !p.is_empty());
                            if ui
                                .add_enabled(
                                    ready,
                                    egui::Button::new(RichText::new(tr!("Apply", "Применить")).strong())
                                        .min_size(vec2(110.0, 0.0)),
                                )
                                .clicked()
                            {
                                self.start_asked = true;
                            }
                            close |= ui
                                .add_enabled(
                                    !busy,
                                    egui::Button::new(tr!("Cancel", "Отмена")).min_size(vec2(90.0, 0.0)),
                                )
                                .clicked();
                        }
                    });
                });
            });
        let sample = self.plan.as_ref().map(|(_, _, p)| p.clone());
        shown.changed |= self.editor.show(ctx, settings, sample.as_deref());
        if !busy
            && !self.editor.open
            && modal.is_top_modal
            && !modal.any_popup_open
            && ctx.memory(|m| m.focused().is_none())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            close = true;
        }
        if close && !busy {
            self.open = false;
            self.editor.open = false;
            self.source = None;
            self.plan = None;
            self.made.clear();
        }
        shown
    }

    /// The album's genre this time: as the preset says, as it is, or one's
    /// own (typed, or chosen among the library's); then what most files
    /// have now and will have.
    fn genre_row(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(tr!("Genre", "Жанр"));
            let mode = match &self.this.genre {
                GenreChoice::Preset => 0,
                GenreChoice::Keep => 1,
                GenreChoice::Own(_) => 2,
            };
            if ui
                .selectable_label(mode == 0, tr!("By the preset", "По пресету"))
                .clicked()
            {
                self.this.genre = GenreChoice::Preset;
            }
            if ui.selectable_label(mode == 1, tr!("As it is", "Как есть")).clicked() {
                self.this.genre = GenreChoice::Keep;
            }
            if ui.selectable_label(mode == 2, tr!("Own", "Свой")).clicked() {
                if self.own_genre.is_empty()
                    && let Some((_, _, plan)) = &self.plan
                {
                    self.own_genre = plan.genre.1.clone();
                }
                self.this.genre = GenreChoice::Own(self.own_genre.clone());
            }
            if mode == 2 {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.own_genre)
                        .desired_width(260.0)
                        .hint_text(tr!("the album's genre", "жанр альбома")),
                );
                let genres = self.source.as_ref().map(|s| s.known.genres.clone()).unwrap_or_default();
                // The genres like what the field holds (each of its parts:
                // "Acoustic / Melodic Modern Metal" is two), then all of
                // them: the whole text matched none, and the menu was empty.
                let parts: Vec<String> = self
                    .own_genre
                    .split([';', ',', '/', '|'])
                    .map(|p| p.trim().to_lowercase())
                    .filter(|p| !p.is_empty())
                    .collect();
                let like: Vec<&String> = genres
                    .iter()
                    .filter(|g| {
                        let g = g.to_lowercase();
                        parts.iter().any(|p| g.contains(p.as_str()) || p.contains(g.as_str()))
                    })
                    .collect();
                let mut chosen = None;
                ui.menu_button(tr!("Choose…", "Выбрать…"), |ui| {
                    ui.set_min_width(220.0);
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        let mut item = |ui: &mut Ui, g: &String| {
                            if ui.button(g).clicked() {
                                chosen = Some(g.clone());
                                ui.close();
                            }
                        };
                        if !like.is_empty() {
                            ui.label(
                                RichText::new(tr!("Like the field", "Похожие"))
                                    .size(12.0)
                                    .color(palette::text_weak()),
                            );
                            for g in &like {
                                item(ui, g);
                            }
                            ui.separator();
                            ui.label(
                                RichText::new(tr!("All the genres", "Все жанры"))
                                    .size(12.0)
                                    .color(palette::text_weak()),
                            );
                        }
                        for g in &genres {
                            item(ui, g);
                        }
                    });
                })
                .response
                .on_hover_text(tr!("The library's genres", "Жанры библиотеки"));
                // Only what is typed or chosen sets one's own genre: a click
                // on "By the preset" or "As it is" in this frame stands.
                let picked = chosen.is_some();
                if let Some(g) = chosen {
                    self.own_genre = g;
                }
                if field.changed() || picked {
                    self.this.genre = GenreChoice::Own(self.own_genre.clone());
                }
            }
            if let Some((_, _, plan)) = &self.plan {
                let (before, after) = &plan.genre;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let colour = if before == after {
                        palette::text_weak()
                    } else {
                        palette::accent()
                    };
                    let dash = |t: &str| if t.is_empty() { "—".to_owned() } else { t.to_owned() };
                    ui.add(egui::Label::new(RichText::new(dash(after)).color(colour)).truncate());
                    ui.label(RichText::new("»").color(palette::text_weak()));
                    ui.add(
                        egui::Label::new(RichText::new(dash(before)).color(palette::text()))
                            .truncate(),
                    );
                });
            }
        });
    }
}

impl PrepareWindow {
    /// The album, its artist and year: what most files will have, and a
    /// value typed goes into every file (in the accent, with a × to go
    /// back to what the preset makes).
    fn album_row(&mut self, ui: &mut Ui) {
        let Some(plan) = self.plan.as_ref().map(|(_, _, p)| p.clone()) else {
            return;
        };
        ui.horizontal(|ui| {
            let fields = [
                (edit::Key::Album, plan.album.as_str(), 0.45),
                (edit::Key::AlbumArtist, plan.album_artist.as_str(), 0.35),
                (edit::Key::Year, plan.year.as_str(), 0.0),
            ];
            // The fields share what the labels and the year leave.
            let room = (ui.available_width() - 420.0).max(200.0);
            for (key, planned, share) in fields {
                ui.label(key.label());
                let Some(slot) = self.this.own_mut(key) else {
                    continue;
                };
                let typed = slot.is_some();
                let mut text = slot.clone().unwrap_or_else(|| planned.to_owned());
                let width = if share > 0.0 { room * share } else { 50.0 };
                let mut field = egui::TextEdit::singleline(&mut text).desired_width(width);
                if typed {
                    field = field.text_color(palette::accent());
                }
                let resp = ui.add(field).on_hover_text(tr!(
                    "What most files will have; a value typed goes into every file",
                    "Что будет у большинства файлов; введённое значение ставится во все файлы"
                ));
                if resp.changed() {
                    *slot = Some(text);
                }
                if typed
                    && ui
                        .small_button("×")
                        .on_hover_text(tr!("As the preset makes it", "Как получается по пресету"))
                        .clicked()
                {
                    *slot = None;
                }
                ui.add_space(6.0);
            }
        });
    }

    /// The masks the files' names (or the folder's) are read by this time:
    /// the preset's, none, or one's own; then what they read.
    fn mask_row(&mut self, ui: &mut Ui, folder: bool) {
        let (choice, own) = if folder {
            (&mut self.this.folder_mask, &mut self.own_folder_mask)
        } else {
            (&mut self.this.file_mask, &mut self.own_file_mask)
        };
        let plan = self.plan.as_ref().map(|(_, _, p)| p.clone());
        ui.horizontal(|ui| {
            let label = if folder {
                tr!("Folder's name", "Имя папки")
            } else {
                tr!("Files' names", "Имена файлов")
            };
            ui.label(label).on_hover_text(tr!(
                "Empty fields read from the names by masks",
                "Пустые поля берутся из имён по маскам"
            ));
            let mode = match choice {
                MaskChoice::Preset => 0,
                MaskChoice::Off => 1,
                MaskChoice::Own(_) => 2,
            };
            if ui.selectable_label(mode == 0, tr!("By the preset", "По пресету")).clicked() {
                *choice = MaskChoice::Preset;
            }
            if ui.selectable_label(mode == 1, tr!("Not read", "Не брать")).clicked() {
                *choice = MaskChoice::Off;
            }
            if ui.selectable_label(mode == 2, tr!("Own mask", "Своя маска")).clicked() {
                if own.is_empty() {
                    let used = plan.as_ref().and_then(|p| {
                        if folder {
                            p.folder_mask.clone()
                        } else {
                            p.file_masks.first().map(|(m, _)| m.clone())
                        }
                    });
                    *own = used.unwrap_or_else(|| {
                        if folder { prepare::FOLDER_MASKS[0] } else { prepare::FILE_MASKS[0] }.to_owned()
                    });
                }
                *choice = MaskChoice::Own(own.clone());
            }
            if mode == 2 {
                let field = ui.add(egui::TextEdit::singleline(own).desired_width(220.0));
                // Only what is typed or added sets the mask: a click on the
                // other two in this frame stands.
                let added = mask_fields_menu(ui, own);
                if field.changed() || added {
                    *choice = MaskChoice::Own(own.clone());
                }
            }
            if let Some(plan) = &plan {
                let (text, colour) = mask_status(plan, folder);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(text).color(colour)).truncate());
                });
            }
        });
    }
}

/// The "+" menu of a mask's fields; true when one was added.
fn mask_fields_menu(ui: &mut Ui, text: &mut String) -> bool {
    let mut added = false;
    ui.menu_button("+", |ui| {
        for field in prepare::MASK_FIELDS {
            let name = crate::title_format::field_label(field).unwrap_or(tr!("Skip a part", "Пропустить часть"));
            if ui.button(format!("{name}  %{field}%")).clicked() {
                text.push_str(&format!("%{field}%"));
                added = true;
                ui.close();
            }
        }
    })
    .response
    .on_hover_text(tr!("Add a field", "Добавить поле"));
    added
}

/// What the masks read, as the row of the files' or the folder's names
/// says it.
fn mask_status(plan: &Plan, folder: bool) -> (String, egui::Color32) {
    let weak = palette::text_weak();
    let quoted = |m: &str| tr!(format!("\"{m}\""), format!("«{m}»"));
    if folder {
        if !plan.folder_masks_tried {
            return (tr!("not read", "не читается").to_owned(), weak);
        }
        let Some(mask) = &plan.folder_mask else {
            return (tr!("no mask fits", "ни одна маска не подходит").to_owned(), palette::error());
        };
        if plan.from_folder.is_empty() {
            return (
                tr!(
                    format!("{}: the fields are filled", quoted(mask)),
                    format!("{}: поля уже заполнены", quoted(mask))
                ),
                weak,
            );
        }
        let names: Vec<&str> = plan.from_folder.iter().map(|k| k.label()).collect();
        return (format!("{}: {}", quoted(mask), names.join(", ")), palette::accent());
    }
    if !plan.file_masks_tried {
        return (tr!("not read", "не читаются").to_owned(), weak);
    }
    let read: usize = plan.file_masks.iter().map(|(_, n)| n).sum();
    if read == 0 {
        return (tr!("no mask fits", "ни одна маска не подходит").to_owned(), palette::error());
    }
    let used: Vec<String> = plan
        .file_masks
        .iter()
        .map(|(m, n)| format!("{} {n}", quoted(m)))
        .collect();
    let filled = plan.files.iter().filter(|f| !f.from_name.is_empty()).count();
    let mut text = used.join(", ");
    if read < plan.files.len() {
        text.push_str(&tr!(
            format!(", {} fit none", plan.files.len() - read),
            format!(", не подходят: {}", plan.files.len() - read)
        ));
    }
    let colour = if filled > 0 { palette::accent() } else { weak };
    if filled == 0 {
        text.push_str(tr!(" (the fields are filled)", " (поля уже заполнены)"));
    }
    (text, colour)
}

/// Where a field's new value was read from, for its row's tooltip.
fn read_hint(plan: &Plan, f: &prepare::FilePlan, key: edit::Key) -> Option<String> {
    if plan.own.contains(&key) {
        return Some(tr!("Typed for this album", "Задано для этого альбома").to_owned());
    }
    if key == edit::Key::Track && f.numbered {
        return Some(
            tr!(
                "Its place in the order: by the disc, then the file's name",
                "Место по порядку: по диску, затем по имени файла"
            )
            .to_owned(),
        );
    }
    if f.from_name.contains(&key) {
        let mask = f.mask.as_deref().unwrap_or("");
        return Some(tr!(
            format!("Read from the file's name by the mask \"{mask}\""),
            format!("Взято из имени файла по маске «{mask}»")
        ));
    }
    if plan.from_folder.contains(&key) {
        let mask = plan.folder_mask.as_deref().unwrap_or("");
        return Some(tr!(
            format!("Read from the folder's name by the mask \"{mask}\" (where the field was empty)"),
            format!("Взято из имени папки по маске «{mask}» (там, где поле было пустым)")
        ));
    }
    None
}

/// The line under the table: what the preset would do.
fn summary(plan: &Plan) -> String {
    if plan.is_empty() {
        return tr!(
            "Nothing to do: the album is as the preset wants it.",
            "Делать нечего: альбом уже такой, как требует пресет."
        )
        .to_owned();
    }
    let mut parts = vec![tr!(
        format!("tags of {} of {} files", plan.writes(), plan.files.len()),
        format!("теги {} из {} файлов", plan.writes(), plan.files.len())
    )];
    if plan.renames() > 0 {
        parts.push(tr!(
            format!("{} renamed", plan.renames()),
            format!("переименований: {}", plan.renames())
        ));
    }
    if plan.new_folder.is_some() {
        parts.push(tr!("the folder renamed", "папка переименовывается").to_owned());
    }
    if plan.folder_jpg.is_some() {
        parts.push("folder.jpg".to_owned());
    }
    if !plan.extras.is_empty() {
        parts.push(tr!(
            format!("{} other files", plan.extras.len()),
            format!("прочих файлов: {}", plan.extras.len())
        ));
    }
    parts.join(" · ")
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// A picture's size and bytes as the table shows them.
fn picture(size: (u32, u32), bytes: usize) -> String {
    let kb = bytes.div_ceil(1024);
    tr!(
        format!("{}×{}, {kb} KB", size.0, size.1),
        format!("{}×{}, {kb} КБ", size.0, size.1)
    )
}

/// The table's columns: the field's name, before, after.
fn widths(ui: &Ui) -> [f32; 3] {
    let rest = ((ui.available_width() - NAME - 16.0) / 2.0).max(80.0);
    [NAME, rest, rest]
}

/// A row of the table: its three cells, and a tooltip for its name (the
/// tag's own code of a field removed).
struct Row {
    cells: [RichText; 3],
    hint: Option<String>,
}

impl From<[RichText; 3]> for Row {
    fn from(cells: [RichText; 3]) -> Row {
        Row { cells, hint: None }
    }
}

/// A row of the table: each cell cut with "…" to its column (whole in
/// the tooltip); `hint` the tooltip of the name and the new value.
fn table_row(ui: &mut Ui, widths: [f32; 3], cells: [RichText; 3], hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (i, (w, text)) in widths.into_iter().zip(cells).enumerate() {
            ui.allocate_ui_with_layout(vec2(w, 18.0), Layout::left_to_right(Align::Center), |ui| {
                ui.set_width(w);
                let resp = ui.add(egui::Label::new(text).truncate());
                if i != 1
                    && let Some(hint) = hint
                {
                    resp.on_hover_text(hint);
                }
            });
        }
    });
}

/// A field of the tag removed: its name in words (the file info's), its
/// value before, "removed" after; the tag's own code as the tooltip.
fn removed_row(item: &Item) -> Row {
    let name = match (item.key, item.code.split_once(':')) {
        (Some(key), _) => super::info::field_name(key),
        (None, Some(("TXXX" | "WXXX", description))) => description.to_owned(),
        (None, _) if item.part == TagPart::Pictures => tr!(
            format!("Picture: {}", item.code),
            format!("Картинка: {}", item.code)
        ),
        (None, _) => tr!(
            format!("Private data ({})", item.code),
            format!("Служебные данные ({})", item.code)
        ),
    };
    let before = if item.value.is_empty() {
        RichText::new(tr!("(empty)", "(пусто)")).color(palette::text_weak())
    } else {
        RichText::new(item.value.replace(['\r', '\n'], " ")).color(palette::text())
    };
    let hint = (name != item.code).then(|| item.code.clone());
    Row {
        cells: [
            RichText::new(name).color(palette::text_weak()),
            before,
            RichText::new(tr!("removed", "удаляется")).color(palette::text_weak()),
        ],
        hint,
    }
}

/// Why the album's title lost its tail: each part cut and the word that
/// named it; none when nothing was cut.
fn cut_hint(plan: &Plan) -> Option<String> {
    if plan.album_cuts.is_empty() {
        return None;
    }
    let parts: Vec<String> = plan
        .album_cuts
        .iter()
        .rev()
        .map(|c| match &c.word {
            Some(w) => tr!(
                format!("{} by the word \"{w}\"", c.part),
                format!("{} — по слову «{w}»", c.part)
            ),
            None => tr!(
                format!("{}: any part in brackets at the end", c.part),
                format!("{} — любая часть в скобках в конце", c.part)
            ),
        })
        .collect();
    Some(tr!(
        format!("Cut from the album's title (the album title without the edition):\n{}", parts.join("\n")),
        format!("Убрано из названия альбома (пункт «Альбом без пометки издания»):\n{}", parts.join("\n"))
    ))
}

/// A changed value: the old one plain, the new one in the accent; a dash
/// for an empty one.
fn change(name: &str, before: &str, after: &str) -> Row {
    let value = |text: &str, colour| {
        if text.is_empty() {
            RichText::new("—").color(palette::text_weak())
        } else {
            RichText::new(text).color(colour)
        }
    };
    Row::from([
        RichText::new(name).color(palette::text_weak()),
        value(before, palette::text()),
        value(after, palette::accent()),
    ])
}

/// A row of what stays as it is.
fn same(name: &str, now: &str) -> Row {
    Row::from([
        RichText::new(name).color(palette::text_weak()),
        RichText::new(now).color(palette::text()),
        RichText::new(tr!("as it is", "без изменений")).color(palette::text_weak()),
    ])
}

/// A group of the table: its heading (a name and a note on the right),
/// then its rows.
fn group(ui: &mut Ui, heading: &str, note: RichText, rows: Vec<Row>) {
    egui::Frame::new()
        .fill(palette::card_bg())
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let note_width = 240.0;
                ui.allocate_ui_with_layout(
                    vec2((ui.available_width() - note_width).max(80.0), 20.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(heading).strong().color(palette::text_bright()))
                                .truncate(),
                        );
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(egui::Label::new(note.size(12.5)).truncate());
                });
            });
            if !rows.is_empty() {
                ui.add_space(2.0);
                let w = widths(ui);
                for row in rows {
                    table_row(ui, w, row.cells, row.hint.as_deref());
                }
            }
        });
    ui.add_space(4.0);
}

/// What the plan would do: the warnings, the album (folder, cover, other
/// files), then each file, as the fields before and after.
fn preview(ui: &mut Ui, plan: &Plan, height: f32, only_changes: bool) {
    for w in &plan.warnings {
        ui.label(RichText::new(w).size(12.5).color(palette::error()));
    }
    // The columns' names, over the table.
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let w = widths(ui);
        let head = |t: &str| RichText::new(t).size(12.0).strong().color(palette::text_weak());
        table_row(
            ui,
            [w[0], w[1], w[2] - 10.0],
            [
                head(tr!("Field", "Поле")),
                head(tr!("Before", "Было")),
                head(tr!("After", "Станет")),
            ],
            None,
        );
    });
    let height = (height - 24.0 - 18.0 * plan.warnings.len() as f32).max(120.0);
    egui::ScrollArea::vertical()
        .id_salt("prepare_preview")
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // The album.
            let mut rows = Vec::new();
            let folder = name_of(&plan.folder);
            match &plan.new_folder {
                Some(to) => {
                    // Made anew from the tags by the template: the old name
                    // is not cut, it is replaced.
                    let made = tr!(
                        format!("Made from the tags by the template \"{}\"", plan.folder_template),
                        format!("Собрано из тегов по шаблону «{}»", plan.folder_template)
                    );
                    let hint = match cut_hint(plan) {
                        Some(cut) => format!("{made}\n\n{cut}"),
                        None => made,
                    };
                    let mut row = change(tr!("Folder", "Папка"), &folder, &name_of(to));
                    row.hint = Some(hint);
                    rows.push(row);
                }
                None => rows.push(same(tr!("Folder", "Папка"), &folder)),
            }
            if let Some(cover) = &plan.cover {
                let from = match cover.kind {
                    Kind::Tag => tr!("the tags", "из тегов").to_owned(),
                    _ => name_of(&cover.from),
                };
                let made = picture(cover.made.size, cover.made.data.len());
                rows.push(Row::from([
                    RichText::new(tr!("Album cover", "Обложка альбома")).color(palette::text_weak()),
                    RichText::new(format!("{from}, {}×{}", cover.size.0, cover.size.1)).color(palette::text()),
                    RichText::new(tr!(format!("for the tags: {made}"), format!("для тегов: {made}")))
                        .color(palette::text()),
                ]));
            }
            let jpg_now = plan
                .folder_jpg_now
                .map(|(size, bytes)| picture(size, bytes));
            match (&plan.folder_jpg, &jpg_now) {
                (Some(bytes), now) => rows.push(change(
                    "folder.jpg",
                    now.as_deref().unwrap_or(""),
                    &picture(prepare::picture_size(bytes).unwrap_or_default(), bytes.len()),
                )),
                (None, Some(now)) => rows.push(same("folder.jpg", now)),
                (None, None) => {}
            }
            if !plan.extras.is_empty() {
                let names: Vec<String> = plan.extras.iter().map(|p| name_of(p)).collect();
                rows.push(change(
                    tr!("Other files", "Прочие файлы"),
                    &names.join(", "),
                    tr!("removed", "убираются"),
                ));
            }
            group(
                ui,
                tr!("Album", "Альбом"),
                RichText::new(tr!(
                    format!("{} files", plan.files.len()),
                    format!("файлов: {}", plan.files.len())
                ))
                .color(palette::text_weak()),
                rows,
            );

            // The files.
            let mut hidden = 0;
            for f in &plan.files {
                if only_changes
                    && !f.changes()
                    && f.skip.is_none()
                    && f.missing.is_empty()
                    && f.unnamed.is_empty()
                {
                    hidden += 1;
                    continue;
                }
                let mut rows = Vec::new();
                if let Some(to) = &f.rename {
                    rows.push(change(tr!("File name", "Имя файла"), &name_of(&f.path), &name_of(to)));
                } else if !f.unnamed.is_empty() {
                    let why = prepare::quoted(&f.unnamed);
                    rows.push(Row::from([
                        RichText::new(tr!("File name", "Имя файла")).color(palette::error()),
                        RichText::new(name_of(&f.path)).color(palette::text()),
                        RichText::new(tr!(format!("kept: no {why}"), format!("не меняется: пусто {why}")))
                            .color(palette::error()),
                    ]));
                }
                for (key, before, after) in &f.fields {
                    let mut row = change(key.label(), before, after);
                    let read = read_hint(plan, f, *key);
                    let cut = (*key == edit::Key::Album).then(|| cut_hint(plan)).flatten();
                    row.hint = match (read, cut) {
                        (Some(r), Some(c)) => Some(format!("{r}\n\n{c}")),
                        (r, c) => r.or(c),
                    };
                    rows.push(row);
                }
                rows.extend(f.removed.iter().map(removed_row));
                if f.cover
                    && let Some(cover) = &plan.cover
                {
                    let now = f.cover_now.map(|(s, b)| picture(s, b)).unwrap_or_default();
                    rows.push(change(
                        tr!("Cover", "Обложка"),
                        &now,
                        &picture(cover.made.size, cover.made.data.len()),
                    ));
                }
                if f.other_tags {
                    rows.push(change("ID3v1 / APE", tr!("present", "есть"), ""));
                }
                if !f.missing.is_empty() {
                    let names: Vec<&str> = f.missing.iter().map(|n| n.label()).collect();
                    rows.push(Row::from([
                        RichText::new(tr!("Missing", "Не хватает")).color(palette::error()),
                        RichText::new(names.join(", ")).color(palette::error()),
                        RichText::new(""),
                    ]));
                }
                let note = match &f.skip {
                    Some(why) => RichText::new(why).color(palette::error()),
                    None if !f.changes() => {
                        RichText::new(tr!("as it is", "без изменений")).color(palette::text_weak())
                    }
                    None => {
                        let n = f.fields.len()
                            + usize::from(f.rename.is_some())
                            + f.removed.len()
                            + usize::from(f.cover)
                            + usize::from(f.other_tags);
                        RichText::new(tr!(format!("changes: {n}"), format!("изменений: {n}")))
                            .color(palette::accent())
                    }
                };
                group(ui, &name_of(&f.path), note, rows);
            }
            if hidden > 0 {
                ui.label(
                    RichText::new(tr!(
                        format!("{hidden} more files stay as they are."),
                        format!("Ещё файлов без изменений: {hidden}.")
                    ))
                    .color(palette::text_weak()),
                );
            }
        });
}

/// What was done.
fn report(ui: &mut Ui, outcome: &Outcome, height: f32) {
    egui::ScrollArea::vertical()
        .id_salt("prepare_report")
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let line = |ui: &mut Ui, text: String| {
                ui.label(RichText::new(text).color(palette::text()));
            };
            let (title, colour) = if outcome.errors.is_empty() {
                (tr!("Done.", "Готово."), palette::text_bright())
            } else {
                (tr!("Done, with errors.", "Готово, но с ошибками."), palette::error())
            };
            ui.label(RichText::new(title).size(16.0).strong().color(colour));
            line(ui, tr!(
                format!("Tags written: {}", outcome.written),
                format!("Записано тегов: {}", outcome.written)
            ));
            let renamed = outcome
                .moved
                .iter()
                .filter(|(a, b)| a.file_name() != b.file_name())
                .count();
            line(ui, tr!(
                format!("Files renamed: {renamed}"),
                format!("Переименовано файлов: {renamed}")
            ));
            if let Some((_, new)) = &outcome.folder {
                line(ui, tr!(
                    format!("The folder is now {}", name_of(new)),
                    format!("Папка теперь называется {}", name_of(new))
                ));
            }
            if outcome.folder_jpg {
                line(ui, tr!("folder.jpg written", "folder.jpg записан").to_owned());
            }
            if outcome.extras > 0 {
                line(ui, tr!(
                    format!("Other files dealt with: {}", outcome.extras),
                    format!("Прочих файлов убрано: {}", outcome.extras)
                ));
            }
            if let Some(journal) = &outcome.journal {
                ui.label(
                    RichText::new(tr!(
                        format!("What was done is kept in {}", journal.display()),
                        format!("Что сделано, записано в {}", journal.display())
                    ))
                    .size(12.0)
                    .color(palette::text_weak()),
                );
            }
            for (path, e) in &outcome.errors {
                ui.label(
                    RichText::new(format!("{}: {e}", name_of(path)))
                        .size(12.5)
                        .color(palette::error()),
                );
            }
        });
}

/// The editor's tabs.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Tags,
    Read,
    Genre,
    Cover,
    Names,
    Folder,
}

impl Tab {
    const ALL: [Tab; 6] = [Tab::Tags, Tab::Read, Tab::Genre, Tab::Cover, Tab::Names, Tab::Folder];

    fn label(self) -> &'static str {
        match self {
            Tab::Tags => tr!("Tags", "Теги"),
            Tab::Read => tr!("From the names", "Из имён"),
            Tab::Genre => tr!("Genre", "Жанр"),
            Tab::Cover => tr!("Cover", "Обложка"),
            Tab::Names => tr!("Names", "Имена"),
            Tab::Folder => tr!("Folder", "Папка"),
        }
    }
}

/// The presets' editor: the list on the left, the one chosen in tabs on
/// the right; a change is the preset's at once, so the table behind shows
/// it.
#[derive(Default)]
pub struct PresetEditor {
    pub open: bool,
    tab: Tab,
}

impl PresetEditor {
    /// Draws it while open; true when the presets changed. `sample`: the
    /// album's plan, for the names' examples.
    fn show(&mut self, ctx: &egui::Context, settings: &mut Settings, sample: Option<&Plan>) -> bool {
        if !self.open || settings.prepare_presets.is_empty() {
            return false;
        }
        let before = (settings.prepare_presets.clone(), settings.prepare_preset.clone());
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("prepare_presets"))
            .frame(
                egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
                    .fill(palette::panel_bg())
                    .inner_margin(egui::Margin::same(18))
                    .corner_radius(10.0),
            )
            .show(ctx, |ui| {
                round_widgets(ui);
                let size = super::modal_size(ctx, EDITOR, 24.0);
                ui.set_width(size.x);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr!("Preparation presets", "Пресеты подготовки"))
                            .size(20.0)
                            .strong()
                            .color(palette::text_bright()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close |= super::close_cross(ui);
                    });
                });
                ui.add_space(8.0);
                let body = (size.y - 110.0).max(240.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(vec2(LIST, body), Layout::top_down(Align::Min), |ui| {
                        ui.set_min_size(vec2(LIST, body));
                        self.list(ui, settings);
                    });
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.set_min_height(body);
                        let Some(i) = settings
                            .prepare_presets
                            .iter()
                            .position(|p| p.name == settings.prepare_preset)
                        else {
                            return;
                        };
                        // The name.
                        ui.horizontal(|ui| {
                            ui.label(tr!("Name", "Название"));
                            let mut name = settings.prepare_presets[i].name.clone();
                            if ui
                                .add(egui::TextEdit::singleline(&mut name).desired_width(260.0))
                                .changed()
                                && !name.trim().is_empty()
                                && !settings.prepare_presets.iter().any(|p| p.name == name)
                            {
                                settings.prepare_presets[i].name = name.clone();
                                settings.prepare_preset = name;
                            }
                        });
                        ui.add_space(6.0);
                        self.tabs(ui);
                        let profile = &mut settings.prepare_presets[i].profile;
                        egui::ScrollArea::vertical()
                            .id_salt(("prepare_tab", self.tab as u8))
                            .max_height(body - 70.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| match self.tab {
                                Tab::Tags => tags_tab(ui, profile),
                                Tab::Read => read_tab(ui, profile),
                                Tab::Genre => genre_tab(ui, profile),
                                Tab::Cover => cover_tab(ui, profile),
                                Tab::Names => names_tab(ui, profile, sample),
                                Tab::Folder => folder_tab(ui, profile),
                            });
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr!(
                            "Changes are kept at once; the table behind shows what they do.",
                            "Изменения сохраняются сразу; таблица за окном показывает, что они делают."
                        ))
                        .size(12.5)
                        .color(palette::text_weak()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close |= ui
                            .add(egui::Button::new(tr!("Close", "Закрыть")).min_size(vec2(90.0, 0.0)))
                            .clicked();
                    });
                });
            });
        if modal.is_top_modal
            && !modal.any_popup_open
            && ctx.memory(|m| m.focused().is_none())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            close = true;
        }
        if close {
            self.open = false;
        }
        (settings.prepare_presets.clone(), settings.prepare_preset.clone()) != before
    }

    /// The presets, one chosen; a new one (a copy of the one chosen), the
    /// one chosen deleted, or set back to the standard one's settings.
    fn list(&mut self, ui: &mut Ui, settings: &mut Settings) {
        egui::Frame::new()
            .fill(palette::list_bg())
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(200.0);
                let mut chosen = None;
                for preset in &settings.prepare_presets {
                    let on = preset.name == settings.prepare_preset;
                    let resp = ui.add_sized(
                        vec2(ui.available_width(), 24.0),
                        egui::Button::selectable(on, &preset.name),
                    );
                    if resp.clicked() {
                        chosen = Some(preset.name.clone());
                    }
                }
                if let Some(name) = chosen {
                    settings.prepare_preset = name;
                }
            });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(tr!("New", "Новый"))
                .on_hover_text(tr!("A copy of the preset chosen", "Копия выбранного пресета"))
                .clicked()
            {
                let profile = profile_in_use(settings);
                let base = tr!("New preset", "Новый пресет");
                let name = (1..)
                    .map(|n| if n == 1 { base.to_owned() } else { format!("{base} {n}") })
                    .find(|n| !settings.prepare_presets.iter().any(|p| p.name == *n))
                    .unwrap_or_default();
                settings.prepare_presets.push(Preset {
                    name: name.clone(),
                    profile,
                });
                settings.prepare_preset = name;
            }
            let several = settings.prepare_presets.len() > 1;
            if ui
                .add_enabled(several, egui::Button::new(tr!("Delete", "Удалить")))
                .clicked()
            {
                let gone = settings.prepare_preset.clone();
                settings.prepare_presets.retain(|p| p.name != gone);
                settings.prepare_preset = settings.prepare_presets[0].name.clone();
            }
            if ui
                .button(tr!("Standard", "Стандартный"))
                .on_hover_text(tr!(
                    "The chosen preset's settings as the standard one's at first",
                    "Настройки выбранного пресета — как у стандартного по умолчанию"
                ))
                .clicked()
            {
                let name = settings.prepare_preset.clone();
                if let Some(p) = settings.prepare_presets.iter_mut().find(|p| p.name == name) {
                    p.profile = Profile::default();
                }
            }
        });
    }

    fn tabs(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            for tab in Tab::ALL {
                let on = self.tab == tab;
                let colour = if on { palette::text_bright() } else { palette::text_weak() };
                let resp = ui.add(
                    egui::Label::new(RichText::new(tab.label()).size(15.0).color(colour))
                        .sense(egui::Sense::click()),
                );
                if on {
                    let r = resp.rect;
                    ui.painter().hline(
                        r.x_range(),
                        r.bottom() + 3.0,
                        egui::Stroke::new(2.0, palette::accent()),
                    );
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.add_space(4.0);
    }
}

/// A list chosen from, `width` wide.
fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    all: &[T],
    label: impl Fn(T) -> String,
    width: f32,
) {
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(label(*value))
        .show_ui(ui, |ui| {
            for &v in all {
                ui.selectable_value(value, v, label(v));
            }
        });
}

/// Toggled labels: each of `all` in `chosen` or not.
fn toggles<T: Copy + PartialEq>(
    ui: &mut Ui,
    chosen: &mut Vec<T>,
    all: &[T],
    label: impl Fn(T) -> &'static str,
    hint: impl Fn(T) -> Option<&'static str>,
) {
    ui.horizontal_wrapped(|ui| {
        for &item in all {
            let on = chosen.contains(&item);
            let mut resp = ui.selectable_label(on, label(item));
            if let Some(h) = hint(item) {
                resp = resp.on_hover_text(h);
            }
            if resp.clicked() {
                if on {
                    chosen.retain(|c| *c != item);
                } else {
                    chosen.push(item);
                }
            }
        }
    });
}

fn tags_tab(ui: &mut Ui, p: &mut Profile) {
    section(ui, tr!("Numbers", "Номера"), |ui| {
        row(
            ui,
            tr!("Track numbers", "Номера треков"),
            Some(tr!(
                "In order: by the disc, then the file's name; each disc from 1",
                "По порядку: по диску, затем по имени файла; каждый диск с 1"
            )),
            240.0,
            |ui| {
                choice(
                    ui,
                    "prepare_numbering",
                    &mut p.numbering,
                    &Numbering::ALL,
                    |n| n.label().to_owned(),
                    220.0,
                )
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Written as", "Формат номера"),
            Some(tr!("x is a digit: 0x is 01, 0x/xx is 01/16", "x — цифра: 0x — это 01, 0x/xx — 01/16")),
            240.0,
            |ui| choice(ui, "prepare_track", &mut p.track, &TrackFormat::ALL, |t| t.label().to_owned(), 220.0),
        );
        divider(ui);
        row(ui, tr!("Disc number", "Номер диска"), None, 240.0, |ui| {
            choice(ui, "prepare_disc", &mut p.disc, &DiscFormat::ALL, |d| d.label().to_owned(), 220.0)
        });
    });
    section(ui, tr!("Values", "Значения"), |ui| {
        row(
            ui,
            tr!("Album title without the edition", "Альбом без пометки издания"),
            Some(tr!(
                "A part in brackets at the title's end holding one of these words is cut",
                "Убирается часть в скобках в конце названия, где есть одно из этих слов"
            )),
            60.0,
            |ui| switch(ui, &mut p.album_title),
        );
        if p.album_title {
            ui.add_enabled_ui(!p.album_brackets, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut p.editions)
                        .desired_rows(2)
                        .desired_width(ui.available_width())
                        .hint_text(tr!("words, separated by commas", "слова через запятую")),
                );
            });
            ui.label(
                RichText::new(tr!(
                    "Whole words, with their endings: \"remaster\" finds \"Remastered\", \"ep\" does not find \"Deep\".",
                    "Целые слова с окончаниями: «remaster» находит «Remastered», «ep» не находит «Deep»."
                ))
                .size(12.0)
                .color(palette::text_weak()),
            );
            row(
                ui,
                tr!("Every part in brackets at the end", "Любые части в скобках в конце"),
                Some(tr!(
                    "Whatever they say: \"Live (In Moscow)\" becomes \"Live\"",
                    "Что бы в них ни было: «Live (In Moscow)» становится «Live»"
                )),
                60.0,
                |ui| switch(ui, &mut p.album_brackets),
            );
        }
        divider(ui);
        row(
            ui,
            tr!("The year as four digits", "Год — четыре цифры"),
            Some("2011-05-01 » 2011"),
            60.0,
            |ui| switch(ui, &mut p.year_only),
        );
        divider(ui);
        tall_row(
            ui,
            tr!("Alike in every file", "Одинаковые во всех файлах"),
            tr!(
                "The value most files have goes into all",
                "Значение, которое есть у большинства файлов, ставится во все"
            ),
            300.0,
            |ui| toggles(ui, &mut p.unify, &Unify::ALL, |u| u.key().label(), |_| None),
        );
        divider(ui);
        tall_row(
            ui,
            tr!("Empty fields filled", "Заполнять пустые"),
            tr!(
                "A compilation's album artist is Various Artists",
                "У сборника исполнитель альбома — Various Artists"
            ),
            300.0,
            |ui| {
                for f in Fill::ALL {
                    let mut on = p.fill.contains(&f);
                    if ui.checkbox(&mut on, f.label()).changed() {
                        if on {
                            p.fill.push(f);
                        } else {
                            p.fill.retain(|x| *x != f);
                        }
                    }
                }
            },
        );
    });
    section(ui, tr!("Cleaning", "Чистка"), |ui| {
        row(
            ui,
            tr!("Remove the rest of the tag", "Удалять из тегов лишнее"),
            Some(tr!(
                "Everything but the main fields, the cover and the parts chosen to stay",
                "Всё, кроме основных полей, обложки и отмеченных частей"
            )),
            60.0,
            |ui| switch(ui, &mut p.clean),
        );
        if p.clean {
            ui.label(RichText::new(tr!("Keep:", "Оставлять:")).color(palette::text_weak()));
            toggles(ui, &mut p.keep, &TagPart::ALL, TagPart::label, |t| Some(t.hint()));
            ui.add_space(6.0);
        }
        divider(ui);
        row(
            ui,
            tr!("Remove ID3v1 and APE", "Удалять ID3v1 и APE"),
            Some(tr!(
                "An MP3 keeps its ID3v2 tag alone; off, they are written as the settings say",
                "У MP3 остаётся только ID3v2; если выключено — как в общих настройках"
            )),
            60.0,
            |ui| switch(ui, &mut p.only_id3v2),
        );
    });
    section(ui, tr!("Check", "Проверка"), |ui| {
        tall_row(
            ui,
            tr!("Every file must have", "Обязательные поля"),
            tr!("A file without one is pointed out", "Файл без такого поля отмечается"),
            300.0,
            |ui| toggles(ui, &mut p.required, &Need::ALL, Need::label, |_| None),
        );
    });
}

/// Masks, one a line: edited as text, kept as a list.
fn masks_field(ui: &mut Ui, id: &str, masks: &mut Vec<String>) {
    let mut text = masks.join("\n");
    ui.horizontal_top(|ui| {
        let field = ui.add(
            egui::TextEdit::multiline(&mut text)
                .id_salt(id)
                .desired_rows(masks.len().clamp(3, 8))
                .desired_width(ui.available_width() - 40.0)
                .font(egui::TextStyle::Monospace),
        );
        let added = mask_fields_menu(ui, &mut text);
        if field.changed() || added {
            *masks = text.lines().map(str::to_owned).collect();
        }
    });
}

fn read_tab(ui: &mut Ui, p: &mut Profile) {
    section(ui, tr!("Tags from the names", "Теги из имён"), |ui| {
        row(
            ui,
            tr!("Read the names", "Брать поля из имён"),
            Some(tr!(
                "The files' and the folder's names read by masks, as Tag&Rename's \"Get Tags from File Name\"",
                "Имена файлов и папки разбираются по маскам, как «Get Tags from File Name» в Tag&Rename"
            )),
            60.0,
            |ui| switch(ui, &mut p.from_names),
        );
        if p.from_names {
            divider(ui);
            row(
                ui,
                tr!("Over the tags' values too", "Заменять и заполненные поля"),
                Some(tr!(
                    "Off: only empty fields are filled. On: a name read wins over the tag (\"Hunger: Part 1\" in the tag becomes \"Hunger - Part 1\" of the name)",
                    "Выключено — заполняются только пустые поля. Включено — прочитанное из имени заменяет значение тега («Hunger: Part 1» в теге станет «Hunger - Part 1» из имени)"
                )),
                60.0,
                |ui| switch(ui, &mut p.names_over_tags),
            );
            divider(ui);
            row(ui, tr!("_ read as a space", "_ считать пробелом"), None, 60.0, |ui| {
                switch(ui, &mut p.names_underscores)
            });
        }
    });
    if p.from_names {
        section(ui, tr!("Files' masks", "Маски имён файлов"), |ui| {
            ui.label(
                RichText::new(tr!(
                    "One a line; each file is read by the first that fits it.",
                    "По одной в строке; каждый файл читается первой подходящей."
                ))
                .size(12.5)
                .color(palette::text_weak()),
            );
            masks_field(ui, "prepare_file_masks", &mut p.file_masks);
            ui.add_space(6.0);
        });
        section(ui, tr!("Folder's masks", "Маски имени папки"), |ui| {
            ui.label(
                RichText::new(tr!(
                    "One a line; the first that fits. The folder's %artist% is the album artist.",
                    "По одной в строке; берётся первая подходящая. %artist% папки — исполнитель альбома."
                ))
                .size(12.5)
                .color(palette::text_weak()),
            );
            masks_field(ui, "prepare_folder_masks", &mut p.folder_masks);
            ui.add_space(6.0);
        });
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(tr!(
            "%year% is four digits, %track% and %disc% are digits; %skip% is a part not read. A \\ takes the folder above too: %artist%\\%year% - %album%. The masks for one album are also chosen in the preparation's window.",
            "%year% — четыре цифры, %track% и %disc% — цифры; %skip% — часть, которая не читается. \\ захватывает и папку выше: %artist%\\%year% - %album%. Маску для конкретного альбома можно задать и в окне подготовки."
        ))
        .size(12.5)
        .color(palette::text_weak()),
    );
}

fn genre_tab(ui: &mut Ui, p: &mut Profile) {
    section(ui, tr!("Genres in the tags", "Жанры в тегах"), |ui| {
        row(
            ui,
            tr!("The artist's genre", "Как у других альбомов исполнителя"),
            Some(tr!(
                "When the library holds other albums of the artist, their genre",
                "Если в библиотеке есть другие альбомы исполнителя, берётся их жанр"
            )),
            60.0,
            |ui| switch(ui, &mut p.genre_from_artist),
        );
        divider(ui);
        row(
            ui,
            tr!("Separated by", "Разделители в поле"),
            Some(tr!(
                "Any of these letters parts the genres of one field; empty: the field is one genre",
                "Любой из этих знаков разделяет жанры в одном поле; пусто — поле считается одним жанром"
            )),
            120.0,
            |ui| {
                ui.add(egui::TextEdit::singleline(&mut p.genre_split).desired_width(100.0));
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Genres kept", "Оставлять жанров"),
            Some(tr!("The first ones of the list", "Первые из списка")),
            160.0,
            |ui| {
                let label = |n: u8| {
                    if n == 0 {
                        tr!("All", "Все").to_owned()
                    } else {
                        n.to_string()
                    }
                };
                choice(ui, "prepare_genre_count", &mut p.genre_count, &[0, 1, 2, 3], label, 140.0)
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Written between them", "Записывать через"),
            Some(tr!("When more than one stays", "Если остаётся больше одного")),
            120.0,
            |ui| {
                ui.add(egui::TextEdit::singleline(&mut p.genre_join).desired_width(100.0));
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Spelt as the library", "Написание как в библиотеке"),
            Some(tr!(
                "\"modern metal\" becomes \"Modern Metal\" when the library or the usual genres spell it so",
                "«modern metal» становится «Modern Metal», если так он записан в библиотеке или в списке жанров"
            )),
            60.0,
            |ui| switch(ui, &mut p.genre_spelling),
        );
    });
    ui.add_space(8.0);
    ui.label(
        RichText::new(tr!(
            "The genre for one album is also chosen in the preparation's window: by the preset, as it is, or one's own.",
            "Жанр конкретного альбома можно выбрать и в окне подготовки: по пресету, как есть или свой."
        ))
        .size(12.5)
        .color(palette::text_weak()),
    );
}

fn cover_tab(ui: &mut Ui, p: &mut Profile) {
    section(ui, tr!("Covers in the tags", "Обложка в тегах"), |ui| {
        row(ui, tr!("The tags' covers", "Обложки в тегах"), None, 260.0, |ui| {
            choice(ui, "prepare_tag_cover", &mut p.tag_cover, &TagCover::ALL, |t| t.label().to_owned(), 240.0)
        });
        if p.tag_cover == TagCover::Fitting {
            divider(ui);
            row(
                ui,
                tr!("A cover that fits", "Подходящая обложка"),
                Some(tr!(
                    "Its sides within these pixels, its size no more than this; a tag's cover that fits becomes the album's",
                    "Стороны в этих пределах (пиксели), размер не больше указанного; подходящая обложка из тегов становится обложкой альбома"
                )),
                300.0,
                |ui| {
                    ui.label(tr!("KB", "КБ"));
                    ui.add(egui::DragValue::new(&mut p.cover_kb).range(10..=20_000).speed(10));
                    ui.label("≤");
                    ui.label(tr!("px,", "пикс.,"));
                    ui.add(egui::DragValue::new(&mut p.cover_max).range(100..=8192).speed(10));
                    ui.label("–");
                    ui.add(egui::DragValue::new(&mut p.cover_min).range(50..=8192).speed(10));
                },
            );
        }
        divider(ui);
        row(ui, tr!("The album's cover from", "Обложка альбома берётся"), None, 260.0, |ui| {
            choice(ui, "prepare_cover_from", &mut p.cover_from, &CoverFrom::ALL, |c| c.label().to_owned(), 240.0)
        });
        divider(ui);
        row(
            ui,
            tr!("Largest side put in", "Наибольшая сторона в тегах"),
            Some(tr!(
                "A larger picture is made smaller, as a JPEG",
                "Картинка больше уменьшается и сохраняется как JPEG"
            )),
            300.0,
            |ui| {
                // Right to left: one's own size's field, then the list.
                if let CoverSide::Custom(px) = p.cover_side {
                    ui.label(tr!("px", "пикс."));
                    let mut own = px;
                    let field = egui::DragValue::new(&mut own).range(CoverSide::OWN).speed(10);
                    if ui
                        .add(field)
                        .on_hover_text(tr!(
                            "100 to 8192 pixels: drag, or click and type",
                            "От 100 до 8192 пикселей: потяните или щёлкните и введите"
                        ))
                        .changed()
                    {
                        p.cover_side = CoverSide::Custom(own);
                    }
                }
                let own = tr!("Own…", "Свой…");
                let shown = if p.cover_side.is_custom() {
                    own.to_owned()
                } else {
                    p.cover_side.label()
                };
                egui::ComboBox::from_id_salt("prepare_side")
                    .width(150.0)
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for side in CoverSide::ALL {
                            let on = !p.cover_side.is_custom() && p.cover_side == side;
                            if ui.selectable_label(on, side.label()).clicked() {
                                p.cover_side = side;
                            }
                        }
                        if ui.selectable_label(p.cover_side.is_custom(), own).clicked()
                            && !p.cover_side.is_custom()
                        {
                            p.cover_side = CoverSide::Custom(p.cover_side.pixels().unwrap_or(800));
                        }
                    });
            },
        );
    });
    section(ui, "folder.jpg", |ui| {
        row(ui, "folder.jpg", None, 260.0, |ui| {
            choice(ui, "prepare_folder_jpg", &mut p.folder_jpg, &FolderJpg::ALL, |f| f.label().to_owned(), 240.0)
        });
    });
}

/// A template's field: the text, a menu of the fields to add, and its
/// result for the album's first file.
fn template(ui: &mut Ui, label: &str, text: &mut String, example: Option<String>) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_sized(vec2(150.0, 20.0), egui::Label::new(RichText::new(label).color(palette::text_weak())));
        ui.add(egui::TextEdit::singleline(text).desired_width(ui.available_width() - 40.0));
        ui.menu_button("+", |ui| {
            for field in ["track", "title", "artist", "album", "albumartist", "year", "disc", "genre"] {
                let name = crate::title_format::field_label(field).unwrap_or(field);
                if ui.button(format!("{name}  %{field}%")).clicked() {
                    text.push_str(&format!("%{field}%"));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tr!("Add a field", "Добавить поле"));
    });
    if let Some(example) = example {
        ui.horizontal(|ui| {
            ui.add_space(158.0);
            ui.label(
                RichText::new(tr!(format!("e.g. {example}"), format!("Например: {example}")))
                    .size(12.5)
                    .color(palette::text_weak()),
            );
        });
    }
}

fn names_tab(ui: &mut Ui, p: &mut Profile, sample: Option<&Plan>) {
    let first = sample.and_then(|s| s.files.first());
    let file_example = first.map(|f| name_of(f.rename.as_ref().unwrap_or(&f.path)));
    let folder_example = sample.map(|s| name_of(s.new_folder.as_ref().unwrap_or(&s.folder)));
    section(ui, tr!("Files", "Файлы"), |ui| {
        row(ui, tr!("Rename the files", "Переименовывать файлы"), None, 60.0, |ui| {
            switch(ui, &mut p.rename_files)
        });
        if p.rename_files {
            template(ui, tr!("Template", "Шаблон"), &mut p.file_name, file_example);
            template(ui, tr!("Several artists", "Разные исполнители"), &mut p.file_name_various, None);
            ui.add_space(6.0);
        }
    });
    section(ui, tr!("Folder", "Папка"), |ui| {
        row(ui, tr!("Rename the folder", "Переименовывать папку"), None, 60.0, |ui| {
            switch(ui, &mut p.rename_folder)
        });
        if p.rename_folder {
            template(ui, tr!("Template", "Шаблон"), &mut p.folder_name, folder_example);
            ui.add_space(6.0);
        }
    });
    section(ui, tr!("Both", "Общее"), |ui| {
        row(ui, tr!("Spaces as underscores", "Пробелы — подчёркивания"), None, 60.0, |ui| {
            switch(ui, &mut p.underscores)
        });
    });
    ui.add_space(8.0);
    ui.label(
        RichText::new(tr!(
            "[ ] shows its part only when a field in it has something; several discs in one folder get %disc%- before the name. The letters Windows refuses are replaced (a colon by a dash).",
            "[ ] показывает свою часть, только если поле в ней не пустое; при нескольких дисках в одной папке к имени добавляется %disc%-. Недопустимые в Windows символы заменяются (двоеточие — дефисом)."
        ))
        .size(12.5)
        .color(palette::text_weak()),
    );
}

fn folder_tab(ui: &mut Ui, p: &mut Profile) {
    section(ui, tr!("The folder's other files", "Прочие файлы папки"), |ui| {
        row(
            ui,
            tr!("Other files", "Прочие файлы"),
            Some(tr!(
                "Every file of the folder but the album's audio and folder.jpg: scans, .cue, .log, .m3u, .nfo, Thumbs.db…",
                "Все файлы папки, кроме аудио альбома и folder.jpg: сканы, .cue, .log, .m3u, .nfo, Thumbs.db…"
            )),
            220.0,
            |ui| choice(ui, "prepare_extras", &mut p.extras, &Extras::ALL, |e| e.label().to_owned(), 200.0),
        );
        if p.extras == Extras::Folder {
            divider(ui);
            row(ui, tr!("Subfolder", "Подпапка"), None, 220.0, |ui| {
                ui.add(egui::TextEdit::singleline(&mut p.extras_folder).desired_width(200.0));
            });
        }
    });
    section(ui, tr!("Files", "Файлы"), |ui| {
        row(
            ui,
            tr!("Take off \"read-only\"", "Снимать «только чтение»"),
            Some(tr!(
                "A file with the read-only attribute is not written otherwise",
                "Иначе файл с атрибутом «только чтение» не записывается"
            )),
            60.0,
            |ui| switch(ui, &mut p.clear_read_only),
        );
    });
}
