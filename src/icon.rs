//! Procedurally drawn application icon. Dependency-free so that `build.rs`
//! can include this file (`#[path = "src/icon.rs"]`) to produce the `.ico`
//! embedded in the executable and the raw RGBA the app shows (window icon,
//! start page, About), so that nothing is rasterised at start-up.
//!
//! Replace `sample` with the app's own drawing; keep `rgba`, `ico` and
//! `ICO_SIZES`, which build.rs, `--export-icon` and `ui::app_icon` rely on.

// build.rs and the app use different parts.
#![allow(dead_code)]

/// Sizes in the `.ico` of the executable.
pub const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// Straight RGBA, channels 0..1.
type Rgba = [f32; 4];

fn hex(c: u32) -> Rgba {
    let ch = |s: u32| ((c >> s) & 0xff) as f32 / 255.0;
    [ch(16), ch(8), ch(0), 1.0]
}

fn with_alpha(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], c[3] * a.clamp(0.0, 1.0)]
}

/// Paint `top` over `under`.
fn over(under: Rgba, top: Rgba) -> Rgba {
    let a = top[3] + under[3] * (1.0 - top[3]);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0, 0.0, 0.0, a];
    for i in 0..3 {
        out[i] = (top[i] * top[3] + under[i] * under[3] * (1.0 - top[3])) / a;
    }
    out
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Signed distance from `(x, y)` to a box from `min` to `max` with corners
/// rounded by `r` (negative inside).
fn rounded_box(x: f32, y: f32, min: (f32, f32), max: (f32, f32), r: f32) -> f32 {
    let (cx, cy) = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
    let (hx, hy) = ((max.0 - min.0) / 2.0 - r, (max.1 - min.1) / 2.0 - r);
    let qx = (x - cx).abs() - hx;
    let qy = (y - cy).abs() - hy;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Coverage of a shape with signed distance `d` (negative inside), the edge
/// smoothed over about one pixel of the layer being drawn.
fn cover(d: f32, px: f32) -> f32 {
    smoothstep(px * 0.6, -px * 0.6, d)
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

/// Signed distance to a circle.
fn circle(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r
}

/// Smooth union of two distances (`k`: the width of the blend).
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

/// Signed distance to a closed polygon (negative inside).
fn polygon(x: f32, y: f32, v: &[(f32, f32)]) -> f32 {
    let mut d = f32::MAX;
    let mut inside = false;
    let mut j = v.len() - 1;
    for i in 0..v.len() {
        let (ax, ay) = v[i];
        let (bx, by) = v[j];
        let (ex, ey) = (bx - ax, by - ay);
        let (wx, wy) = (x - ax, y - ay);
        let t = ((wx * ex + wy * ey) / (ex * ex + ey * ey)).clamp(0.0, 1.0);
        d = d.min(((wx - ex * t).powi(2) + (wy - ey * t).powi(2)).sqrt());
        if (ay > y) != (by > y) && x < ax + (bx - ax) * (y - ay) / (by - ay) {
            inside = !inside;
        }
        j = i;
    }
    if inside { -d } else { d }
}

/// The lightning bolt in screen units: a classic bolt with its top on the
/// right, the guitar's neck crossing it in the middle.
const BOLT: [(f32, f32); 7] = [
    (0.545, 0.035),
    (0.835, 0.035),
    (0.675, 0.385),
    (0.865, 0.385),
    (0.475, 0.965),
    (0.580, 0.540),
    (0.395, 0.540),
];

/// The colour of the icon at `(x, y)`, both in 0..1 of the icon's side;
/// `px` is the size of a pixel of the layer in the same units (1/16 for the
/// 16 px layer). An electric guitar (a single-cutaway body in cherry red, a
/// dark rosewood neck, a black headstock) lying from the bottom left to the
/// top right, over a yellow lightning bolt; lit from the top, with a soft
/// shadow. Layers of 24 px and less drop the small parts (strings, frets,
/// knobs, tuners, binding), which would only blur there, and get a wider
/// neck.
fn sample(x: f32, y: f32, px: f32) -> Rgba {
    let small = px > 1.0 / 40.0;
    // The guitar's own axes: `u` from the body's centre to the headstock,
    // `v` across it.
    let (ox, oy) = (0.355, 0.645);
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let (dx, dy) = (x - ox, y - oy);
    let u = (dx - dy) * s;
    let v = (dx + dy) * s;

    let mut c = [0.0; 4];

    // The bolt, behind the guitar: amber below, yellow above, a darker edge.
    let bolt = polygon(x, y, &BOLT);
    let bolt_fill = mix(hex(0xffe45c), hex(0xffa000), (y - 0.03) / 0.94);
    let edge = if small { px * 0.9 } else { 0.012 };
    c = over(c, with_alpha(hex(0xb86800), cover(bolt, px)));
    c = over(c, with_alpha(bolt_fill, cover(bolt + edge, px)));

    // The guitar's distance fields.
    let lower = circle(u, v, -0.075, 0.0, 0.205);
    let upper = circle(u, v, 0.115, 0.0, 0.155);
    let cutaway = circle(u, v, 0.215, -0.125, 0.085);
    let body = smin(lower, upper, 0.06).max(-cutaway);
    let half = if small {
        0.04_f32.max(px * 0.75)
    } else {
        0.033
    };
    let neck = (u - 0.375).abs().max(v.abs() - half + (u - 0.14) * 0.02) - 0.215;
    let neck = neck.max(v.abs() - half);
    let head = rounded_box(u, v, (0.565, -0.06), (0.715, 0.06), 0.03);
    let guitar = body.min(neck).min(head);

    // Soft shadow under the whole guitar, a little lower.
    let (sx, sy) = (x, y - 0.025);
    let (sdx, sdy) = (sx - ox, sy - oy);
    let (su, sv) = ((sdx - sdy) * s, (sdx + sdy) * s);
    let shadow_body = smin(
        circle(su, sv, -0.075, 0.0, 0.205),
        circle(su, sv, 0.115, 0.0, 0.155),
        0.06,
    );
    let shadow = shadow_body.min(rounded_box(su, sv, (0.15, -0.06), (0.715, 0.06), 0.03));
    c = over(
        c,
        with_alpha(hex(0x000000), 0.32 * smoothstep(0.03, -0.02, shadow)),
    );
    let _ = guitar;

    // The neck and the headstock.
    let wood = mix(hex(0x5a3a26), hex(0x2e1c12), (v + half) / (2.0 * half));
    c = over(c, with_alpha(wood, cover(neck, px)));
    if !small {
        // Frets: thin light bars across the neck.
        let fret = ((u - 0.17) / 0.055).fract();
        if (0.17..0.56).contains(&u) && fret < 0.09 {
            c = over(c, with_alpha(hex(0xc9c2b8), 0.8 * cover(neck + 0.004, px)));
        }
    }
    let head_fill = mix(hex(0x3a3b42), hex(0x16171b), (v + 0.06) / 0.12);
    c = over(c, with_alpha(head_fill, cover(head, px)));
    // A light edge on the headstock keeps it visible on a dark taskbar.
    c = over(
        c,
        with_alpha(hex(0x8a8d96), 0.7 * cover(head.abs() - px * 0.45, px)),
    );
    if !small {
        for (tu, tv) in [(0.60, -0.075), (0.65, -0.075), (0.60, 0.075), (0.65, 0.075)] {
            c = over(
                c,
                with_alpha(hex(0xd8d8dc), cover(circle(u, v, tu, tv, 0.016), px)),
            );
        }
    }

    // The body: cherry red lit from the top, darker at its edge.
    let red = mix(hex(0xe8423a), hex(0x8c1218), (y - 0.38) / 0.5);
    let red = mix(red, hex(0x5a0a0e), smoothstep(-0.05, 0.0, body) * 0.7);
    c = over(c, with_alpha(red, cover(body, px)));
    if !small {
        // Cream binding along the edge.
        let binding = (body + 0.008).abs() - 0.004;
        c = over(c, with_alpha(hex(0xf2e6c8), 0.9 * cover(binding, px)));
        // A sheen on the upper part of the body.
        let sheen = circle(u, v, 0.02, -0.07, 0.11);
        c = over(
            c,
            with_alpha(
                hex(0xffffff),
                0.16 * smoothstep(0.05, -0.05, sheen) * cover(body + 0.02, px),
            ),
        );
    }

    // Pickups: black with a cream frame.
    for pu in [0.075_f32, -0.055] {
        let pickup = rounded_box(u, v, (pu - 0.032, -0.085), (pu + 0.032, 0.085), 0.012);
        if small {
            c = over(c, with_alpha(hex(0x16171b), cover(pickup, px)));
        } else {
            c = over(c, with_alpha(hex(0xe9dfc6), cover(pickup, px)));
            c = over(c, with_alpha(hex(0x16171b), cover(pickup + 0.008, px)));
        }
    }
    if !small {
        // Bridge and tailpiece.
        for (bu, w) in [(-0.125_f32, 0.07_f32), (-0.175, 0.075)] {
            let bar = rounded_box(u, v, (bu - 0.009, -w), (bu + 0.009, w), 0.006);
            c = over(c, with_alpha(hex(0xc8cad0), cover(bar, px)));
        }
        // Knobs on the treble side of the lower bout.
        for (ku, kv) in [(-0.10_f32, 0.135_f32), (-0.175, 0.095), (-0.02, 0.15)] {
            c = over(
                c,
                with_alpha(hex(0xe0b54a), cover(circle(u, v, ku, kv, 0.02), px)),
            );
        }
        // Strings from the tailpiece to the headstock.
        if (-0.175..0.6).contains(&u) {
            for sv in [-0.024_f32, -0.008, 0.008, 0.024] {
                let string = (v - sv).abs() - 0.0018;
                c = over(c, with_alpha(hex(0xeceef2), 0.85 * cover(string, px)));
            }
        }
    }
    c
}

/// Straight (non-premultiplied) RGBA pixels, `size * size * 4` bytes, row
/// major from the top. 4x4 supersampling gives smooth edges.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    for py in 0..size {
        for px in 0..size {
            // Averaged premultiplied, so transparent samples add no colour.
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    let c = sample(x, y, 1.0 / size as f32);
                    for i in 0..3 {
                        acc[i] += c[i] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let count = (SS * SS) as f32;
            let alpha = acc[3] / count;
            let i = (py as usize * n + px as usize) * 4;
            if alpha > 0.0 {
                for ch in 0..3 {
                    out[i + ch] = (acc[ch] / acc[3] * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                out[i + 3] = (alpha * 255.0).round() as u8;
            }
        }
    }
    out
}

/// Encode the icon as a multi-resolution `.ico` of 32-bit BMP layers
/// (`--export-icon`); `build.rs` stores the 256 px layer as PNG instead.
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let layers: Vec<(u32, Vec<u8>)> = sizes.iter().map(|&s| (s, bmp(&rgba(s), s))).collect();
    ico_of(&layers)
}

/// An `.ico` of `layers`: each a side and its image (a BMP from [`bmp`] or
/// a PNG file).
pub fn ico_of(layers: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type = icon
    out.extend_from_slice(&(layers.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * layers.len() as u32;
    for (s, img) in layers {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for (_, img) in layers {
        out.extend_from_slice(img);
    }
    out
}

/// Straight RGBA `px` of `size` x `size` as an icon layer:
/// BITMAPINFOHEADER + bottom-up BGRA pixels + 1-bpp AND mask.
pub fn bmp(px: &[u8], size: u32) -> Vec<u8> {
    let n = size as usize;
    let mask_stride = n.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + n * n * 4 + mask_stride * n);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(2 * size as i32).to_le_bytes()); // XOR + AND
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]); // compression .. colours important
    for y in (0..n).rev() {
        for x in 0..n {
            let i = (y * n + x) * 4;
            out.extend_from_slice(&[px[i + 2], px[i + 1], px[i], px[i + 3]]);
        }
    }
    // Alpha channel carries transparency; an all-zero AND mask is correct.
    out.resize(out.len() + mask_stride * n, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shape() {
        let px = rgba(64);
        assert_eq!(px.len(), 64 * 64 * 4);
        // The corner is transparent, the centre opaque.
        assert_eq!(px[3], 0);
        let c = ((32 * 64) + 32) * 4;
        assert_eq!(px[c + 3], 255);
        let ico = ico(&[16, 32]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
    }
}
