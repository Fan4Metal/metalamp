//! The player bar under the title row, in one of two layouts
//! (`BarLayout`, a setting). `Full`:
//! the player on the left (`PLAYER_WIDTH`), the spectrum analyzer in the
//! room on the right. The player: the time played (a click: the time
//! left) in large digits, the cover, the title, the artist and album, the
//! bit rate, sample rate and channels; the seek bar with the length at
//! its end; Previous, Play/Pause (round, in the accent), Stop, Next;
//! shuffle, repeat, the mute button and the volume. `Classic`, the bar
//! before it: one row of the buttons, the cover and the names, shuffle,
//! repeat and the volume, the seek bar across the bottom with the times
//! at its ends.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use crate::format;
use crate::palette;
use crate::player::Status;
use crate::queue::Repeat;
use crate::settings::{BarLayout, Visual};
use crate::ui::icons::{self, Icon};
use crate::ui::visual::Visualizer;

/// The bar's height in each layout.
pub fn height(layout: BarLayout) -> f32 {
    match layout {
        BarLayout::Full => 118.0,
        BarLayout::Classic => 76.0,
    }
}

/// The player's part on the left of the full layout; the analyzer takes
/// the rest.
pub const PLAYER_WIDTH: f32 = 520.0;
const MARGIN: f32 = 14.0;
/// The room the time has (`-0:00` at the large font fits it).
const TIME_WIDTH: f32 = 92.0;
const COVER: f32 = 52.0;

/// What a click in the bar asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarCmd {
    Previous,
    PlayPause,
    Stop,
    Next,
    Shuffle,
    Repeat,
    Mute,
    Volume(u8),
    Seek(u64),
    /// The cover clicked: the file info window.
    Info,
    /// The time clicked: the time played or the time left.
    TimeMode,
    /// The display clicked, or one chosen in its menu.
    Visual(Visual),
}

/// What the bar shows.
pub struct NowPlaying<'a> {
    pub layout: BarLayout,
    pub visual: Visual,
    pub status: Status,
    pub title: &'a str,
    pub artist: &'a str,
    /// The track's kilobits per second, sample rate and channels (0: not
    /// known).
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// The time shown counts down to the end.
    pub remaining: bool,
    pub volume: u8,
    pub mute: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub cover: Option<egui::TextureHandle>,
}

impl NowPlaying<'_> {
    fn has_track(&self) -> bool {
        self.status != Status::Stopped || !self.title.is_empty()
    }
}

pub fn show(ui: &mut Ui, now: &NowPlaying<'_>, visual: &mut Visualizer) -> Vec<BarCmd> {
    let mut cmds = Vec::new();
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, palette::bar_bg());
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, palette::line()),
    );
    // The free space drags the window, as the title row does.
    crate::frameless::drag_area(ui, rect, ui.id().with("bar_drag"), Sense::hover());
    match now.layout {
        BarLayout::Full => full(ui, &painter, rect, now, visual, &mut cmds),
        BarLayout::Classic => classic(ui, &painter, rect, now, &mut cmds),
    }
    cmds
}

fn full(
    ui: &mut Ui,
    painter: &egui::Painter,
    rect: Rect,
    now: &NowPlaying<'_>,
    visual: &mut Visualizer,
    cmds: &mut Vec<BarCmd>,
) {
    let player = Rect::from_min_size(
        rect.min,
        vec2(PLAYER_WIDTH.min(rect.width()), rect.height()),
    );

    // The upper row: the time, the cover and the names.
    let info = Rect::from_min_max(
        pos2(player.left() + MARGIN, player.top() + 8.0),
        pos2(player.right() - MARGIN, player.top() + 8.0 + COVER),
    );
    let time_rect = Rect::from_min_size(info.min, vec2(TIME_WIDTH, info.height()));
    if now.status != Status::Stopped {
        let (text, tip) = if now.remaining && now.duration_ms > 0 {
            (
                format!(
                    "-{}",
                    format::time(now.duration_ms.saturating_sub(now.position_ms))
                ),
                tr!(
                    "Time left; a click shows the time played",
                    "Осталось; щелчок покажет прошедшее время"
                ),
            )
        } else {
            (
                format::time(now.position_ms),
                tr!(
                    "Time played; a click shows the time left",
                    "Прошло; щелчок покажет оставшееся время"
                ),
            )
        };
        // The largest of the fonts the text fits in; past ten hours left
        // ("-10:00:00") not even the smallest does, and that one is used.
        let layout = |size| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap(
                    text.clone(),
                    FontId::monospace(size),
                    palette::text_bright(),
                )
            })
        };
        let galley = [30.0, 24.0, 18.0]
            .into_iter()
            .map(layout)
            .find(|g| g.size().x <= TIME_WIDTH)
            .unwrap_or_else(|| layout(15.0));
        let resp = ui
            .interact(time_rect, ui.id().with("time"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let color = if resp.hovered() {
            palette::accent_hover()
        } else {
            palette::text_bright()
        };
        let pos = pos2(
            time_rect.left(),
            time_rect.center().y - galley.size().y / 2.0,
        );
        painter.galley(pos, galley, color);
        if resp.on_hover_text(tip).clicked() {
            cmds.push(BarCmd::TimeMode);
        }
    }

    let cover = Rect::from_min_size(
        pos2(time_rect.right() + 6.0, info.top()),
        vec2(COVER, COVER),
    );
    cover_button(ui, painter, cover, now, cmds);
    let text_left = cover.right() + 12.0;
    let room = (info.right() - text_left).max(0.0);
    if now.has_track() {
        let g1 = line(ui, now.title, 15.0, palette::text_bright(), room);
        let g2 = line(ui, now.artist, 12.5, palette::text_weak(), room);
        let g3 = line(ui, &facts(now), 11.5, palette::text_weak(), room);
        painter.galley(
            pos2(text_left, info.top() - 1.0),
            g1,
            palette::text_bright(),
        );
        painter.galley(pos2(text_left, info.top() + 19.0), g2, palette::text_weak());
        painter.galley(pos2(text_left, info.top() + 36.0), g3, palette::text_weak());
    }

    // The seek bar, the length at its end.
    let seek_y = info.bottom() + 10.0;
    let length_w = 44.0;
    let bar = Rect::from_min_max(
        pos2(player.left() + MARGIN, seek_y - 6.0),
        pos2(player.right() - MARGIN - length_w, seek_y + 6.0),
    );
    seek_bar(ui, painter, bar, now, cmds);
    if now.status != Status::Stopped && now.duration_ms > 0 {
        painter.text(
            pos2(player.right() - MARGIN, seek_y),
            Align2::RIGHT_CENTER,
            format::time(now.duration_ms),
            FontId::proportional(12.0),
            palette::text_weak(),
        );
    }

    // The lower row: the transport buttons on the left; shuffle, repeat
    // and the volume on the right.
    let cy = (seek_y + 8.0 + player.bottom()) / 2.0;
    transport(ui, painter, player.left() + MARGIN, cy, now, cmds);
    volume_and_toggles(ui, painter, player.right() - MARGIN, cy, now, cmds);

    // The visualization: a dark display in the room on the right; a click
    // goes on to the next one, the right button lists them.
    if let Some(display) = analyzer_rect(rect) {
        painter.rect(
            display,
            4.0,
            palette::list_bg(),
            egui::Stroke::new(1.0, palette::line()),
            egui::StrokeKind::Inside,
        );
        visual.paint(painter, display.shrink(4.0));
        let response = ui
            .interact(display, ui.id().with("visual"), Sense::click())
            .on_hover_text(format!(
                "{}. {}",
                now.visual.label(),
                tr!(
                    "A click: the next one; the right button: the list",
                    "Щелчок: следующая; правая кнопка: список"
                )
            ));
        if response.clicked() {
            cmds.push(BarCmd::Visual(now.visual.next()));
        }
        egui::Popup::context_menu(&response).show(|ui| {
            ui.set_min_width(160.0);
            for v in Visual::ALL {
                if v == Visual::Off {
                    ui.separator();
                }
                if ui.selectable_label(v == now.visual, v.label()).clicked() {
                    cmds.push(BarCmd::Visual(v));
                    ui.close();
                }
            }
        });
    }
}

/// The bar before the full layout: one row, the seek bar under it.
fn classic(
    ui: &mut Ui,
    painter: &egui::Painter,
    rect: Rect,
    now: &NowPlaying<'_>,
    cmds: &mut Vec<BarCmd>,
) {
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), 50.0));
    let cy = top.center().y + 2.0;
    let x = transport(ui, painter, rect.left() + MARGIN, cy, now, cmds);
    let right = volume_and_toggles(ui, painter, rect.right() - MARGIN, cy, now, cmds);

    // The cover and the names between.
    let cover = Rect::from_min_size(pos2(x + 14.0, cy - 21.0), vec2(42.0, 42.0));
    cover_button(ui, painter, cover, now, cmds);
    let text_left = cover.right() + 12.0;
    let room = (right - 16.0 - text_left).max(0.0);
    if now.has_track() {
        let g1 = line(ui, now.title, 15.0, palette::text_bright(), room);
        let g2 = line(ui, now.artist, 12.5, palette::text_weak(), room);
        painter.galley(pos2(text_left, cy - 19.0), g1, palette::text_bright());
        painter.galley(pos2(text_left, cy + 2.0), g2, palette::text_weak());
    }

    // The seek bar across the bottom, the times at its ends.
    let bottom = Rect::from_min_max(pos2(rect.left(), top.bottom()), rect.max);
    let times_w = 52.0;
    let bar = Rect::from_min_max(
        pos2(bottom.left() + MARGIN + times_w, bottom.center().y - 6.0),
        pos2(bottom.right() - MARGIN - times_w, bottom.center().y + 6.0),
    );
    let shown_ms = seek_bar(ui, painter, bar, now, cmds);
    let font = FontId::proportional(12.0);
    if now.status != Status::Stopped {
        painter.text(
            pos2(bar.left() - 8.0, bar.center().y),
            Align2::RIGHT_CENTER,
            format::time(shown_ms),
            font.clone(),
            palette::text_weak(),
        );
        if now.duration_ms > 0 {
            painter.text(
                pos2(bar.right() + 8.0, bar.center().y),
                Align2::LEFT_CENTER,
                format::time(now.duration_ms),
                font,
                palette::text_weak(),
            );
        }
    }
}

/// Where the analyzer's display sits in a bar of `rect` in the full
/// layout; `None` when the window is too narrow for it.
pub fn analyzer_rect(rect: Rect) -> Option<Rect> {
    let display = Rect::from_min_max(
        pos2(rect.left() + PLAYER_WIDTH + 6.0, rect.top() + 10.0),
        pos2(rect.right() - MARGIN, rect.bottom() - 11.0),
    );
    (display.width() >= 60.0).then_some(display)
}

/// The size of the visualization in a bar of `rect` (none in the classic
/// layout).
pub fn visual_size(layout: BarLayout, rect: Rect) -> Option<egui::Vec2> {
    match layout {
        BarLayout::Full => analyzer_rect(rect).map(|r| r.shrink(4.0).size()),
        BarLayout::Classic => None,
    }
}

/// The third line: `320 kbps · 44.1 kHz · Stereo`.
fn facts(now: &NowPlaying<'_>) -> String {
    let mut parts = Vec::new();
    if now.bitrate > 0 {
        parts.push(format::bitrate(now.bitrate));
    }
    if now.sample_rate > 0 {
        parts.push(format::sample_rate(now.sample_rate));
    }
    if let Some(c) = format::channels(now.channels) {
        parts.push(c);
    }
    parts.join(" · ")
}

/// One line of text cut to `room` points.
fn line(
    ui: &mut Ui,
    text: &str,
    size: f32,
    color: Color32,
    room: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.to_owned(),
        FontId::proportional(size),
        color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    ui.fonts_mut(|f| f.layout_job(job))
}

/// The cover in `cover`: a click opens the file info while there is a
/// track.
fn cover_button(
    ui: &mut Ui,
    painter: &egui::Painter,
    cover: Rect,
    now: &NowPlaying<'_>,
    cmds: &mut Vec<BarCmd>,
) {
    if now.has_track() {
        let resp = ui
            .interact(cover, ui.id().with("cover"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp
            .on_hover_text(tr!("File info (I)", "Сведения о файле (I)"))
            .clicked()
        {
            cmds.push(BarCmd::Info);
        }
    }
    match &now.cover {
        Some(t) => {
            painter.image(
                t.id(),
                cover,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(cover, 3.0, palette::card_bg());
        }
    }
}

/// Previous, Play/Pause, Stop and Next from `x` at height `cy`; where
/// they end.
fn transport(
    ui: &mut Ui,
    painter: &egui::Painter,
    mut x: f32,
    cy: f32,
    now: &NowPlaying<'_>,
    cmds: &mut Vec<BarCmd>,
) -> f32 {
    let mut button = |ui: &mut Ui, icon: Icon, size: f32, cmd: BarCmd, tip: &str, round: bool| {
        let d = if round { 36.0 } else { 30.0 };
        let r = Rect::from_center_size(pos2(x + d / 2.0, cy), vec2(d, d));
        x += d + 4.0;
        let resp = ui.interact(r, ui.id().with(("bar", tip)), Sense::click());
        let (fill, color) = if round {
            let fill = if resp.hovered() {
                palette::accent_hover()
            } else {
                palette::accent()
            };
            (Some(fill), palette::on_accent())
        } else if resp.hovered() {
            (Some(palette::hover()), palette::text_bright())
        } else {
            (None, palette::text())
        };
        if let Some(fill) = fill {
            painter.circle_filled(r.center(), d / 2.0, fill);
        }
        icons::paint(
            painter,
            icon,
            r.center(),
            size,
            color,
            fill.unwrap_or(palette::bar_bg()),
        );
        if resp.on_hover_text(tip).clicked() {
            cmds.push(cmd);
        }
    };
    button(
        ui,
        Icon::SkipBack,
        15.0,
        BarCmd::Previous,
        tr!("Previous (Ctrl+Left, Z)", "Предыдущий (Ctrl+Влево, Z)"),
        false,
    );
    let (icon, tip) = if now.status == Status::Playing {
        (Icon::Pause, tr!("Pause (Space)", "Пауза (Пробел)"))
    } else {
        (Icon::Play, tr!("Play (Space)", "Воспроизвести (Пробел)"))
    };
    button(ui, icon, 17.0, BarCmd::PlayPause, tip, true);
    button(
        ui,
        Icon::Stop,
        14.0,
        BarCmd::Stop,
        tr!("Stop (V)", "Стоп (V)"),
        false,
    );
    button(
        ui,
        Icon::SkipForward,
        15.0,
        BarCmd::Next,
        tr!("Next (Ctrl+Right, B)", "Следующий (Ctrl+Вправо, B)"),
        false,
    );
    x
}

/// The volume slider, the mute button, repeat and shuffle, laid right to
/// left from `right` at height `cy`; where they end.
fn volume_and_toggles(
    ui: &mut Ui,
    painter: &egui::Painter,
    mut right: f32,
    cy: f32,
    now: &NowPlaying<'_>,
    cmds: &mut Vec<BarCmd>,
) -> f32 {
    let slider = Rect::from_min_max(pos2(right - 100.0, cy - 8.0), pos2(right, cy + 8.0));
    right = slider.left() - 6.0;
    let speaker = Rect::from_center_size(pos2(right - 14.0, cy), vec2(28.0, 28.0));
    right = speaker.left() - 12.0;
    let vol_icon = if now.mute || now.volume == 0 {
        Icon::Mute
    } else {
        Icon::Volume(match now.volume {
            0..=33 => 0,
            34..=66 => 1,
            _ => 2,
        })
    };
    let resp = ui.interact(speaker, ui.id().with("mute"), Sense::click());
    if resp.hovered() {
        painter.rect_filled(speaker, 4.0, palette::hover());
    }
    icons::paint(
        painter,
        vol_icon,
        speaker.center(),
        15.0,
        palette::text(),
        palette::bar_bg(),
    );
    if resp
        .on_hover_text(tr!("Mute (M)", "Без звука (M)"))
        .clicked()
    {
        cmds.push(BarCmd::Mute);
    }
    // The volume slider: a click or a drag sets it (the wheel steps it
    // anywhere over the title row and this bar: `App::ui`).
    let resp = ui.interact(
        slider.expand2(vec2(4.0, 6.0)),
        ui.id().with("volume"),
        Sense::click_and_drag(),
    );
    let shown = if now.mute { 0 } else { now.volume };
    let track = Rect::from_center_size(slider.center(), vec2(slider.width(), 4.0));
    painter.rect_filled(track, 2.0, palette::track());
    let fill_x = track.left() + track.width() * f32::from(shown) / 100.0;
    painter.rect_filled(
        Rect::from_min_max(track.min, pos2(fill_x, track.bottom())),
        2.0,
        palette::accent(),
    );
    if resp.hovered() || resp.dragged() {
        painter.circle_filled(pos2(fill_x, track.center().y), 6.0, palette::text_bright());
    }
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let v = ((p.x - track.left()) / track.width() * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8;
        cmds.push(BarCmd::Volume(v));
    }
    resp.on_hover_text(format!("{} %", now.volume));

    let mut toggle = |ui: &mut Ui, icon: Icon, on: bool, cmd: BarCmd, tip: String| {
        let r = Rect::from_center_size(pos2(right - 16.0, cy), vec2(30.0, 28.0));
        right = r.left() - 2.0;
        let resp = ui.interact(r, ui.id().with(("toggle", icon)), Sense::click());
        if on {
            painter.rect_filled(r, 4.0, palette::selected_bg());
        } else if resp.hovered() {
            painter.rect_filled(r, 4.0, palette::hover());
        }
        let color = if on {
            palette::accent()
        } else {
            palette::text_weak()
        };
        icons::paint(painter, icon, r.center(), 15.0, color, palette::bar_bg());
        if resp.on_hover_text(tip).clicked() {
            cmds.push(cmd);
        }
    };
    let (icon, tip) = match now.repeat {
        Repeat::Off => (Icon::Repeat, tr!("Repeat: off (R)", "Повтор: выкл. (R)")),
        Repeat::All => (
            Icon::Repeat,
            tr!("Repeat: the playlist (R)", "Повтор: плейлист (R)"),
        ),
        Repeat::One => (
            Icon::RepeatOne,
            tr!("Repeat: the track (R)", "Повтор: трек (R)"),
        ),
    };
    toggle(
        ui,
        icon,
        now.repeat != Repeat::Off,
        BarCmd::Repeat,
        tip.to_owned(),
    );
    let tip = if now.shuffle {
        tr!("Shuffle: on (S)", "Перемешать: вкл. (S)")
    } else {
        tr!("Shuffle: off (S)", "Перемешать: выкл. (S)")
    };
    toggle(
        ui,
        Icon::Shuffle,
        now.shuffle,
        BarCmd::Shuffle,
        tip.to_owned(),
    );
    right
}

/// The seek bar in `bar`: seeks when let go, the time under the pointer
/// as a tip; the position it shows (the one dragged to while dragging).
fn seek_bar(
    ui: &mut Ui,
    painter: &egui::Painter,
    bar: Rect,
    now: &NowPlaying<'_>,
    cmds: &mut Vec<BarCmd>,
) -> u64 {
    let duration = now.duration_ms;
    let resp = ui.interact(
        bar.expand2(vec2(0.0, 4.0)),
        ui.id().with("seek"),
        Sense::click_and_drag(),
    );
    let target = resp
        .interact_pointer_pos()
        .filter(|_| duration > 0)
        .map(|p| {
            let share = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
            (share as f64 * duration as f64) as u64
        });
    let shown_ms = if resp.dragged() {
        target.unwrap_or(now.position_ms)
    } else {
        now.position_ms
    };
    let share = if duration > 0 {
        (shown_ms as f32 / duration as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let line_r = Rect::from_center_size(bar.center(), vec2(bar.width(), 4.0));
    painter.rect_filled(line_r, 2.0, palette::track());
    let x_at = line_r.left() + line_r.width() * share;
    painter.rect_filled(
        Rect::from_min_max(line_r.min, pos2(x_at, line_r.bottom())),
        2.0,
        palette::accent(),
    );
    if duration > 0 && (resp.hovered() || resp.dragged()) {
        painter.circle_filled(pos2(x_at, line_r.center().y), 6.0, palette::text_bright());
    }
    if (resp.drag_stopped() || (resp.clicked() && !resp.dragged()))
        && let Some(ms) = target
    {
        cmds.push(BarCmd::Seek(ms));
    }
    if let Some(p) = resp.hover_pos().filter(|_| duration > 0) {
        let share = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
        resp.on_hover_text_at_pointer(format::time((share as f64 * duration as f64) as u64));
    }
    shown_ms
}
