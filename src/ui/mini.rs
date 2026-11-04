//! The mini player (Ctrl+M, the title row's button; metalmpv's): the
//! window made a square of the cover heard (the app icon on the bar's
//! ground without one), always over the other windows, 120 points at
//! least. The title and the artist along the top (unless a click on the
//! cover hid them, `Look::names`), on a band fading into the cover; while
//! the pointer moves there (or rests on them, or nothing plays) the band
//! is whole with the way back to the whole window, and the seek bar,
//! Previous, Play/Pause, Next, the speaker and the volume are along the
//! bottom, as many as the width holds. The cover can be a record instead
//! (`Look::vinyl`): the cover as its label, turning while it plays, the
//! position as a ring along its edge, Play/Pause in its middle. The cover
//! moves the window, a double click on it goes back, the right button
//! opens its menu. The window's place and size are `App::enter_mini`'s.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use egui::{Color32, FontId, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui, pos2, vec2};

use super::player_bar::{self, BarCmd, NowPlaying};
use crate::keymap::{Cmd, with_key};
use crate::palette;
use crate::player::Status;
use crate::settings::MiniStyle;
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
pub const LEAST: f32 = 120.0;
/// Its distance from the work area's edges at first, in points.
pub const MARGIN: f32 = 16.0;
/// The cover's largest side as read, in pixels: a large window on a
/// scaled display (the library's thumbnails are 256).
const COVER_PX: u32 = 1000;
/// The controls stay this long after the pointer last moved over the
/// cover, in seconds.
const CONTROLS_FOR: f32 = 2.0;
/// The band over the cover's top with both names, with the title alone
/// (a window under `TWO_NAMES` high).
const TOP: f32 = 44.0;
const TOP_ONE: f32 = 26.0;
const TWO_NAMES: f32 = 170.0;
/// The band along the bottom, and in a window under `NARROW` wide.
const BOTTOM: f32 = 70.0;
const BOTTOM_NARROW: f32 = 56.0;
/// The widths from which the bottom band holds the volume, and Previous
/// and Next beside Play/Pause.
const WITH_VOLUME: f32 = 190.0;
const NARROW: f32 = 150.0;

/// The record: its radius in the half side (the rest for its shadow on
/// what is behind the window), its label's in its own, seen as it is and
/// closer; the speed of 33⅓ turns a minute, in radians a second, reached
/// or lost in about `SPIN_EASE` seconds.
const DISC: f32 = 0.955;
const LABEL: f32 = 0.42;
const LABEL_CLOSE: f32 = 0.66;
const TURNS: f32 = 33.333 / 60.0 * TAU;
const SPIN_EASE: f32 = 0.35;

/// What the mini player shows: the names, the cover as it is or a record.
#[derive(Clone, Copy)]
pub struct Look {
    pub names: bool,
    pub style: MiniStyle,
}

/// How far the record has turned and how fast it turns: it gathers speed
/// when the music starts and runs down when it stops.
#[derive(Default)]
pub struct Spin {
    angle: f32,
    speed: f32,
    last: Option<f64>,
}

impl Spin {
    /// The angle at `now` (seconds), turning towards full speed while
    /// `playing`, towards rest else; whether it still moves.
    fn turn(&mut self, now: f64, playing: bool) -> (f32, bool) {
        let dt = self.last.map_or(0.0, |l| (now - l).clamp(0.0, 0.1) as f32);
        self.last = Some(now);
        let target = if playing { TURNS } else { 0.0 };
        self.speed += (target - self.speed) * (1.0 - (-dt / SPIN_EASE).exp());
        if !playing && self.speed < 0.02 {
            self.speed = 0.0;
        }
        self.angle = (self.angle + self.speed * dt).rem_euclid(TAU);
        (self.angle, self.speed > 0.0)
    }
}

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

/// The middle square of a picture `size` big, in texture coordinates.
fn middle_square(size: [usize; 2]) -> Rect {
    let [w, h] = size.map(|n| n.max(1) as f32);
    let (cut_x, cut_y) = if w > h {
        ((1.0 - h / w) / 2.0, 0.0)
    } else {
        (0.0, (1.0 - w / h) / 2.0)
    };
    Rect::from_min_max(pos2(cut_x, cut_y), pos2(1.0 - cut_x, 1.0 - cut_y))
}

/// The mini player in the whole of `ui`; `icon` is the app's, for a track
/// without a cover.
pub fn show(
    ui: &mut Ui,
    now: &NowPlaying<'_>,
    icon: &TextureHandle,
    look: Look,
    spin: &mut Spin,
) -> Vec<BarCmd> {
    let mut cmds = Vec::new();
    let ctx = ui.ctx().clone();
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    let playing = now.status == Status::Playing;
    let now_s = ctx.input(|i| i.time);
    // A record on nothing: the window is clear round it (`App::clear_color`
    // and `win::set_round`).
    let disc = look.style.is_record().then(|| {
        let (angle, moving) = spin.turn(now_s, playing);
        if moving {
            ctx.request_repaint();
        }
        let record = Record::of(rect, look.style);
        record.paint(&painter, now, icon, angle);
        record
    });
    if disc.is_none() {
        painter.rect_filled(rect, 0.0, palette::bar_bg());
        // Turned on again, the record starts from rest.
        *spin = Spin::default();
        match &now.cover {
            Some(cover) => {
                painter.image(
                    cover.id(),
                    rect,
                    middle_square(cover.size()),
                    Color32::WHITE,
                );
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
    }
    // The cover moves the window; a click shows or hides the names (once
    // no second click can follow: a double click goes back to the whole
    // window); the right button opens the menu.
    let cover = crate::frameless::drag_area(ui, rect, ui.id().with("mini_drag"), Sense::click());
    names_on_click(&ctx, &cover, look, now_s, &mut cmds);
    cover.context_menu(|ui| menu(ui, look, &mut cmds));

    let (hover, still, down) = ctx.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.time_since_last_movement(),
            i.pointer.any_down(),
        )
    });
    let top_h = if rect.height() >= TWO_NAMES {
        TOP
    } else {
        TOP_ONE
    };
    let bottom_h = if rect.width() >= NARROW {
        BOTTOM
    } else {
        BOTTOM_NARROW
    };
    let on_controls = hover.is_some_and(|p| match &disc {
        Some(d) => d.on_controls(p),
        None => p.y <= rect.top() + top_h || p.y >= rect.bottom() - bottom_h,
    });
    let over = hover.is_some_and(|p| match &disc {
        Some(d) => p.distance(d.centre) <= d.radius,
        None => rect.contains(p),
    });
    // `METALAMP_MINI_CONTROLS=1`: always shown, for GUI checks.
    static ALWAYS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let always = *ALWAYS.get_or_init(|| std::env::var_os("METALAMP_MINI_CONTROLS").is_some());
    let shown = always
        || egui::Popup::is_any_open(&ctx)
        || (over && (!playing || on_controls || down || still < CONTROLS_FOR));
    if shown && playing && !on_controls && !down {
        ctx.request_repaint_after(Duration::from_secs_f32((CONTROLS_FOR - still).max(0.05)));
    }
    let ground = palette::bar_bg().gamma_multiply_u8(225);

    if let Some(d) = &disc {
        if look.names {
            d.names(&ctx, &painter, now, shown, now_s);
        }
        if shown {
            d.controls(ui, &painter, now, &mut cmds);
        }
        return cmds;
    }
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), top_h));
    if look.names {
        names(ui, &painter, top, now, shown, now_s, ground);
    }
    if shown {
        back_button(ui, &painter, top, look.names, ground, &mut cmds);
        let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - bottom_h), rect.max);
        bottom_band(ui, &painter, bottom, now, ground, &mut cmds);
    }
    cmds
}

/// A click on the cover: the names shown or hidden once the double-click
/// time has passed without a second click.
fn names_on_click(
    ctx: &egui::Context,
    cover: &egui::Response,
    look: Look,
    now: f64,
    cmds: &mut Vec<BarCmd>,
) {
    let id = egui::Id::new("mini_click");
    if crate::ui::double_clicked(cover) {
        ctx.data_mut(|d| d.remove::<f64>(id));
        cmds.push(BarCmd::Whole);
        return;
    }
    if cover.clicked() {
        ctx.data_mut(|d| d.insert_temp(id, now));
    }
    if let Some(at) = ctx.data(|d| d.get_temp::<f64>(id)) {
        let wait = crate::win::double_click_time() - (now - at);
        if wait <= 0.0 {
            ctx.data_mut(|d| d.remove::<f64>(id));
            cmds.push(BarCmd::MiniNames(!look.names));
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(wait));
        }
    }
}

/// The right button's menu: the names, the record, the whole window.
fn menu(ui: &mut Ui, look: Look, cmds: &mut Vec<BarCmd>) {
    let mut names = look.names;
    if ui.checkbox(&mut names, tr!("Names", "Названия")).changed() {
        cmds.push(BarCmd::MiniNames(names));
    }
    ui.separator();
    for (style, label) in [
        (MiniStyle::Cover, tr!("Cover", "Обложка")),
        (MiniStyle::Record, tr!("Record", "Пластинка")),
        (
            MiniStyle::CloseUp,
            tr!("Record, closer", "Пластинка крупно"),
        ),
    ] {
        if ui.radio(look.style == style, label).clicked() && look.style != style {
            cmds.push(BarCmd::MiniStyle(style));
        }
    }
    ui.separator();
    if ui
        .button(with_key(tr!("Whole window", "Всё окно"), Cmd::Mini))
        .clicked()
    {
        cmds.push(BarCmd::Whole);
        ui.close();
    }
}

/// The title and the artist (the title alone in a band `TOP_ONE` high),
/// centred along `top`: on the band's ground while the controls are
/// `shown`, else on a band fading into the cover.
fn names(
    ui: &Ui,
    painter: &egui::Painter,
    top: Rect,
    now: &NowPlaying<'_>,
    shown: bool,
    now_s: f64,
    ground: Color32,
) {
    let ctx = ui.ctx();
    let two = top.height() >= TOP;
    if shown {
        painter.rect_filled(top, 0.0, ground);
    } else {
        let fade = Rect::from_min_size(top.min, vec2(top.width(), top.height() + 26.0));
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
    // right while it is shown; in a narrow window only there); a soft
    // shadow of the ground under each line, so they read over any cover. A
    // line too long even in a smaller font runs (`marquee`).
    let (left, right) = if !shown {
        (14.0, 14.0)
    } else if top.width() < NARROW {
        (8.0, 40.0)
    } else {
        (44.0, 44.0)
    };
    let clip = Rect::from_min_max(
        pos2(top.left() + left, top.top()),
        pos2(top.right() - right, top.bottom() + 4.0),
    );
    let shadow = palette::bar_bg().gamma_multiply(0.85);
    let line = |key: &str, text: &str, size: f32, colour: Color32, y: f32| {
        name_line(
            ctx, painter, key, text, size, colour, shadow, clip, y, now_s,
        );
    };
    line(
        "title",
        title,
        14.5,
        palette::text_bright(),
        top.top() + 4.0,
    );
    if two && !artist.is_empty() {
        // Over the cover, a shade brighter than over the bar's ground.
        let shade = if shown {
            palette::text_weak()
        } else {
            palette::text()
        };
        line("artist", artist, 12.0, shade, top.top() + 24.0);
    }
}

/// A line of the names with its top at `y`, centred in `clip`'s width, a
/// soft `shadow` under it, so it reads over any cover; in a smaller font
/// when it does not fit, and running (`marquee`) when it does not even
/// then.
#[allow(clippy::too_many_arguments)]
fn name_line(
    ctx: &egui::Context,
    painter: &egui::Painter,
    key: &str,
    text: &str,
    size: f32,
    colour: Color32,
    shadow: Color32,
    clip: Rect,
    y: f32,
    now_s: f64,
) {
    let room = clip.width().max(20.0);
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
            (s, Some(marquee(ctx, key, text, over, now_s)))
        }
    };
    let (galley, under) = (layout(size, colour), layout(size, shadow));
    let x = match offset {
        Some(o) => clip.left() - o,
        None => clip.center().x - galley.size().x / 2.0,
    };
    let painter = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
    painter.galley(pos2(x + 0.8, y + 1.0), under, shadow);
    painter.galley(pos2(x, y), galley, colour);
}

/// The way back to the whole window at the right of `top`; without the
/// names' band, on a ground of its own.
fn back_button(
    ui: &mut Ui,
    painter: &egui::Painter,
    top: Rect,
    band: bool,
    ground: Color32,
    cmds: &mut Vec<BarCmd>,
) {
    let back = Rect::from_center_size(pos2(top.right() - 22.0, top.top() + 22.0), vec2(32.0, 30.0));
    let back = if band {
        back.translate(vec2(0.0, top.center().y - back.center().y))
    } else {
        back.translate(vec2(4.0, -4.0))
    };
    if !band {
        painter.rect_filled(back, 6.0, ground);
    }
    let resp = ui.interact(back, ui.id().with("mini_back"), Sense::click());
    if resp.hovered() {
        painter.rect_filled(back, 4.0, palette::hover());
    }
    let color = if resp.hovered() {
        palette::text_bright()
    } else {
        palette::text()
    };
    icons::paint(painter, Icon::Mini, back.center(), 14.0, color, ground);
    if resp
        .on_hover_text(with_key(tr!("The whole window", "Всё окно"), Cmd::Mini))
        .clicked()
    {
        cmds.push(BarCmd::Whole);
    }
}

/// A round button: the accent's for Play/Pause (`round`), else on the
/// `ground` given (none: the band's).
#[allow(clippy::too_many_arguments)]
fn button(
    ui: &mut Ui,
    painter: &egui::Painter,
    centre: Pos2,
    d: f32,
    icon: Icon,
    size: f32,
    round: bool,
    ground: Option<Color32>,
    tip: &str,
) -> bool {
    let r = Rect::from_center_size(centre, vec2(d, d));
    let resp = ui.interact(r, ui.id().with(("mini", icon)), Sense::click());
    if !round && let Some(g) = ground {
        painter.circle_filled(centre, d / 2.0, g);
    }
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
        painter.circle_filled(centre, d / 2.0, fill);
    }
    let behind = fill.or(ground).unwrap_or_else(palette::bar_bg);
    icons::paint(painter, icon, centre, size, color, behind);
    resp.on_hover_text(tip).clicked()
}

/// The bottom: the seek bar over the buttons and, as the width allows,
/// Previous and Next, the speaker and the volume.
fn bottom_band(
    ui: &mut Ui,
    painter: &egui::Painter,
    bottom: Rect,
    now: &NowPlaying<'_>,
    ground: Color32,
    cmds: &mut Vec<BarCmd>,
) {
    let playing = now.status == Status::Playing;
    painter.rect_filled(bottom, 0.0, ground);
    let narrow = bottom.width() < NARROW;
    let seek = Rect::from_center_size(
        pos2(
            bottom.center().x,
            bottom.top() + if narrow { 12.0 } else { 14.0 },
        ),
        vec2(bottom.width() - if narrow { 20.0 } else { 28.0 }, 12.0),
    );
    player_bar::seek_bar(ui, painter, seek, now, cmds);
    let cy = bottom.bottom() - if narrow { 21.0 } else { 24.0 };
    let (play, play_tip) = player_bar::play_pause(playing);
    let previous = with_key(tr!("Previous", "Предыдущий"), Cmd::Previous);
    let next = with_key(tr!("Next", "Следующий"), Cmd::Next);
    if narrow {
        let c = pos2(bottom.center().x, cy);
        if button(ui, painter, c, 32.0, play, 16.0, true, None, &play_tip) {
            cmds.push(BarCmd::PlayPause);
        }
        return;
    }
    // Previous, Play/Pause and Next: from the left beside the volume,
    // else in the middle.
    let with_volume = bottom.width() >= WITH_VOLUME;
    let row = 30.0 + 4.0 + 36.0 + 4.0 + 30.0;
    let mut x = if with_volume {
        bottom.left() + 8.0
    } else {
        bottom.center().x - row / 2.0
    };
    let mut place = |d: f32| {
        let c = pos2(x + d / 2.0, cy);
        x += d + 4.0;
        c
    };
    let at = place(30.0);
    if button(
        ui,
        painter,
        at,
        30.0,
        Icon::SkipBack,
        15.0,
        false,
        None,
        &previous,
    ) {
        cmds.push(BarCmd::Previous);
    }
    let at = place(36.0);
    if button(ui, painter, at, 36.0, play, 17.0, true, None, &play_tip) {
        cmds.push(BarCmd::PlayPause);
    }
    let at = place(30.0);
    if button(
        ui,
        painter,
        at,
        30.0,
        Icon::SkipForward,
        15.0,
        false,
        None,
        &next,
    ) {
        cmds.push(BarCmd::Next);
    }
    if !with_volume {
        return;
    }
    // The volume takes the room left, at most as wide as the bar's.
    let right = bottom.right() - 14.0;
    let width = (right - x - 28.0 - 6.0 - 8.0).clamp(30.0, 100.0);
    let slider = Rect::from_min_max(pos2(right - width, cy - 8.0), pos2(right, cy + 8.0));
    let speaker = Rect::from_center_size(pos2(slider.left() - 6.0 - 14.0, cy), vec2(28.0, 28.0));
    player_bar::mute_button(ui, painter, speaker, now, cmds);
    player_bar::volume_slider(ui, painter, slider, now, cmds);
}

/// The record in the window: where it is and how large.
struct Record {
    centre: Pos2,
    /// The disc's radius, its label's, and the spindle hole's.
    radius: f32,
    label: f32,
    hole: f32,
}

impl Record {
    fn of(rect: Rect, style: MiniStyle) -> Record {
        let radius = rect.width().min(rect.height()) / 2.0 * DISC;
        let label = radius
            * if style == MiniStyle::CloseUp {
                LABEL_CLOSE
            } else {
                LABEL
            };
        Record {
            centre: rect.center(),
            radius,
            label,
            hole: (radius * 0.03).max(2.0),
        }
    }

    /// The middle of the band of grooves, from the centre: where the names
    /// and the buttons on the vinyl go.
    fn band(&self) -> f32 {
        (self.label + self.radius) / 2.0
    }

    /// The width of the disc at `d` from its centre across, less a margin.
    fn chord(&self, d: f32) -> f32 {
        2.0 * (self.radius * self.radius - d * d).max(0.0).sqrt() - 12.0
    }

    /// Whether the pointer at `p` rests on a control: the label (Play/Pause)
    /// or a button on the vinyl.
    fn on_controls(&self, p: Pos2) -> bool {
        let band = self.band();
        p.distance(self.centre) <= self.label
            || [vec2(-band, 0.0), vec2(band, 0.0), vec2(0.0, band)]
                .iter()
                .any(|&o| p.distance(self.centre + o) <= 20.0)
    }

    /// The disc on nothing (a soft shadow round it), its grooves and sheen,
    /// the label turned by `angle` with the spindle's hole through both,
    /// the position as a ring along the edge.
    fn paint(
        &self,
        painter: &egui::Painter,
        now: &NowPlaying<'_>,
        icon: &TextureHandle,
        angle: f32,
    ) {
        let (c, r) = (self.centre, self.radius);
        // The shadow on what is behind the window, a little lower, fading
        // out within the margin round the disc.
        let margin = (r / DISC - r).max(1.0);
        let steps = margin.ceil() as usize;
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let alpha = (60.0 * (1.0 - t) * (1.0 - t)) as u8;
            painter.circle_stroke(
                c + vec2(0.0, 1.0),
                r + i as f32 + 0.5,
                egui::Stroke::new(1.0, Color32::from_black_alpha(alpha)),
            );
        }
        // The vinyl: a ring from the hole to the edge (a stroke that wide
        // left a gap in its middle), its outer edge smoothed by a thin
        // stroke over it.
        let vinyl = Color32::from_rgb(16, 16, 18);
        painter.add(ring(c, self.hole, r - 0.5, vinyl));
        painter.circle_stroke(c, r - 0.75, egui::Stroke::new(1.5, vinyl));
        // Grooves: fine rings a little lighter, a wider gap now and then
        // between "tracks".
        let groove = Color32::from_white_alpha(9);
        let mut g = self.label * 1.06 + 1.0;
        let mut n = 0;
        while g < r * 0.97 {
            painter.circle_stroke(c, g, egui::Stroke::new(0.6, groove));
            n += 1;
            g += if n % 9 == 0 { 4.5 } else { 2.2 };
        }
        // The light on it: two faint wedges across, which stay put while
        // the disc turns.
        for middle in [-0.25 * std::f32::consts::PI, 0.75 * std::f32::consts::PI] {
            painter.add(sheen(c, self.label * 1.04, r * 0.98, middle, 0.42, 30));
        }
        painter.circle_stroke(
            c,
            r - 0.5,
            egui::Stroke::new(1.0, Color32::from_white_alpha(18)),
        );

        // The label: the cover, else the app icon on the card's ground,
        // with the hole left open.
        let label = self.label;
        match &now.cover {
            Some(cover) => {
                painter.add(turned_disc(
                    cover.id(),
                    c,
                    self.hole,
                    label,
                    middle_square(cover.size()),
                    angle,
                ));
            }
            None => {
                painter.add(ring(c, self.hole, label, palette::card_bg()));
                let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.add(turned_square(icon.id(), c, label * 0.62, full, angle));
                // The icon's middle over the hole: opened again.
                painter.circle_filled(c, self.hole, palette::card_bg());
            }
        }
        // The label's edge and the hole's rim, smoothing the mesh's edges.
        painter.circle_stroke(
            c,
            label,
            egui::Stroke::new(1.5, Color32::from_black_alpha(110)),
        );
        painter.circle_stroke(
            c,
            self.hole + 0.4,
            egui::Stroke::new(1.2, Color32::from_black_alpha(150)),
        );

        // The position along the edge, from the top clockwise.
        if now.status != Status::Stopped && now.duration_ms > 0 {
            let part = (now.position_ms as f32 / now.duration_ms as f32).clamp(0.0, 1.0);
            let ring = r - 2.5;
            painter.circle_stroke(
                c,
                ring,
                egui::Stroke::new(2.0, Color32::from_white_alpha(14)),
            );
            if part > 0.0 {
                let steps = ((part * 160.0).ceil() as usize).max(2);
                let points: Vec<Pos2> = (0..=steps)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2 + part * TAU * i as f32 / steps as f32;
                        c + ring * vec2(a.cos(), a.sin())
                    })
                    .collect();
                painter.add(egui::Shape::line(
                    points,
                    egui::Stroke::new(2.0, palette::accent()),
                ));
            }
        }
    }

    /// The title on the vinyl over the label, the artist under it (where
    /// the way back is while the controls are `shown`), light over the
    /// black whatever the theme, each as wide as the disc is there.
    fn names(
        &self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        now: &NowPlaying<'_>,
        shown: bool,
        now_s: f64,
    ) {
        let (title, artist) = if now.has_track() {
            (now.title, now.artist)
        } else {
            ("MetalAMP", "")
        };
        let band = self.band();
        let thick = self.radius - self.label;
        let shadow = Color32::from_black_alpha(200);
        let line = |key: &str, text: &str, size: f32, colour: Color32, middle: f32| {
            let height = size * 1.3;
            let far = (middle - self.centre.y).abs() + height / 2.0;
            let width = self.chord(far).max(20.0);
            let clip =
                Rect::from_center_size(pos2(self.centre.x, middle), vec2(width, height + 4.0));
            name_line(
                ctx,
                painter,
                key,
                text,
                size,
                colour,
                shadow,
                clip,
                middle - height / 2.0,
                now_s,
            );
        };
        let title_size = (thick * 0.62).min(14.5);
        line(
            "title",
            title,
            title_size,
            Color32::from_gray(235),
            self.centre.y - band,
        );
        if !shown && !artist.is_empty() {
            let size = (thick * 0.52).min(12.0);
            line(
                "artist",
                artist,
                size,
                Color32::from_gray(175),
                self.centre.y + band,
            );
        }
    }

    /// Play/Pause in the label's middle; the way back on the vinyl under
    /// it; Previous and Next on the vinyl either side when the window is
    /// wide enough.
    fn controls(
        &self,
        ui: &mut Ui,
        painter: &egui::Painter,
        now: &NowPlaying<'_>,
        cmds: &mut Vec<BarCmd>,
    ) {
        let playing = now.status == Status::Playing;
        let (play, tip) = player_bar::play_pause(playing);
        let d = (self.label * 0.95).clamp(30.0, 56.0);
        if button(
            ui,
            painter,
            self.centre,
            d,
            play,
            d * 0.45,
            true,
            None,
            &tip,
        ) {
            cmds.push(BarCmd::PlayPause);
        }
        let ground = Some(Color32::from_rgba_unmultiplied(40, 40, 44, 220));
        let band = self.band();
        let side = (self.radius - self.label - 4.0).clamp(22.0, 34.0);
        let back = with_key(tr!("The whole window", "Всё окно"), Cmd::Mini);
        let at = self.centre + vec2(0.0, band);
        if button(
            ui,
            painter,
            at,
            side,
            Icon::Mini,
            13.0,
            false,
            ground,
            &back,
        ) {
            cmds.push(BarCmd::Whole);
        }
        if self.radius * 2.0 < NARROW {
            return;
        }
        let previous = with_key(tr!("Previous", "Предыдущий"), Cmd::Previous);
        let next = with_key(tr!("Next", "Следующий"), Cmd::Next);
        let at = self.centre - vec2(band, 0.0);
        if button(
            ui,
            painter,
            at,
            side,
            Icon::SkipBack,
            14.0,
            false,
            ground,
            &previous,
        ) {
            cmds.push(BarCmd::Previous);
        }
        let at = self.centre + vec2(band, 0.0);
        if button(
            ui,
            painter,
            at,
            side,
            Icon::SkipForward,
            14.0,
            false,
            ground,
            &next,
        ) {
            cmds.push(BarCmd::Next);
        }
    }
}

/// A wedge of light across the ring from `inner` to `outer`, `width`
/// radians wide about `middle`, brightest (`peak` of white) in its middle.
fn sheen(c: Pos2, inner: f32, outer: f32, middle: f32, width: f32, peak: u8) -> egui::Mesh {
    const STEPS: usize = 24;
    let mut mesh = egui::Mesh::default();
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let a = middle + (t - 0.5) * width;
        // Strongest in the middle, nothing at the sides.
        let light = (std::f32::consts::PI * t).sin().powi(2);
        let colour = Color32::from_white_alpha((f32::from(peak) * light) as u8);
        let dir = vec2(a.cos(), a.sin());
        mesh.colored_vertex(c + inner * dir, colour);
        mesh.colored_vertex(c + outer * dir, colour);
        if i > 0 {
            let k = (2 * i) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    mesh
}

/// A ring of `colour` at `c` from `inner` to `outer`, its edges not
/// smoothed (strokes over them do it).
fn ring(c: Pos2, inner: f32, outer: f32, colour: Color32) -> egui::Mesh {
    const STEPS: u32 = 128;
    let mut mesh = egui::Mesh::default();
    for i in 0..=STEPS {
        let a = TAU * i as f32 / STEPS as f32;
        let dir = vec2(a.cos(), a.sin());
        mesh.colored_vertex(c + inner * dir, colour);
        mesh.colored_vertex(c + outer * dir, colour);
        if i > 0 {
            let k = 2 * i;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    mesh
}

/// A round picture with a hole: the circle in `uv` drawn at `c` with
/// radius `r`, less the middle within `hole`, turned by `angle`
/// (clockwise on screen).
fn turned_disc(
    texture: egui::TextureId,
    c: Pos2,
    hole: f32,
    r: f32,
    uv: Rect,
    angle: f32,
) -> egui::Mesh {
    const STEPS: u32 = 96;
    let mut mesh = egui::Mesh::with_texture(texture);
    let half = uv.size() / 2.0;
    for i in 0..=STEPS {
        let a = TAU * i as f32 / STEPS as f32;
        // The picture's point that shows at the angle `a` once turned.
        let b = a - angle;
        let (dir, turned) = (
            vec2(a.cos(), a.sin()),
            vec2(half.x * b.cos(), half.y * b.sin()),
        );
        for radius in [hole, r] {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: c + radius * dir,
                uv: uv.center() + turned * (radius / r),
                color: Color32::WHITE,
            });
        }
        if i > 0 {
            let k = 2 * i;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    mesh
}

/// A square picture `half` from its middle at `c`, turned by `angle`.
fn turned_square(texture: egui::TextureId, c: Pos2, half: f32, uv: Rect, angle: f32) -> egui::Mesh {
    let mut mesh = egui::Mesh::with_texture(texture);
    let (sin, cos) = angle.sin_cos();
    let corners = [
        (vec2(-half, -half), uv.left_top()),
        (vec2(half, -half), uv.right_top()),
        (vec2(half, half), uv.right_bottom()),
        (vec2(-half, half), uv.left_bottom()),
    ];
    for (d, uv) in corners {
        let turned = vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
        mesh.vertices.push(egui::epaint::Vertex {
            pos: c + turned,
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
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

    #[test]
    fn the_record_gathers_speed_and_runs_down() {
        let mut spin = Spin::default();
        let mut t = 0.0;
        // Playing for 2 s at 60 frames a second: full speed.
        for _ in 0..120 {
            t += 1.0 / 60.0;
            spin.turn(t, true);
        }
        assert!((spin.speed - TURNS).abs() < 0.01 * TURNS);
        // Stopped: at rest within 2 s, and it says so.
        let mut moving = true;
        for _ in 0..120 {
            t += 1.0 / 60.0;
            moving = spin.turn(t, false).1;
        }
        assert!(!moving && spin.speed == 0.0);
        // A frame late by seconds turns it at most a tenth of a second's.
        let before = spin.turn(t, true).0;
        let after = spin.turn(t + 5.0, true).0;
        assert!((after - before).rem_euclid(TAU) <= TURNS * 0.1 + 1e-4);
    }

    #[test]
    fn a_picture_not_square_is_cut_to_its_middle() {
        let wide = middle_square([200, 100]);
        assert!((wide.left() - 0.25).abs() < 1e-6 && (wide.right() - 0.75).abs() < 1e-6);
        assert_eq!((wide.top(), wide.bottom()), (0.0, 1.0));
        let tall = middle_square([100, 400]);
        assert!((tall.top() - 0.375).abs() < 1e-6 && (tall.bottom() - 0.625).abs() < 1e-6);
        // A size of 0 read from a file does not divide by it.
        assert!(middle_square([0, 0]).is_finite());
    }
}
