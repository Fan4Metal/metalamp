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

/// The lightning bolt in icon units: a classic bolt, its top on the right
/// of the centre, its point at the bottom left, as wide as it is tall
/// allows.
const BOLT: [(f32, f32); 7] = [
    (0.400, 0.030),
    (0.775, 0.030),
    (0.600, 0.385),
    (0.830, 0.385),
    (0.300, 0.970),
    (0.425, 0.555),
    (0.190, 0.555),
];

/// The colour of the icon at `(x, y)`, both in 0..1 of the icon's side;
/// `px` is the size of a pixel of the layer in the same units (1/16 for the
/// 16 px layer). A yellow lightning bolt, light at the top and amber at the
/// bottom, with a darker edge (so it keeps its shape on a light taskbar), a
/// light upper-left side and a soft shadow; layers of 24 px and less have a
/// thinner edge and no light side.
fn sample(x: f32, y: f32, px: f32) -> Rgba {
    let small = px > 1.0 / 40.0;
    let mut c = [0.0; 4];

    // The shadow, a little lower and to the right.
    let shadow = polygon(x - 0.015, y - 0.03, &BOLT);
    c = over(
        c,
        with_alpha(hex(0x000000), 0.35 * smoothstep(0.035, -0.02, shadow)),
    );

    let bolt = polygon(x, y, &BOLT);
    let edge = if small { px * 0.9 } else { 0.02 };
    c = over(c, with_alpha(hex(0xa85400), cover(bolt, px)));
    let fill = mix(hex(0xfff27a), hex(0xff9a00), (y - 0.03) / 0.94);
    c = over(c, with_alpha(fill, cover(bolt + edge, px)));
    if !small {
        // The upper-left side lit: lighter left of a line along the bolt.
        let lit = smoothstep(0.02, -0.02, x - 0.53 + (y - 0.03) * 0.25);
        c = over(
            c,
            with_alpha(hex(0xffffff), 0.28 * lit * cover(bolt + edge, px)),
        );
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
