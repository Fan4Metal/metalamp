//! The media library view: a search field over three panes,
//! the artists on the left ("All" first), the albums at the top right (a
//! grid of covers or a table), the tracks under them. Choosing an artist
//! narrows the albums, an album the tracks; the search narrows all three
//! as it is typed (every word must be in the track's artist, album, title,
//! genre or year).
//!
//! What is shown is a [`View`], built only when the library, the search,
//! the choice or a sort changed (a few milliseconds for 5000 tracks): the
//! panes then only paint the rows in sight.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align2, FontId, Key, Modifiers, Rect, Sense, Ui, pos2, vec2};

use crate::covers::Covers;
use crate::format::{self, natural_cmp};
use crate::library::{AlbumId, ArtistId, Library, TrackId, unknown_artist};
use crate::palette;
use crate::settings::{AlbumView, Settings};
use crate::ui::icons::{self, Icon};
use crate::ui::table::{self, Column, Sort, Table};

const TOP_BAR: f32 = 36.0;
const SPLITTER: f32 = 5.0;
/// An album's cell in the grid, and its cover's side.
/// The side of a cover in the grid: `settings.cover_size` within these;
/// its cell is 16 points wider and 48 taller (the two lines of text).
pub const MIN_COVER: f32 = 96.0;
pub const MAX_COVER: f32 = 256.0;
/// What Ctrl and a wheel notch over the grid change the side by.
const COVER_STEP: f32 = 16.0;
/// The sizes the covers' button offers by the right button (English and
/// Russian names, the side).
const COVER_PRESETS: [((&str, &str), f32); 4] = [
    (("Small", "Мелкие"), 112.0),
    (("Medium", "Средние"), 152.0),
    (("Large", "Крупные"), 208.0),
    (("Largest", "Самые крупные"), 256.0),
];

/// What the library asks the app to do.
#[derive(Debug)]
pub enum LibraryAction {
    /// Replace the playlist with `tracks` and play the one at `start`.
    Play {
        tracks: Vec<TrackId>,
        start: usize,
    },
    /// Add `tracks` to the playlist's end.
    Enqueue(Vec<TrackId>),
    Rescan,
    AddFolder,
    RemoveFolder(usize),
    /// Open the file info window for a track.
    Info(TrackId),
    /// Open the settings window.
    Settings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pane {
    #[default]
    Artists,
    Albums,
    Tracks,
}

/// An artist in the view: how many of its albums and tracks the search
/// leaves.
pub struct ArtistRow {
    pub id: ArtistId,
    pub albums: u32,
    pub tracks: u32,
}

/// What the panes show.
pub struct View {
    pub artists: Vec<ArtistRow>,
    pub albums: Vec<AlbumId>,
    pub tracks: Vec<TrackId>,
    /// The sums of `tracks`, for the status line.
    pub length_ms: u64,
    pub bytes: u64,
    /// How long building the view took, shown in the status line.
    pub took: Duration,
    /// The chosen artist's and album's rows in `artists` and `albums`.
    pub artist_row: Option<usize>,
    pub album_row: Option<usize>,
    /// Albums in all the view's artists (the "All" row).
    pub all_albums: usize,
}

#[derive(Clone, PartialEq)]
struct ViewKey {
    library: usize,
    query: String,
    artist: Option<String>,
    album: Option<String>,
    sorts: [Option<Sort>; 3],
}

pub struct LibraryView {
    pub query: String,
    /// The chosen artist (lower-case name) and album (`Album::key`): kept
    /// by name, so a choice survives a new scan.
    artist: Option<String>,
    album: Option<String>,
    pub view: Option<View>,
    key: Option<ViewKey>,
    /// Selected rows of the tracks' table, and where a Shift range starts.
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    /// The pane the arrow keys move in.
    pub focus: Pane,
    pub covers: Covers,
    /// Focus the search field in the next frame (Ctrl+F).
    pub focus_search: bool,
    /// Rows to scroll into sight in the next frame.
    scroll: [Option<usize>; 3],
    /// The tracks' table's first row after a new album or artist.
    reset_tracks: bool,
    /// Play every track shown once the view is built (an artist double
    /// clicked).
    play_shown: bool,
    /// The artists' pane in the frame before, for `pointer_over_artists`.
    artists_rect: Option<Rect>,
    /// Letters typed over the artists' pane go to an artist (set by the
    /// app before its own keys, which then leave the letters alone).
    pub letters: bool,
    typed: TypeAhead,
}

/// The letters typed in a row, as in Windows' lists: each within
/// `TypeAhead::PAUSE` of the one before adds to the text looked for.
#[derive(Default)]
struct TypeAhead {
    text: String,
    at: Option<Instant>,
}

impl TypeAhead {
    const PAUSE: Duration = Duration::from_secs(1);

    /// Takes the letters typed this frame; true when there were any.
    fn feed(&mut self, ui: &Ui) -> bool {
        let typed: String = ui.input(|i| {
            if i.modifiers.alt || i.modifiers.command {
                return String::new();
            }
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect()
        });
        let typed: String = typed.chars().filter(|c| !c.is_whitespace()).collect();
        if typed.is_empty() {
            return false;
        }
        let now = Instant::now();
        if self
            .at
            .is_none_or(|at| now.duration_since(at) > Self::PAUSE)
        {
            self.text.clear();
        }
        self.at = Some(now);
        self.text.extend(typed.chars().flat_map(char::to_lowercase));
        true
    }
}

/// The place among `names` (lower-case, in the order shown) that typing
/// `typed` (lower-case) goes to: the first name starting with it; one
/// letter typed again and again goes on through the names starting with
/// it, as in Windows' lists.
fn type_ahead(names: &[String], typed: &str) -> Option<usize> {
    let mut chars = typed.chars();
    let first = chars.next()?;
    let repeats = chars.clone().count();
    if repeats > 0 && chars.all(|c| c == first) {
        let starting: Vec<usize> = (0..names.len())
            .filter(|&i| names[i].starts_with(first))
            .collect();
        if starting.is_empty() {
            return None;
        }
        return Some(starting[repeats % starting.len()]);
    }
    names.iter().position(|n| n.starts_with(typed))
}

impl LibraryView {
    pub fn new(ctx: &egui::Context) -> LibraryView {
        LibraryView {
            query: String::new(),
            artist: None,
            album: None,
            view: None,
            key: None,
            selected: BTreeSet::new(),
            anchor: None,
            focus: Pane::Artists,
            covers: Covers::new(ctx),
            focus_search: false,
            scroll: [None; 3],
            reset_tracks: false,
            play_shown: false,
            artists_rect: None,
            letters: false,
            typed: TypeAhead::default(),
        }
    }

    /// Whether the pointer is over the artists' pane (with nothing laid
    /// over it there), where typed letters go to an artist.
    pub fn pointer_over_artists(&self, ctx: &egui::Context) -> bool {
        let (Some(rect), Some(pos)) = (self.artists_rect, ctx.pointer_hover_pos()) else {
            return false;
        };
        rect.contains(pos)
            && ctx
                .layer_id_at(pos)
                .is_none_or(|l| l.order == egui::Order::Background)
    }

    /// The track selected last in the tracks' table, if any.
    pub fn selected_track(&self) -> Option<TrackId> {
        let row = self
            .anchor
            .filter(|a| self.selected.contains(a))
            .or_else(|| self.selected.first().copied())?;
        self.view.as_ref()?.tracks.get(row).copied()
    }

    /// The view for `library` as the search, the choice and the sorts are
    /// now; built again only when one of them changed.
    fn refresh(&mut self, library: &Arc<Library>, settings: &Settings) {
        let key = ViewKey {
            library: Arc::as_ptr(library) as usize,
            query: self.query.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            sorts: [
                settings.sorts.get("artists").copied(),
                settings.sorts.get("albums").copied(),
                settings.sorts.get("tracks").copied(),
            ],
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let same_tracks = self.key.as_ref().is_some_and(|k| {
            k.library == key.library
                && k.query == key.query
                && k.artist == key.artist
                && k.album == key.album
        });
        let view = build(library, &key);
        // A choice the search or a new scan took away is dropped.
        if view.artist_row.is_none() {
            self.artist = None;
        }
        if view.album_row.is_none() {
            self.album = None;
        }
        if !same_tracks {
            self.selected.clear();
            self.anchor = None;
        }
        self.view = Some(view);
        self.key = Some(ViewKey {
            artist: self.artist.clone(),
            album: self.album.clone(),
            ..key
        });
    }

    fn choose_artist(&mut self, library: &Library, artist: Option<ArtistId>) {
        self.artist = artist.map(|a| library.artists[a as usize].name.to_lowercase());
        self.album = None;
        self.reset_tracks = true;
    }

    fn choose_album(&mut self, library: &Library, album: Option<AlbumId>) {
        self.album = album.map(|a| library.albums[a as usize].key.clone());
        self.reset_tracks = true;
    }
}

/// Builds the view: the tracks the search leaves, the artists and albums
/// that have any of them, each sorted.
fn build(library: &Library, key: &ViewKey) -> View {
    let start = Instant::now();
    let words: Vec<String> = key
        .query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let shown: Vec<bool> = if words.is_empty() {
        vec![true; library.tracks.len()]
    } else {
        (0..library.tracks.len())
            .map(|t| library.matches(t as TrackId, &words))
            .collect()
    };
    let in_album: Vec<u32> = library
        .albums
        .iter()
        .map(|a| a.tracks.iter().filter(|&&t| shown[t as usize]).count() as u32)
        .collect();

    let mut artists: Vec<ArtistRow> = library
        .artists
        .iter()
        .enumerate()
        .filter_map(|(id, a)| {
            let albums = a
                .albums
                .iter()
                .filter(|&&al| in_album[al as usize] > 0)
                .count() as u32;
            let tracks: u32 = a.albums.iter().map(|&al| in_album[al as usize]).sum();
            (tracks > 0).then_some(ArtistRow {
                id: id as ArtistId,
                albums,
                tracks,
            })
        })
        .collect();
    if let Some(sort) = key.sorts[0] {
        artists.sort_by(|a, b| {
            let o = match sort.column {
                1 => a.albums.cmp(&b.albums),
                2 => a.tracks.cmp(&b.tracks),
                _ => natural_cmp(
                    &library.artists[a.id as usize].name,
                    &library.artists[b.id as usize].name,
                ),
            };
            if sort.descending { o.reverse() } else { o }
        });
    }
    let artist_row = key.artist.as_ref().and_then(|name| {
        artists
            .iter()
            .position(|a| library.artists[a.id as usize].name.to_lowercase() == *name)
    });
    let all_albums = in_album.iter().filter(|&&n| n > 0).count();

    let mut albums: Vec<AlbumId> = match artist_row {
        Some(row) => library.artists[artists[row].id as usize]
            .albums
            .iter()
            .copied()
            .filter(|&a| in_album[a as usize] > 0)
            .collect(),
        None => {
            // The artists' order, each one's albums by year.
            let mut all = Vec::with_capacity(all_albums);
            for a in &library.artists {
                all.extend(
                    a.albums
                        .iter()
                        .copied()
                        .filter(|&al| in_album[al as usize] > 0),
                );
            }
            all
        }
    };
    if let Some(sort) = key.sorts[1] {
        albums.sort_by(|&x, &y| {
            let (a, b) = (&library.albums[x as usize], &library.albums[y as usize]);
            let o = match sort.column {
                1 => natural_cmp(&a.title, &b.title),
                2 => a.year.cmp(&b.year),
                3 => in_album[x as usize].cmp(&in_album[y as usize]),
                4 => natural_cmp(&a.genre, &b.genre),
                5 => a.length_ms.cmp(&b.length_ms),
                6 => a.size.cmp(&b.size),
                _ => natural_cmp(&a.artist, &b.artist),
            }
            .then_with(|| a.year.cmp(&b.year))
            .then_with(|| natural_cmp(&a.title, &b.title));
            if sort.descending { o.reverse() } else { o }
        });
    }
    let album_row = key.album.as_ref().and_then(|k| {
        albums
            .iter()
            .position(|&a| library.albums[a as usize].key == *k)
    });

    let mut tracks: Vec<TrackId> = Vec::new();
    let from: Vec<AlbumId> = match album_row {
        Some(row) => vec![albums[row]],
        None => albums.clone(),
    };
    for a in from {
        tracks.extend(
            library.albums[a as usize]
                .tracks
                .iter()
                .copied()
                .filter(|&t| shown[t as usize]),
        );
    }
    if let Some(sort) = key.sorts[2] {
        tracks.sort_by(|&x, &y| {
            let (a, b) = (&library.tracks[x as usize], &library.tracks[y as usize]);
            let o = match sort.column {
                0 => (a.disc, a.track).cmp(&(b.disc, b.track)),
                1 => natural_cmp(&a.title, &b.title),
                2 => a.duration_ms.cmp(&b.duration_ms),
                3 => natural_cmp(&a.album, &b.album),
                4 => a.year.cmp(&b.year),
                5 => natural_cmp(&a.artist, &b.artist),
                6 => natural_cmp(&a.genre, &b.genre),
                _ => a.bitrate.cmp(&b.bitrate),
            };
            if sort.descending { o.reverse() } else { o }
        });
    }
    let length_ms = tracks
        .iter()
        .map(|&t| u64::from(library.tracks[t as usize].duration_ms))
        .sum();
    let bytes = tracks
        .iter()
        .map(|&t| library.tracks[t as usize].size)
        .sum();
    View {
        artists,
        albums,
        tracks,
        length_ms,
        bytes,
        took: start.elapsed(),
        artist_row,
        album_row,
        all_albums,
    }
}

/// The name shown for an artist or album without one.
fn shown_name(name: &str, unknown: &str) -> String {
    if name.is_empty() {
        unknown.to_owned()
    } else {
        name.to_owned()
    }
}

fn unknown_album() -> &'static str {
    tr!("Unknown album", "Неизвестный альбом")
}

/// The whole library pane: the bar with the search, then the three panes.
pub fn show(
    ui: &mut Ui,
    lv: &mut LibraryView,
    library: &Arc<Library>,
    settings: &mut Settings,
    scanning: bool,
    playing: Option<TrackId>,
    keys: bool,
) -> (Vec<LibraryAction>, bool) {
    let mut actions = Vec::new();
    let mut changed = false;
    lv.refresh(library, settings);
    if std::mem::take(&mut lv.play_shown)
        && let Some(view) = &lv.view
        && !view.tracks.is_empty()
    {
        actions.push(LibraryAction::Play {
            tracks: view.tracks.clone(),
            start: 0,
        });
    }
    let full = ui.available_rect_before_wrap();
    ui.painter().rect_filled(full, 0.0, palette::panel_bg());

    let bar = Rect::from_min_size(full.min, vec2(full.width(), TOP_BAR));
    top_bar(ui, bar, lv, settings, &mut actions, &mut changed);

    let rest = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
    if library.tracks.is_empty() {
        let text = if scanning {
            tr!("Reading the library…", "Чтение библиотеки…")
        } else {
            tr!(
                "No music found in the library's folders",
                "В папках библиотеки нет музыки"
            )
        };
        ui.painter().text(
            rest.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(14.0),
            palette::text_weak(),
        );
        ui.advance_cursor_after_rect(full);
        return (actions, changed);
    }

    // The artists on the left, the albums over the tracks on the right,
    // with strips between them that move the borders.
    let width = settings
        .artists_width
        .clamp(160.0, (rest.width() * 0.5).max(160.0));
    let left = Rect::from_min_size(rest.min, vec2(width, rest.height()));
    let split_v = Rect::from_min_size(
        pos2(left.right(), rest.top()),
        vec2(SPLITTER, rest.height()),
    );
    let right = Rect::from_min_max(pos2(split_v.right(), rest.top()), rest.max);
    let height = settings
        .tracks_height
        .clamp(100.0, (right.height() - 140.0).max(100.0));
    let albums = Rect::from_min_max(
        right.min,
        pos2(right.right(), right.bottom() - height - SPLITTER),
    );
    let split_h = Rect::from_min_max(
        pos2(right.left(), albums.bottom()),
        pos2(right.right(), albums.bottom() + SPLITTER),
    );
    let tracks = Rect::from_min_max(pos2(right.left(), split_h.bottom()), right.max);

    if let Some(d) = splitter(ui, split_v, "split_artists", true) {
        settings.artists_width = (width + d).max(160.0);
        changed = true;
    }
    if let Some(d) = splitter(ui, split_h, "split_tracks", false) {
        settings.tracks_height = (height - d).max(100.0);
        changed = true;
    }

    let keys_free = keys && !ui.ctx().memory(|m| m.focused().is_some());
    artists_pane(
        ui,
        left,
        lv,
        library,
        settings,
        keys_free,
        &mut actions,
        &mut changed,
    );
    match settings.album_view {
        AlbumView::Grid => albums_grid(
            ui,
            albums,
            lv,
            library,
            settings,
            keys_free,
            &mut actions,
            &mut changed,
        ),
        AlbumView::Table => albums_table(
            ui,
            albums,
            lv,
            library,
            settings,
            keys_free,
            &mut actions,
            &mut changed,
        ),
    }
    tracks_pane(
        ui,
        tracks,
        lv,
        library,
        settings,
        keys_free,
        playing,
        &mut actions,
        &mut changed,
    );
    ui.advance_cursor_after_rect(full);
    (actions, changed)
}

/// A strip between two panes; how far it was dragged this frame.
fn splitter(ui: &mut Ui, rect: Rect, id: &str, vertical: bool) -> Option<f32> {
    let resp = ui.interact(rect, ui.id().with(id), Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(if vertical {
            egui::CursorIcon::ResizeHorizontal
        } else {
            egui::CursorIcon::ResizeVertical
        });
    }
    let d = resp.drag_delta();
    let d = if vertical { d.x } else { d.y };
    (resp.dragged() && d != 0.0).then_some(d)
}

fn top_bar(
    ui: &mut Ui,
    bar: Rect,
    lv: &mut LibraryView,
    settings: &mut Settings,
    actions: &mut Vec<LibraryAction>,
    changed: &mut bool,
) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(vec2(10.0, 4.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut child;
    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    icons::paint(
        ui.painter(),
        Icon::Search,
        r.center(),
        icons::SIZE,
        palette::text_weak(),
        palette::panel_bg(),
    );
    let edit = egui::TextEdit::singleline(&mut lv.query)
        .hint_text(tr!("Search (Ctrl+F)", "Поиск (Ctrl+F)"))
        .desired_width(320.0)
        .id(ui.id().with("library_search"));
    let resp = ui.add(edit);
    if std::mem::take(&mut lv.focus_search) {
        resp.request_focus();
    }
    if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
        lv.query.clear();
        resp.surrender_focus();
    }
    if !lv.query.is_empty()
        && crate::ui::icon_button(
            ui,
            Icon::Clear,
            false,
            tr!("Clear the search", "Очистить поиск"),
        )
        .clicked()
    {
        lv.query.clear();
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if crate::ui::icon_button(
            ui,
            Icon::Settings,
            false,
            tr!("Settings (F5)", "Настройки (F5)"),
        )
        .clicked()
        {
            actions.push(LibraryAction::Settings);
        }
        ui.add_space(8.0);
        let folders = crate::ui::icon_button(
            ui,
            Icon::Library,
            false,
            tr!("The library's folders", "Папки библиотеки"),
        );
        egui::Popup::menu(&folders).show(|ui| {
            ui.set_min_width(260.0);
            for (i, folder) in settings.library_folders.iter().enumerate() {
                ui.horizontal(|ui| {
                    if crate::ui::icon_button(
                        ui,
                        Icon::Close,
                        false,
                        tr!("Remove from the library", "Убрать из библиотеки"),
                    )
                    .clicked()
                    {
                        actions.push(LibraryAction::RemoveFolder(i));
                        ui.close();
                    }
                    ui.label(folder.display().to_string());
                });
            }
            if !settings.library_folders.is_empty() {
                ui.separator();
            }
            if ui
                .button(tr!(
                    "Add a folder… (Ctrl+Shift+O)",
                    "Добавить папку… (Ctrl+Shift+O)"
                ))
                .clicked()
            {
                actions.push(LibraryAction::AddFolder);
                ui.close();
            }
        });
        if crate::ui::icon_button(
            ui,
            Icon::Refresh,
            false,
            tr!(
                "Read the folders again (Ctrl+R)",
                "Перечитать папки (Ctrl+R)"
            ),
        )
        .clicked()
        {
            actions.push(LibraryAction::Rescan);
        }
        if crate::ui::icon_button(
            ui,
            Icon::Contents,
            settings.playlist_visible,
            tr!("The playlist (Alt+E)", "Плейлист (Alt+E)"),
        )
        .clicked()
        {
            settings.playlist_visible = !settings.playlist_visible;
            *changed = true;
        }
        ui.add_space(8.0);
        let table = settings.album_view == AlbumView::Table;
        if crate::ui::icon_button(
            ui,
            Icon::List,
            table,
            tr!("Albums as a table", "Альбомы таблицей"),
        )
        .clicked()
        {
            settings.album_view = AlbumView::Table;
            *changed = true;
        }
        let gallery = crate::ui::icon_button(
            ui,
            Icon::Gallery,
            !table,
            tr!(
                "Albums as covers; the right button: their size",
                "Альбомы обложками; правая кнопка: их размер"
            ),
        );
        if gallery.clicked() {
            settings.album_view = AlbumView::Grid;
            *changed = true;
        }
        // The right button: the covers' size, as presets and a slider.
        egui::Popup::context_menu(&gallery)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(220.0);
                for (name, size) in COVER_PRESETS {
                    let on = (settings.cover_size - size).abs() < 0.5;
                    if ui
                        .selectable_label(on, format!("{} · {size:.0}", tr!(name.0, name.1)))
                        .clicked()
                    {
                        settings.cover_size = size;
                        settings.album_view = AlbumView::Grid;
                        *changed = true;
                        ui.close();
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(tr!("Size", "Размер"));
                    ui.spacing_mut().slider_width = 130.0;
                    if ui
                        .add(
                            egui::Slider::new(&mut settings.cover_size, MIN_COVER..=MAX_COVER)
                                .step_by(8.0)
                                .fixed_decimals(0),
                        )
                        .changed()
                    {
                        settings.album_view = AlbumView::Grid;
                        *changed = true;
                    }
                });
            });
    });
}

/// A pane's table in `rect`, the widths and sort kept under `name`.
fn pane_ui(ui: &mut Ui, rect: Rect, name: &str) -> Ui {
    ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(name))
}

fn widths(settings: &mut Settings, name: &str, default: &[f32]) -> Vec<f32> {
    settings
        .columns
        .get(name)
        .filter(|w| w.len() == default.len())
        .cloned()
        .unwrap_or_else(|| default.to_vec())
}

/// Rows a screen holds in a table of `rect`.
fn page(rect: Rect) -> usize {
    ((rect.height() - table::HEADER_HEIGHT) / table::ROW_HEIGHT).max(1.0) as usize
}

#[allow(clippy::too_many_arguments)]
fn artists_pane(
    ui: &mut Ui,
    rect: Rect,
    lv: &mut LibraryView,
    library: &Library,
    settings: &mut Settings,
    keys_free: bool,
    actions: &mut Vec<LibraryAction>,
    changed: &mut bool,
) {
    lv.artists_rect = Some(rect);
    let Some(view) = &lv.view else { return };
    let columns = [
        Column::left(tr!("Artist", "Исполнитель")),
        Column::right(tr!("Albums", "Альбомы")),
        Column::right(tr!("Tracks", "Треки")),
    ];
    let mut w = widths(settings, "artists", &[150.0, 55.0, 55.0]);
    let mut sort = settings.sorts.get("artists").copied();
    let rows = view.artists.len() + 1;
    let current = view.artist_row.map_or(0, |r| r + 1);
    let mut ui = pane_ui(ui, rect, "artists");
    let total_tracks: u32 = view.artists.iter().map(|a| a.tracks).sum();
    let resp = Table {
        id: egui::Id::new("artists"),
        columns: &columns,
        widths: &mut w,
        sort: &mut sort,
        rows,
        scroll_to: lv.scroll[0].take(),
    }
    .show(
        &mut ui,
        &mut |row, col| {
            if row == 0 {
                return match col {
                    0 => tr!(
                        format!("All ({} artists)", view.artists.len()),
                        format!(
                            "Все ({})",
                            crate::i18n::count(
                                view.artists.len() as u64,
                                ["artist", "artists"],
                                ["исполнитель", "исполнителя", "исполнителей"]
                            )
                        )
                    ),
                    1 => view.all_albums.to_string(),
                    _ => total_tracks.to_string(),
                };
            }
            let a = &view.artists[row - 1];
            match col {
                0 => shown_name(&library.artists[a.id as usize].name, unknown_artist()),
                1 => a.albums.to_string(),
                _ => a.tracks.to_string(),
            }
        },
        &|row| (row == current, false),
    );
    if resp.focused {
        lv.focus = Pane::Artists;
    }
    let mut choose = resp.clicked.or(resp.double_clicked).or(resp.secondary);
    match context_menu(
        resp.area.as_ref(),
        resp.secondary.is_some(),
        "artists",
        false,
    ) {
        Some(MenuChoice::Play) => lv.play_shown = true,
        Some(MenuChoice::Enqueue) => actions.push(LibraryAction::Enqueue(view.tracks.clone())),
        Some(MenuChoice::ShowInFolder) => {
            if let Some(&t) = view.tracks.first() {
                show_folder_of(library, t);
            }
        }
        _ => {}
    }
    if keys_free
        && lv.focus == Pane::Artists
        && let Some(row) = table::key_move(&ui, Some(current), rows, page(rect))
        && row != current
    {
        choose = Some(row);
        lv.scroll[0] = Some(row);
    }
    // Letters typed over the pane: the first artist starting with them.
    if lv.letters && lv.typed.feed(&ui) {
        let names: Vec<String> = view
            .artists
            .iter()
            .map(|a| {
                shown_name(&library.artists[a.id as usize].name, unknown_artist()).to_lowercase()
            })
            .collect();
        if let Some(i) = type_ahead(&names, &lv.typed.text) {
            choose = Some(i + 1);
            lv.scroll[0] = Some(i + 1);
            lv.focus = Pane::Artists;
        }
    }
    let id_of = |row: usize| (row > 0).then(|| view.artists[row - 1].id);
    let enter = keys_free && lv.focus == Pane::Artists && ui.input(|i| i.key_pressed(Key::Enter));
    // The tracks of the artist chosen are known once the view is built
    // for it, in the next frame.
    let play = resp.double_clicked.is_some() || enter;
    if let Some(row) = choose {
        let id = id_of(row);
        lv.choose_artist(library, id);
    }
    if play {
        lv.play_shown = true;
    }
    if resp.sorted {
        store_sort(settings, "artists", sort);
        *changed = true;
    }
    if resp.resized {
        settings.columns.insert("artists".into(), w);
        *changed = true;
    }
}

fn store_sort(settings: &mut Settings, name: &str, sort: Option<Sort>) {
    match sort {
        Some(s) => settings.sorts.insert(name.into(), s),
        None => settings.sorts.remove(name),
    };
}

fn album_columns() -> [Column; 7] {
    [
        Column::left(tr!("Artist", "Исполнитель")),
        Column::left(tr!("Album", "Альбом")),
        Column::right(tr!("Year", "Год")),
        Column::right(tr!("Tracks", "Треки")),
        Column::left(tr!("Genre", "Жанр")),
        Column::right(tr!("Length", "Длительность")),
        Column::right(tr!("Size", "Размер")),
    ]
}

const ALBUM_WIDTHS: [f32; 7] = [150.0, 200.0, 50.0, 50.0, 100.0, 80.0, 80.0];

#[allow(clippy::too_many_arguments)]
fn albums_table(
    ui: &mut Ui,
    rect: Rect,
    lv: &mut LibraryView,
    library: &Library,
    settings: &mut Settings,
    keys_free: bool,
    actions: &mut Vec<LibraryAction>,
    changed: &mut bool,
) {
    let Some(view) = &lv.view else { return };
    let columns = album_columns();
    let mut w = widths(settings, "albums", &ALBUM_WIDTHS);
    let mut sort = settings.sorts.get("albums").copied();
    let rows = view.albums.len() + 1;
    let current = view.album_row.map_or(0, |r| r + 1);
    let mut ui = pane_ui(ui, rect, "albums");
    let resp = Table {
        id: egui::Id::new("albums"),
        columns: &columns,
        widths: &mut w,
        sort: &mut sort,
        rows,
        scroll_to: lv.scroll[1].take(),
    }
    .show(
        &mut ui,
        &mut |row, col| album_cell(library, view, row, col),
        &|row| (row == current, false),
    );
    if resp.focused {
        lv.focus = Pane::Albums;
    }
    let mut choose = resp.clicked.or(resp.double_clicked).or(resp.secondary);
    let menu_album = view.album_row.map(|r| view.albums[r]);
    album_menu(
        library,
        view,
        menu_album,
        context_menu(
            resp.area.as_ref(),
            resp.secondary.is_some(),
            "albums",
            false,
        ),
        actions,
    );
    if keys_free
        && lv.focus == Pane::Albums
        && let Some(row) = table::key_move(&ui, Some(current), rows, page(rect))
        && row != current
    {
        choose = Some(row);
        lv.scroll[1] = Some(row);
    }
    let album_of = |row: usize| (row > 0).then(|| view.albums[row - 1]);
    let enter = keys_free && lv.focus == Pane::Albums && ui.input(|i| i.key_pressed(Key::Enter));
    if let Some(row) = resp.double_clicked {
        actions.push(play_albums(library, view, album_of(row)));
    } else if enter {
        actions.push(play_albums(library, view, album_of(current)));
    }
    if let Some(row) = choose {
        let id = album_of(row);
        lv.choose_album(library, id);
    }
    if resp.sorted {
        store_sort(settings, "albums", sort);
        *changed = true;
    }
    if resp.resized {
        settings.columns.insert("albums".into(), w);
        *changed = true;
    }
}

/// What an album's context menu asked for: `album`, or every album
/// shown.
fn album_menu(
    library: &Library,
    view: &View,
    album: Option<AlbumId>,
    choice: Option<MenuChoice>,
    actions: &mut Vec<LibraryAction>,
) {
    match choice {
        Some(MenuChoice::Play) => actions.push(play_albums(library, view, album)),
        Some(MenuChoice::Enqueue) => {
            if let LibraryAction::Play { tracks, .. } = play_albums(library, view, album) {
                actions.push(LibraryAction::Enqueue(tracks));
            }
        }
        Some(MenuChoice::ShowInFolder) => {
            let first = match album {
                Some(a) => library.albums[a as usize].tracks.first().copied(),
                None => view.tracks.first().copied(),
            };
            if let Some(t) = first {
                show_folder_of(library, t);
            }
        }
        _ => {}
    }
}

fn show_folder_of(library: &Library, track: TrackId) {
    crate::win::show_in_folder(&library.tracks[track as usize].path);
}

/// Play `album`, or every album shown.
fn play_albums(library: &Library, view: &View, album: Option<AlbumId>) -> LibraryAction {
    let tracks = match album {
        Some(a) => library.albums[a as usize].tracks.clone(),
        None => view.tracks.clone(),
    };
    LibraryAction::Play { tracks, start: 0 }
}

/// The albums table's cell of `row` (0: "All") and `col`.
fn album_cell(library: &Library, view: &View, row: usize, col: usize) -> String {
    if row == 0 {
        return if col == 0 {
            tr!(
                format!("All ({} albums)", view.albums.len()),
                format!(
                    "Все ({})",
                    crate::i18n::count(
                        view.albums.len() as u64,
                        ["album", "albums"],
                        ["альбом", "альбома", "альбомов"]
                    )
                )
            )
        } else {
            String::new()
        };
    }
    let a = &library.albums[view.albums[row - 1] as usize];
    match col {
        0 => shown_name(&a.artist, unknown_artist()),
        1 => shown_name(&a.title, unknown_album()),
        2 => year(a.year),
        3 => a.tracks.len().to_string(),
        4 => a.genre.clone(),
        5 => format::time(a.length_ms),
        _ => format::size(a.size),
    }
}

fn year(y: u16) -> String {
    if y == 0 { String::new() } else { y.to_string() }
}

#[allow(clippy::too_many_arguments)]
fn albums_grid(
    ui: &mut Ui,
    rect: Rect,
    lv: &mut LibraryView,
    library: &Library,
    settings: &mut Settings,
    keys_free: bool,
    actions: &mut Vec<LibraryAction>,
    changed: &mut bool,
) {
    let Some(view) = &lv.view else { return };
    let ctx = ui.ctx().clone();
    // The same header as the table: it sorts the grid.
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), table::HEADER_HEIGHT));
    let columns = album_columns();
    let mut w = widths(settings, "albums", &ALBUM_WIDTHS);
    let mut sort = settings.sorts.get("albums").copied();
    // The edges fit the columns to the albums' table's texts.
    let painter = ui.painter().clone();
    let (sorted, resized) = table::header_row(
        ui,
        egui::Id::new("albums"),
        header,
        (header.left(), header.width()),
        &columns,
        &mut w,
        &mut sort,
        &mut |i| {
            table::fit_width(
                &painter,
                &columns[i].title,
                view.albums.len() + 1,
                &mut |row| album_cell(library, view, row, i),
            )
        },
    );
    if sorted {
        store_sort(settings, "albums", sort);
        *changed = true;
    }
    if resized {
        settings.columns.insert("albums".into(), w);
        *changed = true;
    }

    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    ui.painter().rect_filled(body, 0.0, palette::list_bg());
    // Ctrl and the wheel over the grid: larger or smaller covers (egui
    // keeps a scroll area still while Ctrl is held).
    if ui.rect_contains_pointer(body) && ui.input(|i| i.modifiers.command) {
        let notches = crate::ui::wheel_notches(ui);
        if notches != 0 {
            settings.cover_size =
                (settings.cover_size + COVER_STEP * notches as f32).clamp(MIN_COVER, MAX_COVER);
            *changed = true;
        }
    }
    let cover_side = settings.cover_size.clamp(MIN_COVER, MAX_COVER);
    let cell_size = vec2(cover_side + 16.0, cover_side + 48.0);
    // Room for the scroll bar, so that no cell goes under it.
    let bar = ui.spacing().scroll.allocated_width();
    let per_row = ((body.width() - 8.0 - bar) / cell_size.x).floor().max(1.0) as usize;
    let n = view.albums.len();
    let rows = n.div_ceil(per_row);
    let current = view.album_row;

    let mut moved = None;
    if keys_free && lv.focus == Pane::Albums && n > 0 {
        ui.input(|i| {
            let at = current.unwrap_or(0) as isize;
            let step = if i.key_pressed(Key::ArrowRight) {
                1
            } else if i.key_pressed(Key::ArrowLeft) {
                -1
            } else if i.key_pressed(Key::ArrowDown) {
                per_row as isize
            } else if i.key_pressed(Key::ArrowUp) {
                -(per_row as isize)
            } else if i.key_pressed(Key::Home) {
                -at
            } else if i.key_pressed(Key::End) {
                n as isize - 1 - at
            } else {
                0
            };
            if step != 0 || (current.is_none() && i.key_pressed(Key::ArrowDown)) {
                moved = Some((at + step).clamp(0, n as isize - 1) as usize);
            }
        });
    }
    let scroll_to = lv.scroll[1].take().or(moved);

    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt("albums_grid"));
    let mut area = egui::ScrollArea::vertical()
        .id_salt("albums_grid_scroll")
        .auto_shrink([false, false]);
    if let Some(i) = scroll_to {
        let top = (i / per_row) as f32 * cell_size.y;
        let offset = ctx
            .data(|d| d.get_temp::<f32>(egui::Id::new("albums_grid_offset")))
            .unwrap_or(0.0);
        if top < offset {
            area = area.vertical_scroll_offset(top);
        } else if top + cell_size.y > offset + body.height() {
            area = area.vertical_scroll_offset(top + cell_size.y - body.height());
        }
    }
    let mut clicked: Option<Option<usize>> = None;
    let mut double: Option<usize> = None;
    let mut focused = false;
    let mut secondary = false;
    let out = area.show_viewport(&mut child, |ui, viewport| {
        let height = (rows as f32 * cell_size.y + 8.0).max(viewport.height());
        let (space, area_resp) =
            ui.allocate_exact_size(vec2(viewport.width(), height), Sense::click());
        if area_resp.clicked() {
            clicked = Some(None);
            focused = true;
        }
        let first_row = (viewport.top() / cell_size.y).floor().max(0.0) as usize;
        let last_row = ((viewport.bottom() / cell_size.y).ceil() as usize).min(rows);
        let painter = ui.painter().clone();
        for row in first_row..last_row {
            for col in 0..per_row {
                let i = row * per_row + col;
                if i >= n {
                    break;
                }
                let album_id = view.albums[i];
                let album = &library.albums[album_id as usize];
                let cell = Rect::from_min_size(
                    space.min
                        + vec2(
                            8.0 + col as f32 * cell_size.x,
                            8.0 + row as f32 * cell_size.y,
                        ),
                    cell_size - vec2(8.0, 8.0),
                );
                let resp = ui.interact(
                    cell,
                    egui::Id::new(("album_cell", album_id)),
                    Sense::click(),
                );
                let selected = current == Some(i);
                if selected {
                    painter.rect_filled(cell, 4.0, palette::selected_bg());
                } else if resp.hovered() {
                    painter.rect_filled(cell, 4.0, palette::hover());
                }
                let cover = Rect::from_min_size(
                    pos2(cell.center().x - cover_side / 2.0, cell.top() + 4.0),
                    vec2(cover_side, cover_side),
                );
                match lv.covers.get(&ctx, library, album) {
                    Some(texture) => {
                        let [w, h] = texture.size();
                        // Not square: fit inside, centred.
                        let scale = cover_side / (w.max(h) as f32);
                        let size = vec2(w as f32 * scale, h as f32 * scale);
                        let r = Rect::from_center_size(cover.center(), size);
                        painter.image(
                            texture.id(),
                            r,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    None => {
                        painter.rect_filled(cover, 3.0, palette::card_bg());
                        if lv.covers.is_missing(library, album) {
                            icons::paint(
                                &painter,
                                Icon::Play,
                                cover.center(),
                                28.0,
                                palette::text_weak(),
                                palette::card_bg(),
                            );
                        }
                    }
                }
                if selected {
                    painter.rect_stroke(
                        cover,
                        3.0,
                        egui::Stroke::new(2.0, palette::accent()),
                        egui::StrokeKind::Outside,
                    );
                }
                let text_w = cell.width() - 8.0;
                let title = shown_name(&album.title, unknown_album());
                let second = if view.artist_row.is_some() {
                    year(album.year)
                } else {
                    let artist = shown_name(&album.artist, unknown_artist());
                    if album.year > 0 {
                        format!("{artist} · {}", album.year)
                    } else {
                        artist
                    }
                };
                let line = |text: String, size: f32, color| {
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        text,
                        FontId::proportional(size),
                        color,
                    );
                    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let g1 = line(
                    title,
                    13.0,
                    if selected {
                        palette::selected_text()
                    } else {
                        palette::list_text()
                    },
                );
                let g2 = line(second, 11.5, palette::text_weak());
                let y = cover.bottom() + 6.0;
                painter.galley(
                    pos2(cell.center().x - g1.size().x / 2.0, y),
                    g1,
                    palette::list_text(),
                );
                painter.galley(
                    pos2(cell.center().x - g2.size().x / 2.0, y + 17.0),
                    g2,
                    palette::text_weak(),
                );
                if crate::ui::double_clicked(&resp) {
                    double = Some(i);
                    focused = true;
                } else if resp.clicked() {
                    clicked = Some(Some(i));
                    focused = true;
                }
                if resp.secondary_clicked() {
                    clicked = Some(Some(i));
                    secondary = true;
                    focused = true;
                }
                resp.on_hover_text(format!(
                    "{}\n{}",
                    shown_name(&album.title, unknown_album()),
                    shown_name(&album.artist, unknown_artist())
                ));
            }
        }
        area_resp
    });
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("albums_grid_offset"), out.state.offset.y));
    let menu_album = match clicked {
        Some(choice) => choice.map(|i| view.albums[i]),
        None => current.map(|i| view.albums[i]),
    };
    album_menu(
        library,
        view,
        menu_album,
        context_menu(Some(&out.inner), secondary, "albums_grid", false),
        actions,
    );
    if focused {
        lv.focus = Pane::Albums;
    }
    if let Some(i) = moved {
        clicked = Some(Some(i));
    }
    let enter = keys_free && lv.focus == Pane::Albums && ui.input(|i| i.key_pressed(Key::Enter));
    if let Some(i) = double {
        actions.push(play_albums(library, view, Some(view.albums[i])));
        clicked = Some(Some(i));
    } else if enter {
        actions.push(play_albums(library, view, current.map(|i| view.albums[i])));
    }
    if let Some(choice) = clicked {
        let id = choice.map(|i| view.albums[i]);
        lv.choose_album(library, id);
    }
}

#[allow(clippy::too_many_arguments)]
fn tracks_pane(
    ui: &mut Ui,
    rect: Rect,
    lv: &mut LibraryView,
    library: &Library,
    settings: &mut Settings,
    keys_free: bool,
    playing: Option<TrackId>,
    actions: &mut Vec<LibraryAction>,
    changed: &mut bool,
) {
    if std::mem::take(&mut lv.reset_tracks) {
        lv.scroll[2] = Some(0);
    }
    let Some(view) = &lv.view else { return };
    let columns = [
        Column::right("#"),
        Column::left(tr!("Title", "Название")),
        Column::right(tr!("Length", "Длит.")),
        Column::left(tr!("Album", "Альбом")),
        Column::right(tr!("Year", "Год")),
        Column::left(tr!("Artist", "Исполнитель")),
        Column::left(tr!("Genre", "Жанр")),
        Column::right(tr!("Bit rate", "Битрейт")),
    ];
    let mut w = widths(
        settings,
        "tracks",
        &[44.0, 240.0, 60.0, 200.0, 50.0, 160.0, 100.0, 80.0],
    );
    let mut sort = settings.sorts.get("tracks").copied();
    let rows = view.tracks.len();
    let mut ui = pane_ui(ui, rect, "tracks");
    let selected = &lv.selected;
    let resp = Table {
        id: egui::Id::new("tracks"),
        columns: &columns,
        widths: &mut w,
        sort: &mut sort,
        rows,
        scroll_to: lv.scroll[2].take(),
    }
    .show(
        &mut ui,
        &mut |row, col| {
            let t = &library.tracks[view.tracks[row] as usize];
            match col {
                0 => match (t.disc, t.track) {
                    (_, 0) => String::new(),
                    (d, n) if d > 1 => format!("{d}-{n:02}"),
                    (_, n) => n.to_string(),
                },
                1 => t.title.clone(),
                2 => format::time(u64::from(t.duration_ms)),
                3 => shown_name(&t.album, unknown_album()),
                4 => year(t.year),
                5 => shown_name(&t.artist, unknown_artist()),
                6 => t.genre.clone(),
                _ => format::bitrate(t.bitrate),
            }
        },
        &|row| (selected.contains(&row), Some(view.tracks[row]) == playing),
    );
    if resp.focused {
        lv.focus = Pane::Tracks;
    }
    let modifiers = ui.input(|i| i.modifiers);
    if let Some(row) = resp
        .clicked
        .or(resp.secondary.filter(|r| !lv.selected.contains(r)))
    {
        table::click_select(&mut lv.selected, &mut lv.anchor, row, modifiers);
    }
    match context_menu(resp.area.as_ref(), resp.secondary.is_some(), "tracks", true) {
        Some(MenuChoice::Play) if !lv.selected.is_empty() => {
            actions.push(play_selection(view, &lv.selected));
        }
        Some(MenuChoice::Enqueue) => {
            let tracks: Vec<TrackId> = lv.selected.iter().map(|&r| view.tracks[r]).collect();
            actions.push(LibraryAction::Enqueue(tracks));
        }
        Some(MenuChoice::ShowInFolder) => {
            if let Some(&row) = lv.selected.first() {
                show_folder_of(library, view.tracks[row]);
            }
        }
        Some(MenuChoice::Info) => {
            if let Some(t) = lv.selected_track() {
                actions.push(LibraryAction::Info(t));
            }
        }
        _ => {}
    }
    if keys_free && lv.focus == Pane::Tracks {
        let from = lv.anchor;
        if let Some(row) = table::key_move(&ui, from, rows, page(rect)) {
            let m = if modifiers.shift {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            if m.shift {
                // Shift+arrows grow the range from where it started.
                let start = *lv.selected.first().unwrap_or(&row);
                let (lo, hi) = if start <= row {
                    (start, row)
                } else {
                    (row, start)
                };
                lv.selected = (lo..=hi).collect();
                lv.anchor = Some(row);
            } else {
                table::click_select(&mut lv.selected, &mut lv.anchor, row, m);
            }
            lv.scroll[2] = Some(row);
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
            lv.selected = (0..rows).collect();
        }
        if ui.input(|i| i.key_pressed(Key::Enter)) && !lv.selected.is_empty() {
            actions.push(play_selection(view, &lv.selected));
        }
    }
    if let Some(row) = resp.double_clicked {
        lv.selected = std::iter::once(row).collect();
        lv.anchor = Some(row);
        // The whole table goes to the playlist, from this
        // track on.
        actions.push(LibraryAction::Play {
            tracks: view.tracks.clone(),
            start: row,
        });
    }
    if resp.sorted {
        store_sort(settings, "tracks", sort);
        *changed = true;
    }
    if resp.resized {
        settings.columns.insert("tracks".into(), w);
        *changed = true;
    }
}

/// What was chosen in a pane's context menu.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuChoice {
    Play,
    Enqueue,
    ShowInFolder,
    Info,
}

/// A pane's context menu, opened where the right button was let go on a
/// row (`open`) or on the pane's free space (`area`); any click elsewhere
/// closes it. The right button counts as clicked even if the mouse moved
/// while it was held (`App::raw_input_hook`). `info`: with the file info
/// item (a track's menu).
fn context_menu(
    area: Option<&egui::Response>,
    open: bool,
    id: &str,
    info: bool,
) -> Option<MenuChoice> {
    let area = area?;
    let open = (open || area.secondary_clicked()).then_some(egui::SetOpenCommand::Bool(true));
    let mut choice = None;
    egui::Popup::context_menu(area)
        .id(egui::Id::new(("menu", id)))
        .open_memory(open)
        .show(|ui| {
            ui.set_min_width(200.0);
            for (item, label) in [
                (MenuChoice::Play, tr!("Play", "Воспроизвести")),
                (
                    MenuChoice::Enqueue,
                    tr!("Add to the playlist", "Добавить в плейлист"),
                ),
                (
                    MenuChoice::ShowInFolder,
                    tr!("Show in folder", "Показать в папке"),
                ),
            ] {
                if item == MenuChoice::ShowInFolder {
                    ui.separator();
                }
                if ui.button(label).clicked() {
                    choice = Some(item);
                    ui.close();
                }
            }
            if info
                && ui
                    .button(tr!("File info… (I)", "Сведения о файле… (I)"))
                    .clicked()
            {
                choice = Some(MenuChoice::Info);
                ui.close();
            }
        });
    choice
}

/// Enter on the tracks: one selected plays the table from it, several
/// play themselves.
fn play_selection(view: &View, selected: &BTreeSet<usize>) -> LibraryAction {
    if selected.len() == 1 {
        LibraryAction::Play {
            tracks: view.tracks.clone(),
            start: *selected.first().unwrap_or(&0),
        }
    } else {
        LibraryAction::Play {
            tracks: selected.iter().map(|&r| view.tracks[r]).collect(),
            start: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Track;

    #[test]
    fn typed_letters_go_to_the_first_name_and_then_on() {
        let names: Vec<String> = [
            "all",
            "arion",
            "auri",
            "battle beast",
            "beast in black",
            "billy idol",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(type_ahead(&names, "b"), Some(3));
        assert_eq!(type_ahead(&names, "be"), Some(4));
        assert_eq!(type_ahead(&names, "bi"), Some(5));
        assert_eq!(type_ahead(&names, "bb"), Some(4));
        assert_eq!(type_ahead(&names, "bbb"), Some(5));
        assert_eq!(type_ahead(&names, "bbbb"), Some(3));
        assert_eq!(type_ahead(&names, "c"), None);
        assert_eq!(type_ahead(&names, "bx"), None);
        assert_eq!(type_ahead(&names, ""), None);
    }

    fn lib() -> Library {
        let t = |path: &str, artist: &str, album: &str, year: u16, n: u16| Track {
            path: path.into(),
            title: format!("{album} {n}"),
            artist: artist.into(),
            album: album.into(),
            year,
            track: n,
            ..Track::default()
        };
        Library::new(vec![
            t("a/1.mp3", "Ария", "Химера", 2001, 1),
            t("a/2.mp3", "Ария", "Химера", 2001, 2),
            t("b/1.mp3", "Ария", "Герой асфальта", 1987, 1),
            t("c/1.mp3", "ABBA", "Gold", 1992, 1),
        ])
    }

    fn key(query: &str, artist: Option<&str>, album: Option<&str>) -> ViewKey {
        ViewKey {
            library: 0,
            query: query.into(),
            artist: artist.map(Into::into),
            album: album.map(Into::into),
            sorts: [None; 3],
        }
    }

    #[test]
    fn all_then_artist_then_album() {
        let lib = lib();
        let v = build(&lib, &key("", None, None));
        assert_eq!(v.artists.len(), 2);
        assert_eq!(v.albums.len(), 3);
        assert_eq!(v.tracks.len(), 4);
        let v = build(&lib, &key("", Some("ария"), None));
        assert_eq!(v.artist_row, Some(1));
        // By year: Герой асфальта (1987) before Химера (2001).
        assert_eq!(lib.albums[v.albums[0] as usize].title, "Герой асфальта");
        assert_eq!(v.tracks.len(), 3);
        let album = lib.albums[v.albums[1] as usize].key.clone();
        let v = build(&lib, &key("", Some("ария"), Some(&album)));
        assert_eq!(v.tracks.len(), 2);
    }

    #[test]
    fn search_narrows_every_pane() {
        let lib = lib();
        // Every word somewhere in the track: the artist and the album.
        let v = build(&lib, &key("ария химера", None, None));
        assert_eq!(v.artists.len(), 1);
        assert_eq!(v.artists[0].tracks, 2);
        assert_eq!(v.albums.len(), 1);
        assert_eq!(v.tracks.len(), 2);
        let v = build(&lib, &key("1987", None, None));
        assert_eq!(v.tracks.len(), 1);
        // A choice the search leaves out is no choice.
        let v = build(&lib, &key("gold", Some("ария"), None));
        assert_eq!(v.artist_row, None);
        assert_eq!(v.tracks.len(), 1);
    }

    #[test]
    fn sorts_by_a_column_either_way() {
        let lib = lib();
        let mut k = key("", None, None);
        k.sorts[1] = Some(Sort {
            column: 2,
            descending: true,
        });
        let v = build(&lib, &k);
        let years: Vec<u16> = v
            .albums
            .iter()
            .map(|&a| lib.albums[a as usize].year)
            .collect();
        assert_eq!(years, [2001, 1992, 1987]);
    }
}
