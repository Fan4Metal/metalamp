//! The lyrics of the track heard in a rectangle (under the playlist, in
//! the cover's place): the lines centred; synced ones lit as they are
//! sung, the line sung kept at [`FOLLOW_AT`] of the height by a scroll
//! easing to it (the wheel scrolls by hand, and the following waits
//! [`HAND_S`]), a click on a line seeking to it; plain ones scrolled by the
//! wheel. While looking, a spinner; without lyrics, a word why. The right
//! button: the cover, copy, LRCLIB asked again, hide. `LyricsWindow`
//! shows them in a window of their own (the player and playlist layout).

use egui::{Align, FontId, Rect, Sense, Ui, pos2, vec2};

use crate::lyrics::{How, Note, Source, State, Text};
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
    /// The lyrics looked for again this way.
    Ask(How),
    /// From under the playlist into a window of their own, and back.
    ToWindow,
    ToPanel,
    /// LRCLIB's lyrics written into the file's tag.
    SaveToTag,
}

/// Where the lyrics are shown, for their menu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Under the playlist, in the cover's place.
    Panel,
    /// In a window of their own; `dock`: they may go back under the
    /// playlist (the layout with the library).
    Window { dock: bool },
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

/// The lyrics in `rect`, shown at `place` (for the menu's items).
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    id: egui::Id,
    state: &State,
    position_ms: u64,
    online: bool,
    place: Place,
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
            // LRCLIB's lyrics asked for and not had: a line over the own.
            let why = match &text.note {
                Note::None => None,
                Note::LrclibHasNone => Some(tr!(
                    "LRCLIB has no lyrics of this track",
                    "В LRCLIB нет текста этого трека"
                )),
                Note::LrclibFailed(_) => Some(tr!("LRCLIB did not answer", "LRCLIB не ответил")),
                Note::SaveFailed(e) if *e == crate::library::edit::read_only() => Some(tr!(
                    "The file is read-only: the lyrics were not written",
                    "Файл только для чтения: текст не записан"
                )),
                Note::SaveFailed(_) => Some(tr!(
                    "The lyrics could not be written into the tag",
                    "Не удалось записать текст в тег"
                )),
            };
            let mut inner = inner;
            if let Some(why) = why {
                let line = painter.layout(
                    why.to_owned(),
                    FontId::proportional(11.5),
                    palette::text_weak(),
                    inner.width(),
                );
                let h = line.size().y;
                painter.galley(
                    pos2(inner.center().x - line.size().x / 2.0, inner.top()),
                    line,
                    palette::text_weak(),
                );
                inner.min.y += h + GAP;
                if let Note::LrclibFailed(e) | Note::SaveFailed(e) = &text.note
                    && resp.hovered()
                {
                    resp.clone().on_hover_text(e.as_str());
                }
            }
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
            match place {
                Place::Panel => {
                    if ui
                        .button(tr!("Open in a window", "Открыть в отдельном окне"))
                        .clicked()
                    {
                        acts.push(Act::ToWindow);
                        ui.close();
                    }
                    if ui.button(tr!("Show the cover", "Показать обложку")).clicked() {
                        acts.push(Act::ShowCover);
                        ui.close();
                    }
                }
                Place::Window { dock: true } => {
                    if ui
                        .button(tr!("Show under the playlist", "Показать под плейлистом"))
                        .clicked()
                    {
                        acts.push(Act::ToPanel);
                        ui.close();
                    }
                }
                Place::Window { dock: false } => {}
            }
            if let State::Found(text) = state
                && ui.button(tr!("Copy the lyrics", "Копировать текст")).clicked()
            {
                ui.ctx().copy_text(text.plain());
                ui.close();
            }
            let (again, from_lrclib, own) = match state {
                State::Found(t) if t.source == Source::Lrclib => (true, false, t.own),
                // The file's own: LRCLIB's may be shown over them.
                State::Found(_) => (false, true, None),
                State::None(_) | State::Instrumental => (true, false, None),
                State::Idle | State::Looking => (false, false, None),
            };
            if online
                && again
                && ui
                    .button(tr!("Look on LRCLIB again", "Искать в LRCLIB заново"))
                    .clicked()
            {
                acts.push(Act::Ask(How::Fresh));
                ui.close();
            }
            if online
                && from_lrclib
                && ui
                    .button(tr!("Show LRCLIB's lyrics", "Показать текст из LRCLIB"))
                    .on_hover_text(tr!(
                        "Shown in place of the file's own from then on for this track; the file is not changed.",
                        "Для этого трека он будет показываться и дальше вместо своего; файл не меняется."
                    ))
                    .clicked()
            {
                acts.push(Act::Ask(How::Lrclib));
                ui.close();
            }
            if let State::Found(t) = state
                && t.source == Source::Lrclib
                && t.writable
                && ui
                    .button(tr!("Save into the tag", "Сохранить в тег"))
                    .on_hover_text(tr!(
                        "The lyrics written into the file's tag, synced ones with their times (LRC), as the file info writes tags; the file then has them without the internet.",
                        "Текст записывается в тег файла, синхронизированный — с метками времени (LRC), так же, как теги пишет окно сведений о файле; после этого он есть в файле и без интернета."
                    ))
                    .clicked()
            {
                acts.push(Act::SaveToTag);
                ui.close();
            }
            if let Some(own) = own {
                let label = match own {
                    Source::File => tr!("Back to the .lrc file's lyrics", "Вернуть текст из файла .lrc"),
                    _ => tr!("Back to the tag's lyrics", "Вернуть текст из тега"),
                };
                if ui.button(label).clicked() {
                    acts.push(Act::Ask(How::Own));
                    ui.close();
                }
            }
            ui.separator();
            let hide = if place != Place::Panel {
                tr!("Close", "Закрыть")
            } else {
                tr!("Hide the lyrics", "Скрыть текст")
            };
            if ui.button(hide).clicked() {
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

/// The window's title, by which it is found.
fn window_title() -> &'static str {
    tr!("Lyrics", "Текст песни")
}

fn window_viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("lyrics_window")
}

/// The window's size at first, and its least.
const WINDOW_SIZE: egui::Vec2 = vec2(420.0, 560.0);
const WINDOW_MIN: egui::Vec2 = vec2(260.0, 220.0);
/// The heading's height (it moves the window) and the room round it all.
const HEADING: f32 = 48.0;
const MARGIN: f32 = 10.0;

/// The lyrics in a window of their own (the player and playlist layout,
/// whose playlist has no room for them): frameless, resizable, owned by
/// the program's window, off the taskbar; the track's title and artist
/// on top (moving it), the cross; Esc closes it. Its place is the
/// settings' (`lyrics_window_place`, points), else right of the program's
/// window when the screen has room there, else over it.
#[derive(Default)]
pub struct LyricsWindow {
    window: super::OwnWindow,
    /// Where it was put as it opened, given to the builder each frame (a
    /// place following the window's own would fight its moves).
    opened: Option<Rect>,
    /// The player's keys pressed in it, for the app to run (as the sound
    /// tuning window's).
    pub cmds: Vec<crate::keymap::Cmd>,
}

/// What the window shows: the title and the second line of the track.
pub struct Shown<'a> {
    pub title: &'a str,
    pub second: &'a str,
    pub state: &'a State,
    pub position_ms: u64,
    pub online: bool,
    /// The lyrics may go back under the playlist (the layout with the
    /// library).
    pub dock: bool,
}

impl LyricsWindow {
    /// Shows the window while `open`; `place` its rectangle, kept as it
    /// moves (returns whether it moved). The panel's acts are returned;
    /// closed, `open` is cleared. The chords of `keymap` (none while a
    /// modal is open) go to [`LyricsWindow::cmds`].
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        owner: Option<isize>,
        open: &mut bool,
        place: &mut Option<[f32; 4]>,
        shown: &Shown,
        keymap: Option<&crate::keymap::Keymap>,
    ) -> (Vec<Act>, bool) {
        if !*open {
            if self.opened.take().is_some() {
                self.window.reset();
            }
            return (Vec::new(), false);
        }
        let opened = *self.opened.get_or_insert_with(|| first_place(ctx, *place));
        let builder = super::OwnWindow::builder(egui::ViewportBuilder::default())
            .with_title(window_title())
            .with_decorations(false)
            .with_resizable(true)
            .with_inner_size(opened.size())
            .with_min_inner_size(WINDOW_MIN)
            .with_position(opened.min)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            .with_icon(crate::window_icon());
        let dark = palette::current().dark;
        let mut acts = Vec::new();
        let mut close = false;
        let mut moved = false;
        ctx.show_viewport_immediate(window_viewport(), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            self.window.follow(&ctx, window_title(), owner, dark);
            close |= ctx.input(|i| i.viewport().close_requested());
            if !egui::Popup::is_any_open(&ctx)
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                close = true;
            }
            // The player's keys: the window has no text field, so all of
            // them.
            if let Some(keymap) = keymap {
                self.cmds.extend(crate::keymap::take(&ctx, keymap, |_| true));
            }
            if let (Some(outer), Some(inner)) =
                ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect))
                && self.window.hwnd().is_some()
            {
                let now = [outer.left(), outer.top(), inner.width(), inner.height()];
                if place.is_none_or(|p| p.iter().zip(now).any(|(a, b)| (a - b).abs() > 0.5)) {
                    *place = Some(now);
                    moved = true;
                }
            }
            crate::frameless::resize_edges(ui, true);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(palette::panel_bg())
                        .inner_margin(egui::Margin::same(MARGIN as i8)),
                )
                .show(ui, |ui| {
                    let top = Rect::from_min_size(
                        ctx.content_rect().min,
                        vec2(ctx.content_rect().width(), HEADING),
                    );
                    crate::frameless::drag_area(ui, top, ui.id().with("lyrics_drag"), Sense::hover());
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            close |= super::close_cross(ui);
                            ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(shown.title)
                                            .size(16.0)
                                            .strong()
                                            .color(palette::text_bright()),
                                    )
                                    .truncate()
                                    .selectable(false),
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(shown.second)
                                            .size(12.0)
                                            .color(palette::text_weak()),
                                    )
                                    .truncate()
                                    .selectable(false),
                                );
                            });
                        });
                    });
                    ui.add_space(8.0);
                    let room = ui.available_rect_before_wrap();
                    ui.allocate_rect(room, Sense::hover());
                    acts = show(
                        ui,
                        room,
                        egui::Id::new("lyrics_window_panel"),
                        shown.state,
                        shown.position_ms,
                        shown.online,
                        Place::Window { dock: shown.dock },
                    );
                });
        });
        if acts.iter().any(|a| matches!(a, Act::Hide | Act::ToPanel)) {
            close = true;
        }
        if close {
            *open = false;
            self.opened = None;
            self.window.reset();
        }
        (acts, moved)
    }
}

/// Where the window opens: its place kept when it is still on a monitor,
/// else right of the program's window when the work area has room there,
/// else over its middle.
fn first_place(ctx: &egui::Context, place: Option<[f32; 4]>) -> Rect {
    let ppp = ctx.pixels_per_point();
    let pixels = |r: Rect| {
        [
            (r.left() * ppp) as i32,
            (r.top() * ppp) as i32,
            (r.right() * ppp) as i32,
            (r.bottom() * ppp) as i32,
        ]
    };
    if let Some([x, y, w, h]) = place {
        let r = Rect::from_min_size(pos2(x, y), vec2(w, h).max(WINDOW_MIN));
        if crate::win::on_a_monitor(pixels(r)) {
            return r;
        }
    }
    let main = ctx
        .input(|i| i.viewport().outer_rect)
        .unwrap_or(ctx.content_rect());
    // As tall as the program's window, beside it.
    let size = vec2(WINDOW_SIZE.x, main.height().max(WINDOW_MIN.y));
    let beside = Rect::from_min_size(pos2(main.right() + 8.0, main.top()), size);
    let fits = crate::win::work_area_at(pixels(main)).is_some_and(|[_, _, right, bottom]| {
        beside.right() * ppp <= right as f32 && beside.bottom() * ppp <= bottom as f32
    });
    if fits {
        beside
    } else {
        Rect::from_center_size(main.center(), WINDOW_SIZE)
    }
}