//! The title row of the frameless window (metalmpv's `ui/title.rs`): the
//! icon (a click: About, as F1) and the title on the left, a newer
//! release's button, then minimise, maximise and close on the right. Its
//! free area drags the window; a double click maximises it or brings it
//! back. The family's "i" button is left out: the row is bare.

use egui::{Align2, FontId, Rect, Sense, TextureHandle, Ui, pos2, vec2};

use super::icons::{self, Icon};
use crate::keymap::{Cmd, with_key};
use crate::palette;

/// The width of the window's own buttons, as Windows draws them.
const BUTTON: f32 = 46.0;
/// The row's height.
pub const HEIGHT: f32 = 30.0;

/// What a click in the title row asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleCmd {
    Minimize,
    ToggleMaximize,
    Close,
    About,
    /// The player and playlist layout, or the library's (the window
    /// layout's key).
    Layout,
    /// The mini player: the cover and the least of the controls, over
    /// the other windows (Ctrl+M).
    Mini,
}

/// The modes the title row's own buttons show lit.
#[derive(Clone, Copy, Default)]
pub struct Modes {
    /// The player and playlist layout, without the library.
    pub alone: bool,
}

/// `update` is the tag of a newer release, shown as a button left of
/// the window's own.
pub fn row(
    ui: &mut Ui,
    title: &str,
    maximized: bool,
    icon: &TextureHandle,
    update: Option<&str>,
    modes: Modes,
) -> Option<TitleCmd> {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, palette::title_bg());
    // First, under the buttons: what they leave drags the window.
    let drag = crate::frameless::drag_area(ui, rect, ui.id().with("title_drag"), Sense::CLICK);
    let mut cmd = None;
    if super::double_clicked(&drag) {
        cmd = Some(TitleCmd::ToggleMaximize);
    }
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(18.0, 18.0));
    let icon_resp = ui.interact(
        icon_rect.expand(5.0),
        ui.id().with("app_icon"),
        Sense::click(),
    );
    if icon_resp.hovered() {
        ui.painter()
            .rect_filled(icon_rect.expand(5.0), 4.0, palette::hover());
    }
    ui.painter().image(
        icon.id(),
        icon_rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    if icon_resp
        .on_hover_text(with_key(
            &tr!(
                format!("About {}", crate::APP_NAME),
                format!("О программе {}", crate::APP_NAME)
            ),
            Cmd::About,
        ))
        .clicked()
    {
        cmd = Some(TitleCmd::About);
    }

    let buttons = [
        (TitleCmd::Close, Icon::Close, tr!("Close", "Закрыть")),
        (
            TitleCmd::ToggleMaximize,
            if maximized {
                Icon::Restore
            } else {
                Icon::Maximize
            },
            if maximized {
                tr!("Restore down", "Свернуть в окно")
            } else {
                tr!("Maximize", "Развернуть")
            },
        ),
        (
            TitleCmd::Minimize,
            Icon::Minimize,
            tr!("Minimize", "Свернуть"),
        ),
    ];
    let mut right = rect.right();
    for (kind, icon, tip) in buttons {
        let r = Rect::from_min_max(pos2(right - BUTTON, rect.top()), pos2(right, rect.bottom()));
        right = r.left();
        let response = ui.interact(r, ui.id().with(("title_button", tip)), Sense::CLICK);
        let hovered = response.hovered();
        if hovered {
            let fill = if kind == TitleCmd::Close {
                palette::close_hover()
            } else {
                palette::hover()
            };
            ui.painter().rect_filled(r, 0.0, fill);
        }
        let color = if hovered {
            palette::text_bright()
        } else {
            palette::text()
        };
        icons::paint(
            ui.painter(),
            icon,
            r.center(),
            12.0,
            color,
            palette::title_bg(),
        );
        if response.on_hover_text(tip).clicked() {
            cmd = Some(kind);
        }
    }

    // The mini player and the layout without the library, left of the
    // window's own: narrower, lit in the accent while on. ("Over the
    // others" is Ctrl+T's.)
    let toggles = [
        (
            TitleCmd::Mini,
            Icon::Mini,
            false,
            with_key(tr!("Mini player", "Мини-плеер"), Cmd::Mini),
        ),
        (
            TitleCmd::Layout,
            Icon::PlaylistLayout,
            modes.alone,
            with_key(
                if modes.alone {
                    tr!(
                        "Back to the library",
                        "Вернуться к библиотеке"
                    )
                } else {
                    tr!(
                        "Without the library: the player and the playlist",
                        "Без библиотеки: плеер и плейлист"
                    )
                },
                Cmd::WindowLayout,
            ),
        ),
    ];
    for (kind, icon, on, tip) in toggles {
        let r = Rect::from_min_max(pos2(right - 36.0, rect.top()), pos2(right, rect.bottom()));
        right = r.left();
        let response = ui.interact(r, ui.id().with(("title_toggle", icon)), Sense::CLICK);
        if response.hovered() {
            ui.painter().rect_filled(r, 0.0, palette::hover());
        }
        let color = if on {
            palette::accent()
        } else if response.hovered() {
            palette::text_bright()
        } else {
            palette::text_weak()
        };
        icons::paint(
            ui.painter(),
            icon,
            r.center(),
            13.0,
            color,
            palette::title_bg(),
        );
        if response.on_hover_text(tip).clicked() {
            cmd = Some(kind);
        }
    }

    // The update button, if any, before the window's own.
    if let Some(tag) = update {
        let r = Rect::from_min_max(
            pos2(right - 200.0, rect.top()),
            pos2(right - 6.0, rect.bottom()),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(r)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if super::update_button(&mut child, tag).clicked() {
            crate::win::shell_open(crate::update::release_url(tag));
        }
        right = child.min_rect().left() - 8.0;
    }

    // The title, left after the icon and cut to the room before the
    // buttons.
    let left = rect.left() + 34.0;
    let room = right - left;
    if room > 40.0 {
        let galley = ui.fonts_mut(|f| {
            let mut job = egui::text::LayoutJob::simple_singleline(
                title.to_owned(),
                FontId::proportional(12.5),
                palette::text(),
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            f.layout_job(job)
        });
        let pos = Align2::LEFT_CENTER
            .anchor_size(pos2(left, rect.center().y), galley.size())
            .min;
        ui.painter().galley(pos, galley, palette::text());
    }
    cmd
}
