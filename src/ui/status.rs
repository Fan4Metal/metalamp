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
        egui::Stroke::new(1.0, palette::LINE),
    );
    ui.horizontal_centered(|ui| {
        ui.add_space(10.0);
        let totals = app.library.totals;
        if app.loaded {
            let text = format!(
                "{} [{}] [{}]",
                count(
                    totals.tracks,
                    ["track", "tracks"],
                    ["трек", "трека", "треков"]
                ),
                format::long_duration(totals.length_ms),
                format::size(totals.bytes),
            );
            ui.label(RichText::new(text).color(palette::TEXT_WEAK).size(12.0));
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
                ui.add(egui::Spinner::new().size(12.0).color(palette::ACCENT));
                ui.label(RichText::new(text).color(palette::TEXT_WEAK).size(12.0));
            }
        });
    });
}
