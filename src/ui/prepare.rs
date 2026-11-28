//! The album's preparation window ("Prepare the album…" in an album's
//! menu, "Prepare a folder…" in the folders' menu): the profile on the
//! left, what it would do on the right, worked out again at every change
//! (`library::prepare::plan`), Apply at the bottom, then what was done.
//! The files are read and written on threads of their own; the app stops
//! the player first when a file of the album plays and is to be renamed,
//! and afterwards moves the plays, ratings, loudness and playlist entries
//! to the new paths and reads the folders again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};

use egui::{Align, Key, Layout, RichText, Ui, vec2};

use crate::library::edit::{self, TagPart};
use crate::library::prepare::{self, Extras, Need, Outcome, Plan, Preset, Profile, Source};
use crate::palette;
use crate::settings::Settings;
use crate::ui::settings::{round_widgets, switch};

/// The window's size, when the program's window has room for it.
const SIZE: egui::Vec2 = vec2(1040.0, 700.0);
/// The profile's column.
const LEFT: f32 = 360.0;

#[derive(Default)]
pub struct PrepareWindow {
    pub open: bool,
    /// The album's name, for the heading.
    title: String,
    reading: Option<Receiver<Source>>,
    source: Option<Arc<Source>>,
    /// The plan for the profile it was made of.
    plan: Option<(Profile, Arc<Plan>)>,
    applying: Option<Applying>,
    outcome: Option<Outcome>,
    /// Apply was clicked: the app starts it (`start`).
    start_asked: bool,
    /// A name typed for the profile to be kept under.
    naming: Option<String>,
}

struct Applying {
    done: Arc<AtomicUsize>,
    total: usize,
    answer: Receiver<Outcome>,
}

/// What the window did this frame, for the app.
#[derive(Default)]
pub struct Shown {
    /// The profile or the presets changed: the settings are saved.
    pub changed: bool,
    /// The preparation ended.
    pub done: Option<Outcome>,
}

impl PrepareWindow {
    /// Opens the window for the files at `paths` (an album's, or a
    /// folder's audio) and reads them; `side` the tags' largest cover.
    pub fn open(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, title: String, side: Option<u32>) {
        if paths.is_empty() || self.applying.is_some() {
            return;
        }
        *self = PrepareWindow {
            open: true,
            title,
            ..PrepareWindow::default()
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("prepare read".into())
            .spawn(move || {
                let _ = tx.send(prepare::read(paths, side));
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
        self.plan.as_ref().map(|(_, plan)| plan.clone())
    }

    /// Does the plan on a thread.
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        profile: Profile,
        how: edit::WriteTags,
        releaser: crate::audio::Releaser,
    ) {
        let Some((_, plan)) = self.plan.clone() else {
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
            && let Ok(source) = rx.try_recv()
        {
            self.source = Some(Arc::new(source));
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
        if let Some(source) = &self.source
            && self.outcome.is_none()
            && self.plan.as_ref().is_none_or(|(p, _)| *p != settings.prepare)
        {
            let plan = prepare::plan(source, &settings.prepare);
            self.plan = Some((settings.prepare.clone(), Arc::new(plan)));
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
                    ui.label(RichText::new(&self.title).size(15.0).color(palette::text_weak()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !busy {
                            close |= super::close_cross(ui);
                        }
                    });
                });
                ui.add_space(6.0);
                let body = (size.y - 110.0).max(240.0);
                if let Some(outcome) = &self.outcome {
                    report(ui, outcome, body);
                } else {
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            vec2(LEFT, body),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.add_enabled_ui(!busy, |ui| {
                                    shown.changed |= profile_column(
                                        ui,
                                        settings,
                                        &mut self.naming,
                                        body,
                                    );
                                });
                            },
                        );
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.set_min_height(body);
                            match (&self.plan, &self.source) {
                                (Some((_, plan)), Some(source)) => preview(ui, plan, source, body),
                                _ => {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(tr!("Reading the tags…", "Чтение тегов…"));
                                    });
                                }
                            }
                        });
                    });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(applying) = &self.applying {
                        ui.spinner();
                        ui.label(tr!(
                            format!(
                                "Working… {} of {}",
                                applying.done.load(Ordering::Relaxed).min(applying.total),
                                applying.total
                            ),
                            format!(
                                "Выполняется… {} из {}",
                                applying.done.load(Ordering::Relaxed).min(applying.total),
                                applying.total
                            )
                        ));
                    } else if self.outcome.is_none()
                        && let Some((_, plan)) = &self.plan
                    {
                        ui.label(RichText::new(summary(plan)).color(palette::text_weak()));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.outcome.is_some() {
                            close |= ui
                                .add(egui::Button::new(tr!("Close", "Закрыть")).min_size(vec2(90.0, 0.0)))
                                .clicked();
                            if let Some((_, new)) = self.outcome.as_ref().and_then(|o| o.folder.clone()) {
                                if ui.button(tr!("Show in folder", "Показать в папке")).clicked() {
                                    crate::win::shell_open(&new);
                                }
                            } else if let Some(plan) = self.plan.as_ref().map(|(_, p)| p.clone())
                                && ui.button(tr!("Show in folder", "Показать в папке")).clicked()
                            {
                                crate::win::shell_open(&plan.folder);
                            }
                        } else {
                            let ready = !busy
                                && self.plan.as_ref().is_some_and(|(_, p)| !p.is_empty());
                            if ui
                                .add_enabled(
                                    ready,
                                    egui::Button::new(
                                        RichText::new(tr!("Apply", "Применить")).strong(),
                                    )
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
        if !busy
            && modal.is_top_modal
            && !modal.any_popup_open
            && ctx.memory(|m| m.focused().is_none())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            close = true;
        }
        if close && !busy {
            self.open = false;
            self.source = None;
            self.plan = None;
        }
        shown
    }
}

/// The line under the plan: what it would do.
fn summary(plan: &Plan) -> String {
    if plan.is_empty() {
        return tr!(
            "Nothing to do: the album is as the profile wants it.",
            "Делать нечего: альбом уже такой, как требует профиль."
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

/// The profile: the ready and kept ones, then each step.
fn profile_column(ui: &mut Ui, settings: &mut Settings, naming: &mut Option<String>, height: f32) -> bool {
    let mut changed = false;
    // The profile in use, by its name.
    let ready = prepare::ready();
    let current = ready
        .iter()
        .find(|(_, p)| *p == settings.prepare)
        .map(|(n, _)| (*n).to_owned())
        .or_else(|| {
            settings
                .prepare_presets
                .iter()
                .find(|p| p.profile == settings.prepare)
                .map(|p| p.name.clone())
        });
    ui.horizontal(|ui| {
        ui.label(tr!("Profile", "Профиль"));
        egui::ComboBox::from_id_salt("prepare_profile")
            .width(170.0)
            .selected_text(current.clone().unwrap_or_else(|| tr!("Custom", "Свой").to_owned()))
            .show_ui(ui, |ui| {
                for (name, profile) in &ready {
                    if ui.selectable_label(current.as_deref() == Some(name), *name).clicked() {
                        settings.prepare = profile.clone();
                        changed = true;
                    }
                }
                if !settings.prepare_presets.is_empty() {
                    ui.separator();
                }
                for preset in &settings.prepare_presets {
                    if ui
                        .selectable_label(current.as_deref() == Some(preset.name.as_str()), &preset.name)
                        .clicked()
                    {
                        settings.prepare = preset.profile.clone();
                        changed = true;
                    }
                }
            });
        if ui
            .small_button("+")
            .on_hover_text(tr!("Keep these settings under a name", "Сохранить настройки под именем"))
            .clicked()
        {
            *naming = Some(String::new());
        }
        let own = settings
            .prepare_presets
            .iter()
            .position(|p| p.profile == settings.prepare);
        if let Some(i) = own
            && ui
                .small_button("×")
                .on_hover_text(tr!("Delete this profile", "Удалить этот профиль"))
                .clicked()
        {
            settings.prepare_presets.remove(i);
            changed = true;
        }
    });
    if let Some(name) = naming {
        let mut done = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(name)
                    .hint_text(tr!("The profile's name", "Имя профиля"))
                    .desired_width(200.0),
            );
            field.request_focus();
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.button(tr!("Save", "Сохранить")).clicked() || enter) && !name.trim().is_empty() {
                let name = name.trim().to_owned();
                settings.prepare_presets.retain(|p| p.name != name);
                settings.prepare_presets.push(Preset {
                    name,
                    profile: settings.prepare.clone(),
                });
                changed = true;
                done = true;
            }
            if ui.button(tr!("Cancel", "Отмена")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                done = true;
            }
        });
        if done {
            *naming = None;
        }
    }
    ui.add_space(4.0);
    let p = &mut settings.prepare;
    let before = p.clone();
    egui::ScrollArea::vertical()
        .id_salt("prepare_steps")
        .max_height(height - 40.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            heading(ui, tr!("Tags", "Теги"));
            step(
                ui,
                &mut p.album_title,
                tr!("Album title without the edition", "Альбом без пометки издания"),
                tr!(
                    "A part in brackets at the title's end holding one of these words is cut: \"Album [Deluxe Edition]\" → \"Album\".",
                    "Часть в скобках в конце названия, где есть одно из этих слов, убирается: «Album [Deluxe Edition]» → «Album»."
                ),
            );
            if p.album_title {
                indented(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut p.editions)
                            .desired_rows(2)
                            .desired_width(ui.available_width())
                            .hint_text(tr!("words, separated by commas", "слова через запятую")),
                    );
                });
            }
            step(
                ui,
                &mut p.numbers,
                tr!("Track numbers as 01", "Номера треков 01"),
                tr!(
                    "Without the total (\"1/16\" → \"01\"); the disc's number only on an album of several discs.",
                    "Без общего числа («1/16» → «01»); номер диска — только у многодисковых альбомов."
                ),
            );
            step(
                ui,
                &mut p.unify,
                tr!("Album, album artist, year, genre alike", "Единые альбом, исполнитель альбома, год, жанр"),
                tr!(
                    "The value most files have goes into all; an empty album artist is taken from the artists, an empty title from the file's name.",
                    "Значение, которое есть у большинства файлов, ставится во все; пустой исполнитель альбома берётся из исполнителей, пустое название — из имени файла."
                ),
            );
            step(
                ui,
                &mut p.year_only,
                tr!("The year as four digits", "Год — четыре цифры"),
                tr!("\"2011-05-01\" → \"2011\"", "«2011-05-01» → «2011»"),
            );
            step(
                ui,
                &mut p.clean,
                tr!("Remove the rest of the tag", "Удалять из тегов лишнее"),
                tr!(
                    "Everything but the main fields, the cover and the parts chosen to stay.",
                    "Всё, кроме основных полей, обложки и выбранных ниже частей."
                ),
            );
            if p.clean {
                indented(ui, |ui| {
                    ui.label(RichText::new(tr!("Keep:", "Оставлять:")).color(palette::text_weak()));
                    ui.horizontal_wrapped(|ui| {
                        for part in TagPart::ALL {
                            let on = p.keep.contains(&part);
                            if ui.selectable_label(on, part.label()).on_hover_text(part.hint()).clicked() {
                                if on {
                                    p.keep.retain(|k| *k != part);
                                } else {
                                    p.keep.push(part);
                                }
                            }
                        }
                    });
                });
            }
            step(
                ui,
                &mut p.only_id3v2,
                tr!("Remove ID3v1 and APE", "Удалять ID3v1 и APE"),
                tr!(
                    "An MP3 keeps its ID3v2 tag alone; off, they are written as the settings say.",
                    "У MP3 остаётся только тег ID3v2; если выключено — как в настройках."
                ),
            );
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("Every file must have:", "Обязательные поля:")).color(palette::text_weak()));
            ui.horizontal_wrapped(|ui| {
                for need in Need::ALL {
                    let on = p.required.contains(&need);
                    if ui.selectable_label(on, need.label()).clicked() {
                        if on {
                            p.required.retain(|n| *n != need);
                        } else {
                            p.required.push(need);
                        }
                    }
                }
            });

            heading(ui, tr!("Cover", "Обложка"));
            step(
                ui,
                &mut p.cover,
                tr!("folder.jpg and the same cover in each tag", "folder.jpg и та же обложка в каждом теге"),
                tr!(
                    "Taken from folder.jpg, else a picture of the folder named as a cover, else the cover most tags hold.",
                    "Берётся из folder.jpg, иначе из картинки в папке (cover, front…), иначе из обложки, которая есть у большинства тегов."
                ),
            );
            if p.cover {
                indented(ui, |ui| {
                    ui.radio_value(&mut p.folder_jpg_same, true, tr!("One picture for both", "Одна и та же картинка"));
                    ui.radio_value(
                        &mut p.folder_jpg_same,
                        false,
                        tr!("folder.jpg as it is, the tags smaller", "folder.jpg — исходная, в тегах — уменьшенная"),
                    );
                    ui.label(
                        RichText::new(tr!(
                            "The tags' size is the one chosen in the file info's cover menu.",
                            "Размер для тегов — тот, что выбран в меню обложки окна сведений о файле."
                        ))
                        .size(12.0)
                        .color(palette::text_weak()),
                    );
                });
            }

            heading(ui, tr!("Names", "Имена"));
            step(
                ui,
                &mut p.rename_files,
                tr!("Rename the files", "Переименовать файлы"),
                fields_hint(),
            );
            if p.rename_files {
                indented(ui, |ui| {
                    template(ui, tr!("Album", "Альбом"), &mut p.file_name);
                    template(ui, tr!("Several artists", "Разные исполнители"), &mut p.file_name_various);
                });
            }
            step(ui, &mut p.rename_folder, tr!("Rename the folder", "Переименовать папку"), fields_hint());
            if p.rename_folder {
                indented(ui, |ui| template(ui, tr!("Folder", "Папка"), &mut p.folder_name));
            }

            heading(ui, tr!("Folder", "Папка"));
            ui.horizontal(|ui| {
                ui.label(tr!("Other files", "Прочие файлы"))
                    .on_hover_text(tr!(
                        "Every file of the folder but the album's audio and folder.jpg: scans, .cue, .log, .m3u, .nfo…",
                        "Все файлы папки, кроме аудио альбома и folder.jpg: сканы, .cue, .log, .m3u, .nfo…"
                    ));
                let label = |e: Extras| match e {
                    Extras::Keep => tr!("keep", "оставить"),
                    Extras::Delete => tr!("to the recycle bin", "в корзину"),
                    Extras::Folder => tr!("into a subfolder", "в подпапку"),
                };
                egui::ComboBox::from_id_salt("prepare_extras")
                    .selected_text(label(p.extras))
                    .show_ui(ui, |ui| {
                        for e in [Extras::Keep, Extras::Delete, Extras::Folder] {
                            ui.selectable_value(&mut p.extras, e, label(e));
                        }
                    });
                if p.extras == Extras::Folder {
                    ui.add(egui::TextEdit::singleline(&mut p.extras_folder).desired_width(80.0));
                }
            });
            step(
                ui,
                &mut p.clear_read_only,
                tr!("Take off \"read-only\"", "Снимать «только чтение»"),
                tr!(
                    "A file with the read-only attribute is not written otherwise.",
                    "Иначе файл с атрибутом «только чтение» не записывается."
                ),
            );
            if ui
                .small_button(tr!("Defaults", "По умолчанию"))
                .on_hover_text(tr!("The profile as it was at first", "Профиль, как был изначально"))
                .clicked()
            {
                *p = Profile::default();
            }
        });
    changed || *p != before
}

fn fields_hint() -> &'static str {
    tr!(
        "Fields: %track% %title% %artist% %album% %albumartist% %year% %disc% %genre%; [ ] shows its part only when a field in it has something.",
        "Поля: %track% %title% %artist% %album% %albumartist% %year% %disc% %genre%; [ ] показывает свою часть, только если поле в ней не пустое."
    )
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).strong().color(palette::text_bright()));
    ui.add_space(2.0);
}

/// A step: its switch and its name, the hint as the tooltip.
fn step(ui: &mut Ui, on: &mut bool, label: &str, hint: &str) {
    ui.horizontal(|ui| {
        switch(ui, on).on_hover_text(hint);
        let name = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
        if name.clicked() {
            *on = !*on;
        }
        name.on_hover_text(hint);
    });
}

fn indented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(48.0);
        ui.vertical(add);
    });
}

fn template(ui: &mut Ui, label: &str, text: &mut String) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, 20.0), egui::Label::new(RichText::new(label).color(palette::text_weak())));
        ui.add(egui::TextEdit::singleline(text).desired_width(ui.available_width()));
    });
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// What the plan would do: the folder, the cover, the other files, the
/// warnings, then each file.
fn preview(ui: &mut Ui, plan: &Plan, source: &Source, height: f32) {
    let weak = palette::text_weak();
    let new = palette::accent();
    let bad = palette::error();
    egui::ScrollArea::vertical()
        .id_salt("prepare_preview")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr!("Folder", "Папка")).color(weak));
                ui.label(RichText::new(name_of(&plan.folder)).color(palette::text_bright()));
                if let Some(to) = &plan.new_folder {
                    ui.label(RichText::new("»").color(weak));
                    ui.label(RichText::new(name_of(to)).color(new).strong());
                }
            });
            if let Some(cover) = &source.cover {
                let from = if cover.in_tag {
                    tr!("a tag", "тега").to_owned()
                } else {
                    name_of(&cover.from)
                };
                let folder = if plan.folder_jpg.is_some() {
                    if source.folder_jpg.is_some() {
                        tr!("folder.jpg rewritten", "folder.jpg перезаписывается")
                    } else {
                        tr!("folder.jpg made", "folder.jpg создаётся")
                    }
                } else {
                    tr!("folder.jpg as it is", "folder.jpg без изменений")
                };
                let covers = plan.files.iter().filter(|f| f.cover).count();
                ui.label(
                    RichText::new(tr!(
                        format!(
                            "Cover from {from}, {}×{}; into the tags {}×{} ({covers} files); {folder}",
                            cover.size.0, cover.size.1, cover.tag_size.0, cover.tag_size.1
                        ),
                        format!(
                            "Обложка из {from}, {}×{}; в теги {}×{} (файлов: {covers}); {folder}",
                            cover.size.0, cover.size.1, cover.tag_size.0, cover.tag_size.1
                        )
                    ))
                    .color(weak),
                );
            }
            if !plan.extras.is_empty() {
                let names: Vec<String> = plan.extras.iter().map(|p| name_of(p)).collect();
                ui.label(
                    RichText::new(tr!(
                        format!("Other files ({}): {}", names.len(), names.join(", ")),
                        format!("Прочие файлы ({}): {}", names.len(), names.join(", "))
                    ))
                    .color(weak),
                );
            }
            for w in &plan.warnings {
                ui.label(RichText::new(w).color(bad));
            }
            ui.add_space(6.0);
            for f in &plan.files {
                egui::Frame::new()
                    .fill(palette::card_bg())
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            let name = name_of(&f.path);
                            match &f.rename {
                                Some(to) => {
                                    ui.label(RichText::new(name).color(weak));
                                    ui.label(RichText::new("»").color(weak));
                                    ui.label(RichText::new(name_of(to)).color(new).strong());
                                }
                                None => {
                                    ui.label(RichText::new(name).color(palette::text_bright()));
                                }
                            }
                        });
                        if let Some(why) = &f.skip {
                            ui.label(RichText::new(why).size(12.5).color(bad));
                        }
                        for (key, before, after) in &f.fields {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(format!("{}:", key.label())).size(12.5).color(weak));
                                let before = if before.is_empty() { "—" } else { before };
                                let after = if after.is_empty() { "—" } else { after };
                                ui.label(RichText::new(before).size(12.5).strikethrough().color(weak));
                                ui.label(RichText::new(after).size(12.5).color(new));
                            });
                        }
                        if !f.removed.is_empty() {
                            let mut names: Vec<&str> = f.removed.iter().map(|(_, n)| n.as_str()).collect();
                            names.dedup();
                            ui.label(
                                RichText::new(tr!(
                                    format!("Removed: {}", names.join(", ")),
                                    format!("Удаляется: {}", names.join(", "))
                                ))
                                .size(12.5)
                                .color(weak),
                            );
                        }
                        if f.cover {
                            ui.label(
                                RichText::new(tr!("The cover is put in", "Обложка ставится в тег"))
                                    .size(12.5)
                                    .color(weak),
                            );
                        }
                        if f.other_tags {
                            ui.label(
                                RichText::new(tr!("ID3v1 / APE removed", "Удаляются ID3v1 / APE"))
                                    .size(12.5)
                                    .color(weak),
                            );
                        }
                        if !f.missing.is_empty() {
                            let names: Vec<&str> = f.missing.iter().map(|n| n.label()).collect();
                            ui.label(
                                RichText::new(tr!(
                                    format!("Missing: {}", names.join(", ")),
                                    format!("Нет: {}", names.join(", "))
                                ))
                                .size(12.5)
                                .color(bad),
                            );
                        }
                        if !f.writes() && f.rename.is_none() && f.skip.is_none() && f.missing.is_empty() {
                            ui.label(
                                RichText::new(tr!("As it should be", "Без изменений"))
                                    .size(12.5)
                                    .color(weak),
                            );
                        }
                    });
                ui.add_space(4.0);
            }
        });
}

/// What was done.
fn report(ui: &mut Ui, outcome: &Outcome, height: f32) {
    egui::ScrollArea::vertical()
        .id_salt("prepare_report")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let line = |ui: &mut Ui, text: String| {
                ui.label(RichText::new(text).color(palette::text()));
            };
            if outcome.errors.is_empty() {
                ui.label(
                    RichText::new(tr!("Done.", "Готово."))
                        .size(16.0)
                        .strong()
                        .color(palette::text_bright()),
                );
            } else {
                ui.label(
                    RichText::new(tr!("Done, with errors.", "Готово, но с ошибками."))
                        .size(16.0)
                        .strong()
                        .color(palette::error()),
                );
            }
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
