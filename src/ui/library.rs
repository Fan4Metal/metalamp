//! The media library view. For now a summary of what the scan found; the
//! artists, the albums with their covers and the tracks come next.

use egui::{RichText, Ui};

use crate::app::App;
use crate::i18n::count;
use crate::palette;

pub fn show(ui: &mut Ui, app: &mut App) {
    let library = &app.library;
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 2.0 - 30.0);
        if library.tracks.is_empty() {
            let text = if app.scanning() {
                tr!("Reading the library…", "Чтение библиотеки…")
            } else {
                tr!(
                    "No music found in the library's folders",
                    "В папках библиотеки нет музыки"
                )
            };
            ui.label(RichText::new(text).color(palette::TEXT_WEAK));
            return;
        }
        ui.label(
            RichText::new(format!(
                "{}, {}",
                count(
                    library.artists.len() as u64,
                    ["artist", "artists"],
                    ["исполнитель", "исполнителя", "исполнителей"]
                ),
                count(
                    library.albums.len() as u64,
                    ["album", "albums"],
                    ["альбом", "альбома", "альбомов"]
                ),
            ))
            .size(16.0),
        );
    });
}
