//! The equalizer's window, opened by the player bar's button under it:
//! the same equalizer as the settings window's Sound page (the switch,
//! the presets, one's own presets, the sliders), on a card of its own.
//! Not modal: the player and its keys go on. The cross, Esc or the bar's
//! button close it; it can be dragged by its free space. When the
//! program's window cannot hold it (the player and playlist layout), it
//! is a window of its own, as the settings are (`super::OwnWindow`).

use egui::{Align, Layout, Pos2, Rect, RichText, pos2, vec2};

use crate::audio::dsp::DspSettings;
use crate::palette;
use crate::settings::EqPreset;

/// The width of the content in points (the settings page's card is about
/// as wide).
const WIDTH: f32 = 560.0;
/// The window's inner margin.
const MARGIN: i8 = 16;
/// Kept from the window's edges and from the button.
const GAP: f32 = 6.0;

#[derive(Default)]
pub struct EqWindow {
    pub open: bool,
    /// Opened this frame: placed under the button.
    fresh: bool,
    /// The button's bottom right corner.
    anchor: Pos2,
    /// The name a preset is being saved under, while one is.
    naming: Option<String>,
    /// In a window of its own (decided as it opens, and when the program's
    /// window gets too small), that window, and its height as last laid
    /// out (the content's).
    windowed: bool,
    window: super::OwnWindow,
    height: f32,
    /// The player's keys pressed in its own window, for the app to run
    /// (the program's window does not hear them).
    pub cmds: Vec<crate::keymap::Cmd>,
}

/// The window of its own's title, by which it is found.
fn window_title() -> &'static str {
    tr!("Equalizer", "Эквалайзер")
}

impl EqWindow {
    /// Closed, unless `METALAMP_EQ` is set: then open at start, under where
    /// the full bar's button is, for looking at it without a hand on the
    /// window (GUI checks).
    pub fn from_env() -> EqWindow {
        let mut window = EqWindow::default();
        if std::env::var_os("METALAMP_EQ").is_some() {
            window.toggle(Rect::from_min_max(pos2(470.0, 120.0), pos2(500.0, 148.0)));
        }
        window
    }

    /// The bar's button clicked: opens the window under `button`, or
    /// closes it.
    pub fn toggle(&mut self, button: Rect) {
        self.open = !self.open;
        self.fresh = self.open;
        self.anchor = button.right_bottom();
        self.naming = None;
    }

    /// Draws the window while it is open, editing `dsp` and `presets` in
    /// place; `keys`: no modal over it takes Esc.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        dsp: &mut DspSettings,
        presets: &mut Vec<EqPreset>,
        keys: bool,
        keymap: &crate::keymap::Keymap,
        owner: Option<isize>,
    ) {
        if !self.open {
            return;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let outer = WIDTH + 2.0 * f32::from(MARGIN);
        let screen = ctx.content_rect();
        // Out of the program's window when it cannot hold it: as it opens,
        // and when it gets smaller (another layout chosen); once out, it
        // stays out until closed.
        let small = screen.width() < outer + 2.0 * GAP;
        if fresh || (small && !self.windowed) {
            self.windowed = small;
            self.window.reset();
        }
        if self.windowed {
            if self.window(ctx, dsp, presets, keys.then_some(keymap), owner, fresh) {
                self.open = false;
            }
            return;
        }
        let mut window = egui::Window::new(tr!("Equalizer", "Эквалайзер"))
            .id(egui::Id::new("eq_window"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .frame(
                egui::Frame::window(&ctx.style_of(ctx.theme()))
                    .fill(palette::card_bg())
                    .stroke(egui::Stroke::new(1.0, palette::line()))
                    .inner_margin(egui::Margin::same(MARGIN))
                    .corner_radius(10.0),
            );
        if fresh {
            // Right-aligned with the button, under it, within the window.
            let x = (self.anchor.x - outer)
                .min(screen.right() - outer - GAP)
                .max(screen.left() + GAP);
            window = window.current_pos(pos2(x, self.anchor.y + GAP));
        }
        let mut close = false;
        window.show(ctx, |ui| {
            close |= self.content(ui, dsp, presets);
        });
        // Esc: a list open over it or the name being typed take it first.
        if !fresh
            && keys
            && self.naming.is_none()
            && !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            close = true;
        }
        if close {
            self.open = false;
        }
    }

    /// The title with the cross, and the equalizer; whether the cross was
    /// clicked.
    fn content(
        &mut self,
        ui: &mut egui::Ui,
        dsp: &mut DspSettings,
        presets: &mut Vec<EqPreset>,
    ) -> bool {
        let mut close = false;
        crate::ui::settings::round_widgets(ui);
        ui.set_width(WIDTH);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr!("Equalizer", "Эквалайзер"))
                    .size(17.0)
                    .strong()
                    .color(palette::text_bright()),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close |= super::close_cross(ui);
            });
        });
        ui.add_space(6.0);
        crate::ui::settings::equalizer(ui, dsp, presets, &mut self.naming);
        close
    }

    /// The equalizer in a window of its own (the settings' way): frameless,
    /// owned by the program's window, centred over it, as tall as its
    /// content; moved by its free space; the cross, Esc (not while a list
    /// or the name takes it) or closing it close it. Whether to close.
    fn window(
        &mut self,
        ctx: &egui::Context,
        dsp: &mut DspSettings,
        presets: &mut Vec<EqPreset>,
        keymap: Option<&crate::keymap::Keymap>,
        owner: Option<isize>,
        fresh: bool,
    ) -> bool {
        let margin = f32::from(MARGIN);
        let width = WIDTH + 2.0 * margin;
        if self.height <= 0.0 {
            self.height = 380.0;
        }
        let size = vec2(width, self.height);
        let centre = ctx
            .input(|i| i.viewport().outer_rect)
            .unwrap_or(ctx.content_rect())
            .center();
        let builder = super::OwnWindow::builder(egui::ViewportBuilder::default())
            .with_title(window_title())
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(size)
            .with_position(centre - size / 2.0)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            .with_icon(crate::window_icon());
        let id = egui::ViewportId::from_hash_of("eq_window");
        let dark = palette::current().dark;
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            self.window.follow(&ctx, window_title(), owner, dark);
            close |= ctx.input(|i| i.viewport().close_requested());
            if !fresh
                && self.naming.is_none()
                && !egui::Popup::is_any_open(&ctx)
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                close = true;
            }
            // The player's keys (not those a field being typed in keeps):
            // not modal, the player goes on from here too.
            if let Some(keymap) = keymap {
                let typing = ctx.memory(|m| m.focused().is_some());
                let cmds = crate::keymap::take(&ctx, keymap, |c| !(typing && c.edits_text()));
                self.cmds.extend(cmds);
            }
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(palette::card_bg())
                        .inner_margin(egui::Margin::same(MARGIN)),
                )
                .show(ui, |ui| {
                    // Its free space moves it; the widgets come later and
                    // take their clicks first.
                    crate::frameless::drag_area(
                        ui,
                        ctx.content_rect(),
                        ui.id().with("eq_drag"),
                        egui::Sense::hover(),
                    );
                    let content = ui.scope(|ui| self.content(ui, dsp, presets));
                    close |= content.inner;
                    // As tall as the content (the name row comes and goes).
                    let height = content.response.rect.height() + 2.0 * margin;
                    if (height - self.height).abs() > 0.5 {
                        self.height = height;
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(
                            width, height,
                        )));
                    }
                });
        });
        close
    }
}
