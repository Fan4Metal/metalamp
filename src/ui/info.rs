//! The track info panel under the artists (Alt+3, as Winamp's File Info):
//! every field of the file's tag, its pictures, its audio properties and
//! its path. The file is the track selected last (in the playlist when it
//! was clicked last, else in the library's tracks), else the one playing.
//! Files are read on a thread of its own (`library::details`); while one is
//! read, the panel keeps showing the one before.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Align2, FontId, Rect, RichText, Ui, pos2, vec2};
use lofty::picture::PictureType;
use lofty::prelude::ItemKey;

use crate::format;
use crate::library::details::{self, Details};
use crate::palette;
use crate::ui::table::HEADER_HEIGHT;

type Read = (PathBuf, Result<Details, String>);

#[derive(Default)]
pub struct InfoView {
    /// The file asked for last.
    wanted: Option<PathBuf>,
    /// What was read last: of `wanted`, or of the file before while it is
    /// read.
    shown: Option<Read>,
    /// To the reading thread, started with the first file asked for.
    requests: Option<Sender<PathBuf>>,
    results: Option<Receiver<Read>>,
}

impl InfoView {
    /// Show `path` (none: nothing selected or playing).
    pub fn want(&mut self, ctx: &egui::Context, path: Option<&Path>) {
        if path == self.wanted.as_deref() {
            return;
        }
        self.wanted = path.map(Path::to_path_buf);
        let Some(path) = path else {
            self.shown = None;
            return;
        };
        if self.requests.is_none() {
            let (req_tx, req_rx) = channel::<PathBuf>();
            let (res_tx, res_rx) = channel();
            let ctx = ctx.clone();
            let started = std::thread::Builder::new()
                .name("info".into())
                .spawn(move || {
                    while let Ok(mut path) = req_rx.recv() {
                        // Only the last of the files asked for meanwhile
                        // (the arrow key held in a table).
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

    fn poll(&mut self) {
        let Some(rx) = &self.results else { return };
        while let Ok(read) = rx.try_recv() {
            if self.wanted.as_deref() == Some(read.0.as_path()) {
                self.shown = Some(read);
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, rect: Rect) {
        self.poll();
        let painter = ui.painter().clone();
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_HEIGHT));
        painter.rect_filled(header, 0.0, palette::HEADER_BG);
        let font = FontId::proportional(12.5);
        painter.text(
            pos2(header.left() + 6.0, header.center().y),
            Align2::LEFT_CENTER,
            tr!("Track info", "Сведения о треке"),
            font.clone(),
            palette::TEXT_WEAK,
        );
        if let Some((_, Ok(d))) = &self.shown {
            painter.text(
                pos2(header.right() - 6.0, header.center().y),
                Align2::RIGHT_CENTER,
                d.tags.join(" · "),
                font,
                palette::TEXT_WEAK,
            );
        }
        let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
        painter.rect_filled(body, 0.0, palette::LIST_BG);
        let text = match &self.shown {
            None => Some(tr!("No track selected", "Трек не выбран").to_owned()),
            Some((path, Err(e))) => Some(tr!(
                format!("Cannot read {}: {e}", path.display()),
                format!("Не удалось прочитать {}: {e}", path.display())
            )),
            Some((_, Ok(_))) => None,
        };
        if let Some(text) = text {
            let galley = painter.layout(
                text,
                FontId::proportional(13.0),
                palette::TEXT_WEAK,
                body.width() - 24.0,
            );
            let pos = body.center() - galley.size() / 2.0;
            painter.galley(pos, galley, palette::TEXT_WEAK);
            return;
        }
        let Some((_, Ok(d))) = &self.shown else { return };
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(body.shrink2(vec2(6.0, 4.0)))
                .id_salt("track_info"),
        );
        egui::ScrollArea::vertical()
            .id_salt("track_info_scroll")
            .auto_shrink([false, false])
            .show(&mut child, |ui| rows(ui, d));
    }
}

/// The fields as rows of a label and a value; the value wraps.
fn rows(ui: &mut Ui, d: &Details) {
    ui.spacing_mut().item_spacing.y = 3.0;
    let label_w = (ui.available_width() * 0.36).clamp(64.0, 120.0);
    let row = |ui: &mut Ui, label: &str, value: &str| {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                vec2(label_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(label_w);
                    ui.add(
                        egui::Label::new(RichText::new(label).color(palette::TEXT_WEAK))
                            .truncate(),
                    );
                },
            );
            ui.add(egui::Label::new(RichText::new(value).color(palette::TEXT)).wrap());
        });
    };
    for (key, value) in &d.fields {
        row(ui, &field_name(*key), value);
    }
    if !d.fields.is_empty() {
        ui.separator();
    }
    let ext = d
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    row(ui, tr!("Format", "Формат"), &ext);
    row(ui, tr!("Length", "Длительность"), &format::time(d.duration_ms));
    if d.bitrate > 0 {
        row(ui, tr!("Bit rate", "Битрейт"), &format::bitrate(d.bitrate));
    }
    if d.sample_rate > 0 {
        row(
            ui,
            tr!("Sample rate", "Частота"),
            &tr!(
                format!("{} Hz", d.sample_rate),
                format!("{} Гц", d.sample_rate)
            ),
        );
    }
    if let Some(bits) = d.bit_depth {
        row(
            ui,
            tr!("Bit depth", "Разрядность"),
            &tr!(format!("{bits} bits"), format!("{bits} бит")),
        );
    }
    if d.channels > 0 {
        let channels = match d.channels {
            1 => tr!("Mono", "Моно").to_owned(),
            2 => tr!("Stereo", "Стерео").to_owned(),
            n => n.to_string(),
        };
        row(ui, tr!("Channels", "Каналы"), &channels);
    }
    if let Some((delay, padding)) = d.gapless {
        row(
            ui,
            tr!("LAME tag", "Тег LAME"),
            &tr!(
                format!("delay {delay}, padding {padding} samples"),
                format!("задержка {delay}, добивка {padding} сэмплов")
            ),
        );
    }
    for p in &d.pictures {
        let mut text = p.format.clone();
        if let Some((w, h)) = p.pixels {
            text += &format!(" {w}×{h}");
        }
        text += &format!(", {}", format::size(p.bytes as u64));
        row(ui, &picture_name(p.kind), &text);
    }
    row(ui, tr!("Size", "Размер"), &format::size(d.size));
    row(ui, tr!("File", "Файл"), &d.path.display().to_string());
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
        ItemKey::Year => tr!("Year", "Год"),
        // ID3v2.3's TYER, read as ID3v2.4's TDRC.
        ItemKey::RecordingDate => tr!("Year", "Год"),
        ItemKey::Genre => tr!("Genre", "Жанр"),
        ItemKey::Comment => tr!("Comment", "Комментарий"),
        ItemKey::AlbumArtist => tr!("Album artist", "Исп. альбома"),
        ItemKey::Composer => tr!("Composer", "Композитор"),
        ItemKey::Publisher => tr!("Publisher", "Издатель"),
        ItemKey::Label => tr!("Label", "Лейбл"),
        ItemKey::OriginalArtist => tr!("Orig. artist", "Ориг. исп."),
        ItemKey::CopyrightMessage => tr!("Copyright", "Копирайт"),
        ItemKey::AudioFileUrl => "URL",
        ItemKey::EncodedBy => tr!("Encoded by", "Кодировщик"),
        ItemKey::EncoderSoftware => tr!("Encoder", "Программа"),
        ItemKey::Bpm | ItemKey::IntegerBpm => "BPM",
        ItemKey::ReplayGainTrackGain => tr!("Track gain", "Усиление трека"),
        ItemKey::ReplayGainAlbumGain => tr!("Album gain", "Усиление альбома"),
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
