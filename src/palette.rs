//! The look: a base theme (`Theme`: the grounds, lines and text) and an
//! accent (`Accent`: the controls, the selection, the track playing),
//! both chosen on the settings' General page and applied at once (`set`,
//! then `apply`). Every colour of the interface comes from here as a
//! function of the palette in use, not as literals scattered about.
//!
//! The default is the graphite dark theme with the amber of the icon's
//! bolt; the other accents are metalmpv's (blue, teal, green, orange,
//! red, pink, purple, graphite). Every accent's shades (the hover, the
//! selection's ground, the track playing, the spectrum) are mixed from its
//! colour alike, so the look is of one hue (the slate selection and light
//! blue track amber had at first looked out of place, the user found).

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

/// The contrast ratio of two colours (WCAG).
fn contrast(a: Color32, b: Color32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
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

/// What the palette is made of: the settings `theme`, `accent`,
/// `selection` and `playing`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub theme: Theme,
    pub accent: Accent,
    pub selection: Tint,
    pub playing: Tint,
}

/// The base of the look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Black lists and panels (for OLED screens and dark rooms).
    Black,
    /// Dark with a blue cast.
    Midnight,
    /// Graphite greys.
    #[default]
    Dark,
    /// Winamp's Modern skin: black lists, steel-blue text, a steel frame.
    Steel,
    /// Softer greys with a cool cast, lighter than graphite.
    Dimmed,
    /// Warm paper.
    Sepia,
    Light,
}

impl Theme {
    /// From the darkest to the lightest, as the list shows them (a test
    /// holds the order).
    pub const ALL: [Theme; 7] = [
        Theme::Black,
        Theme::Midnight,
        Theme::Dark,
        Theme::Steel,
        Theme::Dimmed,
        Theme::Sepia,
        Theme::Light,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Black => tr!("Black", "Чёрная"),
            Theme::Midnight => tr!("Midnight blue", "Тёмно-синяя"),
            Theme::Dark => tr!("Dark", "Тёмная"),
            Theme::Steel => tr!("Steel", "Стальная"),
            Theme::Dimmed => tr!("Dimmed", "Приглушённая"),
            Theme::Sepia => tr!("Sepia", "Сепия"),
            Theme::Light => tr!("Light", "Светлая"),
        }
    }

    pub fn is_dark(self) -> bool {
        !matches!(self, Theme::Sepia | Theme::Light)
    }
}

/// The colour of the controls; kept in the settings by name (`amber`,
/// `#12AB9F`), an unknown one read as the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
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
    /// The user's own, `0xRRGGBB`.
    Custom(u32),
}

impl From<String> for Accent {
    fn from(name: String) -> Self {
        Accent::from_name(&name).unwrap_or_default()
    }
}

impl From<Accent> for String {
    fn from(accent: Accent) -> Self {
        accent.name()
    }
}

impl Accent {
    /// The ready ones (not the user's own).
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

    /// The name in the settings: a ready one's, or `#RRGGBB`.
    pub fn name(self) -> String {
        let name = match self {
            Accent::Amber => "amber",
            Accent::Blue => "blue",
            Accent::Teal => "teal",
            Accent::Green => "green",
            Accent::Orange => "orange",
            Accent::Red => "red",
            Accent::Pink => "pink",
            Accent::Purple => "purple",
            Accent::Graphite => "graphite",
            Accent::Custom(c) => return format!("#{:06X}", c & 0xffffff),
        };
        name.to_owned()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        if let Some([r, g, b]) = parse_hex(name) {
            return Some(Accent::Custom(u32::from_be_bytes([0, r, g, b])));
        }
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    /// A colour as typed in its field: `#RRGGBB` or `#RGB` (the `#` may
    /// be left out), `RGB(r, g, b)` or `r, g, b`, `HSL(h, s%, l%)`; spaces
    /// around it ignored.
    pub fn typed(text: &str) -> Option<Self> {
        let text = text.trim();
        let [r, g, b] = hex_typed(text)
            .or_else(|| rgb_typed(text))
            .or_else(|| hsl_typed(text))?;
        Some(Accent::Custom(u32::from_be_bytes([0, r, g, b])))
    }

    pub fn label(self) -> &'static str {
        match self {
            Accent::Custom(_) => tr!("Your own colour", "Свой цвет"),
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
            Accent::Custom(c) => c & 0xffffff,
        })
    }
}

/// Where a shade comes from: the selected rows' ground (`Look::selection`)
/// and the track playing's text (`Look::playing`) are mixed from the
/// accent (the default), from a colour of their own, or are the user's
/// colour as it is. Amber over a dark ground mixes to a brown the user
/// found dirty; this lets them go their own way. Kept in the settings by
/// name (`accent`, `grey`, `#2B3442`), an unknown one read as the
/// default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Tint {
    #[default]
    Accent,
    /// The theme's brightest text (white on a dark ground, black on a
    /// light one): the track playing read best. Not a selection's.
    Bright,
    Grey,
    Slate,
    Blue,
    Teal,
    Purple,
    /// The user's own, `0xRRGGBB`, used as it is.
    Custom(u32),
}

impl From<String> for Tint {
    fn from(name: String) -> Self {
        Tint::from_name(&name).unwrap_or_default()
    }
}

impl From<Tint> for String {
    fn from(selection: Tint) -> Self {
        selection.name()
    }
}

impl Tint {
    /// The ready ones (not the user's own).
    pub const ALL: [Tint; 7] = [
        Tint::Accent,
        Tint::Bright,
        Tint::Grey,
        Tint::Slate,
        Tint::Blue,
        Tint::Teal,
        Tint::Purple,
    ];
    /// Those offered for the selection's ground and for the track
    /// playing.
    pub const SELECTION: [Tint; 6] = [
        Tint::Accent,
        Tint::Grey,
        Tint::Slate,
        Tint::Blue,
        Tint::Teal,
        Tint::Purple,
    ];
    pub const PLAYING: [Tint; 7] = Tint::ALL;

    pub fn name(self) -> String {
        let name = match self {
            Tint::Accent => "accent",
            Tint::Bright => "bright",
            Tint::Grey => "grey",
            Tint::Slate => "slate",
            Tint::Blue => "blue",
            Tint::Teal => "teal",
            Tint::Purple => "purple",
            Tint::Custom(c) => return format!("#{:06X}", c & 0xffffff),
        };
        name.to_owned()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        if let Some([r, g, b]) = parse_hex(name) {
            return Some(Tint::Custom(u32::from_be_bytes([0, r, g, b])));
        }
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tint::Accent => tr!("Of the colour above", "Из цвета выше"),
            Tint::Bright => tr!("Bright", "Яркий"),
            Tint::Grey => tr!("Grey", "Серый"),
            Tint::Slate => tr!("Slate", "Сланцевый"),
            Tint::Blue => tr!("Blue", "Синий"),
            Tint::Teal => tr!("Teal", "Бирюзовый"),
            Tint::Purple => tr!("Purple", "Фиолетовый"),
            Tint::Custom(_) => tr!("Your own colour", "Свой цвет"),
        }
    }

    /// The colour the shade is mixed from (`None`: the accent's; the
    /// user's own is not mixed).
    fn base(self) -> Option<Color32> {
        Some(hex(match self {
            Tint::Accent | Tint::Bright | Tint::Custom(_) => return None,
            Tint::Grey => 0x8b919c,
            Tint::Slate => 0x5a7aa6,
            Tint::Blue => 0x2f7cf0,
            Tint::Teal => 0x0d9488,
            Tint::Purple => 0x7c3aed,
        }))
    }
}

/// `#RRGGBB` (the `#` required, six hex digits).
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// How the own colour's code is shown: `#RRGGBB`, `RGB(r, g, b)` or
/// `HSL(h, s%, l%)`; any of them is read back (`Accent::typed`)
/// (metalmpv's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColourText {
    #[default]
    Hex,
    Rgb,
    Hsl,
}

impl ColourText {
    pub fn next(self) -> Self {
        match self {
            ColourText::Hex => ColourText::Rgb,
            ColourText::Rgb => ColourText::Hsl,
            ColourText::Hsl => ColourText::Hex,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ColourText::Hex => "HEX",
            ColourText::Rgb => "RGB",
            ColourText::Hsl => "HSL",
        }
    }

    pub fn show(self, [r, g, b]: [u8; 3]) -> String {
        match self {
            ColourText::Hex => format!("#{r:02X}{g:02X}{b:02X}"),
            ColourText::Rgb => format!("RGB({r}, {g}, {b})"),
            ColourText::Hsl => {
                let [h, s, l] = hsl_of([r, g, b]);
                format!(
                    "HSL({:.0}, {:.0}%, {:.0}%)",
                    h.round() % 360.0,
                    s * 100.0,
                    l * 100.0
                )
            }
        }
    }
}

/// Six or three hex digits, `#` before them or not.
fn hex_typed(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    let six = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => digits.to_owned(),
        _ => return None,
    };
    parse_hex(&format!("#{six}"))
}

/// The three numbers inside `name(…)` (any case), or the text itself
/// when it has no such name but commas; units after them are the caller's.
fn parts<'a>(text: &'a str, name: &str) -> Option<Vec<&'a str>> {
    let inner = match text.get(..name.len()) {
        Some(head) if head.eq_ignore_ascii_case(name) => text[name.len()..]
            .trim()
            .strip_prefix('(')?
            .strip_suffix(')')?,
        _ if text.contains(',') && !text.contains('(') => text,
        _ => return None,
    };
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    (parts.len() == 3).then_some(parts)
}

/// `RGB(r, g, b)` or `r, g, b`, each 0–255.
fn rgb_typed(text: &str) -> Option<[u8; 3]> {
    let parts = parts(text, "rgb")?;
    let mut rgb = [0; 3];
    for (value, part) in rgb.iter_mut().zip(parts) {
        *value = part.parse().ok()?;
    }
    Some(rgb)
}

/// `HSL(h, s%, l%)`: the hue in degrees (`deg` or `°` after it allowed),
/// the others in per cent (`%` optional), 0–100.
fn hsl_typed(text: &str) -> Option<[u8; 3]> {
    let parts = parts(text, "hsl")?;
    let number = |part: &str, units: &[&str]| {
        let part = units
            .iter()
            .fold(part, |p, u| p.strip_suffix(u).unwrap_or(p))
            .trim();
        part.parse::<f32>().ok().filter(|v| v.is_finite())
    };
    let h = number(parts[0], &["deg", "°"])?;
    let s = number(parts[1], &["%"]).filter(|v| (0.0..=100.0).contains(v))?;
    let l = number(parts[2], &["%"]).filter(|v| (0.0..=100.0).contains(v))?;
    Some(rgb_of_hsl(h.rem_euclid(360.0), s / 100.0, l / 100.0))
}

/// Hue in degrees, saturation and lightness 0–1.
fn hsl_of([r, g, b]: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = [r, g, b].map(|c| f32::from(c) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return [0.0, 0.0, l];
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    [h, s, l]
}

fn rgb_of_hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// Every colour of the interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// The title row.
    pub title_bg: Color32,
    /// The settings window's column of pages (the title row's ground
    /// unless the theme says otherwise).
    pub sidebar_bg: Color32,
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
    /// The rows of the lists and tables (`text` unless the theme says
    /// otherwise), and the column headers (`text_weak` unless so).
    pub list_text: Color32,
    pub header_text: Color32,
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
    /// The background of selected rows, and their text.
    pub selected_bg: Color32,
    pub selected_text: Color32,
    /// The spectrum analyzer's bars at the foot and full, and the peaks.
    pub spectrum_low: Color32,
    pub spectrum_high: Color32,
    pub spectrum_peak: Color32,
}

impl Palette {
    pub fn of(look: Look) -> Palette {
        let Look {
            theme,
            accent,
            selection,
            playing,
        } = look;
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
            // Taken from a screenshot of Winamp's media library.
            Theme::Steel => Palette {
                dark: true,
                title_bg: hex(0x343946),
                sidebar_bg: hex(0x1f2223),
                bar_bg: hex(0x1a1b1c),
                panel_bg: hex(0x333738),
                list_bg: hex(0x08090a),
                list_stripe: hex(0x0d0e0f),
                header_bg: hex(0x494c4d),
                card_bg: hex(0x26292a),
                line: hex(0x404446),
                // Winamp's: steel blue in the lists, light grey around
                // them and on the column headers.
                text: hex(0xd2d2d2),
                list_text: hex(0x93afb9),
                header_text: hex(0xd2d2d2),
                text_weak: hex(0x9a9fa1),
                text_bright: hex(0xf0f0f0),
                hover: hex(0x1c1f24),
                pressed: hex(0x2a2e39),
                track: hex(0x3a3e40),
                scroll_handle: hex(0x494c4d),
                scroll_handle_hover: hex(0x646768),
                scroll_handle_drag: hex(0x7d8081),
                close_hover: hex(0xc42b1c),
                error: hex(0xff8a80),
                ..Palette::BLANK
            },
            Theme::Dimmed => Palette {
                dark: true,
                title_bg: hex(0x25292f),
                bar_bg: hex(0x2a2f36),
                panel_bg: hex(0x30353d),
                list_bg: hex(0x22272e),
                list_stripe: hex(0x262b33),
                header_bg: hex(0x343a43),
                card_bg: hex(0x383e48),
                line: hex(0x414852),
                text: hex(0xd8dde3),
                text_weak: hex(0x959ca6),
                text_bright: hex(0xffffff),
                hover: hex(0x333943),
                pressed: hex(0x434a55),
                track: hex(0x4a515c),
                scroll_handle: hex(0x5a626e),
                scroll_handle_hover: hex(0x737b88),
                scroll_handle_drag: hex(0x8c94a0),
                close_hover: hex(0xc42b1c),
                error: hex(0xff8a80),
                ..Palette::BLANK
            },
            Theme::Sepia => Palette {
                dark: false,
                title_bg: hex(0xe9e0cc),
                bar_bg: hex(0xefe7d5),
                panel_bg: hex(0xebe3d0),
                list_bg: hex(0xf8f3e6),
                list_stripe: hex(0xf2ecdc),
                header_bg: hex(0xe4dac4),
                card_bg: hex(0xdfd4bc),
                line: hex(0xcfc3a8),
                text: hex(0x3b3125),
                text_weak: hex(0x6e6250),
                text_bright: hex(0x1a140c),
                hover: hex(0xe2d8c2),
                pressed: hex(0xd6cab0),
                track: hex(0xcbbea2),
                scroll_handle: hex(0xb9ab8e),
                scroll_handle_hover: hex(0xa29475),
                scroll_handle_drag: hex(0x8a7c5f),
                close_hover: hex(0xc42b1c),
                error: hex(0xb3261e),
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
        // What a theme leaves out follows the others.
        if p.sidebar_bg == Color32::TRANSPARENT {
            p.sidebar_bg = p.title_bg;
        }
        if p.list_text == Color32::TRANSPARENT {
            p.list_text = p.text;
        }
        if p.header_text == Color32::TRANSPARENT {
            p.header_text = p.text_weak;
        }
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
        } else {
            // On white the accent is a shade deeper, to be read.
            let deep = mix(base, black, 0.12);
            p.accent = deep;
            p.accent_hover = mix(deep, black, 0.12);
            p.playing = mix(base, black, 0.3);
            p.selected_bg = mix(p.list_bg, base, 0.2);
            p.spectrum_low = mix(base, black, 0.1);
            p.spectrum_high = mix(base, white, 0.25);
            p.spectrum_peak = mix(p.text, base, 0.3);
        }
        p.on_accent = if luminance(p.accent) > 0.32 {
            hex(0x1a1b1f)
        } else {
            white
        };
        if let Tint::Custom(c) = selection {
            p.selected_bg = hex(c & 0xffffff);
        } else if let Some(base) = selection.base() {
            p.selected_bg = if p.dark {
                mix(p.list_bg, base, 0.26)
            } else {
                mix(p.list_bg, base, 0.2)
            };
        }
        // The track playing's text, as the accent's is mixed.
        if let Tint::Custom(c) = playing {
            p.playing = hex(c & 0xffffff);
        } else if playing == Tint::Bright {
            p.playing = p.text_bright;
        } else if let Some(base) = playing.base() {
            p.playing = if p.dark {
                mix(base, white, 0.45)
            } else {
                mix(base, black, 0.3)
            };
        }
        // The theme's bright text, unless the user's own colour is too
        // light (or dark) for it: then black or white, whichever reads.
        p.selected_text = [p.text_bright, black, white]
            .into_iter()
            .find(|&t| contrast(t, p.selected_bg) >= 4.5)
            .unwrap_or(if luminance(p.selected_bg) > 0.18 {
                black
            } else {
                white
            });
        p
    }

    const BLANK: Palette = Palette {
        dark: true,
        title_bg: Color32::BLACK,
        sidebar_bg: Color32::TRANSPARENT,
        bar_bg: Color32::BLACK,
        panel_bg: Color32::BLACK,
        list_bg: Color32::BLACK,
        list_stripe: Color32::BLACK,
        header_bg: Color32::BLACK,
        card_bg: Color32::BLACK,
        line: Color32::BLACK,
        text: Color32::WHITE,
        list_text: Color32::TRANSPARENT,
        header_text: Color32::TRANSPARENT,
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
        selected_text: Color32::WHITE,
        spectrum_low: Color32::BLACK,
        spectrum_high: Color32::BLACK,
        spectrum_peak: Color32::BLACK,
    };
}

/// The palette in use.
static CURRENT: RwLock<Option<Palette>> = RwLock::new(None);

/// Use `look` from now on (then `apply` for egui's own).
pub fn set(look: Look) {
    let p = Palette::of(look);
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
        .unwrap_or_else(|| Palette::of(Look::default()))
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
    sidebar_bg,
    list_text,
    header_text,
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
    selected_text,
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
        v.selection.stroke.color = p.selected_text;
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

    #[test]
    fn the_default_is_dark_amber_its_shades_amber_too() {
        let p = Palette::of(Look {
            theme: Theme::Dark,
            accent: Accent::Amber,
            selection: Tint::Accent,
            playing: Tint::Accent,
        });
        assert_eq!(p.accent, hex(0xf5a623));
        assert_eq!(p.list_bg, hex(0x16171a));
        assert_eq!(p.on_accent, hex(0x1a1b1f));
        // The selection and the track playing in the accent's own hue,
        // as with every other accent (red over blue).
        assert!(p.selected_bg.r() > p.selected_bg.b());
        assert!(p.playing.r() > p.playing.b());
    }

    #[test]
    fn themes_go_from_the_darkest_to_the_lightest() {
        // The window's mean luminance: the lists take most of it, then
        // the panels around them, then the player bar.
        let light = |t: Theme| {
            let p = Palette::of(Look {
                theme: t,
                ..Look::default()
            });
            assert_eq!(p.dark, t.is_dark(), "{t:?}");
            0.6 * luminance(p.list_bg) + 0.25 * luminance(p.panel_bg) + 0.15 * luminance(p.bar_bg)
        };
        for pair in Theme::ALL.windows(2) {
            assert!(
                light(pair[0]) < light(pair[1]),
                "{:?} before {:?}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(Theme::default(), Theme::Dark);
    }

    #[test]
    fn every_selection_can_be_read() {
        let own = [0xffffff, 0x000000, 0x777777, 0xffd700, 0x2b3442, 0x00ff00];
        for theme in Theme::ALL {
            for selection in Tint::SELECTION.into_iter().chain(own.map(Tint::Custom)) {
                let p = Palette::of(Look {
                    theme,
                    accent: Accent::Amber,
                    selection,
                    playing: Tint::Accent,
                });
                let what = format!("{theme:?} {selection:?}");
                assert!(
                    contrast(p.selected_text, p.selected_bg) >= 4.5,
                    "{what}: selected text"
                );
                if !matches!(selection, Tint::Custom(_)) {
                    assert!(contrast(p.text, p.selected_bg) >= 4.5, "{what}: text");
                }
            }
        }
        // Grey is grey whatever the accent; the user's own is as chosen.
        let grey = Palette::of(Look {
            theme: Theme::Dark,
            accent: Accent::Amber,
            selection: Tint::Grey,
            playing: Tint::Accent,
        })
        .selected_bg;
        assert_eq!(
            grey,
            Palette::of(Look {
                theme: Theme::Dark,
                accent: Accent::Red,
                selection: Tint::Grey,
                playing: Tint::Accent
            })
            .selected_bg
        );
        assert_eq!(
            Palette::of(Look {
                theme: Theme::Light,
                accent: Accent::Amber,
                selection: Tint::Custom(0x2b3442),
                playing: Tint::Accent
            })
            .selected_bg,
            hex(0x2b3442)
        );
        assert_eq!(Tint::from_name("grey"), Some(Tint::Grey));
        assert_eq!(
            serde_json::to_string(&Tint::Custom(0x2b3442)).unwrap(),
            "\"#2B3442\""
        );
        assert_eq!(
            serde_json::from_str::<Tint>("\"pink\"").unwrap(),
            Tint::Accent
        );
    }

    #[test]
    fn the_track_playing_can_be_read() {
        for theme in Theme::ALL {
            for playing in Tint::PLAYING {
                let p = Palette::of(Look {
                    theme,
                    playing,
                    ..Look::default()
                });
                assert!(
                    contrast(p.playing, p.list_bg) >= 3.0,
                    "{theme:?} {playing:?}"
                );
            }
        }
        // The user's own as it is; the selection does not move it.
        let own = Look {
            playing: Tint::Custom(0x8fc3ff),
            selection: Tint::Grey,
            ..Look::default()
        };
        assert_eq!(Palette::of(own).playing, hex(0x8fc3ff));
        let bright = |theme| {
            Palette::of(Look {
                theme,
                playing: Tint::Bright,
                ..Look::default()
            })
            .playing
        };
        assert_eq!(bright(Theme::Dark), Color32::WHITE);
        assert_eq!(bright(Theme::Light), Color32::BLACK);
    }

    #[test]
    fn colour_texts() {
        let teal = [0x44, 0x99, 0x99];
        assert_eq!(ColourText::Hex.show(teal), "#449999");
        assert_eq!(ColourText::Rgb.show(teal), "RGB(68, 153, 153)");
        assert_eq!(ColourText::Hsl.show(teal), "HSL(180, 38%, 43%)");
        for mode in [ColourText::Hex, ColourText::Rgb] {
            assert_eq!(
                Accent::typed(&mode.show(teal)),
                Some(Accent::Custom(0x449999))
            );
        }
        // HSL in whole numbers comes back within a step of each part.
        let Some(Accent::Custom(c)) = Accent::typed(&ColourText::Hsl.show(teal)) else {
            panic!("HSL not read");
        };
        let [_, r, g, b] = c.to_be_bytes();
        for (got, was) in [r, g, b].into_iter().zip(teal) {
            assert!(got.abs_diff(was) <= 2, "{got} for {was}");
        }
        assert_eq!(
            Accent::typed("rgb( 255 ,0, 10 )"),
            Some(Accent::Custom(0xff000a))
        );
        assert_eq!(Accent::typed("255, 0, 10"), Some(Accent::Custom(0xff000a)));
        assert_eq!(Accent::typed("RGB(256, 0, 0)"), None);
        assert_eq!(Accent::typed("RGB(1, 2)"), None);
        assert_eq!(
            Accent::typed("hsl(0, 100%, 50%)"),
            Some(Accent::Custom(0xff0000))
        );
        assert_eq!(
            Accent::typed("HSL(120deg, 100, 25)"),
            Some(Accent::Custom(0x008000))
        );
        assert_eq!(Accent::typed("hsl(0, 120%, 50%)"), None);
        assert_eq!(ColourText::Hex.next().next().next(), ColourText::Hex);
    }

    #[test]
    fn accent_names() {
        for a in Accent::ALL {
            assert_eq!(Accent::from_name(&a.name()), Some(a));
        }
        assert_eq!(Accent::from_name("cyan"), None);
        let own = Accent::Custom(0x12ab9f);
        assert_eq!(own.name(), "#12AB9F");
        assert_eq!(Accent::from_name("#12AB9F"), Some(own));
        assert_eq!(Accent::typed(" #12AB9F\n"), Some(own));
        assert_eq!(Accent::typed("1a9"), Some(Accent::Custom(0x11aa99)));
        assert_eq!(Accent::typed("#12ab9"), None);
        assert_eq!(Accent::typed("#ЖЖЖ"), None);
        // Kept in the settings by name; an unknown one is the default.
        assert_eq!(serde_json::to_string(&own).unwrap(), "\"#12AB9F\"");
        assert_eq!(
            serde_json::from_str::<Accent>("\"teal\"").unwrap(),
            Accent::Teal
        );
        assert_eq!(serde_json::from_str::<Accent>("\"#12ab9f\"").unwrap(), own);
        assert_eq!(
            serde_json::from_str::<Accent>("\"cyan\"").unwrap(),
            Accent::Amber
        );
    }

    #[test]
    fn every_palette_can_be_read() {
        for theme in Theme::ALL {
            for accent in Accent::ALL {
                let p = Palette::of(Look {
                    theme,
                    accent,
                    selection: Tint::Accent,
                    playing: Tint::Accent,
                });
                let what = format!("{theme:?} {accent:?}");
                assert!(contrast(p.text, p.list_bg) >= 7.0, "{what}: text");
                assert!(contrast(p.list_text, p.list_bg) >= 7.0, "{what}: lists");
                assert!(contrast(p.text_weak, p.list_bg) >= 4.0, "{what}: weak text");
                // The settings' cards and the column headers too.
                assert!(contrast(p.text, p.card_bg) >= 7.0, "{what}: text on a card");
                assert!(
                    contrast(p.text_weak, p.card_bg) >= 3.5,
                    "{what}: weak text on a card"
                );
                assert!(
                    contrast(p.text_weak, p.panel_bg) >= 3.5,
                    "{what}: weak text on a panel"
                );
                assert!(
                    contrast(p.header_text, p.header_bg) >= 4.0,
                    "{what}: headers"
                );
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
