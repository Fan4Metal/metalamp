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

use crate::settings::Visual;

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
/// `selection`, `playing` and `visual_colours`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub theme: Theme,
    pub accent: Accent,
    pub selection: Tint,
    pub playing: Tint,
    pub visual: VisualColours,
}

/// The visualization's colours, apart from the theme's: each
/// visualization's own (`Own`, the default: `scheme`), the accent's
/// shades, a rainbow across the frequencies, Winamp's green to red by the
/// level, fire by the level, the colours of the cover of the track playing
/// (`set_cover`) across the frequencies, or the user's own colour's
/// shades. Kept by name, `rainbow` or `#12AB9F`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum VisualColours {
    /// Colours chosen for each visualization (`scheme`), the same on any
    /// dark theme; made readable on a light one.
    #[default]
    Own,
    Accent,
    Rainbow,
    Winamp,
    Fire,
    /// Up to three colours of the cover heard, across the place as the
    /// rainbow's; the accent's shades while there is no cover.
    Cover,
    /// `0xRRGGBB`, its shades mixed as the accent's.
    Custom(u32),
}

impl From<String> for VisualColours {
    fn from(name: String) -> Self {
        VisualColours::from_name(&name).unwrap_or_default()
    }
}

impl From<VisualColours> for String {
    fn from(colours: VisualColours) -> Self {
        colours.name()
    }
}

impl VisualColours {
    /// The ready ones (not the user's own).
    pub const ALL: [VisualColours; 6] = [
        VisualColours::Own,
        VisualColours::Accent,
        VisualColours::Rainbow,
        VisualColours::Winamp,
        VisualColours::Fire,
        VisualColours::Cover,
    ];

    pub fn name(self) -> String {
        let name = match self {
            VisualColours::Own => "own",
            VisualColours::Accent => "accent",
            VisualColours::Rainbow => "rainbow",
            VisualColours::Winamp => "winamp",
            VisualColours::Fire => "fire",
            VisualColours::Cover => "cover",
            VisualColours::Custom(c) => return format!("#{:06X}", c & 0xffffff),
        };
        name.to_owned()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        if let Some([r, g, b]) = parse_hex(name) {
            return Some(VisualColours::Custom(u32::from_be_bytes([0, r, g, b])));
        }
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            VisualColours::Own => tr!(
                "Each visualization's own",
                "Свои у каждой визуализации"
            ),
            VisualColours::Accent => tr!("Of the accent", "Из цвета акцента"),
            VisualColours::Rainbow => tr!("Rainbow", "Радуга"),
            VisualColours::Winamp => tr!("Winamp", "Как в Winamp"),
            VisualColours::Fire => tr!("Fire", "Огонь"),
            VisualColours::Cover => tr!("Of the cover", "Из обложки"),
            VisualColours::Custom(_) => tr!("Your own colour", "Свой цвет"),
        }
    }
}

/// Winamp's spectrum, by the level: green, yellow, red.
const WINAMP_STOPS: [(f32, u32); 3] = [(0.0, 0x1c9e2a), (0.55, 0xd9cf26), (1.0, 0xe2311d)];
/// Fire, by the level: a dark red, orange, yellow.
const FIRE_STOPS: [(f32, u32); 3] = [(0.0, 0x5e0b02), (0.5, 0xe2520c), (1.0, 0xffdc55)];

/// A visualization's own colours: rows at levels from the floor (0) to the
/// top (1), each a gradient across the place (the bass or the left edge
/// first), and the peaks' across; `by_level` as `VisualPalette::by_level`.
struct Scheme {
    rows: &'static [(f32, &'static [u32])],
    peak: &'static [u32],
    by_level: bool,
}

/// The most rows and colours across a scheme may have.
const MOST_ROWS: usize = 5;
const MOST_ACROSS: usize = 7;

/// The colours chosen for each visualization (`VisualColours::Own`), for a
/// dark ground.
fn scheme(visual: Visual) -> Scheme {
    match visual {
        // Neon: magenta at the bass, violet, cyan at the treble.
        Visual::Spectrum | Visual::Off => Scheme {
            rows: &[
                (0.0, &[0x4a0f5e, 0x24166e, 0x08406a]),
                (0.55, &[0xd61c9e, 0x6c3df2, 0x169fe0]),
                (1.0, &[0xff6ad5, 0xa98cff, 0x6ff0ff]),
            ],
            peak: &[0xffe0f6, 0xece6ff, 0xe0fdff],
            by_level: false,
        },
        // A hi-fi's meter: green, then yellow, orange and red at the top.
        Visual::Led => Scheme {
            rows: &[
                (0.0, &[0x18c850]),
                (0.5, &[0x5ee83a]),
                (0.7, &[0xffd21e]),
                (0.85, &[0xff8a1a]),
                (1.0, &[0xff2e2e]),
            ],
            peak: &[0xffece0],
            by_level: true,
        },
        // The sea: turquoise, azure, indigo.
        Visual::Curve => Scheme {
            rows: &[
                (0.0, &[0x06324f, 0x063a5a, 0x1c2a60]),
                (1.0, &[0x2cf0c8, 0x34b4ff, 0x8f80ff]),
            ],
            peak: &[0xd0fff4, 0xd4f0ff, 0xe6e0ff],
            by_level: false,
        },
        // A sunset: amber, rose, violet.
        Visual::Mirror => Scheme {
            rows: &[
                (0.0, &[0x5a1800, 0x580a2c, 0x2c0e58]),
                (1.0, &[0xffab2e, 0xff3f80, 0xa95eff]),
            ],
            peak: &[0xffe6bf, 0xffcde2, 0xe9dbff],
            by_level: false,
        },
        // Acid neon: mint at the bass, lime, lemon at the treble.
        Visual::Ridges => Scheme {
            rows: &[
                (0.0, &[0x0a4a3a, 0x1e5a14, 0x4a5a0a]),
                (1.0, &[0x3dffc0, 0x8aff5a, 0xe6ff4d]),
            ],
            peak: &[0xe0fff4, 0xeeffe0, 0xfcffe0],
            by_level: false,
        },
        // Dusk: coral bass in front, orange, gold at the back.
        Visual::Strata => Scheme {
            rows: &[
                (0.0, &[0x4a0e36, 0x52200c, 0x4c4410]),
                (1.0, &[0xff5b7f, 0xffa040, 0xffe46a]),
            ],
            peak: &[0xffe0e8, 0xffecd6, 0xfff8d8],
            by_level: false,
        },
        // A thermal camera's: violet, crimson, orange, pale yellow.
        Visual::Spectrogram => Scheme {
            rows: &[
                (0.0, &[0x2c0a5e]),
                (0.3, &[0x8a226a]),
                (0.6, &[0xe65d2f]),
                (1.0, &[0xfcf4a2]),
            ],
            peak: &[0xffffff],
            by_level: true,
        },
        // Outrun: the sun from yellow to hot pink, the grid cyan at the
        // sides and magenta in the middle.
        Visual::Synthwave => Scheme {
            rows: &[
                (0.0, &[0x2a0a5a, 0x3a0a4a, 0x2a0a5a]),
                (0.45, &[0xff2a8a, 0xff2a8a, 0xff2a8a]),
                (0.85, &[0x3ad8ff, 0xff3cd0, 0x3ad8ff]),
                (1.0, &[0xffe45c, 0xffb04a, 0xffe45c]),
            ],
            peak: &[0xfff2c8],
            by_level: false,
        },
        // An electric arc: cyan at the ends, violet at the core.
        Visual::Lightning => Scheme {
            rows: &[
                (0.0, &[0x10306a, 0x2a1a6a, 0x10306a]),
                (1.0, &[0x5fdcff, 0xb08cff, 0x5fdcff]),
            ],
            peak: &[0xf0f6ff],
            by_level: false,
        },
        // Psychedelic: pink, blue, mint.
        Visual::Vortex => Scheme {
            rows: &[
                (0.0, &[0x40082a, 0x14205a, 0x06402e]),
                (1.0, &[0xff4fa8, 0x6f7dff, 0x2effc4]),
            ],
            peak: &[0xffffff],
            by_level: false,
        },
        // Flames: embers, red, orange, yellow, white-hot.
        Visual::Fire => Scheme {
            rows: &[
                (0.0, &[0x2a0802]),
                (0.3, &[0x9e1e04]),
                (0.55, &[0xe85a0a]),
                (0.8, &[0xffb22a]),
                (1.0, &[0xfff2b0]),
            ],
            peak: &[0xfff8e6],
            by_level: true,
        },
        // A welder's sparks: red-orange, amber, pale gold.
        Visual::Sparks => Scheme {
            rows: &[
                (0.0, &[0x5a1a04, 0x5a3604, 0x56501c]),
                (1.0, &[0xff5a1f, 0xffb52e, 0xfff27a]),
            ],
            peak: &[0xfff6e6],
            by_level: false,
        },
        // The aurora's own: green, teal, violet higher up.
        Visual::Aurora => Scheme {
            rows: &[
                (0.0, &[0x0a4a2a, 0x0a4646, 0x2e1452]),
                (1.0, &[0x4dff9a, 0x3ff0e0, 0xc27cff]),
            ],
            peak: &[0xdcffe9, 0xd9fffa, 0xf0e0ff],
            by_level: false,
        },
        // Indigo peaks round an ice-blue middle.
        Visual::Waveform => Scheme {
            rows: &[
                (0.0, &[0x5a2ee0, 0x3a5ef0, 0x1e8ef0]),
                (1.0, &[0xb48cff, 0x7cc8ff, 0x7ff6ff]),
            ],
            peak: &[0xffffff],
            by_level: false,
        },
        // An oscilloscope's green phosphor, brighter as it swings.
        Visual::Scope => Scheme {
            rows: &[(0.0, &[0x14a040]), (1.0, &[0xb4ffcc])],
            peak: &[0xe8fff0],
            by_level: true,
        },
        // An amber phosphor, as a studio's meter.
        Visual::Goniometer => Scheme {
            rows: &[(0.0, &[0x5a2e00]), (1.0, &[0xffc860])],
            peak: &[0xfff0c8],
            by_level: false,
        },
        // A colour wheel round the twelve notes.
        Visual::Chroma => Scheme {
            rows: &[
                (
                    0.0,
                    &[0x591b1b, 0x59401b, 0x40541b, 0x1b5438, 0x1b4659, 0x2a2559, 0x512059],
                ),
                (
                    1.0,
                    &[0xff4d4d, 0xffb84d, 0xb8f04d, 0x4df0a0, 0x4dc8ff, 0x7a6bff, 0xe85dff],
                ),
            ],
            peak: &[0xffffff],
            by_level: false,
        },
        // Stage light on the keys: cyan, blue, orchid up the keyboard.
        Visual::Piano => Scheme {
            rows: &[
                (0.0, &[0x0e3e58, 0x22285e, 0x44205a]),
                (1.0, &[0x3ddcff, 0x7a84ff, 0xdc72ff]),
            ],
            peak: &[0xffffff],
            by_level: false,
        },
        // The strings: bronze wound ones, then gold, the plain steel ones.
        Visual::Strings => Scheme {
            rows: &[
                (0.0, &[0x5a2a08, 0x5a400a, 0x3e4450]),
                (1.0, &[0xff9a3c, 0xffd36b, 0xe4eeff]),
            ],
            peak: &[0xfff6e0],
            by_level: false,
        },
    }
}

/// A scheme's colours as shown: as they are on a dark ground, made
/// `readable` on a light one (bright lines would vanish on it).
#[derive(Clone, Copy, Debug, PartialEq)]
struct OwnShades {
    levels: [f32; MOST_ROWS],
    rows: [[Color32; MOST_ACROSS]; MOST_ROWS],
    across: [usize; MOST_ROWS],
    count: usize,
    peak: [Color32; MOST_ACROSS],
    peaks: usize,
    by_level: bool,
}

impl OwnShades {
    fn of(visual: Visual, dark: bool) -> OwnShades {
        let s = scheme(visual);
        let shown = |c: u32| if dark { hex(c) } else { readable(hex(c), false) };
        let mut o = OwnShades {
            levels: [0.0; MOST_ROWS],
            rows: [[Color32::BLACK; MOST_ACROSS]; MOST_ROWS],
            across: [1; MOST_ROWS],
            count: s.rows.len().min(MOST_ROWS),
            peak: [Color32::BLACK; MOST_ACROSS],
            peaks: s.peak.len().clamp(1, MOST_ACROSS),
            by_level: s.by_level,
        };
        for (i, (level, colours)) in s.rows.iter().take(MOST_ROWS).enumerate() {
            o.levels[i] = *level;
            o.across[i] = colours.len().clamp(1, MOST_ACROSS);
            for (j, &c) in colours.iter().take(MOST_ACROSS).enumerate() {
                o.rows[i][j] = shown(c);
            }
        }
        for (j, &c) in s.peak.iter().take(MOST_ACROSS).enumerate() {
            o.peak[j] = shown(c);
        }
        o
    }

    fn at(&self, level: f32, place: f32) -> Color32 {
        let row = |i: usize| along_colours(&self.rows[i][..self.across[i]], place);
        let n = self.count;
        if n < 2 {
            return row(0);
        }
        let i = (0..n - 1)
            .position(|i| level <= self.levels[i + 1])
            .unwrap_or(n - 2);
        let (a, b) = (self.levels[i], self.levels[i + 1]);
        mix(row(i), row(i + 1), ((level - a) / (b - a).max(1e-6)).clamp(0.0, 1.0))
    }

    fn peak(&self, place: f32) -> Color32 {
        along_colours(&self.peak[..self.peaks], place)
    }
}

/// The colour at `place` (0 to 1) of a gradient through `colours`, evenly
/// spaced.
fn along_colours(colours: &[Color32], place: f32) -> Color32 {
    let n = colours.len();
    if n < 2 {
        return colours.first().copied().unwrap_or(Color32::BLACK);
    }
    let x = place.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (x as usize).min(n - 2);
    mix(colours[i], colours[i + 1], x - i as f32)
}

/// The visualization's colours in use: a colour for a height and a place
/// across (`at`), and the peaks' (`peak`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualPalette {
    pub colours: VisualColours,
    /// The shades of the accent or the user's own colour: the floor, the
    /// top, the peaks.
    low: Color32,
    high: Color32,
    peak: Color32,
    dark: bool,
    text: Color32,
    /// The cover's colours made readable on the ground (`Cover`; none: no
    /// cover, the accent's shades).
    cover: Option<[Color32; 3]>,
    /// The visualization's own colours (`Own`, `visual_for`; none: the
    /// accent's shades).
    own: Option<OwnShades>,
}

impl VisualPalette {
    /// The colours tell the level (Winamp's, fire), not only the height:
    /// a waveform's column, a scope's line are coloured by their loudness.
    pub fn by_level(self) -> bool {
        match self.colours {
            VisualColours::Winamp | VisualColours::Fire => true,
            VisualColours::Own => self.own.is_some_and(|o| o.by_level),
            _ => false,
        }
    }

    /// The ground is dark (light shows as light on it).
    pub fn is_dark(self) -> bool {
        self.dark
    }

    /// The colour at `level` (0: the floor, 1: the top) and `place`
    /// across (0: the lowest band or the left edge, 1: the highest or the
    /// right edge).
    pub fn at(self, level: f32, place: f32) -> Color32 {
        let level = level.clamp(0.0, 1.0);
        match self.colours {
            VisualColours::Own => match &self.own {
                Some(own) => own.at(level, place),
                None => mix(self.low, self.high, level),
            },
            VisualColours::Accent | VisualColours::Custom(_) => mix(self.low, self.high, level),
            VisualColours::Rainbow => {
                let hue = rainbow(place);
                mix(
                    mix(hue, Color32::BLACK, 0.3),
                    mix(hue, Color32::WHITE, 0.2),
                    level,
                )
            }
            VisualColours::Winamp => along(&WINAMP_STOPS, level),
            VisualColours::Fire => along(&FIRE_STOPS, level),
            VisualColours::Cover => match self.cover {
                Some(colours) => {
                    let hue = across(colours, place);
                    mix(
                        mix(hue, Color32::BLACK, 0.3),
                        mix(hue, Color32::WHITE, 0.2),
                        level,
                    )
                }
                None => mix(self.low, self.high, level),
            },
        }
    }

    /// The peaks' colour at `place`.
    pub fn peak(self, place: f32) -> Color32 {
        match self.colours {
            VisualColours::Own => match &self.own {
                Some(own) => own.peak(place),
                None => self.peak,
            },
            VisualColours::Accent | VisualColours::Custom(_) => self.peak,
            VisualColours::Cover => match self.cover {
                Some(colours) if self.dark => mix(across(colours, place), Color32::WHITE, 0.55),
                Some(colours) => mix(across(colours, place), Color32::BLACK, 0.35),
                None => self.peak,
            },
            VisualColours::Rainbow if self.dark => mix(rainbow(place), Color32::WHITE, 0.55),
            VisualColours::Rainbow => mix(rainbow(place), Color32::BLACK, 0.35),
            VisualColours::Winamp if self.dark => hex(0xd8d8d8),
            VisualColours::Fire if self.dark => hex(0xfff0c0),
            VisualColours::Winamp | VisualColours::Fire => self.text,
        }
    }
}

/// The rainbow's colour at `place` (0: red, 1: violet).
fn rainbow(place: f32) -> Color32 {
    let hue = place.clamp(0.0, 1.0) * 280.0;
    // HSV at a saturation of 0.8 and full value.
    let (s, v) = (0.8, 1.0);
    let c = v * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |u: f32| ((u + m) * 255.0).round() as u8;
    rgb(to(r), to(g), to(b))
}

/// The colour at `place` (0 to 1) of a gradient through `colours`.
fn across(colours: [Color32; 3], place: f32) -> Color32 {
    let place = place.clamp(0.0, 1.0);
    if place <= 0.5 {
        mix(colours[0], colours[1], place * 2.0)
    } else {
        mix(colours[1], colours[2], place * 2.0 - 1.0)
    }
}

/// The colour at `t` (0 to 1) of a gradient of `stops`.
fn along(stops: &[(f32, u32)], t: f32) -> Color32 {
    let i = stops
        .windows(2)
        .position(|w| t <= w[1].0)
        .unwrap_or(stops.len() - 2);
    let ((a, ca), (b, cb)) = (stops[i], stops[i + 1]);
    mix(hex(ca), hex(cb), ((t - a) / (b - a)).clamp(0.0, 1.0))
}

/// A view's icon in `own` colour (none: the accent in use), a shade
/// deeper on a light ground as the accent is.
pub fn icon_colour(own: Option<Accent>) -> Color32 {
    match own {
        None => accent(),
        Some(a) if current().dark => a.base(),
        Some(a) => mix(a.base(), Color32::BLACK, 0.12),
    }
}

/// The colours of `visual` in use: the cover's (`set_cover`) or the
/// visualization's own (`scheme`) when they are chosen.
pub fn visual_for(visual: Visual) -> VisualPalette {
    let mut v = with_own(current().visual, visual);
    if v.colours == VisualColours::Cover {
        v.cover = cover().map(|c| c.map(|c| readable(c, v.dark)));
    }
    v
}

/// `v` with `visual`'s own colours when they are chosen (also for the
/// settings' swatch).
pub fn with_own(mut v: VisualPalette, visual: Visual) -> VisualPalette {
    if v.colours == VisualColours::Own {
        v.own = Some(OwnShades::of(visual, v.dark));
    }
    v
}

/// The cover's colours as `visual` would show them, for the settings'
/// swatch: with no cover heard, a sample of three.
pub fn with_cover(mut v: VisualPalette) -> VisualPalette {
    const SAMPLE: [Color32; 3] = [hex(0xd0487a), hex(0x7a5cf0), hex(0x2fb2c4)];
    v.cover = Some(cover().unwrap_or(SAMPLE).map(|c| readable(c, v.dark)));
    v
}

/// The colours of the cover heard, as `cover_colours` found them (none:
/// no cover).
static COVER: RwLock<Option<[Color32; 3]>> = RwLock::new(None);

/// The cover heard has these colours from now on (`CoverFade` moves them).
pub fn set_cover(colours: Option<[Color32; 3]>) {
    if let Ok(mut cover) = COVER.write() {
        *cover = colours;
    }
}

fn cover() -> Option<[Color32; 3]> {
    COVER.read().ok().and_then(|c| *c)
}

/// `c` lightened or darkened until its luminance is within what reads on
/// the visualization's ground: a dark cover on a dark theme, a pale one on
/// a light theme are brought into it.
fn readable(c: Color32, dark: bool) -> Color32 {
    let (lo, hi) = if dark { (0.16, 0.55) } else { (0.06, 0.22) };
    let l = luminance(c);
    let (towards, bound) = if l < lo {
        (Color32::WHITE, lo)
    } else if l > hi {
        (Color32::BLACK, hi)
    } else {
        return c;
    };
    // The least mix that reaches the bound.
    let (mut a, mut b) = (0.0f32, 1.0f32);
    for _ in 0..14 {
        let t = (a + b) / 2.0;
        let m = luminance(mix(c, towards, t));
        let reached = if towards == Color32::WHITE {
            m >= bound
        } else {
            m <= bound
        };
        if reached {
            b = t;
        } else {
            a = t;
        }
    }
    mix(c, towards, b)
}

/// Up to three colours that stand for a cover of `pixels`, the one that
/// stands out most first: the pixels (about 4096 of them) counted in
/// buckets of 4 bits a channel, each bucket's mean weighed by its count,
/// its saturation and how far it is from black and white, and the best
/// ones taken that differ from those taken before. Two found: their mix
/// between them; one: it three times; none (a transparent picture): none.
pub fn cover_colours(pixels: &[Color32]) -> Option<[Color32; 3]> {
    const SAMPLES: usize = 4096;
    // How far apart (a weighted RGB distance) two colours taken must be,
    // and the least share of the picture a colour after the first needs.
    const APART: f32 = 90.0;
    const LEAST_SHARE: f32 = 0.01;
    let step = (pixels.len() / SAMPLES).max(1);
    let mut buckets = vec![[0u32; 4]; 4096];
    let mut counted = 0u32;
    for p in pixels.iter().step_by(step).filter(|p| p.a() >= 128) {
        let (r, g, b) = (p.r(), p.g(), p.b());
        let i = (usize::from(r >> 4) << 8) | (usize::from(g >> 4) << 4) | usize::from(b >> 4);
        let bucket = &mut buckets[i];
        bucket[0] += 1;
        bucket[1] += u32::from(r);
        bucket[2] += u32::from(g);
        bucket[3] += u32::from(b);
        counted += 1;
    }
    let mut weighed: Vec<(f32, u32, Color32)> = buckets
        .iter()
        .filter(|b| b[0] > 0)
        .map(|b| {
            let n = b[0];
            let c = rgb((b[1] / n) as u8, (b[2] / n) as u8, (b[3] / n) as u8);
            let (saturation, lightness) = saturation_lightness(c);
            let mid = (1.0 - (2.0 * lightness - 1.0).powi(2)).max(0.05);
            (n as f32 * (0.15 + saturation).powi(2) * mid, n, c)
        })
        .collect();
    weighed.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut taken: Vec<Color32> = Vec::with_capacity(3);
    for &(_, n, c) in &weighed {
        if !taken.is_empty() && (n as f32) < counted as f32 * LEAST_SHARE {
            continue;
        }
        if taken.iter().all(|&t| distance(t, c) > APART) {
            taken.push(c);
            if taken.len() == 3 {
                break;
            }
        }
    }
    match taken[..] {
        [] => None,
        [a] => Some([a, a, a]),
        [a, b] => Some([a, mix(a, b, 0.5), b]),
        [a, b, c, ..] => Some([a, b, c]),
    }
}

/// HSL's saturation and lightness of `c`, 0 to 1.
fn saturation_lightness(c: Color32) -> (f32, f32) {
    let [r, g, b] = [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let lightness = (max + min) / 2.0;
    let saturation = if max == min {
        0.0
    } else {
        (max - min) / (1.0 - (2.0 * lightness - 1.0).abs()).max(1e-6)
    };
    (saturation.min(1.0), lightness)
}

/// How far apart two colours look: RGB weighed by the red's mean
/// (redmean), 0 to about 765.
fn distance(a: Color32, b: Color32) -> f32 {
    let mean = (f32::from(a.r()) + f32::from(b.r())) / 2.0;
    let d = |x: u8, y: u8| f32::from(x) - f32::from(y);
    let (dr, dg, db) = (d(a.r(), b.r()), d(a.g(), b.g()), d(a.b(), b.b()));
    ((2.0 + mean / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - mean) / 256.0) * db * db)
        .sqrt()
}

/// The cover's colours moving to the next cover's over `FADE_S`, rather
/// than at once, and to the accent's when a track has none.
pub struct CoverFade {
    from: [Color32; 3],
    to: Option<[Color32; 3]>,
    shown: Option<[Color32; 3]>,
    since: std::time::Instant,
}

impl Default for CoverFade {
    fn default() -> Self {
        CoverFade {
            from: [accent(); 3],
            to: None,
            shown: None,
            // At rest: no way under way.
            since: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(1))
                .unwrap_or_else(std::time::Instant::now),
        }
    }
}

impl CoverFade {
    const FADE_S: f32 = 0.6;

    /// Moves on towards `target` (`Some(None)`: no cover; `None`: not
    /// known yet, the way goes on as it was) and hands the colours to
    /// `set_cover`; true while they are still moving.
    pub fn follow(&mut self, target: Option<Option<[Color32; 3]>>) -> bool {
        if let Some(target) = target
            && target != self.to
        {
            self.from = self.shown.unwrap_or([accent(); 3]);
            self.to = target;
            self.since = std::time::Instant::now();
        }
        let t = (self.since.elapsed().as_secs_f32() / Self::FADE_S).min(1.0);
        let shown = if t >= 1.0 {
            self.to
        } else {
            let ease = t * t * (3.0 - 2.0 * t);
            let to = self.to.unwrap_or([accent(); 3]);
            Some(std::array::from_fn(|i| mix(self.from[i], to[i], ease)))
        };
        if shown != self.shown {
            self.shown = shown;
            set_cover(shown);
        }
        t < 1.0
    }
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
    /// Black lists with steel-blue text, light grey text around them, a
    /// steel frame.
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
    pub visual: VisualPalette,
}

impl Palette {
    pub fn of(look: Look) -> Palette {
        let Look {
            theme,
            accent,
            selection,
            playing,
            visual,
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
                // Steel blue in the lists, light grey around them and on
                // the column headers.
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
        // The visualization's shades: the accent's, or the user's own.
        let (mut low, mut high, mut peak) = (black, black, black);
        if p.dark {
            p.accent = base;
            p.accent_hover = mix(base, white, 0.16);
            p.playing = mix(base, white, 0.45);
            p.selected_bg = mix(p.list_bg, base, 0.26);
            p.spectrum_low = mix(base, black, 0.22);
            p.spectrum_high = mix(base, white, 0.45);
            p.spectrum_peak = mix(p.text, base, 0.1);
            if let VisualColours::Custom(c) = visual {
                let own = hex(c);
                (low, high, peak) = (
                    mix(own, black, 0.22),
                    mix(own, white, 0.45),
                    mix(p.text, own, 0.1),
                );
            }
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
            if let VisualColours::Custom(c) = visual {
                let own = hex(c);
                (low, high, peak) = (
                    mix(own, black, 0.1),
                    mix(own, white, 0.25),
                    mix(p.text, own, 0.3),
                );
            }
        }
        if !matches!(visual, VisualColours::Custom(_)) {
            (low, high, peak) = (p.spectrum_low, p.spectrum_high, p.spectrum_peak);
        }
        p.visual = VisualPalette {
            colours: visual,
            low,
            high,
            peak,
            dark: p.dark,
            text: p.text,
            cover: None,
            own: None,
        };
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
        visual: VisualPalette {
            colours: VisualColours::Accent,
            low: Color32::BLACK,
            high: Color32::BLACK,
            peak: Color32::BLACK,
            dark: true,
            text: Color32::WHITE,
            cover: None,
            own: None,
        },
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
    fn visual_colours() {
        for c in VisualColours::ALL.into_iter().chain([VisualColours::Custom(0x12ab9f)]) {
            assert_eq!(VisualColours::from_name(&c.name()), Some(c));
        }
        assert_eq!(VisualColours::from(String::from("plaid")), VisualColours::Own);
        let of = |visual| Palette::of(Look { visual, ..Look::default() }).visual;
        // The rainbow: red at the bass, violet at the top; the place
        // matters, not for the others.
        let rainbow = of(VisualColours::Rainbow);
        let (bass, top) = (rainbow.at(1.0, 0.0), rainbow.at(1.0, 1.0));
        assert!(bass.r() > bass.g() && bass.r() > bass.b(), "{bass:?}");
        assert!(top.b() > top.g() && top.r() > top.g(), "{top:?}");
        // Winamp's: green at the floor, red at the top, by the level.
        let winamp = of(VisualColours::Winamp);
        assert!(winamp.by_level() && !rainbow.by_level());
        let (floor, loud) = (winamp.at(0.0, 0.3), winamp.at(1.0, 0.3));
        assert!(floor.g() > floor.r() && loud.r() > loud.g());
        assert_eq!(winamp.at(0.5, 0.0), winamp.at(0.5, 1.0));
        // The accent's are the spectrum's shades as before; one's own
        // colour's are mixed from it.
        let accent = of(VisualColours::Accent);
        let p = Palette::of(Look::default());
        assert_eq!(accent.at(0.0, 0.5), p.spectrum_low);
        assert_eq!(accent.at(1.0, 0.5), p.spectrum_high);
        let own = of(VisualColours::Custom(0x2f7cf0));
        let blue = own.at(1.0, 0.5);
        assert!(blue.b() > blue.r(), "{blue:?}");
        // The cover's: the accent's shades without a cover, its colours
        // across the place with one.
        let mut cover = of(VisualColours::Cover);
        assert_eq!(cover.at(1.0, 0.5), p.spectrum_high);
        cover.cover = Some([hex(0xc03030), hex(0x30c030), hex(0x3030c0)]);
        let (bass, top) = (cover.at(1.0, 0.0), cover.at(1.0, 1.0));
        assert!(bass.r() > bass.b() && top.b() > top.r(), "{bass:?} {top:?}");
        assert!(!cover.by_level());
    }

    /// Each visualization's own colours: a well-formed scheme, its own
    /// (no two alike), as written on a dark ground and readable on a
    /// light one; the fire and the spectrogram by the level.
    #[test]
    fn own_colours() {
        let dark = Palette::of(Look::default()).visual;
        let light = Palette::of(Look {
            theme: Theme::Light,
            ..Look::default()
        })
        .visual;
        assert_eq!(dark.colours, VisualColours::Own);
        let mut seen = Vec::new();
        for v in Visual::ALL {
            let s = scheme(v);
            assert!((2..=MOST_ROWS).contains(&s.rows.len()), "{v:?}");
            assert_eq!(s.rows[0].0, 0.0, "{v:?}");
            assert_eq!(s.rows[s.rows.len() - 1].0, 1.0, "{v:?}");
            assert!(s.rows.windows(2).all(|w| w[0].0 < w[1].0), "{v:?}");
            assert!(s.rows.iter().all(|r| (1..=MOST_ACROSS).contains(&r.1.len())));
            assert!((1..=MOST_ACROSS).contains(&s.peak.len()));
            let own = with_own(dark, v);
            let top = own.at(1.0, 0.0);
            assert_eq!(top, hex(s.rows[s.rows.len() - 1].1[0]), "{v:?}");
            if v != Visual::Off {
                let shades: Vec<_> = (0..=4).map(|i| own.at(i as f32 / 4.0, 0.3)).collect();
                assert!(!seen.contains(&shades), "{v:?} is like another");
                seen.push(shades);
            }
            // On a light ground nothing is paler than the cover's bound.
            let own = with_own(light, v);
            for i in 0..=4 {
                let c = own.at(i as f32 / 4.0, 0.5);
                assert!(luminance(c) <= 0.23, "{v:?} {c:?}");
            }
        }
        assert!(with_own(dark, Visual::Fire).by_level());
        assert!(with_own(dark, Visual::Spectrogram).by_level());
        assert!(!with_own(dark, Visual::Spectrum).by_level());
        // Between rows and across: the LED's yellow in its middle, the
        // notes' wheel red at C and violet towards B.
        let led = with_own(dark, Visual::Led);
        assert_eq!(led.at(0.7, 0.1), hex(0xffd21e));
        let notes = with_own(dark, Visual::Chroma);
        let (c, b) = (notes.at(1.0, 0.0), notes.at(1.0, 1.0));
        assert!(c.r() > c.g() && c.r() > c.b() && b.b() > b.g(), "{c:?} {b:?}");
        // Without a visualization named, the accent's shades.
        assert_eq!(dark.at(1.0, 0.5), Palette::of(Look::default()).spectrum_high);
    }

    /// A cover's colours: the vivid ones first, apart from one another,
    /// over a larger dull ground; a grey cover grey; a transparent one
    /// none.
    #[test]
    fn colours_of_a_cover() {
        let mut pixels = vec![hex(0x202020); 60 * 100];
        pixels.extend(vec![hex(0xd02020); 25 * 100]);
        pixels.extend(vec![hex(0x2050e0); 15 * 100]);
        let [a, b, c] = cover_colours(&pixels).unwrap();
        assert!(a.r() > 180 && a.g() < 60, "{a:?}");
        assert!(b.b() > 180 && b.r() < 60, "{b:?}");
        assert!(distance(c, a) > 90.0 && distance(c, b) > 90.0, "{c:?}");
        let grey = cover_colours(&vec![hex(0x808080); 1000]).unwrap();
        assert_eq!(grey, [hex(0x808080); 3]);
        // Two: their mix between them.
        let mut two = vec![hex(0xd02020); 500];
        two.extend(vec![hex(0x2050e0); 500]);
        let [a, m, b] = cover_colours(&two).unwrap();
        assert_eq!(m, mix(a, b, 0.5));
        assert_eq!(cover_colours(&vec![Color32::TRANSPARENT; 100]), None);
        assert_eq!(cover_colours(&[]), None);
    }

    /// Whatever the cover, its colours are brought to what reads on the
    /// ground: lighter on a dark theme, deeper on a light one.
    #[test]
    fn cover_colours_read_on_every_theme() {
        for theme in Theme::ALL {
            let p = Palette::of(Look {
                theme,
                ..Look::default()
            });
            for c in [0x000000, 0xffffff, 0xffff40, 0x101060, 0x808080, 0xe03030] {
                let r = readable(hex(c), p.dark);
                let l = luminance(r);
                let (lo, hi) = if p.dark { (0.16, 0.55) } else { (0.06, 0.22) };
                assert!(l >= lo - 0.01 && l <= hi + 0.01, "{theme:?} #{c:06x}: {l}");
                assert!(contrast(r, p.list_bg) >= 2.5, "{theme:?} #{c:06x}");
            }
            // One already within is kept.
            let fine = if p.dark { hex(0x4080d0) } else { hex(0x305080) };
            assert_eq!(readable(fine, p.dark), fine);
        }
    }

    #[test]
    fn cover_fade_moves_to_the_next_cover() {
        let mut fade = CoverFade::default();
        let red = [hex(0xd02020); 3];
        // Not known yet: nothing to move to.
        assert!(!fade.follow(None));
        assert!(fade.follow(Some(Some(red))));
        fade.since -= std::time::Duration::from_secs(1);
        assert!(!fade.follow(Some(Some(red))));
        assert_eq!(fade.shown, Some(red));
        // A track without a cover: back to the accent's, then none.
        assert!(fade.follow(Some(None)));
        fade.since -= std::time::Duration::from_secs(1);
        assert!(!fade.follow(None));
        assert_eq!(fade.shown, None);
    }

    #[test]
    fn the_default_is_dark_amber_its_shades_amber_too() {
        let p = Palette::of(Look {
            theme: Theme::Dark,
            accent: Accent::Amber,
            selection: Tint::Accent,
            playing: Tint::Accent,
            visual: VisualColours::Accent,
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
                    visual: VisualColours::Accent,
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
            visual: VisualColours::Accent,
        })
        .selected_bg;
        assert_eq!(
            grey,
            Palette::of(Look {
                theme: Theme::Dark,
                accent: Accent::Red,
                selection: Tint::Grey,
                playing: Tint::Accent,
                visual: VisualColours::Accent,
            })
            .selected_bg
        );
        assert_eq!(
            Palette::of(Look {
                theme: Theme::Light,
                accent: Accent::Amber,
                selection: Tint::Custom(0x2b3442),
                playing: Tint::Accent,
                visual: VisualColours::Accent,
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
                    visual: VisualColours::Accent,
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
