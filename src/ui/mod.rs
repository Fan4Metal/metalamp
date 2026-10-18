//! User interface pieces: the app icon as a texture, the toolbar button
//! with a painted icon (`icons`), the About and update buttons, the close
//! cross of a dialog; the start screen (`start`), the title row of the
//! frameless window (`title`). The theme is always dark (`palette`), so the
//! family's theme switch is left out.

pub mod about;
pub mod icons;
pub mod info;
pub mod library;
pub mod licenses;
pub mod player_bar;
pub mod playlist;
pub mod settings;
pub mod spectrum;
pub mod start;
pub mod status;
pub mod table;
pub mod title;
pub mod visual;

use egui::{Response, TextureHandle, TextureOptions, Ui};
use icons::Icon;

/// The app icon as a texture for `points` wide, from the smallest embedded
/// size (64, 128, 256 px) that covers it at the display's pixel density;
/// kept in `cache` and replaced when that density changes.
pub fn app_icon(
    ctx: &egui::Context,
    points: f32,
    cache: &mut Option<TextureHandle>,
) -> TextureHandle {
    let px = (points * ctx.pixels_per_point()).round() as u32;
    let size = [64u32, 128, 256]
        .into_iter()
        .find(|&s| s >= px)
        .unwrap_or(256);
    let n = size as usize;
    if cache.as_ref().is_none_or(|t| t.size() != [n, n]) {
        let image = egui::ColorImage::from_rgba_unmultiplied([n, n], crate::embedded_icon(size));
        let options = TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
        *cache = Some(ctx.load_texture(format!("app_icon_{size}"), image, options));
    }
    cache.clone().expect("set above")
}

/// A toolbar button with `icon` (`icons::SIZE` points) in the button's
/// text colour; `selected` gives it the look of a toggle that is on.
pub fn icon_button(ui: &mut Ui, icon: Icon, selected: bool, tip: &str) -> Response {
    let id = ui.id().with(("icon", icon));
    let resp = egui::Button::new(egui::Atom::custom(id, egui::Vec2::splat(icons::SIZE)))
        .selected(selected)
        .atom_ui(ui);
    let response = resp.response.clone().on_hover_text(tip);
    if let Some(rect) = resp.rect(id) {
        let visuals = ui.style().interact_selectable(&response, selected);
        icons::paint(
            ui.painter(),
            icon,
            rect.center(),
            icons::SIZE,
            visuals.text_color(),
            visuals.weak_bg_fill,
        );
    }
    response
}

/// A newer release: its version with a download arrow in the link colour;
/// a click opens its release page. Shown in the title row, before the
/// window's buttons, only while `Updates::newer` is `Some`.
pub fn update_button(ui: &mut Ui, tag: &str) -> Response {
    let version = crate::update::version_of(tag);
    let id = ui.id().with("update_icon");
    let resp = egui::Button::new((
        egui::Atom::custom(id, egui::Vec2::splat(icons::SIZE)),
        version,
    ))
    .atom_ui(ui);
    let response = resp.response.clone().on_hover_text(tr!(
        format!("Version {version} is available: open its release page"),
        format!("Доступна версия {version}: открыть страницу выпуска")
    ));
    if let Some(rect) = resp.rect(id) {
        let (link, fill) = (
            ui.visuals().hyperlink_color,
            ui.style().interact(&response).weak_bg_fill,
        );
        icons::paint(
            ui.painter(),
            Icon::Download,
            rect.center(),
            icons::SIZE,
            link,
            fill,
        );
    }
    response
}

/// The size of a modal of `wanted` points, cut to the window less
/// `margin` on each side.
pub fn modal_size(ctx: &egui::Context, wanted: egui::Vec2, margin: f32) -> egui::Vec2 {
    let room = ctx.content_rect().size() - egui::vec2(2.0 * margin, 2.0 * margin);
    wanted.min(room).max(egui::vec2(200.0, 160.0))
}

/// A cross that closes a dialog; true when clicked.
pub fn close_cross(ui: &mut Ui) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, visuals.bg_fill);
    }
    icons::paint(
        ui.painter(),
        Icon::Close,
        rect.center(),
        icons::SIZE,
        visuals.fg_stroke.color,
        visuals.bg_fill,
    );
    response.on_hover_text(tr!("Close", "Закрыть")).clicked()
}

/// A double click, also when it is the start of a triple one (egui reports
/// the third click as a triple click instead of a second double click).
/// `text` on one line at most `width` wide: a longer one cut with "…"
/// (`Galley::elided`), as Windows' lists cut it; the caller shows the
/// whole text as a tooltip then.
pub fn elided(
    painter: &egui::Painter,
    text: String,
    font: egui::FontId,
    color: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    painter.layout_job(job)
}

pub fn double_clicked(response: &Response) -> bool {
    response.double_clicked() || response.triple_clicked()
}

/// How many whole wheel notches turned this frame, up or down (metalmpv's).
/// A touchpad or a free-spinning wheel sends many small deltas: they add
/// up, and the rest waits for the next frames (forgotten after a second of
/// rest or when the direction changes), so one swipe is not dozens of
/// notches.
pub fn wheel_notches(ui: &Ui) -> i32 {
    /// Points of a pixel-precise delta per notch.
    const POINTS_PER_NOTCH: f32 = 50.0;
    let (delta, time) = ui.input(|i| {
        let delta: f32 = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                    egui::MouseWheelUnit::Point => delta.y / POINTS_PER_NOTCH,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                }),
                _ => None,
            })
            .sum();
        (delta, i.time)
    });
    if delta == 0.0 || !delta.is_finite() {
        return 0;
    }
    let id = egui::Id::new("wheel_notches");
    ui.ctx()
        .data_mut(|d| notches(d.get_temp_mut_or_default::<(f32, f64)>(id), delta, time))
}

/// `delta` notches (a fraction from a touchpad) at `time` added to `kept`
/// (the rest and when it came); the whole notches taken out.
fn notches(kept: &mut (f32, f64), delta: f32, time: f64) -> i32 {
    let (rest, last) = kept;
    if time - *last > 1.0 || (*rest != 0.0 && rest.signum() != delta.signum()) {
        *rest = 0.0;
    }
    *last = time;
    *rest += delta;
    let whole = rest.trunc();
    *rest -= whole;
    whole as i32
}

/// The key that let a text field go (Enter, Esc) taken: egui leaves it in
/// the frame's input, where the window's own use of it would act too (a
/// modal closing, the pane last clicked playing its selection). True when
/// `field` was let go by Enter.
pub fn take_leaving_key(ui: &Ui, field: &egui::Response) -> bool {
    if !field.lost_focus() {
        return false;
    }
    ui.input_mut(|i| {
        i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
    })
}

#[cfg(test)]
mod tests {
    use super::notches;

    #[test]
    fn small_wheel_deltas_add_up() {
        let mut kept = (0.0, 0.0);
        // A touchpad's swipe: a tenth of a notch at a time.
        let turned: i32 = (0..25)
            .map(|i| notches(&mut kept, 0.1, 1.0 + i as f64 * 0.01))
            .sum();
        assert_eq!(turned, 2);
        // A whole notch of a wheel is one.
        assert_eq!(notches(&mut (0.0, 0.0), 1.0, 0.0), 1);
        assert_eq!(notches(&mut (0.0, 0.0), -3.0, 0.0), -3);
    }

    #[test]
    fn the_rest_is_forgotten_after_a_pause_or_a_turn_back() {
        let mut kept = (0.0, 0.0);
        assert_eq!(notches(&mut kept, 0.8, 1.0), 0);
        assert_eq!(notches(&mut kept, 0.8, 2.5), 0);
        assert_eq!(notches(&mut kept, -0.5, 2.6), 0);
        assert_eq!(notches(&mut kept, -0.6, 2.7), -1);
    }
}
