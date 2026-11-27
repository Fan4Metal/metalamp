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

use crate::audio::dsp::{DspSettings, Enhancement, enhancer, eq};
use crate::audio::output::Device;
use crate::i18n::LangChoice;
use crate::keymap::{Action, Chord, Group, Keymap};
use crate::palette;
use crate::palette::{Accent, Palette, Theme, Tint, VisualColours};
use crate::settings::{
    ApeTag, BarLayout, CoverPriority, EnhancerPreset, EqPreset, Id3Version, Id3v1Encoding,
    Id3v1Tag, MediaKeys,
    Settings, TrackNotice, Visual, WindowLayout,
};
use crate::ui::icons::{self, Icon};
use crate::update::{Status, Updates, release_url, version_of};

/// The size of the window's content in points.
const SIZE: egui::Vec2 = vec2(820.0, 560.0);
/// The width of the category column.
const SIDEBAR_WIDTH: f32 = 180.0;
const NARROW_SIDEBAR: f32 = 140.0;
/// The corner radius of the window.
const RADIUS: u8 = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    General,
    Sound,
    Keys,
    Updates,
}

impl Page {
    const ALL: [Page; 4] = [Page::General, Page::Sound, Page::Keys, Page::Updates];

    fn title(self) -> &'static str {
        match self {
            Page::General => tr!("General", "Общие"),
            Page::Sound => tr!("Sound", "Звук"),
            Page::Keys => tr!("Keyboard shortcuts", "Горячие клавиши"),
            Page::Updates => tr!("Updates", "Обновления"),
        }
    }

    /// The line under the page's title.
    fn summary(self) -> &'static str {
        match self {
            Page::General => tr!(
                "The language, the theme and colour, the layout of the player bar, the media keys and how tags are written.",
                "Язык, тема и цвет, вид панели плеера, медиаклавиши и запись тегов."
            ),
            Page::Sound => tr!(
                "The device the sound goes to; one loudness for every track, the limiter, the equalizer, the crossfade and the enhancer.",
                "Устройство, на которое выводится звук; одна громкость у всех треков, лимитер, эквалайзер, кроссфейд и улучшение звука."
            ),
            Page::Keys => tr!(
                "The keys of the player and what they do.",
                "Клавиши плеера и что они делают."
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
    /// The crossfade (`settings.crossfade`, `crossfade_in_albums`).
    pub crossfade: bool,
    /// The keys (`keymap::publish`).
    pub keys: bool,
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
    /// The name an equalizer preset and an enhancer preset is being saved
    /// under, while one is.
    naming: (Option<String>, Option<String>),
    /// A scroll offset to put the page at once (`from_env`).
    scroll_to: Option<f32>,
    /// A key being chosen on the Keys page: the action, and which of its
    /// keys it replaces (`None`: one more).
    capture: Option<(Action, Option<usize>)>,
    /// The press that gave the choice up: its click must not start it
    /// again.
    gave_up: Option<(Action, Option<usize>)>,
    /// What the last change of keys did that may not have been meant (a
    /// key taken from another action, an action's own keys reset) and the
    /// keys before, for Undo.
    taken: Option<(String, Keymap)>,
    /// What the Keys page shows: actions whose name or keys have it.
    key_filter: String,
    /// Open in a window of its own (decided as it opens), and that
    /// window's handle once found.
    windowed: bool,
    window: super::OwnWindow,
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
                "keys" => Page::Keys,
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
        owner: Option<isize>,
        settings: &mut Settings,
        updates: &mut Updates,
    ) -> Changed {
        let mut changed = Changed::default();
        if !self.open {
            return changed;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let before = settings.clone();
        if self.page != Page::Keys {
            self.capture = None;
        }
        // Kept by the update check, saved with the settings.
        let checking = updates.enabled;
        let mut close = false;
        // In a window of its own when the program's is too small for it
        // (the player and playlist layout): squeezed, its pages broke. As
        // it opens, and when the program's window gets smaller (that
        // layout chosen here); once out, it stays out until closed.
        let room = ctx.content_rect().size();
        let small = room.x < SIZE.x + 24.0 || room.y < SIZE.y + 24.0;
        let moved_out = !fresh && small && !self.windowed;
        if fresh || moved_out {
            self.windowed = small;
        }
        if self.windowed {
            close |= self.window(ctx, owner, settings, updates, fresh || moved_out);
        } else {
            self.window.reset();
            // Esc or Enter close it unless a list, a field or a key being
            // chosen takes them (as of the frame before: a field let go
            // by Enter or Esc leaves the key in the input).
            let free = !egui::Popup::is_any_open(ctx)
                && !ctx.text_edit_focused()
                && self.capture.is_none();
            let modal = egui::Modal::new(egui::Id::new("settings"))
                .frame(
                    egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                        .fill(palette::panel_bg())
                        .corner_radius(RADIUS)
                        .inner_margin(Margin::ZERO),
                )
                .show(ctx, |ui| {
                    close |= self.surface(ui, settings, updates, false);
                });
            close |= !fresh
                && (modal.backdrop_response.clicked()
                    || free
                        && ctx.input_mut(|i| {
                            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                                || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                        }));
        }
        if close {
            self.open = false;
            self.window.reset();
        }
        changed.device = settings.output_device != before.output_device;
        changed.dsp = settings.dsp != before.dsp;
        changed.crossfade = (
            settings.crossfade,
            settings.crossfade_in_albums,
            settings.crossfade_manual,
            settings.crossfade_seek,
        ) != (
            before.crossfade,
            before.crossfade_in_albums,
            before.crossfade_manual,
            before.crossfade_seek,
        );
        changed.look = settings.look() != before.look();
        changed.keys = settings.keys != before.keys;
        changed.any = *settings != before || updates.enabled != checking;
        changed
    }

    /// The window's content in `ui`: the pages' column with Reset and
    /// Close, the page; whether it is to close. `windowed`: in a window of
    /// its own, which its top strip moves.
    fn surface(
        &mut self,
        ui: &mut Ui,
        settings: &mut Settings,
        updates: &mut Updates,
        windowed: bool,
    ) -> bool {
        let ctx = ui.ctx().clone();
        // A key being chosen takes the keys before any widget sees them.
        self.capture_key(&ctx, &mut settings.keys);
        let mut close = false;
        round_widgets(ui);
        // Smaller in a small window, so Close and the cross stay
        // on the screen; a window of its own has its size.
        let size = if windowed {
            SIZE
        } else {
            super::modal_size(&ctx, SIZE, 12.0)
        };
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        // A window of its own moves by its top strip.
        if windowed {
            crate::frameless::drag_area(
                ui,
                Rect::from_min_size(rect.min, vec2(rect.width(), 56.0)),
                ui.id().with("settings_drag"),
                Sense::hover(),
            );
        }

        // The category column, flush with the window's left edge.
        // Narrower in a narrow window (the player and playlist
        // layout), so the page keeps its room.
        let sidebar_width = if size.x < 760.0 {
            NARROW_SIDEBAR
        } else {
            SIDEBAR_WIDTH
        };
        let sidebar = Rect::from_min_size(rect.min, vec2(sidebar_width, rect.height()));
        let inner = if windowed { 0 } else { RADIUS - 1 };
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
            vec2(sidebar_width - 24.0, BUTTON),
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
            self.naming = (None, None);
            self.capture = None;
            self.taken = None;
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
                    Page::Keys => self.keys_page(ui, &mut settings.keys),
                    Page::Updates => updates_page(ui, updates),
                });
        });
        close
    }

    /// The settings in a window of their own (metalmpv's playlist window's
    /// way): frameless, owned by the program's window (over it, minimized
    /// with it, off the taskbar), centred over it on its work area when
    /// opened. Esc and Enter close it, as the modal; whether to close.
    fn window(
        &mut self,
        ctx: &egui::Context,
        owner: Option<isize>,
        settings: &mut Settings,
        updates: &mut Updates,
        fresh: bool,
    ) -> bool {
        if fresh {
            self.window.reset();
        }
        let centre = ctx
            .input(|i| i.viewport().outer_rect)
            .unwrap_or(ctx.content_rect())
            .center();
        let builder = super::OwnWindow::builder(egui::ViewportBuilder::default())
            .with_title(tr!("Settings", "Настройки"))
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(SIZE)
            .with_position(centre - SIZE / 2.0)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            .with_icon(crate::window_icon());
        let id = egui::ViewportId::from_hash_of("settings_window");
        let mut close = false;
        let dark = settings.theme.is_dark();
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            self.window
                .follow(&ctx, tr!("Settings", "Настройки"), owner, dark);
            close |= ctx.input(|i| i.viewport().close_requested());
            // Esc or Enter as the modal's, unless a list, a field or a
            // key being chosen takes them (as of the frame before).
            let free = !egui::Popup::is_any_open(&ctx)
                && !ctx.text_edit_focused()
                && self.capture.is_none();
            if free
                && !fresh
                && ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                })
            {
                close = true;
            }
            egui::CentralPanel::no_frame().show(ui, |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, palette::panel_bg());
                close |= self.surface(ui, settings, updates, true);
            });
        });
        close
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

impl SettingsDialog {
    /// The next press while a key is being chosen: a key with its
    /// modifiers, or a side button of the mouse; Esc or another click
    /// gives up. The keys pressed meanwhile reach no widget and do not
    /// close the window.
    fn capture_key(&mut self, ctx: &egui::Context, keys: &mut Keymap) {
        let Some((action, slot)) = self.capture else {
            return;
        };
        let (events, modifiers) = ctx.input(|i| (i.events.clone(), i.modifiers));
        for event in &events {
            match event {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => {
                    self.capture = None;
                    break;
                }
                egui::Event::PointerButton {
                    button,
                    pressed: true,
                    ..
                } if Chord::of_button(*button, egui::Modifiers::NONE).is_none() => {
                    self.gave_up = self.capture.take();
                    break;
                }
                _ => {}
            }
            if let Some((chord, false)) = Chord::of_event(event, modifiers) {
                let before = keys.clone();
                self.taken = keys
                    .assign(action, slot, chord)
                    .map(|other| (lost_keys(&[(chord, other)]), before));
                self.capture = None;
                break;
            }
        }
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::Key { .. }
                        | egui::Event::Text(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Paste(_)
                )
            });
        });
    }

    /// The keys page: every action with its keys; a click on a key waits
    /// for the one to take its place, `+` for one more, the cross removes
    /// it.
    fn keys_page(&mut self, ui: &mut Ui, keys: &mut Keymap) {
        section(ui, tr!("Choosing keys", "Выбор клавиш"), |ui| {
            row(
                ui,
                tr!("Default keys", "Клавиши по умолчанию"),
                Some(tr!(
                    "A click on a key waits for another to take its place, + for one more; Esc gives up. A key another action had is taken from it.",
                    "Щелчок по клавише ждёт нажатия новой на её место, «+» — ещё одной; Esc — отмена. Клавиша другого действия переходит к этому."
                )),
                150.0,
                |ui| {
                    if ui
                        .add_enabled(
                            !keys.is_default(),
                            egui::Button::new(tr!("Reset the keys", "Сбросить клавиши")),
                        )
                        .clicked()
                    {
                        *keys = Keymap::default();
                        self.capture = None;
                        self.taken = None;
                    }
                },
            );
        });
        let mut undo = None;
        if let Some((what, before)) = &self.taken {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(what).size(12.5).color(palette::accent()));
                if ui.small_button(tr!("Undo", "Отменить")).clicked() {
                    undo = Some(before.clone());
                }
            });
        }
        if let Some(before) = undo {
            *keys = before;
            self.taken = None;
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.key_filter)
                    .hint_text(tr!("Find an action or a key", "Найти действие или клавишу"))
                    .desired_width(260.0),
            );
            if !self.key_filter.is_empty() && ui.small_button("×").clicked() {
                self.key_filter.clear();
            }
        });
        let filter = self.key_filter.trim().to_lowercase();
        let shown: Vec<Action> = Action::ALL
            .iter()
            .copied()
            .filter(|&action| {
                filter.is_empty()
                    || action.name().to_lowercase().contains(&filter)
                    || keys
                        .chords(action)
                        .iter()
                        .any(|c| c.label().to_lowercase().contains(&filter))
            })
            .collect();
        let mut any = false;
        for group in Group::ALL {
            let actions: Vec<Action> = shown
                .iter()
                .copied()
                .filter(|a| a.group() == group)
                .collect();
            if actions.is_empty() {
                continue;
            }
            any = true;
            section(ui, group.title(), |ui| {
                for (i, &action) in actions.iter().enumerate() {
                    if i > 0 {
                        divider(ui);
                    }
                    // Many or long keys take more room from the name.
                    let width = self
                        .chips_width(ui, keys, action)
                        .clamp(KEYS_WIDTH, (ui.available_width() - 120.0).max(KEYS_WIDTH));
                    row(ui, action.name(), None, width, |ui| {
                        self.key_chips(ui, keys, action);
                    });
                }
            });
        }
        if !any {
            ui.add_space(12.0);
            ui.label(
                RichText::new(tr!("Nothing found.", "Ничего не найдено."))
                    .color(palette::text_weak()),
            );
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new(tr!(
                "Not chosen here: the arrows, Page Up/Down, Home, End and Enter move in the library and the playlist and play; Delete removes from the playlist; letters typed with the pointer over the artists go to an artist; with the pointer over the title row or the player bar Left and Right seek by 5 s and Up and Down change the volume, as the mouse wheel does there; Esc leaves the mini player; the media keys play, pause, stop and change tracks. While a text field has the focus only the F-keys, Alt and Ctrl chords act. Ctrl+V and Shift+Insert cannot be chosen, and Ctrl+Insert is taken as Ctrl+C.",
                "Здесь не выбираются: стрелки, Page Up/Down, Home, End и Enter перемещают по библиотеке и плейлисту и запускают воспроизведение; Delete убирает из плейлиста; буквы, набранные с указателем над исполнителями, ведут к исполнителю; с указателем над строкой заголовка или панелью плеера Влево и Вправо перематывают на 5 с, а Вверх и Вниз меняют громкость, как там же колесо мыши; Esc закрывает мини-плеер; медиаклавиши запускают, приостанавливают, останавливают и переключают треки. Пока в текстовом поле курсор, действуют только F-клавиши и сочетания с Alt и Ctrl. Ctrl+V и Shift+Insert выбрать нельзя, а Ctrl+Insert считается сочетанием Ctrl+C."
            ))
            .size(12.5)
            .color(palette::text_weak()),
        );
        // The click that gave a choice up has ended (every key has seen
        // it): another one starts a choice again.
        if !ui.input(|i| i.pointer.any_down()) {
            self.gave_up = None;
        }
    }

    /// The width the keys of `action` take, as [`key_chip`] draws them.
    fn chips_width(&self, ui: &Ui, keys: &Keymap, action: Action) -> f32 {
        const GAP: f32 = 6.0;
        let chip = |label: String, waiting: bool| {
            let text = ui.painter().layout_no_wrap(
                label,
                FontId::proportional(13.0),
                palette::text_bright(),
            );
            text.size().x + 18.0 + if waiting { 0.0 } else { 18.0 } + GAP
        };
        // `+` and the round arrow back.
        let mut width = 26.0 + GAP + TOOL + GAP;
        if self.capture == Some((action, None)) {
            width += chip(waiting(), true);
        }
        for (slot, chord) in keys.chords(action).iter().enumerate() {
            width += if self.capture == Some((action, Some(slot))) {
                chip(waiting(), true)
            } else {
                chip(chord.label(), false)
            };
        }
        width
    }

    /// The keys of `action` in a right-to-left layout: the arrow back and
    /// `+` first, then the keys from the last.
    fn key_chips(&mut self, ui: &mut Ui, keys: &mut Keymap, action: Action) {
        ui.spacing_mut().item_spacing.x = 6.0;
        // The round arrow back to the action's own keys, at the right as
        // the equalizer's is; grey while they are.
        let defaults: Vec<String> = action.defaults().iter().map(|c| c.label()).collect();
        let tip = if defaults.is_empty() {
            tr!(
                "Back to the default: no key",
                "Вернуть по умолчанию: без клавиши"
            )
            .to_owned()
        } else {
            tr!(
                format!("Back to the default: {}", defaults.join(", ")),
                format!("Вернуть по умолчанию: {}", defaults.join(", "))
            )
        };
        let off = !keys.is_default_for(action);
        if tool(ui, &tip, off, Icon::RotateLeft).clicked() {
            let before = keys.clone();
            let taken = keys.reset(action);
            self.capture = None;
            // Its own keys besides the defaults go too: Undo brings them back.
            let what = if taken.is_empty() {
                tr!(
                    format!("“{}” has its default keys again.", action.name()),
                    format!("У «{}» снова клавиши по умолчанию.", action.name())
                )
            } else {
                lost_keys(&taken)
            };
            self.taken = Some((what, before));
        }
        let add = ui
            .add(egui::Button::new("+").min_size(vec2(26.0, KEY_HEIGHT)))
            .on_hover_text(tr!("Add a key", "Добавить клавишу"));
        if add.clicked() && self.gave_up != Some((action, None)) {
            self.capture = Some((action, None));
            self.taken = None;
        }
        if self.capture == Some((action, None)) {
            key_chip(ui, (action, usize::MAX), &waiting(), true);
        }
        let chords = keys.chords(action).to_vec();
        for (slot, chord) in chords.iter().enumerate().rev() {
            let capturing = self.capture == Some((action, Some(slot)));
            let label = if capturing { waiting() } else { chord.label() };
            match key_chip(ui, (action, slot), &label, capturing) {
                Some(Chip::Change) if self.gave_up != Some((action, Some(slot))) => {
                    self.capture = Some((action, Some(slot)));
                    self.taken = None;
                }
                Some(Chip::Remove) => {
                    keys.remove(action, slot);
                    self.capture = None;
                    self.taken = None;
                }
                _ => {}
            }
        }
    }
}

/// The least width of the keys of a row.
const KEYS_WIDTH: f32 = 260.0;
/// The height of a key.
const KEY_HEIGHT: f32 = 24.0;

/// What a key on the keys page was clicked for.
enum Chip {
    Change,
    Remove,
}

/// The label of a key waiting for a press.
fn waiting() -> String {
    tr!("Press a key…", "Нажмите клавишу…").to_owned()
}

/// What the actions that lost their keys no longer do, a sentence each.
fn lost_keys(taken: &[(Chord, Action)]) -> String {
    taken
        .iter()
        .map(|(chord, other)| {
            tr!(
                format!("{} no longer does “{}”.", chord.label(), other.name()),
                format!("{} больше не делает «{}».", chord.label(), other.name())
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A key on a keycap ground with a cross that removes it; while it waits
/// for a press, in the accent colour without the cross.
fn key_chip(ui: &mut Ui, id: (Action, usize), label: &str, waiting: bool) -> Option<Chip> {
    const CROSS: f32 = 18.0;
    let text_colour = if waiting {
        palette::on_accent()
    } else {
        palette::text_bright()
    };
    let text =
        ui.painter()
            .layout_no_wrap(label.to_owned(), FontId::proportional(13.0), text_colour);
    let cross_width = if waiting { 0.0 } else { CROSS };
    let size = vec2(text.size().x + 18.0 + cross_width, KEY_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let key_rect = Rect::from_min_max(rect.min, pos2(rect.right() - cross_width, rect.bottom()));
    let id = ui.id().with(("key_chip", id));
    let key = ui
        .interact(key_rect, id, Sense::CLICK)
        .on_hover_text(if waiting {
            tr!("Esc or a click gives up", "Esc или щелчок — отмена")
        } else {
            tr!("Choose another key", "Выбрать другую клавишу")
        });
    let cross = (!waiting).then(|| {
        let r = Rect::from_min_max(pos2(key_rect.right(), rect.top()), rect.max);
        ui.interact(r, id.with("remove"), Sense::CLICK)
            .on_hover_text(tr!("Remove the key", "Убрать клавишу"))
    });
    let hovered = key.hovered() || cross.as_ref().is_some_and(Response::hovered);
    let fill = if waiting {
        palette::accent()
    } else if hovered {
        palette::hover()
    } else {
        palette::panel_bg()
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        5.0,
        fill,
        Stroke::new(1.0, palette::line()),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        pos2(rect.left() + 9.0, rect.center().y - text.size().y / 2.0),
        text,
        text_colour,
    );
    if let Some(cross) = &cross {
        let c = if cross.hovered() {
            palette::text_bright()
        } else {
            palette::text_weak()
        };
        let o = pos2(cross.rect.center().x - 2.0, cross.rect.center().y);
        let d = 3.5;
        let stroke = Stroke::new(1.4, c);
        painter.line_segment([o + vec2(-d, -d), o + vec2(d, d)], stroke);
        painter.line_segment([o + vec2(-d, d), o + vec2(d, -d)], stroke);
    }
    if cross.is_some_and(|c| c.clicked()) {
        Some(Chip::Remove)
    } else if key.clicked() {
        Some(Chip::Change)
    } else {
        None
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
pub(crate) fn divider(ui: &mut Ui) {
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
            |ui| accents(ui, &mut settings.accent, settings.theme),
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
            tr!("Window", "Окно"),
            Some(tr!(
                "With the library: the player bar, the library and the playlist beside it. Player and playlist: the player bar over the playlist alone, as in the early Winamp. Each keeps its own size and place.",
                "С библиотекой: панель плеера, библиотека и плейлист рядом. Плеер и плейлист: панель плеера над одним плейлистом, как в раннем Winamp. У каждого вида свои размер и место окна."
            )),
            CONTROL,
            |ui| {
                egui::ComboBox::from_id_salt("settings_window_layout")
                    .selected_text(settings.window_layout.label())
                    .width(CONTROL - 10.0)
                    .show_ui(ui, |ui| {
                        for l in WindowLayout::ALL {
                            ui.selectable_value(&mut settings.window_layout, l, l.label());
                        }
                    });
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
        divider(ui);
        row(
            ui,
            tr!("Visualization", "Визуализация"),
            Some(tr!(
                "On the right of the player bar with a visualization; each window layout (with the library, the player and playlist) keeps its own; a click on it goes on to the next one (unless switched off below), the right button lists them, \"No visualization\" (the program's name) among them. Switched off, it shows the program's name (the narrow player's room stays dark), takes no clicks and no time.",
                "Справа на панели плеера с визуализацией; у каждого вида окна (с библиотекой, плеер и плейлист) своя; щелчок по ней переключает на следующую (если не выключено ниже), правая кнопка показывает список, в том числе «Без визуализации» (название программы). Выключенная здесь, она показывает название программы (в узком плеере место остаётся тёмным), не отзывается на щелчки и не тратит время."
            )),
            CONTROL,
            |ui| {
                switch(ui, &mut settings.visual_enabled);
                let on = settings.visual_enabled;
                ui.add_enabled_ui(on, |ui| {
                    egui::ComboBox::from_id_salt("settings_visual")
                        .selected_text(settings.visual().label())
                        .width(CONTROL - 70.0)
                        .show_ui(ui, |ui| {
                            for v in Visual::ALL {
                                if v.starts_group() {
                                    ui.separator();
                                }
                                ui.selectable_value(settings.visual_mut(), v, v.label());
                            }
                        });
                });
            },
        );
        divider(ui);
        row(
            ui,
            tr!(
                "A click switches the visualization",
                "Щелчок переключает визуализацию"
            ),
            Some(tr!(
                "A click on the visualization goes on to the next one. Off, it is chosen from the list of the right button or here.",
                "Щелчок по визуализации переключает на следующую. Если выключено, она выбирается из списка по правой кнопке или здесь."
            )),
            CONTROL,
            |ui| {
                ui.add_enabled_ui(settings.visual_enabled, |ui| {
                    switch(ui, &mut settings.visual_click);
                });
            },
        );
        divider(ui);
        row(
            ui,
            tr!("Visualization's colours", "Цвета визуализации"),
            Some(tr!(
                "Apart from the theme: each visualization's own colours, chosen for it, the accent's shades, a rainbow across the frequencies, Winamp's green to red or fire by the level, the colours of the cover of the track playing across the frequencies (the accent's shades for a track without one), or one's own colour's shades.",
                "Отдельно от темы: свои цвета у каждой визуализации, подобранные для неё, оттенки акцента, радуга по частотам, зелёный-жёлтый-красный как в Winamp или огонь по уровню, цвета обложки играющего трека по частотам (для трека без обложки — оттенки акцента), либо оттенки своего цвета."
            )),
            ACCENTS_WIDTH,
            |ui| {
                let look = settings.look();
                ui.add_enabled_ui(settings.visual_enabled, |ui| {
                    let visual = settings.visual();
                    visual_colours(ui, settings.visual_colours_mut(), look, visual);
                });
            },
        );
    });
    section(ui, tr!("Covers", "Обложки"), |ui| {
        row(
            ui,
            tr!("Source priority", "Приоритет источника"),
            Some(tr!(
                "For the library and player. If the preferred source has no readable cover, the other is used. The tag editor always shows the file's embedded cover first.",
                "В библиотеке и плеере. Если в выбранном источнике нет читаемой обложки, используется другой. Редактор тегов всегда сначала показывает встроенную обложку файла."
            )),
            CONTROL,
            |ui| {
                egui::ComboBox::from_id_salt("settings_cover_priority")
                    .selected_text(settings.cover_priority.label())
                    .width(CONTROL - 10.0)
                    .show_ui(ui, |ui| {
                        for priority in CoverPriority::ALL {
                            ui.selectable_value(
                                &mut settings.cover_priority,
                                priority,
                                priority.label(),
                            );
                        }
                    });
            },
        );
    });
    section(ui, tr!("Playlist", "Плейлист"), |ui| {
        playlist_rows(ui, settings)
    });
    section(ui, tr!("Playback", "Воспроизведение"), |ui| {
        row(
            ui,
            tr!("Seek step", "Шаг перемотки"),
            Some(tr!(
                "How far Shift+Left / Shift+Right, and Left / Right with the pointer over the player bar, seek.",
                "На сколько перематывают Shift+Влево / Shift+Вправо и Влево / Вправо с указателем над панелью плеера."
            )),
            CONTROL,
            |ui| {
                ui.spacing_mut().slider_width = CONTROL - 70.0;
                settings.seek_step = settings.seek_step.clamp(1, crate::settings::MAX_SEEK_STEP);
                ui.add(
                    egui::Slider::new(&mut settings.seek_step, 1..=crate::settings::MAX_SEEK_STEP)
                        .suffix(tr!(" s", " с")),
                );
            },
        );
        row(
            ui,
            tr!("Volume step", "Шаг громкости"),
            Some(tr!(
                "How far Ctrl+Up / Ctrl+Down, Up / Down with the pointer over the player bar and a wheel notch over it change the volume.",
                "На сколько меняют громкость Ctrl+Вверх / Ctrl+Вниз, Вверх / Вниз с указателем над панелью плеера и щелчок колеса над ней."
            )),
            CONTROL,
            |ui| {
                ui.spacing_mut().slider_width = CONTROL - 70.0;
                settings.volume_step = settings
                    .volume_step
                    .clamp(1, crate::settings::MAX_VOLUME_STEP);
                ui.add(
                    egui::Slider::new(
                        &mut settings.volume_step,
                        1..=crate::settings::MAX_VOLUME_STEP,
                    )
                    .suffix(" %"),
                );
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
        divider(ui);
        row(
            ui,
            tr!("Minimize to the tray", "Сворачивать в трей"),
            Some(tr!(
                "An icon in the notification area: minimized, the window leaves the taskbar for it; a click on it brings the window back, its menu controls playing.",
                "Значок в области уведомлений: свёрнутое окно уходит с панели задач в него; щелчок по значку возвращает окно, его меню управляет воспроизведением."
            )),
            50.0,
            |ui| switch(ui, &mut settings.tray),
        );
        divider(ui);
        row(
            ui,
            tr!("Only in the tray", "Только в трее"),
            Some(tr!(
                "The program has no button on the taskbar, shown or minimized: its icon in the notification area brings the window back.",
                "У программы нет кнопки на панели задач ни у открытого, ни у свёрнутого окна: окно возвращает значок в области уведомлений."
            )),
            50.0,
            |ui| switch(ui, &mut settings.tray_only),
        );
    });
    section(ui, tr!("Track notices", "Уведомления о треках"), |ui| {
        row(
            ui,
            tr!("Show", "Показывать"),
            Some(tr!(
                "A small window by the notification area names each track as it starts, with its cover, for a few seconds; the pointer over it keeps it, a click brings the player back. Not shown over a full-screen program or a presentation.",
                "Небольшое окно у области уведомлений на несколько секунд называет каждый начавшийся трек и показывает его обложку; указатель над ним удерживает его, щелчок возвращает окно плеера. Не показывается поверх полноэкранной программы и во время презентации."
            )),
            CONTROL,
            |ui| {
                egui::ComboBox::from_id_salt("settings_track_notice")
                    .selected_text(settings.track_notice.label())
                    .width(CONTROL - 10.0)
                    .show_ui(ui, |ui| {
                        for n in TrackNotice::ALL {
                            ui.selectable_value(&mut settings.track_notice, n, n.label());
                        }
                    });
            },
        );
        divider(ui);
        row(
            ui,
            tr!("For", "В течение"),
            Some(tr!(
                "How long a notice stays before it fades.",
                "Сколько уведомление держится, прежде чем погаснуть."
            )),
            CONTROL,
            |ui| {
                ui.add_enabled_ui(settings.track_notice != TrackNotice::Off, |ui| {
                    ui.spacing_mut().slider_width = CONTROL - 70.0;
                    settings.track_notice_seconds = settings
                        .track_notice_seconds
                        .clamp(1, crate::settings::MAX_NOTICE_SECONDS);
                    ui.add(
                        egui::Slider::new(
                            &mut settings.track_notice_seconds,
                            1..=crate::settings::MAX_NOTICE_SECONDS,
                        )
                        .suffix(tr!(" s", " с")),
                    );
                });
            },
        );
    });
    section(ui, tr!("Lyrics", "Тексты песен"), |ui| {
        row(
            ui,
            tr!("Look on the internet", "Искать в интернете"),
            Some(tr!(
                "Lyrics that neither the file's tag nor an .lrc file beside it has are looked for on LRCLIB (lrclib.net), synced ones first: the artist, the title, the album and the length of the track playing are sent there. What it answers is kept, so a track is asked for once. The lyrics show under the playlist, by the microphone's button.",
                "Текст, которого нет ни в теге файла, ни в файле .lrc рядом с ним, ищется в LRCLIB (lrclib.net), в первую очередь синхронизированный: туда отправляются исполнитель, название, альбом и длительность играющего трека. Ответ сохраняется, так что о каждом треке сервис спрашивается один раз. Текст показывается под плейлистом, по кнопке с микрофоном."
            )),
            50.0,
            |ui| switch(ui, &mut settings.lyrics_online),
        );
        divider(ui);
        row(
            ui,
            tr!("Source priority", "Приоритет источника"),
            Some(tr!(
                "Which lyrics are shown when both the file (its tag or an .lrc file) and LRCLIB have them; the other is shown when the preferred one has none. A choice made for a track in the lyrics' menu goes first.",
                "Какой текст показывается, если он есть и в файле (в теге или файле .lrc), и в LRCLIB; если в выбранном источнике текста нет, показывается другой. Выбор, сделанный для трека в меню текста, важнее."
            )),
            CONTROL,
            |ui| {
                ui.add_enabled_ui(settings.lyrics_online, |ui| {
                    egui::ComboBox::from_id_salt("settings_lyrics_priority")
                        .selected_text(settings.lyrics_priority.label())
                        .width(CONTROL - 10.0)
                        .show_ui(ui, |ui| {
                            for p in crate::settings::LyricsPriority::ALL {
                                ui.selectable_value(&mut settings.lyrics_priority, p, p.label());
                            }
                        });
                });
            },
        );
    });
    section(ui, tr!("Tags", "Теги"), |ui| tag_rows(ui, settings));
}

/// How the file info writes an MP3's tags: the ID3v2 version, its ID3v1
/// and APE tags.
fn tag_rows(ui: &mut Ui, settings: &mut Settings) {
    row(
        ui,
        tr!("ID3v2 version of MP3", "Версия ID3v2 в MP3"),
        Some(tr!(
            "The version an MP3's tag is written in when its tags are edited. 2.3 is read by every program and device, Windows Explorer and car stereos among them; 2.4 keeps text in UTF-8 and full dates, but some devices do not read it. \"As in the file\" keeps a file's version and gives a file without a tag the one named.",
            "Версия, в которой пишется тег MP3 при правке тегов. 2.3 читают все программы и устройства, в том числе Проводник Windows и магнитолы; 2.4 хранит текст в UTF-8 и полные даты, но часть устройств его не читает. «Как в файле» сохраняет версию файла, а файлу без тега даёт указанную."
        )),
        CONTROL,
        |ui| {
            egui::ComboBox::from_id_salt("settings_id3v2")
                .selected_text(settings.id3v2_version.label())
                .width(CONTROL - 10.0)
                .show_ui(ui, |ui| {
                    for v in Id3Version::ALL {
                        ui.selectable_value(&mut settings.id3v2_version, v, v.label());
                    }
                });
        },
    );
    divider(ui);
    row(
        ui,
        tr!("ID3v1 of MP3", "ID3v1 в MP3"),
        Some(tr!(
            "The short tag at an MP3's end that old players and car stereos read (30 letters a field, in the code page below). Left as it was, it keeps the values from before an edit.",
            "Короткий тег в конце MP3, который читают старые плееры и магнитолы (30 букв в поле, в кодировке ниже). Если его не трогать, в нём останутся значения до правки."
        )),
        CONTROL,
        |ui| {
            egui::ComboBox::from_id_salt("settings_id3v1")
                .selected_text(settings.id3v1.label())
                .width(CONTROL - 10.0)
                .show_ui(ui, |ui| {
                    for v in Id3v1Tag::ALL {
                        ui.selectable_value(&mut settings.id3v1, v, v.label());
                    }
                });
        },
    );
    divider(ui);
    row(
        ui,
        tr!("ID3v1 code page", "Кодировка ID3v1"),
        Some(tr!(
            "ID3v1 keeps bytes, not letters: they are read and written in this code page. Windows' own is the one Winamp, the old taggers and car stereos use; another is for files made on a Windows of another language. Changed, the library reads again the MP3s with letters past Latin ones.",
            "ID3v1 хранит байты, а не буквы: они читаются и пишутся в этой кодировке. Кодировку Windows используют Winamp, старые редакторы тегов и магнитолы; другая нужна для файлов, сделанных на Windows другого языка. При смене библиотека перечитывает MP3, в тегах которых есть буквы помимо латинских."
        )),
        CONTROL,
        |ui| {
            egui::ComboBox::from_id_salt("settings_id3v1_encoding")
                .selected_text(settings.id3v1_encoding.label())
                .width(CONTROL - 10.0)
                .show_ui(ui, |ui| {
                    for v in Id3v1Encoding::ALL {
                        ui.selectable_value(&mut settings.id3v1_encoding, v, v.label());
                    }
                });
        },
    );
    divider(ui);
    row(
        ui,
        tr!("APE of MP3", "APE в MP3"),
        Some(tr!(
            "The tag foobar2000 and MP3Gain leave at an MP3's end. Updated, it gets the fields changed and keeps the rest (MP3Gain's ReplayGain among it); removed, that goes too.",
            "Тег, который оставляют в конце MP3 foobar2000 и MP3Gain. При обновлении в него пишутся изменённые поля, а остальное (в том числе ReplayGain от MP3Gain) сохраняется; при удалении пропадает и оно."
        )),
        CONTROL,
        |ui| {
            egui::ComboBox::from_id_salt("settings_ape")
                .selected_text(settings.ape.label())
                .width(CONTROL - 10.0)
                .show_ui(ui, |ui| {
                    for v in ApeTag::ALL {
                        ui.selectable_value(&mut settings.ape, v, v.label());
                    }
                });
        },
    );
}

fn sound(
    ui: &mut Ui,
    settings: &mut Settings,
    devices: &[Device],
    naming: &mut (Option<String>, Option<String>),
) {
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
    processing(
        ui,
        &mut settings.dsp,
        &mut settings.eq_presets,
        &mut naming.0,
        (&mut settings.enhancer_presets, &mut naming.1),
        (
            &mut settings.crossfade,
            &mut settings.crossfade_in_albums,
            &mut settings.crossfade_manual,
            &mut settings.crossfade_seek,
        ),
    );
}

/// The loudness, the equalizer and the enhancer.
fn processing(
    ui: &mut Ui,
    dsp: &mut DspSettings,
    presets: &mut Vec<EqPreset>,
    naming: &mut Option<String>,
    (enhancer_presets, enhancer_naming): (&mut Vec<EnhancerPreset>, &mut Option<String>),
    crossfade: (&mut u8, &mut bool, &mut f32, &mut bool),
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
        divider(ui);
        row(
            ui,
            tr!("Limiter", "Лимитер"),
            Some(limiter_hint()),
            50.0,
            |ui| switch(ui, dsp.limiter_mut()),
        );
    });
    section(ui, tr!("Channels", "Каналы"), |ui| {
        row(
            ui,
            tr!("Balance", "Баланс"),
            Some(tr!(
                "Turns one side down: towards the left, the right is quieter, and the other way.",
                "Приглушает одну сторону: влево — тише правый канал, вправо — левый."
            )),
            CONTROL,
            |ui| balance(ui, &mut dsp.balance),
        );
        divider(ui);
        row(
            ui,
            tr!("Mono", "Моно"),
            Some(tr!(
                "Both channels carry the same sound, the two mixed: for one earphone, or speakers far apart.",
                "Оба канала звучат одинаково, смесью двух: для одного наушника или далеко разнесённых колонок."
            )),
            50.0,
            |ui| switch(ui, &mut dsp.mono),
        );
    });
    section(
        ui,
        tr!("Between tracks", "Между треками"),
        |ui| {
            row(
                ui,
                tr!("Crossfade", "Кроссфейд"),
                Some(tr!(
                    "The end of a track fades into the start of the next, this many seconds; 0: one after the other, without a gap.",
                    "Конец трека плавно переходит в начало следующего за столько секунд; 0 — один за другим, без паузы."
                )),
                CONTROL,
                |ui| {
                    ui.spacing_mut().slider_width = CONTROL - 70.0;
                    ui.add(
                        egui::Slider::new(crossfade.0, 0..=crate::player::MAX_CROSSFADE)
                            .suffix(tr!(" s", " с")),
                    );
                },
            );
            divider(ui);
            row(
                ui,
                tr!("Also within an album", "И внутри альбома"),
                Some(tr!(
                    "Off: the tracks of one folder play one after the other without a gap, as an album is meant to.",
                    "Выключено — треки одной папки играют подряд без паузы, как задуман альбом."
                )),
                50.0,
                |ui| ui.add_enabled_ui(*crossfade.0 > 0, |ui| switch(ui, crossfade.1)),
            );
            divider(ui);
            row(
                ui,
                tr!("On a track chosen", "При выборе трека"),
                Some(tr!(
                    "A track chosen while another plays (Next, Previous, a double click) fades in as the other fades out, this many seconds; 0: the other fades out at once.",
                    "Трек, выбранный во время игры другого (Следующий, Предыдущий, двойной щелчок), плавно вступает, пока тот затихает, за столько секунд; 0 — прежний трек сразу затихает."
                )),
                CONTROL,
                |ui| {
                    ui.spacing_mut().slider_width = CONTROL - 70.0;
                    ui.add(
                        egui::Slider::new(crossfade.2, 0.0..=crate::player::MAX_MANUAL_CROSSFADE)
                            .step_by(0.1)
                            .fixed_decimals(1)
                            .suffix(tr!(" s", " с")),
                    );
                },
            );
            divider(ui);
            row(
                ui,
                tr!("On a seek", "При перемотке"),
                Some(tr!(
                    "On: after a seek the old place and the new one sound together for 0.3 s, the one fading out as the other fades in. Off: the old place fades out in 30 ms, then the new one fades in in 30 ms, a short dip without overlap. Neither clicks.",
                    "Включено — после перемотки старое и новое место 0,3 с звучат вместе: одно затихает, другое нарастает. Выключено — старое место затихает за 30 мс, затем новое нарастает за 30 мс: короткий провал без наложения. Щелчка нет в обоих случаях."
                )),
                50.0,
                |ui| switch(ui, crossfade.3),
            );
        },
    );
    section(ui, tr!("Equalizer", "Эквалайзер"), |ui| {
        ui.add_space(8.0);
        equalizer(ui, dsp, presets, naming, None);
    });
    section(ui, tr!("Enhancer", "Улучшение звука"), |ui| {
        ui.add_space(8.0);
        enhancer(ui, dsp, enhancer_presets, enhancer_naming, None);
    });
}

/// What the enhancer's switch does: the Sound page's row and the tip of
/// the switch in the equalizer's window.
fn enhancer_hint() -> &'static str {
    tr!(
        "Raises quiet sound band by band and gives it character: a multiband automatic gain control. The Loud preset is the default. The limiter works with it.",
        "Поднимает тихий звук по полосам и придаёт ему характер: многополосная автоматическая регулировка усиления. По умолчанию выбран пресет «Громкий». Вместе с ним работает лимитер."
    )
}

/// One of the enhancer's controls: its name, what it does, and its value
/// as a number with the slider's range and step, read from and written to
/// the settings, and shown.
struct Control {
    title: &'static str,
    hint: &'static str,
    range: (f32, f32),
    step: f32,
    /// The marks under the slider: one every so much, longer at the ends
    /// and at the values listed (`slider_ticks`).
    ticks: (f32, &'static [f32]),
    get: fn(&Enhancement) -> f32,
    set: fn(&mut Enhancement, f32),
    text: fn(f32) -> String,
}

fn level_text(v: f32) -> String {
    format!("{v:+.1}{}", tr!(" dB", " дБ"))
}

fn amount_text(v: f32) -> String {
    format!("{v:.1}")
}

/// The controls in rows, as the window lays them out: how much and of
/// what, the character of the bass and the treble, the levels.
fn enhancer_controls() -> [Vec<Control>; 3] {
    [
        vec![
            Control {
                title: tr!("Drive", "Драйв"),
                hint: tr!(
                    "How much of all the rest: how far quiet sound is raised and how much of the character below is heard. 0: only the levels.",
                    "Сколько всего остального: насколько поднимается тихий звук и сколько слышно характера ниже. 0 — только уровни."
                ),
                range: (0.0, f32::from(Enhancement::MOST_DRIVE)),
                step: 1.0,
                ticks: (10.0, &[50.0, 100.0]),
                get: |e| f32::from(e.drive),
                set: |e, v| e.drive = v.round() as u8,
                text: |v| format!("{v:.0} %"),
            },
            Control {
                title: tr!("Fidelity", "Чёткость"),
                hint: tr!(
                    "Raises quiet sound across the range, at 10 up to about 20 dB, less the louder it is.",
                    "Поднимает тихий звук во всём диапазоне, на 10 примерно до 20 дБ, тем меньше, чем он громче."
                ),
                range: (0.0, 10.0),
                step: 0.1,
                ticks: (1.0, &[5.0]),
                get: |e| e.fidelity,
                set: |e, v| e.fidelity = v,
                text: amount_text,
            },
            Control {
                title: tr!("Ambience", "Объём"),
                hint: tr!(
                    "As fidelity, mostly from 160 Hz to 10 kHz and a little unevenly across it, which gives the room. Together the two raise less than each would add.",
                    "Как чёткость, но в основном от 160 Гц до 10 кГц и немного неровно по частотам, что даёт ощущение пространства. Вместе они поднимают меньше, чем сумма каждой."
                ),
                range: (0.0, 10.0),
                step: 0.1,
                ticks: (1.0, &[5.0]),
                get: |e| e.ambience,
                set: |e, v| e.ambience = v,
                text: amount_text,
            },
        ],
        vec![
            Control {
                title: tr!("Bass type", "Тип баса"),
                hint: tr!(
                    "Deep: the low bass raised far more, the quieter it is the more. Punch: the low middle (250 Hz to 1 kHz) raised instead.",
                    "Глубокий — низкий бас поднимается гораздо сильнее, и тем больше, чем он тише. Плотный — вместо этого поднимается нижняя середина (250 Гц – 1 кГц)."
                ),
                range: (-100.0, 100.0),
                step: 1.0,
                ticks: (20.0, &[0.0]),
                get: |e| f32::from(e.bass_type),
                set: |e, v| e.bass_type = v.round() as i8,
                text: |v| match v.round() as i32 {
                    0 => tr!("Neutral", "Нейтральный").to_owned(),
                    v if v < 0 => format!("{} {}", tr!("Deep", "Глубокий"), -v),
                    v => format!("{} {v}", tr!("Punch", "Плотный")),
                },
            },
            Control {
                title: tr!("Treble type", "Тип верха"),
                hint: tr!(
                    "Sharp: 5 to 12 kHz raised. Smooth: the whole top from 1.6 kHz raised and the middle a little lower.",
                    "Резкий — поднимается 5–12 кГц. Мягкий — поднимается весь верх от 1,6 кГц, а середина чуть ниже."
                ),
                range: (-100.0, 100.0),
                step: 1.0,
                ticks: (20.0, &[0.0]),
                get: |e| f32::from(e.treble_type),
                set: |e, v| e.treble_type = v.round() as i8,
                text: |v| match v.round() as i32 {
                    0 => tr!("Neutral", "Нейтральный").to_owned(),
                    v if v < 0 => format!("{} {}", tr!("Sharp", "Резкий"), -v),
                    v => format!("{} {v}", tr!("Smooth", "Мягкий")),
                },
            },
        ],
        vec![
            Control {
                title: tr!("Bass", "Бас"),
                hint: tr!(
                    "A fixed level after the rest: under 150 Hz.",
                    "Постоянный уровень после всего остального: ниже 150 Гц."
                ),
                range: (-Enhancement::LEVEL_DB, Enhancement::LEVEL_DB),
                step: 0.1,
                ticks: (1.0, &[0.0]),
                get: |e| e.bass,
                set: |e, v| e.bass = v,
                text: level_text,
            },
            Control {
                title: tr!("Mid", "Середина"),
                hint: tr!(
                    "A fixed level after the rest: 300 Hz to 1 kHz, and half of it from 1.6 to 5 kHz.",
                    "Постоянный уровень после всего остального: 300 Гц – 1 кГц и наполовину 1,6–5 кГц."
                ),
                range: (-Enhancement::LEVEL_DB, Enhancement::LEVEL_DB),
                step: 0.1,
                ticks: (1.0, &[0.0]),
                get: |e| e.mid,
                set: |e, v| e.mid = v,
                text: level_text,
            },
            Control {
                title: tr!("Treble", "Верх"),
                hint: tr!(
                    "A fixed level after the rest: from 10 kHz, and half of it from 1.6 to 5 kHz.",
                    "Постоянный уровень после всего остального: от 10 кГц и наполовину 1,6–5 кГц."
                ),
                range: (-Enhancement::LEVEL_DB, Enhancement::LEVEL_DB),
                step: 0.1,
                ticks: (1.0, &[0.0]),
                get: |e| e.treble,
                set: |e, v| e.treble = v,
                text: level_text,
            },
        ],
    ]
}

/// The gap between the columns of the enhancer's grid, and between its
/// rows.
const ENHANCER_GAP: f32 = 28.0;
const ENHANCER_ROW_GAP: f32 = 14.0;

/// The enhancer: its head (`enhancer_head`) and its controls in rows
/// (`enhancer_grid`), greyed while it is off. With `open` (the sound
/// tuning window), a section that folds as the equalizer's over it, kept
/// in the settings: a click on its name or its chevron folds it.
pub(crate) fn enhancer(
    ui: &mut Ui,
    dsp: &mut DspSettings,
    presets: &mut Vec<EnhancerPreset>,
    naming: &mut Option<String>,
    open: Option<&mut bool>,
) {
    let Some(open) = open else {
        enhancer_head(ui, dsp, presets, naming, None);
        ui.add_space(12.0);
        let base = enhancer_base(ui, &dsp.enhancement, presets);
        enhancer_grid(ui, &mut dsp.enhancement, base, dsp.enhancer);
        return;
    };
    let mut fold = fold_of(ui, "eq_enhancer", *open);
    let openness = fold.openness(ui.ctx());
    if enhancer_head(ui, dsp, presets, naming, Some(openness)) {
        fold.toggle(ui);
    }
    *open = fold.is_open();
    let base = enhancer_base(ui, &dsp.enhancement, presets);
    let on = dsp.enhancer;
    fold.show_body_unindented(ui, |ui| {
        ui.add_space(12.0);
        enhancer_grid(ui, &mut dsp.enhancement, base, on);
    });
}

/// Where the preset last put at (chosen, or saved as one's own) is kept,
/// shared by the settings window and the sound tuning window.
fn enhancer_base_id() -> egui::Id {
    egui::Id::new("enhancer_base")
}

/// The settings the controls are told against (the value in the accent
/// when it differs, a double click back to it): the preset they stand at,
/// else the one last put at, else Loud.
fn enhancer_base(ui: &Ui, e: &Enhancement, presets: &[EnhancerPreset]) -> Enhancement {
    let standing = enhancer::PRESETS
        .iter()
        .map(|p| p.settings)
        .chain(presets.iter().map(|p| p.settings))
        .find(|s| s == e);
    if let Some(s) = standing {
        ui.data_mut(|d| d.insert_temp(enhancer_base_id(), s));
        return s;
    }
    ui.data(|d| d.get_temp::<Enhancement>(enhancer_base_id()))
        .unwrap_or(Enhancement::LOUD)
}

/// The enhancer's switch, the built-in presets and the user's own in one
/// list (the one the controls stand at named; "Custom" otherwise), and
/// beside it saving the controls as one's own and deleting one's own, the
/// name typed in a row under it: the equalizer's head (`eq_head`), so the
/// two switches, lists and tools stand in columns. With `fold` (the
/// section's openness, in the sound tuning window), the name follows the
/// switch and the fold's chevron the tools; whether the name or the
/// chevron was clicked.
fn enhancer_head(
    ui: &mut Ui,
    dsp: &mut DspSettings,
    presets: &mut Vec<EnhancerPreset>,
    naming: &mut Option<String>,
    fold: Option<f32>,
) -> bool {
    let mut toggled = false;
    let e = dsp.enhancement;
    // The one of mine chosen last while the controls still stand at it,
    // else the first that does: two of the same values are two presets.
    let same = |p: &EnhancerPreset| p.settings == e;
    let chosen_id = egui::Id::new("enhancer_mine");
    let mine = ui
        .data(|d| d.get_temp::<usize>(chosen_id))
        .filter(|&i| presets.get(i).is_some_and(same))
        .or_else(|| presets.iter().position(same));
    let built_in = enhancer::PRESETS.iter().find(|p| p.settings == e);
    let mut chosen: Option<(Enhancement, Option<usize>)> = None;
    ui.horizontal(|ui| {
        switch(ui, &mut dsp.enhancer).on_hover_text(enhancer_hint());
        ui.add_space(4.0);
        if fold.is_some() {
            toggled |= fold_title(ui, tr!("Enhancer", "Улучшение звука")).clicked();
            ui.add_space(8.0);
        }
        let name = match (mine, built_in) {
            (Some(i), _) => presets[i].name.clone(),
            (None, Some(p)) => tr!(p.name, p.ru).to_owned(),
            (None, None) => tr!("Custom", "Другой").to_owned(),
        };
        let tools = if fold.is_some() { 3.0 } else { 2.0 };
        let width =
            ui.available_width() - tools * TOOL - (tools + 1.0) * ui.spacing().item_spacing.x;
        egui::ComboBox::from_id_salt("enhancer_preset")
            .selected_text(name)
            .width(width)
            .show_ui(ui, |ui| {
                for preset in enhancer::PRESETS {
                    let on = mine.is_none() && preset.settings == e;
                    if ui
                        .selectable_label(on, tr!(preset.name, preset.ru))
                        .clicked()
                    {
                        chosen = Some((preset.settings, None));
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
                            chosen = Some((p.settings.clamped(), Some(i)));
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
        if let Some(openness) = fold {
            toggled |= fold_button(ui, openness).clicked();
        }
    });
    if let Some((settings, i)) = chosen {
        dsp.enhancement = settings;
        dsp.enhancer = true;
        ui.data_mut(|d| {
            d.insert_temp(enhancer_base_id(), settings);
            if let Some(i) = i {
                d.insert_temp(chosen_id, i);
            } else {
                d.remove::<usize>(chosen_id);
            }
        });
    }
    // A name in any case is the same name ("Rock" is "rock").
    let same_name = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    let full = presets.len() >= MAX_OWN_PRESETS;
    let names: Vec<String> = presets.iter().map(|p| p.name.clone()).collect();
    if let Some(name) = name_row(
        ui,
        naming,
        tr!("My enhancer", "Моё улучшение"),
        |n| full && !names.iter().any(|o| same_name(o, n)),
    ) {
        let name: String = name.chars().take(MAX_NAME).collect();
        let settings = dsp.enhancement;
        let at = match presets.iter().position(|p| same_name(&p.name, &name)) {
            Some(i) => {
                presets[i] = EnhancerPreset { name, settings };
                i
            }
            None => {
                // A list made longer by hand keeps only the newest.
                while presets.len() >= MAX_OWN_PRESETS {
                    presets.remove(0);
                }
                presets.push(EnhancerPreset { name, settings });
                presets.len() - 1
            }
        };
        ui.data_mut(|d| {
            d.insert_temp(chosen_id, at);
            d.insert_temp(enhancer_base_id(), settings);
        });
    }
    toggled
}

/// The controls in their rows, each told against `base`, greyed while the
/// enhancer is off.
fn enhancer_grid(ui: &mut Ui, e: &mut Enhancement, base: Enhancement, on: bool) {
    ui.add_enabled_ui(on, |ui| {
        let width = ui.available_width();
        let rows = enhancer_controls();
        let last = rows.len() - 1;
        for (r, controls) in rows.into_iter().enumerate() {
            let n = controls.len() as f32;
            let cell = ((width - ENHANCER_GAP * (n - 1.0)) / n).floor();
            // Top-aligned: a centred row put the second cell lower.
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = ENHANCER_GAP;
                for control in &controls {
                    ui.allocate_ui_with_layout(
                        vec2(cell, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(cell);
                            enhancer_cell(ui, control, e, base, on);
                        },
                    );
                }
            });
            if r < last {
                ui.add_space(ENHANCER_ROW_GAP);
            }
        }
    });
}

/// One control in the enhancer's grid: its name (the hint as its tip) and
/// its value (in the accent when not `base`'s) over a slider of the
/// cell's width; a double click on the slider puts `base`'s value back.
fn enhancer_cell(ui: &mut Ui, c: &Control, e: &mut Enhancement, base: Enhancement, on: bool) {
    let width = ui.available_width();
    let standard = (c.get)(&base);
    let tip = format!(
        "{}\n\n{}",
        c.hint,
        tr!(
            "A double click puts the preset's value back.",
            "Двойной щелчок возвращает значение пресета."
        )
    );
    let mut value = (c.get)(e);
    ui.horizontal(|ui| {
        ui.label(RichText::new(c.title).size(13.5).color(palette::text()))
            .on_hover_text(&tip);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let color = if value != standard && on {
                palette::accent_hover()
            } else {
                palette::text_weak()
            };
            ui.add(
                egui::Label::new(
                    RichText::new((c.text)(value))
                        .size(13.0)
                        .strong()
                        .color(color),
                )
                .truncate(),
            );
        });
    });
    ui.add_space(4.0);
    ui.spacing_mut().slider_width = width;
    // The marks' place, under the handle drawn after it.
    let marks = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .add(
            egui::Slider::new(&mut value, c.range.0..=c.range.1)
                .step_by(f64::from(c.step))
                .show_value(false),
        )
        .on_hover_text(&tip);
    slider_ticks(ui, marks, response.rect, c);
    // Dragged near a long mark, the handle stands on it; the keys step on
    // past it.
    if response.dragged() || response.is_pointer_button_down_on() {
        value = snapped(response.rect, ui.style().visuals.handle_shape, c, value);
    }
    // Only what the user did: the slider's stepping alone moves a value by
    // a rounding error (0.0 read as 8.9e-8 on a 0.1 step from -6, 1.8 as
    // 1.8000001), and says it changed it.
    if slider_double_clicked(ui, &response) {
        (c.set)(e, standard);
    } else if (value - (c.get)(e)).abs() > 0.4 * c.step {
        let per_unit = (1.0 / c.step).round();
        (c.set)(e, (value * per_unit).round() / per_unit);
    }
}

/// The marks under an enhancer's slider (its rect `slider`), right under
/// its rail, into `slot` (added before the slider, so the handle covers
/// them where it stands): one every `c.ticks.0`, longer at the ends and at
/// `c.ticks.1`, each where the handle's middle stands at that value
/// (`slider_tick_x`; `ticks_stand_under_the_handle` checks it against the
/// handle egui draws). What they reach past the slider's rect is made room
/// for under it.
fn slider_ticks(ui: &mut Ui, slot: egui::layers::ShapeIdx, slider: Rect, c: &Control) {
    /// From the rail's lower edge.
    const GAP: f32 = 1.0;
    const SHORT: f32 = 3.0;
    const LONG: f32 = 5.0;
    let every = c.ticks.0;
    let (lo, hi) = c.range;
    let top = slider.center().y + ui.spacing().slider_rail_height / 2.0 + GAP;
    let below = top + LONG - slider.bottom();
    if below > 0.0 {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.allocate_exact_size(vec2(slider.width(), below), Sense::hover());
    }
    if hi <= lo || every <= 0.0 {
        return;
    }
    let count = ((hi - lo) / every).round() as i32;
    let shape = ui.style().visuals.handle_shape;
    let painter = ui.painter();
    let marks = (0..=count)
        .map(|i| {
            let v = lo + i as f32 * every;
            let long = long_marks(c).any(|m| (m - v).abs() < every * 0.01);
            let (length, colour) = if long {
                (LONG, palette::text_weak().gamma_multiply(0.85))
            } else {
                (SHORT, palette::text_weak().gamma_multiply(0.5))
            };
            egui::Shape::vline(
                painter.round_to_pixel_center(slider_tick_x(slider, shape, c.range, v)),
                top..=top + length,
                Stroke::new(1.0, colour),
            )
        })
        .collect();
    painter.set(slot, egui::Shape::Vec(marks));
}

/// The values of `c`'s long marks: its ends and `c.ticks.1`.
fn long_marks(c: &Control) -> impl Iterator<Item = f32> + '_ {
    [c.range.0, c.range.1]
        .into_iter()
        .chain(c.ticks.1.iter().copied())
}

/// How near, in points, a handle dragged comes to a long mark to stand on
/// it: a little, so the values beside a mark can still be reached.
const SNAP: f32 = 4.0;

/// `value` on `c`'s slider (its rect `slider`, its handle's `shape`), or the
/// long mark nearest it when that is within `SNAP` points of the handle.
fn snapped(slider: Rect, shape: egui::style::HandleShape, c: &Control, value: f32) -> f32 {
    let x = slider_tick_x(slider, shape, c.range, value);
    long_marks(c)
        .map(|m| (m, (slider_tick_x(slider, shape, c.range, m) - x).abs()))
        .filter(|&(_, off)| off <= SNAP)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(value, |(m, _)| m)
}

/// Where the middle of egui's handle (of `shape`) stands at `value` on a
/// horizontal slider of the rect `slider` and `range`, as egui works it
/// out: the handle's "radius" is the slider's height over 2.5, its half
/// width that, or that times the aspect ratio for a rectangle (egui's
/// default, 0.75), and its middle travels the width less the half width
/// at either end, in a straight line.
fn slider_tick_x(
    slider: Rect,
    shape: egui::style::HandleShape,
    range: (f32, f32),
    value: f32,
) -> f32 {
    let radius = slider.height() / 2.5;
    let half = match shape {
        egui::style::HandleShape::Circle => radius,
        egui::style::HandleShape::Rect { aspect_ratio } => radius * aspect_ratio,
    };
    let span = (slider.width() - 2.0 * half).max(0.0);
    let t = ((value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0);
    slider.left() + half + t * span
}

/// What the limiter does: the loudness card's hint, and the tip of its
/// state in the equalizer's window (`ui::eq_window`).
pub(crate) fn limiter_hint() -> &'static str {
    tr!(
        "Uses 4x inter-sample peak detection and adaptive release to hold peaks near -1 dBTP after processing, so raised bands, preamp or enhancer never clip; a raised preamp then makes a loud track denser rather than louder. Turned off, a raise is heard in full, and what passes full scale is clipped.\n\nWith the enhancer on, the switch is its own, off by default: the enhancer leaves room for its peaks, and the rare ones past full scale are clipped. It is worth turning on with the equalizer's bands raised or the enhancer's controls near their ends.",
        "Контролирует межсемпловые пики с передискретизацией 4× и адаптивным восстановлением, удерживая их около -1 dBTP после обработки, чтобы поднятые полосы, предусиление или улучшение звука не клиповали; поднятое предусиление тогда делает громкий трек плотнее, а не громче. Если его выключить, подъём слышен целиком, а то, что выходит за полную шкалу, обрезается.\n\nПри включённом улучшении звука у лимитера свой переключатель, по умолчанию выключенный: улучшение оставляет запас для своих пиков, а редкие пики за полной шкалой обрезаются. Его стоит включить, если подняты полосы эквалайзера или регуляторы улучшения стоят у краёв."
    )
}

/// The balance's slider (-100 to 100, the side named and how far) and a
/// round arrow back to the middle.
fn balance(ui: &mut Ui, value: &mut i8) {
    // The row lays its control out from the right: the arrow at the end.
    if tool(
        ui,
        tr!("Back to the middle", "Вернуть в центр"),
        *value != 0,
        Icon::RotateLeft,
    )
    .clicked()
    {
        *value = 0;
    }
    ui.spacing_mut().slider_width = CONTROL - 70.0 - TOOL;
    ui.add(
        egui::Slider::new(value, -100..=100)
            .custom_formatter(|v, _| match v.round() as i32 {
                0 => tr!("Centre", "Центр").to_owned(),
                v if v < 0 => format!("{} {}", tr!("L", "Л"), -v),
                v => format!("{} {v}", tr!("R", "П")),
            })
            .custom_parser(|text| {
                let t = text.trim().to_lowercase();
                let number = |rest: &str| rest.trim().parse::<f64>().ok();
                if let Some(rest) = t.strip_prefix('l').or_else(|| t.strip_prefix('л')) {
                    number(rest).map(|v| -v)
                } else if let Some(rest) = t.strip_prefix('r').or_else(|| t.strip_prefix('п')) {
                    number(rest)
                } else if t == "centre" || t == "center" || t == "центр" {
                    Some(0.0)
                } else {
                    number(&t)
                }
            }),
    );
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

/// A folding section's state, kept by the caller in `open` (the settings),
/// egui's under `id` only for the animation.
fn fold_of(ui: &Ui, id: &str, open: bool) -> egui::collapsing_header::CollapsingState {
    let mut fold = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        egui::Id::new(id),
        open,
    );
    fold.set_open(open);
    fold
}

/// A folding section's name in its head; a click on it folds or unfolds.
fn fold_title(ui: &mut Ui, title: &str) -> Response {
    ui.add(
        egui::Label::new(
            RichText::new(title)
                .size(15.0)
                .strong()
                .color(palette::text_bright()),
        )
        .sense(Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The fold's button of a section, a `tool` of its own: a chevron in the
/// icons' lines, pointing right while folded and turning down as it opens
/// (`openness`, 0 to 1, egui's animation of the fold).
pub(crate) fn fold_button(ui: &mut Ui, openness: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(TOOL, 24.0), Sense::click());
    let color = if response.hovered() {
        palette::text_bright()
    } else {
        palette::text()
    };
    if response.hovered() {
        let fill = if response.is_pointer_button_down_on() {
            palette::pressed()
        } else {
            palette::hover()
        };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    // A chevron pointing right, turned a quarter clockwise when open.
    let turn = egui::emath::Rot2::from_angle(openness * std::f32::consts::FRAC_PI_2);
    let c = rect.center();
    let points = [vec2(-2.5, -5.0), vec2(2.5, 0.0), vec2(-2.5, 5.0)]
        .map(|p| c + turn * p)
        .to_vec();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.5, color)));
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if openness > 0.5 {
            tr!("Fold", "Свернуть")
        } else {
            tr!("Unfold", "Развернуть")
        })
}

/// A built-in preset uses neutral preamp, unlike a manually adjusted curve.
fn built_in_preset(dsp: &DspSettings) -> Option<usize> {
    eq::PRESETS
        .iter()
        .position(|p| p.bands == dsp.bands && p.preamp == dsp.preamp)
}

/// The equalizer's switch, the built-in presets and the user's own in one
/// list (the one the sliders stand at named; "Custom" otherwise), and
/// beside it saving the sliders as one's own and deleting one's own; the
/// name is typed in a row under it (metalmpv's equalizer head). With
/// `fold` (the bands' openness, in the equalizer's window), the name
/// follows the switch and the fold's chevron the tools; whether the name or
/// the chevron was clicked.
fn eq_head(
    ui: &mut Ui,
    dsp: &mut DspSettings,
    presets: &mut Vec<EqPreset>,
    naming: &mut Option<String>,
    fold: Option<f32>,
) -> bool {
    let mut toggled = false;
    // The one of mine chosen last while the sliders still stand at it,
    // else the first that does: two of the same values are two presets.
    let same = |p: &EqPreset| p.bands == dsp.bands && p.preamp == dsp.preamp;
    let chosen_id = egui::Id::new("eq_mine");
    let mine = ui
        .data(|d| d.get_temp::<usize>(chosen_id))
        .filter(|&i| presets.get(i).is_some_and(same))
        .or_else(|| presets.iter().position(same));
    // Built-in presets start at neutral gain; saved user presets keep their preamp.
    let built_in = built_in_preset(dsp).map(|i| {
        let p = eq::PRESETS[i];
        tr!(p.name, p.ru)
    });
    ui.horizontal(|ui| {
        switch(ui, &mut dsp.eq).on_hover_text(tr!(
            "Ten bands and a manual preamp. Built-in presets use 0 dB preamp; the limiter catches peaks when enabled. The line includes the bands and preamp at 48 kHz.",
            "Десять полос и ручное предусиление. Встроенные пресеты используют предусиление 0 дБ; включённый лимитер удерживает пики. Линия учитывает полосы и предусиление при 48 кГц."
        ));
        ui.add_space(4.0);
        if fold.is_some() {
            toggled |= fold_title(ui, tr!("Equalizer", "Эквалайзер")).clicked();
            ui.add_space(8.0);
        }
        let name = match (mine, built_in) {
            (Some(i), _) => presets[i].name.clone(),
            (None, Some(p)) => p.to_owned(),
            (None, None) => tr!("Custom", "Другой").to_owned(),
        };
        let tools = if fold.is_some() { 3.0 } else { 2.0 };
        let width = ui.available_width() - tools * TOOL - (tools + 1.0) * ui.spacing().item_spacing.x;
        egui::ComboBox::from_id_salt("eq_preset")
            .selected_text(name)
            .width(width)
            .show_ui(ui, |ui| {
                for preset in eq::PRESETS {
                    let on = mine.is_none()
                        && preset.bands == dsp.bands
                        && preset.preamp == dsp.preamp;
                    if ui.selectable_label(on, tr!(preset.name, preset.ru)).clicked() {
                        dsp.bands = preset.bands;
                        dsp.preamp = preset.preamp;
                        // Flat leaves the switch as it is: nothing to hear.
                        if preset.bands.iter().any(|&b| b != 0.0) {
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
        if let Some(openness) = fold {
            toggled |= fold_button(ui, openness).clicked();
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
    toggled
}

/// The equalizer: the switch, the presets and the sliders, here and in
/// the window of the player bar's button (`ui::eq_window`), on a ground
/// of `palette::card_bg`. With `open` (that window), a section that folds
/// as the enhancer's under it: its name in the head, the sliders folded
/// away, the head (the presets) and the name being typed left.
pub(crate) fn equalizer(
    ui: &mut Ui,
    dsp: &mut DspSettings,
    presets: &mut Vec<EqPreset>,
    naming: &mut Option<String>,
    open: Option<&mut bool>,
) {
    let Some(open) = open else {
        eq_head(ui, dsp, presets, naming, None);
        bands(ui, dsp);
        return;
    };
    let mut fold = fold_of(ui, "eq_bands", *open);
    let openness = fold.openness(ui.ctx());
    if eq_head(ui, dsp, presets, naming, Some(openness)) {
        fold.toggle(ui);
    }
    *open = fold.is_open();
    fold.show_body_unindented(ui, |ui| bands(ui, dsp));
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
/// and reset under it, the 0 dB line across and the combined frequency
/// response including the preamp; greyed while the equalizer is off.
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
                        // A vertical slider lays itself out in `ui.vertical`,
                        // at the left of the column: it gets a rect of its
                        // own width in the column's middle, under the gain
                        // and over the label.
                        let thickness = ui
                            .text_style_height(&egui::TextStyle::Body)
                            .max(ui.spacing().interact_size.y);
                        let size = vec2(thickness, ui.spacing().slider_width);
                        let (place, _) = ui.allocate_exact_size(size, Sense::hover());
                        let response = ui
                            .new_child(egui::UiBuilder::new().max_rect(place))
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
        if let Some((top, bottom)) = travel {
            shapes.push(egui::Shape::line(
                eq_response_curve(ui, dsp, &knobs, top, bottom),
                Stroke::new(2.0, curve),
            ));
        }
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

#[derive(Clone)]
struct EqCurve {
    key: [u32; 11],
    db: Vec<f32>,
}

/// Sample the actual filters only when the controls move. The columns
/// stay evenly spaced; frequency between them is interpolated in log Hz.
fn eq_response_curve(
    ui: &Ui,
    dsp: &DspSettings,
    knobs: &[egui::Pos2],
    top: f32,
    bottom: f32,
) -> Vec<egui::Pos2> {
    const STEPS: usize = 16;
    let mut key = [0; 11];
    key[0] = dsp.preamp.to_bits();
    for (to, gain) in key[1..].iter_mut().zip(dsp.bands) {
        *to = gain.to_bits();
    }
    let id = ui.id().with("eq_response_curve");
    let cached = ui.data(|d| d.get_temp::<EqCurve>(id)).filter(|c| c.key == key);
    let db = cached.map_or_else(
        || {
            let response = eq::Response::new(48000, dsp.preamp, &dsp.bands);
            let mut db = Vec::with_capacity((eq::BANDS.len() - 1) * STEPS + 1);
            for pair in eq::BANDS.windows(2) {
                for step in 0..STEPS {
                    let t = step as f32 / STEPS as f32;
                    let hz = pair[0] * (pair[1] / pair[0]).powf(t);
                    db.push(response.gain_db(hz));
                }
            }
            db.push(response.gain_db(eq::BANDS[eq::BANDS.len() - 1]));
            ui.data_mut(|d| d.insert_temp(id, EqCurve { key, db: db.clone() }));
            db
        },
        |c| c.db,
    );
    let zero = (top + bottom) / 2.0;
    db.into_iter()
        .enumerate()
        .map(|(n, gain)| {
            let band = n / STEPS;
            let x = if band == knobs.len() - 1 {
                knobs[band].x
            } else {
                egui::lerp(
                    knobs[band].x..=knobs[band + 1].x,
                    (n % STEPS) as f32 / STEPS as f32,
                )
            };
            let y = zero - gain.clamp(-eq::RANGE_DB, eq::RANGE_DB)
                * (bottom - top) / (2.0 * eq::RANGE_DB);
            pos2(x, y)
        })
        .collect()
}

/// The width of the accents' row: eleven swatches.
const ACCENTS_WIDTH: f32 = 302.0;
/// The size of an accent's swatch.
const SWATCH: f32 = 22.0;

/// The accents as round swatches, the chosen one ringed: first `theme`'s
/// own (`Accent::Theme`), then the ready ones but the one of its colour
/// (amber in the dark theme, which then counts as the theme's), last the
/// user's own colour, picked in a popup (metalmpv's).
fn accents(ui: &mut Ui, accent: &mut Accent, theme: Theme) {
    ui.spacing_mut().item_spacing.x = 5.0;
    let own = Accent::Theme.base(theme);
    let ready = Accent::ALL.into_iter().filter(|a| a.base(theme) != own);
    // Right to left: the own colour first, then the ready ones from the
    // last, then the theme's.
    own_accent(ui, accent);
    for a in ready.rev().chain(std::iter::once(Accent::Theme)) {
        let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
        let response = response.on_hover_text(a.label());
        let selected = *accent == a
            || (a == Accent::Theme
                && *accent != Accent::Theme
                && !matches!(accent, Accent::Custom(_))
                && accent.base(theme) == own);
        swatch(ui, rect, selected, response.hovered(), |p, centre, r| {
            p.circle_filled(centre, r, a.base(theme));
        });
        if response.clicked() {
            *accent = a;
        }
    }
}

/// A view's icon colour, left to right: the accent in use (none), the
/// ready accents, the user's own (its picker starting from the colour in
/// use).
pub(crate) fn view_colours(ui: &mut Ui, colour: &mut Option<Accent>) {
    ui.spacing_mut().item_spacing.x = 5.0;
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    let response = response.on_hover_text(tr!("The accent's", "Цвет акцента"));
    swatch(
        ui,
        rect,
        colour.is_none(),
        response.hovered(),
        |p, centre, r| {
            p.circle_filled(centre, r, palette::accent());
            // A dot of the ground: it follows the accent.
            p.circle_filled(centre, r * 0.35, palette::panel_bg());
        },
    );
    if response.clicked() {
        *colour = None;
    }
    for a in Accent::ALL {
        let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
        let response = response.on_hover_text(a.label());
        swatch(
            ui,
            rect,
            *colour == Some(a),
            response.hovered(),
            |p, centre, r| {
                p.circle_filled(centre, r, a.base(palette::current().theme));
            },
        );
        if response.clicked() {
            *colour = Some(a);
        }
    }
    let mut own = match *colour {
        Some(c @ Accent::Custom(_)) => c,
        _ => {
            let c = palette::icon_colour(*colour);
            custom([c.r(), c.g(), c.b()])
        }
    };
    let chosen = matches!(colour, Some(Accent::Custom(_)));
    if own_colour(ui, &mut own, chosen) {
        *colour = Some(own);
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
                p.circle_filled(centre, r * 0.35, accent.base(palette::current().theme));
            }
        });
        if response.clicked() {
            *tint = s;
        }
    }
}

/// The playlist's rows: the line's template, chosen among the ready ones
/// (named by their fields) or typed; the numbers and the lengths.
fn playlist_rows(ui: &mut Ui, settings: &mut Settings) {
    use crate::title_format::{FIELDS, PRESETS, field_label, render};
    let label = |t: &str| render(t, &|n| field_label(n).map(str::to_owned));
    row(
        ui,
        tr!("Track's line", "Строка трека"),
        Some(tr!(
            "How a track is written in the playlist and on the narrow player.",
            "Как трек записан в плейлисте и в узком плеере."
        )),
        CONTROL,
        |ui| {
            let format = &mut settings.playlist_format;
            let own = !PRESETS.contains(&format.as_str());
            let shown = if own {
                tr!("Own template", "Свой шаблон").to_owned()
            } else {
                label(format)
            };
            egui::ComboBox::from_id_salt("settings_playlist_format")
                .selected_text(shown)
                .width(CONTROL - 10.0)
                .show_ui(ui, |ui| {
                    for p in PRESETS {
                        if ui.selectable_label(format == p, label(p)).clicked() {
                            *format = p.to_owned();
                        }
                    }
                });
        },
    );
    divider(ui);
    let fields: Vec<String> = FIELDS.iter().map(|f| format!("%{f}%")).collect();
    let fields = fields.join(", ");
    row(
        ui,
        tr!("Template", "Шаблон"),
        Some(&tr!(
            format!(
                "The fields {fields}; a part in square brackets is shown only when a field in it has something: [%artist% – ]%title%."
            ),
            format!(
                "Поля {fields}; часть в квадратных скобках показывается, только если в ней есть хоть одно поле: [%artist% – ]%title%."
            )
        )),
        CONTROL,
        |ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut settings.playlist_format)
                    .desired_width(CONTROL - 10.0),
            );
            super::take_leaving_key(ui, &field);
        },
    );
    divider(ui);
    row(
        ui,
        tr!("Numbers", "Номера"),
        Some(tr!(
            "Each row's number in the playlist.",
            "Номер каждой строки плейлиста."
        )),
        CONTROL,
        |ui| {
            switch(ui, &mut settings.playlist_numbers);
        },
    );
    divider(ui);
    row(
        ui,
        tr!("Lengths", "Длительность"),
        Some(tr!(
            "Each track's length at the right of its row.",
            "Длительность трека справа в строке."
        )),
        CONTROL,
        |ui| {
            switch(ui, &mut settings.playlist_lengths);
        },
    );
}

/// The visualization's colours as round swatches, each painted with its
/// gradient (`gradient_disc`: up the level, across the place), the chosen
/// one ringed (`Own` showing `visual`'s); last the user's own colour, its
/// picker starting from the accent.
fn visual_colours(ui: &mut Ui, colours: &mut VisualColours, look: palette::Look, visual: Visual) {
    ui.spacing_mut().item_spacing.x = 5.0;
    let mut own = match *colours {
        VisualColours::Custom(c) => Accent::Custom(c),
        _ => {
            let c = look.accent.base(look.theme);
            custom([c.r(), c.g(), c.b()])
        }
    };
    let chosen = matches!(colours, VisualColours::Custom(_));
    if own_colour(ui, &mut own, chosen)
        && let Accent::Custom(c) = own
    {
        *colours = VisualColours::Custom(c);
    }
    for &c in VisualColours::ALL.iter().rev() {
        visual_swatch(ui, colours, c, look, visual);
    }
}

/// The visualization's colours in the display's menu, left to right: the
/// ready ones, then the user's own once chosen (it is picked in the
/// settings window); `Own` shows `visual`'s.
pub(crate) fn visual_colours_row(
    ui: &mut Ui,
    colours: &mut VisualColours,
    look: palette::Look,
    visual: Visual,
) {
    ui.spacing_mut().item_spacing.x = 5.0;
    let own = matches!(*colours, VisualColours::Custom(_)).then_some(*colours);
    for c in VisualColours::ALL.into_iter().chain(own) {
        visual_swatch(ui, colours, c, look, visual);
    }
}

/// The swatch of `c`, chosen by a click.
fn visual_swatch(
    ui: &mut Ui,
    colours: &mut VisualColours,
    c: VisualColours,
    look: palette::Look,
    visual: Visual,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    let response = response.on_hover_text(c.label());
    let mut shades = Palette::of(palette::Look { visual: c, ..look }).visual;
    match c {
        VisualColours::Cover => shades = palette::with_cover(shades),
        VisualColours::Own => shades = palette::with_own(shades, visual),
        _ => {}
    }
    swatch(
        ui,
        rect,
        *colours == c,
        response.hovered(),
        |p, centre, r| {
            gradient_disc(p, centre, r, shades);
        },
    );
    if response.clicked() {
        *colours = c;
    }
}

/// A disc of `shades`' colours: the level up, the place across.
fn gradient_disc(
    painter: &egui::Painter,
    centre: egui::Pos2,
    r: f32,
    shades: palette::VisualPalette,
) {
    let colour = |p: egui::Pos2| {
        let level = (centre.y + r - p.y) / (2.0 * r);
        let place = (p.x - centre.x + r) / (2.0 * r);
        shades.at(level, place)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, colour(centre));
    const STEPS: u32 = 32;
    for i in 0..=STEPS {
        let a = i as f32 / STEPS as f32 * std::f32::consts::TAU;
        let p = centre + vec2(a.cos(), a.sin()) * r;
        mesh.colored_vertex(p, colour(p));
        if i > 0 {
            mesh.add_triangle(0, i, i + 1);
        }
    }
    painter.add(mesh);
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
    let current = accent.base(palette::current().theme);
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
            let mut colour = accent.base(palette::current().theme);
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
                let base = accent.base(palette::current().theme);
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
                let base = accent.base(palette::current().theme);
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
    settings.cover_priority = d.cover_priority;
    settings.lyrics_online = d.lyrics_online;
    settings.lyrics_priority = d.lyrics_priority;
    settings.window_layout = d.window_layout;
    settings.visualization = d.visualization;
    settings.playlist_visualization = d.playlist_visualization;
    settings.visual_enabled = d.visual_enabled;
    settings.visual_click = d.visual_click;
    settings.visual_colours = d.visual_colours;
    settings.playlist_visual_colours = d.playlist_visual_colours;
    settings.playlist_format = d.playlist_format;
    settings.playlist_numbers = d.playlist_numbers;
    settings.playlist_lengths = d.playlist_lengths;
    settings.theme = d.theme;
    settings.accent = d.accent;
    settings.selection = d.selection;
    settings.playing = d.playing;
    settings.media_keys = d.media_keys;
    settings.id3v2_version = d.id3v2_version;
    settings.id3v1 = d.id3v1;
    settings.id3v1_encoding = d.id3v1_encoding;
    settings.ape = d.ape;
    settings.keys = d.keys;
    settings.output_device = d.output_device;
    settings.output_device_name = d.output_device_name;
    settings.dsp = d.dsp;
    settings.crossfade = d.crossfade;
    settings.crossfade_in_albums = d.crossfade_in_albums;
    settings.crossfade_manual = d.crossfade_manual;
    settings.crossfade_seek = d.crossfade_seek;
    settings.seek_step = d.seek_step;
    settings.volume_step = d.volume_step;
    settings.tray = d.tray;
    settings.tray_only = d.tray_only;
    settings.track_notice = d.track_notice;
    settings.track_notice_seconds = d.track_notice_seconds;
    updates.enabled = d.check_updates;
}

#[cfg(test)]
mod eq_tests {
    use super::*;

    /// Every mark under every enhancer slider stands under the middle of
    /// the handle egui draws at the mark's value, in cells of a whole and
    /// a fractional width.
    #[test]
    fn ticks_stand_under_the_handle() {
        fn flat(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| flat(s, out)),
                s => out.push(s.clone()),
            }
        }
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            ..Default::default()
        };
        for width in [200.0, 333.7] {
            for control in enhancer_controls().iter().flatten() {
                let (every, _) = control.ticks;
                let count = ((control.range.1 - control.range.0) / every).round() as i32;
                for i in 0..=count {
                    let v = control.range.0 + i as f32 * every;
                    let mut e = Enhancement::default();
                    (control.set)(&mut e, v);
                    let base = e;
                    let out = ctx.run_ui(input.clone(), |ui| {
                        ui.allocate_ui_with_layout(
                            vec2(width, 0.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(width);
                                enhancer_cell(ui, control, &mut e, base, true);
                            },
                        );
                    });
                    let mut out = out;
                    out.textures_delta.clear();
                    let mut shapes = Vec::new();
                    for clipped in &out.shapes {
                        flat(&clipped.shape, &mut shapes);
                    }
                    // The handle, round or a rectangle taller than the
                    // rail.
                    let rail = ctx.global_style().spacing.slider_rail_height;
                    let handle = shapes
                        .iter()
                        .find_map(|s| match s {
                            egui::Shape::Circle(c) => Some(c.center),
                            egui::Shape::Rect(r) if r.rect.height() > rail + 1.0 => {
                                Some(r.rect.center())
                            }
                            _ => None,
                        })
                        .expect("the handle");
                    let ticks: Vec<f32> = shapes
                        .iter()
                        .filter_map(|s| match s {
                            egui::Shape::LineSegment { points: [a, b], .. }
                                if a.x == b.x && a.y.min(b.y) > handle.y =>
                            {
                                Some(a.x)
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(ticks.len() as i32, count + 1, "{}", control.title);
                    let off = (ticks[i as usize] - handle.x).abs();
                    assert!(
                        off <= 0.5,
                        "{} at {v}, {width} wide: the mark {off} off the handle",
                        control.title
                    );
                }
            }
        }
    }

    /// A value within `SNAP` points of a long mark stands on it, one
    /// further off stays as it is, and the short marks do not hold it.
    #[test]
    fn a_dragged_handle_sticks_to_the_long_marks() {
        let slider = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 18.0));
        let shape = egui::style::HandleShape::Rect { aspect_ratio: 0.75 };
        let [first, _, last] = enhancer_controls();
        let drive = &first[0];
        // Drive: 140 % over 189.2 points, 1.35 points a per cent.
        assert_eq!(snapped(slider, shape, drive, 98.0), 100.0);
        assert_eq!(snapped(slider, shape, drive, 102.0), 100.0);
        assert_eq!(snapped(slider, shape, drive, 104.0), 104.0);
        assert_eq!(snapped(slider, shape, drive, 95.0), 95.0);
        assert_eq!(snapped(slider, shape, drive, 52.0), 50.0);
        assert_eq!(snapped(slider, shape, drive, 2.0), 0.0);
        assert_eq!(snapped(slider, shape, drive, 138.0), 140.0);
        // A short mark (30 %) holds nothing.
        assert_eq!(snapped(slider, shape, drive, 31.0), 31.0);
        // The bass: 12 dB over 189.2 points, 15.8 points a dB.
        let bass = &last[0];
        assert_eq!(snapped(slider, shape, bass, 0.2), 0.0);
        assert_eq!(snapped(slider, shape, bass, -0.2), 0.0);
        assert_eq!(snapped(slider, shape, bass, 0.3), 0.3);
        assert_eq!(snapped(slider, shape, bass, 1.0), 1.0);
    }

    #[test]
    fn built_in_presets_include_their_preamp() {
        for (index, preset) in eq::PRESETS.iter().enumerate() {
            let mut dsp = DspSettings {
                preamp: preset.preamp,
                bands: preset.bands,
                ..Default::default()
            };
            assert_eq!(built_in_preset(&dsp), Some(index), "{}", preset.name);
            dsp.preamp += 0.5;
            assert_eq!(built_in_preset(&dsp), None, "{}", preset.name);
        }
    }

    #[test]
    fn equalizer_renders_and_the_curve_follows_the_preamp() {
        let ctx = egui::Context::default();
        let preset = eq::PRESETS[6];
        let mut dsp = DspSettings {
            eq: true,
            preamp: preset.preamp,
            bands: preset.bands,
            ..Default::default()
        };
        let saved = dsp.clone();
        let mut own = Vec::new();
        let mut naming = None;
        let mut mine = Vec::new();
        let mut enhancer_naming = None;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input.clone(), |ui| {
            ui.set_width(560.0);
            equalizer(ui, &mut dsp, &mut own, &mut naming, None);
        });
        out.textures_delta.clear();
        assert_eq!(dsp, saved);
        assert!(!ctx.tessellate(out.shapes, out.pixels_per_point).is_empty());

        // The sound tuning window's way: both sections folding, folded or
        // not, left as they are by a frame without a click.
        for open in [true, false] {
            let (mut bands_open, mut enhancer_open) = (open, open);
            let mut out = ctx.run_ui(input.clone(), |ui| {
                ui.set_width(560.0);
                equalizer(ui, &mut dsp, &mut own, &mut naming, Some(&mut bands_open));
                enhancer(
                    ui,
                    &mut dsp,
                    &mut mine,
                    &mut enhancer_naming,
                    Some(&mut enhancer_open),
                );
            });
            out.textures_delta.clear();
            assert_eq!((bands_open, enhancer_open), (open, open));
            assert_eq!(dsp, saved);
            assert!(!ctx.tessellate(out.shapes, out.pixels_per_point).is_empty());
        }

        let knobs: Vec<_> = (0..10).map(|i| pos2(40.0 * i as f32, 200.0)).collect();
        let mut out = ctx.run_ui(input, |ui| {
            let mut flat = DspSettings::default();
            let zero = eq_response_curve(ui, &flat, &knobs, 100.0, 300.0);
            assert!(zero.iter().all(|p| (p.y - 200.0).abs() < 1e-4));
            flat.preamp = -3.0;
            let lower = eq_response_curve(ui, &flat, &knobs, 100.0, 300.0);
            assert!(lower.iter().all(|p| (p.y - 225.0).abs() < 1e-4));
        });
        out.textures_delta.clear();
    }
}
