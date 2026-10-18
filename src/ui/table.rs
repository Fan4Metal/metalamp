//! A table as Winamp's media library has them: a header whose columns sort
//! when clicked (again: the other way) and change width when their right
//! edge is dragged, rows of one height painted only where they are seen
//! (`ScrollArea::show_viewport`: thousands of rows cost what a screenful
//! does), stripes, the selection, the row playing. Columns wider together
//! than the table scroll sideways, the header with them. The caller keeps
//! the rows and the selection; the table says what was clicked.

use egui::{FontId, Rect, Sense, Ui, pos2, vec2};

use crate::palette;

pub const ROW_HEIGHT: f32 = 20.0;
pub const HEADER_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 13.0;
const PAD: f32 = 6.0;
const MIN_WIDTH: f32 = 30.0;

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
    /// The table was clicked anywhere: it takes the keyboard.
    pub focused: bool,
    /// The response of the rows' area (for a context menu on empty space).
    pub area: Option<egui::Response>,
}

pub struct Table<'a> {
    pub id: egui::Id,
    pub columns: &'a [Column],
    /// Each column's width; the last one takes the room left.
    pub widths: &'a mut Vec<f32>,
    pub sort: &'a mut Option<Sort>,
    pub rows: usize,
    /// Scroll so that this row is seen.
    pub scroll_to: Option<usize>,
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
        let full = ui.available_rect_before_wrap();
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, palette::LIST_BG);

        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
        // The columns at their own widths; when they are wider than the
        // table, it scrolls sideways.
        let needed: f32 = self.widths.iter().sum();

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
        let widths = self.widths.clone();
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
                let resp = ui.interact(r, self.id.with(("row", row)), Sense::click());
                let (selected, playing) = style(row);
                let fill = if selected {
                    Some(palette::SELECTED_BG)
                } else if resp.hovered() {
                    Some(palette::HOVER)
                } else if row % 2 == 1 {
                    Some(palette::LIST_STRIPE)
                } else {
                    None
                };
                if let Some(fill) = fill {
                    painter.rect_filled(r, 0.0, fill);
                }
                let color = if playing {
                    palette::PLAYING
                } else if selected {
                    palette::TEXT_BRIGHT
                } else {
                    palette::TEXT
                };
                for (i, col) in self.columns.iter().enumerate() {
                    let (left, right) = edges[i];
                    let clip = Rect::from_min_max(
                        pos2(left + PAD, r.top()),
                        pos2(right - PAD, r.bottom()),
                    );
                    if clip.width() <= 0.0 {
                        continue;
                    }
                    let text = cell(row, i);
                    let galley = painter.layout_no_wrap(text, font.clone(), color);
                    // Right-aligned text that does not fit is cut at its end.
                    let x = match col.align {
                        Align::Right if galley.size().x <= clip.width() => {
                            clip.right() - galley.size().x
                        }
                        _ => clip.left(),
                    };
                    let pos = pos2(x, clip.center().y - galley.size().y / 2.0);
                    painter
                        .with_clip_rect(clip.intersect(viewport_rect(ui)))
                        .galley(pos, galley, color);
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
        let (sorted, resized) = header_row(
            ui,
            self.id,
            header,
            (left, width),
            self.columns,
            self.widths,
            self.sort,
        );
        out.sorted = sorted;
        out.resized = resized;
        ui.advance_cursor_after_rect(full);
        out
    }
}

/// The header of a table in `header`: the columns' titles, a click sorts
/// by one (again: the other way), a column's right edge dragged changes
/// its width. The columns start at `span.0` and take `span.1` (wider than
/// `header` when the table scrolls sideways; what is outside is cut).
/// Whether the sort and whether a width changed. Also over the album grid,
/// which sorts by the same columns.
pub fn header_row(
    ui: &mut Ui,
    id: egui::Id,
    header: Rect,
    span: (f32, f32),
    columns: &[Column],
    widths: &mut Vec<f32>,
    sort: &mut Option<Sort>,
) -> (bool, bool) {
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(header));
    ui.set_clip_rect(header.intersect(ui.clip_rect()));
    let ui = &mut ui;
    let painter = ui.painter().clone();
    painter.rect_filled(header, 0.0, palette::HEADER_BG);
    let n = columns.len();
    widths.resize(n, 100.0);
    let (mut sorted, mut resized) = (false, false);
    let edges = column_edges(span.0, span.1, widths);
    for (i, col) in columns.iter().enumerate() {
        let (left, right) = edges[i];
        let r = Rect::from_min_max(pos2(left, header.top()), pos2(right, header.bottom()));
        let resp = ui.interact(r, id.with(("head", i)), Sense::click());
        if resp.hovered() {
            painter.rect_filled(r, 0.0, palette::HOVER);
        }
        if resp.clicked() {
            *sort = Some(match *sort {
                Some(s) if s.column == i => Sort {
                    column: i,
                    descending: !s.descending,
                },
                _ => Sort {
                    column: i,
                    descending: false,
                },
            });
            sorted = true;
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
                palette::TEXT_WEAK,
                egui::Stroke::NONE,
            ));
            text_right -= 12.0;
        }
        let clip = Rect::from_min_max(clip.min, pos2(text_right.max(clip.left()), clip.bottom()))
            .intersect(header);
        cell_text(&painter, clip, &col.title, col.align, palette::TEXT_WEAK);
        painter.line_segment(
            [
                pos2(right - 0.5, r.top() + 4.0),
                pos2(right - 0.5, r.bottom() - 4.0),
            ],
            egui::Stroke::new(1.0, palette::LINE),
        );
        // The right edge resizes the column (not the last one).
        if i + 1 < n {
            let grip = Rect::from_center_size(pos2(right, r.center().y), vec2(8.0, r.height()));
            let g = ui.interact(grip, id.with(("grip", i)), Sense::drag());
            if g.hovered() || g.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if g.dragged() {
                let w = &mut widths[i];
                *w = (*w + g.drag_delta().x).max(MIN_WIDTH);
                resized = true;
            }
        }
    }
    (sorted, resized)
}

/// The visible part of the scroll area's content.
fn viewport_rect(ui: &Ui) -> Rect {
    ui.clip_rect()
}

/// Left and right of each column: the given widths, the last column to
/// the table's right edge (at least its own width).
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
fn cell_text(painter: &egui::Painter, clip: Rect, text: &str, align: Align, color: egui::Color32) {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        FontId::proportional(FONT_SIZE - 0.5),
        color,
    );
    let x = match align {
        Align::Right if galley.size().x <= clip.width() => clip.right() - galley.size().x,
        _ => clip.left(),
    };
    let pos = pos2(x, clip.center().y - galley.size().y / 2.0);
    painter.with_clip_rect(clip).galley(pos, galley, color);
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
    fn last_column_takes_the_rest() {
        let edges = column_edges(0.0, 500.0, &[100.0, 50.0, 80.0]);
        assert_eq!(edges, [(0.0, 100.0), (100.0, 150.0), (150.0, 500.0)]);
    }
}
