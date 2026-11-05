//! The library's statistics: a modal of cards over the library. On top
//! the tiles with the headline numbers (tracks, albums, artists, genres,
//! length, size, plays, time heard), under them the charts in two columns
//! (one in a narrow window): bit rates and lengths as columns, the formats
//! as a ring, the albums by year as columns across, the tracks added as an
//! area over the months, genres, artists, ratings and sample rates as bars,
//! the tags as meters, the tracks played most as a list. Every mark shows
//! its numbers under the pointer; a click on an artist or a track goes to
//! it in the library (`LibraryAction::GoTo`) and closes the window.
//!
//! The numbers are `library::summary`'s, worked out again only when the
//! library or the plays changed. Bars are the accent (one hue: they tell
//! how much); the formats' ring alone is categorical, a fixed colour per
//! format from the dataviz skill's reference palette, with its legend.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::{Duration, Instant};

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Margin, Pos2, Rect, Sense, Shape,
    Stroke, Ui, Vec2, pos2, vec2,
};

use super::icons::{self, Icon};
use super::library::LibraryAction;
use crate::format;
use crate::i18n::{Lang, lang, ru_plural};
use crate::library::summary::{self, BITRATE_LOSSLESS, Summary};
use crate::library::{AlbumId, Library, TrackId};
use crate::palette;
use crate::stats::Stats;

/// The window's size when the program's window holds it.
const SIZE: Vec2 = vec2(1060.0, 780.0);
const RADIUS: u8 = 10;
/// The header's height and the content's margins.
const HEADER: f32 = 64.0;
const PAD: f32 = 22.0;
/// Between the tiles and the cards.
const GAP: f32 = 14.0;
const TILE: f32 = 92.0;
/// A card's title and note over its chart.
const CARD_HEAD: f32 = 62.0;
/// A bar's row at most.
const ROW: f32 = 26.0;
/// Two columns of cards from this width.
const TWO_COLUMNS: f32 = 720.0;
/// The rows of a list of bars.
const TOP_ROWS: usize = 10;
/// The share of a card's width for the labels of its bars.
const LABELS: f32 = 0.36;
const RATE_ROWS: usize = 6;

#[derive(Default)]
pub struct StatisticsView {
    pub open: bool,
    /// Opened this frame: the key that opened it must not close it.
    fresh: bool,
    /// The numbers, of (library generation, plays revision).
    summary: Option<(u64, u64, Summary)>,
    took: Duration,
    /// Scrolled to this offset at start (`METALAMP_STATISTICS=<offset>`).
    scroll: Option<f32>,
    /// No library is shown (the player and playlist layout): the artists
    /// and tracks lead nowhere. Set by the app each frame.
    pub no_library: bool,
}

/// Where a click takes the library.
#[derive(Clone, Copy)]
enum Go {
    Artist(AlbumId),
    Track(AlbumId, TrackId),
}

/// Whether the rows lead to the library, and where the one clicked does.
struct Going {
    allowed: bool,
    to: Option<Go>,
}

impl Go {
    fn action(self) -> LibraryAction {
        match self {
            Go::Artist(album) => LibraryAction::GoTo {
                album,
                track: None,
                to_album: false,
            },
            Go::Track(album, track) => LibraryAction::GoTo {
                album,
                track: Some(track),
                to_album: true,
            },
        }
    }
}

impl StatisticsView {
    /// Open at start with `METALAMP_STATISTICS=<scroll offset>` (GUI
    /// checks: `0` the top).
    pub fn from_env() -> StatisticsView {
        let wanted = std::env::var("METALAMP_STATISTICS").ok();
        StatisticsView {
            open: wanted.is_some(),
            scroll: wanted.and_then(|w| w.trim().parse().ok()),
            ..StatisticsView::default()
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.fresh = self.open;
    }

    /// Draws the window while it is open; closed by the cross, Esc, Enter
    /// or a click outside. A click on an artist or a track closes it and
    /// returns where to go.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        library: &Library,
        stats: &Stats,
    ) -> Option<LibraryAction> {
        if !self.open {
            return None;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let key = (library.generation, stats.revision);
        if self
            .summary
            .as_ref()
            .is_none_or(|(g, r, _)| (*g, *r) != key)
        {
            let start = Instant::now();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            self.summary = Some((key.0, key.1, Summary::of(library, stats, now)));
            self.took = start.elapsed();
            log::debug!("library statistics in {:?}", self.took);
        }
        let summary = &self.summary.as_ref().expect("set above").2;
        // Over the first frames: the first ones do not know the content's
        // height yet, to which the offset is held.
        let scroll = self.scroll.filter(|_| ctx.cumulative_frame_nr() < 20);
        let mut go = Going {
            allowed: !self.no_library,
            to: None,
        };
        let mut close = false;
        let modal = egui::Modal::new(Id::new("statistics"))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                    .fill(palette::panel_bg())
                    .corner_radius(RADIUS)
                    .inner_margin(Margin::ZERO),
            )
            .show(ctx, |ui| {
                close = page(ui, summary, library, self.took, scroll, &mut go);
            });
        close |= !fresh
            && (modal.backdrop_response.clicked()
                || !egui::Popup::is_any_open(ctx)
                    && ctx.input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    }));
        if close || go.to.is_some() {
            self.open = false;
        }
        go.to.map(Go::action)
    }
}

/// The window: the header with the cross, the cards in a scroll area.
/// True when the cross was clicked.
fn page(
    ui: &mut Ui,
    s: &Summary,
    library: &Library,
    took: Duration,
    scroll: Option<f32>,
    go: &mut Going,
) -> bool {
    let size = super::modal_size(ui.ctx(), SIZE, 12.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
    {
        let painter = ui.painter();
        let accent = palette::accent();
        let badge = Rect::from_center_size(
            pos2(header.left() + PAD + 17.0, header.center().y),
            Vec2::splat(34.0),
        );
        painter.rect_filled(badge, 9.0, accent.gamma_multiply(0.16));
        icons::paint(
            painter,
            Icon::Chart,
            badge.center(),
            18.0,
            accent,
            palette::panel_bg(),
        );
        let x = badge.right() + 12.0;
        painter.text(
            pos2(x, header.center().y - 1.0),
            Align2::LEFT_BOTTOM,
            tr!("Library statistics", "Статистика библиотеки"),
            FontId::proportional(18.0),
            palette::text_bright(),
        );
        let note = tr!(
            format!(
                "The whole library · worked out in {}",
                format::millis_in(Lang::En, took.as_secs_f64())
            ),
            format!(
                "Вся библиотека · подсчитано за {}",
                format::millis_in(Lang::Ru, took.as_secs_f64())
            )
        );
        painter.text(
            pos2(x, header.center().y + 3.0),
            Align2::LEFT_TOP,
            note,
            FontId::proportional(12.0),
            palette::text_weak(),
        );
        painter.hline(
            rect.x_range(),
            header.bottom() - 0.5,
            Stroke::new(1.0, palette::line()),
        );
    }
    let corner = Rect::from_min_size(
        pos2(header.right() - PAD - 20.0, header.center().y - 12.0),
        vec2(24.0, 24.0),
    );
    let closed = super::close_cross(&mut ui.new_child(egui::UiBuilder::new().max_rect(corner)));

    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(y) = scroll {
        area = area.vertical_scroll_offset(y);
    }
    area.show(&mut child, |ui| {
        egui::Frame::NONE
            .inner_margin(Margin::symmetric(PAD as i8, 18))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let style = ui.style_mut();
                style.interaction.tooltip_delay = 0.0;
                style.interaction.show_tooltips_only_when_still = false;
                content(ui, s, library, go);
            });
    });
    closed
}

/// The tiles and the cards, laid out in the content's width.
fn content(ui: &mut Ui, s: &Summary, library: &Library, go: &mut Going) {
    if s.tracks == 0 {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(tr!("The library is empty", "Библиотека пуста"))
                    .size(16.0)
                    .color(palette::text_weak()),
            );
        });
        ui.add_space(80.0);
        return;
    }
    tiles(ui, s);
    ui.add_space(GAP);
    let width = ui.available_width();
    let two = width >= TWO_COLUMNS;
    let cards = cards(s);
    let mut i = 0;
    while i < cards.len() {
        let card = cards[i];
        let pair = two && !card.wide && cards.get(i + 1).is_some_and(|c| !c.wide);
        if pair {
            let next = cards[i + 1];
            let height = card.height.max(next.height);
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let half = (width - GAP) / 2.0;
            let left = Rect::from_min_size(rect.min, vec2(half, height));
            let right =
                Rect::from_min_size(pos2(rect.right() - half, rect.top()), vec2(half, height));
            draw(ui, card.kind, left, s, library, go);
            draw(ui, next.kind, right, s, library, go);
            i += 2;
        } else {
            let (rect, _) = ui.allocate_exact_size(vec2(width, card.height), Sense::hover());
            draw(ui, card.kind, rect, s, library, go);
            i += 1;
        }
        ui.add_space(GAP);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    Bitrates,
    Formats,
    Years,
    Added,
    Lengths,
    Genres,
    Artists,
    Listening,
    TopTracks,
    PlayedArtists,
    Ratings,
    Tags,
    SampleRates,
}

#[derive(Clone, Copy)]
struct Card {
    kind: Kind,
    /// Across the whole width.
    wide: bool,
    height: f32,
}

/// The cards the numbers fill, in their order (pairs side by side).
fn cards(s: &Summary) -> Vec<Card> {
    let card = |kind, height| Card {
        kind,
        wide: false,
        height,
    };
    let rows = |n: usize| CARD_HEAD + 18.0 + n.max(1) as f32 * ROW;
    let mut out = vec![card(Kind::Bitrates, 270.0), card(Kind::Formats, 270.0)];
    if !s.years.is_empty() {
        out.push(Card {
            kind: Kind::Years,
            wide: true,
            height: 270.0,
        });
    }
    if s.added.len() >= 2 {
        out.push(card(Kind::Added, 270.0));
    }
    out.push(card(Kind::Lengths, 270.0));
    if !s.genres.is_empty() {
        out.push(card(Kind::Genres, rows(genre_rows(s))));
    }
    out.push(card(Kind::Artists, rows(s.top_artists.len())));
    // Pairs of about one height: the listening and the ratings, the
    // tracks and the artists played most.
    out.push(card(Kind::Listening, 300.0));
    out.push(card(Kind::Ratings, rows(5)));
    out.push(card(Kind::TopTracks, rows(s.top_tracks.len().max(4))));
    if !s.played_artists.is_empty() {
        out.push(card(Kind::PlayedArtists, rows(s.played_artists.len())));
    }
    out.push(card(Kind::Tags, rows(5)));
    if !s.sample_rates.is_empty() {
        out.push(card(
            Kind::SampleRates,
            rows(s.sample_rates.len().min(RATE_ROWS)),
        ));
    }
    out
}

/// The genres' rows: the first ones, and the rest as one.
fn genre_rows(s: &Summary) -> usize {
    if s.genres.len() > TOP_ROWS {
        TOP_ROWS + 1
    } else {
        s.genres.len()
    }
}

/// A count with its word: `4,942 tracks` / `4 942 трека`.
fn counted(n: u64, en: [&str; 2], ru: [&str; 3]) -> String {
    let number = format::number(n);
    match lang() {
        Lang::En => format!("{number} {}", if n == 1 { en[0] } else { en[1] }),
        Lang::Ru => format!("{number} {}", ru_plural(n, ru[0], ru[1], ru[2])),
    }
}

fn tracks(n: u64) -> String {
    counted(n, ["track", "tracks"], ["трек", "трека", "треков"])
}

fn albums(n: u64) -> String {
    counted(n, ["album", "albums"], ["альбом", "альбома", "альбомов"])
}

fn plays(n: u64) -> String {
    counted(
        n,
        ["play", "plays"],
        ["прослушивание", "прослушивания", "прослушиваний"],
    )
}

/// A long time in days and hours, or hours and minutes: `15 d 16 h`.
fn span(ms: u64) -> String {
    let minutes = ms / 60_000;
    let (d, h, m) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    if d > 0 {
        tr!(format!("{d} d {h} h"), format!("{d} дн. {h} ч"))
    } else if h > 0 {
        tr!(format!("{h} h {m} min"), format!("{h} ч {m} мин"))
    } else {
        tr!(format!("{m} min"), format!("{m} мин"))
    }
}

/// A month (`year * 12 + month - 1`) in words: `March 2021` / `март 2021`.
fn month_name(month: i32) -> String {
    const EN: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const RU: [&str; 12] = [
        "январь",
        "февраль",
        "март",
        "апрель",
        "май",
        "июнь",
        "июль",
        "август",
        "сентябрь",
        "октябрь",
        "ноябрь",
        "декабрь",
    ];
    let (year, m) = (month.div_euclid(12), month.rem_euclid(12) as usize);
    tr!(format!("{} {year}", EN[m]), format!("{} {year}", RU[m]))
}

/// The ground of the mark under the pointer: a faint band.
fn hover_band() -> Color32 {
    palette::text_weak().gamma_multiply(0.10)
}

/// The tiles: the headline numbers, four in a row (two in a narrow window).
fn tiles(ui: &mut Ui, s: &Summary) {
    let avg = |total: u64, n: u64| total.checked_div(n).unwrap_or(0);
    let per = |a: u64, b: u64| {
        let v = if b == 0 { 0.0 } else { a as f64 / b as f64 };
        let text = format!("{v:.1}");
        match lang() {
            Lang::En => text,
            Lang::Ru => text.replace('.', ","),
        }
    };
    let genres = s.genres.len() as u64;
    let items: [(Icon, &str, String, String); 8] = [
        (
            Icon::Note,
            tr!("Tracks", "Треки"),
            format::number(s.tracks),
            tr!(
                format!(
                    "{} a track on average",
                    format::time(avg(s.length_ms, s.tracks))
                ),
                format!(
                    "в среднем {} на трек",
                    format::time(avg(s.length_ms, s.tracks))
                )
            ),
        ),
        (
            Icon::Disc,
            tr!("Albums", "Альбомы"),
            format::number(s.albums),
            tr!(
                format!("{} tracks an album on average", per(s.tracks, s.albums)),
                format!("в среднем {} трека в альбоме", per(s.tracks, s.albums))
            ),
        ),
        (
            Icon::Mic,
            tr!("Artists", "Исполнители"),
            format::number(s.artists),
            tr!(
                format!("{} albums an artist on average", per(s.albums, s.artists)),
                format!(
                    "в среднем {} альбома у исполнителя",
                    per(s.albums, s.artists)
                )
            ),
        ),
        (
            Icon::Globe,
            tr!("Genres", "Жанры"),
            format::number(genres),
            match s.genres.first() {
                Some((name, _)) => tr!(format!("Most: {name}"), format!("Больше всего: {name}")),
                None => tr!("No genres in the tags", "В тегах нет жанров").to_owned(),
            },
        ),
        (
            Icon::Clock,
            tr!("Length", "Длительность"),
            span(s.length_ms),
            format::long_duration(s.length_ms),
        ),
        (
            Icon::Folder,
            tr!("Size", "Размер"),
            format::size(s.bytes),
            tr!(
                format!(
                    "{} a track on average",
                    format::size(avg(s.bytes, s.tracks))
                ),
                format!("в среднем {} на трек", format::size(avg(s.bytes, s.tracks)))
            ),
        ),
        (
            Icon::Play,
            tr!("Plays", "Прослушивания"),
            format::number(s.plays),
            tr!(
                format!("{} played at least once", tracks(s.played)),
                format!("хоть раз сыграно: {}", tracks(s.played))
            ),
        ),
        (
            Icon::Headphones,
            tr!("Time heard", "Время прослушивания"),
            span(s.heard_ms),
            tr!(
                format!(
                    "{} of the library heard",
                    format::percent(s.played, s.tracks)
                ),
                format!(
                    "прослушано {} библиотеки",
                    format::percent(s.played, s.tracks)
                )
            ),
        ),
    ];
    let width = ui.available_width();
    let per_row = if width >= 860.0 { 4 } else { 2 };
    let tile_w = (width - GAP * (per_row - 1) as f32) / per_row as f32;
    for row in items.chunks(per_row) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, TILE), Sense::hover());
        for (i, (icon, label, value, note)) in row.iter().enumerate() {
            let tile = Rect::from_min_size(
                pos2(rect.left() + i as f32 * (tile_w + GAP), rect.top()),
                vec2(tile_w, TILE),
            );
            paint_tile(ui, tile, *icon, label, value, note);
        }
        ui.add_space(GAP);
    }
    // The last row's gap is the cards' own.
    ui.add_space(-GAP);
}

fn paint_tile(ui: &Ui, rect: Rect, icon: Icon, label: &str, value: &str, note: &str) {
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, palette::card_bg());
    let accent = palette::accent();
    let badge = Rect::from_min_size(
        pos2(rect.right() - 16.0 - 30.0, rect.top() + 14.0),
        Vec2::splat(30.0),
    );
    painter.rect_filled(badge, 8.0, accent.gamma_multiply(0.16));
    icons::paint(
        painter,
        icon,
        badge.center(),
        15.0,
        accent,
        palette::card_bg(),
    );
    let left = rect.left() + 16.0;
    let room = badge.left() - 8.0 - left;
    let text = |y: f32, text: &str, size: f32, colour: Color32, width: f32| {
        let g = super::elided(
            painter,
            text.to_owned(),
            FontId::proportional(size),
            colour,
            width,
        );
        painter.galley(pos2(left, y), g, colour);
    };
    text(rect.top() + 14.0, label, 12.5, palette::text_weak(), room);
    text(rect.top() + 34.0, value, 23.0, palette::text_bright(), room);
    text(
        rect.top() + 66.0,
        note,
        11.5,
        palette::text_weak(),
        rect.width() - 32.0,
    );
}

/// A card: its ground, title and note; the rect left for its chart.
fn card_frame(ui: &Ui, rect: Rect, title: &str, note: &str) -> Rect {
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, palette::card_bg());
    let inner = rect.shrink2(vec2(18.0, 0.0));
    painter.text(
        pos2(inner.left(), rect.top() + 16.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(15.0),
        palette::text_bright(),
    );
    let g = super::elided(
        painter,
        note.to_owned(),
        FontId::proportional(12.0),
        palette::text_weak(),
        inner.width(),
    );
    painter.galley(
        pos2(inner.left(), rect.top() + 38.0),
        g,
        palette::text_weak(),
    );
    Rect::from_min_max(
        pos2(inner.left(), rect.top() + CARD_HEAD + 4.0),
        pos2(inner.right(), rect.bottom() - 16.0),
    )
}

/// The formats' colours, fixed per format so that one keeps its colour
/// whatever the others: the reference palette's categorical slots, in
/// their dark or light steps.
fn format_colour(name: &str, dark: bool) -> Color32 {
    const DARK: [u32; 6] = [0x3987e5, 0xd95926, 0x199e70, 0xc98500, 0xd55181, 0x9085e9];
    const LIGHT: [u32; 6] = [0x2a78d6, 0xeb6834, 0x1baf7a, 0xeda100, 0xe87ba4, 0x6250d6];
    let slot = match name {
        "MP3" => 0,
        "FLAC" => 1,
        "M4A" | "AAC" | "MP4" => 2,
        "WMA" => 3,
        "WAV" => 4,
        _ => 5,
    };
    let hex = if dark { DARK[slot] } else { LIGHT[slot] };
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// One card: its title and note, and its chart.
fn draw(ui: &Ui, kind: Kind, rect: Rect, s: &Summary, library: &Library, go: &mut Going) {
    let id = Id::new("statistics").with(kind);
    match kind {
        Kind::Bitrates => {
            let known: u64 = s.bitrates.iter().sum();
            let lossy: u64 = known - s.bitrates[BITRATE_LOSSLESS];
            let high = s.bitrates[5] + s.bitrates[6];
            let mut note = tr!(
                format!(
                    "Lossless: {} · 256 kbps and more: {}",
                    format::percent(s.bitrates[BITRATE_LOSSLESS], known),
                    format::percent(high, lossy.max(1))
                ),
                format!(
                    "Без потерь: {} · от 256 кбит/с: {}",
                    format::percent(s.bitrates[BITRATE_LOSSLESS], known),
                    format::percent(high, lossy.max(1))
                )
            );
            if s.no_bitrate > 0 {
                note += &tr!(
                    format!(" · not known: {}", format::number(s.no_bitrate)),
                    format!(" · неизвестно: {}", format::number(s.no_bitrate))
                );
            }
            let plot = card_frame(ui, rect, tr!("Bit rate", "Битрейт"), &note);
            let bounds = summary::BITRATES;
            let cols: Vec<Column> = (0..=BITRATE_LOSSLESS)
                .map(|i| {
                    let n = s.bitrates[i];
                    let (label, range) = if i == BITRATE_LOSSLESS {
                        (
                            "Lossless".to_owned(),
                            tr!(
                                "Lossless (FLAC, WAV, ALAC, WMA Lossless)",
                                "Без потерь (FLAC, WAV, ALAC, WMA Lossless)"
                            )
                            .to_owned(),
                        )
                    } else if i == 0 {
                        (
                            "<128".to_owned(),
                            tr!("Under 128 kbps", "Меньше 128 кбит/с").to_owned(),
                        )
                    } else if i == bounds.len() - 1 {
                        (bounds[i].to_string(), format::bitrate(bounds[i]))
                    } else {
                        let to = bounds[i + 1] - 1;
                        (
                            bounds[i].to_string(),
                            tr!(
                                format!("{}–{to} kbps", bounds[i]),
                                format!("{}–{to} кбит/с", bounds[i])
                            ),
                        )
                    };
                    Column {
                        label: Some(label),
                        value: n,
                        tip: format!("{range}\n{} · {}", tracks(n), format::percent(n, known)),
                    }
                })
                .collect();
            columns(ui, plot, id, &cols, true);
        }
        Kind::Formats => {
            let total: u64 = s.formats.iter().map(|f| f.tracks).sum();
            let note = tr!(
                format!("{} kinds of files", s.formats.len()),
                format!("Типов файлов: {}", s.formats.len())
            );
            let plot = card_frame(ui, rect, tr!("Formats", "Форматы"), &note);
            let dark = palette::current().dark;
            // Five at most, the rest as one.
            let mut slices: Vec<Slice> = s
                .formats
                .iter()
                .take(5)
                .map(|f| Slice {
                    name: f.name.clone(),
                    value: f.tracks,
                    detail: format!(
                        "{}\n{} · {}\n{}",
                        f.name,
                        tracks(f.tracks),
                        format::percent(f.tracks, total),
                        format::size(f.bytes)
                    ),
                    colour: format_colour(&f.name, dark),
                })
                .collect();
            if s.formats.len() > 5 {
                let rest: Vec<_> = s.formats[5..].iter().collect();
                let n: u64 = rest.iter().map(|f| f.tracks).sum();
                let bytes: u64 = rest.iter().map(|f| f.bytes).sum();
                let names: Vec<&str> = rest.iter().map(|f| f.name.as_str()).collect();
                slices.push(Slice {
                    name: tr!("Other", "Другие").to_owned(),
                    value: n,
                    detail: format!(
                        "{}\n{} · {}\n{}",
                        names.join(", "),
                        tracks(n),
                        format::percent(n, total),
                        format::size(bytes)
                    ),
                    colour: palette::text_weak(),
                });
            }
            let centre = (format::number(total), tr!("tracks", "треков").to_owned());
            donut(ui, plot, id, &slices, total, centre);
        }
        Kind::Years => {
            let busiest = s
                .years
                .iter()
                .max_by_key(|(y, n)| (*n, std::cmp::Reverse(*y)));
            let mut note = match (s.years.first(), s.years.last(), busiest) {
                (Some((first, _)), Some((last, _)), Some((year, n))) => tr!(
                    format!(
                        "From {first} to {last} · the most in {year}: {}",
                        albums(*n)
                    ),
                    format!(
                        "С {first} по {last} · больше всего в {year}: {}",
                        albums(*n)
                    )
                ),
                _ => String::new(),
            };
            if s.no_year > 0 {
                note += &tr!(
                    format!(" · without a year: {}", format::number(s.no_year)),
                    format!(" · без года: {}", format::number(s.no_year))
                );
            }
            let plot = card_frame(ui, rect, tr!("Albums by year", "Альбомы по годам"), &note);
            let slot = (plot.width() - 40.0) / s.years.len() as f32;
            let step = [1u16, 2, 5, 10, 20, 50]
                .into_iter()
                .find(|&k| slot * f32::from(k) >= 40.0)
                .unwrap_or(100);
            let cols: Vec<Column> = s
                .years
                .iter()
                .map(|&(year, n)| Column {
                    label: (year % step == 0).then(|| year.to_string()),
                    value: n,
                    tip: format!("{year}\n{}", albums(n)),
                })
                .collect();
            columns(ui, plot, id, &cols, slot >= 26.0);
        }
        Kind::Added => {
            let busiest = s
                .added
                .iter()
                .max_by_key(|(m, n)| (*n, std::cmp::Reverse(*m)));
            let note = match busiest {
                Some((m, n)) => tr!(
                    format!(
                        "By when the files came in · the most in {}: {}",
                        month_name(*m),
                        tracks(*n)
                    ),
                    format!(
                        "По времени появления файлов · больше всего — {}: {}",
                        month_name(*m),
                        tracks(*n)
                    )
                ),
                None => String::new(),
            };
            let plot = card_frame(
                ui,
                rect,
                tr!("The library's growth", "Рост библиотеки"),
                &note,
            );
            growth(ui, plot, id, &s.added);
        }
        Kind::Lengths => {
            let note = tr!(
                format!(
                    "Minutes · {} a track on average",
                    format::time(s.length_ms / s.tracks.max(1))
                ),
                format!(
                    "Минуты · в среднем {} на трек",
                    format::time(s.length_ms / s.tracks.max(1))
                )
            );
            let plot = card_frame(ui, rect, tr!("Track lengths", "Длина треков"), &note);
            let b = summary::LENGTHS;
            let cols: Vec<Column> = (0..b.len())
                .map(|i| {
                    let label = if i == 0 {
                        format!("<{}", b[1])
                    } else if i == b.len() - 1 {
                        format!("{}+", b[i])
                    } else {
                        format!("{}–{}", b[i], b[i + 1])
                    };
                    let n = s.lengths[i];
                    let range = tr!(format!("{label} min"), format!("{label} мин"));
                    Column {
                        label: Some(label),
                        value: n,
                        tip: format!("{range}\n{} · {}", tracks(n), format::percent(n, s.tracks)),
                    }
                })
                .collect();
            columns(ui, plot, id, &cols, true);
        }
        Kind::Genres => {
            let mut note = tr!(
                format!(
                    "Tracks by genre · {} genres",
                    format::number(s.genres.len() as u64)
                ),
                format!(
                    "Треки по жанрам · всего жанров: {}",
                    format::number(s.genres.len() as u64)
                )
            );
            if s.no_genre > 0 {
                note += &tr!(
                    format!(" · without one: {}", format::number(s.no_genre)),
                    format!(" · без жанра: {}", format::number(s.no_genre))
                );
            }
            let plot = card_frame(ui, rect, tr!("Genres", "Жанры"), &note);
            let mut rows: Vec<Row> = s
                .genres
                .iter()
                .take(TOP_ROWS)
                .map(|(name, n)| Row {
                    label: Label::Text(name.clone()),
                    value: *n,
                    text: format::number(*n),
                    tip: format!("{name}\n{} · {}", tracks(*n), format::percent(*n, s.tracks)),
                    go: None,
                })
                .collect();
            if s.genres.len() > TOP_ROWS {
                let rest = &s.genres[TOP_ROWS..];
                let n: u64 = rest.iter().map(|(_, n)| n).sum();
                let label = tr!(
                    format!("{} more genres", rest.len()),
                    format!("Ещё жанров: {}", rest.len())
                );
                let named: Vec<&str> = rest.iter().take(8).map(|(g, _)| g.as_str()).collect();
                let more = if rest.len() > 8 { ", …" } else { "" };
                rows.push(Row {
                    label: Label::Rest(label),
                    value: n,
                    text: format::number(n),
                    tip: format!("{}{more}\n{}", named.join(", "), tracks(n)),
                    go: None,
                });
            }
            hbars(ui, plot, id, &rows, LABELS, go);
        }
        Kind::Artists => {
            let note = tr!(
                "By their tracks · a click goes to the artist",
                "По числу треков · щелчок — к исполнителю"
            );
            let plot = card_frame(ui, rect, tr!("Artists", "Исполнители"), note);
            let rows = artist_rows(&s.top_artists, |a| u64::from(a.tracks));
            hbars(ui, plot, id, &rows, LABELS, go);
        }
        Kind::PlayedArtists => {
            let note = tr!(
                "By their plays · a click goes to the artist",
                "По прослушиваниям · щелчок — к исполнителю"
            );
            let plot = card_frame(
                ui,
                rect,
                tr!("Artists heard most", "Самые слушаемые исполнители"),
                note,
            );
            let rows = artist_rows(&s.played_artists, |a| a.plays);
            hbars(ui, plot, id, &rows, LABELS, go);
        }
        Kind::Listening => {
            let note = tr!(
                format!("{} · {} heard", plays(s.plays), span(s.heard_ms)),
                format!("{} · {} звучания", plays(s.plays), span(s.heard_ms))
            );
            let plot = card_frame(ui, rect, tr!("Listening", "Прослушивание"), &note);
            let meter_rect = Rect::from_min_size(plot.min, vec2(plot.width(), ROW));
            meters(
                ui,
                meter_rect,
                id.with("played"),
                &[Meter {
                    label: tr!("Played at least once", "Сыграно хоть раз").to_owned(),
                    part: s.played,
                    whole: s.tracks,
                    tip: tr!(
                        format!("{} of {}", format::number(s.played), tracks(s.tracks)),
                        format!("{} из {}", format::number(s.played), tracks(s.tracks))
                    ),
                }],
            );
            let names = [
                tr!("This week", "За неделю"),
                tr!("1–4 weeks", "1–4 недели"),
                tr!("1–12 months", "1–12 месяцев"),
                tr!("Over a year", "Больше года"),
            ];
            let tips = [
                tr!(
                    "Last played within the last week",
                    "Последний раз — за последнюю неделю"
                ),
                tr!(
                    "Last played one to four weeks ago",
                    "Последний раз — от недели до месяца назад"
                ),
                tr!(
                    "Last played one to twelve months ago",
                    "Последний раз — от месяца до года назад"
                ),
                tr!(
                    "Last played over a year ago",
                    "Последний раз — больше года назад"
                ),
            ];
            let cols: Vec<Column> = (0..4)
                .map(|i| Column {
                    label: Some(names[i].to_owned()),
                    value: s.recency[i],
                    tip: format!("{}\n{}", tips[i], tracks(s.recency[i])),
                })
                .collect();
            let rest = Rect::from_min_max(pos2(plot.left(), plot.top() + ROW + 16.0), plot.max);
            painter_label(
                ui,
                pos2(rest.left(), rest.top() - 6.0),
                tr!(
                    "The tracks played, by their last play",
                    "Сыгранные треки по последнему прослушиванию"
                ),
            );
            let rest = Rect::from_min_max(pos2(rest.left(), rest.top() + 12.0), rest.max);
            columns(ui, rest, id.with("recency"), &cols, true);
        }
        Kind::TopTracks => {
            let note = tr!(
                "By their plays · a click goes to the track",
                "По прослушиваниям · щелчок — к треку"
            );
            let plot = card_frame(ui, rect, tr!("Played most", "Чаще всего"), note);
            if s.top_tracks.is_empty() {
                empty(
                    ui,
                    plot,
                    tr!(
                        "Nothing has been played yet: a play counts after half the track or four minutes",
                        "Пока ничего не сыграно: прослушивание засчитывается после половины трека или четырёх минут"
                    ),
                );
            } else {
                let rows = track_rows(s, library);
                hbars(ui, plot, id, &rows, 0.6, go);
            }
        }
        Kind::Ratings => {
            let note = tr!(
                format!("Rated: {} of {}", format::number(s.rated), tracks(s.tracks)),
                format!(
                    "С оценкой: {} из {}",
                    format::number(s.rated),
                    tracks(s.tracks)
                )
            );
            let plot = card_frame(ui, rect, tr!("Ratings", "Оценки"), &note);
            let rows: Vec<Row> = (1..=5u8)
                .rev()
                .map(|stars| {
                    let n = s.ratings[usize::from(stars)];
                    Row {
                        label: Label::Stars(stars),
                        value: n,
                        text: format::number(n),
                        tip: format!(
                            "{}\n{} · {}",
                            tr!(format!("{stars} of 5 stars"), format!("{stars} из 5 звёзд")),
                            tracks(n),
                            format::percent(n, s.rated.max(1))
                        ),
                        go: None,
                    }
                })
                .collect();
            if s.rated == 0 {
                empty(
                    ui,
                    plot,
                    tr!(
                        "No ratings yet: tracks are rated from their menu in the library",
                        "Оценок пока нет: треки оцениваются из их меню в библиотеке"
                    ),
                );
            } else {
                hbars(ui, plot, id, &rows, LABELS, go);
            }
        }
        Kind::Tags => {
            let note = tr!(
                "How many tracks have them in their tags",
                "Сколько треков имеют их в тегах"
            );
            let plot = card_frame(ui, rect, tr!("Tags", "Теги"), note);
            let t = s.tagged;
            let meter = |label: &str, part: u64| Meter {
                label: label.to_owned(),
                part,
                whole: s.tracks,
                tip: tr!(
                    format!("{} of {}", format::number(part), tracks(s.tracks)),
                    format!("{} из {}", format::number(part), tracks(s.tracks))
                ),
            };
            let rows = [
                meter(tr!("Year", "Год"), t.year),
                meter(tr!("Genre", "Жанр"), t.genre),
                meter(tr!("Album artist", "Исполнитель альбома"), t.album_artist),
                meter(tr!("Track number", "Номер трека"), t.number),
                meter(tr!("Cover in the tag", "Обложка в теге"), t.cover),
            ];
            meters(ui, plot, id, &rows);
        }
        Kind::SampleRates => {
            let [mono, stereo, more] = s.channels;
            let mut note = tr!(
                format!("Stereo: {}", format::number(stereo)),
                format!("Стерео: {}", format::number(stereo))
            );
            if mono > 0 {
                note += &tr!(
                    format!(" · mono: {}", format::number(mono)),
                    format!(" · моно: {}", format::number(mono))
                );
            }
            if more > 0 {
                note += &tr!(
                    format!(" · more channels: {}", format::number(more)),
                    format!(" · многоканальных: {}", format::number(more))
                );
            }
            let plot = card_frame(ui, rect, tr!("Sample rate", "Частота дискретизации"), &note);
            let rows: Vec<Row> = s
                .sample_rates
                .iter()
                .take(RATE_ROWS)
                .map(|&(hz, n)| Row {
                    label: Label::Text(format::sample_rate(hz)),
                    value: n,
                    text: format::number(n),
                    tip: format!(
                        "{}\n{} · {}",
                        format::sample_rate(hz),
                        tracks(n),
                        format::percent(n, s.tracks)
                    ),
                    go: None,
                })
                .collect();
            hbars(ui, plot, id, &rows, LABELS, go);
        }
    }
}

/// A card's chart left empty: why, in the middle of its first 120 points
/// (a card beside a taller one is taller than its chart).
fn empty(ui: &Ui, rect: Rect, text: &str) {
    let rect = Rect::from_min_size(rect.min, vec2(rect.width(), rect.height().min(120.0)));
    let painter = ui.painter();
    let g = painter.layout(
        text.to_owned(),
        FontId::proportional(12.5),
        palette::text_weak(),
        (rect.width() - 40.0).max(80.0),
    );
    let at = rect.center() - g.size() / 2.0;
    painter.galley(at, g, palette::text_weak());
}

/// The rows of the tracks played most.
fn track_rows(s: &Summary, library: &Library) -> Vec<Row> {
    s.top_tracks
        .iter()
        .enumerate()
        .filter_map(|(i, &(track, n))| {
            let t = library.tracks.get(track as usize)?;
            let title = if t.title.trim().is_empty() {
                t.path
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                t.title.trim().to_owned()
            };
            let artist = if t.artist.trim().is_empty() {
                crate::library::unknown_artist().to_owned()
            } else {
                t.artist.trim().to_owned()
            };
            let album = if t.album.trim().is_empty() {
                String::new()
            } else {
                format!(" — {}", t.album.trim())
            };
            let n = u64::from(n);
            Some(Row {
                tip: format!("{title}\n{artist}{album}\n{}", plays(n)),
                label: Label::Track(i + 1, title, artist),
                value: n,
                text: format::number(n),
                go: library
                    .album_of
                    .get(track as usize)
                    .map(|&album| Go::Track(album, track)),
            })
        })
        .collect()
}

/// A line of weak text over a part of a card.
fn painter_label(ui: &Ui, pos: Pos2, text: &str) {
    ui.painter().text(
        pos,
        Align2::LEFT_TOP,
        text,
        FontId::proportional(11.5),
        palette::text_weak(),
    );
}

fn artist_rows(
    artists: &[summary::TopArtist],
    value: impl Fn(&summary::TopArtist) -> u64,
) -> Vec<Row> {
    artists
        .iter()
        .map(|a| {
            let name = if a.name.is_empty() {
                crate::library::unknown_artist().to_owned()
            } else {
                a.name.clone()
            };
            let v = value(a);
            Row {
                tip: format!(
                    "{name}\n{} · {} · {}\n{}",
                    albums(u64::from(a.albums)),
                    tracks(u64::from(a.tracks)),
                    span(a.length_ms),
                    plays(a.plays)
                ),
                label: Label::Text(name),
                value: v,
                text: format::number(v),
                go: Some(Go::Artist(a.album)),
            }
        })
        .collect()
}

/// The steps of a count axis up to `max`: its top and the step between
/// its lines, 1, 2 or 5 times a power of ten (2.5 past ten), about four.
fn ticks(max: u64) -> (u64, u64) {
    let max = max.max(1);
    let raw = (max as f64 / 4.0).max(1.0);
    let magnitude = 10f64.powf(raw.log10().floor());
    let steps: &[f64] = if magnitude >= 10.0 {
        &[1.0, 2.0, 2.5, 5.0, 10.0]
    } else {
        &[1.0, 2.0, 5.0, 10.0]
    };
    let step = steps
        .iter()
        .map(|m| m * magnitude)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * magnitude);
    let step = (step.round() as u64).max(1);
    (max.div_ceil(step) * step, step)
}

struct Column {
    /// Under it on the axis, if any.
    label: Option<String>,
    value: u64,
    tip: String,
}

/// A column chart from a base line: the counts' lines and their numbers
/// on the left, a column per value (at most 24 points thick, rounded at
/// the top), its label under it, its count over it when `caps` and it
/// fits; the column under the pointer lit, with its tip.
fn columns(ui: &Ui, rect: Rect, id: Id, cols: &[Column], caps: bool) {
    let painter = ui.painter();
    let font = FontId::proportional(11.0);
    let (top, step) = ticks(cols.iter().map(|c| c.value).max().unwrap_or(0));
    let axis = painter
        .layout_no_wrap(format::compact(top), font.clone(), palette::text_weak())
        .size()
        .x
        + 10.0;
    let labels = cols.iter().any(|c| c.label.is_some());
    let plot = Rect::from_min_max(
        pos2(
            rect.left() + axis,
            rect.top() + if caps { 16.0 } else { 6.0 },
        ),
        pos2(
            rect.right(),
            rect.bottom() - if labels { 20.0 } else { 0.0 },
        ),
    );
    if plot.height() < 10.0 || plot.width() < 10.0 {
        return;
    }
    let y_of = |v: u64| plot.bottom() - v as f32 / top as f32 * plot.height();
    let mut v = 0;
    while v <= top {
        let y = y_of(v).round() + 0.5;
        painter.hline(plot.x_range(), y, Stroke::new(1.0, palette::line()));
        painter.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format::compact(v),
            font.clone(),
            palette::text_weak(),
        );
        v += step;
    }
    let n = cols.len().max(1);
    let slot = plot.width() / n as f32;
    let bar_w = if slot < 4.0 {
        (slot - 1.0).max(1.0)
    } else {
        (slot * 0.62).clamp(2.0, 24.0)
    };
    let (accent, lit) = (palette::accent(), palette::accent_hover());
    for (i, c) in cols.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let band = Rect::from_x_y_ranges(
            cx - slot / 2.0..=cx + slot / 2.0,
            rect.top()..=plot.bottom(),
        );
        let resp = ui.interact(band, id.with(i), Sense::hover());
        let hovered = resp.hovered();
        if hovered {
            painter.rect_filled(band, 4.0, hover_band());
        }
        let h = plot.bottom() - y_of(c.value);
        if c.value > 0 {
            let bar = Rect::from_min_max(
                pos2(cx - bar_w / 2.0, plot.bottom() - h.max(1.5)),
                pos2(cx + bar_w / 2.0, plot.bottom()),
            );
            let r = (bar_w / 2.0).min(4.0).min(h).floor() as u8;
            let corners = CornerRadius {
                nw: r,
                ne: r,
                sw: 0,
                se: 0,
            };
            painter.rect_filled(bar, corners, if hovered { lit } else { accent });
            if caps {
                let g =
                    painter.layout_no_wrap(format::number(c.value), font.clone(), palette::text());
                if g.size().x <= slot - 2.0 {
                    let at = pos2(cx - g.size().x / 2.0, bar.top() - g.size().y - 2.0);
                    painter.galley(at, g, palette::text());
                }
            }
        }
        if let Some(label) = &c.label {
            // Smaller where it would not fit between its neighbours.
            let g = painter.layout_no_wrap(label.clone(), font.clone(), palette::text_weak());
            let labelled = |j: &usize| cols[*j].label.is_some();
            let before = (0..i).rev().find(labelled).map_or(n, |j| i - j);
            let after = (i + 1..n).find(labelled).map_or(n, |j| j - i);
            let room = slot * before.min(after) as f32;
            let size = if g.size().x > room - 4.0 && n > 1 {
                (11.0 * (room - 4.0) / g.size().x).max(8.5)
            } else {
                11.0
            };
            painter.text(
                pos2(cx, plot.bottom() + 5.0),
                Align2::CENTER_TOP,
                label,
                FontId::proportional(size),
                if hovered {
                    palette::text()
                } else {
                    palette::text_weak()
                },
            );
        }
        resp.on_hover_ui_at_pointer(|ui| {
            ui.label(&c.tip);
        });
    }
}

enum Label {
    Text(String),
    Stars(u8),
    /// A track: its rank, title and artist (weaker).
    Track(usize, String, String),
    /// The rest as one: its text and value without a bar, whose length
    /// would dwarf the others'.
    Rest(String),
}

struct Row {
    label: Label,
    value: u64,
    /// The value as shown at the bar's end.
    text: String,
    tip: String,
    /// Where a click goes, if anywhere.
    go: Option<Go>,
}

/// Horizontal bars: the labels on the left (`labels` of the width), a bar
/// per row from a common start (at most 12 points thick, rounded at its
/// end) with its value after it; the row under the pointer lit, with its
/// tip; a click on a row that leads somewhere goes there.
fn hbars(ui: &Ui, rect: Rect, id: Id, rows: &[Row], labels: f32, go: &mut Going) {
    let painter = ui.painter();
    let font = FontId::proportional(12.5);
    let row_h = (rect.height() / rows.len().max(1) as f32).min(ROW);
    let label_w = (rect.width() * labels).clamp(70.0, 200.0f32.max(rect.width() * labels.min(0.6)));
    let value_w = rows
        .iter()
        .map(|r| {
            painter
                .layout_no_wrap(r.text.clone(), font.clone(), palette::text())
                .size()
                .x
        })
        .fold(0.0, f32::max)
        + 12.0;
    let max = rows
        .iter()
        .filter(|r| !matches!(r.label, Label::Rest(_)))
        .map(|r| r.value)
        .max()
        .unwrap_or(0)
        .max(1);
    let bar_left = rect.left() + label_w + 10.0;
    let room = (rect.right() - value_w - bar_left).max(10.0);
    let (accent, lit) = (palette::accent(), palette::accent_hover());
    for (i, r) in rows.iter().enumerate() {
        let leads = r.go.filter(|_| go.allowed);
        let row = Rect::from_min_size(
            pos2(rect.left(), rect.top() + i as f32 * row_h),
            vec2(rect.width(), row_h),
        );
        let area = row.expand2(vec2(8.0, 0.0));
        let sense = if leads.is_some() {
            Sense::click()
        } else {
            Sense::hover()
        };
        let resp = ui.interact(area, id.with(i), sense);
        let hovered = resp.hovered();
        if hovered {
            painter.rect_filled(area, 5.0, hover_band());
        }
        let y = row.center().y;
        match &r.label {
            Label::Text(text) | Label::Rest(text) => {
                let g = super::elided(
                    painter,
                    text.clone(),
                    font.clone(),
                    palette::text(),
                    label_w,
                );
                painter.galley(pos2(row.left(), y - g.size().y / 2.0), g, palette::text());
            }
            Label::Track(rank, title, artist) => {
                const RANK: f32 = 24.0;
                painter.text(
                    pos2(row.left() + RANK - 10.0, y),
                    Align2::RIGHT_CENTER,
                    rank.to_string(),
                    font.clone(),
                    palette::text_weak(),
                );
                let room = label_w - RANK;
                let g = super::elided(painter, title.clone(), font.clone(), palette::text(), room);
                let title_w = g.size().x;
                painter.galley(
                    pos2(row.left() + RANK, y - g.size().y / 2.0),
                    g,
                    palette::text(),
                );
                let rest = room - title_w - 6.0;
                if rest > 24.0 {
                    let g = super::elided(
                        painter,
                        format!("— {artist}"),
                        font.clone(),
                        palette::text_weak(),
                        rest,
                    );
                    painter.galley(
                        pos2(row.left() + RANK + title_w + 6.0, y - g.size().y / 2.0),
                        g,
                        palette::text_weak(),
                    );
                }
            }
            Label::Stars(n) => {
                for k in 0..*n {
                    icons::paint(
                        painter,
                        Icon::StarFilled,
                        pos2(row.left() + 6.0 + f32::from(k) * 14.0, y),
                        12.0,
                        palette::text(),
                        palette::card_bg(),
                    );
                }
            }
        }
        let w = if r.value == 0 || matches!(r.label, Label::Rest(_)) {
            0.0
        } else {
            (r.value as f32 / max as f32 * room).max(2.0)
        };
        let thick = (row_h * 0.46).min(12.0);
        if w > 0.0 {
            let radius = (w / 2.0).min(4.0).floor() as u8;
            painter.rect_filled(
                Rect::from_min_size(pos2(bar_left, y - thick / 2.0), vec2(w, thick)),
                CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: radius,
                    se: radius,
                },
                if hovered { lit } else { accent },
            );
        }
        painter.text(
            pos2(bar_left + w + 6.0, y),
            Align2::LEFT_CENTER,
            &r.text,
            font.clone(),
            if hovered {
                palette::text()
            } else {
                palette::text_weak()
            },
        );
        let mut resp = resp.on_hover_ui_at_pointer(|ui| {
            ui.label(&r.tip);
        });
        if leads.is_some() {
            resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        }
        if resp.clicked() {
            go.to = leads;
        }
    }
}

struct Meter {
    label: String,
    part: u64,
    whole: u64,
    tip: String,
}

/// Shares as meters: the label, a track filled in the accent as far as
/// the share goes, the share in percent.
fn meters(ui: &Ui, rect: Rect, id: Id, rows: &[Meter]) {
    let painter = ui.painter();
    let font = FontId::proportional(12.5);
    let row_h = (rect.height() / rows.len().max(1) as f32).min(ROW + 4.0);
    let label_w = (rect.width() * 0.36).clamp(70.0, 200.0);
    let value_w = 52.0;
    let left = rect.left() + label_w + 10.0;
    let room = (rect.right() - value_w - left).max(10.0);
    for (i, m) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(rect.left(), rect.top() + i as f32 * row_h),
            vec2(rect.width(), row_h),
        );
        let area = row.expand2(vec2(8.0, 0.0));
        let resp = ui.interact(area, id.with(i), Sense::hover());
        if resp.hovered() {
            painter.rect_filled(area, 5.0, hover_band());
        }
        let y = row.center().y;
        let g = super::elided(
            painter,
            m.label.clone(),
            font.clone(),
            palette::text(),
            label_w,
        );
        painter.galley(pos2(row.left(), y - g.size().y / 2.0), g, palette::text());
        let track = Rect::from_min_size(pos2(left, y - 4.0), vec2(room, 8.0));
        painter.rect_filled(track, 4.0, palette::track());
        let share = if m.whole == 0 {
            0.0
        } else {
            m.part as f32 / m.whole as f32
        };
        if share > 0.0 {
            let filled = Rect::from_min_size(track.min, vec2((room * share).max(8.0), 8.0));
            painter.rect_filled(filled, 4.0, palette::accent());
        }
        painter.text(
            pos2(rect.right(), y),
            Align2::RIGHT_CENTER,
            format::percent(m.part, m.whole),
            font.clone(),
            palette::text(),
        );
        resp.on_hover_ui_at_pointer(|ui| {
            ui.label(&m.tip);
        });
    }
}

struct Slice {
    name: String,
    value: u64,
    detail: String,
    colour: Color32,
}

/// A ring of shares with its legend on the right: a 2-point gap between
/// the slices, the slice under the pointer (or its legend's row) grown;
/// the total in the middle.
fn donut(ui: &Ui, rect: Rect, id: Id, slices: &[Slice], total: u64, centre: (String, String)) {
    let painter = ui.painter();
    let total = total.max(1);
    let d = rect.height().min(rect.width() * 0.45);
    let outer = d / 2.0 - 5.0;
    let inner = outer * 0.62;
    let c = pos2(rect.left() + d / 2.0, rect.center().y);
    let gap = if slices.len() > 1 {
        2.0 / ((inner + outer) / 2.0)
    } else {
        0.0
    };
    let sweeps: Vec<f32> = slices
        .iter()
        .map(|s| s.value as f32 / total as f32 * TAU)
        .collect();

    // The slice under the pointer: on the ring, or its legend's row.
    let ring = ui.interact(
        Rect::from_center_size(c, Vec2::splat(d)),
        id,
        Sense::hover(),
    );
    let mut hovered = None;
    if let Some(p) = ring.hover_pos() {
        let v = p - c;
        if (inner..=outer + 6.0).contains(&v.length()) {
            let mut a = v.y.atan2(v.x) + FRAC_PI_2;
            if a < 0.0 {
                a += TAU;
            }
            let mut start = 0.0;
            for (i, sweep) in sweeps.iter().enumerate() {
                if a < start + sweep {
                    hovered = Some(i);
                    break;
                }
                start += sweep;
            }
        }
    }
    let legend_left = c.x + d / 2.0 + 24.0;
    let lh = 26.0;
    let legend_top = rect.center().y - lh * slices.len() as f32 / 2.0;
    let rows: Vec<Rect> = (0..slices.len())
        .map(|i| {
            Rect::from_min_max(
                pos2(legend_left, legend_top + i as f32 * lh),
                pos2(rect.right(), legend_top + (i + 1) as f32 * lh),
            )
        })
        .collect();
    let legend: Vec<egui::Response> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            ui.interact(
                r.expand2(vec2(6.0, 0.0)),
                id.with(("legend", i)),
                Sense::hover(),
            )
        })
        .collect();
    if let Some(i) = legend.iter().position(egui::Response::hovered) {
        hovered = Some(i);
    }

    let mut start = -FRAC_PI_2;
    for (i, (s, sweep)) in slices.iter().zip(&sweeps).enumerate() {
        let (a0, a1) = (start + gap / 2.0, start + sweep - gap / 2.0);
        start += sweep;
        if a1 <= a0 {
            continue;
        }
        let grow = if hovered == Some(i) { 5.0 } else { 0.0 };
        let (r_in, r_out) = (inner, outer + grow);
        let mid = (r_in + r_out) / 2.0;
        let steps = ((a1 - a0) / TAU * 160.0).ceil().max(2.0) as usize;
        let points: Vec<Pos2> = (0..=steps)
            .map(|k| {
                let a = a0 + (a1 - a0) * k as f32 / steps as f32;
                c + mid * vec2(a.cos(), a.sin())
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(r_out - r_in, s.colour)));
    }
    painter.text(
        c - vec2(0.0, 7.0),
        Align2::CENTER_CENTER,
        &centre.0,
        FontId::proportional(20.0),
        palette::text_bright(),
    );
    painter.text(
        c + vec2(0.0, 13.0),
        Align2::CENTER_CENTER,
        &centre.1,
        FontId::proportional(11.5),
        palette::text_weak(),
    );
    if let Some(i) = hovered
        && ring.hovered()
    {
        ring.on_hover_ui_at_pointer(|ui| {
            ui.label(&slices[i].detail);
        });
    }

    let font = FontId::proportional(12.5);
    for ((s, row), resp) in slices.iter().zip(&rows).zip(legend) {
        let lit = resp.hovered();
        if lit {
            painter.rect_filled(row.expand2(vec2(6.0, 0.0)), 5.0, hover_band());
        }
        let y = row.center().y;
        painter.circle_filled(pos2(row.left() + 5.0, y), 5.0, s.colour);
        painter.text(
            pos2(row.left() + 18.0, y),
            Align2::LEFT_CENTER,
            &s.name,
            font.clone(),
            palette::text(),
        );
        painter.text(
            pos2(row.right(), y),
            Align2::RIGHT_CENTER,
            format!(
                "{} · {}",
                format::number(s.value),
                format::percent(s.value, total)
            ),
            font.clone(),
            if lit {
                palette::text()
            } else {
                palette::text_weak()
            },
        );
        resp.on_hover_ui_at_pointer(|ui| {
            ui.label(&s.detail);
        });
    }
}

/// The tracks added over the months as an area: the running total as a
/// line over a faint wash, the years along the bottom, the total at its
/// end; under the pointer a line across the month, its point and its
/// numbers.
fn growth(ui: &Ui, rect: Rect, id: Id, added: &[(i32, u64)]) {
    let painter = ui.painter();
    let font = FontId::proportional(11.0);
    let mut total = 0;
    let running: Vec<u64> = added
        .iter()
        .map(|(_, n)| {
            total += n;
            total
        })
        .collect();
    let (top, step) = ticks(total);
    let axis = painter
        .layout_no_wrap(format::compact(top), font.clone(), palette::text_weak())
        .size()
        .x
        + 10.0;
    let plot = Rect::from_min_max(
        pos2(rect.left() + axis, rect.top() + 16.0),
        pos2(rect.right() - 6.0, rect.bottom() - 20.0),
    );
    if plot.height() < 10.0 || plot.width() < 10.0 || added.len() < 2 {
        return;
    }
    let y_of = |v: u64| plot.bottom() - v as f32 / top as f32 * plot.height();
    let x_of = |i: usize| plot.left() + i as f32 / (added.len() - 1) as f32 * plot.width();
    let mut v = 0;
    while v <= top {
        let y = y_of(v).round() + 0.5;
        painter.hline(plot.x_range(), y, Stroke::new(1.0, palette::line()));
        painter.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format::compact(v),
            font.clone(),
            palette::text_weak(),
        );
        v += step;
    }
    // The years at their Januaries, as many apart as leave room.
    let per_year = plot.width() / (added.len() as f32 / 12.0);
    let every = [1, 2, 5, 10, 20]
        .into_iter()
        .find(|&k| per_year * k as f32 >= 44.0)
        .unwrap_or(50);
    let mut labelled = false;
    for (i, (month, _)) in added.iter().enumerate() {
        let year = month.div_euclid(12);
        if month.rem_euclid(12) == 0 && year % every == 0 {
            let x = x_of(i);
            painter.vline(
                x,
                plot.bottom()..=plot.bottom() + 3.0,
                Stroke::new(1.0, palette::line()),
            );
            painter.text(
                pos2(x, plot.bottom() + 5.0),
                Align2::CENTER_TOP,
                year.to_string(),
                font.clone(),
                palette::text_weak(),
            );
            labelled = true;
        }
    }
    if !labelled {
        painter.text(
            pos2(plot.left(), plot.bottom() + 5.0),
            Align2::LEFT_TOP,
            month_name(added[0].0),
            font.clone(),
            palette::text_weak(),
        );
    }

    let accent = palette::accent();
    let points: Vec<Pos2> = running
        .iter()
        .enumerate()
        .map(|(i, &v)| pos2(x_of(i), y_of(v)))
        .collect();
    let mut wash = egui::Mesh::default();
    let fill = accent.gamma_multiply(0.14);
    for pair in points.windows(2) {
        let base = wash.vertices.len() as u32;
        wash.colored_vertex(pair[0], fill);
        wash.colored_vertex(pair[1], fill);
        wash.colored_vertex(pos2(pair[0].x, plot.bottom()), fill);
        wash.colored_vertex(pos2(pair[1].x, plot.bottom()), fill);
        wash.add_triangle(base, base + 1, base + 2);
        wash.add_triangle(base + 2, base + 1, base + 3);
    }
    painter.add(wash);
    painter.add(Shape::line(points.clone(), Stroke::new(2.0, accent)));
    let end = *points.last().expect("two at least");
    let dot = |at: Pos2| {
        painter.circle_filled(at, 6.0, palette::card_bg());
        painter.circle_filled(at, 4.0, accent);
    };
    dot(end);
    painter.text(
        end - vec2(8.0, 8.0),
        Align2::RIGHT_BOTTOM,
        format::number(total),
        font.clone(),
        palette::text(),
    );

    let resp = ui.interact(plot.expand2(vec2(4.0, 12.0)), id, Sense::hover());
    if let Some(p) = resp.hover_pos() {
        let t = ((p.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        let i = (t * (added.len() - 1) as f32).round() as usize;
        let at = points[i];
        painter.vline(
            at.x,
            plot.y_range(),
            Stroke::new(1.0, palette::text_weak().gamma_multiply(0.6)),
        );
        dot(at);
        let (month, n) = added[i];
        let tip = format!(
            "{}\n{}\n{}",
            month_name(month),
            tr!(
                format!("Added: {}", tracks(n)),
                format!("Добавлено: {}", tracks(n))
            ),
            tr!(
                format!("In all: {}", tracks(running[i])),
                format!("Всего: {}", tracks(running[i]))
            )
        );
        resp.on_hover_ui_at_pointer(|ui| {
            ui.label(tip);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_steps_are_round() {
        assert_eq!(ticks(0), (1, 1));
        assert_eq!(ticks(1), (1, 1));
        assert_eq!(ticks(10), (10, 5));
        assert_eq!(ticks(23), (30, 10));
        assert_eq!(ticks(4942), (6000, 2000));
        assert_eq!(ticks(90), (100, 25));
        assert_eq!(ticks(7), (8, 2));
    }
}
