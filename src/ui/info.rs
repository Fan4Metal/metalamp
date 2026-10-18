//! The file info window (I, Alt+3): a click on the
//! player bar's cover opens it for the track heard, I, Alt+3 or the context
//! menus of the tracks and the playlist for the track selected. The path
//! cover, the title, the artist and the album on top with the audio
//! properties as chips; the tag's fields in their usual order in two columns
//! (empty ones as a dash), the tag's other fields under them; the file's
//! path and the rest. Shown, not edited; the text can be selected and
//! copied. The cross, Esc, Enter or a click outside closes it. The file is
//! read on a thread of its own (`library::details`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Key, RichText, TextureHandle, Ui, vec2};
use lofty::picture::PictureType;
use lofty::prelude::ItemKey;

use crate::format;
use crate::library::details::{self, Details};
use crate::palette;

type Read = (PathBuf, Result<Details, String>);

const WIDTH: f32 = 720.0;
/// The cover's side in the window, in points.
const COVER: f32 = 150.0;

/// The usual ID3v2 fields, left column, then right column; a
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
        let modal = egui::Modal::new(egui::Id::new("file_info"))
            .frame(
                egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
                    .fill(palette::panel_bg())
                    .inner_margin(egui::Margin::same(20))
                    .corner_radius(10.0),
            )
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                match shown {
                    None => {
                        close |= hero(ui, None, &wanted, None);
                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.weak(tr!("Reading…", "Чтение…"));
                        });
                    }
                    Some(Err(e)) => {
                        close |= hero(ui, None, &wanted, None);
                        ui.add_space(16.0);
                        ui.colored_label(
                            palette::error(),
                            tr!(
                                format!("Cannot read the file: {e}"),
                                format!("Не удалось прочитать файл: {e}")
                            ),
                        );
                    }
                    Some(Ok(d)) => {
                        close |= hero(ui, Some(d), &wanted, cover.as_ref());
                        // A long tag (lyrics, many fields) scrolls within
                        // the screen instead of pushing the window off it.
                        // The height is forced (`min_scrolled_height`), not
                        // left to the room below: a centred modal is placed
                        // by its size of the frame before, so an area that
                        // only took the room left froze at the height the
                        // window had while the file was read.
                        let room =
                            (ctx.content_rect().height() - 2.0 * 20.0 - COVER - 60.0).max(120.0);
                        egui::ScrollArea::vertical()
                            .id_salt("file_info_body")
                            .min_scrolled_height(room)
                            .max_height(room)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                // Room for the scroll bar at the right edge
                                // (`set_max_width` here also shrank the
                                // area's height to its minimum).
                                egui::Frame::new()
                                    .inner_margin(egui::Margin {
                                        right: 14,
                                        ..egui::Margin::ZERO
                                    })
                                    .show(ui, |ui| body(ui, d));
                            });
                    }
                }
            });
        let keys = !fresh && ctx.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter));
        if close || keys || (!fresh && modal.should_close()) {
            self.open = false;
        }
    }
}

/// The top of the window: the cover, the title, the artist, the album
/// with its year and genre, and the audio properties as chips; the cross
/// in the corner (true when clicked). Without details (while the file is
/// read, or when it cannot be), the file's name alone.
fn hero(ui: &mut Ui, d: Option<&Details>, path: &Path, cover: Option<&TextureHandle>) -> bool {
    let mut close = false;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    ui.horizontal_top(|ui| {
        cover_box(ui, cover, COVER);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            let title = d
                .map(|d| value(d, &[ItemKey::TrackTitle]))
                .filter(|t| !t.is_empty())
                .unwrap_or(&stem);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    close = super::close_cross(ui);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(title)
                                    .size(22.0)
                                    .strong()
                                    .color(palette::text_bright()),
                            )
                            .truncate(),
                        );
                    });
                });
            });
            let Some(d) = d else { return };
            let artist = value(d, &[ItemKey::TrackArtist, ItemKey::AlbumArtist]);
            if !artist.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(artist).size(16.0).color(palette::text()))
                        .truncate(),
                );
            }
            let album: Vec<&str> = [
                value(d, &[ItemKey::AlbumTitle]),
                value(d, &[ItemKey::Year, ItemKey::RecordingDate]),
                value(d, &[ItemKey::Genre]),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
            if !album.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(album.join("  ·  "))
                            .size(13.5)
                            .color(palette::text_weak()),
                    )
                    .truncate(),
                );
            }
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for text in chips(d) {
                    chip(ui, &text);
                }
            });
        });
    });
    close
}

/// The cover at `side` points with rounded corners, or a placeholder.
fn cover_box(ui: &mut Ui, cover: Option<&TextureHandle>, side: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), egui::Sense::hover());
    let painter = ui.painter();
    match cover {
        Some(t) => {
            let [w, h] = t.size();
            let scale = side / w.max(h) as f32;
            let r = egui::Rect::from_center_size(
                rect.center(),
                vec2(w as f32 * scale, h as f32 * scale),
            );
            painter.rect_filled(rect, 8.0, palette::list_bg());
            egui::Image::new(egui::load::SizedTexture::new(t.id(), r.size()))
                .corner_radius(8.0)
                .paint_at(ui, r);
        }
        None => {
            painter.rect_filled(rect, 8.0, palette::card_bg());
            crate::ui::icons::paint(
                painter,
                crate::ui::icons::Icon::File,
                rect.center(),
                side * 0.3,
                palette::text_weak(),
                palette::card_bg(),
            );
        }
    }
}

/// A small rounded label with a fact (`320 kbps`).
fn chip(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(12.0),
        palette::text(),
    );
    let size = galley.size() + vec2(18.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, size.y / 2.0, palette::card_bg());
    ui.painter()
        .galley(rect.min + vec2(9.0, 4.0), galley, palette::text());
}

/// The chips of the hero: the format, the length, the bit rate, the
/// sample rate and depth, the channels, the size.
fn chips(d: &Details) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ext) = d.path.extension() {
        out.push(ext.to_string_lossy().to_uppercase());
    }
    out.push(format::time(d.duration_ms));
    if d.bitrate > 0 {
        out.push(format::bitrate(d.bitrate));
    }
    if d.sample_rate > 0 {
        out.push(format::sample_rate(d.sample_rate));
    }
    if let Some(bits) = d.bit_depth {
        out.push(tr!(format!("{bits} bit"), format!("{bits} бит")));
    }
    if let Some(c) = format::channels(d.channels) {
        out.push(c);
    }
    out.push(format::size(d.size));
    out
}

/// The sections under the hero: the tag's fields, then the file.
fn body(ui: &mut Ui, d: &Details) {
    ui.add_space(18.0);
    let tags = d.tags.join(" · ");
    section(ui, tr!("Tag", "Тег"), |ui| {
        ui.label(RichText::new(tags).size(11.0).color(palette::text_weak()));
    });
    ui.columns(2, |cols| {
        fields_grid(&mut cols[0], "info_left", &LEFT, d);
        fields_grid(&mut cols[1], "info_right", &RIGHT, d);
    });
    ui.add_space(4.0);
    egui::Grid::new("info_comment")
        .num_columns(2)
        .spacing(vec2(12.0, 6.0))
        .show(ui, |ui| {
            field(
                ui,
                &field_name(ItemKey::Comment),
                value(d, &[ItemKey::Comment]),
                true,
            );
            ui.end_row();
            let shown: Vec<ItemKey> = LEFT
                .iter()
                .chain(RIGHT.iter())
                .flat_map(|keys| keys.iter().copied())
                .chain([ItemKey::Comment])
                .collect();
            for (key, text) in d.fields.iter().filter(|(k, _)| !shown.contains(k)) {
                if is_hidden(*key) {
                    continue;
                }
                field(ui, &field_name(*key), text, true);
                ui.end_row();
            }
        });

    // The lyrics apart: many lines, which the fields' grid would stretch.
    let lyrics = value(d, &[ItemKey::UnsyncLyrics, ItemKey::Lyrics]);
    if !lyrics.is_empty() {
        ui.add_space(14.0);
        section(ui, tr!("Lyrics", "Текст"), |_| {});
        ui.add(egui::Label::new(RichText::new(lyrics).color(palette::text())).wrap());
    }

    ui.add_space(14.0);
    section(ui, tr!("File", "Файл"), |ui| {
        if ui
            .small_button(tr!("Show in folder", "Показать в папке"))
            .clicked()
        {
            crate::win::show_in_folder(&d.path);
        }
    });
    egui::Grid::new("info_file")
        .num_columns(2)
        .spacing(vec2(12.0, 6.0))
        .show(ui, |ui| {
            field(ui, tr!("Path", "Путь"), &d.path.display().to_string(), true);
            ui.end_row();
            if let Some((delay, padding)) = d.gapless {
                field(
                    ui,
                    tr!("LAME tag", "Тег LAME"),
                    &tr!(
                        format!("encoder delay {delay}, padding {padding} samples"),
                        format!("задержка кодировщика {delay}, добивка {padding} сэмплов")
                    ),
                    true,
                );
                ui.end_row();
            }
            for p in &d.pictures {
                let mut text = p.format.clone();
                if let Some((w, h)) = p.pixels {
                    text += &format!(" {w}×{h}");
                }
                text += &format!(", {}", format::size(p.bytes as u64));
                field(ui, &picture_name(p.kind), &text, true);
                ui.end_row();
            }
        });
}

/// Fields not listed among the others: the lyrics have a section of
/// their own, and TLEN (the length in milliseconds) is the chip already.
fn is_hidden(key: ItemKey) -> bool {
    matches!(
        key,
        ItemKey::UnsyncLyrics | ItemKey::Lyrics | ItemKey::Length
    )
}

/// A section's heading: small capitals, `right` (a note or a button) at
/// the right end, a line under them.
fn section(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .size(11.0)
                .strong()
                .color(palette::text_weak()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.left()..=r.right(),
        r.top() + 2.0,
        egui::Stroke::new(1.0, palette::line()),
    );
    ui.add_space(10.0);
}

/// A column of fields: a label and a value each, in a grid that sizes
/// the labels' column to the widest.
fn fields_grid(ui: &mut Ui, id: &str, keys: &[&[ItemKey]], d: &Details) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(vec2(12.0, 6.0))
        .show(ui, |ui| {
            for keys in keys {
                field(ui, &field_name(keys[0]), value(d, keys), false);
                ui.end_row();
            }
        });
}

/// A label in the weak colour and its value (a dash when empty), as two
/// cells of a grid; `wrap`: a long value takes more lines, else it is cut.
fn field(ui: &mut Ui, label: &str, text: &str, wrap: bool) {
    // Not truncated: a grid sizes its columns by the frame before, so a
    // truncated label would never get its room.
    ui.add(egui::Label::new(RichText::new(label).color(palette::text_weak())).extend());
    if text.is_empty() {
        ui.label(RichText::new("—").color(palette::line()));
    } else {
        let label = egui::Label::new(RichText::new(text).color(palette::text()));
        ui.add(if wrap { label.wrap() } else { label.truncate() });
    }
}

/// The value of the first of `keys` the tag has, else empty.
fn value<'a>(d: &'a Details, keys: &[ItemKey]) -> &'a str {
    keys.iter()
        .find_map(|k| d.fields.iter().find(|(f, _)| f == k))
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

/// A field's name as players and taggers show it; the others by
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
        ItemKey::EncodedBy => tr!("Encoded by", "Кодировщик"),
        ItemKey::EncoderSoftware => tr!("Encoder", "Программа"),
        ItemKey::Bpm | ItemKey::IntegerBpm => "BPM",
        ItemKey::ReplayGainTrackGain => tr!("Track gain", "Усил. трека"),
        ItemKey::ReplayGainAlbumGain => tr!("Album gain", "Усил. альбома"),
        ItemKey::ReplayGainTrackPeak => tr!("Track peak", "Пик трека"),
        ItemKey::ReplayGainAlbumPeak => tr!("Album peak", "Пик альбома"),
        ItemKey::Lyricist => tr!("Lyricist", "Автор текста"),
        ItemKey::Conductor => tr!("Conductor", "Дирижёр"),
        ItemKey::Lyrics | ItemKey::UnsyncLyrics => tr!("Lyrics", "Текст"),
        ItemKey::CommercialInformationUrl => tr!("Shop URL", "URL магазина"),
        ItemKey::CopyrightUrl => tr!("Copyright URL", "URL копирайта"),
        ItemKey::TrackArtistUrl => tr!("Artist URL", "URL исполнителя"),
        ItemKey::PublisherUrl => tr!("Publisher URL", "URL издателя"),
        ItemKey::AudioSourceUrl => tr!("Source URL", "URL источника"),
        ItemKey::RadioStationUrl => tr!("Station URL", "URL станции"),
        ItemKey::PaymentUrl => tr!("Payment URL", "URL оплаты"),
        ItemKey::EncoderSettings => tr!("Encoder settings", "Настройки кодировщика"),
        ItemKey::EncodingTime => tr!("Encoded on", "Дата кодирования"),
        ItemKey::TaggingTime => tr!("Tagged on", "Дата тега"),
        ItemKey::OriginalReleaseDate => tr!("Original release", "Первый выпуск"),
        ItemKey::ReleaseDate => tr!("Released", "Выпущен"),
        ItemKey::Isrc => "ISRC",
        ItemKey::CatalogNumber => tr!("Catalogue number", "Номер в каталоге"),
        ItemKey::InitialKey => tr!("Key", "Тональность"),
        ItemKey::Mood => tr!("Mood", "Настроение"),
        ItemKey::FlagCompilation => tr!("Compilation", "Сборник"),
        ItemKey::TrackSubtitle => tr!("Subtitle", "Подзаголовок"),
        ItemKey::SetSubtitle => tr!("Disc subtitle", "Название диска"),
        ItemKey::ContentGroup => tr!("Grouping", "Группа"),
        ItemKey::OriginalAlbumTitle => tr!("Original album", "Ориг. альбом"),
        ItemKey::OriginalLyricist => tr!("Original lyricist", "Ориг. автор текста"),
        ItemKey::Arranger => tr!("Arranger", "Аранжировщик"),
        ItemKey::Writer => tr!("Writer", "Автор"),
        ItemKey::Director => tr!("Director", "Режиссёр"),
        ItemKey::Engineer => tr!("Engineer", "Звукоинженер"),
        ItemKey::MixDj => tr!("Mix DJ", "Диджей сведения"),
        ItemKey::MixEngineer => tr!("Mix engineer", "Звукорежиссёр сведения"),
        ItemKey::Performer => tr!("Performer", "Исполнители"),
        ItemKey::Producer => tr!("Producer", "Продюсер"),
        ItemKey::Remixer => tr!("Remixer", "Ремикс"),
        ItemKey::Language => tr!("Language", "Язык"),
        ItemKey::Script => tr!("Script", "Письменность"),
        ItemKey::Barcode => tr!("Barcode", "Штрихкод"),
        ItemKey::Description => tr!("Description", "Описание"),
        ItemKey::AlbumTitleSortOrder => tr!("Album sort", "Сорт. альбома"),
        ItemKey::AlbumArtistSortOrder => tr!("Album artist sort", "Сорт. исп. альбома"),
        ItemKey::TrackTitleSortOrder => tr!("Title sort", "Сорт. названия"),
        ItemKey::TrackArtistSortOrder => tr!("Artist sort", "Сорт. исполнителя"),
        ItemKey::ComposerSortOrder => tr!("Composer sort", "Сорт. композитора"),
        ItemKey::Popularimeter => tr!("Rating", "Рейтинг"),
        // MusicBrainz, AcoustID and the like: names of services, the same
        // in either language.
        other => return words(&format!("{other:?}")),
    };
    name.to_owned()
}

/// `MusicBrainzTrackId` as `MusicBrainz Track Id`: a word before each
/// capital, except within the names of services.
fn words(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out.replace("Music Brainz", "MusicBrainz")
        .replace("Acoust Id", "AcoustID")
        .replace(" Id", " ID")
        .replace(" Url", " URL")
}

fn picture_name(kind: PictureType) -> String {
    let name = match kind {
        PictureType::CoverFront => tr!("Front cover", "Обложка"),
        PictureType::CoverBack => tr!("Back cover", "Задняя обложка"),
        PictureType::Leaflet => tr!("Leaflet", "Буклет"),
        PictureType::Media => tr!("Disc", "Диск"),
        PictureType::Artist | PictureType::LeadArtist => tr!("Artist", "Исполнитель"),
        PictureType::Band => tr!("Band", "Группа"),
        PictureType::Icon => tr!("Icon", "Значок"),
        PictureType::OtherIcon => tr!("Other icon", "Другой значок"),
        PictureType::Conductor => tr!("Conductor", "Дирижёр"),
        PictureType::Composer => tr!("Composer", "Композитор"),
        PictureType::Lyricist => tr!("Lyricist", "Автор текста"),
        PictureType::RecordingLocation => tr!("Recording location", "Место записи"),
        PictureType::DuringRecording => tr!("During recording", "Во время записи"),
        PictureType::DuringPerformance => tr!("During performance", "Во время выступления"),
        PictureType::ScreenCapture => tr!("Screen capture", "Кадр из видео"),
        PictureType::BrightFish => tr!("Bright fish", "Яркая рыбка"),
        PictureType::Illustration => tr!("Illustration", "Иллюстрация"),
        PictureType::BandLogo => tr!("Band logo", "Логотип группы"),
        PictureType::PublisherLogo => tr!("Publisher logo", "Логотип издателя"),
        _ => tr!("Picture", "Картинка"),
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_words() {
        assert_eq!(words("MusicBrainzTrackId"), "MusicBrainz Track ID");
        assert_eq!(words("AcoustIdFingerprint"), "AcoustID Fingerprint");
        assert_eq!(words("Barcode"), "Barcode");
    }
}
