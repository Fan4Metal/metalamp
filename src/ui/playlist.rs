//! The playlist on the right, as Winamp's: a search field over the list,
//! "N. Artist – Title" and the length of each, the entry playing in the
//! accent, the total at the bottom. Rows are selected as in the tables
//! (Ctrl, Shift), dragged to another place, removed with Delete; a double
//! click or Enter plays one; the right button opens a menu. Under the
//! list, the cover of the track playing (a button in the bar hides it; a
//! click on it opens the file info).

use std::collections::BTreeSet;

use egui::{Align2, FontId, Key, Modifiers, Rect, Sense, TextureHandle, Ui, pos2, vec2};

use crate::library::Library;
use crate::player::Player;
use crate::ui::icons::{self, Icon};
use crate::ui::table::{self, ROW_HEIGHT};
use crate::{format, palette};

const HEADER: f32 = 36.0;
const FOOTER: f32 = 24.0;
/// The cover under the list: its largest side, its margins, and the
/// least the list keeps above it (else the cover is left out).
const COVER_MAX: f32 = 360.0;
const COVER_PAD: f32 = 10.0;
const COVER_MIN_LIST: f32 = 4.0 * ROW_HEIGHT;

#[derive(Default)]
pub struct PlaylistView {
    pub query: String,
    /// Selected entries, as indices into the playlist.
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    /// The playlist takes the keys (it was clicked last).
    pub focused: bool,
    /// Rows being dragged: where the drag started.
    dragging: bool,
    scroll_to: Option<usize>,
    /// The playlist's revision the selection belongs to.
    revision: u64,
}

impl PlaylistView {
    /// The entry selected last, if any.
    pub fn selected_entry(&self) -> Option<usize> {
        self.anchor
            .filter(|a| self.selected.contains(a))
            .or_else(|| self.selected.first().copied())
    }
}

/// What the playlist asks the app for.
pub enum PlaylistAction {
    Play(usize),
    ShowInFolder(usize),
    /// Open the file info window for an entry.
    Info(usize),
    /// The cover clicked: the file info window for the track playing.
    InfoPlaying,
}

/// The line of an entry: "Artist – Title" from the library, else the
/// file's name; and its length, if known.
fn describe(library: &Library, path: &std::path::Path) -> (String, Option<u32>) {
    match library.by_path.get(path) {
        Some(&id) => {
            let t = &library.tracks[id as usize];
            let text = if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} – {}", t.artist, t.title)
            };
            (text, Some(t.duration_ms))
        }
        None => (
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            None,
        ),
    }
}

pub fn show(
    ui: &mut Ui,
    pv: &mut PlaylistView,
    player: &mut Player,
    library: &Library,
    keys: bool,
    cover: Option<&TextureHandle>,
    show_cover: &mut bool,
) -> Vec<PlaylistAction> {
    let mut actions = Vec::new();
    if pv.revision != player.queue.revision {
        // The list changed elsewhere: indices no longer mean the same rows.
        pv.selected.clear();
        pv.anchor = None;
        pv.revision = player.queue.revision;
    }
    let full = ui.available_rect_before_wrap();
    let painter = ui.painter().clone();
    painter.rect_filled(full, 0.0, palette::panel_bg());
    painter.line_segment(
        [full.left_top(), full.left_bottom()],
        egui::Stroke::new(1.0, palette::line()),
    );

    // The search field and Clear.
    let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER));
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header.shrink2(vec2(8.0, 4.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let clear = crate::ui::icon_button(
        &mut bar,
        Icon::Delete,
        false,
        tr!("Clear the playlist", "Очистить плейлист"),
    );
    if clear.clicked() {
        player.stop();
        player.queue.clear();
    }
    if crate::ui::icon_button(
        &mut bar,
        Icon::Image,
        *show_cover,
        tr!("The cover of the track playing", "Обложка играющего трека"),
    )
    .clicked()
    {
        *show_cover = !*show_cover;
    }
    let edit = egui::TextEdit::singleline(&mut pv.query)
        .hint_text(tr!("Search in the playlist", "Поиск в плейлисте"))
        .desired_width(f32::INFINITY);
    let resp = bar.add(edit);
    if resp.has_focus() && bar.input(|i| i.key_pressed(Key::Escape)) {
        pv.query.clear();
        resp.surrender_focus();
    }

    // The rows the search leaves, as indices into the playlist.
    let words: Vec<String> = pv.query.split_whitespace().map(str::to_lowercase).collect();
    let entries = player.queue.entries();
    let count = entries.len();
    let lines: Vec<(String, Option<u32>)> =
        entries.iter().map(|e| describe(library, &e.path)).collect();
    let shown: Vec<usize> = (0..entries.len())
        .filter(|&i| {
            words.is_empty() || {
                let text = lines[i].0.to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            }
        })
        .collect();
    let current = player.queue.current();
    let playing = player
        .playing
        .and_then(|id| player.queue.index_of(id))
        .or(current);

    // The cover under the list, when it is shown and the list keeps
    // enough rows above it.
    let side = (full.width() - 2.0 * COVER_PAD).min(COVER_MAX);
    let cover_h = side + COVER_PAD;
    let list_h = full.height() - HEADER - FOOTER;
    let cover_rect =
        (*show_cover && side > 40.0 && list_h - cover_h >= COVER_MIN_LIST).then(|| {
            Rect::from_center_size(
                pos2(full.center().x, full.bottom() - FOOTER - cover_h / 2.0),
                vec2(side, side),
            )
        });
    let body = Rect::from_min_max(
        pos2(full.left(), header.bottom()),
        pos2(
            full.right(),
            full.bottom() - FOOTER - cover_rect.map_or(0.0, |_| cover_h + COVER_PAD),
        ),
    );
    painter.rect_filled(body, 0.0, palette::list_bg());
    if let Some(r) = cover_rect {
        match cover {
            Some(t) => {
                let [w, h] = t.size();
                let scale = side / w.max(h) as f32;
                let fit =
                    Rect::from_center_size(r.center(), vec2(w as f32 * scale, h as f32 * scale));
                egui::Image::new(egui::load::SizedTexture::new(t.id(), fit.size()))
                    .corner_radius(6.0)
                    .paint_at(ui, fit);
            }
            None => {
                painter.rect_filled(r, 6.0, palette::card_bg());
                icons::paint(
                    &painter,
                    Icon::File,
                    r.center(),
                    side * 0.25,
                    palette::line(),
                    palette::card_bg(),
                );
            }
        }
        let resp = ui.interact(r, egui::Id::new("playlist_cover"), Sense::click());
        if cover.is_some() {
            let resp = resp
                .clone()
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tr!("File info (I)", "Сведения о файле (I)"));
            if resp.clicked() {
                actions.push(PlaylistAction::InfoPlaying);
            }
        }
        egui::Popup::context_menu(&resp)
            .id(egui::Id::new("playlist_cover_menu"))
            .show(|ui| {
                if ui.button(tr!("Hide the cover", "Скрыть обложку")).clicked() {
                    *show_cover = false;
                    ui.close();
                }
            });
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .id_salt("playlist_rows"),
    );
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("playlist_scroll")
        .auto_shrink([false, false]);
    if let Some(i) = pv.scroll_to.take()
        && let Some(row) = shown.iter().position(|&s| s == i)
    {
        let offset = ui
            .ctx()
            .data(|d| d.get_temp::<f32>(egui::Id::new("playlist_offset")))
            .unwrap_or(0.0);
        let top = row as f32 * ROW_HEIGHT;
        if top < offset {
            scroll = scroll.vertical_scroll_offset(top);
        } else if top + ROW_HEIGHT > offset + body.height() {
            scroll = scroll.vertical_scroll_offset(top + ROW_HEIGHT - body.height());
        }
    }
    let modifiers = ui.input(|i| i.modifiers);
    let mut clicked_row = None;
    let mut double_row = None;
    let mut secondary_row = None;
    let mut drop_slot = None;
    let number_w = format!("{}.", count.max(1)).len() as f32 * 7.5 + 8.0;
    let out = scroll.show_viewport(&mut child, |ui, viewport| {
        let height = (shown.len() as f32 * ROW_HEIGHT).max(viewport.height());
        // The viewport's width: less the scroll bar's.
        let width = viewport.width();
        let (area, area_resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        let first = (viewport.top() / ROW_HEIGHT).floor().max(0.0) as usize;
        let last = ((viewport.bottom() / ROW_HEIGHT).ceil() as usize).min(shown.len());
        let painter = ui.painter();
        let font = FontId::proportional(13.0);
        for (row, &i) in shown.iter().enumerate().take(last).skip(first) {
            let r = Rect::from_min_size(
                pos2(area.left(), area.top() + row as f32 * ROW_HEIGHT),
                vec2(width, ROW_HEIGHT),
            );
            let resp = ui.interact(
                r,
                egui::Id::new(("playlist_row", row)),
                Sense::click_and_drag(),
            );
            let selected = pv.selected.contains(&i);
            if selected {
                painter.rect_filled(r, 0.0, palette::selected_bg());
            } else if resp.hovered() {
                painter.rect_filled(r, 0.0, palette::hover());
            }
            let color = if Some(i) == playing {
                palette::playing()
            } else if selected {
                palette::selected_text()
            } else {
                palette::list_text()
            };
            let (text, length) = &lines[i];
            let length = length
                .map(|ms| format::time(u64::from(ms)))
                .unwrap_or_default();
            let num =
                painter.layout_no_wrap(format!("{}.", i + 1), font.clone(), palette::text_weak());
            painter.galley(
                pos2(
                    r.left() + 6.0 + number_w - num.size().x - 6.0,
                    r.center().y - num.size().y / 2.0,
                ),
                num,
                palette::text_weak(),
            );
            let len = painter.layout_no_wrap(length, font.clone(), color);
            let len_x = r.right() - 8.0 - len.size().x;
            let text_left = r.left() + 6.0 + number_w;
            let clip = Rect::from_min_max(pos2(text_left, r.top()), pos2(len_x - 8.0, r.bottom()));
            let g = painter.layout_no_wrap(text.clone(), font.clone(), color);
            painter
                .with_clip_rect(clip.intersect(ui.clip_rect()))
                .galley(pos2(text_left, r.center().y - g.size().y / 2.0), g, color);
            painter.galley(pos2(len_x, r.center().y - len.size().y / 2.0), len, color);

            if crate::ui::double_clicked(&resp) {
                double_row = Some(i);
            } else if resp.clicked() {
                clicked_row = Some(i);
            }
            if resp.secondary_clicked() {
                secondary_row = Some(i);
            }
            if resp.drag_started() && words.is_empty() {
                if !pv.selected.contains(&i) {
                    table::click_select(&mut pv.selected, &mut pv.anchor, i, Modifiers::NONE);
                }
                pv.dragging = true;
            }
        }
        // While dragging: the slot under the pointer, shown as a line.
        if pv.dragging
            && let Some(p) = ui.ctx().pointer_interact_pos()
        {
            let slot =
                (((p.y - area.top()) / ROW_HEIGHT).round().max(0.0) as usize).min(shown.len());
            let y = area.top() + slot as f32 * ROW_HEIGHT;
            painter.line_segment(
                [pos2(area.left(), y), pos2(area.right(), y)],
                egui::Stroke::new(2.0, palette::accent()),
            );
            drop_slot = Some(slot);
        }
        area_resp
    });
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("playlist_offset"), out.state.offset.y));
    if pv.dragging && ui.input(|i| i.pointer.any_released()) {
        pv.dragging = false;
        if let Some(slot) = drop_slot {
            let indices: Vec<usize> = pv.selected.iter().copied().collect();
            let n = indices.len();
            let at = player.queue.move_to(&indices, slot);
            player.refresh_preload();
            pv.revision = player.queue.revision;
            pv.selected = (at..at + n).collect();
            pv.anchor = Some(at);
        }
    }

    if let Some(i) = clicked_row.or(secondary_row.filter(|i| !pv.selected.contains(i))) {
        table::click_select(&mut pv.selected, &mut pv.anchor, i, modifiers);
    }
    if let Some(i) = double_row {
        actions.push(PlaylistAction::Play(i));
    }
    let open_menu = secondary_row.is_some() || out.inner.secondary_clicked();
    egui::Popup::context_menu(&out.inner)
        .id(egui::Id::new("playlist_menu"))
        .open_memory(open_menu.then_some(egui::SetOpenCommand::Bool(true)))
        .show(|ui| {
            ui.set_min_width(200.0);
            let first = pv.selected.first().copied();
            if ui
                .add_enabled(
                    first.is_some(),
                    egui::Button::new(tr!("Play", "Воспроизвести")),
                )
                .clicked()
                && let Some(i) = first
            {
                actions.push(PlaylistAction::Play(i));
                ui.close();
            }
            if ui
                .add_enabled(
                    !pv.selected.is_empty(),
                    egui::Button::new(tr!("Remove (Delete)", "Убрать (Delete)")),
                )
                .clicked()
            {
                remove_selected(pv, player);
                ui.close();
            }
            if ui
                .add_enabled(
                    first.is_some(),
                    egui::Button::new(tr!("Show in folder", "Показать в папке")),
                )
                .clicked()
                && let Some(i) = first
            {
                actions.push(PlaylistAction::ShowInFolder(i));
                ui.close();
            }
            if ui
                .add_enabled(
                    first.is_some(),
                    egui::Button::new(tr!("File info… (I)", "Сведения о файле… (I)")),
                )
                .clicked()
                && let Some(i) = pv.selected_entry()
            {
                actions.push(PlaylistAction::Info(i));
                ui.close();
            }
            ui.separator();
            if ui
                .button(tr!("Clear the playlist", "Очистить плейлист"))
                .clicked()
            {
                player.stop();
                player.queue.clear();
                ui.close();
            }
        });

    // Keys, while the playlist has them.
    let keys_free = keys && !ui.ctx().memory(|m| m.focused().is_some());
    if pv.focused && keys_free {
        let rows = shown.len();
        let at = pv.anchor.and_then(|a| shown.iter().position(|&s| s == a));
        if let Some(row) = table::key_move(ui, at, rows, (body.height() / ROW_HEIGHT) as usize) {
            let i = shown[row];
            table::click_select(&mut pv.selected, &mut pv.anchor, i, Modifiers::NONE);
            pv.scroll_to = Some(i);
        }
        if ui.input(|i| i.key_pressed(Key::Enter))
            && let Some(&i) = pv.selected.first()
        {
            actions.push(PlaylistAction::Play(i));
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) {
            remove_selected(pv, player);
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
            pv.selected = shown.iter().copied().collect();
        }
    }

    // The total, in the last FOOTER points (the cover may sit between
    // the list and it).
    let footer = Rect::from_min_max(pos2(full.left(), full.bottom() - FOOTER), full.max);
    let total: u64 = lines.iter().filter_map(|l| l.1).map(u64::from).sum();
    let text = format!(
        "{} [{}]",
        crate::i18n::count(
            count as u64,
            ["track", "tracks"],
            ["трек", "трека", "треков"]
        ),
        format::long_duration(total)
    );
    painter.text(
        pos2(footer.left() + 10.0, footer.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.0),
        palette::text_weak(),
    );
    if let Some(i) = playing {
        // Where the entry playing is: a click on the footer shows it.
        let r = Rect::from_center_size(
            pos2(footer.right() - 18.0, footer.center().y),
            vec2(24.0, 20.0),
        );
        let resp = ui.interact(r, egui::Id::new("playlist_locate"), Sense::click());
        if resp.hovered() {
            painter.rect_filled(r, 3.0, palette::hover());
        }
        icons::paint(
            &painter,
            Icon::Play,
            r.center(),
            10.0,
            palette::accent(),
            palette::panel_bg(),
        );
        if resp
            .on_hover_text(tr!("Show the track playing", "Показать играющий трек"))
            .clicked()
        {
            pv.query.clear();
            pv.scroll_to = Some(i);
        }
    }
    ui.advance_cursor_after_rect(full);
    actions
}

fn remove_selected(pv: &mut PlaylistView, player: &mut Player) {
    if pv.selected.is_empty() {
        return;
    }
    let indices: Vec<usize> = pv.selected.iter().copied().collect();
    let playing_removed = player
        .playing
        .and_then(|id| player.queue.index_of(id))
        .is_some_and(|i| pv.selected.contains(&i));
    let first = indices[0];
    player.queue.remove(&indices);
    if playing_removed {
        // The entry heard was removed: what was after it plays on.
        player.stop();
        if first < player.queue.len() {
            player.play_index(first);
        }
    } else {
        player.refresh_preload();
    }
    pv.revision = player.queue.revision;
    pv.selected.clear();
    if first < player.queue.len() {
        pv.selected.insert(first);
        pv.anchor = Some(first);
    } else {
        pv.anchor = None;
    }
}
