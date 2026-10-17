//! The egui window: the eframe `App`. The frameless window with its own
//! title row (`ui/title.rs`), About, the update check and the settings;
//! the library, the player and the playlist come in their own modules.

use std::time::{Duration, Instant};

use egui::{Key, Modifiers};

use crate::settings::{Settings, WindowPlace};
use crate::ui::about::AboutDialog;
use crate::ui::title::{self, TitleCmd};
use crate::update::Updates;
use crate::{frameless, palette, win};

/// How long the window stays cloaked at most while it is being maximized.
const UNCLOAK_TIMEOUT: Duration = Duration::from_secs(1);
/// How long the window must rest after a move or resize before its place
/// is saved: not at every frame of a drag, and not only at exit, which a
/// crash or a logoff may skip.
const PLACE_SAVE_DELAY: Duration = Duration::from_secs(1);

pub struct App {
    settings: Settings,
    about: AboutDialog,
    updates: Updates,
    /// The window's handle; `None` off Windows.
    hwnd: Option<isize>,
    /// The window is cloaked until its first maximized frame (the time it
    /// is shown anyway and the frame it was cloaked in).
    cloak: Option<(Instant, u64)>,
    /// When the window last moved or changed size, until its place is
    /// saved.
    place_changed: Option<Instant>,
    /// The app icon of the start screen and of the title row.
    start_icon: Option<egui::TextureHandle>,
    title_icon: Option<egui::TextureHandle>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings) -> Self {
        let ctx = &cc.egui_ctx;
        // Before any text is made.
        crate::i18n::set_lang(settings.language.resolve());
        palette::apply(ctx);
        ctx.options_mut(|o| {
            // Ctrl+plus and Ctrl+minus are the app's, not egui's zoom.
            o.zoom_with_keyboard = false;
            o.input_options.max_double_click_delay = win::double_click_time();
        });

        let hwnd = crate::caption::hwnd(cc);
        let mut cloak = None;
        if let Some(hwnd) = hwnd {
            // The window is still hidden (eframe shows it after the first
            // frame): the dark border and the place are set before it is
            // seen.
            crate::caption::set_dark(hwnd, true);
            win::round_corners(hwnd);
            let maximized = settings.window.is_some_and(|w| w.maximized);
            if maximized {
                win::cloak(hwnd, true);
                cloak = Some((Instant::now() + UNCLOAK_TIMEOUT, ctx.cumulative_frame_nr()));
            }
            if let Some(place) = settings.window {
                win::place(hwnd, Some(place.normal), crate::DEFAULT_SIZE, maximized);
            }
            let wake = ctx.clone();
            win::hook_window(hwnd, move || wake.request_repaint());
        }

        let mut updates = Updates::new(settings.check_updates, settings.last_update_check);
        updates.start_if_due(ctx);
        let mut about = AboutDialog::default();
        about.lang = settings.language;
        Self {
            settings,
            about,
            updates,
            hwnd,
            cloak,
            place_changed: None,
            start_icon: None,
            title_icon: None,
        }
    }

    /// Show the window once it is maximized (see `win::place`).
    fn uncloak(&mut self, ctx: &egui::Context) {
        let Some((until, frame)) = self.cloak else {
            return;
        };
        let ready = ctx.cumulative_frame_nr() > frame && frameless::is_maximized(ctx);
        if ready || Instant::now() >= until {
            if let Some(hwnd) = self.hwnd {
                win::cloak(hwnd, false);
            }
            self.cloak = None;
        } else {
            ctx.request_repaint();
        }
    }

    /// Keep the window's place in the settings, saved once it has rested.
    fn remember_place(&mut self) {
        let Some(hwnd) = self.hwnd else { return };
        // Not while it is being shown maximized: the placement is not final.
        if self.cloak.is_none()
            && let Some((normal, maximized)) = win::placement(hwnd)
        {
            let place = WindowPlace { normal, maximized };
            if self.settings.window != Some(place) {
                self.settings.window = Some(place);
                self.place_changed = Some(Instant::now());
            }
        }
        if self
            .place_changed
            .is_some_and(|at| at.elapsed() >= PLACE_SAVE_DELAY)
        {
            self.place_changed = None;
            self.sync_settings();
            self.settings.save_in_background();
        }
    }

    /// Copy what lives elsewhere (About, the update check) into the
    /// settings.
    fn sync_settings(&mut self) {
        self.settings.language = self.about.lang;
        self.settings.check_updates = self.updates.enabled;
        self.settings.last_update_check = self.updates.last_check;
    }

    fn save_now(&mut self) {
        self.remember_place();
        self.sync_settings();
        self.settings.save();
    }

    /// What the window's messages asked for: media keys, the end of the
    /// session, sleep.
    fn take_hooked(&mut self, ctx: &egui::Context) {
        for hooked in win::take_hooked() {
            match hooked {
                win::Hooked::EndSession | win::Hooked::Suspend => self.save_now(),
                win::Hooked::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                win::Hooked::Media { key, .. } => log::debug!("media key {key:?}"),
            }
        }
    }

    fn title_row(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let icon = crate::ui::app_icon(&ctx, 18.0, &mut self.title_icon);
        let update = self.updates.newer().map(str::to_owned);
        let maximized = frameless::is_maximized(&ctx);
        match title::row(ui, crate::APP_NAME, maximized, &icon, update.as_deref()) {
            Some(TitleCmd::Close) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Some(TitleCmd::Minimize) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true))
            }
            Some(TitleCmd::ToggleMaximize) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized))
            }
            Some(TitleCmd::About) => self.about.toggle(),
            None => {}
        }
    }
}

impl eframe::App for App {
    /// Every frame, also while the window is minimised.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.updates.poll();
        self.take_hooked(ctx);
        self.remember_place();
    }

    // egui 0.36: `ui` with the root `Ui`, not `update(ctx)`; panels are
    // `egui::Panel::top/left/right/bottom(id).show(ui, …)` and
    // `CentralPanel::default()` / `CentralPanel::no_frame()`.
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.uncloak(&ctx);
        // A modal takes the keys while it is open.
        if !self.about.open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F1)) {
            self.about.toggle();
        }
        if !frameless::is_maximized(&ctx) {
            frameless::resize_edges(root_ui);
        }
        egui::Panel::top("title")
            .exact_size(title::HEIGHT)
            .frame(egui::Frame::NONE)
            .show(root_ui, |ui| self.title_row(ui));
        egui::CentralPanel::default().show(root_ui, |ui| {
            let hint = tr!(
                "Add a music folder with Ctrl+Shift+O or drop it here",
                "Добавьте папку с музыкой: Ctrl+Shift+O или перетащите её сюда"
            );
            crate::ui::start::show(ui, &mut self.start_icon, hint);
        });
        self.about.show(&ctx, &mut self.updates);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_now();
    }
}
