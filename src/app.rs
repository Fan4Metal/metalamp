//! The egui window: the eframe `App` with the pieces every app of the
//! family has (theme switch, About, update check, dark caption, settings).
//! The app's own state and panels go in here too, or in modules of their
//! own with `impl App` blocks (`ui/toolbar.rs`, `ui/status.rs`, …).

use egui::{Key, Modifiers, ThemePreference};

use crate::settings::Settings;
use crate::ui::about::AboutDialog;
use crate::update::Updates;

pub struct App {
    theme: ThemePreference,
    about: AboutDialog,
    updates: Updates,
    /// The window's handle, for the caption; `None` off Windows.
    hwnd: Option<isize>,
    /// The theme the caption was last painted in.
    shown_theme: Option<egui::Theme>,
    /// The caption must be repainted in the next frame (the theme changed).
    refresh_caption: bool,
    /// The app icon of the start screen, as a texture.
    start_icon: Option<egui::TextureHandle>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = cc.storage.map(Settings::load).unwrap_or_default();
        // Before any text is made.
        crate::i18n::set_lang(settings.language.resolve());
        cc.egui_ctx.set_theme(settings.theme);
        // The window is still hidden (eframe shows it after the first
        // frame), so a caption set now is shown from the start; set after
        // egui's first frame, Windows 10 would keep the caption of the
        // Windows theme. Following Windows, winit has already done it.
        let hwnd = crate::caption::hwnd(cc);
        if let Some(hwnd) = hwnd {
            match settings.theme {
                ThemePreference::Dark => crate::caption::set_dark(hwnd, true),
                ThemePreference::Light => crate::caption::set_dark(hwnd, false),
                ThemePreference::System => {}
            }
        }
        let mut updates = Updates::new(settings.check_updates, settings.last_update_check);
        updates.start_if_due(&cc.egui_ctx);
        let mut about = AboutDialog::default();
        about.lang = settings.language;
        Self {
            theme: settings.theme,
            about,
            updates,
            hwnd,
            shown_theme: None,
            refresh_caption: false,
            start_icon: None,
        }
    }

    /// Bring the window caption in line with the theme. egui passes a new
    /// theme to the window at the end of the frame it changed in, and
    /// Windows applies it to the caption only on the next repaint of the
    /// frame, so the frame after the change asks for one.
    fn follow_theme(&mut self, ctx: &egui::Context) {
        let theme = ctx.theme();
        if std::mem::take(&mut self.refresh_caption)
            && let Some(hwnd) = self.hwnd
        {
            crate::caption::set_dark(hwnd, theme == egui::Theme::Dark);
            crate::caption::refresh(hwnd);
        }
        if self.shown_theme.is_some_and(|t| t != theme) {
            self.refresh_caption = true;
            ctx.request_repaint();
        }
        self.shown_theme = Some(theme);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // The app's own controls go first; these three sit at the
            // right end of the toolbar.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(tag) = self.updates.newer().map(str::to_owned)
                    && crate::ui::update_button(ui, &tag).clicked()
                {
                    crate::win::shell_open(crate::update::release_url(&tag));
                }
                if crate::ui::about_button(ui).clicked() {
                    self.about.toggle();
                }
                if crate::ui::theme_button(ui, self.theme).clicked() {
                    self.theme = crate::ui::next_theme(self.theme);
                    ui.ctx().set_theme(self.theme);
                }
            });
        });
    }
}

impl eframe::App for App {
    // egui 0.36: `ui` with the root `Ui`, not `update(ctx)`; panels are
    // `egui::Panel::top/left/right/bottom(id).show(ui, …)` and
    // `CentralPanel::default()` / `CentralPanel::no_frame()`.
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.updates.poll();
        self.follow_theme(&ctx);
        // A modal takes the keys while it is open.
        if !self.about.open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F1)) {
            self.about.toggle();
        }
        egui::Panel::top("toolbar").show(root_ui, |ui| self.toolbar(ui));
        egui::CentralPanel::default().show(root_ui, |ui| {
            // While nothing is open; the app's own view takes its place.
            let hint = tr!(
                "Open a file with Ctrl+O or drop it here",
                "Откройте файл: Ctrl+O или перетащите его сюда"
            );
            crate::ui::start::show(ui, &mut self.start_icon, hint);
        });
        self.about.show(&ctx, &mut self.updates);
    }

    /// Called by eframe every `auto_save_interval` and on exit.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        Settings {
            theme: self.theme,
            language: self.about.lang,
            check_updates: self.updates.enabled,
            last_update_check: self.updates.last_check,
        }
        .save(storage);
    }
}
