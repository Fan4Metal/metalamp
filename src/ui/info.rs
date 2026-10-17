//! The file info window, as Winamp's File Info (Alt+3): a click on the
//! player bar's cover opens it for the track heard, Alt+3 or the context
//! menus of the tracks and the playlist for the track selected. The path
//! on top; the cover with the audio properties on the left; the tag's
//! fields in Winamp's places on the right (shown read-only, empty ones
//! too), the tag's other fields under them. The file is read on a thread
//! of its own (`library::details`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Key, RichText, TextureHandle, Ui, vec2};
use lofty::picture::PictureType;
use lofty::prelude::ItemKey;

use crate::format;
use crate::library::details::{self, Details};
use crate::palette;

type Read = (PathBuf, Result<Details, String>);

/// The cover's side in the window, in points.
const COVER: f32 = 220.0;
const LABEL: f32 = 92.0;

/// The fields of Winamp's ID3v2 page, left column, then right column; a
/// field shown for the first of its keys the tag has.
const LEFT: [&[ItemKey]; 8] = [
    &[ItemKey::TrackNumber],
    &[ItemKey::TrackTitle],
    &[ItemKey::TrackArtist],
    &[ItemKey::AlbumTitle],
    &[ItemKey::Year, ItemKey::RecordingDate],
    &[ItemKey::Genre],
    &[ItemKey::AlbumArtist],
    &[ItemKey::Composer],
];
const RIGHT: [&[ItemKey]; 9] = [
    &[ItemKey::DiscNumber],
    &[ItemKey::Publisher, ItemKey::Label],
    &[ItemKey::OriginalArtist],
    &[ItemKey::CopyrightMessage],
    &[ItemKey::AudioFileUrl],
    &[ItemKey::EncodedBy],
    &[ItemKey::Bpm, ItemKey::IntegerBpm],
    &[ItemKey::ReplayGainTrackGain],
    &[ItemKey::ReplayGainAlbumGain],
];

#[derive(Default)]
pub struct InfoView {
    pub open: bool,
    /// Opened this frame: the key that opened it must not close it.
    fresh: bool,
    /// The file asked for.
    wanted: Option<PathBuf>,
    /// What was read of it (or of the file before, until it is read).
    shown: Option<Read>,
    cover: Option<(PathBuf, TextureHandle)>,
    /// To the reading thread, started with the first file asked for.
    requests: Option<Sender<PathBuf>>,
    results: Option<Receiver<Read>>,
}

impl InfoView {
    /// Open the window for `path`; it is read again (the tag may have been
    /// changed since).
    pub fn open(&mut self, ctx: &egui::Context, path: &Path) {
        self.open = true;
        self.fresh = true;
        self.wanted = Some(path.to_path_buf());
        if self.requests.is_none() {
            let (req_tx, req_rx) = channel::<PathBuf>();
            let (res_tx, res_rx) = channel();
            let ctx = ctx.clone();
            let started = std::thread::Builder::new()
                .name("info".into())
                .spawn(move || {
                    while let Ok(mut path) = req_rx.recv() {
                        // Only the last of the files asked for meanwhile.
                        while let Ok(next) = req_rx.try_recv() {
                            path = next;
                        }
                        let read = details::read(&path);
                        if res_tx.send((path, read)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                });
            if let Err(e) = started {
                log::warn!("no info thread: {e}");
                return;
            }
            self.requests = Some(req_tx);
            self.results = Some(res_rx);
        }
        if let Some(tx) = &self.requests {
            let _ = tx.send(path.to_path_buf());
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.results else { return };
        while let Ok(mut read) = rx.try_recv() {
            if self.wanted.as_deref() != Some(read.0.as_path()) {
                continue;
            }
            self.cover = match &mut read.1 {
                Ok(d) => d.cover.take().map(|image| {
                    let texture = ctx.load_texture("file_info_cover", image, Default::default());
                    (read.0.clone(), texture)
                }),
                Err(_) => None,
            };
            self.shown = Some(read);
        }
    }

    /// Draws the window while it is open; closes it on the cross, the
    /// button, Esc, Enter or a click outside.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
        if !self.open {
            return;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let wanted = self.wanted.clone().unwrap_or_default();
        let shown = self
            .shown
            .as_ref()
            .filter(|(p, _)| *p == wanted)
            .map(|(_, d)| d);
        let cover = self
            .cover
            .as_ref()
            .filter(|(p, _)| *p == wanted)
            .map(|(_, t)| t.clone());
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("file_info")).show(ctx, |ui| {
            ui.set_width(800.0);
            ui.horizontal(|ui| {
                ui.heading(tr!("File info", "Сведения о файле"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close |= super::close_cross(ui);
                });
            });
            ui.add_space(4.0);
            let path = wanted.display().to_string();
            ui.add(field(egui::TextEdit::singleline(&mut path.as_str())).desired_width(f32::INFINITY));
            ui.add_space(8.0);
            match shown {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak(tr!("Reading…", "Чтение…"));
                    });
                }
                Some(Err(e)) => {
                    ui.colored_label(
                        palette::ERROR,
                        tr!(
                            format!("Cannot read the file: {e}"),
                            format!("Не удалось прочитать файл: {e}")
                        ),
                    );
                }
                Some(Ok(d)) => body(ui, d, cover.as_ref()),
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(tr!("Show in folder", "Показать в папке"))
                    .clicked()
                {
                    crate::win::show_in_folder(&wanted);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(tr!("Close", "Закрыть")).min_size(vec2(90.0, 0.0)))
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        });
        let keys = !fresh && ctx.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter));
        if close || keys || (!fresh && modal.should_close()) {
            self.open = false;
        }
    }
}

/// The cover and the properties on the left, the fields on the right.
fn body(ui: &mut Ui, d: &Details, cover: Option<&TextureHandle>) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(COVER);
            let (rect, _) = ui.allocate_exact_size(vec2(COVER, COVER), egui::Sense::hover());
            match cover {
                Some(t) => {
                    let [w, h] = t.size();
                    let scale = COVER / w.max(h) as f32;
                    let r = egui::Rect::from_center_size(
                        rect.center(),
                        vec2(w as f32 * scale, h as f32 * scale),
                    );
                    ui.painter().image(
                        t.id(),
                        r,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().rect_filled(rect, 4.0, palette::CARD_BG);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        tr!("No cover", "Нет обложки"),
                        egui::FontId::proportional(13.0),
                        palette::TEXT_WEAK,
                    );
                }
            }
            ui.add_space(6.0);
            for line in properties(d) {
                ui.add(egui::Label::new(RichText::new(line).color(palette::TEXT_WEAK)).wrap());
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.horizontal_top(|ui| {
                fields_grid(ui, "info_left", &LEFT, d, 230.0);
                ui.add_space(8.0);
                fields_grid(ui, "info_right", &RIGHT, d, 150.0);
            });
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                label(ui, &field_name(ItemKey::Comment));
                let mut comment = value(d, &[ItemKey::Comment]);
                ui.add(
                    field(egui::TextEdit::multiline(&mut comment))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
            });
            let shown: Vec<ItemKey> = LEFT
                .iter()
                .chain(RIGHT.iter())
                .flat_map(|keys| keys.iter().copied())
                .chain([ItemKey::Comment])
                .collect();
            let rest: Vec<&(ItemKey, String)> =
                d.fields.iter().filter(|(k, _)| !shown.contains(k)).collect();
            if !rest.is_empty() {
                ui.add_space(8.0);
                ui.weak(tr!("Other fields", "Другие поля"));
                egui::ScrollArea::vertical()
                    .id_salt("info_rest")
                    .max_height(140.0)
                    .show(ui, |ui| {
                        egui::Grid::new("info_rest_grid")
                            .num_columns(2)
                            .spacing(vec2(8.0, 4.0))
                            .show(ui, |ui| {
                                for (key, text) in rest {
                                    label(ui, &field_name(*key));
                                    let mut text = text.as_str();
                                    ui.add(
                                        field(egui::TextEdit::singleline(&mut text))
                                            .desired_width(f32::INFINITY),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
            }
        });
    });
}

/// A column of fields: a label and a read-only field each.
fn fields_grid(ui: &mut Ui, id: &str, keys: &[&[ItemKey]], d: &Details, width: f32) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(vec2(8.0, 4.0))
        .show(ui, |ui| {
            for keys in keys {
                label(ui, &field_name(keys[0]));
                let mut text = value(d, keys);
                ui.add(field(egui::TextEdit::singleline(&mut text)).desired_width(width));
                ui.end_row();
            }
        });
}

/// A read-only field in a box, as an editable one (egui draws read-only
/// ones without, and an empty one would not be seen).
fn field(edit: egui::TextEdit<'_>) -> egui::TextEdit<'_> {
    edit.frame(
        egui::Frame::new()
            .fill(palette::LIST_BG)
            .stroke(egui::Stroke::new(1.0, palette::LINE))
            .corner_radius(3.0)
            .inner_margin(egui::Margin::symmetric(5, 3)),
    )
}

/// A field's label, right-aligned in its column as Winamp's.
fn label(ui: &mut Ui, text: &str) {
    ui.allocate_ui_with_layout(
        vec2(LABEL, 20.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.set_min_width(LABEL);
            ui.add(egui::Label::new(RichText::new(text).color(palette::TEXT_WEAK)).truncate());
        },
    );
}

/// The value of the first of `keys` the tag has, else empty.
fn value<'a>(d: &'a Details, keys: &[ItemKey]) -> &'a str {
    keys.iter()
        .find_map(|k| d.fields.iter().find(|(f, _)| f == k))
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

/// The lines under the cover: the format and the audio properties, the
/// size, the tags, the LAME tag, the pictures.
fn properties(d: &Details) -> Vec<String> {
    let ext = d
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    let mut lines = vec![format!("{ext} · {}", format::time(d.duration_ms))];
    let mut audio = Vec::new();
    if d.bitrate > 0 {
        audio.push(format::bitrate(d.bitrate));
    }
    if d.sample_rate > 0 {
        audio.push(tr!(
            format!("{} Hz", d.sample_rate),
            format!("{} Гц", d.sample_rate)
        ));
    }
    if let Some(bits) = d.bit_depth {
        audio.push(tr!(format!("{bits} bits"), format!("{bits} бит")));
    }
    match d.channels {
        0 => {}
        1 => audio.push(tr!("mono", "моно").to_owned()),
        2 => audio.push(tr!("stereo", "стерео").to_owned()),
        n => audio.push(tr!(format!("{n} channels"), format!("каналов: {n}"))),
    }
    if !audio.is_empty() {
        lines.push(audio.join(", "));
    }
    lines.push(tr!(
        format!("Size: {}", format::size(d.size)),
        format!("Размер: {}", format::size(d.size))
    ));
    if !d.tags.is_empty() {
        lines.push(tr!(
            format!("Tags: {}", d.tags.join(", ")),
            format!("Теги: {}", d.tags.join(", "))
        ));
    }
    if let Some((delay, padding)) = d.gapless {
        lines.push(tr!(
            format!("LAME: delay {delay}, padding {padding}"),
            format!("LAME: задержка {delay}, добивка {padding}")
        ));
    }
    for p in &d.pictures {
        let mut text = format!("{}: {}", picture_name(p.kind), p.format);
        if let Some((w, h)) = p.pixels {
            text += &format!(" {w}×{h}");
        }
        text += &format!(", {}", format::size(p.bytes as u64));
        lines.push(text);
    }
    lines
}

/// A field's name as Winamp's File Info window has it; fields it lacks by
/// lofty's name for them, in words.
fn field_name(key: ItemKey) -> String {
    let name = match key {
        ItemKey::TrackNumber => tr!("Track #", "Трек №"),
        ItemKey::DiscNumber => tr!("Disc #", "Диск №"),
        ItemKey::TrackTitle => tr!("Title", "Название"),
        ItemKey::TrackArtist => tr!("Artist", "Исполнитель"),
        ItemKey::AlbumTitle => tr!("Album", "Альбом"),
        // ID3v2.3's TYER is read as ID3v2.4's TDRC.
        ItemKey::Year | ItemKey::RecordingDate => tr!("Year", "Год"),
        ItemKey::Genre => tr!("Genre", "Жанр"),
        ItemKey::Comment => tr!("Comment", "Комментарий"),
        ItemKey::AlbumArtist => tr!("Album artist", "Исп. альбома"),
        ItemKey::Composer => tr!("Composer", "Композитор"),
        ItemKey::Publisher | ItemKey::Label => tr!("Publisher", "Издатель"),
        ItemKey::OriginalArtist => tr!("Orig. artist", "Ориг. исп."),
        ItemKey::CopyrightMessage => tr!("Copyright", "Копирайт"),
        ItemKey::AudioFileUrl => "URL",
        ItemKey::EncodedBy => tr!("Encoded", "Кодировщик"),
        ItemKey::EncoderSoftware => tr!("Encoder", "Программа"),
        ItemKey::Bpm | ItemKey::IntegerBpm => "BPM",
        ItemKey::ReplayGainTrackGain => tr!("Track gain", "Усил. трека"),
        ItemKey::ReplayGainAlbumGain => tr!("Album gain", "Усил. альбома"),
        ItemKey::ReplayGainTrackPeak => tr!("Track peak", "Пик трека"),
        ItemKey::ReplayGainAlbumPeak => tr!("Album peak", "Пик альбома"),
        ItemKey::Lyricist => tr!("Lyricist", "Автор текста"),
        ItemKey::Conductor => tr!("Conductor", "Дирижёр"),
        ItemKey::Lyrics => tr!("Lyrics", "Текст"),
        other => return words(&format!("{other:?}")),
    };
    name.to_owned()
}

/// `MusicBrainzTrackId` as `Music Brainz Track Id`.
fn words(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn picture_name(kind: PictureType) -> String {
    let name = match kind {
        PictureType::CoverFront => tr!("Front cover", "Обложка"),
        PictureType::CoverBack => tr!("Back cover", "Задняя обложка"),
        PictureType::Leaflet => tr!("Leaflet", "Буклет"),
        PictureType::Media => tr!("Disc", "Диск"),
        PictureType::Artist | PictureType::LeadArtist => tr!("Artist", "Исполнитель"),
        PictureType::Band => tr!("Band", "Группа"),
        PictureType::Other => tr!("Picture", "Картинка"),
        other => return words(&format!("{other:?}")),
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_words() {
        assert_eq!(words("MusicBrainzTrackId"), "Music Brainz Track Id");
        assert_eq!(words("Isrc"), "Isrc");
    }
}
