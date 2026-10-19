//! The view editor: a modal where a view of the library
//! (`library::filter`) gets its name and its rules. Each rule is a row:
//! the field, the operation and the value; a text field's value has a
//! menu of the values the library holds (the genres as the tags spell
//! them, so "Soundtrack" is picked and not mistyped), a folder's a browse
//! button. Under the rules a live count says how many artists, albums and
//! tracks the draft leaves. OK saves (a new view is switched to), Delete
//! removes the view edited. Esc, Cancel or the cross give the draft up; a
//! click outside does not, so a draft is not lost by a slip.

use egui::{Align, Key, Layout, RichText, Sense, Stroke, Ui, pos2, vec2};
use std::path::PathBuf;

use crate::format::natural_cmp;
use crate::library::filter::{Field, Filter, Kind, Op, Rule};
use crate::library::{ArtistId, Library, search_form};
use crate::palette;
use crate::settings::Settings;
use crate::ui::icons::Icon;
use crate::ui::settings::{TOOL, round_widgets, tool};

/// The width of the window's content in points.
const WIDTH: f32 = 660.0;
/// The most rules a view may have.
const MAX_RULES: usize = 12;
/// The longest name, in letters.
const MAX_NAME: usize = 40;

#[derive(Default)]
pub struct ViewEditor {
    pub open: bool,
    /// Opened this frame: the key that opened it must not close it.
    fresh: bool,
    draft: Filter,
    /// The name of the view being edited; `None` for a new one.
    editing: Option<String>,
    suggestions: Suggestions,
    /// The counts, and the draft they were counted for.
    counted: Option<(Filter, Counts)>,
}

/// The values the library holds, offered for a rule's value.
#[derive(Default)]
struct Suggestions {
    genres: Vec<String>,
    artists: Vec<String>,
    album_artists: Vec<String>,
    albums: Vec<String>,
    folders: Vec<String>,
    /// Each album's artist, for the counts.
    artist_of_album: Vec<ArtistId>,
}

/// How many artists, albums and tracks a draft leaves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    artists: usize,
    albums: usize,
    tracks: usize,
}

impl Suggestions {
    fn of(library: &Library, folders: &[PathBuf]) -> Suggestions {
        fn distinct<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
            let mut v: Vec<String> = values
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            v.sort_by(|a, b| natural_cmp(a, b));
            v.dedup();
            v
        }
        let tracks = &library.tracks;
        let mut artist_of_album = vec![0; library.albums.len()];
        for (i, a) in library.artists.iter().enumerate() {
            for &album in &a.albums {
                artist_of_album[album as usize] = i as ArtistId;
            }
        }
        // The library's folders and the artists' folders under them.
        let folder_names = folders
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .chain(library.albums.iter().filter_map(|a| {
                a.folder
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
            }));
        let folders: Vec<String> = folder_names.collect();
        Suggestions {
            genres: distinct(tracks.iter().map(|t| t.genre.as_str())),
            artists: distinct(
                tracks
                    .iter()
                    .flat_map(|t| [t.artist.as_str(), t.album_artist.as_str()]),
            ),
            album_artists: distinct(tracks.iter().map(|t| t.album_artist.as_str())),
            albums: distinct(library.albums.iter().map(|a| a.title.as_str())),
            folders: distinct(folders.iter().map(String::as_str)),
            artist_of_album,
        }
    }

    fn for_field(&self, field: Field) -> Option<&[String]> {
        let list = match field {
            Field::Genre => &self.genres,
            Field::Artist => &self.artists,
            Field::AlbumArtist => &self.album_artists,
            Field::Album => &self.albums,
            Field::Folder => &self.folders,
            _ => return None,
        };
        (!list.is_empty()).then_some(list.as_slice())
    }
}

impl ViewEditor {
    /// Opens the editor for a new view.
    pub fn open_new(&mut self, library: &Library, folders: &[PathBuf]) {
        let draft = Filter {
            name: String::new(),
            any: false,
            rules: vec![Rule::default()],
        };
        self.start(library, folders, draft, None);
    }

    /// Opens the editor on `filter`, an existing view.
    pub fn open_edit(&mut self, library: &Library, folders: &[PathBuf], filter: Filter) {
        let name = filter.name.clone();
        self.start(library, folders, filter, Some(name));
    }

    fn start(
        &mut self,
        library: &Library,
        folders: &[PathBuf],
        draft: Filter,
        editing: Option<String>,
    ) {
        self.open = true;
        self.fresh = true;
        self.draft = draft;
        self.editing = editing;
        self.suggestions = Suggestions::of(library, folders);
        self.counted = None;
    }

    /// The counts for the draft as it is, counted anew only when it
    /// changed.
    fn count(&mut self, library: &Library) -> Counts {
        if let Some((draft, counts)) = &self.counted
            && *draft == self.draft
        {
            return *counts;
        }
        let compiled = self.draft.compile(now());
        let mut albums = vec![false; library.albums.len()];
        let mut tracks = 0;
        for (t, track) in library.tracks.iter().enumerate() {
            if compiled.matches(track) {
                tracks += 1;
                albums[library.album_of[t] as usize] = true;
            }
        }
        let mut artists = vec![false; library.artists.len()];
        for (a, &has) in albums.iter().enumerate() {
            if has {
                artists[self.suggestions.artist_of_album[a] as usize] = true;
            }
        }
        let counts = Counts {
            artists: artists.iter().filter(|&&b| b).count(),
            albums: albums.iter().filter(|&&b| b).count(),
            tracks,
        };
        self.counted = Some((self.draft.clone(), counts));
        counts
    }

    /// Draws the editor while it is open; true when the views changed
    /// (saved or deleted).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        library: &Library,
        frame: &eframe::Frame,
    ) -> bool {
        if !self.open {
            return false;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let counts = self.count(library);
        let name = self.draft.name.trim();
        let taken = settings
            .views
            .iter()
            .any(|v| v.name == name && self.editing.as_deref() != Some(v.name.as_str()));
        let valid = self.draft.is_complete() && !taken;
        let (mut close, mut save, mut delete) = (false, false, false);
        let modal = egui::Modal::new(egui::Id::new("view_editor"))
            .frame(
                egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
                    .fill(palette::panel_bg())
                    .inner_margin(egui::Margin::same(20))
                    .corner_radius(10.0),
            )
            .show(ctx, |ui| {
                round_widgets(ui);
                let width = super::modal_size(ctx, vec2(WIDTH, 400.0), 12.0).x;
                ui.set_width(width);
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);

                ui.horizontal(|ui| {
                    let title = match &self.editing {
                        Some(_) => tr!("View", "Представление"),
                        None => tr!("New view", "Новое представление"),
                    };
                    ui.label(
                        RichText::new(title)
                            .size(20.0)
                            .strong()
                            .color(palette::text_bright()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        close |= super::close_cross(ui);
                    });
                });
                ui.add_space(6.0);

                // The name.
                ui.horizontal(|ui| {
                    ui.add_sized(
                        vec2(70.0, ui.spacing().interact_size.y),
                        egui::Label::new(tr!("Name", "Название")),
                    );
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.draft.name)
                            .char_limit(MAX_NAME)
                            .hint_text(tr!("Soundtracks", "Саундтреки"))
                            .desired_width(ui.available_width()),
                    );
                    if fresh && self.editing.is_none() {
                        field.request_focus();
                    }
                    super::take_leaving_key(ui, &field);
                });
                if taken {
                    ui.label(
                        RichText::new(tr!(
                            "A view of this name exists.",
                            "Представление с таким названием уже есть."
                        ))
                        .size(12.5)
                        .color(palette::error()),
                    );
                }
                ui.add_space(8.0);

                // The rules.
                ui.horizontal(|ui| {
                    ui.label(tr!(
                        "Tracks for which hold",
                        "Треки, для которых выполняются"
                    ));
                    ui.radio_value(
                        &mut self.draft.any,
                        false,
                        tr!("all the rules", "все условия"),
                    );
                    ui.radio_value(&mut self.draft.any, true, tr!("any rule", "любое условие"));
                });
                ui.add_space(2.0);
                let room = (ctx.content_rect().height() - 360.0).clamp(80.0, 400.0);
                egui::ScrollArea::vertical()
                    .id_salt("view_rules")
                    .max_height(room)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let mut remove = None;
                        let several = self.draft.rules.len() > 1;
                        for (i, rule) in self.draft.rules.iter_mut().enumerate() {
                            if rule_row(ui, i, rule, &self.suggestions, several, frame) {
                                remove = Some(i);
                            }
                        }
                        if let Some(i) = remove {
                            self.draft.rules.remove(i);
                        }
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.draft.rules.len() < MAX_RULES,
                            egui::Button::new(tr!("+ Add a rule", "+ Добавить условие")),
                        )
                        .clicked()
                    {
                        let field = self.draft.rules.last().map_or(Field::Genre, |r| r.field);
                        self.draft.rules.push(Rule {
                            field,
                            op: field.ops()[0],
                            ..Rule::default()
                        });
                    }
                });
                ui.add_space(10.0);
                ui.label(
                    RichText::new(tr!(
                        format!(
                            "Leaves {} · {} · {}",
                            crate::i18n::count(counts.artists as u64, ["artist", "artists"], ["", "", ""]),
                            crate::i18n::count(counts.albums as u64, ["album", "albums"], ["", "", ""]),
                            crate::i18n::count(counts.tracks as u64, ["track", "tracks"], ["", "", ""]),
                        ),
                        format!(
                            "Остаётся {} · {} · {}",
                            crate::i18n::count(
                                counts.artists as u64,
                                ["", ""],
                                ["исполнитель", "исполнителя", "исполнителей"]
                            ),
                            crate::i18n::count(
                                counts.albums as u64,
                                ["", ""],
                                ["альбом", "альбома", "альбомов"]
                            ),
                            crate::i18n::count(
                                counts.tracks as u64,
                                ["", ""],
                                ["трек", "трека", "треков"]
                            ),
                        )
                    ))
                    .color(palette::text_weak()),
                );
                ui.add_space(12.0);

                // The buttons.
                ui.horizontal(|ui| {
                    if self.editing.is_some() {
                        delete = ui
                            .button(tr!("Delete", "Удалить"))
                            .on_hover_text(tr!(
                                "Remove this view (the music stays)",
                                "Убрать это представление (музыка остаётся)"
                            ))
                            .clicked();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        save |= ui
                            .add_enabled(valid, egui::Button::new("OK").min_size(vec2(80.0, 0.0)))
                            .clicked();
                        close |= ui
                            .add(egui::Button::new(tr!("Cancel", "Отмена")).min_size(vec2(80.0, 0.0)))
                            .clicked();
                    });
                });
            });
        if !fresh && modal.is_top_modal && !modal.any_popup_open {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                close = true;
            }
            if valid
                && ctx.memory(|m| m.focused().is_none())
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
            {
                save = true;
            }
        }

        let mut changed = false;
        if save && valid {
            let mut saved = self.draft.clone();
            saved.name = saved.name.trim().to_owned();
            for r in &mut saved.rules {
                r.value = r.value.trim().to_owned();
                r.to = r.to.trim().to_owned();
            }
            match &self.editing {
                Some(old) => {
                    match settings.views.iter_mut().find(|v| v.name == *old) {
                        Some(v) => *v = saved.clone(),
                        None => settings.views.push(saved.clone()),
                    }
                    if settings.view == *old {
                        settings.view = saved.name;
                    }
                }
                None => {
                    settings.view = saved.name.clone();
                    settings.views.push(saved);
                }
            }
            changed = true;
            close = true;
        }
        if delete && let Some(old) = &self.editing {
            settings.views.retain(|v| v.name != *old);
            if settings.view == *old {
                settings.view.clear();
            }
            changed = true;
            close = true;
        }
        if close {
            self.open = false;
        }
        changed
    }
}

/// One rule's row: the field, the operation, the value as the field
/// takes it, and a minus that removes the row (true when clicked).
fn rule_row(
    ui: &mut Ui,
    i: usize,
    rule: &mut Rule,
    suggestions: &Suggestions,
    removable: bool,
    frame: &eframe::Frame,
) -> bool {
    let mut remove = false;
    ui.horizontal(|ui| {
        let mut field = rule.field;
        egui::ComboBox::from_id_salt(("view_field", i))
            .width(165.0)
            .selected_text(field.label())
            .show_ui(ui, |ui| {
                for f in Field::ALL {
                    ui.selectable_value(&mut field, f, f.label());
                }
            });
        if field != rule.field {
            if field.kind() != rule.field.kind() {
                rule.value.clear();
                rule.to.clear();
            }
            rule.field = field;
            if !field.ops().contains(&rule.op) {
                rule.op = field.ops()[0];
            }
        }
        let kind = rule.field.kind();
        egui::ComboBox::from_id_salt(("view_op", i))
            .width(135.0)
            .selected_text(rule.op.label(kind))
            .show_ui(ui, |ui| {
                for &op in rule.field.ops() {
                    ui.selectable_value(&mut rule.op, op, op.label(kind));
                }
            });

        let spacing = ui.spacing().item_spacing.x;
        // The value takes the room left of the minus.
        let room = ui.available_width() - TOOL - spacing;
        match kind {
            Kind::Text | Kind::Folder if rule.op.values() > 0 => {
                let list = suggestions.for_field(rule.field);
                let browse = kind == Kind::Folder;
                let tools = usize::from(list.is_some()) + usize::from(browse);
                let width = room - tools as f32 * (TOOL + spacing);
                let hint = match kind {
                    Kind::Folder => tr!(r"D:\Music\Soundtracks", r"D:\Музыка\Саундтреки"),
                    _ => "",
                };
                ui.add(
                    egui::TextEdit::singleline(&mut rule.value)
                        .hint_text(hint)
                        .desired_width(width.max(60.0)),
                );
                if let Some(list) = list {
                    suggestion_menu(ui, i, &mut rule.value, list);
                }
                if browse
                    && tool(ui, tr!("Choose a folder…", "Выбрать папку…"), true, Icon::Folder).clicked()
                    && let Some(folder) = rfd::FileDialog::new()
                        .set_title(tr!("Folder of the view", "Папка представления"))
                        .set_parent(frame)
                        .pick_folder()
                {
                    rule.value = folder.to_string_lossy().into_owned();
                }
            }
            Kind::Number => {
                number_field(ui, &mut rule.value);
                if rule.op == Op::Between {
                    ui.label("–");
                    number_field(ui, &mut rule.to);
                }
            }
            _ => {}
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            remove = tool(
                ui,
                tr!("Remove the rule", "Убрать условие"),
                removable,
                Icon::Minus,
            )
            .clicked();
        });
    });
    remove
}

/// A field for a number, red while it is not one.
fn number_field(ui: &mut Ui, value: &mut String) {
    let bad = !value.trim().is_empty() && crate::library::filter::parse_number(value).is_none();
    let mut edit = egui::TextEdit::singleline(value).desired_width(70.0);
    if bad {
        edit = edit.text_color(palette::error());
    }
    ui.add(edit);
}

/// A small button with a triangle that lists the library's `values` for
/// the field: those containing what is typed, else all; a click takes one.
fn suggestion_menu(ui: &mut Ui, i: usize, value: &mut String, values: &[String]) {
    let (rect, response) = ui.allocate_exact_size(vec2(TOOL, 24.0), Sense::click());
    let colour = if response.hovered() {
        palette::text_bright()
    } else {
        palette::text()
    };
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, palette::hover());
    }
    let c = rect.center();
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            pos2(c.x - 4.0, c.y - 2.0),
            pos2(c.x + 4.0, c.y - 2.0),
            pos2(c.x, c.y + 2.5),
        ],
        colour,
        Stroke::NONE,
    ));
    let typed = search_form(value.trim());
    egui::Popup::menu(&response)
        .id(egui::Id::new(("view_values", i)))
        .show(|ui| {
            ui.set_min_width(280.0);
            let matching: Vec<&String> = values
                .iter()
                .filter(|v| typed.is_empty() || search_form(v).contains(typed.as_str()))
                .collect();
            let shown = if matching.is_empty() {
                values.iter().collect()
            } else {
                matching
            };
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for v in shown {
                        if ui.button(v).clicked() {
                            *value = v.clone();
                            ui.close();
                        }
                    }
                });
        });
    response.on_hover_text(tr!("Values in the library", "Значения в библиотеке"));
}

/// Unix seconds now.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
