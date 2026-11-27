//! Pictures drawn by hand with no window and no GPU: egui's meshes
//! rasterised with their textures and vertex colours, blended as
//! premultiplied colours (in gamma space, near enough to what the app
//! shows). The text is laid out at the picture's own density. The track's
//! spectrum is saved as a PNG this way; in tests, the visualizations are
//! drawn into a PNG sheet of frames one under another to look at.

use std::collections::HashMap;

use egui::epaint::{ImageData, Primitive};
use egui::{Color32, Mesh, Pos2, Rect, TextureId, Vec2, vec2};

/// A frame's pixels, RGB from 0 to 1, row by row.
pub type Picture = Vec<[f32; 3]>;

pub struct Renderer {
    pub ctx: egui::Context,
    /// The textures egui made, as it hands them over.
    textures: HashMap<TextureId, ([usize; 2], Vec<Color32>)>,
    /// Pixels a point.
    scale: f32,
}

impl Renderer {
    pub fn new(scale: f32) -> Renderer {
        Renderer {
            ctx: egui::Context::default(),
            textures: HashMap::new(),
            scale,
        }
    }

    /// The size in pixels of a picture of `size` points.
    pub fn pixels(&self, size: Vec2) -> [usize; 2] {
        [
            (size.x * self.scale) as usize,
            (size.y * self.scale) as usize,
        ]
    }

    /// `paint` in `rect` (from the screen's top left) on `ground`.
    pub fn frame(
        &mut self,
        rect: Rect,
        ground: Color32,
        paint: impl FnOnce(&egui::Painter),
    ) -> Picture {
        self.frame_ui(rect, ground, |ui| paint(ui.painter()))
    }

    /// Widgets laid out by `show` in `rect` (from the screen's top left)
    /// on `ground`, as a window would show them.
    pub fn frame_ui(
        &mut self,
        rect: Rect,
        ground: Color32,
        show: impl FnOnce(&mut egui::Ui),
    ) -> Picture {
        let mut paint = Some(show);
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, rect.max.to_vec2())),
            ..Default::default()
        };
        // The fonts rasterised at the picture's pixels, not scaled up.
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(self.scale);
        let out = self.ctx.run_ui(input, |ui| {
            if let Some(show) = paint.take() {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                show(&mut child);
            }
        });
        let mut textures = out.textures_delta;
        for (id, deltas) in std::mem::take(&mut textures.set) {
            for delta in deltas {
                let ImageData::Color(image) = delta.image;
                match delta.pos {
                    None => {
                        self.textures
                            .insert(id, (image.size, image.pixels.clone()));
                    }
                    Some([x0, y0]) => {
                        if let Some((size, pixels)) = self.textures.get_mut(&id) {
                            for y in 0..image.size[1] {
                                for x in 0..image.size[0] {
                                    pixels[(y0 + y) * size[0] + x0 + x] =
                                        image.pixels[y * image.size[0] + x];
                                }
                            }
                        }
                    }
                }
            }
        }
        let prims = self.ctx.tessellate(out.shapes, self.scale);
        let [w, h] = self.pixels(rect.size());
        let g = ground.to_array().map(|c| c as f32 / 255.0);
        let mut picture = vec![[g[0], g[1], g[2]]; w * h];
        for p in &prims {
            if let Primitive::Mesh(m) = &p.primitive {
                let clip = p.clip_rect.intersect(rect);
                self.raster(&mut picture, rect, clip, m);
            }
        }
        // Freed after the painting, as a backend frees them (a texture
        // made and dropped within the frame is still painted).
        for id in &textures.free {
            self.textures.remove(id);
        }
        textures.clear();
        picture
    }

    /// A mesh's triangles blended into `picture` of `rect`, within `clip`.
    fn raster(&self, picture: &mut Picture, rect: Rect, clip: Rect, m: &Mesh) {
        let [w, h] = self.pixels(rect.size());
        let s = self.scale;
        let texture = self.textures.get(&m.texture_id);
        let at = |p: Pos2| (p - rect.min) * s;
        let (cx0, cy0) = (((clip.min.x - rect.min.x) * s).max(0.0), ((clip.min.y - rect.min.y) * s).max(0.0));
        let (cx1, cy1) = ((clip.max.x - rect.min.x) * s, (clip.max.y - rect.min.y) * s);
        for tri in m.indices.chunks(3) {
            let v = [0, 1, 2].map(|i| &m.vertices[tri[i] as usize]);
            let p = v.map(|v| at(v.pos));
            let area = (p[1] - p[0]).x * (p[2] - p[0]).y - (p[1] - p[0]).y * (p[2] - p[0]).x;
            if area.abs() < 1e-6 {
                continue;
            }
            let lo = p.iter().fold(vec2(f32::MAX, f32::MAX), |a, b| a.min(*b));
            let hi = p.iter().fold(vec2(f32::MIN, f32::MIN), |a, b| a.max(*b));
            let x0 = lo.x.max(cx0).max(0.0) as usize;
            let x1 = (hi.x.ceil().min(cx1).max(0.0) as usize).min(w);
            let y0 = lo.y.max(cy0).max(0.0) as usize;
            let y1 = (hi.y.ceil().min(cy1).max(0.0) as usize).min(h);
            let colour = |i: usize| v[i].color.to_array().map(|c| c as f32 / 255.0);
            let (ca, cb, cc) = (colour(0), colour(1), colour(2));
            for y in y0..y1 {
                for x in x0..x1 {
                    let q = vec2(x as f32 + 0.5, y as f32 + 0.5);
                    let edge = |a: Vec2, b: Vec2| (b - a).x * (q - a).y - (b - a).y * (q - a).x;
                    let l = [edge(p[1], p[2]), edge(p[2], p[0]), edge(p[0], p[1])].map(|e| e / area);
                    if l.iter().any(|&l| l < 0.0) {
                        continue;
                    }
                    let mut src: [f32; 4] =
                        std::array::from_fn(|j| ca[j] * l[0] + cb[j] * l[1] + cc[j] * l[2]);
                    if let Some((size, pixels)) = texture {
                        let uv = v[0].uv.to_vec2() * l[0] + v[1].uv.to_vec2() * l[1] + v[2].uv.to_vec2() * l[2];
                        let tx = ((uv.x * size[0] as f32) as usize).min(size[0] - 1);
                        let ty = ((uv.y * size[1] as f32) as usize).min(size[1] - 1);
                        let t = pixels[ty * size[0] + tx].to_array().map(|c| c as f32 / 255.0);
                        for j in 0..4 {
                            src[j] *= t[j];
                        }
                    }
                    let d = &mut picture[y * w + x];
                    for j in 0..3 {
                        d[j] = src[j] + d[j] * (1.0 - src[3]);
                    }
                }
            }
        }
    }
}

/// A picture's pixels as RGB bytes.
pub fn rgb8(picture: &Picture) -> Vec<u8> {
    picture
        .iter()
        .flat_map(|c| c.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect()
}

/// `frames` of `size` pixels one under another, as a PNG at `path`.
#[cfg(test)]
pub fn save(path: &str, frames: &[Picture], size: [usize; 2]) {
    let [w, h] = size;
    let sheet: Vec<u8> = frames.iter().flat_map(rgb8).collect();
    image::save_buffer(
        path,
        &sheet,
        w as u32,
        (h * frames.len()) as u32,
        image::ColorType::Rgb8,
    )
    .unwrap();
}
