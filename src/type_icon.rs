//! Icons of the registered file types (`filetypes::FILE_TYPES`), qview's
//! design in GOM Player's colours: a white page with a folded corner, a
//! screen in the format's light colour with a white Play (video), note
//! (audio) or list with a Play (playlist), and across the page a band in its dark colour with the
//! extension in a pixel font. Dependency-free, drawn at every size (not
//! scaled), so that the letters stay sharp at 16 x 16: they sit on whole
//! pixels at a whole scale, while the shapes are supersampled.

#![allow(dead_code)]

use crate::filetypes::{FileType, Kind};

/// 3 or more columns by 5 rows, `#` for ink.
fn glyph(c: char) -> &'static [&'static str; 5] {
    match c {
        'A' => &[".#.", "#.#", "###", "#.#", "#.#"],
        'B' => &["##.", "#.#", "###", "#.#", "##."],
        'C' => &["###", "#..", "#..", "#..", "###"],
        'D' => &["##.", "#.#", "#.#", "#.#", "##."],
        'E' => &["###", "#..", "##.", "#..", "###"],
        'F' => &["###", "#..", "##.", "#..", "#.."],
        'G' => &["###", "#..", "#.#", "#.#", "###"],
        'I' => &["###", ".#.", ".#.", ".#.", "###"],
        'K' => &["#.#", "#.#", "##.", "#.#", "#.#"],
        'L' => &["#..", "#..", "#..", "#..", "###"],
        'M' => &["#...#", "##.##", "#.#.#", "#...#", "#...#"],
        'O' => &["###", "#.#", "#.#", "#.#", "###"],
        'P' => &["###", "#.#", "###", "#..", "#.."],
        'R' => &["##.", "#.#", "##.", "#.#", "#.#"],
        'S' => &["###", "#..", "###", "..#", "###"],
        'T' => &["###", ".#.", ".#.", ".#.", ".#."],
        'U' => &["#.#", "#.#", "#.#", "#.#", "###"],
        'V' => &["#.#", "#.#", "#.#", "#.#", ".#."],
        'W' => &["#...#", "#...#", "#.#.#", "##.##", "#...#"],
        '3' => &["###", "..#", ".##", "..#", "###"],
        '4' => &["#.#", "#.#", "###", "..#", "..#"],
        // Not a letter of any label (tested).
        _ => &["###", "###", "###", "###", "###"],
    }
}

/// Width of `text` in font pixels (one column between letters).
fn text_width(text: &str) -> u32 {
    let letters: u32 = text.chars().map(|c| glyph(c)[0].len() as u32).sum();
    letters + text.chars().count().saturating_sub(1) as u32
}

/// Whether font pixel `(x, y)` of `text` is ink.
fn text_ink(text: &str, x: u32, y: u32) -> bool {
    let mut left = 0;
    for c in text.chars() {
        let rows = glyph(c);
        let w = rows[0].len() as u32;
        if x >= left && x < left + w {
            return rows[y as usize].as_bytes()[(x - left) as usize] == b'#';
        }
        left += w + 1;
    }
    false
}

type Rgba = [f32; 4];

fn rgb(c: [u8; 3]) -> Rgba {
    [c[0] as f32, c[1] as f32, c[2] as f32, 255.0]
}

fn shade(c: [u8; 3], f: f32) -> Rgba {
    [c[0] as f32 * f, c[1] as f32 * f, c[2] as f32 * f, 255.0]
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

const WHITE: Rgba = [255.0, 255.0, 255.0, 255.0];

/// Paint `top` over `under`.
fn over(under: Rgba, top: Rgba) -> Rgba {
    let a = top[3] / 255.0;
    let out_a = a + under[3] / 255.0 * (1.0 - a);
    if out_a <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0; 4];
    for i in 0..3 {
        out[i] = (top[i] * a + under[i] * (under[3] / 255.0) * (1.0 - a)) / out_a;
    }
    out[3] = out_a * 255.0;
    out
}

fn inside_rounded(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// Whether `(x, y)` is inside the triangle `a`, `b`, `c`.
fn inside_triangle(x: f32, y: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
    let (s0, s1, s2) = (side(a, b), side(b, c), side(c, a));
    (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0)
}

/// Geometry of the icon at `n` x `n` pixels.
struct Layout {
    n: f32,
    /// The page, and the size of its folded corner.
    page: (f32, f32, f32, f32),
    fold: f32,
    /// Outline width.
    line: f32,
    /// The band, in whole pixels.
    band: (u32, u32, u32, u32),
    /// Font scale and the top left corner of the text, in whole pixels.
    scale: u32,
    text: String,
    text_at: (u32, u32),
    /// The screen above the band, if there is room for it.
    picture: Option<(f32, f32, f32, f32)>,
}

fn layout(t: &FileType, n: u32) -> Layout {
    let nf = n as f32;
    // A label too wide for its size is drawn smaller; at the smallest sizes
    // it loses its last letters.
    let fits = |text: &str, scale: u32| text_width(text) * scale <= n - 2;
    let mut text: String = t.label.into();
    let mut scale = (n / 16).max(1);
    while scale > 1 && !fits(&text, scale) {
        scale -= 1;
    }
    while !fits(&text, scale) && text.len() > 3 {
        text.pop();
    }
    let pad = scale.max(1);
    let band_h = 5 * scale + 2 * pad;
    // Lower than qview's (0.62): the screen above it gets the room.
    let band_y = ((nf * 0.70) as u32)
        .saturating_sub(band_h / 2)
        .min(n - band_h - 1);
    let text_w = text_width(&text) * scale;
    let band_w = (text_w + 2 * pad.max(2)).max((nf * 0.8) as u32).min(n);
    let band_x = (n - band_w) / 2;
    let page = (nf * 0.17, nf * 0.04, nf * 0.83, nf * 0.96);
    let line = (nf / 32.0).max(1.0);
    let picture = (n >= 32).then(|| {
        let m = nf * 0.1;
        (
            page.0 + m,
            page.1 + nf * 0.12,
            page.2 - m,
            band_y as f32 - nf * 0.05,
        )
    });
    Layout {
        n: nf,
        page,
        fold: nf * 0.2,
        line,
        band: (band_x, band_y, band_x + band_w, band_y + band_h),
        scale,
        text_at: (band_x + (band_w - text_w) / 2, band_y + pad),
        text,
        picture,
    }
}

/// Colour of a sample point of the shapes (page, fold, screen, band).
fn shapes(t: &FileType, l: &Layout, x: f32, y: f32) -> Rgba {
    let mut c = [0.0; 4];
    let (x0, y0, x1, y1) = l.page;
    // The page without its top right corner, cut along the diagonal.
    let cut = x - (x1 - l.fold) + (y0 + l.fold - y);
    let on_page = inside_rounded(x, y, l.page, l.n * 0.03) && cut <= l.fold;
    if on_page {
        let inner = (x0 + l.line, y0 + l.line, x1 - l.line, y1 - l.line);
        let edge = !inside_rounded(x, y, inner, l.n * 0.02) || cut > l.fold - l.line * 1.4;
        c = if edge {
            rgb([0x80, 0x84, 0x88])
        } else {
            rgb([0xfb, 0xfb, 0xfb])
        };
        // The fold: a small triangle below the cut.
        let fx = x - (x1 - l.fold);
        let fy = y - y0;
        if fx >= 0.0 && fy <= l.fold && fx <= fy + 0.001 && cut <= l.fold {
            let border = fx < l.line || fy > l.fold - l.line;
            c = if border {
                rgb([0x80, 0x84, 0x88])
            } else {
                rgb([0xd8, 0xdc, 0xe0])
            };
        }
        if let Some(r) = l.picture
            && inside_rounded(x, y, r, l.n * 0.025)
        {
            c = picture(t, x, y, r);
        }
    }
    let (bx0, by0, bx1, by1) = l.band;
    let band = (bx0 as f32, by0 as f32, bx1 as f32, by1 as f32);
    if inside_rounded(x, y, band, l.n * 0.03) {
        let inner = (
            band.0 + l.line,
            band.1 + l.line,
            band.2 - l.line,
            band.3 - l.line,
        );
        c = if inside_rounded(x, y, inner, l.n * 0.02) {
            rgb(t.colors.band)
        } else {
            shade(t.colors.band, 0.7)
        };
    }
    c
}

/// The screen in `r`: the type's light colour, lighter at the top, with a
/// white Play or note in the middle.
fn picture(t: &FileType, x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32)) -> Rgba {
    let (w, h) = (x1 - x0, y1 - y0);
    let v = (y - y0) / h;
    let ground = mix(
        mix(rgb(t.colors.light), WHITE, 0.25),
        shade(t.colors.light, 0.88),
        v,
    );
    // The motif in a square of the screen's height, centred.
    let s = h * 0.78;
    let (cx, cy) = (x0 + w / 2.0, y0 + h / 2.0);
    let (u, v) = ((x - cx) / s, (y - cy) / s);
    let ink = match t.kind {
        // Moved right a little: exactly centred, Play looks shifted left.
        Kind::Video => inside_triangle(u, v, (-0.28, -0.36), (0.40, 0.0), (-0.28, 0.36)),
        Kind::Audio => {
            // An eighth note: a slanted head, a stem and a flag.
            let (hx, hy) = (u + 0.12, v - 0.27);
            let (rx, ry) = (hx * 0.92 + hy * 0.4, hy * 0.92 - hx * 0.4);
            let head = (rx / 0.2).powi(2) + (ry / 0.14).powi(2) <= 1.0;
            let stem = (0.04..=0.12).contains(&u) && (-0.42..=0.25).contains(&v);
            let flag = (0.04..=0.36).contains(&u)
                && v >= -0.42 + (u - 0.04) * 0.55
                && v <= -0.22 + (u - 0.04) * 0.75;
            head || stem || flag
        }
        Kind::Playlist => {
            // Three lines of a list, a small Play before the first.
            let line = |v0: f32, left: f32| (left..=0.42).contains(&u) && (v - v0).abs() <= 0.075;
            let play = inside_triangle(u, v, (-0.42, -0.4), (-0.16, -0.27), (-0.42, -0.14));
            play || line(-0.27, -0.06) || line(0.0, -0.42) || line(0.27, -0.42)
        }
    };
    if ink { WHITE } else { ground }
}

/// Straight RGBA pixels of the icon of `t`, `n * n * 4` bytes.
pub fn rgba(t: &FileType, n: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let l = layout(t, n);
    let mut out = vec![0u8; (n * n * 4) as usize];
    for py in 0..n {
        for px in 0..n {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let c = shapes(
                        t,
                        &l,
                        px as f32 + (sx as f32 + 0.5) / SS as f32,
                        py as f32 + (sy as f32 + 0.5) / SS as f32,
                    );
                    // Premultiplied average.
                    for i in 0..3 {
                        acc[i] += c[i] * c[3] / 255.0;
                    }
                    acc[3] += c[3];
                }
            }
            let a = acc[3] / (SS * SS) as f32;
            let mut c = if a > 0.0 {
                [
                    acc[0] / acc[3] * 255.0,
                    acc[1] / acc[3] * 255.0,
                    acc[2] / acc[3] * 255.0,
                    a,
                ]
            } else {
                [0.0; 4]
            };
            // Letters on whole pixels, not supersampled.
            let (tx, ty) = l.text_at;
            if px >= tx && py >= ty {
                let (fx, fy) = ((px - tx) / l.scale, (py - ty) / l.scale);
                if fy < 5 && fx < text_width(&l.text) && text_ink(&l.text, fx, fy) {
                    c = over(c, WHITE);
                }
            }
            let i = ((py * n + px) * 4) as usize;
            for k in 0..4 {
                out[i + k] = c[k].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filetypes::FILE_TYPES;

    #[test]
    fn labels_fit_and_letters_are_known() {
        for t in FILE_TYPES {
            for n in [16, 20, 24, 32, 48, 256] {
                let l = layout(t, n);
                assert!(l.band.2 <= n && l.band.3 < n, "{} at {n}", t.label);
                assert!(
                    l.text_at.0 + text_width(&l.text) * l.scale <= l.band.2,
                    "{} at {n}",
                    t.label
                );
            }
            for c in t.label.chars() {
                assert_ne!(glyph(c), glyph('?'), "no glyph for {c} ({})", t.label);
            }
        }
    }

    #[test]
    fn letters_are_sharp_at_16() {
        // "MKV": the top left pixel of M is ink, the one right of it is not.
        let t = FILE_TYPES.iter().find(|t| t.id == "mkv").unwrap();
        let px = rgba(t, 16);
        let l = layout(t, 16);
        let at = |x: u32, y: u32| {
            let i = ((y * 16 + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2]]
        };
        let (tx, ty) = l.text_at;
        assert_eq!(at(tx, ty), [0xff, 0xff, 0xff]);
        assert_eq!(at(tx + 1, ty + 1), [0xff, 0xff, 0xff]);
        assert_eq!(at(tx + 2, ty), t.colors.band);
    }

    /// Writes the icons of all types, 96 pixels each in rows of eight on a
    /// transparent background, into the PNG named by
    /// `METALMPV_TYPES_IMAGE`: `images/file_types.png` of the README.
    #[test]
    #[ignore]
    fn readme_image() {
        let path = std::env::var("METALMPV_TYPES_IMAGE").expect("METALMPV_TYPES_IMAGE");
        let (n, gap, columns) = (96u32, 16u32, 8u32);
        let rows = (FILE_TYPES.len() as u32).div_ceil(columns);
        let mut out = image::RgbaImage::new(
            columns * n + (columns - 1) * gap,
            rows * n + (rows - 1) * gap,
        );
        for (i, t) in FILE_TYPES.iter().enumerate() {
            let icon = image::RgbaImage::from_raw(n, n, rgba(t, n)).unwrap();
            let (col, row) = (i as u32 % columns, i as u32 / columns);
            let (x, y) = (col * (n + gap), row * (n + gap));
            image::imageops::overlay(&mut out, &icon, x as i64, y as i64);
        }
        out.save(path).unwrap();
    }

    /// Writes the icons of all types at all sizes on a light and a dark
    /// strip into the PNG named by `METALMPV_ICON_SHEET`, for a look at the
    /// design.
    #[test]
    #[ignore]
    fn preview_sheet() {
        let path = std::env::var("METALMPV_ICON_SHEET").expect("METALMPV_ICON_SHEET");
        let sizes = [16u32, 24, 32, 48, 96];
        let gap = 8u32;
        // One type's icons on a light and on a dark ground.
        let half = sizes.iter().map(|n| n + gap).sum::<u32>() + gap;
        let row = sizes[sizes.len() - 1] + gap;
        let rows = FILE_TYPES.len().div_ceil(2) as u32;
        let mut sheet = image::RgbaImage::from_fn(half * 4, rows * row + gap, |x, _| {
            if (x / half).is_multiple_of(2) {
                image::Rgba([0xf0, 0xf0, 0xf0, 255])
            } else {
                image::Rgba([0x2b, 0x2b, 0x2b, 255])
            }
        });
        for (i, t) in FILE_TYPES.iter().enumerate() {
            let (column, line) = (i as u32 / rows, i as u32 % rows);
            for ground in 0..2 {
                let mut x = (column * 2 + ground) * half + gap;
                for &n in &sizes {
                    let icon = image::RgbaImage::from_raw(n, n, rgba(t, n)).unwrap();
                    let y = line * row + gap;
                    image::imageops::overlay(&mut sheet, &icon, x as i64, y as i64);
                    x += n + gap;
                }
            }
        }
        sheet.save(path).unwrap();
    }
}
