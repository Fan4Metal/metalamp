//! The look: always dark, as Winamp's media library: near-black lists, a
//! slate band for the selection and a light blue for the track playing
//! (Winamp's), the amber of the icon's bolt for the controls (the Play
//! button, the seek bar, the toggles on). Every colour of the interface
//! is here, not scattered as literals.

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
/// The spectrum analyzer's bars: amber at the foot, pale gold at a full
/// bar; the peaks in a warm white.
pub const SPECTRUM_LOW: Color32 = rgb(0xc2, 0x74, 0x12);
pub const SPECTRUM_HIGH: Color32 = rgb(0xff, 0xd6, 0x6e);
pub const SPECTRUM_PEAK: Color32 = rgb(0xe8, 0xe3, 0xd6);

/// A placeholder for a cover not loaded yet.
pub const CARD_BG: Color32 = rgb(0x2a, 0x2d, 0x33);
/// Lines between parts.
pub const LINE: Color32 = rgb(0x30, 0x33, 0x3a);
pub const TEXT: Color32 = rgb(0xd6, 0xd8, 0xdc);
pub const TEXT_WEAK: Color32 = rgb(0x8c, 0x91, 0x9a);
pub const TEXT_BRIGHT: Color32 = rgb(0xff, 0xff, 0xff);
/// A hovered flat button's or row's background.
pub const HOVER: Color32 = rgb(0x25, 0x29, 0x31);
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

/// The accent of the controls: the amber of the bolt in the app icon.
pub const ACCENT: Color32 = rgb(0xf5, 0xa6, 0x23);
pub const ACCENT_HOVER: Color32 = rgb(0xff, 0xbe, 0x4d);
/// The text of the track playing in the lists: a light blue, which sits
/// with the slate band and does not fight the covers as the amber did.
pub const PLAYING: Color32 = rgb(0x8f, 0xc3, 0xff);
/// The background of selected rows: Winamp's slate band.
pub const SELECTED_BG: Color32 = rgb(0x2b, 0x34, 0x42);

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
