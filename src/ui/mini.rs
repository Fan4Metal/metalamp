//! The mini player (Ctrl+M, the title row's button; metalmpv's): the
//! window made a square of the cover heard (the app icon on the bar's
//! ground without one), always over the other windows. The title and the
//! artist along the top always, on a band fading into the cover; while the
//! pointer moves there (or rests on them, or nothing plays) the band is
//! whole with the way back to the whole window, and the seek bar,
//! Previous, Play/Pause, Next, the speaker and the volume are along the
//! bottom. The cover moves the window, a double click on it goes
//! back. The window's place and size are `App::enter_mini`'s.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use egui::{Color32, FontId, Rect, Sense, TextureHandle, TextureOptions, Ui, pos2, vec2};

use super::player_bar::{self, BarCmd, NowPlaying};
use crate::keymap::{Cmd, with_key};
use crate::palette;
use crate::player::Status;
use crate::ui::icons::{self, Icon};

/// A running line's pause at either end, in seconds, and its speed, in
/// points a second.
const MARQUEE_PAUSE: f64 = 1.6;
const MARQUEE_SPEED: f64 = 28.0;

/// How far a line `over` points too long has run at `now` (seconds): from
/// the time its text came (kept by `key` in egui's memory), a pause at the
/// start, on to its end, a pause, back, and so on, as Winamp's title. The
/// next frame is asked for: soon while it moves, at the pause's end else.
fn marquee(ctx: &egui::Context, key: &str, text: &str, over: f32, now: f64) -> f32 {
    let id = egui::Id::new(("mini_marquee", key));
    let start = ctx.data_mut(|d| {
        let kept = d.get_temp_mut_or_insert_with(id, || (text.to_owned(), now));
        if kept.0 != text {
            *kept = (text.to_owned(), now);
        }
        kept.1
    });
    let (offset, still) = marquee_at(now - start, over);
    let wait = if still > 0.0 { still } else { 1.0 / 30.0 };
    ctx.request_repaint_after(Duration::from_secs_f64(wait));
    over * offset as f32
}

/// Where a line `over` points too long is `t` seconds after its text
/// came: 0 to 1 of the way to its end, and how long it stays there (0
/// while it moves).
fn marquee_at(t: f64, over: f32) -> (f64, f64) {
    let travel = f64::from(over.max(1.0)) / MARQUEE_SPEED;
    let cycle = 2.0 * (MARQUEE_PAUSE + travel);
    let t = t.rem_euclid(cycle);
    if t < MARQUEE_PAUSE {
        (0.0, MARQUEE_PAUSE - t)
    } else if t < MARQUEE_PAUSE + travel {
        ((t - MARQUEE_PAUSE) / travel, 0.0)
    } else if t < 2.0 * MARQUEE_PAUSE + travel {
        (1.0, 2.0 * MARQUEE_PAUSE + travel - t)
    } else {
        (1.0 - (t - 2.0 * MARQUEE_PAUSE - travel) / travel, 0.0)
    }
}

/// The window's side at first, in points.
pub const SIDE: f32 = 300.0;
/// Its least side, in points.
pub const LEAST: f32 = 200.0;
/// Its distance from the work area's edges at first, in points.
pub const MARGIN: f32 = 16.0;
/// The cover's largest side as read, in pixels: a large window on a
/// scaled display (the library's thumbnails are 256).
const COVER_PX: u32 = 1000;
/// The controls stay this long after the pointer last moved over the
/// cover, in seconds.
const CONTROLS_FOR: f32 = 2.0;
/// The strips over the cover's top and bottom.
const TOP: f32 = 44.0;
const BOTTOM: f32 = 70.0;

/// The cover of the track heard, read whole on a thread of its own.
#[derive(Default)]
pub struct Cover {
    /// The file asked for last.
    path: Option<PathBuf>,
    texture: Option<TextureHandle>,
    rx: Option<Receiver<Option<egui::ColorImage>>>,
}

impl Cover {
    /// The cover of `path` once read; the one before meanwhile (the next
    /// track of an album shows no gap).
    pub fn of(&mut self, ctx: &egui::Context, path: Option<&Path>) -> Option<TextureHandle> {
        if let Some(rx) = &self.rx
            && let Ok(image) = rx.try_recv()
        {
            self.rx = None;
            let options =
                TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
            self.texture = image.map(|i| ctx.load_texture("mini_cover", i, options));
        }
        if self.path.as_deref() != path {
            self.path = path.map(Path::to_path_buf);
            self.rx = None;
            match path {
                None => self.texture = None,
                Some(path) => {
                    let (tx, rx) = channel();
                    let path = path.to_path_buf();
                    let wake = ctx.clone();
                    let spawned =
                        std::thread::Builder::new()
                            .name("mini cover".into())
                            .spawn(move || {
                                let image = crate::guard::catch(&path, || {
                                    crate::covers::large(&path, COVER_PX)
                                })
                                .flatten();
                                let _ = tx.send(image);
                                wake.request_repaint();
                            });
                    if spawned.is_ok() {
                        self.rx = Some(rx);
                    }
                }
            }
        }
        self.texture.clone()
    }

    /// Let go when the mini player ends.
    pub fn clear(&mut self) {
        *self = Cover::default();
    }
}

/// The mini player in the whole of `ui`; `icon` is the app's, for a track
/// without a cover.
pub fn show(ui: &mut Ui, now: &NowPlaying<'_>, icon: &TextureHandle) -> Vec<BarCmd> {
    let mut cmds = Vec::new();
    let ctx = ui.ctx().clone();
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, palette::bar_bg());
    match &now.cover {
        Some(cover) => {
            // A picture not square is cut to its middle square.
            let [w, h] = cover.size().map(|n| n.max(1) as f32);
            let (cut_x, cut_y) = if w > h {
                ((1.0 - h / w) / 2.0, 0.0)
            } else {
                (0.0, (1.0 - w / h) / 2.0)
            };
            let uv = Rect::from_min_max(pos2(cut_x, cut_y), pos2(1.0 - cut_x, 1.0 - cut_y));
            painter.image(cover.id(), rect, uv, Color32::WHITE);
        }
        None => {
            let side = rect.width().min(rect.height()) * 0.42;
            painter.image(
                icon.id(),
                Rect::from_center_size(rect.center(), vec2(side, side)),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }
    // The cover moves the window; a double click: the whole window.
    let cover = crate::frameless::drag_area(ui, rect, ui.id().with("mini_drag"), Sense::click());
    if crate::ui::double_clicked(&cover) {
        cmds.push(BarCmd::Whole);
    }

    let (hover, still, down) = ctx.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.time_since_last_movement(),
            i.pointer.any_down(),
        )
    });
    let playing = now.status == Status::Playing;
    let on_controls =
        hover.is_some_and(|p| p.y <= rect.top() + TOP || p.y >= rect.bottom() - BOTTOM);
    // `METALAMP_MINI_CONTROLS=1`: always shown, for GUI checks.
    static ALWAYS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let always = *ALWAYS.get_or_init(|| std::env::var_os("METALAMP_MINI_CONTROLS").is_some());
    let shown = always
        || egui::Popup::is_any_open(&ctx)
        || (hover.is_some_and(|p| rect.contains(p))
            && (!playing || on_controls || down || still < CONTROLS_FOR));
    if shown && playing && !on_controls && !down {
        ctx.request_repaint_after(Duration::from_secs_f32((CONTROLS_FOR - still).max(0.05)));
    }
    let ground = palette::bar_bg().gamma_multiply_u8(225);

    // The top: the title and the artist always; the way back with the
    // controls. Without them, the band fades into the cover below the
    // names, so the cover is hidden the least.
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP));
    if shown {
        painter.rect_filled(top, 0.0, ground);
        let back =
            Rect::from_center_size(pos2(top.right() - 22.0, top.center().y), vec2(32.0, 30.0));
        let resp = ui.interact(back, ui.id().with("mini_back"), Sense::click());
        if resp.hovered() {
            painter.rect_filled(back, 4.0, palette::hover());
        }
        let color = if resp.hovered() {
            palette::text_bright()
        } else {
            palette::text()
        };
        icons::paint(&painter, Icon::Mini, back.center(), 14.0, color, ground);
        if resp
            .on_hover_text(with_key(tr!("The whole window", "Всё окно"), Cmd::Mini))
            .clicked()
        {
            cmds.push(BarCmd::Whole);
        }
    } else {
        let fade = Rect::from_min_size(rect.min, vec2(rect.width(), TOP + 26.0));
        let mut mesh = egui::Mesh::default();
        let solid = palette::bar_bg().gamma_multiply(0.9);
        let under = palette::bar_bg().gamma_multiply(0.75);
        let clear = Color32::TRANSPARENT;
        mesh.colored_vertex(fade.left_top(), solid);
        mesh.colored_vertex(fade.right_top(), solid);
        mesh.colored_vertex(pos2(fade.right(), top.bottom()), under);
        mesh.colored_vertex(pos2(fade.left(), top.bottom()), under);
        mesh.colored_vertex(fade.right_bottom(), clear);
        mesh.colored_vertex(fade.left_bottom(), clear);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        mesh.add_triangle(3, 2, 4);
        mesh.add_triangle(3, 4, 5);
        painter.add(mesh);
    }
    let (title, artist) = if now.has_track() {
        (now.title, now.artist)
    } else {
        ("MetalAMP", "")
    };
    // Centred, with room kept alike on both sides (the way back's on the
    // right while it is shown); a soft shadow of the ground under each
    // line, so they read over any cover. A line too long even in a smaller
    // font runs (`marquee`).
    let side = if shown { 44.0 } else { 14.0 };
    let room = (top.width() - 2.0 * side).max(20.0);
    let clip = Rect::from_min_max(
        pos2(top.left() + side, top.top()),
        pos2(top.right() - side, top.bottom() + 4.0),
    );
    let now_s = ctx.input(|i| i.time);
    let line = |key: &str, text: &str, size: f32, colour: Color32, y: f32| {
        let shadow = palette::bar_bg().gamma_multiply(0.85);
        let layout = |size: f32, colour: Color32| {
            painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), colour)
        };
        let fits = [size, size * 0.9, size * 0.82]
            .into_iter()
            .find(|&s| layout(s, colour).size().x <= room);
        let (size, offset) = match fits {
            Some(s) => (s, None),
            None => {
                let s = size * 0.82;
                let over = layout(s, colour).size().x - room;
                (s, Some(marquee(&ctx, key, text, over, now_s)))
            }
        };
        let (galley, under) = (layout(size, colour), layout(size, shadow));
        let x = match offset {
            Some(o) => clip.left() - o,
            None => top.center().x - galley.size().x / 2.0,
        };
        let painter = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
        painter.galley(pos2(x + 0.8, y + 1.0), under, shadow);
        painter.galley(pos2(x, y), galley, colour);
    };
    line("title", title, 14.5, palette::text_bright(), top.top() + 4.0);
    if !artist.is_empty() {
        // Over the cover, a shade brighter than over the bar's ground.
        let shade = if shown {
            palette::text_weak()
        } else {
            palette::text()
        };
        line("artist", artist, 12.0, shade, top.top() + 24.0);
    }

    if !shown {
        return cmds;
    }

    // The bottom: the seek bar over the buttons and the volume.
    let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - BOTTOM), rect.max);
    painter.rect_filled(bottom, 0.0, ground);
    let seek = Rect::from_center_size(
        pos2(bottom.center().x, bottom.top() + 14.0),
        vec2(bottom.width() - 28.0, 12.0),
    );
    player_bar::seek_bar(ui, &painter, seek, now, &mut cmds);
    let cy = bottom.bottom() - 24.0;
    let mut x = bottom.left() + 8.0;
    let mut button = |ui: &mut Ui, icon: Icon, size: f32, cmd: BarCmd, tip: &str, round: bool| {
        let d = if round { 36.0 } else { 30.0 };
        let r = Rect::from_center_size(pos2(x + d / 2.0, cy), vec2(d, d));
        x += d + 4.0;
        let resp = ui.interact(r, ui.id().with(("mini", icon)), Sense::click());
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
            &painter,
            icon,
            r.center(),
            size,
            color,
            fill.unwrap_or(ground),
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
    let (play, tip) = player_bar::play_pause(playing);
    button(ui, play, 17.0, BarCmd::PlayPause, &tip, true);
    button(
        ui,
        Icon::SkipForward,
        15.0,
        BarCmd::Next,
        &with_key(tr!("Next", "Следующий"), Cmd::Next),
        false,
    );
    // The volume takes the room left, at most as wide as the bar's.
    let right = bottom.right() - 14.0;
    let width = (right - x - 28.0 - 6.0 - 8.0).clamp(30.0, 100.0);
    let slider = Rect::from_min_max(pos2(right - width, cy - 8.0), pos2(right, cy + 8.0));
    let speaker = Rect::from_center_size(pos2(slider.left() - 6.0 - 14.0, cy), vec2(28.0, 28.0));
    player_bar::mute_button(ui, &painter, speaker, now, &mut cmds);
    player_bar::volume_slider(ui, &painter, slider, now, &mut cmds);
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_line_runs_and_rests() {
        // 56 points too long: 2 s on the way at 28 points a second.
        let over = 56.0;
        assert_eq!(marquee_at(0.0, over), (0.0, MARQUEE_PAUSE));
        let (half, still) = marquee_at(MARQUEE_PAUSE + 1.0, over);
        assert!((half - 0.5).abs() < 1e-9 && still == 0.0);
        let (end, still) = marquee_at(MARQUEE_PAUSE + 2.0 + 0.5, over);
        assert!(end == 1.0 && (still - (MARQUEE_PAUSE - 0.5)).abs() < 1e-9);
        let (back, _) = marquee_at(2.0 * MARQUEE_PAUSE + 2.0 + 1.0, over);
        assert!((back - 0.5).abs() < 1e-9);
        // Round again.
        let cycle = 2.0 * (MARQUEE_PAUSE + 2.0);
        assert_eq!(marquee_at(cycle, over), marquee_at(0.0, over));
    }
}
