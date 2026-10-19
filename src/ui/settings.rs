//! The settings window (F5 as metalmpv's, Ctrl+P, or the gear in the library's bar), after metalmpv's: one surface without inner
//! frames, the categories in a darker column on the left with Reset and
//! Close under them, the page of the chosen category on the right in
//! cards of rows (the name and a hint on the left, the control on the
//! right). The language and the update check live here, not in About.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Rect, Response, RichText, Sense,
    Stroke, Ui, pos2, vec2,
};

use crate::palette::ColourText;

use crate::audio::dsp::{DspSettings, eq};
use crate::audio::output::Device;
use crate::i18n::LangChoice;
use crate::palette;
use crate::palette::{Accent, Palette, Theme, Tint};
use crate::settings::{BarLayout, EqPreset, MediaKeys, Settings};
use crate::ui::icons::{self, Icon};
use crate::update::{Status, Updates, release_url, version_of};

/// The size of the window's content in points.
const SIZE: egui::Vec2 = vec2(820.0, 560.0);
/// The width of the category column.
const SIDEBAR_WIDTH: f32 = 180.0;
/// The corner radius of the window.
const RADIUS: u8 = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    General,
    Sound,
    Updates,
}

impl Page {
    const ALL: [Page; 3] = [Page::General, Page::Sound, Page::Updates];

    fn title(self) -> &'static str {
        match self {
            Page::General => tr!("General", "Общие"),
            Page::Sound => tr!("Sound", "Звук"),
            Page::Updates => tr!("Updates", "Обновления"),
        }
    }

    /// The line under the page's title.
    fn summary(self) -> &'static str {
        match self {
            Page::General => tr!(
                "The language, the theme and colour, the layout of the player bar and the media keys.",
                "Язык, тема и цвет, вид панели плеера и медиаклавиши."
            ),
            Page::Sound => tr!(
                "The device the sound goes to; one loudness for every track, the equalizer and the enhancer.",
                "Устройство, на которое выводится звук; одна громкость у всех треков, эквалайзер и улучшение звука."
            ),
            Page::Updates => tr!(
                "Asking GitHub whether a newer version is out.",
                "Проверка выхода новой версии на GitHub."
            ),
        }
    }
}

/// What changed in the window this frame and has to be applied.
#[derive(Default)]
pub struct Changed {
    /// The output device (`settings.output_device`).
    pub device: bool,
    /// The sound processing (`settings.dsp`).
    pub dsp: bool,
    /// The theme or the accent: `palette::set` and `apply`.
    pub look: bool,
    /// Anything to save.
    pub any: bool,
}

#[derive(Default)]
pub struct SettingsDialog {
    pub open: bool,
    /// Opened this frame: the key that opened it must not close it.
    fresh: bool,
    /// The category shown; kept while the program runs.
    page: Page,
    /// The output devices, listed when the window opens.
    devices: Vec<Device>,
    /// The name an equalizer preset is being saved under, while one is.
    naming: Option<String>,
    /// A scroll offset to put the page at once (`from_env`).
    scroll_to: Option<f32>,
}

impl SettingsDialog {
    /// Closed, unless `METALAMP_SETTINGS` names a page (`general`, `sound`,
    /// `updates`), optionally with a scroll offset after a colon
    /// (`sound:800`): then open on it at start, for looking at a page
    /// without a hand on the window (GUI checks).
    pub fn from_env() -> SettingsDialog {
        let mut dialog = SettingsDialog::default();
        if let Ok(spec) = std::env::var("METALAMP_SETTINGS") {
            let (page, offset) = spec.split_once(':').unwrap_or((&spec, ""));
            dialog.page = match page {
                "sound" => Page::Sound,
                "updates" => Page::Updates,
                _ => Page::General,
            };
            dialog.scroll_to = offset.parse().ok();
            dialog.toggle();
        }
        dialog
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.fresh = self.open;
        if self.open {
            self.devices = crate::audio::output::list_devices();
        }
    }

    /// Draws the window while it is open, editing `settings` and `updates`
    /// in place (the language is applied at once).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        updates: &mut Updates,
    ) -> Changed {
        let mut changed = Changed::default();
        if !self.open {
            return changed;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let before = settings.clone();
        // Kept by the update check, saved with the settings.
        let checking = updates.enabled;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("settings"))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                    .fill(palette::panel_bg())
                    .corner_radius(RADIUS)
                    .inner_margin(Margin::ZERO),
            )
            .show(ctx, |ui| {
                round_widgets(ui);
                // Smaller in a small window, so Close and the cross stay
                // on the screen.
                let size = super::modal_size(ctx, SIZE, 12.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());

                // The category column, flush with the window's left edge.
                let sidebar = Rect::from_min_size(rect.min, vec2(SIDEBAR_WIDTH, rect.height()));
                let inner = RADIUS - 1;
                ui.painter().rect_filled(
                    sidebar,
                    CornerRadius {
                        nw: inner,
                        sw: inner,
                        ne: 0,
                        se: 0,
                    },
                    palette::sidebar_bg(),
                );
                ui.painter().text(
                    sidebar.left_top() + vec2(20.0, 20.0),
                    Align2::LEFT_TOP,
                    tr!("Settings", "Настройки"),
                    FontId::proportional(18.0),
                    palette::text_bright(),
                );
                let list = Rect::from_min_max(
                    sidebar.left_top() + vec2(10.0, 62.0),
                    sidebar.right_bottom() - vec2(10.0, 100.0),
                );
                self.categories(ui, list);

                const BUTTON: f32 = 30.0;
                let close_rect = Rect::from_min_size(
                    pos2(sidebar.left() + 12.0, sidebar.bottom() - 14.0 - BUTTON),
                    vec2(SIDEBAR_WIDTH - 24.0, BUTTON),
                );
                let reset_rect = close_rect.translate(vec2(0.0, -BUTTON - 8.0));
                if child(ui, reset_rect)
                    .add_sized(
                        reset_rect.size(),
                        egui::Button::new(tr!("Reset", "Сбросить")),
                    )
                    .on_hover_text(tr!(
                        "Put the settings of this window back to their defaults",
                        "Вернуть настройкам этого окна значения по умолчанию"
                    ))
                    .clicked()
                {
                    reset(settings, updates);
                    self.naming = None;
                }
                close |= child(ui, close_rect)
                    .add_sized(
                        close_rect.size(),
                        egui::Button::new(tr!("Close", "Закрыть")),
                    )
                    .clicked();

                // The page.
                let page = Rect::from_min_max(pos2(sidebar.right(), rect.top()), rect.max);
                let cross = Rect::from_min_size(
                    pos2(page.right() - 14.0 - 24.0, page.top() + 14.0),
                    vec2(24.0, 24.0),
                );
                close |= super::close_cross(&mut child(ui, cross));
                // The scroll bar takes the right margin, next to the cards.
                let content = Rect::from_min_max(
                    page.left_top() + vec2(28.0, 18.0),
                    page.right_bottom() - vec2(10.0, 0.0),
                );
                let mut ui = child(ui, content);
                ui.label(
                    RichText::new(self.page.title())
                        .size(20.0)
                        .strong()
                        .color(palette::text_bright()),
                );
                ui.add_space(2.0);
                ui.label(RichText::new(self.page.summary()).color(palette::text_weak()));
                ui.add_space(4.0);
                let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
                if let Some(offset) = self.scroll_to.take() {
                    area = area.vertical_scroll_offset(offset);
                }
                area.show(&mut ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin {
                            left: 0,
                            right: 18,
                            top: 0,
                            bottom: 18,
                        })
                        .show(ui, |ui| match self.page {
                            Page::General => general(ui, settings),
                            Page::Sound => sound(ui, settings, &self.devices, &mut self.naming),
                            Page::Updates => updates_page(ui, updates),
                        });
                });
            });
        if close
            || (!fresh
                && (ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    || modal.should_close()))
        {
            self.open = false;
        }
        changed.device = settings.output_device != before.output_device;
        changed.dsp = settings.dsp != before.dsp;
        changed.look = settings.look() != before.look();
        changed.any = *settings != before || updates.enabled != checking;
        changed
    }

    /// The list of categories in `rect`.
    fn categories(&mut self, ui: &mut Ui, rect: Rect) {
        let mut list = child(ui, rect);
        list.spacing_mut().item_spacing.y = 2.0;
        for page in Page::ALL {
            self.category(&mut list, page);
        }
    }

    /// One row of the categories.
    fn category(&mut self, ui: &mut Ui, page: Page) {
        let (row, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        let selected = self.page == page;
        let fill = if selected {
            palette::pressed()
        } else if response.hovered() {
            palette::hover()
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(row, 6.0, fill);
        if selected {
            // A short accent bar at the left edge marks the page.
            let bar =
                Rect::from_center_size(pos2(row.left() + 3.0, row.center().y), vec2(3.0, 16.0));
            ui.painter().rect_filled(bar, 1.5, palette::accent());
        }
        let color = if selected {
            palette::text_bright()
        } else {
            palette::text()
        };
        ui.painter().text(
            pos2(row.left() + 16.0, row.center().y),
            Align2::LEFT_CENTER,
            page.title(),
            FontId::proportional(14.0),
            color,
        );
        if response.clicked() {
            self.page = page;
        }
    }
}

/// Rounder buttons, lists and fields in the window.
pub(crate) fn round_widgets(ui: &mut Ui) {
    let w = &mut ui.style_mut().visuals.widgets;
    for v in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        v.corner_radius = CornerRadius::same(5);
    }
    ui.spacing_mut().button_padding = vec2(10.0, 4.0);
}

/// A part of the window in `rect`, laid out from the top.
fn child(ui: &mut Ui, rect: Rect) -> Ui {
    ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    )
}

/// A card: a lighter rounded ground without a frame.
fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(palette::card_bg())
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(16, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// A titled group of settings: the title over a card.
fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(16.0);
    ui.label(RichText::new(title).strong().color(palette::text_bright()));
    ui.add_space(4.0);
    card(ui, add);
}

/// A line between the rows of a card.
fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, palette::line()),
    );
}

/// A row of a card: the name with an optional hint under it on the left,
/// what `control` adds right-aligned in `control_width` points on the
/// right, centred vertically.
fn row<R>(
    ui: &mut Ui,
    title: &str,
    hint: Option<&str>,
    control_width: f32,
    control: impl FnOnce(&mut Ui) -> R,
) -> R {
    const PAD: f32 = 8.0;
    let width = ui.available_width();
    let text_width = (width - control_width - 24.0).max(80.0);
    let title = ui.painter().layout(
        title.to_owned(),
        FontId::proportional(14.0),
        palette::text(),
        text_width,
    );
    let hint = hint.map(|h| {
        ui.painter().layout(
            h.to_owned(),
            FontId::proportional(12.5),
            palette::text_weak(),
            text_width,
        )
    });
    let text_height = title.size().y + hint.as_ref().map_or(0.0, |h| 2.0 + h.size().y);
    let height = text_height.max(ui.spacing().interact_size.y) + 2.0 * PAD;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut y = rect.center().y - text_height / 2.0;
    let title_height = title.size().y;
    ui.painter()
        .galley(pos2(rect.left(), y), title, palette::text());
    y += title_height + 2.0;
    if let Some(hint) = hint {
        ui.painter()
            .galley(pos2(rect.left(), y), hint, palette::text_weak());
    }
    let control_rect = Rect::from_min_max(pos2(rect.right() - control_width, rect.top()), rect.max);
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(control_rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    control(&mut ui)
}

/// An on/off switch: a pill with a knob that slides.
fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let size = vec2(40.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(response.id, *on);
        let hovered = response.hovered();
        let radius = rect.height() / 2.0;
        let painter = ui.painter();
        if *on {
            let fill = if hovered {
                palette::accent_hover()
            } else {
                palette::accent()
            };
            painter.rect_filled(rect, radius, fill);
        } else {
            painter.rect_filled(rect, radius, palette::track());
        }
        let knob = if *on || hovered {
            palette::text_bright()
        } else {
            palette::text_weak()
        };
        let x = egui::lerp(rect.left() + radius..=rect.right() - radius, t);
        painter.circle_filled(pos2(x, rect.center().y), radius - 4.0 + t, knob);
    }
    response
}

const CONTROL: f32 = 250.0;

fn general(ui: &mut Ui, settings: &mut Settings) {
    section(ui, tr!("Interface", "Интерфейс"), |ui| {
        row(
            ui,
            tr!("Language", "Язык"),
            Some(tr!(
                "\"As in Windows\" follows the language of Windows.",
                "«Как в Windows» следует языку Windows."
            )),
            CONTROL,
            |ui| {
                let mut choice = settings.language;
                egui::ComboBox::from_id_salt("settings_language")
                    .selected_text(choice.label())
                    .width(CONTROL - 10.0)
                    .show_ui(ui, |ui| {
                        for c in LangChoice::ALL {
                            ui.selectable_value(&mut choice, c, c.label());
                        }
                    });
                if choice != settings.language {
                    settings.language = choice;
                    crate::i18n::set_lang(choice.resolve());
                }
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Theme", "Тема"),
            Some(tr!(
                "The grounds and the text, from the darkest to the lightest.",
                "Фон и текст, от самой тёмной до самой светлой."
            )),
            CONTROL,
            |ui| {
                egui::ComboBox::from_id_salt("settings_theme")
                    .selected_text(settings.theme.label())
                    .width(CONTROL - 10.0)
                    .show_ui(ui, |ui| {
                        for t in Theme::ALL {
                            ui.selectable_value(&mut settings.theme, t, t.label());
                        }
                    });
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Colour", "Цвет"),
            Some(tr!(
                "Of the controls, the selection and the track playing.",
                "Кнопок управления, выделения и играющего трека."
            )),
            ACCENTS_WIDTH,
            |ui| accents(ui, &mut settings.accent),
        );
        divider(ui);
        row(
            ui,
            tr!("Selection", "Выделение"),
            Some(tr!(
                "The ground of the selected rows: mixed from the colour above, from one of its own, or one's own colour as it is.",
                "Фон выбранных строк: из цвета выше, своего оттенка или своего цвета как есть."
            )),
            ACCENTS_WIDTH,
            |ui| {
                let look = settings.look();
                tints(
                    ui,
                    &mut settings.selection,
                    &Tint::SELECTION,
                    look.accent,
                    |t| {
                        Palette::of(palette::Look {
                            selection: t,
                            ..look
                        })
                        .selected_bg
                    },
                );
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Track playing", "Играющий трек"),
            Some(tr!(
                "Its text in the lists and the playlist: mixed from the colour above, the brightest, a shade of its own, or one's own colour as it is.",
                "Его текст в списках и плейлисте: из цвета выше, самый яркий, свой оттенок или свой цвет как есть."
            )),
            ACCENTS_WIDTH,
            |ui| {
                let look = settings.look();
                tints(
                    ui,
                    &mut settings.playing,
                    &Tint::PLAYING,
                    look.accent,
                    |t| Palette::of(palette::Look { playing: t, ..look }).playing,
                );
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Player bar", "Панель плеера"),
            Some(tr!(
                "The time in large digits and a visualization on the right (the right button on it chooses one); or the compact bar of one row.",
                "Крупное время и визуализация справа (правая кнопка на ней — выбор); или компактная панель в одну строку."
            )),
            CONTROL,
            |ui| {
                egui::ComboBox::from_id_salt("settings_bar")
                    .selected_text(settings.bar_layout.label())
                    .width(CONTROL - 10.0)
                    .show_ui(ui, |ui| {
                        for l in BarLayout::ALL {
                            ui.selectable_value(&mut settings.bar_layout, l, l.label());
                        }
                    });
            },
        );
    });
    section(ui, tr!("Media keys", "Медиаклавиши"), |ui| {
        row(
            ui,
            tr!("Media keys", "Медиаклавиши"),
            Some(tr!(
                "The keys of keyboards, headsets and remotes. \"Always\": also while another window is active, through Windows' media controls, whose panel by the volume shows the track.",
                "Клавиши клавиатур, гарнитур и пультов. «Всегда»: и когда активно другое окно, через медиаэлементы Windows, панель которых у регулятора громкости показывает трек."
            )),
            CONTROL,
            |ui| {
                egui::ComboBox::from_id_salt("settings_media_keys")
                    .selected_text(settings.media_keys.label())
                    .width(CONTROL - 10.0)
                    .show_ui(ui, |ui| {
                        for m in MediaKeys::ALL {
                            ui.selectable_value(&mut settings.media_keys, m, m.label());
                        }
                    });
            },
        );
    });
}

fn sound(ui: &mut Ui, settings: &mut Settings, devices: &[Device], naming: &mut Option<String>) {
    section(ui, tr!("Output", "Вывод"), |ui| {
        row(
            ui,
            tr!("Device", "Устройство"),
            Some(tr!(
                "Windows' default device follows the one chosen in Windows. A device that is not present when a track starts is replaced by the default one until it is back.",
                "Устройство Windows по умолчанию следует выбранному в Windows. Устройство, которого нет при запуске трека, подменяется устройством по умолчанию, пока оно не вернётся."
            )),
            CONTROL,
            |ui| {
                let system = tr!("Windows' default", "Устройство Windows по умолчанию");
                let name = if settings.output_device.is_empty() {
                    system.to_owned()
                } else {
                    devices
                        .iter()
                        .find(|d| d.id == settings.output_device)
                        .map_or_else(
                            || {
                                tr!(
                                    format!("{} (not present)", settings.output_device_name),
                                    format!("{} (нет)", settings.output_device_name)
                                )
                            },
                            |d| d.name.clone(),
                        )
                };
                // Cut by its width: Cyrillic is wider than a count of
                // letters allowed for.
                egui::ComboBox::from_id_salt("settings_device")
                    .selected_text(name)
                    .truncate()
                    .width(CONTROL - 10.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(settings.output_device.is_empty(), system)
                            .clicked()
                        {
                            settings.output_device.clear();
                            settings.output_device_name.clear();
                        }
                        for d in devices {
                            if ui
                                .selectable_label(settings.output_device == d.id, &d.name)
                                .clicked()
                            {
                                settings.output_device = d.id.clone();
                                settings.output_device_name = d.name.clone();
                            }
                        }
                    });
            },
        );
    });
    processing(ui, &mut settings.dsp, &mut settings.eq_presets, naming);
}

/// The loudness, the equalizer and the enhancer.
fn processing(
    ui: &mut Ui,
    dsp: &mut DspSettings,
    presets: &mut Vec<EqPreset>,
    naming: &mut Option<String>,
) {
    section(ui, tr!("Loudness", "Громкость"), |ui| {
        row(
            ui,
            tr!(
                "The same loudness for every track",
                "Одна громкость у всех треков"
            ),
            Some(tr!(
                "A track is measured (EBU R128) in the background before its first play and brought to -18 LUFS; the measurement is kept, so each track is read once. A quiet track is raised only as far as its peaks allow.",
                "Трек измеряется (EBU R128) в фоне перед первым воспроизведением и приводится к -18 LUFS; замер сохраняется, так что каждый трек читается один раз. Тихий трек поднимается лишь настолько, насколько позволяют его пики."
            )),
            50.0,
            |ui| switch(ui, &mut dsp.normalize),
        );
    });
    section(ui, tr!("Equalizer", "Эквалайзер"), |ui| {
        ui.add_space(8.0);
        eq_head(ui, dsp, presets, naming);
        bands(ui, dsp);
    });
    section(ui, tr!("Enhancer", "Улучшение звука"), |ui| {
        row(
            ui,
            tr!("Enhancer", "Улучшение звука"),
            Some(tr!(
                "What the classic sound improvers did, in four amounts; 0 leaves a part alone.",
                "То, что делали классические улучшайзеры звука, четырьмя регуляторами; 0 оставляет часть без изменений."
            )),
            50.0,
            |ui| switch(ui, &mut dsp.enhancer),
        );
        divider(ui);
        amount(
            ui,
            tr!("Bass", "Бас"),
            tr!(
                "A shelf under 100 Hz and the harmonics of the bass, which small speakers make heard as bass they cannot play.",
                "Полка ниже 100 Гц и гармоники баса, которые на небольших колонках слышны как бас, недоступный им самим."
            ),
            &mut dsp.bass,
        );
        divider(ui);
        amount(
            ui,
            tr!("Clarity", "Ясность"),
            tr!(
                "Air over 6 kHz, the harmonics of the top: what an MP3's low-pass took away.",
                "Воздух выше 6 кГц, гармоники верха: то, что срезал фильтр MP3."
            ),
            &mut dsp.clarity,
        );
        divider(ui);
        amount(
            ui,
            tr!("Width", "Ширина"),
            tr!(
                "The stereo image wider, the bass kept in the middle.",
                "Шире стереокартина, бас остаётся в центре."
            ),
            &mut dsp.width,
        );
        divider(ui);
        amount(
            ui,
            tr!("Warmth", "Теплота"),
            tr!(
                "A soft rounding of the peaks, as tape does.",
                "Мягкое скругление пиков, как у плёнки."
            ),
            &mut dsp.warmth,
        );
    });
    section(ui, tr!("Limiter", "Лимитер"), |ui| {
        row(
            ui,
            tr!("Limiter", "Лимитер"),
            Some(tr!(
                "Holds the peaks at -1 dBFS after the processing, so raised bands, preamp or enhancer never clip; a raised preamp then makes a loud track denser rather than louder. Turned off, a raise is heard in full, and what passes full scale is clipped by Windows.",
                "Держит пики на -1 dBFS после обработки, чтобы поднятые полосы, предусиление или улучшение звука не клиповали; поднятое предусиление тогда делает громкий трек плотнее, а не громче. Если его выключить, подъём слышен целиком, а то, что выходит за полную шкалу, обрезает Windows."
            )),
            50.0,
            |ui| switch(ui, &mut dsp.limiter),
        );
    });
}

/// A row with a slider of 0 to 10.
fn amount(ui: &mut Ui, title: &str, hint: &str, value: &mut u8) {
    row(ui, title, Some(hint), CONTROL, |ui| {
        ui.spacing_mut().slider_width = CONTROL - 50.0;
        ui.add(egui::Slider::new(value, 0..=10));
    });
}

/// The width of a small icon button (`tool`).
pub(crate) const TOOL: f32 = 26.0;
/// The most presets of the user's kept; saving one more drops the oldest.
const MAX_OWN_PRESETS: usize = 20;

/// A small icon button, as metalmpv's: `icon` in `TOOL` points, lit
/// under the pointer, greyed and inert while not `enabled`.
pub(crate) fn tool(ui: &mut Ui, tip: &str, enabled: bool, icon: Icon) -> Response {
    let enabled = enabled && ui.is_enabled();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(TOOL, 24.0), sense);
    let color = if !enabled {
        palette::text_weak().gamma_multiply(0.5)
    } else if response.hovered() {
        palette::text_bright()
    } else {
        palette::text()
    };
    let mut fill = palette::card_bg();
    if enabled && response.hovered() {
        fill = if response.is_pointer_button_down_on() {
            palette::pressed()
        } else {
            palette::hover()
        };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    icons::paint(ui.painter(), icon, rect.center(), icons::SIZE, color, fill);
    response.on_hover_text(tip)
}

/// The equalizer's switch, the built-in presets and the user's own in one
/// list (the one the sliders stand at named; "Custom" otherwise), and
/// beside it saving the sliders as one's own and deleting one's own; the
/// name is typed in a row under it (metalmpv's equalizer head).
fn eq_head(
    ui: &mut Ui,
    dsp: &mut DspSettings,
    presets: &mut Vec<EqPreset>,
    naming: &mut Option<String>,
) {
    // The one of mine chosen last while the sliders still stand at it,
    // else the first that does: two of the same values are two presets.
    let same = |p: &EqPreset| p.bands == dsp.bands && p.preamp == dsp.preamp;
    let chosen_id = egui::Id::new("eq_mine");
    let mine = ui
        .data(|d| d.get_temp::<usize>(chosen_id))
        .filter(|&i| presets.get(i).is_some_and(same))
        .or_else(|| presets.iter().position(same));
    // A built-in preset sets the preamp to 0: one stands at it only so.
    let built_in = eq::PRESETS
        .iter()
        .find(|(_, _, bands)| *bands == dsp.bands && dsp.preamp == 0.0)
        .map(|(en, ru, _)| tr!(*en, *ru));
    ui.horizontal(|ui| {
        switch(ui, &mut dsp.eq).on_hover_text(tr!(
            "Ten bands and a preamp; a limiter after them keeps the sound from clipping.",
            "Десять полос и предусиление; лимитер после них не даёт звуку клиповать."
        ));
        ui.add_space(4.0);
        let name = match (mine, built_in) {
            (Some(i), _) => presets[i].name.clone(),
            (None, Some(p)) => p.to_owned(),
            (None, None) => tr!("Custom", "Другой").to_owned(),
        };
        let width = ui.available_width() - 2.0 * TOOL - 3.0 * ui.spacing().item_spacing.x;
        egui::ComboBox::from_id_salt("eq_preset")
            .selected_text(name)
            .width(width)
            .show_ui(ui, |ui| {
                for (en, ru, bands) in eq::PRESETS {
                    let on = mine.is_none() && bands == dsp.bands && dsp.preamp == 0.0;
                    if ui.selectable_label(on, tr!(en, ru)).clicked() {
                        dsp.bands = bands;
                        dsp.preamp = 0.0;
                        // Flat leaves the switch as it is: nothing to hear.
                        if bands.iter().any(|&b| b != 0.0) {
                            dsp.eq = true;
                        }
                    }
                }
                if !presets.is_empty() {
                    ui.separator();
                    ui.label(
                        RichText::new(tr!("Mine", "Мои"))
                            .small()
                            .color(palette::text_weak()),
                    );
                    for (i, p) in presets.iter().enumerate() {
                        if ui.selectable_label(mine == Some(i), &p.name).clicked() {
                            dsp.bands = p.bands;
                            dsp.preamp = p.preamp;
                            dsp.eq = true;
                            ui.data_mut(|d| d.insert_temp(chosen_id, i));
                        }
                    }
                }
            });
        if tool(
            ui,
            tr!("Save as my own…", "Сохранить как свой…"),
            naming.is_none(),
            Icon::Plus,
        )
        .clicked()
        {
            *naming = Some(mine.map(|i| presets[i].name.clone()).unwrap_or_default());
        }
        if tool(
            ui,
            tr!("Delete this preset of mine", "Удалить этот свой пресет"),
            mine.is_some(),
            Icon::Delete,
        )
        .clicked()
            && let Some(i) = mine
        {
            presets.remove(i);
            ui.data_mut(|d| d.remove::<usize>(chosen_id));
        }
    });
    // A name in any case is the same name ("Rock" is "rock").
    let same_name = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    let full = presets.len() >= MAX_OWN_PRESETS;
    let names: Vec<String> = presets.iter().map(|p| p.name.clone()).collect();
    if let Some(name) = name_row(
        ui,
        naming,
        tr!("My equalizer", "Мой эквалайзер"),
        |n| full && !names.iter().any(|o| same_name(o, n)),
    ) {
        let name: String = name.chars().take(MAX_NAME).collect();
        let at = match presets.iter().position(|p| same_name(&p.name, &name)) {
            Some(i) => {
                let p = &mut presets[i];
                p.name = name;
                p.preamp = dsp.preamp;
                p.bands = dsp.bands;
                i
            }
            None => {
                // A list made longer by hand keeps only the newest.
                while presets.len() >= MAX_OWN_PRESETS {
                    presets.remove(0);
                }
                presets.push(EqPreset {
                    name,
                    preamp: dsp.preamp,
                    bands: dsp.bands,
                });
                presets.len() - 1
            }
        };
        ui.data_mut(|d| d.insert_temp(chosen_id, at));
    }
}

/// The longest name of a preset of one's own, in letters.
const MAX_NAME: usize = 40;

/// The row where a name is typed for what is to be saved, while `naming`
/// holds it: Enter or the tick saves (the name returned), Esc or the
/// cross gives up. `drops_oldest`: saving that name drops the oldest of a
/// full list, which is said under it.
fn name_row(
    ui: &mut Ui,
    naming: &mut Option<String>,
    hint: &str,
    drops_oldest: impl Fn(&str) -> bool,
) -> Option<String> {
    let name = naming.as_mut()?;
    let (mut done, mut saved) = (false, None);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_sized(
            vec2(60.0, ui.spacing().interact_size.y),
            egui::Label::new(tr!("Name", "Название")),
        );
        let width = ui.available_width() - 2.0 * TOOL - 3.0 * ui.spacing().item_spacing.x;
        let field = ui.add(
            egui::TextEdit::singleline(name)
                .hint_text(hint)
                .desired_width(width),
        );
        if ui.memory(|m| m.focused().is_none()) {
            field.request_focus();
        }
        let filled = !name.trim().is_empty();
        // Esc lets the field go before it is drawn.
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let enter = super::take_leaving_key(ui, &field);
        let ok = tool(ui, tr!("Save", "Сохранить"), filled, Icon::Check);
        if (enter || ok.clicked()) && filled {
            saved = Some(name.trim().to_owned());
            done = true;
        }
        let cancel = tool(ui, tr!("Cancel", "Отмена"), true, Icon::Close);
        if cancel.clicked() || escape {
            // Not the window's Esc too.
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            done = true;
        }
    });
    if drops_oldest(name.trim()) {
        ui.label(
            RichText::new(tr!(
                "The list is full: the oldest of mine gives way.",
                "Список полон: самый старый свой уступит место."
            ))
            .size(12.5)
            .color(palette::text_weak()),
        );
    }
    if done {
        *naming = None;
    }
    saved
}

/// A gain for a label: `+4`, `−3.5` (a minus sign, not a hyphen; `−3,5`
/// in Russian), `0`.
fn gain_text(gain: f32) -> String {
    let text = if gain == 0.0 {
        "0".to_owned()
    } else if gain.fract() == 0.0 {
        format!("{gain:+.0}")
    } else {
        format!("{gain:+.1}")
    };
    let text = text.replace('-', "\u{2212}");
    tr!(text.clone(), text.replace('.', ","))
}

/// The bands as metalmpv's: the preamp's column, a scale of ±12 dB, then
/// a vertical slider per band with its gain over it and its frequency
/// and reset under it, the 0 dB line across and a curve through the
/// knobs drawn under them; greyed while the equalizer is off.
fn bands(ui: &mut Ui, dsp: &mut DspSettings) {
    let on = dsp.eq;
    ui.add_space(6.0);
    ui.add_enabled_ui(on, |ui| {
        let scale = 28.0;
        let column = ((ui.available_width() - scale) / (eq::BANDS.len() + 1) as f32).floor();
        let under = ui.painter().add(egui::Shape::Noop);
        let mut knobs = Vec::new();
        let mut travel = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.spacing_mut().slider_width = 130.0;
            // One column: the gain, the slider, the label, the reset; the
            // knob's place for the curve.
            let band = |ui: &mut Ui, value: &mut f32, label: String, hz: Option<f32>| {
                ui.allocate_ui_with_layout(
                    vec2(column, 216.0),
                    Layout::top_down(Align::Center),
                    |ui| {
                        let color = if *value != 0.0 && on {
                            palette::accent_hover()
                        } else {
                            palette::text_weak()
                        };
                        ui.label(RichText::new(gain_text(*value)).size(11.0).color(color));
                        let tip = match hz {
                            Some(hz) => tr!(
                                format!("{hz} Hz; a double click puts it back to 0"),
                                format!("{hz} Гц; двойной щелчок возвращает 0")
                            ),
                            None => tr!(
                                "The preamp, before the bands; a double click puts it back to 0",
                                "Предусиление перед полосами; двойной щелчок возвращает 0"
                            )
                            .to_owned(),
                        };
                        let response = ui
                            .add(
                                egui::Slider::new(value, -eq::RANGE_DB..=eq::RANGE_DB)
                                    .vertical()
                                    .show_value(false)
                                    .step_by(0.5),
                            )
                            .on_hover_text(tip);
                        let r = response.rect;
                        let radius = r.width() / 2.5;
                        let (bottom, top) = (r.bottom() - radius, r.top() + radius);
                        let t = (*value + eq::RANGE_DB) / (2.0 * eq::RANGE_DB);
                        let knob = pos2(r.center().x, egui::lerp(bottom..=top, t));
                        ui.label(RichText::new(label).size(11.0));
                        if slider_double_clicked(ui, &response)
                            || tool(
                                ui,
                                tr!("Back to 0", "Вернуть 0"),
                                *value != 0.0,
                                Icon::RotateLeft,
                            )
                            .clicked()
                        {
                            *value = 0.0;
                        }
                        (knob, (top, bottom))
                    },
                )
                .inner
            };
            let (_, t) = band(ui, &mut dsp.preamp, tr!("Pre", "Пред.").to_owned(), None);
            travel = Some(t);
            let (gutter, _) = ui.allocate_exact_size(vec2(scale, 10.0), Sense::hover());
            for (i, &hz) in eq::BANDS.iter().enumerate() {
                let label = if hz >= 1000.0 {
                    format!("{}k", hz / 1000.0)
                } else {
                    format!("{hz}")
                };
                let (knob, _) = band(ui, &mut dsp.bands[i], label, Some(hz));
                knobs.push(knob);
            }
            if let Some((top, bottom)) = travel {
                let x = gutter.right() - 6.0;
                let font = FontId::proportional(10.0);
                let painter = ui.painter();
                for (y, text) in [(top, "+12"), ((top + bottom) / 2.0, "0"), (bottom, "−12")] {
                    painter.text(
                        pos2(x, y),
                        Align2::RIGHT_CENTER,
                        text,
                        font.clone(),
                        palette::text_weak(),
                    );
                }
            }
        });
        let mut shapes = Vec::new();
        if let (Some((top, bottom)), Some(first), Some(last)) =
            (travel, knobs.first(), knobs.last())
        {
            let zero = (top + bottom) / 2.0;
            let span = (first.x - column / 2.0)..=(last.x + column / 2.0);
            shapes.push(egui::Shape::hline(
                span,
                zero,
                Stroke::new(1.0, palette::line()),
            ));
        }
        let curve = if on {
            palette::accent().gamma_multiply(0.8)
        } else {
            palette::text_weak().gamma_multiply(0.4)
        };
        shapes.push(egui::Shape::line(smooth(&knobs), Stroke::new(2.0, curve)));
        ui.painter().set(under, egui::Shape::Vec(shapes));
    });
    ui.add_space(4.0);
}

/// A double click on a slider. egui's slider senses only drags, so it is
/// never clicked and `Response::double_clicked` never holds: a press on it
/// starts a drag at once, and the double click is the release of such a
/// drag that the pointer reports as the second (or third) click.
fn slider_double_clicked(ui: &Ui, response: &Response) -> bool {
    response.drag_stopped()
        && ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
                || i.pointer
                    .button_triple_clicked(egui::PointerButton::Primary)
        })
}

/// A curve through `points` (Catmull-Rom, eight steps a span).
fn smooth(points: &[egui::Pos2]) -> Vec<egui::Pos2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let at = |i: isize| points[i.clamp(0, points.len() as isize - 1) as usize].to_vec2();
    let mut out = Vec::new();
    for i in 0..points.len() as isize - 1 {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        for step in 0..8 {
            let t = step as f32 / 8.0;
            let (t2, t3) = (t * t, t * t * t);
            let v = 0.5
                * ((2.0 * p1)
                    + (p2 - p0) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3);
            out.push(v.to_pos2());
        }
    }
    out.push(*points.last().expect("three points or more"));
    out
}

/// The width of the accents' row: ten swatches.
const ACCENTS_WIDTH: f32 = 275.0;
/// The size of an accent's swatch.
const SWATCH: f32 = 22.0;

/// The accents as round swatches, the chosen one ringed, and last the
/// user's own colour, picked in a popup (metalmpv's).
fn accents(ui: &mut Ui, accent: &mut Accent) {
    ui.spacing_mut().item_spacing.x = 5.0;
    // Right to left: the own colour first, then the ready ones from the last.
    own_accent(ui, accent);
    for a in Accent::ALL.into_iter().rev() {
        let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
        let response = response.on_hover_text(a.label());
        let selected = *accent == a;
        swatch(ui, rect, selected, response.hovered(), |p, centre, r| {
            p.circle_filled(centre, r, a.base());
        });
        if response.clicked() {
            *accent = a;
        }
    }
}

/// A round swatch in `rect`: ringed when chosen, a little bigger under the
/// pointer; `fill` paints the inside at a centre and a radius.
fn swatch(
    ui: &Ui,
    rect: Rect,
    selected: bool,
    hovered: bool,
    fill: impl FnOnce(&egui::Painter, egui::Pos2, f32),
) {
    let painter = ui.painter();
    let centre = rect.center();
    let radius = SWATCH / 2.0;
    if selected {
        painter.circle_stroke(
            centre,
            radius - 1.0,
            Stroke::new(2.0, palette::text_bright()),
        );
        fill(painter, centre, radius - 4.5);
    } else {
        let grow = if hovered { 1.0 } else { 0.0 };
        fill(painter, centre, radius - 3.0 + grow);
    }
}

fn custom([r, g, b]: [u8; 3]) -> Accent {
    Accent::Custom(u32::from_be_bytes([0, r, g, b]))
}

/// A shade's choices (the selection's ground, the track playing's text)
/// as round swatches, each as the lists show it (`colour_of`) in this
/// theme, the chosen one ringed; the first, mixed from the accent, with
/// the accent's dot in it; last the user's own colour, as for the accent,
/// its picker starting from the shade in use.
fn tints(
    ui: &mut Ui,
    tint: &mut Tint,
    offered: &[Tint],
    accent: Accent,
    colour_of: impl Fn(Tint) -> Color32,
) {
    ui.spacing_mut().item_spacing.x = 5.0;
    let mut own = match *tint {
        Tint::Custom(c) => Accent::Custom(c),
        t => {
            let c = colour_of(t);
            custom([c.r(), c.g(), c.b()])
        }
    };
    let chosen = matches!(tint, Tint::Custom(_));
    if own_colour(ui, &mut own, chosen)
        && let Accent::Custom(c) = own
    {
        *tint = Tint::Custom(c);
    }
    for &s in offered.iter().rev() {
        let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
        let response = response.on_hover_text(s.label());
        let fill = colour_of(s);
        swatch(ui, rect, *tint == s, response.hovered(), |p, centre, r| {
            // Outlined: a dark ground is hardly seen on a dark card.
            let edge = palette::text_weak().gamma_multiply(0.7);
            p.circle(centre, r, fill, Stroke::new(1.0, edge));
            if s == Tint::Accent {
                p.circle_filled(centre, r * 0.35, accent.base());
            }
        });
        if response.clicked() {
            *tint = s;
        }
    }
}

/// The width of the own colour's picker, its square and its hue bar.
const PICKER: f32 = 180.0;

/// The own colour's swatch: a colour wheel, or the colour once chosen; a
/// click opens the picker, a field for its code (HEX, RGB or HSL) and R,
/// G, B fields, which start from the accent in use.
fn own_accent(ui: &mut Ui, accent: &mut Accent) {
    let own = matches!(accent, Accent::Custom(_));
    let mut edited = *accent;
    if own_colour(ui, &mut edited, own) {
        *accent = edited;
    }
}

/// An own colour's swatch and its picker (`accent` is `Accent::Custom`
/// while picked: its colour is where the picker starts); `chosen`: the
/// own colour is the one in use. True when a colour was picked.
fn own_colour(ui: &mut Ui, accent: &mut Accent, chosen: bool) -> bool {
    let before = *accent;
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    let response = response.on_hover_text(tr!("Your own colour…", "Свой цвет…"));
    let own = chosen;
    let current = accent.base();
    swatch(ui, rect, own, response.hovered(), |p, centre, r| {
        if own {
            p.circle_filled(centre, r, current);
        } else {
            colour_wheel(p, centre, r);
        }
    });
    egui::Popup::menu(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut colour = accent.base();
            if colour_picker(ui, &mut colour) {
                *accent = custom([colour.r(), colour.g(), colour.b()]);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // The code's form, HEX, RGB or HSL, changed by a click on
                // its name; the field takes any of them.
                let mode_id = ui.id().with("accent_text_mode");
                let mut mode = ui.data(|d| d.get_temp::<ColourText>(mode_id).unwrap_or_default());
                let label = ui
                    .add(egui::Label::new(mode.name()).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(tr!(
                        "Show the code as HEX, RGB or HSL",
                        "Показать код как HEX, RGB или HSL"
                    ));
                if label.clicked() {
                    mode = mode.next();
                    ui.data_mut(|d| d.insert_temp(mode_id, mode));
                }
                // Kept as typed while the field has the focus.
                let id = ui.id().with("accent_hex");
                let base = accent.base();
                let mut text = ui
                    .data_mut(|d| d.get_temp::<String>(id))
                    .unwrap_or_else(|| mode.show([base.r(), base.g(), base.b()]));
                let width = PICKER - label.rect.width() - ui.spacing().item_spacing.x;
                let field = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width));
                super::take_leaving_key(ui, &field);
                if field.changed()
                    && let Some(typed) = Accent::typed(&text)
                {
                    *accent = typed;
                }
                if field.has_focus() {
                    ui.data_mut(|d| d.insert_temp(id, text));
                } else {
                    ui.data_mut(|d| d.remove::<String>(id));
                }
            });
            // The same colour as red, green and blue, each 0–255, in
            // three fields as wide as the picker.
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let spacing = ui.spacing().item_spacing.x;
                let size = vec2((PICKER - 2.0 * spacing) / 3.0, ui.spacing().interact_size.y);
                let base = accent.base();
                let mut rgb = [base.r(), base.g(), base.b()];
                let mut edited = false;
                for (value, name) in rgb.iter_mut().zip(["R ", "G ", "B "]) {
                    let field = egui::DragValue::new(value)
                        .speed(0.5)
                        .range(0..=255)
                        .prefix(name);
                    let field = ui.add_sized(size, field);
                    super::take_leaving_key(ui, &field);
                    edited |= field.changed();
                }
                if edited {
                    *accent = custom(rgb);
                }
            });
        });
    *accent != before
}

/// The own colour's picker: saturation across and brightness up a square,
/// the hue along a bar under it (metalmpv's). True when `colour` was
/// changed.
fn colour_picker(ui: &mut Ui, colour: &mut Color32) -> bool {
    use egui::ecolor::HsvaGamma;
    // The hue kept while it gives the colour shown: a grey has none.
    let id = ui.id().with("accent_hsv");
    let mut hsv = ui
        .data(|d| d.get_temp::<(Color32, HsvaGamma)>(id))
        .filter(|(c, _)| c == colour)
        .map_or_else(|| HsvaGamma::from(*colour), |(_, hsv)| hsv);
    let before = hsv;
    let marker = |p: &egui::Painter, at: egui::Pos2, r: f32| {
        p.circle_stroke(at, r + 1.0, Stroke::new(1.0, Color32::BLACK));
        p.circle_stroke(at, r, Stroke::new(1.5, Color32::WHITE));
    };

    let (square, response) = ui.allocate_exact_size(vec2(PICKER, PICKER), Sense::click_and_drag());
    if let Some(at) = response.interact_pointer_pos() {
        hsv.s = egui::remap_clamp(at.x, square.x_range(), 0.0..=1.0);
        hsv.v = egui::remap_clamp(at.y, square.bottom()..=square.top(), 0.0..=1.0);
    }
    const STEPS: u32 = 24;
    let mut mesh = egui::Mesh::default();
    for row in 0..=STEPS {
        for column in 0..=STEPS {
            let (s, v) = (column as f32 / STEPS as f32, row as f32 / STEPS as f32);
            let at = square.lerp_inside(vec2(s, 1.0 - v));
            let shade = HsvaGamma {
                s,
                v,
                a: 1.0,
                ..hsv
            };
            mesh.colored_vertex(at, shade.into());
            if row < STEPS && column < STEPS {
                let i = row * (STEPS + 1) + column;
                mesh.add_triangle(i, i + 1, i + STEPS + 1);
                mesh.add_triangle(i + 1, i + STEPS + 2, i + STEPS + 1);
            }
        }
    }
    let painter = ui.painter();
    painter.add(mesh);
    marker(painter, square.lerp_inside(vec2(hsv.s, 1.0 - hsv.v)), 5.0);

    ui.add_space(6.0);
    let (bar, response) = ui.allocate_exact_size(vec2(PICKER, 14.0), Sense::click_and_drag());
    if let Some(at) = response.interact_pointer_pos() {
        hsv.h = egui::remap_clamp(at.x, bar.x_range(), 0.0..=1.0);
    }
    const HUES: u32 = 36;
    let mut mesh = egui::Mesh::default();
    for i in 0..=HUES {
        let h = i as f32 / HUES as f32;
        let x = egui::lerp(bar.x_range(), h);
        let hue: Color32 = HsvaGamma {
            h,
            s: 1.0,
            v: 1.0,
            a: 1.0,
        }
        .into();
        mesh.colored_vertex(pos2(x, bar.top()), hue);
        mesh.colored_vertex(pos2(x, bar.bottom()), hue);
        if i < HUES {
            let j = 2 * i;
            mesh.add_triangle(j, j + 1, j + 2);
            mesh.add_triangle(j + 1, j + 3, j + 2);
        }
    }
    let painter = ui.painter();
    painter.add(mesh);
    marker(
        painter,
        pos2(egui::lerp(bar.x_range(), hsv.h), bar.center().y),
        5.0,
    );

    let changed = hsv != before;
    if changed {
        *colour = hsv.into();
    }
    ui.data_mut(|d| d.insert_temp(id, (*colour, hsv)));
    changed
}

/// A disc of all the hues round its centre.
fn colour_wheel(painter: &egui::Painter, centre: egui::Pos2, radius: f32) {
    const SLICES: usize = 36;
    let point = |i: usize| {
        let angle = i as f32 / SLICES as f32 * std::f32::consts::TAU;
        centre + radius * vec2(angle.cos(), angle.sin())
    };
    for i in 0..SLICES {
        let hue = (i as f32 + 0.5) / SLICES as f32;
        let colour = Color32::from(egui::ecolor::Hsva::new(hue, 0.75, 0.95, 1.0));
        painter.add(egui::Shape::convex_polygon(
            vec![centre, point(i), point(i + 1)],
            colour,
            Stroke::new(0.6, colour),
        ));
    }
}

fn updates_page(ui: &mut Ui, updates: &mut Updates) {
    section(
        ui,
        tr!("Update check", "Проверка обновлений"),
        |ui| {
            let enabled = row(
                ui,
                tr!("Check at start-up", "Проверять при запуске"),
                Some(tr!("Once a day.", "Раз в сутки.")),
                50.0,
                |ui| switch(ui, &mut updates.enabled),
            );
            if enabled.changed() && updates.enabled {
                updates.start_if_due(ui.ctx());
            }
            divider(ui);
            let installed = tr!(
                format!("Installed version: {}", crate::VERSION),
                format!("Установленная версия: {}", crate::VERSION)
            );
            let checking = updates.status == Status::Checking;
            let check = row(ui, &installed, None, 160.0, |ui| {
                ui.add_enabled(
                    !checking,
                    egui::Button::new(tr!("Check now", "Проверить сейчас")),
                )
                .clicked()
            });
            if check {
                updates.start(ui.ctx());
            }
            if updates.status != Status::Unknown {
                ui.add_space(2.0);
                update_status(ui, updates);
                ui.add_space(8.0);
            }
        },
    );
    section(
        ui,
        tr!("Privacy", "Конфиденциальность"),
        |ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(tr!(
                "Only api.github.com is asked, with the program's version in the request; nothing is downloaded or installed. A newer version shows a button in the title row that opens its release page.",
                "Запрос уходит только к api.github.com и содержит лишь версию программы; ничего не скачивается и не устанавливается. О новой версии сообщает кнопка в строке заголовка, открывающая страницу выпуска."
            )).color(palette::text_weak()));
            ui.add_space(8.0);
        },
    );
}

/// The outcome of the last check, if any.
fn update_status(ui: &mut Ui, updates: &Updates) {
    match &updates.status {
        Status::Unknown => {}
        Status::Checking => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.weak(tr!("Checking…", "Проверка…"));
            });
        }
        Status::UpToDate => {
            ui.weak(tr!(
                "This is the latest version",
                "Установлена последняя версия"
            ));
        }
        Status::Newer(tag) => {
            let version = version_of(tag);
            let url = release_url(tag);
            if ui
                .link(tr!(
                    format!("Version {version} is available"),
                    format!("Доступна версия {version}")
                ))
                .on_hover_text(url.as_str())
                .clicked()
                && !crate::win::shell_open(&url)
            {
                log::warn!("could not open {url}");
            }
        }
        Status::Failed(e) => {
            ui.weak(tr!(
                "Could not check for updates",
                "Не удалось проверить обновления"
            ))
            .on_hover_text(e);
        }
    }
}

/// The settings of this window back to their defaults.
fn reset(settings: &mut Settings, updates: &mut Updates) {
    let d = Settings::default();
    if settings.language != d.language {
        crate::i18n::set_lang(d.language.resolve());
    }
    settings.language = d.language;
    settings.bar_layout = d.bar_layout;
    settings.theme = d.theme;
    settings.accent = d.accent;
    settings.selection = d.selection;
    settings.playing = d.playing;
    settings.media_keys = d.media_keys;
    settings.output_device = d.output_device;
    settings.output_device_name = d.output_device_name;
    settings.dsp = d.dsp;
    updates.enabled = d.check_updates;
}
