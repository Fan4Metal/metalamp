//! The mini player (Ctrl+M, the title row's button; metalmpv's): the
//! window made a square of the cover heard (the app icon on the bar's
//! ground without one), always over the other windows. Over the cover
//! while the pointer moves there (or rests on them, or nothing plays): the
//! title, the artist and the way back to the whole window along the top;
//! the seek bar, Previous, Play/Pause, Next, the speaker and the volume
//! along the bottom. The cover moves the window, a double click on it goes
//! back. The window's place and size are `App::enter_mini`'s.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use egui::{Color32, FontId, Rect, Sense, TextureHandle, TextureOptions, Ui, pos2, vec2};

use super::player_bar::{self, BarCmd, NowPlaying};
use crate::palette;
use crate::player::Status;
use crate::ui::icons::{self, Icon};

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
    if !shown {
        return cmds;
    }
    if playing && !on_controls && !down {
        ctx.request_repaint_after(Duration::from_secs_f32((CONTROLS_FOR - still).max(0.05)));
    }
    let ground = palette::bar_bg().gamma_multiply_u8(225);

    // The top: the title and the artist, the way back.
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP));
    painter.rect_filled(top, 0.0, ground);
    let back = Rect::from_center_size(pos2(top.right() - 22.0, top.center().y), vec2(32.0, 30.0));
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
        .on_hover_text(tr!("The whole window (Ctrl+M)", "Всё окно (Ctrl+M)"))
        .clicked()
    {
        cmds.push(BarCmd::Whole);
    }
    let room = back.left() - 6.0 - (top.left() + 12.0);
    let (title, artist) = if now.has_track() {
        (now.title, now.artist)
    } else {
        ("MetalAMP", "")
    };
    let title = crate::ui::elided(
        &painter,
        title.to_owned(),
        FontId::proportional(13.5),
        palette::text_bright(),
        room,
    );
    painter.galley(
        pos2(top.left() + 12.0, top.top() + 6.0),
        title,
        Color32::WHITE,
    );
    if !artist.is_empty() {
        let artist = crate::ui::elided(
            &painter,
            artist.to_owned(),
            FontId::proportional(11.5),
            palette::text_weak(),
            room,
        );
        painter.galley(
            pos2(top.left() + 12.0, top.top() + 25.0),
            artist,
            Color32::WHITE,
        );
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
        tr!("Previous (Ctrl+Left, Z)", "Предыдущий (Ctrl+Влево, Z)"),
        false,
    );
    let (play, tip) = if playing {
        (Icon::Pause, tr!("Pause (Space)", "Пауза (Пробел)"))
    } else {
        (Icon::Play, tr!("Play (Space)", "Воспроизвести (Пробел)"))
    };
    button(ui, play, 17.0, BarCmd::PlayPause, tip, true);
    button(
        ui,
        Icon::SkipForward,
        15.0,
        BarCmd::Next,
        tr!("Next (Ctrl+Right, B)", "Следующий (Ctrl+Вправо, B)"),
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
