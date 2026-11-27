//! The own scheme's panel, opened by its swatch in the visualization's
//! menu: the scheme's four colours, how the visualization shown lays them
//! out and the tools (`settings::scheme_editor`, shared with the settings'
//! General page), under the display, so the visualization itself shows
//! each change. Not modal: the player and its keys go on. The cross or
//! Esc close it; it can be dragged by its free space. When the program's
//! window cannot hold it under the display (the player alone, a low
//! window), it is a window of its own, as the sound tuning window is
//! (`super::OwnWindow`).

use egui::{Align, Layout, Rect, RichText, pos2, vec2};

use crate::palette::{self, Look, VisualScheme};
use crate::settings::Visual;
use crate::ui::settings::SCHEME_WIDTH;

/// The window's inner margin.
const MARGIN: i8 = 16;
/// Kept from the window's edges and from the display.
const GAP: f32 = 6.0;
/// The window's height until it has been laid out once.
const HEIGHT: f32 = 230.0;

#[derive(Default)]
pub struct SchemeWindow {
    pub open: bool,
    /// Opened this frame: placed under the display.
    fresh: bool,
    /// The display it opened under (none until it opens).
    anchor: Option<Rect>,
    /// In a window of its own (decided as it opens, and when the program's
    /// window gets too small), that window, and its height as last laid
    /// out.
    windowed: bool,
    window: super::OwnWindow,
    height: f32,
    /// The player's keys pressed in its own window, for the app to run.
    pub cmds: Vec<crate::keymap::Cmd>,
}

/// The title, by which the window of its own is found too.
fn window_title() -> &'static str {
    tr!("Your own scheme", "Своя схема")
}

impl SchemeWindow {
    /// Closed, unless `METALAMP_SCHEME_PANEL` is set: then open at start,
    /// under where the full bar's display is, for looking at it without a
    /// hand on the window (GUI checks).
    pub fn from_env() -> SchemeWindow {
        let mut window = SchemeWindow::default();
        if std::env::var_os("METALAMP_SCHEME_PANEL").is_some() {
            window.open_under(Rect::from_min_max(pos2(560.0, 40.0), pos2(1270.0, 150.0)));
        }
        window
    }

    /// Opens under `display` (again there when open already).
    pub fn open_under(&mut self, display: Rect) {
        self.open = true;
        self.fresh = true;
        self.anchor = Some(display);
    }

    /// Draws the panel while it is open, editing `scheme` in place, laid
    /// out as `visual` shows it; `keys`: no modal over it takes Esc.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        scheme: &mut VisualScheme,
        look: Look,
        visual: Visual,
        keys: bool,
        keymap: &crate::keymap::Keymap,
        owner: Option<isize>,
    ) {
        if !self.open {
            return;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let outer = SCHEME_WIDTH + 2.0 * f32::from(MARGIN);
        let screen = ctx.content_rect();
        let anchor = self.anchor.unwrap_or(screen);
        if self.height <= 0.0 {
            self.height = HEIGHT;
        }
        let small = screen.width() < outer + 2.0 * GAP
            || screen.bottom() < anchor.bottom() + self.height + 2.0 * GAP;
        if fresh || (small && !self.windowed) {
            self.windowed = small;
            self.window.reset();
        }
        if self.windowed {
            if self.window(
                ctx,
                scheme,
                (look, visual),
                keys.then_some(keymap),
                owner,
                fresh,
            ) {
                self.open = false;
            }
            return;
        }
        let mut window = egui::Window::new(window_title())
            .id(egui::Id::new("scheme_window"))
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
            // Left-aligned with the display, under it, within the window.
            let x = anchor
                .left()
                .min(screen.right() - outer - GAP)
                .max(screen.left() + GAP);
            window = window.current_pos(pos2(x, anchor.bottom() + GAP));
        }
        let mut close = false;
        let shown = window.show(ctx, |ui| {
            close |= content(ui, scheme, look, visual);
        });
        if let Some(shown) = shown {
            self.height = shown.response.rect.height();
        }
        // Esc: a picker or a menu open over it takes it first.
        if !fresh
            && keys
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|m| m.focused().is_none())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            close = true;
        }
        if close {
            self.open = false;
        }
    }

    /// The panel in a window of its own (the sound tuning window's way):
    /// frameless, owned by the program's window, centred over it, as tall
    /// as its content; moved by its free space; the cross, Esc or closing
    /// it close it. Whether to close.
    fn window(
        &mut self,
        ctx: &egui::Context,
        scheme: &mut VisualScheme,
        (look, visual): (Look, Visual),
        keymap: Option<&crate::keymap::Keymap>,
        owner: Option<isize>,
        fresh: bool,
    ) -> bool {
        let margin = f32::from(MARGIN);
        let width = SCHEME_WIDTH + 2.0 * margin;
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
        let id = egui::ViewportId::from_hash_of("scheme_window");
        let dark = palette::current().dark;
        let mut close = false;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            self.window.follow(&ctx, window_title(), owner, dark);
            close |= ctx.input(|i| i.viewport().close_requested());
            if !fresh
                && !egui::Popup::is_any_open(&ctx)
                && ctx.memory(|m| m.focused().is_none())
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                close = true;
            }
            // The player's keys (not those a field being typed in keeps).
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
                    crate::frameless::drag_area(
                        ui,
                        ctx.content_rect(),
                        ui.id().with("scheme_drag"),
                        egui::Sense::hover(),
                    );
                    let shown = ui.scope(|ui| content(ui, scheme, look, visual));
                    close |= shown.inner;
                    let height = shown.response.rect.height() + 2.0 * margin;
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

/// The title with the cross, and the editor; whether the cross was
/// clicked.
fn content(ui: &mut egui::Ui, scheme: &mut VisualScheme, look: Look, visual: Visual) -> bool {
    let mut close = false;
    crate::ui::settings::round_widgets(ui);
    ui.set_width(SCHEME_WIDTH);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(window_title())
                .size(17.0)
                .strong()
                .color(palette::text_bright()),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close |= super::close_cross(ui);
        });
    });
    ui.add_space(8.0);
    crate::ui::settings::scheme_editor(ui, scheme, look, visual);
    close
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel's content drawn by hand into a PNG to look at, for the
    /// visualizations of each layout: `METALAMP_FRAMES=<file.png> cargo
    /// test scheme_panel_frames -- --ignored` (`METALAMP_THEME=light`).
    #[test]
    #[ignore]
    fn scheme_panel_frames() {
        let out = std::env::var("METALAMP_FRAMES").expect("METALAMP_FRAMES=<file.png>");
        let mut look = Look {
            visual: palette::VisualColours::Scheme,
            ..Look::default()
        };
        if std::env::var("METALAMP_THEME").is_ok_and(|t| t == "light") {
            look.theme = palette::Theme::Light;
        }
        palette::set(look);
        let margin = f32::from(MARGIN);
        let size = vec2(SCHEME_WIDTH + 2.0 * margin, 250.0);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), size);
        let mut renderer = crate::ui::raster::Renderer::new(2.0);
        let mut frames = Vec::new();
        let dark = VisualScheme {
            low: 0x101020,
            ..VisualScheme::default()
        };
        let cases = [
            (VisualScheme::default(), Visual::Spectrum),
            (VisualScheme::default(), Visual::Synthwave),
            (VisualScheme::default(), Visual::Chroma),
            (dark, Visual::Waveform),
        ];
        for (scheme, visual) in cases {
            let mut scheme = scheme;
            // Twice: the first frame lays the fonts out.
            for _ in 0..2 {
                let picture = renderer.frame_ui(rect, palette::card_bg(), |ui| {
                    let inner = rect.shrink(margin);
                    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                    content(&mut ui, &mut scheme, look, visual);
                });
                frames.push(picture);
            }
            frames.remove(frames.len() - 2);
        }
        crate::ui::raster::save(&out, &frames, renderer.pixels(size));
    }
}