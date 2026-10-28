//! The keys of the player as the user set them (metalmpv's `keymap.rs`):
//! the actions a key can do (`Cmd`), the keys (`Chord`s) each has by
//! default, the user's changes kept in the settings, and the labels the
//! tooltips show (`with_key`).
//!
//! Only the changes are kept (`Keymap::to_setting`): an action not named
//! there has its default keys, so a new action of a later version gets
//! its keys even where others were changed. Keys that depend on where
//! the pointer is (the arrows over the player bar, letters over the
//! artists) or on the pane that has the keys (the lists' arrows, Enter,
//! Delete) are not here.

use std::sync::{LazyLock, RwLock};

use egui::{Key, Modifiers, PointerButton};

/// What a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    PlayPause,
    Play,
    Stop,
    StopAfter,
    Previous,
    Next,
    /// Back (-1) or forward (1) by the seek step (`settings.seek_step`).
    Seek(i64),
    Repeat,
    Shuffle,
    /// Percent up or down.
    Volume(i8),
    Mute,
    Equalizer,
    EqualizerOnOff,
    Normalize,
    Mini,
    OnTop,
    Playlist,
    WindowLayout,
    Visualization,
    Minimize,
    Maximize,
    Close,
    Search,
    Open,
    AddFolder,
    Rescan,
    Info,
    EditTags,
    /// The whole library (0) or a view, as its menu lists them.
    View(u8),
    Settings,
    About,
}

/// What a chord presses: a key, or a side button of the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Press {
    Key(Key),
    /// The mouse's back button (egui's `Extra1`).
    MouseBack,
    /// The mouse's forward button (egui's `Extra2`).
    MouseForward,
}

/// A key or a side button with the modifiers held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub press: Press,
}

/// `m` with Ctrl: a copy or cut event comes from a press with it.
fn with_ctrl(m: Modifiers) -> Modifiers {
    Modifiers { ctrl: true, ..m }
}

impl Chord {
    pub const fn key(key: Key) -> Self {
        Self::of(Press::Key(key), false, false, false)
    }

    const fn ctrl(key: Key) -> Self {
        Self::of(Press::Key(key), true, false, false)
    }

    const fn shift(key: Key) -> Self {
        Self::of(Press::Key(key), false, false, true)
    }

    const fn alt(key: Key) -> Self {
        Self::of(Press::Key(key), false, true, false)
    }

    const fn of(press: Press, ctrl: bool, alt: bool, shift: bool) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            press,
        }
    }

    pub fn new(press: Press, m: Modifiers) -> Self {
        Self::of(press, m.command || m.ctrl, m.alt, m.shift)
    }

    /// The chord of a key press, unless the key is a modifier itself
    /// (egui-winit passes the physical Shift, Ctrl and Alt as keys).
    pub fn of_key(key: Key, m: Modifiers) -> Option<Self> {
        let modifier = matches!(
            key,
            Key::ShiftLeft
                | Key::ShiftRight
                | Key::ControlLeft
                | Key::ControlRight
                | Key::AltLeft
                | Key::AltRight
                | Key::SuperLeft
                | Key::SuperRight
        );
        (!modifier).then(|| Self::new(Press::Key(key), m))
    }

    /// The chord of a press of the mouse's side button.
    pub fn of_button(button: PointerButton, m: Modifiers) -> Option<Self> {
        let press = match button {
            PointerButton::Extra1 => Press::MouseBack,
            PointerButton::Extra2 => Press::MouseForward,
            _ => return None,
        };
        Some(Self::new(press, m))
    }

    /// The chord of a press in `event`, `modifiers` held, and whether it is
    /// a held key's repeat. egui-winit turns `Ctrl+C` and `Ctrl+X` (with
    /// any other modifiers), `Ctrl+Insert` and `Shift+Delete` into `Copy`
    /// and `Cut` with no key event: the modifiers held tell them apart, but
    /// for `Ctrl+Insert`, taken for `Ctrl+C`. `Ctrl+V` and `Shift+Insert`
    /// become a paste of the clipboard's text, so they are no keys here.
    pub fn of_event(event: &egui::Event, modifiers: egui::Modifiers) -> Option<(Self, bool)> {
        use egui::Event;
        match event {
            Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } => Some((Self::of_key(*key, *modifiers)?, *repeat)),
            Event::PointerButton {
                button,
                pressed: true,
                modifiers,
                ..
            } => Some((Self::of_button(*button, *modifiers)?, false)),
            Event::Copy => Some((Self::new(Press::Key(Key::C), with_ctrl(modifiers)), false)),
            // Shift+Delete, or Ctrl+X.
            Event::Cut if modifiers.shift && !modifiers.ctrl => {
                Some((Self::new(Press::Key(Key::Delete), modifiers), false))
            }
            Event::Cut => Some((Self::new(Press::Key(Key::X), with_ctrl(modifiers)), false)),
            _ => None,
        }
    }

    /// As kept in the settings: `Ctrl+Shift+Left`, `Alt+MouseBack`.
    pub fn name(&self) -> String {
        let press = match self.press {
            Press::Key(key) => key.name(),
            Press::MouseBack => "MouseBack",
            Press::MouseForward => "MouseForward",
        };
        self.with_modifiers(press)
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let mut chord = Self::key(Key::Space);
        let mut press = None;
        for part in name.split('+') {
            match part {
                "Ctrl" => chord.ctrl = true,
                "Alt" => chord.alt = true,
                "Shift" => chord.shift = true,
                "MouseBack" => press = Some(Press::MouseBack),
                "MouseForward" => press = Some(Press::MouseForward),
                other => press = Some(Press::Key(Key::from_name(other)?)),
            }
        }
        chord.press = press?;
        Some(chord)
    }

    /// As shown to the user: `Ctrl+←`, `Пробел`.
    pub fn label(&self) -> String {
        let press = match self.press {
            Press::Key(key) => key_label(key),
            Press::MouseBack => tr!("Mouse back", "Мышь назад"),
            Press::MouseForward => tr!("Mouse forward", "Мышь вперёд"),
        };
        self.with_modifiers(press)
    }

    fn with_modifiers(&self, press: &str) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
        ] {
            if on {
                s.push_str(name);
            }
        }
        s.push_str(press);
        s
    }

    /// A chord a text field takes for itself (typing, moving the cursor,
    /// the clipboard, Esc): it does nothing else while one has the focus.
    /// F-keys, Alt chords and the other Ctrl chords still act.
    pub fn edits_text(&self) -> bool {
        let Press::Key(key) = self.press else {
            return false;
        };
        if self.alt || is_function_key(key) {
            return false;
        }
        if !self.ctrl {
            return true;
        }
        matches!(
            key,
            Key::ArrowLeft
                | Key::ArrowRight
                | Key::ArrowUp
                | Key::ArrowDown
                | Key::Home
                | Key::End
                | Key::Backspace
                | Key::Delete
                | Key::Insert
                | Key::A
                | Key::C
                | Key::V
                | Key::X
                | Key::Y
                | Key::Z
        )
    }

    /// A key typing a character (a letter, a digit, a sign) with no
    /// modifiers: typed over the artists' pane, it goes to an artist.
    pub fn types(&self) -> bool {
        let Press::Key(key) = self.press else {
            return false;
        };
        !self.ctrl
            && !self.alt
            && !self.shift
            && (key.name().chars().count() == 1
                || matches!(
                    key,
                    Key::Minus
                        | Key::Plus
                        | Key::Equals
                        | Key::Comma
                        | Key::Period
                        | Key::Semicolon
                        | Key::Colon
                        | Key::Slash
                        | Key::Backslash
                        | Key::Pipe
                        | Key::OpenBracket
                        | Key::CloseBracket
                        | Key::Backtick
                        | Key::Quote
                        | Key::Questionmark
                        | Key::Exclamationmark
                ))
    }
}

fn is_function_key(key: Key) -> bool {
    let name = key.name();
    name.len() >= 2 && name.starts_with('F') && name[1..].chars().all(|c| c.is_ascii_digit())
}

/// A key's name on a keycap.
fn key_label(key: Key) -> &'static str {
    match key {
        // In words: the program's font has no arrows.
        Key::ArrowLeft => tr!("Left", "Влево"),
        Key::ArrowRight => tr!("Right", "Вправо"),
        Key::ArrowUp => tr!("Up", "Вверх"),
        Key::ArrowDown => tr!("Down", "Вниз"),
        Key::Space => tr!("Space", "Пробел"),
        Key::Escape => "Esc",
        Key::Tab => "Tab",
        Key::PageUp => "PgUp",
        Key::PageDown => "PgDn",
        Key::Minus => "-",
        // The key left of 1 and the brackets by their symbols in both
        // languages, whatever the layout puts on them.
        Key::Backtick => "`",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        // A symbol the font may lack (Enter's, Backspace's) by its name.
        _ => {
            let symbol = key.symbol_or_name();
            if symbol.is_ascii() {
                symbol
            } else {
                key.name()
            }
        }
    }
}

/// The parts of the key list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Playback,
    Sound,
    Window,
    Library,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Playback, Group::Sound, Group::Window, Group::Library];

    pub fn title(self) -> &'static str {
        match self {
            Group::Playback => tr!("Playback", "Воспроизведение"),
            Group::Sound => tr!("Sound", "Звук"),
            Group::Window => tr!("Window", "Окно"),
            Group::Library => tr!("Library and files", "Библиотека и файлы"),
        }
    }
}

macro_rules! actions {
    ($($action:ident $id:literal $group:ident $cmd:expr, [$($chord:expr),*];)*) => {
        /// What a key can do.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Action {
            $($action,)*
        }

        impl Action {
            pub const ALL: &'static [Action] = &[$(Action::$action,)*];

            /// The name in the settings.
            pub fn id(self) -> &'static str {
                match self {
                    $(Action::$action => $id,)*
                }
            }

            pub fn group(self) -> Group {
                match self {
                    $(Action::$action => Group::$group,)*
                }
            }

            pub fn cmd(self) -> Cmd {
                match self {
                    $(Action::$action => $cmd,)*
                }
            }

            pub fn defaults(self) -> &'static [Chord] {
                match self {
                    $(Action::$action => const { &[$($chord),*] },)*
                }
            }
        }
    };
}

/// The step of the volume keys and of a wheel notch, in percent.
pub const VOLUME_STEP: i8 = 5;

actions! {
    PlayPause "play_pause" Playback Cmd::PlayPause, [Chord::key(Key::Space), Chord::key(Key::C)];
    Play "play" Playback Cmd::Play, [Chord::key(Key::X)];
    Stop "stop" Playback Cmd::Stop, [Chord::key(Key::V)];
    StopAfter "stop_after" Playback Cmd::StopAfter, [Chord::shift(Key::V)];
    Previous "previous" Playback Cmd::Previous, [Chord::ctrl(Key::ArrowLeft), Chord::key(Key::Z)];
    Next "next" Playback Cmd::Next, [Chord::ctrl(Key::ArrowRight), Chord::key(Key::B)];
    SeekBack "seek_back" Playback Cmd::Seek(-1), [Chord::shift(Key::ArrowLeft)];
    SeekForward "seek_forward" Playback Cmd::Seek(1), [Chord::shift(Key::ArrowRight)];
    Repeat "repeat" Playback Cmd::Repeat, [Chord::key(Key::R)];
    Shuffle "shuffle" Playback Cmd::Shuffle, [Chord::key(Key::S)];

    VolumeUp "volume_up" Sound Cmd::Volume(VOLUME_STEP), [Chord::ctrl(Key::ArrowUp)];
    VolumeDown "volume_down" Sound Cmd::Volume(-VOLUME_STEP), [Chord::ctrl(Key::ArrowDown)];
    Mute "mute" Sound Cmd::Mute, [Chord::key(Key::M)];
    Equalizer "equalizer" Sound Cmd::Equalizer, [];
    EqualizerOnOff "equalizer_on_off" Sound Cmd::EqualizerOnOff, [];
    Normalize "normalize" Sound Cmd::Normalize, [];

    Mini "mini_player" Window Cmd::Mini, [Chord::ctrl(Key::M)];
    OnTop "on_top" Window Cmd::OnTop, [Chord::ctrl(Key::T)];
    Playlist "playlist" Window Cmd::Playlist, [Chord::alt(Key::E)];
    WindowLayout "window_layout" Window Cmd::WindowLayout, [];
    Visualization "visualization" Window Cmd::Visualization, [];
    Minimize "minimize" Window Cmd::Minimize, [];
    Maximize "maximize" Window Cmd::Maximize, [];
    Close "close" Window Cmd::Close, [];

    Search "search" Library Cmd::Search, [Chord::ctrl(Key::F)];
    Open "open" Library Cmd::Open, [Chord::ctrl(Key::O)];
    AddFolder "add_folder" Library Cmd::AddFolder, [Chord::of(Press::Key(Key::O), true, false, true)];
    Rescan "rescan" Library Cmd::Rescan, [Chord::ctrl(Key::R)];
    Info "info" Library Cmd::Info, [Chord::key(Key::I), Chord::alt(Key::Num3)];
    EditTags "edit_tags" Library Cmd::EditTags, [Chord::ctrl(Key::E)];
    WholeLibrary "view_0" Library Cmd::View(0), [Chord::ctrl(Key::Num1)];
    View1 "view_1" Library Cmd::View(1), [Chord::ctrl(Key::Num2)];
    View2 "view_2" Library Cmd::View(2), [Chord::ctrl(Key::Num3)];
    View3 "view_3" Library Cmd::View(3), [Chord::ctrl(Key::Num4)];
    View4 "view_4" Library Cmd::View(4), [Chord::ctrl(Key::Num5)];
    View5 "view_5" Library Cmd::View(5), [Chord::ctrl(Key::Num6)];
    View6 "view_6" Library Cmd::View(6), [Chord::ctrl(Key::Num7)];
    View7 "view_7" Library Cmd::View(7), [Chord::ctrl(Key::Num8)];
    View8 "view_8" Library Cmd::View(8), [Chord::ctrl(Key::Num9)];
    Settings "settings" Library Cmd::Settings, [Chord::key(Key::F5), Chord::ctrl(Key::P)];
    About "about" Library Cmd::About, [Chord::key(Key::F1)];
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::PlayPause => tr!("Play / pause", "Воспроизведение / пауза"),
            Action::Play => tr!(
                "Play (from the start when stopped)",
                "Воспроизвести (после стопа — сначала)"
            ),
            Action::Stop => tr!("Stop", "Стоп"),
            Action::StopAfter => tr!("Stop after the current track", "Стоп после текущего трека"),
            Action::Previous => tr!(
                "Previous track (the start of this one after 3 s)",
                "Предыдущий трек (после 3 с — начало текущего)"
            ),
            Action::Next => tr!("Next track", "Следующий трек"),
            Action::SeekBack => tr!(
                "Seek back (the step is on the General page)",
                "Перемотка назад (шаг — на странице «Общие»)"
            ),
            Action::SeekForward => tr!(
                "Seek forward (the step is on the General page)",
                "Перемотка вперёд (шаг — на странице «Общие»)"
            ),
            Action::Repeat => tr!(
                "Repeat: off, the playlist, the track",
                "Повтор: выключен, плейлист, трек"
            ),
            Action::Shuffle => tr!("Shuffle on / off", "Перемешивание вкл. / выкл."),
            Action::VolumeUp => tr!("Volume up", "Громче"),
            Action::VolumeDown => tr!("Volume down", "Тише"),
            Action::Mute => tr!("Mute", "Без звука"),
            Action::Equalizer => tr!("Equalizer window", "Окно эквалайзера"),
            Action::EqualizerOnOff => tr!("Equalizer on / off", "Эквалайзер вкл. / выкл."),
            Action::Normalize => tr!(
                "Loudness normalisation on / off",
                "Нормализация громкости вкл. / выкл."
            ),
            Action::Mini => tr!("Mini player / the whole window", "Мини-плеер / всё окно"),
            Action::OnTop => tr!(
                "Over the other windows on / off",
                "Поверх всех окон вкл. / выкл."
            ),
            Action::Playlist => tr!("Show or hide the playlist", "Показать или скрыть плейлист"),
            Action::WindowLayout => tr!(
                "With the library / player and playlist",
                "С библиотекой / плеер и плейлист"
            ),
            Action::Visualization => tr!("Visualization on / off", "Визуализация вкл. / выкл."),
            Action::Minimize => tr!("Minimize the window", "Свернуть окно"),
            Action::Maximize => tr!(
                "Maximize / restore the window",
                "Развернуть / восстановить окно"
            ),
            Action::Close => tr!("Close the program", "Закрыть программу"),
            Action::Search => tr!("Search the library", "Поиск в библиотеке"),
            Action::Open => tr!(
                "Open files (in place of the playlist)",
                "Открыть файлы (вместо плейлиста)"
            ),
            Action::AddFolder => tr!("Add a folder to the library", "Добавить папку в библиотеку"),
            Action::Rescan => tr!("Read the folders again", "Перечитать папки"),
            Action::Info => tr!("File info", "Сведения о файле"),
            Action::EditTags => tr!("Edit tags", "Изменить теги"),
            Action::WholeLibrary => tr!("The whole library", "Вся библиотека"),
            Action::View1 => tr!("The first view", "Первое представление"),
            Action::View2 => tr!("The second view", "Второе представление"),
            Action::View3 => tr!("The third view", "Третье представление"),
            Action::View4 => tr!("The fourth view", "Четвёртое представление"),
            Action::View5 => tr!("The fifth view", "Пятое представление"),
            Action::View6 => tr!("The sixth view", "Шестое представление"),
            Action::View7 => tr!("The seventh view", "Седьмое представление"),
            Action::View8 => tr!("The eighth view", "Восьмое представление"),
            Action::Settings => tr!("Settings", "Настройки"),
            Action::About => tr!("About", "О программе"),
        }
    }

    /// Whether a held key repeats it: seeking and the volume.
    pub fn repeats(self) -> bool {
        matches!(self.cmd(), Cmd::Seek(_) | Cmd::Volume(_))
    }

    fn index(self) -> usize {
        Action::ALL
            .iter()
            .position(|&a| a == self)
            .expect("every action is in ALL")
    }

    fn from_id(id: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.id() == id)
    }
}

/// The chords of every action, in the order of `Action::ALL`.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    chords: Vec<Vec<Chord>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            chords: Action::ALL.iter().map(|a| a.defaults().to_vec()).collect(),
        }
    }
}

/// Kept in the settings as the text of [`Keymap::to_setting`].
impl serde::Serialize for Keymap {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_setting())
    }
}

impl<'de> serde::Deserialize<'de> for Keymap {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Ok(Keymap::from_setting(&text))
    }
}

impl Keymap {
    pub fn chords(&self, action: Action) -> &[Chord] {
        &self.chords[action.index()]
    }

    /// The action of `chord`, if any.
    pub fn action(&self, chord: Chord) -> Option<Action> {
        Action::ALL
            .iter()
            .copied()
            .find(|&a| self.chords(a).contains(&chord))
    }

    /// Give `action` the chord, in place of its chord `slot` or as one more
    /// (`None`); the action that had the chord loses it and is returned.
    pub fn assign(&mut self, action: Action, slot: Option<usize>, chord: Chord) -> Option<Action> {
        let mine = &self.chords[action.index()];
        if mine.contains(&chord) {
            return None;
        }
        let taken = self.action(chord);
        if let Some(other) = taken {
            self.chords[other.index()].retain(|&c| c != chord);
        }
        let mine = &mut self.chords[action.index()];
        match slot {
            Some(i) if i < mine.len() => mine[i] = chord,
            _ => mine.push(chord),
        }
        taken
    }

    pub fn remove(&mut self, action: Action, slot: usize) {
        let mine = &mut self.chords[action.index()];
        if slot < mine.len() {
            mine.remove(slot);
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The keys of `action` are its defaults.
    pub fn is_default_for(&self, action: Action) -> bool {
        self.chords(action) == action.defaults()
    }

    /// Give `action` its default keys back; each is taken from the action
    /// that had it, and those are returned with the key they lost.
    pub fn reset(&mut self, action: Action) -> Vec<(Chord, Action)> {
        let mut taken = Vec::new();
        for &chord in action.defaults() {
            if let Some(other) = self.action(chord).filter(|&a| a != action) {
                self.chords[other.index()].retain(|&c| c != chord);
                taken.push((chord, other));
            }
        }
        self.chords[action.index()] = action.defaults().to_vec();
        taken
    }

    /// The actions whose keys differ from the defaults, a line each:
    /// `id<TAB>chord chord` (nothing after the tab for no keys).
    pub fn to_setting(&self) -> String {
        Action::ALL
            .iter()
            .filter(|a| self.chords(**a) != a.defaults())
            .map(|a| {
                let chords: Vec<String> = self.chords(*a).iter().map(Chord::name).collect();
                format!("{}\t{}", a.id(), chords.join(" "))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Read what `to_setting` wrote; unknown actions and chords are left
    /// out, a chord given to two actions stays with the first.
    pub fn from_setting(text: &str) -> Self {
        let mut map = Self::default();
        let mut changed = Vec::new();
        for line in text.lines() {
            let Some((id, chords)) = line.split_once('\t') else {
                continue;
            };
            let Some(action) = Action::from_id(id.trim()) else {
                continue;
            };
            if changed.contains(&action) {
                continue;
            }
            let mut mine: Vec<Chord> = Vec::new();
            for chord in chords.split_whitespace().filter_map(Chord::from_name) {
                let taken = changed.iter().any(|a| map.chords(*a).contains(&chord));
                if !taken && !mine.contains(&chord) {
                    mine.push(chord);
                }
            }
            map.chords[action.index()] = mine;
            changed.push(action);
        }
        // A default chord of an action not named may have been given to
        // one that is: the named one keeps it.
        for action in Action::ALL {
            if changed.contains(action) {
                continue;
            }
            let taken: Vec<Chord> = changed
                .iter()
                .flat_map(|a| map.chords(*a).to_vec())
                .collect();
            map.chords[action.index()].retain(|c| !taken.contains(c));
        }
        map
    }

    /// The command of `chord`; `repeat` is a key held down, which repeats
    /// only what [`Action::repeats`] says.
    pub fn command(&self, chord: Chord, repeat: bool) -> Option<Cmd> {
        let action = self.action(chord)?;
        (!repeat || action.repeats()).then(|| action.cmd())
    }
}

/// The commands of this frame's keys and of the mouse's side buttons, in
/// order, of the chords `allowed` lets act; their events are taken out,
/// so the lists and fields do not see them (a held key's repeat of a
/// chord that does not repeat too).
pub fn take(ctx: &egui::Context, map: &Keymap, allowed: impl Fn(&Chord) -> bool) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    ctx.input_mut(|i| {
        let modifiers = i.modifiers;
        i.events.retain(|e| {
            let Some((chord, repeat)) = Chord::of_event(e, modifiers) else {
                return true;
            };
            if !allowed(&chord) || map.action(chord).is_none() {
                return true;
            }
            if let Some(cmd) = map.command(chord, repeat) {
                cmds.push(cmd);
            }
            // A side button's release, which egui pairs with the press,
            // stays; a key's release is no chord.
            matches!(e, egui::Event::PointerButton { .. })
        });
    });
    cmds
}

/// The keys in use, for the labels of the tooltips.
static CURRENT: LazyLock<RwLock<Keymap>> = LazyLock::new(|| RwLock::new(Keymap::default()));

/// Make `map` the one the labels show.
pub fn publish(map: &Keymap) {
    if let Ok(mut current) = CURRENT.write() {
        current.clone_from(map);
    }
}

/// The label of the first key of the action that does `cmd`, or nothing.
pub fn shortcut(cmd: Cmd) -> String {
    let Ok(map) = CURRENT.read() else {
        return String::new();
    };
    Action::ALL
        .iter()
        .find(|a| a.cmd() == cmd)
        .and_then(|a| map.chords(*a).first())
        .map(Chord::label)
        .unwrap_or_default()
}

/// `text` with the key of `cmd` in brackets after it, if it has one.
pub fn with_key(text: &str, cmd: Cmd) -> String {
    let key = shortcut(cmd);
    if key.is_empty() {
        text.to_owned()
    } else {
        format!("{text} ({key})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_and_cut_keep_their_modifiers() {
        let name = |event, m| Chord::of_event(&event, m).map(|(c, _)| c.name());
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(
            name(egui::Event::Copy, Modifiers::CTRL).as_deref(),
            Some("Ctrl+C")
        );
        assert_eq!(
            name(egui::Event::Copy, ctrl_shift).as_deref(),
            Some("Ctrl+Shift+C")
        );
        assert_eq!(
            name(egui::Event::Cut, Modifiers::SHIFT).as_deref(),
            Some("Shift+Delete")
        );
    }

    #[test]
    fn ids_and_defaults_are_unique() {
        let mut ids = std::collections::HashSet::new();
        let mut chords = std::collections::HashSet::new();
        for a in Action::ALL {
            assert!(ids.insert(a.id()), "{} twice", a.id());
            for c in a.defaults() {
                assert!(chords.insert(*c), "{} twice", c.name());
            }
        }
    }

    #[test]
    fn names_round_trip() {
        for key in Key::ALL {
            let chord = Chord::new(Press::Key(*key), Modifiers::NONE);
            assert_eq!(Chord::from_name(&chord.name()), Some(chord), "{key:?}");
        }
        for name in ["Ctrl+Shift+Left", "Alt+MouseBack", "MouseForward", "Alt+3"] {
            assert_eq!(Chord::from_name(name).unwrap().name(), name);
        }
        assert_eq!(Chord::from_name("Ctrl+"), None);
        assert_eq!(Chord::from_name("Ctrl+Nothing"), None);
    }

    #[test]
    fn labels_are_in_the_font() {
        // The program's font has ASCII and Cyrillic, no arrows or symbols.
        for key in Key::ALL {
            let label = Chord::key(*key).label();
            assert!(
                label.chars().all(|c| c.is_ascii()
                    || ('а'..='я').contains(&c.to_lowercase().next().unwrap())
                    || c == 'ё'),
                "{key:?}: {label}"
            );
        }
    }

    #[test]
    fn modifiers_alone_are_no_chord() {
        assert_eq!(Chord::of_key(Key::ShiftLeft, Modifiers::SHIFT), None);
        assert_eq!(
            Chord::of_key(Key::A, Modifiers::COMMAND),
            Some(Chord::ctrl(Key::A))
        );
    }

    #[test]
    fn what_a_text_field_keeps() {
        // Typing and moving in the field.
        assert!(Chord::key(Key::Space).edits_text());
        assert!(Chord::key(Key::I).edits_text());
        assert!(Chord::shift(Key::ArrowLeft).edits_text());
        assert!(Chord::ctrl(Key::ArrowLeft).edits_text());
        assert!(Chord::ctrl(Key::C).edits_text());
        // The program's.
        assert!(!Chord::key(Key::F5).edits_text());
        assert!(!Chord::alt(Key::E).edits_text());
        assert!(!Chord::ctrl(Key::F).edits_text());
        assert!(!Chord::of(Press::MouseBack, false, false, false).edits_text());
        // Letters and digits type; Space, Shift and Ctrl chords do not.
        assert!(Chord::key(Key::S).types());
        assert!(Chord::key(Key::Num7).types());
        assert!(Chord::key(Key::Minus).types());
        assert!(!Chord::key(Key::Space).types());
        assert!(!Chord::shift(Key::V).types());
        assert!(!Chord::ctrl(Key::F).types());
        assert!(!Chord::key(Key::F1).types());
    }

    #[test]
    fn assigning_takes_the_chord_from_another() {
        let mut map = Keymap::default();
        let space = Chord::key(Key::Space);
        assert_eq!(
            map.assign(Action::Stop, None, space),
            Some(Action::PlayPause)
        );
        assert_eq!(map.action(space), Some(Action::Stop));
        assert_eq!(map.chords(Action::PlayPause), [Chord::key(Key::C)]);
        // In place of the second key.
        let g = Chord::key(Key::G);
        assert_eq!(map.assign(Action::Info, Some(1), g), None);
        assert_eq!(map.chords(Action::Info), [Chord::key(Key::I), g]);
        map.remove(Action::Info, 0);
        assert_eq!(map.chords(Action::Info), [g]);
    }

    #[test]
    fn an_action_gets_its_keys_back() {
        let mut map = Keymap::default();
        let space = Chord::key(Key::Space);
        map.assign(Action::Equalizer, None, space);
        assert!(!map.is_default_for(Action::PlayPause));
        // Space comes back from the equalizer, which keeps no other key.
        assert_eq!(map.reset(Action::PlayPause), [(space, Action::Equalizer)]);
        assert!(map.is_default());
    }

    #[test]
    fn only_changes_are_kept() {
        assert_eq!(Keymap::default().to_setting(), "");
        let mut map = Keymap::default();
        map.assign(Action::Equalizer, None, Chord::key(Key::Space));
        map.remove(Action::Info, 1);
        let text = map.to_setting();
        assert_eq!(text, "play_pause\tC\nequalizer\tSpace\ninfo\tI");
        assert_eq!(Keymap::from_setting(&text), map);
        assert!(Keymap::from_setting("").is_default());
        assert!(Keymap::from_setting("nothing\tSpace\nplay_pause\tSpace C Bad+Key").is_default());
        // Kept in the settings' JSON as that text.
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(serde_json::from_str::<Keymap>(&json).unwrap(), map);
    }

    #[test]
    fn a_named_action_keeps_a_default_chord() {
        // Written by a version that had no default of Space for play/pause.
        let map = Keymap::from_setting("stop\tSpace");
        assert_eq!(map.action(Chord::key(Key::Space)), Some(Action::Stop));
        assert_eq!(map.chords(Action::PlayPause), [Chord::key(Key::C)]);
    }

    #[test]
    fn held_keys_repeat_only_the_seek_and_the_volume() {
        let map = Keymap::default();
        assert_eq!(
            map.command(Chord::shift(Key::ArrowLeft), true),
            Some(Cmd::Seek(-1))
        );
        assert_eq!(
            map.command(Chord::ctrl(Key::ArrowUp), true),
            Some(Cmd::Volume(VOLUME_STEP))
        );
        assert_eq!(map.command(Chord::key(Key::Space), true), None);
        assert_eq!(
            map.command(Chord::key(Key::Space), false),
            Some(Cmd::PlayPause)
        );
    }

    #[test]
    fn shortcut_labels() {
        publish(&Keymap::default());
        assert_eq!(shortcut(Cmd::Open), "Ctrl+O");
        assert_eq!(shortcut(Cmd::Equalizer), "");
        assert!(["Ctrl+Left", "Ctrl+Влево"].contains(&shortcut(Cmd::Previous).as_str()));
        assert_eq!(with_key("Settings", Cmd::Settings), "Settings (F5)");
        assert_eq!(with_key("Equalizer", Cmd::Equalizer), "Equalizer");
    }
}
