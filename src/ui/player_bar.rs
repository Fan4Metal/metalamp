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
use crate::keymap::{Cmd, with_key};
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
        BarLayout::Winamp => WINAMP_HEIGHT,
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
    /// The equalizer's button clicked (its rect: the window opens under
    /// it).
    Equalizer(Rect),
    /// The mini player's way back to the whole window.
    Whole,
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
    /// The visualization at all (the settings' switch): off, the display
    /// takes no click and has no menu.
    pub visual_enabled: bool,
    /// A click on the display goes on to the next visualization.
    pub visual_click: bool,
    /// The equalizer is on.
    pub eq: bool,
    /// Playing stops after the track heard (Shift+V): Stop is lit.
    pub stop_after: bool,
    pub cover: Option<egui::TextureHandle>,
    /// The narrow bar's line: "N. Artist – Title", the playlist's.
    pub heading: String,
}

impl NowPlaying<'_> {
    pub(super) fn has_track(&self) -> bool {
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
        BarLayout::Winamp => winamp(ui, &painter, rect, now, visual, &mut cmds),
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
    time(ui, painter, time_rect, now, palette::text_bright(), cmds);

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
    let length = (now.status != Status::Stopped && now.duration_ms > 0).then(|| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(
                format::time(now.duration_ms),
                FontId::proportional(12.0),
                palette::text_weak(),
            )
        })
    });
    // Room for "4:05" at least, more for a length of hours.
    let length_w = length.as_ref().map_or(0.0, |g| g.size().x + 8.0).max(44.0);
    let bar = Rect::from_min_max(
        pos2(player.left() + MARGIN, seek_y - 6.0),
        pos2(player.right() - MARGIN - length_w, seek_y + 6.0),
    );
    seek_bar(ui, painter, bar, now, cmds);
    if let Some(g) = length {
        let at = pos2(
            player.right() - MARGIN - g.size().x,
            seek_y - g.size().y / 2.0,
        );
        painter.galley(at, g, palette::text_weak());
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
        visual_display(
            ui,
            painter,
            display,
            display.shrink(4.0),
            now,
            visual,
            true,
            cmds,
        );
    }
}

/// The time played (or left) in large digits in `time_rect`, the largest
/// of three sizes that fits; a click switches between the two.
fn time(
    ui: &mut Ui,
    painter: &egui::Painter,
    time_rect: Rect,
    now: &NowPlaying<'_>,
    colour: Color32,
    cmds: &mut Vec<BarCmd>,
) {
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
            .find(|g| g.size().x <= time_rect.width())
            .unwrap_or_else(|| layout(15.0));
        let resp = ui
            .interact(time_rect, ui.id().with("time"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let color = if resp.hovered() {
            palette::accent_hover()
        } else {
            colour
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
}

/// The visualization painted in `inner` of `display`; a click on the
/// display goes on to the next one, the right button lists them. `logo`:
/// "No visualization", or the visualization switched off, shows the
/// program's name (the full bar), else the room stays dark (the narrow
/// bar's, under the time).
#[allow(clippy::too_many_arguments)]
fn visual_display(
    ui: &mut Ui,
    painter: &egui::Painter,
    display: Rect,
    inner: Rect,
    now: &NowPlaying<'_>,
    visual: &mut Visualizer,
    logo: bool,
    cmds: &mut Vec<BarCmd>,
) {
    // Switched off in the settings (or by its key): the logo (or the dark
    // room), and no click and no menu.
    if !now.visual_enabled {
        if logo {
            visual.paint(painter, inner);
        }
        return;
    }
    if logo || now.visual != Visual::Off {
        visual.paint(painter, inner);
    }
    let how = if now.visual_click {
        tr!(
            "A click: the next one; the right button: the list",
            "Щелчок: следующая; правая кнопка: список"
        )
    } else {
        tr!("The right button: the list", "Правая кнопка: список")
    };
    let response = ui
        .interact(display, ui.id().with("visual"), Sense::click())
        .on_hover_text(format!("{}. {how}", now.visual.label()));
    if response.clicked() && now.visual_click {
        cmds.push(BarCmd::Visual(now.visual.next()));
    }
    egui::Popup::context_menu(&response).show(|ui| {
        ui.set_min_width(160.0);
        for v in Visual::ALL {
            if v.starts_group() {
                ui.separator();
            }
            if ui.selectable_label(v == now.visual, v.label()).clicked() {
                cmds.push(BarCmd::Visual(v));
                ui.close();
            }
        }
    });
}

/// The narrow bar of the player and playlist layout (the early Winamp's):
/// a display with the time over the visualization on the left; the title
/// in a field, the bit rate and the rest, the volume on the right; the
/// seek bar across; the buttons and the toggles under it.
fn winamp(
    ui: &mut Ui,
    painter: &egui::Painter,
    rect: Rect,
    now: &NowPlaying<'_>,
    visual: &mut Visualizer,
    cmds: &mut Vec<BarCmd>,
) {
    let display = winamp_display(rect);
    painter.rect(
        display,
        4.0,
        palette::list_bg(),
        egui::Stroke::new(1.0, palette::line()),
        egui::StrokeKind::Inside,
    );
    // The state at the left of the time, the time in the accent.
    let state = match now.status {
        Status::Playing => Icon::Play,
        Status::Paused => Icon::Pause,
        Status::Stopped => Icon::Stop,
    };
    let time_rect = Rect::from_min_max(
        pos2(display.left() + 26.0, display.top() + 4.0),
        pos2(display.right() - 8.0, display.top() + 40.0),
    );
    icons::paint(
        painter,
        state,
        pos2(display.left() + 14.0, time_rect.center().y),
        10.0,
        palette::accent(),
        palette::list_bg(),
    );
    visual_display(
        ui,
        painter,
        display,
        winamp_visual_rect(rect),
        now,
        visual,
        false,
        cmds,
    );
    // After the display, whose click covers it: a widget registered later
    // takes the click, and the time's went on to the next visualization.
    time(ui, painter, time_rect, now, palette::accent(), cmds);

    // The title in a field, the facts under it, the volume.
    let right = Rect::from_min_max(
        pos2(display.right() + 10.0, display.top()),
        pos2(rect.right() - MARGIN, display.bottom()),
    );
    let field = Rect::from_min_size(right.min, vec2(right.width(), 26.0));
    painter.rect(
        field,
        4.0,
        palette::list_bg(),
        egui::Stroke::new(1.0, palette::line()),
        egui::StrokeKind::Inside,
    );
    if now.has_track() {
        let text = &now.heading;
        let text = if now.duration_ms > 0 {
            format!("{text} ({})", format::time(now.duration_ms))
        } else {
            text.clone()
        };
        let room = field.width() - 16.0;
        let galley = line(ui, &text, 13.0, palette::text_bright(), room);
        let cut = galley.size().x >= room - 1.0;
        painter.galley(
            pos2(field.left() + 8.0, field.center().y - galley.size().y / 2.0),
            galley,
            palette::text_bright(),
        );
        // A double click: the file info.
        let resp = ui.interact(field, ui.id().with("winamp_title"), Sense::click());
        let tip = if cut {
            format!(
                "{text}\n{}",
                tr!(
                    "A double click: file info",
                    "Двойной щелчок: сведения о файле"
                )
            )
        } else {
            tr!(
                "A double click: file info",
                "Двойной щелчок: сведения о файле"
            )
            .to_owned()
        };
        if crate::ui::double_clicked(&resp.on_hover_text(tip)) {
            cmds.push(BarCmd::Info);
        }
        let facts = line(ui, &facts(now), 11.5, palette::text_weak(), right.width());
        painter.galley(
            pos2(right.left() + 2.0, field.bottom() + 8.0),
            facts,
            palette::text_weak(),
        );
    }
    let cy = right.bottom() - 12.0;
    let speaker = Rect::from_center_size(pos2(right.left() + 14.0, cy), vec2(28.0, 28.0));
    mute_button(ui, painter, speaker, now, cmds);
    let slider = Rect::from_min_max(
        pos2(speaker.right() + 6.0, cy - 8.0),
        pos2(right.right() - 4.0, cy + 8.0),
    );
    volume_slider(ui, painter, slider, now, cmds);

    // The seek bar across.
    let seek_y = display.bottom() + 14.0;
    let bar = Rect::from_min_max(
        pos2(rect.left() + MARGIN, seek_y - 6.0),
        pos2(rect.right() - MARGIN, seek_y + 6.0),
    );
    seek_bar(ui, painter, bar, now, cmds);

    // The buttons, the toggles at the right.
    let cy = (seek_y + 8.0 + rect.bottom()) / 2.0;
    transport(ui, painter, rect.left() + MARGIN, cy, now, cmds);
    toggles(ui, painter, rect.right() - MARGIN, cy, now, cmds);
}

/// The narrow bar's height.
const WINAMP_HEIGHT: f32 = 156.0;

/// The narrow bar's display: the time and the visualization.
fn winamp_display(rect: Rect) -> Rect {
    Rect::from_min_size(
        pos2(rect.left() + MARGIN, rect.top() + 8.0),
        vec2(176.0, 86.0),
    )
}

/// Where the narrow bar's visualization is painted, under the time.
fn winamp_visual_rect(rect: Rect) -> Rect {
    let display = winamp_display(rect);
    Rect::from_min_max(
        pos2(display.left() + 6.0, display.top() + 44.0),
        display.max - vec2(6.0, 6.0),
    )
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
        BarLayout::Winamp => Some(winamp_visual_rect(rect).size()),
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
            .on_hover_text(with_key(tr!("File info", "Сведения о файле"), Cmd::Info))
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
        None => super::no_cover(painter, cover, 3.0),
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
        let resp = ui.interact(r, ui.id().with(("bar", icon)), Sense::click());
        let lit = icon == Icon::Stop && now.stop_after;
        let (fill, color) = if lit {
            (Some(palette::selected_bg()), palette::accent())
        } else if round {
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
        &with_key(tr!("Previous", "Предыдущий"), Cmd::Previous),
        false,
    );
    let (icon, tip) = play_pause(now.status == Status::Playing);
    button(ui, icon, 17.0, BarCmd::PlayPause, &tip, true);
    let stop = with_key(tr!("Stop", "Стоп"), Cmd::Stop);
    let stop_tip = if now.stop_after {
        let after = with_key(
            tr!("stops after this track", "остановка после этого трека"),
            Cmd::StopAfter,
        );
        format!("{stop}; {after}")
    } else {
        stop
    };
    button(ui, Icon::Stop, 14.0, BarCmd::Stop, &stop_tip, false);
    button(
        ui,
        Icon::SkipForward,
        15.0,
        BarCmd::Next,
        &with_key(tr!("Next", "Следующий"), Cmd::Next),
        false,
    );
    x
}

/// Play/Pause's icon and tooltip, by whether it plays.
pub(super) fn play_pause(playing: bool) -> (Icon, String) {
    if playing {
        (Icon::Pause, with_key(tr!("Pause", "Пауза"), Cmd::PlayPause))
    } else {
        (
            Icon::Play,
            with_key(tr!("Play", "Воспроизвести"), Cmd::PlayPause),
        )
    }
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
    mute_button(ui, painter, speaker, now, cmds);
    volume_slider(ui, painter, slider, now, cmds);
    toggles(ui, painter, right, cy, now, cmds)
}

/// The speaker in `speaker`: its waves by the volume, crossed out when
/// muted; a click mutes.
pub(super) fn mute_button(
    ui: &mut Ui,
    painter: &egui::Painter,
    speaker: Rect,
    now: &NowPlaying<'_>,
    cmds: &mut Vec<BarCmd>,
) {
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
        .on_hover_text(with_key(tr!("Mute", "Без звука"), Cmd::Mute))
        .clicked()
    {
        cmds.push(BarCmd::Mute);
    }
}

/// The volume slider in `slider`: a click or a drag sets it (the wheel
/// steps it anywhere over the title row and the bar: `App::ui`).
pub(super) fn volume_slider(
    ui: &mut Ui,
    painter: &egui::Painter,
    slider: Rect,
    now: &NowPlaying<'_>,
    cmds: &mut Vec<BarCmd>,
) {
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
}

/// The equalizer, repeat and shuffle toggles, laid right to left from
/// `right` at height `cy`; where they end.
fn toggles(
    ui: &mut Ui,
    painter: &egui::Painter,
    mut right: f32,
    cy: f32,
    now: &NowPlaying<'_>,
    cmds: &mut Vec<BarCmd>,
) -> f32 {
    // A toggle button from the right; its rect when clicked.
    let mut toggle = |ui: &mut Ui, icon: Icon, on: bool, tip: &str| {
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
        resp.on_hover_text(tip).clicked().then_some(r)
    };
    let tip = if now.eq {
        tr!("Equalizer: on", "Эквалайзер: вкл.")
    } else {
        tr!("Equalizer: off", "Эквалайзер: выкл.")
    };
    if let Some(r) = toggle(ui, Icon::Equalizer, now.eq, tip) {
        cmds.push(BarCmd::Equalizer(r));
    }
    let (icon, tip) = match now.repeat {
        Repeat::Off => (Icon::Repeat, tr!("Repeat: off", "Повтор: выкл.")),
        Repeat::All => (
            Icon::Repeat,
            tr!("Repeat: the playlist", "Повтор: плейлист"),
        ),
        Repeat::One => (Icon::RepeatOne, tr!("Repeat: the track", "Повтор: трек")),
    };
    let tip = with_key(tip, Cmd::Repeat);
    if toggle(ui, icon, now.repeat != Repeat::Off, &tip).is_some() {
        cmds.push(BarCmd::Repeat);
    }
    let tip = if now.shuffle {
        tr!("Shuffle: on", "Перемешать: вкл.")
    } else {
        tr!("Shuffle: off", "Перемешать: выкл.")
    };
    let tip = with_key(tip, Cmd::Shuffle);
    if toggle(ui, Icon::Shuffle, now.shuffle, &tip).is_some() {
        cmds.push(BarCmd::Shuffle);
    }
    right
}

/// The seek bar in `bar`: seeks when let go, the time under the pointer
/// as a tip; the position it shows (the one dragged to while dragging).
pub(super) fn seek_bar(
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
