//! The look: always dark, as Winamp's media library, with one amber accent
//! (the selection's edge, the playing track, the seek bar, the Play
//! button). Every colour of the interface is here, not scattered as
//! literals.

// Some colours belong to panels still to come.
#![allow(dead_code)]

use egui::Color32;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// The title row.
pub const TITLE_BG: Color32 = rgb(0x18, 0x19, 0x1d);
/// The player bar under the title row.
pub const BAR_BG: Color32 = rgb(0x1d, 0x1f, 0x24);
/// Panels around the lists.
pub const PANEL_BG: Color32 = rgb(0x22, 0x24, 0x29);
/// The lists and tables themselves.
pub const LIST_BG: Color32 = rgb(0x16, 0x17, 0x1a);
/// Every other row of a table.
pub const LIST_STRIPE: Color32 = rgb(0x1a, 0x1b, 0x1f);
/// Column headers.
pub const HEADER_BG: Color32 = rgb(0x26, 0x28, 0x2e);
/// A placeholder for a cover not loaded yet.
pub const CARD_BG: Color32 = rgb(0x2a, 0x2d, 0x33);
/// Lines between parts.
pub const LINE: Color32 = rgb(0x30, 0x33, 0x3a);
pub const TEXT: Color32 = rgb(0xd6, 0xd8, 0xdc);
pub const TEXT_WEAK: Color32 = rgb(0x8c, 0x91, 0x9a);
pub const TEXT_BRIGHT: Color32 = rgb(0xff, 0xff, 0xff);
/// A hovered flat button's or row's background.
pub const HOVER: Color32 = rgb(0x2c, 0x2f, 0x36);
pub const PRESSED: Color32 = rgb(0x37, 0x3b, 0x43);
/// The rest of the seek bar and of the volume slider.
pub const TRACK: Color32 = rgb(0x3a, 0x3e, 0x46);
/// A scroll bar's handle, under the pointer, dragged.
pub const SCROLL_HANDLE: Color32 = rgb(0x4c, 0x50, 0x59);
pub const SCROLL_HANDLE_HOVER: Color32 = rgb(0x66, 0x6b, 0x76);
pub const SCROLL_HANDLE_DRAG: Color32 = rgb(0x80, 0x85, 0x90);
/// The close button of the title row under the pointer.
pub const CLOSE_HOVER: Color32 = rgb(0xc4, 0x2b, 0x1c);
pub const ERROR: Color32 = rgb(0xff, 0x8a, 0x80);

/// The accent: the amber of the bolt in the app icon.
pub const ACCENT: Color32 = rgb(0xf5, 0xa6, 0x23);
pub const ACCENT_HOVER: Color32 = rgb(0xff, 0xbe, 0x4d);
/// The text of the playing track.
pub const PLAYING: Color32 = rgb(0xff, 0xc4, 0x5c);
/// The background of selected rows.
pub const SELECTED_BG: Color32 = rgb(0x4a, 0x3a, 0x1c);

/// egui's visuals in these colours.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        let v = &mut s.visuals;
        v.panel_fill = PANEL_BG;
        v.window_fill = PANEL_BG;
        v.extreme_bg_color = LIST_BG;
        v.faint_bg_color = LIST_STRIPE;
        v.window_stroke = egui::Stroke::new(1.0, LINE);
        v.widgets.noninteractive.fg_stroke.color = TEXT;
        v.widgets.noninteractive.bg_stroke.color = LINE;
        v.widgets.inactive.fg_stroke.color = TEXT;
        v.widgets.hovered.weak_bg_fill = HOVER;
        v.widgets.active.weak_bg_fill = PRESSED;
        v.selection.bg_fill = SELECTED_BG;
        v.selection.stroke.color = TEXT_BRIGHT;
        v.hyperlink_color = ACCENT;
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        // Scroll bars always seen (egui's float in only under the pointer):
        // 6 points wide, as much as is kept for them, 10 under the pointer.
        s.spacing.scroll = egui::style::ScrollStyle {
            floating_width: 6.0,
            ..egui::style::ScrollStyle::thin()
        };
        // The handle's colours (egui's are hardly seen on the lists).
        v.widgets.inactive.bg_fill = SCROLL_HANDLE;
        v.widgets.hovered.bg_fill = SCROLL_HANDLE_HOVER;
        v.widgets.active.bg_fill = SCROLL_HANDLE_DRAG;
    });
}
