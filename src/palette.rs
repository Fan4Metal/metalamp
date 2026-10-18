//! The look: a base theme (`Theme`: the grounds, lines and text) and an
//! accent (`Accent`: the controls, the selection, the track playing),
//! both chosen on the settings' General page and applied at once (`set`,
//! then `apply`). Every colour of the interface comes from here as a
//! function of the palette in use, not as literals scattered about.
//!
//! The default is the look MetalAMP had from the start: the graphite dark
//! theme with the amber of the icon's bolt for the controls, Winamp's
//! slate band for the selection and a light blue for the track playing
//! (the amber there fought the covers). Other accents follow metalmpv's
//! (blue, teal, green, orange, red, pink, purple, graphite), their shades
//! mixed from their colour.

use std::sync::RwLock;

use egui::Color32;
use serde::{Deserialize, Serialize};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const fn hex(c: u32) -> Color32 {
    rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// `a` moved towards `b` by `t`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let c = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
}

/// The relative luminance (sRGB, 0 to 1).
fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// The base of the look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Graphite greys, as Winamp's media library.
    #[default]
    Dark,
    /// Dark with a blue cast.
    Midnight,
    /// Black lists and panels (for OLED screens and dark rooms).
    Black,
    Light,
}

impl Theme {
    pub const ALL: [Theme; 4] = [Theme::Dark, Theme::Midnight, Theme::Black, Theme::Light];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => tr!("Dark", "Тёмная"),
            Theme::Midnight => tr!("Midnight blue", "Тёмно-синяя"),
            Theme::Black => tr!("Black", "Чёрная"),
            Theme::Light => tr!("Light", "Светлая"),
        }
    }

    pub fn is_dark(self) -> bool {
        self != Theme::Light
    }
}

/// The colour of the controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    /// The amber of the icon's bolt.
    #[default]
    Amber,
    Blue,
    Teal,
    Green,
    Orange,
    Red,
    Pink,
    Purple,
    Graphite,
}

impl Accent {
    pub const ALL: [Accent; 9] = [
        Accent::Amber,
        Accent::Blue,
        Accent::Teal,
        Accent::Green,
        Accent::Orange,
        Accent::Red,
        Accent::Pink,
        Accent::Purple,
        Accent::Graphite,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Amber => tr!("Amber", "Янтарный"),
            Accent::Blue => tr!("Blue", "Синий"),
            Accent::Teal => tr!("Teal", "Бирюзовый"),
            Accent::Green => tr!("Green", "Зелёный"),
            Accent::Orange => tr!("Orange", "Оранжевый"),
            Accent::Red => tr!("Red", "Красный"),
            Accent::Pink => tr!("Pink", "Розовый"),
            Accent::Purple => tr!("Purple", "Фиолетовый"),
            Accent::Graphite => tr!("Graphite", "Графитовый"),
        }
    }

    /// The colour itself, as its swatch shows it.
    pub fn base(self) -> Color32 {
        hex(match self {
            Accent::Amber => 0xf5a623,
            Accent::Blue => 0x2f7cf0,
            Accent::Teal => 0x0d9488,
            Accent::Green => 0x16a34a,
            Accent::Orange => 0xea580c,
            Accent::Red => 0xdc2626,
            Accent::Pink => 0xdb2777,
            Accent::Purple => 0x7c3aed,
            Accent::Graphite => 0x6b7280,
        })
    }
}

/// Every colour of the interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// The title row.
    pub title_bg: Color32,
    /// The player bar under the title row.
    pub bar_bg: Color32,
    /// Panels around the lists.
    pub panel_bg: Color32,
    /// The lists and tables themselves.
    pub list_bg: Color32,
    /// Every other row of a table.
    pub list_stripe: Color32,
    /// Column headers.
    pub header_bg: Color32,
    /// A placeholder for a cover not loaded yet; the settings' cards.
    pub card_bg: Color32,
    /// Lines between parts.
    pub line: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub text_bright: Color32,
    /// A hovered flat button's or row's background.
    pub hover: Color32,
    pub pressed: Color32,
    /// The rest of the seek bar and of the volume slider.
    pub track: Color32,
    /// A scroll bar's handle, under the pointer, dragged.
    pub scroll_handle: Color32,
    pub scroll_handle_hover: Color32,
    pub scroll_handle_drag: Color32,
    /// The close button of the title row under the pointer.
    pub close_hover: Color32,
    pub error: Color32,
    /// The controls (the Play button, the seek bar, the toggles on).
    pub accent: Color32,
    pub accent_hover: Color32,
    /// What is drawn on the accent (the Play button's glyph).
    pub on_accent: Color32,
    /// The text of the track playing in the lists.
    pub playing: Color32,
    /// The background of selected rows.
    pub selected_bg: Color32,
    /// The spectrum analyzer's bars at the foot and full, and the peaks.
    pub spectrum_low: Color32,
    pub spectrum_high: Color32,
    pub spectrum_peak: Color32,
}

impl Palette {
    pub fn of(theme: Theme, accent: Accent) -> Palette {
        let mut p = match theme {
            Theme::Dark => Palette {
                dark: true,
                title_bg: hex(0x18191d),
                bar_bg: hex(0x1d1f24),
                panel_bg: hex(0x222429),
                list_bg: hex(0x16171a),
                list_stripe: hex(0x1a1b1f),
                header_bg: hex(0x26282e),
                card_bg: hex(0x2a2d33),
                line: hex(0x30333a),
                text: hex(0xd6d8dc),
                text_weak: hex(0x8c919a),
                text_bright: hex(0xffffff),
                hover: hex(0x252931),
                pressed: hex(0x373b43),
                track: hex(0x3a3e46),
                scroll_handle: hex(0x4c5059),
                scroll_handle_hover: hex(0x666b76),
                scroll_handle_drag: hex(0x808590),
                close_hover: hex(0xc42b1c),
                error: hex(0xff8a80),
                ..Palette::BLANK
            },
            Theme::Midnight => Palette {
                dark: true,
                title_bg: hex(0x11151d),
                bar_bg: hex(0x151a23),
                panel_bg: hex(0x1a2029),
                list_bg: hex(0x0e1218),
                list_stripe: hex(0x12171e),
                header_bg: hex(0x1e2530),
                card_bg: hex(0x232b37),
                line: hex(0x2a3341),
                text: hex(0xd5dbe5),
                text_weak: hex(0x8792a5),
                text_bright: hex(0xffffff),
                hover: hex(0x1f2734),
                pressed: hex(0x2d3747),
                track: hex(0x354052),
                scroll_handle: hex(0x465266),
                scroll_handle_hover: hex(0x5d6a80),
                scroll_handle_drag: hex(0x77849a),
                close_hover: hex(0xc42b1c),
                error: hex(0xff8a80),
                ..Palette::BLANK
            },
            Theme::Black => Palette {
                dark: true,
                title_bg: hex(0x000000),
                bar_bg: hex(0x0a0a0a),
                panel_bg: hex(0x0e0e0e),
                list_bg: hex(0x000000),
                list_stripe: hex(0x080808),
                header_bg: hex(0x141414),
                card_bg: hex(0x1a1a1a),
                line: hex(0x242424),
                text: hex(0xd8d8d8),
                text_weak: hex(0x8a8a8a),
                text_bright: hex(0xffffff),
                hover: hex(0x1a1a1a),
                pressed: hex(0x2a2a2a),
                track: hex(0x303030),
                scroll_handle: hex(0x3c3c3c),
                scroll_handle_hover: hex(0x555555),
                scroll_handle_drag: hex(0x707070),
                close_hover: hex(0xc42b1c),
                error: hex(0xff8a80),
                ..Palette::BLANK
            },
            Theme::Light => Palette {
                dark: false,
                title_bg: hex(0xe6e8ec),
                bar_bg: hex(0xeff1f4),
                panel_bg: hex(0xe9ecf0),
                list_bg: hex(0xffffff),
                list_stripe: hex(0xf5f6f8),
                header_bg: hex(0xe1e4e9),
                card_bg: hex(0xdde1e7),
                line: hex(0xcbd0d8),
                text: hex(0x1f2328),
                text_weak: hex(0x5d6570),
                text_bright: hex(0x000000),
                hover: hex(0xdfe3e9),
                pressed: hex(0xcdd3dc),
                track: hex(0xc3c9d3),
                scroll_handle: hex(0xb0b7c2),
                scroll_handle_hover: hex(0x959daa),
                scroll_handle_drag: hex(0x7b8492),
                close_hover: hex(0xc42b1c),
                error: hex(0xc62828),
                ..Palette::BLANK
            },
        };
        let base = accent.base();
        let (white, black) = (Color32::WHITE, Color32::BLACK);
        if p.dark {
            p.accent = base;
            p.accent_hover = mix(base, white, 0.16);
            p.playing = mix(base, white, 0.45);
            p.selected_bg = mix(p.list_bg, base, 0.26);
            p.spectrum_low = mix(base, black, 0.22);
            p.spectrum_high = mix(base, white, 0.45);
            p.spectrum_peak = mix(p.text, base, 0.1);
            if accent == Accent::Amber {
                // The look the app had from the start: Winamp's slate
                // band and a light blue, which does not fight the covers.
                p.selected_bg = if theme == Theme::Dark {
                    hex(0x2b3442)
                } else {
                    mix(p.list_bg, hex(0x4a6a96), 0.3)
                };
                p.playing = hex(0x8fc3ff);
                p.spectrum_low = hex(0xc27412);
                p.spectrum_high = hex(0xffd66e);
                p.spectrum_peak = hex(0xe8e3d6);
            }
        } else {
            // On white the accent is a shade deeper, to be read.
            let deep = mix(base, black, 0.12);
            p.accent = deep;
            p.accent_hover = mix(deep, black, 0.12);
            p.playing = mix(base, black, 0.3);
            p.selected_bg = mix(white, base, 0.2);
            p.spectrum_low = mix(base, black, 0.1);
            p.spectrum_high = mix(base, white, 0.25);
            p.spectrum_peak = mix(p.text, base, 0.3);
            if accent == Accent::Amber {
                p.playing = hex(0x1f5fb0);
                p.selected_bg = hex(0xd3e1f3);
            }
        }
        p.on_accent = if luminance(p.accent) > 0.32 {
            hex(0x1a1b1f)
        } else {
            white
        };
        p
    }

    const BLANK: Palette = Palette {
        dark: true,
        title_bg: Color32::BLACK,
        bar_bg: Color32::BLACK,
        panel_bg: Color32::BLACK,
        list_bg: Color32::BLACK,
        list_stripe: Color32::BLACK,
        header_bg: Color32::BLACK,
        card_bg: Color32::BLACK,
        line: Color32::BLACK,
        text: Color32::WHITE,
        text_weak: Color32::WHITE,
        text_bright: Color32::WHITE,
        hover: Color32::BLACK,
        pressed: Color32::BLACK,
        track: Color32::BLACK,
        scroll_handle: Color32::BLACK,
        scroll_handle_hover: Color32::BLACK,
        scroll_handle_drag: Color32::BLACK,
        close_hover: Color32::BLACK,
        error: Color32::BLACK,
        accent: Color32::BLACK,
        accent_hover: Color32::BLACK,
        on_accent: Color32::BLACK,
        playing: Color32::BLACK,
        selected_bg: Color32::BLACK,
        spectrum_low: Color32::BLACK,
        spectrum_high: Color32::BLACK,
        spectrum_peak: Color32::BLACK,
    };
}

/// The palette in use.
static CURRENT: RwLock<Option<Palette>> = RwLock::new(None);

/// Use `theme` and `accent` from now on (then `apply` for egui's own).
pub fn set(theme: Theme, accent: Accent) {
    let p = Palette::of(theme, accent);
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(p);
    }
}

/// The palette in use (the default before `set`).
pub fn current() -> Palette {
    CURRENT
        .read()
        .ok()
        .and_then(|p| *p)
        .unwrap_or_else(|| Palette::of(Theme::default(), Accent::default()))
}

macro_rules! colours {
    ($($name:ident),* $(,)?) => {
        $(
            #[inline]
            pub fn $name() -> Color32 {
                current().$name
            }
        )*
    };
}

colours!(
    title_bg,
    bar_bg,
    panel_bg,
    list_bg,
    list_stripe,
    header_bg,
    card_bg,
    line,
    text,
    text_weak,
    text_bright,
    hover,
    pressed,
    track,
    close_hover,
    error,
    accent,
    accent_hover,
    on_accent,
    playing,
    selected_bg,
    spectrum_low,
    spectrum_high,
    spectrum_peak,
);

/// egui's visuals in the palette in use: egui's dark or light visuals as
/// the base, its colours over them.
pub fn apply(ctx: &egui::Context) {
    let p = current();
    let theme = if p.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    ctx.style_mut_of(theme, |s| {
        s.visuals = if p.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let v = &mut s.visuals;
        v.panel_fill = p.panel_bg;
        v.window_fill = p.panel_bg;
        v.extreme_bg_color = p.list_bg;
        v.faint_bg_color = p.list_stripe;
        v.window_stroke = egui::Stroke::new(1.0, p.line);
        v.widgets.noninteractive.fg_stroke.color = p.text;
        v.widgets.noninteractive.bg_stroke.color = p.line;
        v.widgets.inactive.fg_stroke.color = p.text;
        v.widgets.hovered.weak_bg_fill = p.hover;
        v.widgets.active.weak_bg_fill = p.pressed;
        v.selection.bg_fill = p.selected_bg;
        v.selection.stroke.color = p.text_bright;
        v.hyperlink_color = p.accent;
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        // Scroll bars always seen (egui's float in only under the pointer):
        // 6 points wide, as much as is kept for them, 10 under the pointer.
        s.spacing.scroll = egui::style::ScrollStyle {
            floating_width: 6.0,
            ..egui::style::ScrollStyle::thin()
        };
        // Buttons on a light ground: egui's light grey is the ground's.
        if !p.dark {
            v.widgets.inactive.weak_bg_fill = p.pressed;
            v.widgets.hovered.weak_bg_fill = p.track;
            v.widgets.active.weak_bg_fill = p.scroll_handle;
        }
        // The handle's colours (egui's are hardly seen on the lists).
        v.widgets.inactive.bg_fill = p.scroll_handle;
        v.widgets.hovered.bg_fill = p.scroll_handle_hover;
        v.widgets.active.bg_fill = p.scroll_handle_drag;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contrast ratio of two colours (WCAG).
    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn the_default_is_the_look_from_the_start() {
        let p = Palette::of(Theme::Dark, Accent::Amber);
        assert_eq!(p.accent, hex(0xf5a623));
        assert_eq!(p.playing, hex(0x8fc3ff));
        assert_eq!(p.selected_bg, hex(0x2b3442));
        assert_eq!(p.list_bg, hex(0x16171a));
        assert_eq!(p.on_accent, hex(0x1a1b1f));
    }

    #[test]
    fn every_palette_can_be_read() {
        for theme in Theme::ALL {
            for accent in Accent::ALL {
                let p = Palette::of(theme, accent);
                let what = format!("{theme:?} {accent:?}");
                assert!(contrast(p.text, p.list_bg) >= 7.0, "{what}: text");
                assert!(contrast(p.text_weak, p.list_bg) >= 4.0, "{what}: weak text");
                assert!(contrast(p.text, p.selected_bg) >= 4.5, "{what}: selected");
                assert!(contrast(p.playing, p.list_bg) >= 3.0, "{what}: playing");
                assert!(
                    contrast(p.on_accent, p.accent) >= 2.5,
                    "{what}: on the accent"
                );
                assert!(contrast(p.accent, p.bar_bg) >= 1.8, "{what}: the accent");
            }
        }
    }
}
