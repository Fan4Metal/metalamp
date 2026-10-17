//! Album covers for the album grid and the player bar.
//!
//! A cover is a 256-pixel thumbnail kept as JPEG in `covers\` next to the
//! settings, named by a hash of the album and of its first file's time, so
//! a changed file gets a new one. Without a thumbnail, the picture comes
//! from the album's folder (`folder.jpg`, `cover.jpg`, …: one small file,
//! the quickest on a hard disk) or else from the first track's tag
//! (metalmpv's `cover.rs`). An album without either gets an empty `.none`
//! file, so it is not looked for again.
//!
//! One worker thread does it all, the album asked for last first: the
//! cells in sight are asked for each frame, so they come before those
//! scrolled past. The UI keeps at most [`MAX_TEXTURES`] textures, the ones
//! shown longest ago dropped first.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

use crate::library::{Album, Library};

/// Thumbnails' longest side in pixels.
const SIDE: u32 = 256;
/// Textures kept: 256 KB each, 64 MB at most.
const MAX_TEXTURES: usize = 256;
/// A picture larger than this is not decoded (a bomb, or a scan of a
/// booklet).
const MAX_DECODED_SIDE: u32 = 8192;
const MAX_DECODED_BYTES: u64 = 256 << 20;
/// Pictures in an album's folder taken as its cover, by stem, in order.
const FOLDER_NAMES: &[&str] = &["folder", "cover", "front", "albumart", "albumartsmall"];

/// The cover of an album: its key in the cache.
pub type CoverKey = u64;

/// What the worker needs to find an album's cover.
struct Job {
    key: CoverKey,
    folder: PathBuf,
    /// The album's first track with a picture in its tag.
    tagged: Option<PathBuf>,
}

struct Queue {
    jobs: Mutex<Vec<Job>>,
    ready: Condvar,
}

enum Found {
    Image(CoverKey, egui::ColorImage),
    Missing(CoverKey),
}

pub struct Covers {
    queue: Arc<Queue>,
    rx: Receiver<Found>,
    textures: HashMap<CoverKey, (egui::TextureHandle, u64)>,
    /// Asked for and not answered yet.
    asked: HashSet<CoverKey>,
    /// Albums without a cover.
    missing: HashSet<CoverKey>,
}

impl Covers {
    pub fn new(ctx: &egui::Context) -> Covers {
        let queue = Arc::new(Queue {
            jobs: Mutex::new(Vec::new()),
            ready: Condvar::new(),
        });
        let (tx, rx) = channel();
        let worker = Arc::clone(&queue);
        let ctx = ctx.clone();
        let dir = crate::settings::data_dir().map(|d| d.join("covers"));
        let spawned = std::thread::Builder::new()
            .name("covers".into())
            .spawn(move || work(&worker, &tx, dir.as_deref(), &ctx));
        if let Err(e) = spawned {
            log::warn!("cannot start the covers' thread: {e}");
        }
        Covers {
            queue,
            rx,
            textures: HashMap::new(),
            asked: HashSet::new(),
            missing: HashSet::new(),
        }
    }

    /// Takes the worker's results as textures; drops those shown longest
    /// ago past the limit.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let frame = ctx.cumulative_frame_nr();
        for found in self.rx.try_iter() {
            match found {
                Found::Image(key, image) => {
                    self.asked.remove(&key);
                    let texture = ctx.load_texture(
                        format!("cover {key:x}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(key, (texture, frame));
                }
                Found::Missing(key) => {
                    self.asked.remove(&key);
                    self.missing.insert(key);
                }
            }
        }
        if self.textures.len() > MAX_TEXTURES {
            let mut by_age: Vec<(u64, CoverKey)> =
                self.textures.iter().map(|(k, (_, f))| (*f, *k)).collect();
            by_age.sort_unstable();
            for (_, key) in by_age.iter().take(self.textures.len() - MAX_TEXTURES) {
                self.textures.remove(key);
            }
        }
    }

    /// The cover of `album` if it is ready; else it is asked for (first in
    /// line) and `None` comes back until it is.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        library: &Library,
        album: &Album,
    ) -> Option<egui::TextureHandle> {
        let key = key_of(library, album);
        if let Some((texture, shown)) = self.textures.get_mut(&key) {
            *shown = ctx.cumulative_frame_nr();
            return Some(texture.clone());
        }
        if self.missing.contains(&key) {
            return None;
        }
        let tagged = album
            .tracks
            .iter()
            .map(|&t| &library.tracks[t as usize])
            .find(|t| t.has_cover)
            .map(|t| t.path.clone());
        let job = Job {
            key,
            folder: album.folder.clone(),
            tagged,
        };
        let mut jobs = self.queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if self.asked.insert(key) {
            jobs.push(job);
        } else if let Some(i) = jobs.iter().position(|j| j.key == key) {
            // Asked again: to the front of the line (the end of the stack).
            let job = jobs.remove(i);
            jobs.push(job);
        }
        self.queue.ready.notify_one();
        None
    }

    /// Whether `album` is known to have no cover.
    pub fn is_missing(&self, library: &Library, album: &Album) -> bool {
        self.missing.contains(&key_of(library, album))
    }
}

/// The album's key in the cache: its artist and title, and its first
/// file's path and time (a retagged or replaced file gets a new cover).
fn key_of(library: &Library, album: &Album) -> CoverKey {
    let first = album.tracks.first().map(|&t| &library.tracks[t as usize]);
    let mut h = Fnv::default();
    h.write(album.key.as_bytes());
    if let Some(t) = first {
        h.write(t.path.as_os_str().as_encoded_bytes());
        h.write(&t.mtime.to_le_bytes());
    }
    h.0
}

/// FNV-1a, 64 bits: stable between runs, unlike `DefaultHasher`'s seed.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn work(queue: &Queue, tx: &Sender<Found>, dir: Option<&Path>, ctx: &egui::Context) {
    if let Some(dir) = dir {
        let _ = std::fs::create_dir_all(dir);
    }
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = jobs.pop() {
                    break job;
                }
                jobs = queue.ready.wait(jobs).unwrap_or_else(|e| e.into_inner());
            }
        };
        let found = match cover(&job, dir) {
            Some(image) => Found::Image(job.key, image),
            None => Found::Missing(job.key),
        };
        if tx.send(found).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// The thumbnail of the job's album: from the cache, else made (and kept).
fn cover(job: &Job, dir: Option<&Path>) -> Option<egui::ColorImage> {
    let cached = dir.map(|d| d.join(format!("{:016x}.jpg", job.key)));
    let none = dir.map(|d| d.join(format!("{:016x}.none", job.key)));
    if let Some(path) = &cached
        && let Ok(bytes) = std::fs::read(path)
        && let Some(picture) = decode(&bytes)
    {
        return Some(color_image(&picture));
    }
    if none.as_ref().is_some_and(|p| p.exists()) {
        return None;
    }
    let bytes = in_folder(&job.folder)
        .and_then(|p| std::fs::read(p).ok())
        .or_else(|| job.tagged.as_deref().and_then(embedded));
    let Some(picture) = bytes.as_deref().and_then(decode) else {
        if let Some(none) = none {
            let _ = std::fs::write(none, b"");
        }
        return None;
    };
    let thumb = if picture.width().max(picture.height()) > SIDE {
        picture.resize(SIDE, SIDE, image::imageops::FilterType::Triangle)
    } else {
        picture
    };
    if let Some(path) = cached {
        save_jpeg(&thumb, &path);
    }
    Some(color_image(&thumb))
}

fn color_image(picture: &image::DynamicImage) -> egui::ColorImage {
    let rgba = picture.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    )
}

fn save_jpeg(picture: &image::DynamicImage, path: &Path) {
    let rgb = picture.to_rgb8();
    let tmp = path.with_extension("tmp");
    let result = std::fs::File::create(&tmp)
        .map_err(image::ImageError::IoError)
        .and_then(|file| {
            let mut writer = std::io::BufWriter::new(file);
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 85).encode_image(&rgb)
        })
        .and_then(|()| std::fs::rename(&tmp, path).map_err(image::ImageError::IoError));
    if let Err(e) = result {
        log::warn!("cannot keep the cover {}: {e}", path.display());
    }
}

/// A picture in `folder` named as a cover ([`FOLDER_NAMES`]), in any case.
fn in_folder(folder: &Path) -> Option<PathBuf> {
    let pictures: Vec<(String, PathBuf)> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_string_lossy().to_lowercase();
            if !matches!(ext.as_str(), "jpg" | "jpeg" | "png") {
                return None;
            }
            let stem = path.file_stem()?.to_string_lossy().to_lowercase();
            Some((stem, path))
        })
        .collect();
    FOLDER_NAMES.iter().find_map(|name| {
        pictures
            .iter()
            .find(|(stem, _)| stem == name)
            .map(|(_, p)| p.clone())
    })
}

/// The front cover in the tag of `path` (else its first picture).
fn embedded(path: &Path) -> Option<Vec<u8>> {
    use lofty::file::TaggedFileExt;
    use lofty::picture::PictureType;
    let tagged = lofty::probe::Probe::open(path)
        .ok()?
        .options(lofty::config::ParseOptions::new().read_properties(false))
        .read()
        .ok()?;
    let pictures: Vec<&lofty::picture::Picture> =
        tagged.tags().iter().flat_map(|t| t.pictures()).collect();
    pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())
        .map(|p| p.data().to_vec())
}

/// A JPEG or PNG, decoded within limits.
fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    let format = image::guess_format(bytes).ok()?;
    if !matches!(format, image::ImageFormat::Jpeg | image::ImageFormat::Png) {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODED_SIDE);
    limits.max_image_height = Some(MAX_DECODED_SIDE);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| log::debug!("cannot decode a cover: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_is_stable() {
        let mut h = Fnv::default();
        h.write(b"a");
        assert_eq!(h.0, 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn folder_picture_by_name_in_any_case() {
        let dir = std::env::temp_dir().join(format!("metalamp_cover_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Back.jpg"), b"x").unwrap();
        assert_eq!(in_folder(&dir), None);
        std::fs::write(dir.join("Folder.JPG"), b"x").unwrap();
        assert_eq!(in_folder(&dir), Some(dir.join("Folder.JPG")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
