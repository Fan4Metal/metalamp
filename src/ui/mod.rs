//! User interface pieces: the app icon as a texture, the toolbar button
//! with a painted icon (`icons`), the About and update buttons, the close
//! cross of a dialog; the start screen (`start`), the title row of the
//! frameless window (`title`). The theme is always dark (`palette`), so the
//! family's theme switch is left out.

pub mod about;
pub mod icons;
pub mod library;
pub mod start;
pub mod status;
pub mod table;
pub mod title;

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

/// Opens the About window: an "i" in a circle.
pub fn about_button(ui: &mut Ui) -> Response {
    let tip = tr!(
        format!("About {} (F1)", crate::APP_NAME),
        format!("О программе {} (F1)", crate::APP_NAME)
    );
    icon_button(ui, Icon::Info, false, &tip)
}

/// A newer release: its version with a download arrow in the link colour;
/// a click opens its release page. Shown right of About (or at the
/// toolbar's right end) only while `Updates::newer` is `Some`.
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
pub fn double_clicked(response: &Response) -> bool {
    response.double_clicked() || response.triple_clicked()
}
