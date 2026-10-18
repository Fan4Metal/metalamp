//! The About dialog: icon, name, version, a short description, the author,
//! the homepage, the license, where the settings are kept. The language
//! and the update check are in the settings window (as metalmpv's).

use egui::{Key, TextureHandle};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");
/// Cargo joins several authors with `:`.
const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
/// Displayed icon size in points.
const ICON: f32 = 64.0;

#[derive(Default)]
pub struct AboutDialog {
    pub open: bool,
    /// Opened this frame: the key that opened it must not close it.
    pub fresh: bool,
    icon: Option<TextureHandle>,
}

impl AboutDialog {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.fresh = self.open;
    }

    /// Draws the dialog while it is open; closes it on the cross, Esc,
    /// Enter or a click outside.
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let icon = super::app_icon(ctx, ICON, &mut self.icon);
        let modal = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
            ui.set_width(340.0);
            // The cross in the top right corner, over the centred content
            // (a child Ui takes no room in the layout).
            let corner = egui::Rect::from_min_size(
                egui::pos2(ui.max_rect().right() - 24.0, ui.cursor().top()),
                egui::vec2(24.0, 24.0),
            );
            let closed =
                super::close_cross(&mut ui.new_child(egui::UiBuilder::new().max_rect(corner)));
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.image((icon.id(), egui::vec2(ICON, ICON)));
                ui.add_space(6.0);
                ui.heading(crate::APP_NAME);
                ui.label(tr!(
                    format!("Version {}", crate::VERSION),
                    format!("Версия {}", crate::VERSION)
                ));
                ui.add_space(8.0);
                // One sentence, the same as Cargo.toml's description and
                // the README tagline; the Russian one is written by hand.
                ui.label(tr!(
                    "A fast audio player with a media library for Windows.",
                    "Быстрый аудиоплеер с медиабиблиотекой для Windows."
                ));
                ui.add_space(8.0);
                if !AUTHORS.is_empty() {
                    let authors = AUTHORS.replace(':', ", ");
                    ui.label(tr!(
                        format!("Author: {authors}"),
                        format!("Автор: {authors}")
                    ));
                }
                if !REPOSITORY.is_empty()
                    && ui
                        .link(tr!("Homepage", "Сайт проекта"))
                        .on_hover_text(REPOSITORY)
                        .clicked()
                    && !crate::win::shell_open(REPOSITORY)
                {
                    log::warn!("could not open {REPOSITORY}");
                }
                if !LICENSE.is_empty() {
                    ui.weak(tr!(
                        format!("{LICENSE} License"),
                        format!("Лицензия {LICENSE}")
                    ));
                }
                let stored = crate::settings::location();
                let place = ui.weak(if stored.portable {
                    tr!(
                        "Settings: next to the program (portable)",
                        "Настройки: рядом с программой (портативно)"
                    )
                } else {
                    tr!(
                        "Settings: in the user profile",
                        "Настройки: в профиле пользователя"
                    )
                });
                match &stored.file {
                    Some(path) => place.on_hover_text(path.display().to_string()),
                    None => place
                        .on_hover_text(tr!("Settings are not saved", "Настройки не сохраняются")),
                };
                ui.add_space(8.0);
                ui.weak(tr!("Settings: Ctrl+P", "Настройки: Ctrl+P"));
                ui.add_space(4.0);
            });
            closed
        });
        let closed = modal.inner
            || (!fresh
                && (ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
                    || modal.should_close()));
        if closed {
            self.open = false;
        }
    }
}
