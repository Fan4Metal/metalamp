//! The egui window: the eframe `App`. The frameless window with its own
//! title row (`ui/title.rs`), About, the update check and the settings;
//! the library, the player and the playlist come in their own modules.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};

use crate::library::Library;
use crate::library::scan::{Scan, ScanMsg, Wake};
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
    /// When the window last moved or changed size, or a setting changed,
    /// until the settings are saved.
    place_changed: Option<Instant>,
    /// The app icon of the start screen and of the title row.
    start_icon: Option<egui::TextureHandle>,
    title_icon: Option<egui::TextureHandle>,
    /// The library as last loaded or scanned.
    pub(crate) library: Arc<Library>,
    /// The scan under way, if any.
    scan: Option<Scan>,
    /// Wakes the UI from the scan's thread.
    wake: Wake,
    /// Tags read and of how many, while a scan reads them.
    pub(crate) scan_progress: Option<(usize, usize)>,
    /// The index has been read (the library shown is not just empty).
    pub(crate) loaded: bool,
    pub(crate) library_view: crate::ui::library::LibraryView,
    /// The right button is down (its press held back, `raw_input_hook`).
    right_held: bool,
    pub(crate) player: crate::player::Player,
    /// When files last came from another start (Explorer starts one per
    /// file of a selection: those within 300 ms make one playlist).
    last_received: Option<Instant>,
    playlist_view: crate::ui::playlist::PlaylistView,
    /// The playlist's revision and current entry last saved.
    saved_playlist: (u64, Option<usize>),
    /// The file info window (I, Alt+3), and a file to open it for at the
    /// end of the frame.
    file_info: crate::ui::info::InfoView,
    info_request: Option<PathBuf>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        settings: Settings,
        scan: Scan,
        wake: Wake,
        files: Vec<PathBuf>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        let _ = wake.set(ctx.clone());
        // Before any text is made.
        crate::i18n::set_lang(settings.language.resolve());
        palette::apply(ctx);
        ctx.options_mut(|o| {
            // Ctrl+plus and Ctrl+minus are the app's, not egui's zoom.
            o.zoom_with_keyboard = false;
            o.input_options.max_double_click_delay = win::double_click_time();
            // A wheel notch scrolls 100 points instead of egui's 40 (as
            // qview and metalmd; Windows and Chromium scroll about that).
            o.input_options.line_scroll_speed = 100.0;
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
            crate::instance::listen(ctx, hwnd);
        }

        let player = crate::player::Player::new(
            ctx,
            settings.volume,
            settings.mute,
            settings.repeat,
            settings.shuffle,
        );
        let mut player = player;
        let saved_playlist = load_playlist(&mut player, settings.playlist_current);
        // Files on the command line (Explorer's Open) play at once.
        let files = expand_playlists(files);
        if !files.is_empty() {
            player.play_paths(files, 0);
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
            library: Arc::new(Library::default()),
            scan: Some(scan),
            wake,
            scan_progress: None,
            loaded: false,
            library_view: crate::ui::library::LibraryView::new(ctx),
            right_held: false,
            player,
            last_received: None,
            playlist_view: crate::ui::playlist::PlaylistView::default(),
            saved_playlist,
            file_info: crate::ui::info::InfoView::default(),
            info_request: None,
        }
    }

    /// The scan's news: the library from the index, the progress, the
    /// library as the folders hold it.
    fn poll_scan(&mut self) {
        let Some(scan) = &self.scan else { return };
        let mut finished = false;
        for msg in scan.poll() {
            match msg {
                ScanMsg::Loaded(library) => {
                    self.library = library;
                    self.loaded = true;
                }
                ScanMsg::Progress { done, total } => self.scan_progress = Some((done, total)),
                ScanMsg::Done(scanned) => {
                    log::info!(
                        "library: {} tracks, {} read, in {} ms",
                        scanned.library.tracks.len(),
                        scanned.read,
                        scanned.took.as_millis()
                    );
                    self.library = scanned.library;
                    self.loaded = true;
                    finished = true;
                }
            }
        }
        if finished {
            self.scan = None;
            self.scan_progress = None;
        }
    }

    /// Scan the library's folders again (F5, a folder added or removed),
    /// keeping the tags of unchanged files. A scan under way is dropped.
    pub(crate) fn rescan(&mut self) {
        self.scan_progress = None;
        self.scan = Some(Scan::start(
            self.settings.library_folders.clone(),
            Some(Arc::clone(&self.library)),
            false,
            Arc::clone(&self.wake),
        ));
    }

    pub(crate) fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// Add `folder` to the library (not one already in it, inside one of
    /// its folders) and scan.
    pub(crate) fn add_folder(&mut self, folder: PathBuf) {
        let inside = |a: &std::path::Path, b: &std::path::Path| {
            let (a, b) = (
                a.to_string_lossy().to_lowercase(),
                b.to_string_lossy().to_lowercase(),
            );
            a == b || a.starts_with(&format!("{}\\", b.trim_end_matches('\\')))
        };
        if self
            .settings
            .library_folders
            .iter()
            .any(|f| inside(&folder, f))
        {
            return;
        }
        // A folder holding ones already listed takes their place.
        self.settings
            .library_folders
            .retain(|f| !inside(f, &folder));
        self.settings.library_folders.push(folder);
        self.sync_settings();
        self.settings.save_in_background();
        self.rescan();
    }

    fn library_action(&mut self, action: crate::ui::library::LibraryAction, frame: &eframe::Frame) {
        use crate::ui::library::LibraryAction;
        match action {
            LibraryAction::Rescan => self.rescan(),
            LibraryAction::AddFolder => self.pick_folder(frame),
            LibraryAction::RemoveFolder(i) => {
                if i < self.settings.library_folders.len() {
                    self.settings.library_folders.remove(i);
                    self.sync_settings();
                    self.settings.save_in_background();
                    self.rescan();
                }
            }
            LibraryAction::Play { tracks, start } => {
                let paths = self.paths_of(&tracks);
                self.player.play_paths(paths, start);
            }
            LibraryAction::Enqueue(tracks) => {
                let paths = self.paths_of(&tracks);
                self.player.enqueue(paths);
            }
            LibraryAction::Info(track) => {
                if let Some(t) = self.library.tracks.get(track as usize) {
                    self.info_request = Some(t.path.clone());
                }
            }
        }
    }

    fn paths_of(&self, tracks: &[crate::library::TrackId]) -> Vec<PathBuf> {
        tracks
            .iter()
            .filter_map(|&t| self.library.tracks.get(t as usize))
            .map(|t| t.path.clone())
            .collect()
    }

    /// The library's track heard, if it is one.
    fn playing_track(&self) -> Option<crate::library::TrackId> {
        let path = self.player.playing_path()?;
        self.library.by_path.get(path).copied()
    }

    /// The title and the second line of the player bar: the track heard,
    /// else the playlist's current one; from the library, else the file's
    /// name.
    fn now_playing(&self) -> (String, String, Option<crate::library::TrackId>) {
        let path = self
            .player
            .playing_path()
            .or_else(|| self.player.queue.current_entry().map(|e| &e.path));
        let Some(path) = path else {
            return (String::new(), String::new(), None);
        };
        match self.library.by_path.get(path) {
            Some(&id) => {
                let t = &self.library.tracks[id as usize];
                let artist = if t.artist.is_empty() {
                    crate::library::unknown_artist().to_owned()
                } else {
                    t.artist.clone()
                };
                let mut second = artist;
                if !t.album.is_empty() {
                    second += &format!(" — {}", t.album);
                    if t.year > 0 {
                        second += &format!(" ({})", t.year);
                    }
                }
                (t.title.clone(), second, Some(id))
            }
            None => (
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path.parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                None,
            ),
        }
    }

    /// The file I or Alt+3 shows: the one selected last, in the playlist when
    /// it was clicked last, else in the library's tracks; else the one
    /// playing (or the playlist's current one).
    fn info_path(&self) -> Option<PathBuf> {
        let entries = self.player.queue.entries();
        if self.playlist_view.focused
            && let Some(e) = self
                .playlist_view
                .selected_entry()
                .and_then(|i| entries.get(i))
        {
            return Some(e.path.clone());
        }
        if let Some(t) = self.library_view.selected_track() {
            return self.library.tracks.get(t as usize).map(|t| t.path.clone());
        }
        self.player
            .playing_path()
            .or_else(|| self.player.queue.current_entry().map(|e| &e.path))
            .cloned()
    }

    /// The path of the track heard, else of the playlist's current one.
    fn playing_or_current_path(&self) -> Option<PathBuf> {
        self.player
            .playing_path()
            .or_else(|| self.player.queue.current_entry().map(|e| &e.path))
            .cloned()
    }

    /// The cover of the track heard (else of the playlist's current one),
    /// when it is in the library.
    fn playing_cover(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let (_, _, track) = self.now_playing();
        let t = track?;
        let album = &self.library.albums[self.library.album_of[t as usize] as usize];
        self.library_view.covers.get(ctx, &self.library, album)
    }

    fn player_bar(&mut self, ui: &mut egui::Ui) {
        use crate::ui::player_bar::{self, BarCmd, NowPlaying};
        let ctx = ui.ctx().clone();
        let (title, artist, _) = self.now_playing();
        let cover = self.playing_cover(&ctx);
        let now = NowPlaying {
            status: self.player.status,
            title: &title,
            artist: &artist,
            position_ms: self.player.position_ms(),
            duration_ms: self.player.duration_ms(),
            volume: self.player.volume,
            mute: self.player.mute,
            shuffle: self.player.queue.shuffle(),
            repeat: self.player.queue.repeat,
            cover,
        };
        for cmd in player_bar::show(ui, &now) {
            match cmd {
                BarCmd::Previous => self.player.previous(),
                BarCmd::PlayPause => self.player.toggle_pause(),
                BarCmd::Stop => self.player.stop(),
                BarCmd::Next => self.player.next(),
                BarCmd::Shuffle => {
                    let on = !self.player.queue.shuffle();
                    self.player.set_shuffle(on);
                    self.place_changed = Some(Instant::now());
                }
                BarCmd::Repeat => {
                    let next = self.player.queue.repeat.next();
                    self.player.set_repeat(next);
                    self.place_changed = Some(Instant::now());
                }
                BarCmd::Mute => {
                    self.player.toggle_mute();
                    self.place_changed = Some(Instant::now());
                }
                BarCmd::Volume(v) => {
                    self.player.set_volume(v);
                    self.place_changed = Some(Instant::now());
                }
                BarCmd::Seek(ms) => self.player.seek(ms),
                BarCmd::Info => self.info_request = self.playing_or_current_path(),
            }
        }
    }

    /// The player's keys, when no text field takes them: Space, Z X C V B
    /// (Winamp's), Ctrl+arrows, Shift+arrows, M, R, S.
    fn player_keys(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if pressed(Modifiers::NONE, Key::Space) || pressed(Modifiers::NONE, Key::C) {
            self.player.toggle_pause();
        }
        if pressed(Modifiers::NONE, Key::X) {
            self.player.play();
        }
        if pressed(Modifiers::NONE, Key::V) {
            self.player.stop();
        }
        if pressed(Modifiers::COMMAND, Key::ArrowRight) || pressed(Modifiers::NONE, Key::B) {
            self.player.next();
        }
        if pressed(Modifiers::COMMAND, Key::ArrowLeft) || pressed(Modifiers::NONE, Key::Z) {
            self.player.previous();
        }
        if pressed(Modifiers::SHIFT, Key::ArrowRight) {
            self.player.seek_by(5);
        }
        if pressed(Modifiers::SHIFT, Key::ArrowLeft) {
            self.player.seek_by(-5);
        }
        let mut volume = None;
        if pressed(Modifiers::COMMAND, Key::ArrowUp) {
            volume = Some(self.player.volume.saturating_add(5).min(100));
        }
        if pressed(Modifiers::COMMAND, Key::ArrowDown) {
            volume = Some(self.player.volume.saturating_sub(5));
        }
        if let Some(v) = volume {
            self.player.set_volume(v);
            self.place_changed = Some(Instant::now());
        }
        if pressed(Modifiers::NONE, Key::M) {
            self.player.toggle_mute();
            self.place_changed = Some(Instant::now());
        }
        if pressed(Modifiers::NONE, Key::R) {
            let next = self.player.queue.repeat.next();
            self.player.set_repeat(next);
            self.place_changed = Some(Instant::now());
        }
        if pressed(Modifiers::NONE, Key::S) {
            let on = !self.player.queue.shuffle();
            self.player.set_shuffle(on);
            self.place_changed = Some(Instant::now());
        }
    }

    /// The window's title: the track heard, as Winamp shows it.
    fn window_title(&self) -> String {
        let (title, _, track) = self.now_playing();
        if self.player.status == crate::player::Status::Stopped || title.is_empty() {
            return crate::APP_NAME.to_owned();
        }
        let artist = track
            .map(|t| self.library.tracks[t as usize].artist.clone())
            .filter(|a| !a.is_empty());
        match artist {
            Some(a) => format!("{a} – {title} — {}", crate::APP_NAME),
            None => format!("{title} — {}", crate::APP_NAME),
        }
    }

    /// Files a later start handed over: they replace the playlist and play,
    /// except those coming within 300 ms of the last ones (the rest of an
    /// Explorer selection), which are added.
    fn take_handed_over(&mut self) {
        let received = crate::instance::take();
        let Some(&(_, first_at)) = received.first() else {
            return;
        };
        let batch = self.last_received.is_some_and(|last| {
            first_at.saturating_duration_since(last) < Duration::from_millis(300)
        });
        self.last_received = received.last().map(|&(_, at)| at);
        let files = expand_playlists(received.into_iter().map(|(p, _)| p).collect());
        if files.is_empty() {
            return;
        }
        if batch {
            self.player.enqueue(files);
        } else {
            self.player.play_paths(files, 0);
        }
    }

    /// Open files to play (Ctrl+O): they replace the playlist.
    fn open_files(&mut self, frame: &eframe::Frame) {
        let mut extensions: Vec<&str> = crate::filetypes::AUDIO_EXTENSIONS.to_vec();
        extensions.extend(["m3u8", "m3u"]);
        let dialog = rfd::FileDialog::new()
            .set_title(tr!("Open files", "Открыть файлы"))
            .add_filter(tr!("Audio and playlists", "Аудио и плейлисты"), &extensions)
            .set_parent(frame);
        if let Some(files) = dialog.pick_files() {
            let files = expand_playlists(files);
            self.player.play_paths(files, 0);
        }
    }

    /// Ask for a folder to add (Ctrl+Shift+O).
    fn pick_folder(&mut self, frame: &eframe::Frame) {
        let dialog = rfd::FileDialog::new()
            .set_title(tr!("Add a music folder", "Добавить папку с музыкой"))
            .set_parent(frame);
        if let Some(folder) = dialog.pick_folder() {
            self.add_folder(folder);
        }
    }

    /// Folders dropped on the window go into the library.
    fn take_dropped(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        let mut files = Vec::new();
        for path in dropped {
            if path.is_dir() {
                self.add_folder(path);
            } else {
                files.push(path);
            }
        }
        // Files dropped go to the playlist (played if nothing plays).
        let files = expand_playlists(files);
        if !files.is_empty() {
            self.player.enqueue(files);
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
        self.settings.volume = self.player.volume;
        self.settings.mute = self.player.mute;
        self.settings.repeat = self.player.queue.repeat;
        self.settings.shuffle = self.player.queue.shuffle();
        self.settings.playlist_current = self.player.queue.current();
    }

    /// Write the playlist when it or its current entry changed.
    fn remember_playlist(&mut self, now: bool) {
        let state = (self.player.queue.revision, self.player.queue.current());
        if state == self.saved_playlist {
            return;
        }
        let revision_changed = state.0 != self.saved_playlist.0;
        self.saved_playlist = state;
        if revision_changed {
            let text = self.player.queue.m3u8(&|path| {
                let &id = self.library.by_path.get(path)?;
                let t = &self.library.tracks[id as usize];
                let title = if t.artist.is_empty() {
                    t.title.clone()
                } else {
                    format!("{} - {}", t.artist, t.title)
                };
                Some((u64::from(t.duration_ms) / 1000, title))
            });
            if let Some(path) = playlist_file() {
                let write = move || {
                    let tmp = path.with_extension("m3u8.tmp");
                    if let Err(e) =
                        std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path))
                    {
                        log::warn!("cannot write the playlist: {e}");
                    }
                };
                if now {
                    write();
                } else {
                    std::thread::spawn(write);
                }
            }
        }
        // The current entry is a setting.
        self.place_changed.get_or_insert_with(Instant::now);
    }

    fn save_now(&mut self) {
        self.remember_playlist(true);
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
                win::Hooked::Media { key, from_command } => {
                    // With the window in front a press comes as the command
                    // and then as the key: the key alone counts then.
                    if from_command && ctx.input(|i| i.focused) {
                        continue;
                    }
                    match key {
                        win::MediaKey::PlayPause => self.player.toggle_pause(),
                        win::MediaKey::Play => self.player.play(),
                        win::MediaKey::Pause => {
                            if self.player.status == crate::player::Status::Playing {
                                self.player.toggle_pause();
                            }
                        }
                        win::MediaKey::Stop => self.player.stop(),
                        win::MediaKey::Next => self.player.next(),
                        win::MediaKey::Previous => self.player.previous(),
                    }
                }
            }
        }
    }

    fn title_row(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let icon = crate::ui::app_icon(&ctx, 18.0, &mut self.title_icon);
        let update = self.updates.newer().map(str::to_owned);
        let maximized = frameless::is_maximized(&ctx);
        let title = self.window_title();
        match title::row(ui, &title, maximized, &icon, update.as_deref()) {
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
    /// The right button's press is held back until it is let go and then
    /// given to egui with the release, where it was let go: egui counts a
    /// press and release as a click only within 6 points and 0.8 s, and a
    /// context menu opened by a click with the mouse on the move was lost
    /// (Windows opens menus on the release, wherever it is; qview's way).
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        let events = std::mem::take(&mut raw.events);
        for event in events {
            match event {
                egui::Event::PointerButton {
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    ..
                } => self.right_held = true,
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers,
                } => {
                    if std::mem::take(&mut self.right_held) {
                        raw.events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed: true,
                            modifiers,
                        });
                    }
                    raw.events.push(event);
                }
                other => raw.events.push(other),
            }
        }
    }

    /// Every frame, also while the window is minimised.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.updates.poll();
        self.poll_scan();
        self.library_view.covers.poll(ctx);
        self.take_hooked(ctx);
        self.take_handed_over();
        self.remember_playlist(false);
        if self.player.poll().track {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));
        }
        if self.player.status == crate::player::Status::Playing {
            // The position moves on: a few repaints a second.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.remember_place();
    }

    // egui 0.36: `ui` with the root `Ui`, not `update(ctx)`; panels are
    // `egui::Panel::top/left/right/bottom(id).show(ui, …)` and
    // `CentralPanel::default()` / `CentralPanel::no_frame()`.
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.uncloak(&ctx);
        // A modal takes the keys while it is open.
        let modal = self.about.open || self.file_info.open;
        // The file info: Alt+3 as Winamp's, or I (not while typing).
        let typing = ctx.memory(|m| m.focused().is_some());
        if !modal
            && ctx.input_mut(|i| {
                i.consume_key(Modifiers::ALT, Key::Num3)
                    || (!typing && i.consume_key(Modifiers::NONE, Key::I))
            })
        {
            self.info_request = self.info_path();
        }
        if !modal {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F1)) {
                self.about.toggle();
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::O)) {
                self.pick_folder(frame);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F5)) {
                self.rescan();
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
                self.library_view.focus_search = true;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O)) {
                self.open_files(frame);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::E)) {
                self.settings.playlist_visible = !self.settings.playlist_visible;
                self.place_changed = Some(Instant::now());
            }
        }
        self.take_dropped(&ctx);
        if !modal {
            self.player_keys(&ctx);
        }
        if !frameless::is_maximized(&ctx) {
            frameless::resize_edges(root_ui);
        }
        let title_rect = egui::Panel::top("title")
            .exact_size(title::HEIGHT)
            .frame(egui::Frame::NONE)
            .show(root_ui, |ui| self.title_row(ui))
            .response
            .rect;
        let bar_rect = egui::Panel::top("player")
            .exact_size(crate::ui::player_bar::HEIGHT)
            .frame(egui::Frame::NONE)
            .show(root_ui, |ui| self.player_bar(ui))
            .response
            .rect;
        // The wheel anywhere over the title row and the player bar steps
        // the volume by 5, as over Winamp's main window.
        if !modal
            && ctx
                .pointer_hover_pos()
                .is_some_and(|p| title_rect.union(bar_rect).contains(p))
        {
            let notches = crate::ui::wheel_notches(root_ui);
            if notches != 0 {
                let v = (i32::from(self.player.volume) + notches * 5).clamp(0, 100) as u8;
                self.player.set_volume(v);
                self.place_changed = Some(Instant::now());
            }
        }
        if self.settings.playlist_visible && !self.settings.library_folders.is_empty() {
            let cover = self.playing_cover(&ctx);
            let shown_cover = self.settings.playlist_cover;
            let panel = egui::Panel::right("playlist")
                .resizable(true)
                .default_size(self.settings.playlist_width)
                .size_range(220.0..=700.0)
                .frame(egui::Frame::NONE)
                .show(root_ui, |ui| {
                    crate::ui::playlist::show(
                        ui,
                        &mut self.playlist_view,
                        &mut self.player,
                        &self.library,
                        !modal,
                        cover.as_ref(),
                        &mut self.settings.playlist_cover,
                    )
                });
            if self.settings.playlist_cover != shown_cover {
                self.place_changed = Some(Instant::now());
            }
            let width = panel.response.rect.width();
            if (width - self.settings.playlist_width).abs() > 0.5 {
                self.settings.playlist_width = width;
                self.place_changed = Some(Instant::now());
            }
            // A press inside the playlist gives it the keys, elsewhere
            // takes them back.
            if ctx.input(|i| i.pointer.any_pressed())
                && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
            {
                self.playlist_view.focused = panel.response.rect.contains(p);
            }
            for action in panel.inner {
                match action {
                    crate::ui::playlist::PlaylistAction::Play(i) => self.player.play_index(i),
                    crate::ui::playlist::PlaylistAction::ShowInFolder(i) => {
                        if let Some(e) = self.player.queue.entries().get(i) {
                            win::show_in_folder(&e.path);
                        }
                    }
                    crate::ui::playlist::PlaylistAction::Info(i) => {
                        self.info_request =
                            self.player.queue.entries().get(i).map(|e| e.path.clone());
                    }
                    crate::ui::playlist::PlaylistAction::InfoPlaying => {
                        self.info_request = self.playing_or_current_path();
                    }
                }
            }
        }
        egui::Panel::bottom("status")
            .exact_size(crate::ui::status::HEIGHT)
            .frame(egui::Frame::NONE.fill(palette::TITLE_BG))
            .show(root_ui, |ui| crate::ui::status::show(ui, self));
        egui::CentralPanel::no_frame().show(root_ui, |ui| {
            if !self.settings.library_folders.is_empty() {
                let scanning = self.scanning();
                let playing = self.playing_track();
                let (actions, changed) = crate::ui::library::show(
                    ui,
                    &mut self.library_view,
                    &self.library,
                    &mut self.settings,
                    scanning,
                    playing,
                    !self.playlist_view.focused && !modal,
                );
                if changed {
                    self.place_changed = Some(Instant::now());
                }
                for action in actions {
                    self.library_action(action, frame);
                }
                return;
            }
            let hint = tr!(
                "Add a music folder with Ctrl+Shift+O or drop it here",
                "Добавьте папку с музыкой: Ctrl+Shift+O или перетащите её сюда"
            );
            crate::ui::start::show(ui, &mut self.start_icon, hint);
        });
        if let Some(path) = self.info_request.take() {
            self.file_info.open(&ctx, &path);
        }
        self.file_info.show(&ctx);
        self.about.show(&ctx, &mut self.updates);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        crate::instance::closing();
        self.save_now();
    }
}

/// `playlist.m3u8` next to the settings.
fn playlist_file() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("playlist.m3u8"))
}

/// The playlist of the last run, its current entry as it was; the state
/// to compare later saves with.
fn load_playlist(
    player: &mut crate::player::Player,
    current: Option<usize>,
) -> (u64, Option<usize>) {
    if let Some(path) = playlist_file()
        && let Ok(text) = std::fs::read_to_string(&path)
    {
        let folder = path.parent().map(PathBuf::from).unwrap_or_default();
        player
            .queue
            .replace(crate::queue::m3u_paths(&text, &folder));
        player.queue.set_current(current);
    }
    (player.queue.revision, player.queue.current())
}

/// Playlists among `files` replaced by their entries; only audio files
/// stay.
fn expand_playlists(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for f in files {
        let ext = f
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext == "m3u8" || ext == "m3u" {
            if let Ok(bytes) = std::fs::read(&f) {
                let text = String::from_utf8_lossy(&bytes);
                let folder = f.parent().map(PathBuf::from).unwrap_or_default();
                out.extend(crate::queue::m3u_paths(&text, &folder));
            }
        } else if crate::filetypes::is_audio(&f) {
            out.push(f);
        }
    }
    out
}
