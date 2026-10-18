//! The status line at the window's bottom, as Winamp's media library has
//! it: how many tracks, how long and how large, and the scan's progress.

use egui::{Align, Layout, RichText, Ui};

use crate::app::App;
use crate::i18n::count;
use crate::{format, palette};

pub const HEIGHT: f32 = 24.0;

pub fn show(ui: &mut Ui, app: &App) {
    let rect = ui.max_rect();
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(1.0, palette::line()),
    );
    ui.horizontal_centered(|ui| {
        ui.add_space(10.0);
        // What the library view shows, as Winamp's status line does, with
        // the time its query took; the whole library until a view exists.
        let (tracks, length, bytes, took) = match &app.library_view.view {
            Some(v) => (v.tracks.len() as u64, v.length_ms, v.bytes, Some(v.took)),
            None => {
                let t = app.library.totals;
                (t.tracks, t.length_ms, t.bytes, None)
            }
        };
        if app.loaded {
            let mut text = format!(
                "{} [{}] [{}]",
                count(tracks, ["track", "tracks"], ["трек", "трека", "треков"]),
                format::long_duration(length),
                format::size(bytes),
            );
            if let Some(took) = took {
                let ms = format::millis_in(crate::i18n::lang(), took.as_secs_f64());
                text += &tr!(format!(" in {ms}"), format!(" за {ms}"));
            }
            ui.label(RichText::new(text).color(palette::text_weak()).size(12.0));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(10.0);
            let text = match app.scan_progress {
                Some((done, total)) => Some(tr!(
                    format!("Reading tags: {done} of {total}"),
                    format!("Чтение тегов: {done} из {total}")
                )),
                None if app.scanning() => {
                    Some(tr!("Checking the library…", "Проверка библиотеки…").to_owned())
                }
                None => None,
            };
            if let Some(text) = text {
                ui.add(egui::Spinner::new().size(12.0).color(palette::accent()));
                ui.label(RichText::new(text).color(palette::text_weak()).size(12.0));
            }
        });
    });
}
