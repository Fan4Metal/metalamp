//! A table of the media library: a header whose columns sort
//! when clicked (again: the other way) and change width when their right
//! edge is dragged (a double click there fits the column to its widest
//! text, as Windows' lists do), rows (a text too long for its cell cut
//! with "…", whole in a tooltip) of one height painted only where they are seen
//! (`ScrollArea::show_viewport`: thousands of rows cost what a screenful
//! does), stripes, the selection, the row playing. Columns wider together
//! than the table scroll sideways, the header with them. A title dragged
//! moves its column; the right button on the header lists the columns to
//! show or hide (`Arrangement`). The caller keeps the rows and the
//! selection; the table says what was clicked.

use egui::{FontId, Rect, Sense, Ui, pos2, vec2};

use crate::palette;

pub const ROW_HEIGHT: f32 = 20.0;
pub const HEADER_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 13.0;
const PAD: f32 = 6.0;
pub const MIN_WIDTH: f32 = 30.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

pub struct Column {
    pub title: String,
    pub align: Align,
}

impl Column {
    pub fn left(title: impl Into<String>) -> Column {
        Column {
            title: title.into(),
            align: Align::Left,
        }
    }

    pub fn right(title: impl Into<String>) -> Column {
        Column {
            title: title.into(),
            align: Align::Right,
        }
    }
}

/// How the rows are sorted: a column and the direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sort {
    pub column: usize,
    pub descending: bool,
}

/// Which columns a table shows and in what order, by their places in its
/// list of columns (which the cells, the widths and the sort go by). A
/// column the order lacks (one added later) follows in its place.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Arrangement {
    pub order: Vec<usize>,
    pub hidden: Vec<usize>,
}

impl Arrangement {
    /// Every column of `n`, in the order shown.
    fn all(&self, n: usize) -> Vec<usize> {
        let mut out = Vec::with_capacity(n);
        for c in self.order.iter().copied().chain(0..n) {
            if c < n && !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }

    /// The columns shown, in their order; all of them rather than none.
    pub fn shown(&self, n: usize) -> Vec<usize> {
        let all = self.all(n);
        let shown: Vec<usize> = all
            .iter()
            .copied()
            .filter(|c| !self.hidden.contains(c))
            .collect();
        if shown.is_empty() { all } else { shown }
    }

    /// Moves `column` to `slot` among those shown (0: first, their number:
    /// last); whether it moved.
    pub fn move_to(&mut self, n: usize, column: usize, slot: usize) -> bool {
        let shown = self.shown(n);
        let Some(from) = shown.iter().position(|&c| c == column) else {
            return false;
        };
        if slot == from || slot == from + 1 {
            return false;
        }
        let mut all = self.all(n);
        all.retain(|&c| c != column);
        let at = match shown.get(slot) {
            Some(before) => all.iter().position(|c| c == before).unwrap_or(all.len()),
            None => shown
                .last()
                .and_then(|last| all.iter().position(|c| c == last))
                .map_or(all.len(), |p| p + 1),
        };
        all.insert(at, column);
        self.order = all;
        true
    }

    pub fn set_hidden(&mut self, column: usize, hidden: bool) {
        self.hidden.retain(|&c| c != column);
        if hidden {
            self.hidden.push(column);
        }
    }

    /// Every one of `n` columns shown in its own place.
    pub fn is_default(&self, n: usize) -> bool {
        self.hidden.iter().all(|&c| c >= n) && self.all(n).into_iter().eq(0..n)
    }
}

/// What happened in a header this frame.
#[derive(Default)]
pub struct HeaderResponse {
    /// The sort changed (a title clicked, or the sorted column hidden).
    pub sorted: bool,
    /// A column's width changed (its edge dragged).
    pub resized: bool,
    /// A column moved, was hidden or shown (`Arrangement`).
    pub arranged: bool,
}

/// What happened in the table this frame.
#[derive(Default)]
pub struct TableResponse {
    pub clicked: Option<usize>,
    pub double_clicked: Option<usize>,
    /// A row the right button was let go on (its context menu).
    pub secondary: Option<usize>,
    /// The sort changed (a header clicked).
    pub sorted: bool,
    /// A column's width changed (a header's edge dragged).
    pub resized: bool,
    /// A column moved, was hidden or shown.
    pub arranged: bool,
    /// The table was clicked anywhere: it takes the keyboard.
    pub focused: bool,
    /// The response of the rows' area (for a context menu on empty space).
    pub area: Option<egui::Response>,
    /// A drag started on this row (`Table::drag`).
    pub drag_started: Option<usize>,
}

pub struct Table<'a> {
    pub id: egui::Id,
    pub columns: &'a [Column],
    /// Each column's width; the last one shown takes the room left.
    pub widths: &'a mut Vec<f32>,
    pub sort: &'a mut Option<Sort>,
    pub arrangement: &'a mut Arrangement,
    pub rows: usize,
    /// Scroll so that this row is seen.
    pub scroll_to: Option<usize>,
    /// Rows can be dragged (to the playlist).
    pub drag: bool,
}

impl Table<'_> {
    /// `cell(row, column)` gives the text; `style(row)` whether the row is
    /// selected and whether it is playing.
    pub fn show(
        self,
        ui: &mut Ui,
        cell: &mut dyn FnMut(usize, usize) -> String,
        style: &dyn Fn(usize) -> (bool, bool),
    ) -> TableResponse {
        let mut out = TableResponse::default();
        let n = self.columns.len();
        self.widths.resize(n, 100.0);
        let shown = self.arrangement.shown(n);
        let shown_widths: Vec<f32> = shown.iter().map(|&c| self.widths[c]).collect();
        let full = ui.available_rect_before_wrap();
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, palette::list_bg());

        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
        // The columns at their own widths; when they are wider than the
        // table, it scrolls sideways.
        let needed: f32 = shown_widths.iter().sum();

        // The rows.
        let body = Rect::from_min_max(pos2(full.left(), header.bottom()), full.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let mut scroll = egui::ScrollArea::both()
            .id_salt(self.id.with("rows"))
            .auto_shrink([false, false]);
        if let Some(row) = self.scroll_to {
            // Show the row: only when it is out of sight.
            let offset = child
                .ctx()
                .data(|d| d.get_temp::<f32>(self.id.with("offset")))
                .unwrap_or(0.0);
            let top = row as f32 * ROW_HEIGHT;
            // Less the sideways scroll bar, when there is one.
            let bar = if needed > body.width() {
                child.spacing().scroll.allocated_width()
            } else {
                0.0
            };
            let seen = body.height() - bar - ROW_HEIGHT;
            if top < offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top > offset + seen {
                scroll = scroll.vertical_scroll_offset(top - seen);
            }
        }
        let rows = self.rows;
        let widths = shown_widths;
        let out_area = scroll.show_viewport(&mut child, |ui, viewport| {
            let height = rows as f32 * ROW_HEIGHT;
            let width = needed.max(viewport.width());
            let (area_rect, area) =
                ui.allocate_exact_size(vec2(width, height.max(viewport.height())), Sense::click());
            let edges = column_edges(area_rect.left(), width, &widths);
            let first = (viewport.top() / ROW_HEIGHT).floor().max(0.0) as usize;
            let last = ((viewport.bottom() / ROW_HEIGHT).ceil() as usize).min(rows);
            let painter = ui.painter();
            let font = FontId::proportional(FONT_SIZE);
            for row in first..last {
                let top = area_rect.top() + row as f32 * ROW_HEIGHT;
                let r = Rect::from_min_size(pos2(area_rect.left(), top), vec2(width, ROW_HEIGHT));
                let sense = if self.drag {
                    Sense::click_and_drag()
                } else {
                    Sense::click()
                };
                let resp = ui.interact(r, self.id.with(("row", row)), sense);
                if resp.drag_started() {
                    out.drag_started = Some(row);
                }
                let (selected, playing) = style(row);
                let mut tip = None;
                let fill = if selected {
                    Some(palette::selected_bg())
                } else if resp.hovered() {
                    Some(palette::hover())
                } else if row % 2 == 1 {
                    Some(palette::list_stripe())
                } else {
                    None
                };
                if let Some(fill) = fill {
                    painter.rect_filled(r, 0.0, fill);
                }
                let color = if playing {
                    palette::playing()
                } else if selected {
                    palette::selected_text()
                } else {
                    palette::list_text()
                };
                for (slot, &i) in shown.iter().enumerate() {
                    let col = &self.columns[i];
                    let (left, right) = edges[slot];
                    let clip = Rect::from_min_max(
                        pos2(left + PAD, r.top()),
                        pos2(right - PAD, r.bottom()),
                    );
                    if clip.width() <= 0.0 {
                        continue;
                    }
                    let text = cell(row, i);
                    // The whole text, for the tooltip of a cut cell under
                    // the pointer.
                    let here = resp.hovered()
                        && ui
                            .ctx()
                            .pointer_hover_pos()
                            .is_some_and(|p| (left..right).contains(&p.x));
                    let whole = here.then(|| text.clone());
                    let galley =
                        crate::ui::elided(painter, text, font.clone(), color, clip.width());
                    if galley.elided {
                        tip = tip.or(whole);
                    }
                    let x = match col.align {
                        Align::Right => clip.right() - galley.size().x,
                        Align::Left => clip.left(),
                    };
                    let pos = pos2(x, clip.center().y - galley.size().y / 2.0);
                    painter
                        .with_clip_rect(clip.intersect(viewport_rect(ui)))
                        .galley(pos, galley, color);
                }
                if let Some(tip) = tip {
                    resp.clone().on_hover_text(tip);
                }
                if crate::ui::double_clicked(&resp) {
                    out.double_clicked = Some(row);
                } else if resp.clicked() {
                    out.clicked = Some(row);
                }
                if resp.secondary_clicked() {
                    out.secondary = Some(row);
                }
                if resp.clicked() || resp.secondary_clicked() {
                    out.focused = true;
                }
            }
            if area.clicked() || area.secondary_clicked() {
                out.focused = true;
            }
            (area, width)
        });
        child
            .ctx()
            .data_mut(|d| d.insert_temp(self.id.with("offset"), out_area.state.offset.y));
        let (area, width) = out_area.inner;
        out.area = Some(area);

        // The header after the rows: it moves with this frame's sideways
        // scroll.
        let left = full.left() - out_area.state.offset.x;
        let painter = ui.painter().clone();
        let columns = self.columns;
        let head = header_row(
            ui,
            self.id,
            header,
            (left, width),
            columns,
            self.widths,
            self.sort,
            self.arrangement,
            &mut |i| fit_width(&painter, &columns[i].title, rows, &mut |row| cell(row, i)),
        );
        out.sorted = head.sorted;
        out.resized = head.resized;
        out.arranged = head.arranged;
        ui.advance_cursor_after_rect(full);
        out
    }
}

/// The header of a table in `header`: the titles of the columns
/// `arrangement` shows, a click sorts by one (again: the other way; a
/// third time: the library's order), a title dragged moves its column (a
/// line shows where), a column's right edge dragged changes its width, a
/// double click on it gives the column the width `fit` says (`fit_width`:
/// its widest text), the right button lists the columns to show or hide
/// and puts them back as they were. The columns start at `span.0` and take
/// `span.1` (wider than `header` when the table scrolls sideways; what is
/// outside is cut). Also over the album grid, which sorts by the same
/// columns.
#[allow(clippy::too_many_arguments)]
pub fn header_row(
    ui: &mut Ui,
    id: egui::Id,
    header: Rect,
    span: (f32, f32),
    columns: &[Column],
    widths: &mut Vec<f32>,
    sort: &mut Option<Sort>,
    arrangement: &mut Arrangement,
    fit: &mut dyn FnMut(usize) -> f32,
) -> HeaderResponse {
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(header));
    ui.set_clip_rect(header.intersect(ui.clip_rect()));
    let ui = &mut ui;
    let painter = ui.painter().clone();
    painter.rect_filled(header, 0.0, palette::header_bg());
    let n = columns.len();
    widths.resize(n, 100.0);
    let mut out = HeaderResponse::default();
    let shown = arrangement.shown(n);
    let shown_widths: Vec<f32> = shown.iter().map(|&c| widths[c]).collect();
    let edges = column_edges(span.0, span.1, &shown_widths);
    // Under the titles: the right button anywhere on the header lists the
    // columns.
    let ground = ui.interact(header, id.with("head_ground"), Sense::click());
    let mut menu = ground.secondary_clicked();
    // A title being dragged (the pointer's x), and one let go.
    let mut dragging = None;
    let mut dropped = None;
    for (slot, &i) in shown.iter().enumerate() {
        let col = &columns[i];
        let (left, right) = edges[slot];
        let r = Rect::from_min_max(pos2(left, header.top()), pos2(right, header.bottom()));
        let resp = ui.interact(r, id.with(("head", i)), Sense::click_and_drag());
        menu |= resp.secondary_clicked();
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            dragging = resp.interact_pointer_pos().map(|p| p.x);
        }
        if resp.drag_stopped() {
            dropped = resp.interact_pointer_pos().map(|p| (i, p.x));
        }
        if resp.hovered() || resp.dragged() {
            painter.rect_filled(r, 0.0, palette::hover());
        }
        if resp.clicked() {
            // Up, down, then the library's order again.
            *sort = match *sort {
                Some(s) if s.column == i && s.descending => None,
                Some(s) if s.column == i => Some(Sort {
                    column: i,
                    descending: true,
                }),
                _ => Some(Sort {
                    column: i,
                    descending: false,
                }),
            };
            out.sorted = true;
        }
        let clip = r.shrink2(vec2(PAD, 0.0));
        let mut text_right = clip.right();
        if let Some(s) = *sort
            && s.column == i
        {
            // A small triangle: up ascending, down descending.
            let c = pos2(clip.right() - 4.0, r.center().y);
            let (a, b, tip) = if s.descending {
                (
                    c + vec2(-4.0, -2.0),
                    c + vec2(4.0, -2.0),
                    c + vec2(0.0, 3.0),
                )
            } else {
                (c + vec2(-4.0, 2.0), c + vec2(4.0, 2.0), c + vec2(0.0, -3.0))
            };
            painter.add(egui::Shape::convex_polygon(
                vec![a, b, tip],
                palette::header_text(),
                egui::Stroke::NONE,
            ));
            text_right -= 12.0;
        }
        let clip = Rect::from_min_max(clip.min, pos2(text_right.max(clip.left()), clip.bottom()))
            .intersect(header);
        if cell_text(
            &painter,
            clip,
            &col.title,
            col.align,
            palette::header_text(),
        ) {
            resp.on_hover_text(&col.title);
        }
        painter.line_segment(
            [
                pos2(right - 0.5, r.top() + 4.0),
                pos2(right - 0.5, r.bottom() - 4.0),
            ],
            egui::Stroke::new(1.0, palette::line()),
        );
    }
    // Where a title dragged would go: a line between two columns.
    if let Some(x) = dragging {
        let slot = slot_at(&edges, x);
        let line_x = if slot == 0 {
            edges[0].0
        } else {
            edges[slot - 1].1
        };
        painter.line_segment(
            [pos2(line_x, header.top()), pos2(line_x, header.bottom())],
            egui::Stroke::new(2.0, palette::accent()),
        );
    }
    if let Some((column, x)) = dropped
        && arrangement.move_to(n, column, slot_at(&edges, x))
    {
        out.arranged = true;
    }
    // The right edges resize the columns (not the last one's): after all
    // the titles, so that the half of an edge over the next title is the
    // edge's and a double click there does not sort by that column.
    for (slot, &(_, right)) in edges.iter().enumerate().take(shown.len().saturating_sub(1)) {
        let i = shown[slot];
        let grip =
            Rect::from_center_size(pos2(right, header.center().y), vec2(8.0, header.height()));
        let g = ui.interact(grip, id.with(("grip", i)), Sense::click_and_drag());
        if g.hovered() || g.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if g.dragged() {
            let w = &mut widths[i];
            *w = (*w + g.drag_delta().x).max(MIN_WIDTH);
            out.resized = true;
        }
        if crate::ui::double_clicked(&g) {
            widths[i] = fit(i).max(MIN_WIDTH).round();
            out.resized = true;
        }
    }
    columns_menu(&ground, menu, id, columns, &shown, sort, arrangement, &mut out);
    out
}

/// The header's menu: a tick per column, shown or hidden (the last one
/// shown stays; hiding the sorted one gives up the sort), and the columns
/// put back as they were.
#[allow(clippy::too_many_arguments)]
fn columns_menu(
    ground: &egui::Response,
    open: bool,
    id: egui::Id,
    columns: &[Column],
    shown: &[usize],
    sort: &mut Option<Sort>,
    arrangement: &mut Arrangement,
    out: &mut HeaderResponse,
) {
    let n = columns.len();
    egui::Popup::context_menu(ground)
        .id(id.with("columns_menu"))
        .open_memory(open.then_some(egui::SetOpenCommand::Bool(true)))
        .show(|ui| {
            ui.set_min_width(180.0);
            for (i, col) in columns.iter().enumerate() {
                let mut on = shown.contains(&i);
                let last = on && shown.len() == 1;
                if ui
                    .add_enabled(!last, egui::Checkbox::new(&mut on, &col.title))
                    .changed()
                {
                    arrangement.set_hidden(i, !on);
                    if !on && sort.is_some_and(|s| s.column == i) {
                        *sort = None;
                        out.sorted = true;
                    }
                    out.arranged = true;
                }
            }
            ui.separator();
            if ui
                .add_enabled(
                    !arrangement.is_default(n),
                    egui::Button::new(tr!("Columns as they were", "Столбцы по умолчанию")),
                )
                .clicked()
            {
                *arrangement = Arrangement::default();
                out.arranged = true;
                ui.close();
            }
        });
}

/// The slot among the columns whose `edges` the pointer at `x` is
/// nearest: before the first column whose middle is past it.
fn slot_at(edges: &[(f32, f32)], x: f32) -> usize {
    edges
        .iter()
        .take_while(|(left, right)| (left + right) / 2.0 < x)
        .count()
}

/// The width that shows a column whole: its widest cell of `rows`
/// (`cell(row)`) and its title with room for the sort's triangle, with the
/// cells' padding. Every row is measured, not only those in sight, as
/// Windows does.
pub fn fit_width(
    painter: &egui::Painter,
    title: &str,
    rows: usize,
    cell: &mut dyn FnMut(usize) -> String,
) -> f32 {
    let width = |text: String, size: f32| {
        painter
            .layout_no_wrap(text, FontId::proportional(size), egui::Color32::WHITE)
            .size()
            .x
    };
    let widest = (0..rows)
        .map(|row| width(cell(row), FONT_SIZE))
        .fold(0.0, f32::max);
    let title = width(title.to_owned(), FONT_SIZE - 0.5) + 12.0;
    widest.max(title) + 2.0 * PAD + 2.0
}

/// The visible part of the scroll area's content.
fn viewport_rect(ui: &Ui) -> Rect {
    ui.clip_rect()
}

/// Left and right of each column: the given widths, the last column to
/// the table's right edge (`total` past `left`; the callers make it room
/// for every width), never narrower than `MIN_WIDTH`.
fn column_edges(left: f32, total: f32, widths: &[f32]) -> Vec<(f32, f32)> {
    let mut x = left;
    let n = widths.len();
    widths
        .iter()
        .enumerate()
        .map(|(i, &w)| {
            let right = if i + 1 == n {
                (left + total).max(x + w.min(MIN_WIDTH))
            } else {
                x + w
            };
            let edge = (x, right);
            x = right;
            edge
        })
        .collect()
}

/// A header's title in `clip`: right-aligned titles that do not fit are
/// cut at their end, as left-aligned ones, not at their start.
/// A title in `clip`, cut with "…" when longer; whether it was cut.
fn cell_text(
    painter: &egui::Painter,
    clip: Rect,
    text: &str,
    align: Align,
    color: egui::Color32,
) -> bool {
    let font = FontId::proportional(FONT_SIZE - 0.5);
    let galley = crate::ui::elided(painter, text.to_owned(), font, color, clip.width());
    let cut = galley.elided;
    let x = match align {
        Align::Right => clip.right() - galley.size().x,
        Align::Left => clip.left(),
    };
    let pos = pos2(x, clip.center().y - galley.size().y / 2.0);
    painter.with_clip_rect(clip).galley(pos, galley, color);
    cut
}

/// The new selection after a click on `row` with the modifiers held: Ctrl
/// toggles the row, Shift selects from the anchor, else the row alone.
/// `selected` and `anchor` are kept by the caller.
pub fn click_select(
    selected: &mut std::collections::BTreeSet<usize>,
    anchor: &mut Option<usize>,
    row: usize,
    modifiers: egui::Modifiers,
) {
    if modifiers.shift
        && let Some(a) = *anchor
    {
        if !modifiers.command {
            selected.clear();
        }
        let (lo, hi) = if a <= row { (a, row) } else { (row, a) };
        selected.extend(lo..=hi);
    } else if modifiers.command {
        if !selected.remove(&row) {
            selected.insert(row);
        }
        *anchor = Some(row);
    } else {
        selected.clear();
        selected.insert(row);
        *anchor = Some(row);
    }
}

/// The row the arrow, page and Home/End keys move to from `from`, among
/// `rows`, `page` rows a screen; `None` when no such key was pressed.
pub fn key_move(ui: &Ui, from: Option<usize>, rows: usize, page: usize) -> Option<usize> {
    if rows == 0 {
        return None;
    }
    let last = rows - 1;
    ui.input(|i| {
        let pressed = |k| i.key_pressed(k);
        let at = from.unwrap_or(0);
        if pressed(egui::Key::ArrowDown) {
            Some(if from.is_none() {
                0
            } else {
                (at + 1).min(last)
            })
        } else if pressed(egui::Key::ArrowUp) {
            Some(at.saturating_sub(1))
        } else if pressed(egui::Key::PageDown) {
            Some((at + page).min(last))
        } else if pressed(egui::Key::PageUp) {
            Some(at.saturating_sub(page))
        } else if pressed(egui::Key::Home) {
            Some(0)
        } else if pressed(egui::Key::End) {
            Some(last)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn selection_by_click_ctrl_and_shift() {
        let (mut sel, mut anchor) = (BTreeSet::new(), None);
        click_select(&mut sel, &mut anchor, 3, egui::Modifiers::NONE);
        assert_eq!(sel.iter().copied().collect::<Vec<_>>(), [3]);
        click_select(&mut sel, &mut anchor, 6, egui::Modifiers::SHIFT);
        assert_eq!(sel.iter().copied().collect::<Vec<_>>(), [3, 4, 5, 6]);
        click_select(&mut sel, &mut anchor, 4, egui::Modifiers::COMMAND);
        assert_eq!(sel.iter().copied().collect::<Vec<_>>(), [3, 5, 6]);
        click_select(&mut sel, &mut anchor, 1, egui::Modifiers::NONE);
        assert_eq!(sel.iter().copied().collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn columns_arranged() {
        let mut a = Arrangement::default();
        assert!(a.is_default(4));
        assert_eq!(a.shown(4), [0, 1, 2, 3]);
        // Moved after the next one, to the end, to the front.
        assert!(a.move_to(4, 0, 2));
        assert_eq!(a.shown(4), [1, 0, 2, 3]);
        assert!(a.move_to(4, 1, 4));
        assert_eq!(a.shown(4), [0, 2, 3, 1]);
        assert!(a.move_to(4, 3, 0));
        assert_eq!(a.shown(4), [3, 0, 2, 1]);
        // Dropped on itself or right after itself: no move.
        assert!(!a.move_to(4, 0, 1));
        assert!(!a.move_to(4, 0, 2));
        // Hidden ones are skipped by the slots and keep their place.
        a.set_hidden(0, true);
        assert_eq!(a.shown(4), [3, 2, 1]);
        assert!(a.move_to(4, 1, 1));
        assert_eq!(a.shown(4), [3, 1, 2]);
        a.set_hidden(0, false);
        assert_eq!(a.shown(4), [3, 0, 1, 2]);
        assert!(!a.is_default(4));
        // A column added later follows; stale and doubled places are left
        // out; all hidden shows all.
        let b = Arrangement {
            order: vec![2, 9, 2, 0],
            hidden: vec![0, 1, 2, 3],
        };
        assert_eq!(b.shown(4), [2, 0, 1, 3]);
        let c = Arrangement {
            order: vec![0, 1],
            hidden: vec![7],
        };
        assert!(c.is_default(3));
    }

    #[test]
    fn slot_by_the_middles() {
        let edges = [(0.0, 100.0), (100.0, 150.0), (150.0, 300.0)];
        assert_eq!(slot_at(&edges, 10.0), 0);
        assert_eq!(slot_at(&edges, 60.0), 1);
        assert_eq!(slot_at(&edges, 130.0), 2);
        assert_eq!(slot_at(&edges, 299.0), 3);
    }

    #[test]
    fn last_column_takes_the_rest() {
        let edges = column_edges(0.0, 500.0, &[100.0, 50.0, 80.0]);
        assert_eq!(edges, [(0.0, 100.0), (100.0, 150.0), (150.0, 500.0)]);
    }
}
