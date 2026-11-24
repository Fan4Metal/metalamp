//! The lyrics of the track heard in a rectangle (under the playlist, in
//! the cover's place): the lines centred; synced ones lit as they are
//! sung, the line sung kept at [`FOLLOW_AT`] of the height by a scroll
//! easing to it (the wheel scrolls by hand, and the following waits
//! [`HAND_S`]), a click on a line seeking to it; plain ones scrolled by the
//! wheel. While looking, a spinner; without lyrics, a word why. The right
//! button: the cover, copy, LRCLIB asked again, hide.

use egui::{Align, FontId, Rect, Sense, Ui, pos2, vec2};

use crate::lyrics::{Source, State, Text};
use crate::palette;

/// The room round the lines, the gap between them, their size.
const PAD: f32 = 10.0;
const GAP: f32 = 6.0;
const SIZE: f32 = 14.0;
/// Where the line sung is kept, as a share of the height from the top.
const FOLLOW_AT: f32 = 0.4;
/// How fast the scroll eases to the line sung: its time constant.
const EASE_S: f32 = 0.12;
/// After the wheel, how long the following waits.
const HAND_S: f64 = 4.0;
/// The fades at the top and the bottom.
const FADE: f32 = 18.0;

/// What the panel asks for.
pub enum Act {
    Seek(u64),
    ShowCover,
    Hide,
    SearchAgain,
}

/// The scroll of the lines, kept in egui's memory by the panel's id.
#[derive(Clone, Copy, Default)]
struct Scroll {
    offset: f32,
    /// The text it belongs to (its address), to start again at the top.
    text: usize,
    /// Until when the wheel's scroll stays (the following waits).
    hand_until: f64,
}

pub fn show(
    ui: &mut Ui,
    rect: Rect,
    id: egui::Id,
    state: &State,
    position_ms: u64,
    online: bool,
) -> Vec<Act> {
    let mut acts = Vec::new();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, palette::list_bg());
    let resp = ui.interact(rect, id, Sense::click());
    let inner = rect.shrink(PAD);
    match state {
        State::Idle => {
            note(
                ui,
                inner,
                rect.center().y,
                tr!("Nothing is playing.", "Ничего не играет."),
            );
        }
        State::Looking => {
            let side = 22.0;
            let spin = Rect::from_center_size(rect.center() - vec2(0.0, 12.0), vec2(side, side));
            egui::Spinner::new()
                .size(side)
                .color(palette::text_weak())
                .paint_at(ui, spin);
            note(
                ui,
                inner,
                rect.center().y + 12.0,
                tr!("Looking for the lyrics…", "Поиск текста…"),
            );
        }
        State::Instrumental => {
            note(
                ui,
                inner,
                rect.center().y,
                tr!("An instrumental", "Инструментальная композиция"),
            );
        }
        State::None(failed) => {
            let why = match failed {
                Some(_) => tr!(
                    "No lyrics: LRCLIB could not be asked.",
                    "Текста нет: LRCLIB не ответил."
                ),
                None if online => tr!("No lyrics found.", "Текст не найден."),
                None => tr!(
                    "No lyrics in the file. Looking on the internet is off in the settings.",
                    "В файле нет текста. Поиск в интернете выключен в настройках."
                ),
            };
            note(ui, inner, rect.center().y, why);
            if let Some(e) = failed {
                resp.clone().on_hover_text(e.as_str());
            }
        }
        State::Found(text) => {
            if let Some(ms) = lines(ui, &resp, inner, text, position_ms) {
                acts.push(Act::Seek(ms));
            }
            if resp.hovered() {
                let from = match text.source {
                    Source::File => tr!(".lrc file", "файл .lrc"),
                    Source::Tag => tr!("tag", "тег"),
                    Source::Lrclib => "LRCLIB",
                };
                painter.text(
                    rect.right_bottom() - vec2(6.0, 4.0),
                    egui::Align2::RIGHT_BOTTOM,
                    from,
                    FontId::proportional(11.0),
                    palette::text_weak(),
                );
            }
        }
    }
    egui::Popup::context_menu(&resp)
        .id(id.with("menu"))
        .show(|ui| {
            if ui.button(tr!("Show the cover", "Показать обложку")).clicked() {
                acts.push(Act::ShowCover);
                ui.close();
            }
            if let State::Found(text) = state
                && ui.button(tr!("Copy the lyrics", "Копировать текст")).clicked()
            {
                ui.ctx().copy_text(text.plain());
                ui.close();
            }
            let from_lrclib = match state {
                State::Found(t) => t.source == Source::Lrclib,
                State::None(_) | State::Instrumental => true,
                State::Idle | State::Looking => false,
            };
            if online
                && from_lrclib
                && ui
                    .button(tr!("Look on LRCLIB again", "Искать в LRCLIB заново"))
                    .clicked()
            {
                acts.push(Act::SearchAgain);
                ui.close();
            }
            ui.separator();
            if ui.button(tr!("Hide the lyrics", "Скрыть текст")).clicked() {
                acts.push(Act::Hide);
                ui.close();
            }
        });
    acts
}

/// A word in the middle, wrapped to the width.
fn note(ui: &Ui, inner: Rect, y: f32, text: &str) {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        FontId::proportional(13.0),
        palette::text_weak(),
        inner.width(),
    );
    job.halign = Align::Center;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let top = y - galley.size().y / 2.0;
    ui.painter_at(inner)
        .galley(pos2(inner.center().x, top), galley, palette::text_weak());
}

/// The lines, scrolled; the time of a synced line clicked.
fn lines(
    ui: &mut Ui,
    resp: &egui::Response,
    inner: Rect,
    text: &Text,
    position_ms: u64,
) -> Option<u64> {
    let id = resp.id.with("scroll");
    let now = ui.input(|i| i.time);
    let key = std::ptr::from_ref(text) as usize;
    let mut scroll = ui
        .data(|d| d.get_temp::<Scroll>(id))
        .filter(|s| s.text == key)
        .unwrap_or(Scroll {
            text: key,
            ..Default::default()
        });
    let current = text.current(position_ms);
    // Laid out once for their heights; the colours are given at painting.
    let galleys: Vec<_> = text
        .lines
        .iter()
        .map(|l| {
            // A synced blank line is a pause in the singing (the font has
            // no note sign).
            let words = if l.text.is_empty() && text.synced {
                "· · ·"
            } else if l.text.is_empty() {
                " "
            } else {
                l.text.as_str()
            };
            let mut job = egui::text::LayoutJob::simple(
                words.to_owned(),
                FontId::proportional(SIZE),
                egui::Color32::PLACEHOLDER,
                inner.width(),
            );
            job.halign = Align::Center;
            ui.fonts_mut(|f| f.layout_job(job))
        })
        .collect();
    let mut tops = Vec::with_capacity(galleys.len());
    let mut y = 0.0;
    for g in &galleys {
        tops.push(y);
        y += g.size().y + GAP;
    }
    let total = (y - GAP).max(0.0);
    let most = (total - inner.height()).max(0.0);

    if resp.hovered() {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            scroll.offset -= wheel;
            scroll.hand_until = now + HAND_S;
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        }
    }
    if text.synced && now >= scroll.hand_until {
        let target = current.map_or(0.0, |i| {
            tops[i] + galleys[i].size().y / 2.0 - inner.height() * FOLLOW_AT
        });
        let target = target.clamp(0.0, most);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let step = 1.0 - (-dt / EASE_S).exp();
        scroll.offset += (target - scroll.offset) * step;
        if (target - scroll.offset).abs() > 0.5 {
            ui.ctx().request_repaint();
        } else {
            scroll.offset = target;
        }
    }
    scroll.offset = scroll.offset.clamp(0.0, most);
    ui.data_mut(|d| d.insert_temp(id, scroll));

    // The line under the pointer, when synced: it seeks.
    let hovered = resp
        .hover_pos()
        .filter(|p| inner.contains(*p) && text.synced)
        .and_then(|p| {
            let y = p.y - inner.top() + scroll.offset;
            (0..galleys.len()).find(|&i| y >= tops[i] && y < tops[i] + galleys[i].size().y)
        });
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let painter = ui.painter_at(inner);
    for (i, g) in galleys.into_iter().enumerate() {
        let top = inner.top() + tops[i] - scroll.offset;
        if top > inner.bottom() || top + g.size().y < inner.top() {
            continue;
        }
        let colour = if Some(i) == current {
            palette::playing()
        } else if Some(i) == hovered {
            palette::text_bright()
        } else if text.synced {
            palette::text_weak()
        } else {
            palette::list_text()
        };
        painter.galley(pos2(inner.center().x, top), g, colour);
    }
    fades(&painter, inner, scroll.offset > 0.5, scroll.offset < most - 0.5);

    if resp.clicked() {
        return hovered.and_then(|i| text.lines[i].at_ms);
    }
    None
}

/// The lines fading into the ground at an edge with more beyond it.
fn fades(painter: &egui::Painter, inner: Rect, top: bool, bottom: bool) {
    let ground = palette::list_bg();
    let clear = egui::Color32::TRANSPARENT;
    let band = |mesh: &mut egui::Mesh, rect: Rect, upper: egui::Color32, lower: egui::Color32| {
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(rect.left_top(), upper);
        mesh.colored_vertex(rect.right_top(), upper);
        mesh.colored_vertex(rect.right_bottom(), lower);
        mesh.colored_vertex(rect.left_bottom(), lower);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    let mut mesh = egui::Mesh::default();
    if top {
        let r = Rect::from_min_size(inner.left_top() - vec2(PAD, 0.0), vec2(inner.width() + 2.0 * PAD, FADE));
        band(&mut mesh, r, ground, clear);
    }
    if bottom {
        let r = Rect::from_min_size(
            pos2(inner.left() - PAD, inner.bottom() - FADE),
            vec2(inner.width() + 2.0 * PAD, FADE),
        );
        band(&mut mesh, r, clear, ground);
    }
    if !mesh.is_empty() {
        painter.add(mesh);
    }
}
