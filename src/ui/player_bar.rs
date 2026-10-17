//! The player bar under the title row: Previous, Play/Pause (round, in
//! the accent), Stop, Next; the cover, the title and the artist of the
//! track heard; shuffle, repeat and the volume on the right; the seek bar
//! across the bottom with the times at its ends.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use crate::format;
use crate::palette;
use crate::player::Status;
use crate::queue::Repeat;
use crate::ui::icons::{self, Icon};

pub const HEIGHT: f32 = 76.0;

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
}

/// What the bar shows.
pub struct NowPlaying<'a> {
    pub status: Status,
    pub title: &'a str,
    pub artist: &'a str,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: u8,
    pub mute: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub cover: Option<egui::TextureHandle>,
}

pub fn show(ui: &mut Ui, now: &NowPlaying<'_>) -> Vec<BarCmd> {
    let mut cmds = Vec::new();
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, palette::BAR_BG);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, palette::LINE),
    );
    // The free space drags the window, as the title row does.
    crate::frameless::drag_area(ui, rect, ui.id().with("bar_drag"), Sense::hover());

    let top = Rect::from_min_size(rect.min, vec2(rect.width(), 50.0));
    let cy = top.center().y + 2.0;

    // The transport buttons.
    let mut x = rect.left() + 14.0;
    let mut button = |ui: &mut Ui, icon: Icon, size: f32, cmd: BarCmd, tip: &str, round: bool| {
        let d = if round { 40.0 } else { 32.0 };
        let r = Rect::from_center_size(pos2(x + d / 2.0, cy), vec2(d, d));
        x += d + 4.0;
        let resp = ui.interact(r, ui.id().with(("bar", tip)), Sense::click());
        let (fill, color) = if round {
            let fill = if resp.hovered() {
                palette::ACCENT_HOVER
            } else {
                palette::ACCENT
            };
            (Some(fill), Color32::from_rgb(0x1a, 0x1b, 0x1f))
        } else if resp.hovered() {
            (Some(palette::HOVER), palette::TEXT_BRIGHT)
        } else {
            (None, palette::TEXT)
        };
        if let Some(fill) = fill {
            painter.circle_filled(r.center(), d / 2.0, fill);
        }
        icons::paint(
            &painter,
            icon,
            r.center(),
            size,
            color,
            fill.unwrap_or(palette::BAR_BG),
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

    // The right end: shuffle, repeat, the volume.
    let mut right = rect.right() - 14.0;
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
        painter.rect_filled(speaker, 4.0, palette::HOVER);
    }
    icons::paint(
        &painter,
        vol_icon,
        speaker.center(),
        15.0,
        palette::TEXT,
        palette::BAR_BG,
    );
    if resp
        .on_hover_text(tr!("Mute (M)", "Без звука (M)"))
        .clicked()
    {
        cmds.push(BarCmd::Mute);
    }
    // The volume slider: a click or a drag sets it, the wheel steps it.
    let resp = ui.interact(
        slider.expand2(vec2(4.0, 6.0)),
        ui.id().with("volume"),
        Sense::click_and_drag(),
    );
    let shown = if now.mute { 0 } else { now.volume };
    let track = Rect::from_center_size(slider.center(), vec2(slider.width(), 4.0));
    painter.rect_filled(track, 2.0, palette::TRACK);
    let fill_x = track.left() + track.width() * f32::from(shown) / 100.0;
    painter.rect_filled(
        Rect::from_min_max(track.min, pos2(fill_x, track.bottom())),
        2.0,
        palette::ACCENT,
    );
    if resp.hovered() || resp.dragged() {
        painter.circle_filled(pos2(fill_x, track.center().y), 6.0, palette::TEXT_BRIGHT);
    }
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let v = ((p.x - track.left()) / track.width() * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8;
        cmds.push(BarCmd::Volume(v));
    }
    if resp.hovered() || ui.rect_contains_pointer(speaker) {
        let notches = crate::ui::wheel_notches(ui);
        if notches != 0 {
            let v = (i32::from(now.volume) + notches * 5).clamp(0, 100) as u8;
            cmds.push(BarCmd::Volume(v));
        }
    }
    resp.on_hover_text(format!("{} %", now.volume));

    let mut toggle = |ui: &mut Ui, icon: Icon, on: bool, cmd: BarCmd, tip: String| {
        let r = Rect::from_center_size(pos2(right - 16.0, cy), vec2(30.0, 28.0));
        right = r.left() - 2.0;
        let resp = ui.interact(r, ui.id().with(("toggle", icon)), Sense::click());
        if on {
            painter.rect_filled(r, 4.0, palette::SELECTED_BG);
        } else if resp.hovered() {
            painter.rect_filled(r, 4.0, palette::HOVER);
        }
        let color = if on {
            palette::ACCENT
        } else {
            palette::TEXT_WEAK
        };
        icons::paint(&painter, icon, r.center(), 15.0, color, palette::BAR_BG);
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

    // The cover and the names between.
    let cover = Rect::from_min_size(pos2(x + 14.0, cy - 21.0), vec2(42.0, 42.0));
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
            painter.rect_filled(cover, 3.0, palette::CARD_BG);
        }
    }
    let text_left = cover.right() + 12.0;
    let room = (right - 16.0 - text_left).max(0.0);
    let line = |text: &str, size: f32, color: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(
            text.to_owned(),
            FontId::proportional(size),
            color,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    if now.status != Status::Stopped || !now.title.is_empty() {
        let g1 = line(now.title, 15.0, palette::TEXT_BRIGHT);
        let g2 = line(now.artist, 12.5, palette::TEXT_WEAK);
        painter.galley(pos2(text_left, cy - 19.0), g1, palette::TEXT_BRIGHT);
        painter.galley(pos2(text_left, cy + 2.0), g2, palette::TEXT_WEAK);
    }

    // The seek bar across the bottom.
    let bottom = Rect::from_min_max(pos2(rect.left(), top.bottom()), rect.max);
    let times_w = 52.0;
    let bar = Rect::from_min_max(
        pos2(bottom.left() + 14.0 + times_w, bottom.center().y - 6.0),
        pos2(bottom.right() - 14.0 - times_w, bottom.center().y + 6.0),
    );
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
    painter.rect_filled(line_r, 2.0, palette::TRACK);
    let x_at = line_r.left() + line_r.width() * share;
    painter.rect_filled(
        Rect::from_min_max(line_r.min, pos2(x_at, line_r.bottom())),
        2.0,
        palette::ACCENT,
    );
    if duration > 0 && (resp.hovered() || resp.dragged()) {
        painter.circle_filled(pos2(x_at, line_r.center().y), 6.0, palette::TEXT_BRIGHT);
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
    let time_color = palette::TEXT_WEAK;
    let font = FontId::proportional(12.0);
    if now.status != Status::Stopped {
        painter.text(
            pos2(bar.left() - 8.0, bar.center().y),
            Align2::RIGHT_CENTER,
            format::time(shown_ms),
            font.clone(),
            time_color,
        );
        if duration > 0 {
            painter.text(
                pos2(bar.right() + 8.0, bar.center().y),
                Align2::LEFT_CENTER,
                format::time(duration),
                font,
                time_color,
            );
        }
    }
    cmds
}
