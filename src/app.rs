//! The egui window: the eframe `App`. The frameless window with its own
//! title row (`ui/title.rs`), About, the update check and the settings;
//! the library, the player and the playlist come in their own modules.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};

use crate::library::Library;
use crate::library::scan::{Scan, ScanMsg, Wake};
use crate::settings::{Settings, WindowLayout, WindowPlace};
use crate::ui::about::AboutDialog;
use crate::ui::settings::SettingsDialog;
use crate::ui::title::{self, TitleCmd};
use crate::update::Updates;
use crate::{frameless, palette, win};

/// How long the window stays cloaked at most while it is being maximized.
const UNCLOAK_TIMEOUT: Duration = Duration::from_secs(1);
/// Frames the window paints, cloaked, before it is shown.
const PAINTED_TO_SHOW: u64 = 3;
/// How long the window must rest after a move or resize before its place
/// is saved: not at every frame of a drag, and not only at exit, which a
/// crash or a logoff may skip.
const PLACE_SAVE_DELAY: Duration = Duration::from_secs(1);

pub struct App {
    settings: Settings,
    settings_dialog: SettingsDialog,
    /// The player bar's visualization and the samples it is fed.
    visual: crate::ui::visual::Visualizer,
    heard: Vec<f32>,
    /// The title row and the player bar, as the frame before laid them
    /// out: with the pointer there the arrows seek and set the volume
    /// (the keys are read before the panels are laid out).
    player_area: egui::Rect,
    about: AboutDialog,
    updates: Updates,
    /// The window's handle; `None` off Windows.
    hwnd: Option<isize>,
    /// The taskbar button's progress and thumbnail buttons.
    taskbar: Option<crate::taskbar::Taskbar>,
    /// Windows' media controls, while the media keys are taken always;
    /// `smtc_failed`: Windows has none, not asked again.
    smtc: Option<crate::smtc::Smtc>,
    smtc_failed: bool,
    /// The last media key: one press may come more than one way.
    last_media: Option<(win::MediaKey, win::Via, Instant)>,
    /// The window is cloaked until it has painted a few frames (else it
    /// flashed white as it showed), and been maximized when it is to be:
    /// the time it is shown anyway, the frame it was cloaked in, maximized.
    cloak: Option<(Instant, u64, bool)>,
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
    /// The library's folders watched, and when they last changed (a scan
    /// follows once they have rested for `watch::QUIET`).
    watcher: crate::library::watch::Watcher,
    watch_at: Option<Instant>,
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
    /// The editor of the library's views.
    view_editor: crate::ui::view_editor::ViewEditor,
    /// The plays and ratings, and the clock of the track heard.
    pub(crate) stats: crate::stats::Stats,
    listening: crate::stats::Listening,
    /// The tag editor.
    tag_editor: crate::ui::tag_editor::TagEditor,
    /// Over the others and the mini player as last applied to the window
    /// (`None`: not yet).
    applied_modes: Option<(bool, bool)>,
    /// The mini player's cover, read large, and the app icon for a track
    /// without one.
    mini_cover: crate::ui::mini::Cover,
    mini_icon: Option<egui::TextureHandle>,
    /// Back from the mini player, the window not grown yet: the whole
    /// window's panels wait, else the playlist and the library's panes
    /// took the mini player's size as their own (at most 1 s).
    growing: Option<Instant>,
    /// The window's layout as last applied (`apply_layout`).
    applied_layout: WindowLayout,
    /// The tray icon while the setting is on, and whether the window's
    /// taskbar button is taken away (minimized to it).
    tray: Option<crate::tray::Tray>,
    off_taskbar: bool,
    /// The equalizer's window (the player bar's button).
    eq_window: crate::ui::eq_window::EqWindow,
    /// `METALAMP_VIEW_EDITOR` is set: open the editor once the library is
    /// loaded, for looking at it without a hand on the window (GUI checks).
    env_view_editor: bool,
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
        crate::keymap::publish(&settings.keys);
        palette::set(settings.look());
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
            crate::caption::set_dark(hwnd, settings.theme.is_dark());
            win::round_corners(hwnd);
            // The mini player at start: the whole window is placed hidden
            // and not maximized, and the mini player put over it before
            // it shows (`apply_modes`).
            let maximized = settings.place().is_some_and(|w| w.maximized) && !settings.mini;
            win::cloak(hwnd, true);
            cloak = Some((
                Instant::now() + UNCLOAK_TIMEOUT,
                ctx.cumulative_frame_nr(),
                maximized,
            ));
            if let Some(place) = settings.place() {
                win::place(hwnd, Some(place.normal), settings.default_size(), maximized);
            }
            win::set_least(settings.min_size());
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
            settings.output_device.clone(),
            settings.dsp.clone(),
        );
        let mut player = player;
        player.set_crossfade(settings.crossfade, settings.crossfade_in_albums);
        let saved_playlist = load_playlist(&mut player, settings.playlist_current);
        // Played where it was left, once Play is pressed.
        if settings.playlist_position > 0
            && let Some(entry) = player.queue.current_entry()
        {
            player.resume = Some((entry.id, settings.playlist_position));
        }
        // Files on the command line (Explorer's Open) play at once.
        let files = expand_playlists(files);
        // The rest of an Explorer selection comes from the other starts
        // as soon as the window listens: added to these, not over them.
        let last_received = (!files.is_empty()).then(Instant::now);
        if !files.is_empty() {
            player.play_paths(files, 0);
        }
        let mut updates = Updates::new(settings.check_updates, settings.last_update_check);
        updates.start_if_due(ctx);
        let about = AboutDialog::default();
        let watcher = crate::library::watch::Watcher::start(&settings.library_folders, &wake);
        let settings_layout = settings.window_layout;
        Self {
            settings,
            settings_dialog: SettingsDialog::from_env(),
            visual: crate::ui::visual::Visualizer::new(),
            heard: Vec::new(),
            player_area: egui::Rect::NOTHING,
            about,
            updates,
            hwnd,
            taskbar: hwnd.and_then(crate::taskbar::Taskbar::new),
            smtc: None,
            smtc_failed: false,
            last_media: None,
            cloak,
            place_changed: None,
            start_icon: None,
            title_icon: None,
            library: Arc::new(Library::default()),
            scan: Some(scan),
            watcher,
            watch_at: None,
            wake,
            scan_progress: None,
            loaded: false,
            library_view: crate::ui::library::LibraryView::new(ctx),
            right_held: false,
            player,
            last_received,
            playlist_view: crate::ui::playlist::PlaylistView::default(),
            saved_playlist,
            file_info: crate::ui::info::InfoView::default(),
            info_request: None,
            view_editor: crate::ui::view_editor::ViewEditor::default(),
            stats: crate::stats::Stats::load(),
            listening: crate::stats::Listening::default(),
            eq_window: crate::ui::eq_window::EqWindow::from_env(),
            tag_editor: crate::ui::tag_editor::TagEditor::default(),
            tray: None,
            off_taskbar: false,
            applied_modes: None,
            mini_cover: Default::default(),
            mini_icon: None,
            growing: None,
            applied_layout: settings_layout,
            env_view_editor: std::env::var_os("METALAMP_VIEW_EDITOR").is_some(),
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
                    if scanned.read > 0 {
                        crate::covers::prune(Arc::clone(&scanned.library));
                    }
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

    /// The folders read again once they have rested after a change: a
    /// frame is asked for when the rest is over (a scan under way wakes
    /// the app when it ends).
    fn follow_folders(&mut self, ctx: &egui::Context) {
        if self.watcher.take_changed() {
            self.watch_at = Some(Instant::now());
        }
        let Some(at) = self.watch_at else { return };
        let rested = at.elapsed();
        if rested < crate::library::watch::QUIET {
            ctx.request_repaint_after(crate::library::watch::QUIET - rested);
        } else if !self.scanning() {
            log::debug!("watch: the folders changed, reading them again");
            self.watch_at = None;
            self.rescan();
        }
    }

    /// Scan the library's folders again (Ctrl+R, a folder added or removed),
    /// keeping the tags of unchanged files. A scan under way is dropped.
    pub(crate) fn rescan(&mut self) {
        self.start_scan(false);
    }

    /// Every file's tags read again (the folders' menu): a tagger may
    /// change a tag and keep the file's size and time.
    fn read_all(&mut self) {
        self.start_scan(true);
    }

    fn start_scan(&mut self, read_all: bool) {
        self.scan_progress = None;
        // The folders as they are now, a disk plugged in since included.
        self.watcher =
            crate::library::watch::Watcher::start(&self.settings.library_folders, &self.wake);
        self.scan = Some(Scan::start(
            self.settings.library_folders.clone(),
            Some(Arc::clone(&self.library)),
            false,
            read_all,
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
            LibraryAction::ReadAll => self.read_all(),
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
            LibraryAction::PlayNext(tracks) => {
                let paths = self.paths_of(&tracks);
                self.player.play_next(paths);
            }
            LibraryAction::Enqueue(tracks) => {
                let paths = self.paths_of(&tracks);
                self.player.enqueue(paths);
            }
            LibraryAction::Settings => self.settings_dialog.toggle(),
            LibraryAction::Info(track) => {
                if let Some(t) = self.library.tracks.get(track as usize) {
                    self.info_request = Some(t.path.clone());
                }
            }
            LibraryAction::GoTo {
                album,
                track,
                to_album,
            } => self.go_to(album, track, to_album),
            LibraryAction::EditTags(tracks) => {
                let paths = self.paths_of(&tracks);
                if let Some(ctx) = self.wake.get().cloned() {
                    self.tag_editor.open(&ctx, paths);
                }
            }
            LibraryAction::Rate(tracks, n) => {
                let paths = self.paths_of(&tracks);
                self.stats.rate(&paths, n);
            }
            LibraryAction::NewView => self
                .view_editor
                .open_new(&self.library, &self.settings.library_folders),
            LibraryAction::EditView(name) => {
                if let Some(view) = self.settings.views.iter().find(|v| v.name == name) {
                    self.view_editor.open_edit(
                        &self.library,
                        &self.settings.library_folders,
                        view.clone(),
                    );
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
        if let Some(t) = self.library_view.selected_track_of(&self.library) {
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

    /// The mini player in the whole window.
    fn mini_player(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let (title, artist, track) = self.now_playing();
        let path = self.playing_or_current_path();
        let cover = self.mini_cover.of(&ctx, path.as_deref());
        let icon = crate::ui::app_icon(&ctx, 128.0, &mut self.mini_icon);
        let now = self.now_shown(&title, &artist, track, cover);
        let cmds = crate::ui::mini::show(ui, &now, &icon);
        self.bar_commands(cmds);
    }

    /// The player bar's layout: the one chosen, or the narrow one of the
    /// player and playlist layout.
    fn bar_layout(&self) -> crate::settings::BarLayout {
        match self.settings.window_layout {
            WindowLayout::Library => self.settings.bar_layout,
            WindowLayout::Playlist => crate::settings::BarLayout::Winamp,
        }
    }

    /// What the player bar and the mini player show.
    fn now_shown<'a>(
        &self,
        title: &'a str,
        artist: &'a str,
        track: Option<crate::library::TrackId>,
        cover: Option<egui::TextureHandle>,
    ) -> crate::ui::player_bar::NowPlaying<'a> {
        let (bitrate, sample_rate, channels) = track
            .and_then(|t| self.library.tracks.get(t as usize))
            .map_or((0, 0, 0), |t| (t.bitrate, t.sample_rate, t.channels));
        // "N. Artist – Title", as the playlist has it.
        let number = self.player.queue.current().map(|i| format!("{}. ", i + 1));
        let heading = match track.and_then(|t| self.library.tracks.get(t as usize)) {
            Some(t) if !t.artist.is_empty() => format!("{} – {}", t.artist, t.title),
            _ => title.to_owned(),
        };
        let heading = format!("{}{heading}", number.unwrap_or_default());
        crate::ui::player_bar::NowPlaying {
            heading,
            layout: self.bar_layout(),
            visual: self.settings.visualization,
            visual_enabled: self.settings.visual_enabled,
            status: self.player.status,
            title,
            artist,
            bitrate,
            sample_rate,
            channels,
            position_ms: self.player.position_ms(),
            duration_ms: self.player.duration_ms(),
            remaining: self.settings.time_remaining,
            volume: self.player.volume,
            mute: self.player.mute,
            shuffle: self.player.queue.shuffle(),
            repeat: self.player.queue.repeat,
            eq: self.settings.dsp.eq,
            stop_after: self.player.stop_after,
            cover,
        }
    }

    fn player_bar(&mut self, ui: &mut egui::Ui) {
        use crate::ui::player_bar;
        let ctx = ui.ctx().clone();
        let (title, artist, track) = self.now_playing();
        let cover = self.playing_cover(&ctx);
        // The visualization: the samples heard now, for its display's size.
        let size = player_bar::visual_size(self.bar_layout(), ui.max_rect());
        let mut heard = std::mem::take(&mut self.heard);
        let at = self
            .player
            .heard_samples(crate::ui::visual::FETCH, &mut heard);
        self.visual.feed(
            self.settings.visual_shown(),
            at.map(|h| (heard.as_slice(), h)),
            size,
        );
        self.heard = heard;
        let now = self.now_shown(&title, &artist, track, cover);
        let cmds = player_bar::show(ui, &now, &mut self.visual);
        self.bar_commands(cmds);
    }

    /// What a click in the player bar or the mini player asks for.
    fn bar_commands(&mut self, cmds: Vec<crate::ui::player_bar::BarCmd>) {
        use crate::ui::player_bar::BarCmd;
        for cmd in cmds {
            match cmd {
                BarCmd::Whole => {
                    self.settings.mini = false;
                    self.place_changed = Some(Instant::now());
                }
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
                BarCmd::TimeMode => {
                    self.settings.time_remaining = !self.settings.time_remaining;
                    self.place_changed = Some(Instant::now());
                }
                BarCmd::Visual(v) => {
                    self.settings.visualization = v;
                    self.place_changed = Some(Instant::now());
                }
                BarCmd::Equalizer(button) => self.eq_window.toggle(button),
            }
        }
    }

    /// The arrows with the pointer over the title row or the player bar
    /// (the mini player): left and right seek, up and down set the volume,
    /// unless a text field has the focus (else they move in the library's
    /// panes).
    fn pointer_keys(&mut self, ctx: &egui::Context, over_player: bool) {
        use crate::keymap::{SEEK_STEP, VOLUME_STEP};
        if !over_player || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let pressed = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        if pressed(Key::ArrowRight) {
            self.player.seek_by(SEEK_STEP);
        }
        if pressed(Key::ArrowLeft) {
            self.player.seek_by(-SEEK_STEP);
        }
        if pressed(Key::ArrowUp) {
            self.step_volume(VOLUME_STEP);
        }
        if pressed(Key::ArrowDown) {
            self.step_volume(-VOLUME_STEP);
        }
    }

    fn step_volume(&mut self, step: i8) {
        let v = (i16::from(self.player.volume) + i16::from(step)).clamp(0, 100) as u8;
        self.player.set_volume(v);
        self.place_changed = Some(Instant::now());
    }

    /// What the playlist's clicks, keys and menus ask for.
    fn playlist_actions(
        &mut self,
        actions: Vec<crate::ui::playlist::PlaylistAction>,
        ctx: &egui::Context,
        frame: &mut eframe::Frame,
    ) {
        for action in actions {
            match action {
                crate::ui::playlist::PlaylistAction::Play(i) => self.player.play_index(i),
                crate::ui::playlist::PlaylistAction::ShowInFolder(i) => {
                    if let Some(e) = self.player.queue.entries().get(i) {
                        win::show_in_folder(&e.path);
                    }
                }
                crate::ui::playlist::PlaylistAction::Info(i) => {
                    self.info_request = self.player.queue.entries().get(i).map(|e| e.path.clone());
                }
                crate::ui::playlist::PlaylistAction::InfoPlaying => {
                    self.info_request = self.playing_or_current_path();
                }
                crate::ui::playlist::PlaylistAction::StopAfter => {
                    self.player.toggle_stop_after();
                }
                crate::ui::playlist::PlaylistAction::PlayNext(indices) => {
                    self.player.move_next(&indices);
                }
                crate::ui::playlist::PlaylistAction::SaveAs(name) => {
                    if let Err(e) = crate::playlists::save(&name, &self.playlist_m3u8()) {
                        log::warn!("cannot save the playlist {name}: {e}");
                    }
                    self.playlist_view.list_playlists_again();
                }
                crate::ui::playlist::PlaylistAction::Open(name) => {
                    if let Some(paths) = crate::playlists::load(&name) {
                        self.player.play_paths(paths, 0);
                    }
                }
                crate::ui::playlist::PlaylistAction::OpenFile(path) => {
                    // Only what plays, as a playlist opened from Explorer.
                    if let Some(paths) = crate::playlists::tracks_in(&path) {
                        let paths = paths
                            .into_iter()
                            .filter(|p| crate::filetypes::is_audio(p))
                            .collect();
                        self.player.play_paths(paths, 0);
                    }
                }
                crate::ui::playlist::PlaylistAction::Delete(name) => {
                    crate::playlists::delete(&name);
                    self.playlist_view.list_playlists_again();
                }
                crate::ui::playlist::PlaylistAction::Hide(path) => {
                    self.settings.hidden_playlists.push(path);
                    self.playlist_view.list_playlists_again();
                    self.place_changed = Some(Instant::now());
                }
                crate::ui::playlist::PlaylistAction::ShowHidden => {
                    self.settings.hidden_playlists.clear();
                    self.playlist_view.list_playlists_again();
                    self.place_changed = Some(Instant::now());
                }
                crate::ui::playlist::PlaylistAction::Export => self.export_playlist(frame),
                crate::ui::playlist::PlaylistAction::EditTags(indices) => {
                    let paths = self.playlist_paths(&indices);
                    self.tag_editor.open(ctx, paths);
                }
                crate::ui::playlist::PlaylistAction::GoTo(i, to_album) => {
                    let track = self
                        .player
                        .queue
                        .entries()
                        .get(i)
                        .and_then(|e| self.library.by_path.get(&e.path).copied());
                    if let Some(t) = track {
                        let album = self.library.album_of[t as usize];
                        self.go_to(album, Some(t), to_album);
                    }
                }
            }
        }
    }

    /// What a key of the keymap asks for.
    fn run_key(&mut self, cmd: crate::keymap::Cmd, ctx: &egui::Context, frame: &mut eframe::Frame) {
        use crate::keymap::Cmd;
        match cmd {
            Cmd::PlayPause => self.player.toggle_pause(),
            Cmd::Play => self.player.play(),
            Cmd::Stop => self.player.stop(),
            Cmd::StopAfter => self.player.toggle_stop_after(),
            Cmd::Previous => self.player.previous(),
            Cmd::Next => self.player.next(),
            Cmd::Seek(seconds) => self.player.seek_by(seconds),
            Cmd::Repeat => {
                let next = self.player.queue.repeat.next();
                self.player.set_repeat(next);
                self.place_changed = Some(Instant::now());
            }
            Cmd::Shuffle => {
                let on = !self.player.queue.shuffle();
                self.player.set_shuffle(on);
                self.place_changed = Some(Instant::now());
            }
            Cmd::Volume(step) => self.step_volume(step),
            Cmd::Mute => {
                self.player.toggle_mute();
                self.place_changed = Some(Instant::now());
            }
            // Under the player bar's right end, where its button is.
            Cmd::Equalizer => {
                let at = self.player_area.right_bottom() - egui::vec2(16.0, 0.0);
                self.eq_window.toggle(egui::Rect::from_min_max(at, at));
            }
            Cmd::EqualizerOnOff | Cmd::Normalize => {
                if cmd == Cmd::Normalize {
                    self.settings.dsp.normalize = !self.settings.dsp.normalize;
                } else {
                    self.settings.dsp.eq = !self.settings.dsp.eq;
                }
                self.player.set_dsp(&self.settings.dsp);
                self.place_changed = Some(Instant::now());
            }
            Cmd::Mini => {
                self.settings.mini = !self.settings.mini;
                self.place_changed = Some(Instant::now());
            }
            Cmd::OnTop => {
                self.settings.on_top = !self.settings.on_top;
                self.place_changed = Some(Instant::now());
            }
            Cmd::Visualization => {
                self.settings.visual_enabled = !self.settings.visual_enabled;
                self.place_changed = Some(Instant::now());
            }
            Cmd::WindowLayout => {
                self.settings.window_layout = match self.settings.window_layout {
                    WindowLayout::Library => WindowLayout::Playlist,
                    WindowLayout::Playlist => WindowLayout::Library,
                };
                self.settings.mini = false;
                self.place_changed = Some(Instant::now());
            }
            // The playlist is the window in the player and playlist layout.
            Cmd::Playlist if self.settings.window_layout == WindowLayout::Playlist => {
                self.settings.mini = false;
            }
            Cmd::Playlist => {
                // From the mini player: the whole window with the playlist.
                self.settings.playlist_visible =
                    self.settings.mini || !self.settings.playlist_visible;
                self.settings.mini = false;
                self.place_changed = Some(Instant::now());
            }
            Cmd::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            // The mini player is not maximized.
            Cmd::Maximize if self.settings.mini => {}
            Cmd::Maximize => ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(
                !frameless::is_maximized(ctx),
            )),
            Cmd::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            // The playlist's search without the library.
            Cmd::Search if self.settings.window_layout == WindowLayout::Playlist => {
                self.playlist_view.focus_search = true;
                self.settings.mini = false;
            }
            Cmd::Search => {
                self.library_view.focus_search = true;
                self.settings.mini = false;
            }
            Cmd::Open => self.open_files(frame),
            Cmd::AddFolder => self.pick_folder(frame),
            Cmd::Rescan => self.rescan(),
            Cmd::Info => self.info_request = self.info_path(),
            // The selection of the playlist when it has the keys, else of
            // the tracks' table.
            Cmd::EditTags => {
                let paths = if self.playlist_view.focused {
                    self.playlist_paths(&self.playlist_view.selection())
                } else {
                    let tracks = self.library_view.selected_tracks_of(&self.library);
                    self.paths_of(&tracks)
                };
                self.tag_editor.open(ctx, paths);
            }
            Cmd::View(n) => {
                if crate::ui::library::choose_view(&mut self.settings, usize::from(n)) {
                    self.place_changed = Some(Instant::now());
                }
            }
            Cmd::Settings => self.settings_dialog.toggle(),
            Cmd::About => self.about.toggle(),
        }
    }

    /// The window's title: the track heard, then the program's name.
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
        let Some((until, frame, maximized)) = self.cloak else {
            return;
        };
        // A few frames painted (Windows showed the window white until its
        // first one was), and maximized when it is to be.
        let ready = ctx.cumulative_frame_nr() >= frame + PAINTED_TO_SHOW
            && (!maximized || frameless::is_maximized(ctx));
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
        let mini = self.applied_modes.map(|m| m.1);
        if mini == Some(true)
            && self.settings.mini
            && !win::is_minimized(hwnd)
            && let Some(rect) = win::window_rect(hwnd)
            && self.settings.mini_rect != Some(rect)
        {
            self.settings.mini_rect = Some(rect);
            self.place_changed = Some(Instant::now());
        }
        if self.cloak.is_none()
            && !self.settings.mini
            && mini == Some(false)
            && self.applied_layout == self.settings.window_layout
            && let Some((normal, maximized)) = win::placement(hwnd)
        {
            let place = WindowPlace { normal, maximized };
            if self.settings.place() != Some(place) {
                *self.settings.place_mut() = Some(place);
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

    /// Copy what lives elsewhere (the update check, the player) into the
    /// settings.
    /// "Go to album" or "Go to artist" in the library; the keys go back to
    /// its panes.
    fn go_to(
        &mut self,
        album: crate::library::AlbumId,
        track: Option<crate::library::TrackId>,
        to_album: bool,
    ) {
        if self.library_view.go_to(
            &self.library,
            &self.stats,
            &mut self.settings,
            album,
            track,
            to_album,
        ) {
            self.place_changed = Some(Instant::now());
        }
        self.playlist_view.focused = false;
    }

    /// The clock of the track heard run; its play counted once it has been
    /// heard long enough (`stats::count_after`).
    fn count_play(&mut self) {
        let playing = self.player.status == crate::player::Status::Playing;
        if self.listening.tick(playing, self.player.duration_ms())
            && let Some(path) = self.player.playing_path().cloned()
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            self.stats.played(&path, now);
        }
    }

    /// The paths of the playlist's entries at `indices`.
    fn playlist_paths(&self, indices: &[usize]) -> Vec<PathBuf> {
        let entries = self.player.queue.entries();
        indices
            .iter()
            .filter_map(|&i| entries.get(i).map(|e| e.path.clone()))
            .collect()
    }

    /// The name of the library view in use, if one narrows the library.
    pub(crate) fn view_name(&self) -> Option<&str> {
        self.settings.current_view().map(|v| v.name.as_str())
    }

    fn sync_settings(&mut self) {
        self.settings.check_updates = self.updates.enabled;
        self.settings.last_update_check = self.updates.last_check;
        self.settings.volume = self.player.volume;
        self.settings.mute = self.player.mute;
        self.settings.repeat = self.player.queue.repeat;
        self.settings.shuffle = self.player.queue.shuffle();
        self.settings.playlist_current = self.player.queue.current();
        self.settings.playlist_position = self.player.position_to_keep();
    }

    /// Write the playlist when it or its current entry changed.
    /// The playlist as M3U8 text, each entry with its length and
    /// "Artist - Title" from the library.
    fn playlist_m3u8(&self) -> String {
        self.player.queue.m3u8(&|path| {
            let &id = self.library.by_path.get(path)?;
            let t = &self.library.tracks[id as usize];
            let title = if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} - {}", t.artist, t.title)
            };
            Some((u64::from(t.duration_ms) / 1000, title))
        })
    }

    /// The playlist saved to a file chosen in Windows' dialog.
    fn export_playlist(&self, frame: &eframe::Frame) {
        let dialog = rfd::FileDialog::new()
            .set_title(tr!("Save the playlist", "Сохранить плейлист"))
            .add_filter("M3U8", &["m3u8"])
            .set_file_name(tr!("Playlist.m3u8", "Плейлист.m3u8"))
            .set_parent(frame);
        if let Some(path) = dialog.save_file()
            && let Err(e) = std::fs::write(&path, self.playlist_m3u8())
        {
            log::warn!("cannot write {}: {e}", path.display());
        }
    }

    fn remember_playlist(&mut self, now: bool) {
        let state = (self.player.queue.revision, self.player.queue.current());
        if state == self.saved_playlist {
            return;
        }
        let revision_changed = state.0 != self.saved_playlist.0;
        self.saved_playlist = state;
        if revision_changed {
            let text = self.playlist_m3u8();
            if let Some(path) = playlist_file() {
                static FILE: crate::kept::Kept = crate::kept::Kept::new();
                let number = FILE.ask();
                let write = move || {
                    if let Err(e) = FILE.write(number, &path, text.as_bytes()) {
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

    /// Closed while minimized or hidden: eframe acts on a close only in a
    /// frame it paints, so what `on_exit` does is done here and the
    /// process ends.
    fn close_now(&mut self) {
        log::info!("closed while minimized: saved, and the process ends");
        crate::instance::closing();
        self.save_now();
        self.tray = None;
        std::process::exit(0);
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
                // The Restart Manager (an installer): minimized, the window
                // gets no frame that would act on a close.
                win::Hooked::Close if self.hwnd.is_some_and(win::is_minimized) => {
                    self.close_now();
                }
                win::Hooked::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                win::Hooked::TaskbarButton => {
                    if let Some(taskbar) = &mut self.taskbar {
                        taskbar.button_created();
                    }
                    // Taken away again if the window is in the tray.
                    self.off_taskbar = false;
                }
                win::Hooked::CloseNow => self.close_now(),
                win::Hooked::TrayCreated => {
                    if let Some(tray) = &mut self.tray {
                        tray.add();
                    }
                }
                win::Hooked::Tray(cmd) => {
                    use crate::tray::TrayCmd;
                    match cmd {
                        TrayCmd::Show => {
                            if let Some(hwnd) = self.hwnd {
                                win::bring_back(hwnd);
                            }
                        }
                        TrayCmd::PlayPause => self.player.toggle_pause(),
                        TrayCmd::Stop => self.player.stop(),
                        TrayCmd::Previous => self.player.previous(),
                        TrayCmd::Next => self.player.next(),
                        TrayCmd::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    }
                }
                win::Hooked::Thumb(id) => match id {
                    crate::taskbar::PREVIOUS => self.player.previous(),
                    crate::taskbar::PLAY_PAUSE => self.player.toggle_pause(),
                    crate::taskbar::NEXT => self.player.next(),
                    _ => {}
                },
                win::Hooked::Media(key, via) => {
                    let now = Instant::now();
                    if same_press(self.last_media, key, via, now) {
                        log::debug!("media key {key:?} by {via:?}: the same press");
                        continue;
                    }
                    log::debug!("media key {key:?} by {via:?}");
                    self.last_media = Some((key, via, now));
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

    /// Whether Previous does something: a track before the current one,
    /// or the current one past its first seconds (back to its start).
    fn has_previous(&self) -> bool {
        let p = &self.player;
        p.queue.before(p.queue.current()).is_some()
            || (p.status != crate::player::Status::Stopped && p.position_ms() > 3000)
    }

    fn has_next(&self) -> bool {
        self.player
            .queue
            .after(self.player.queue.current())
            .is_some()
    }

    /// The position as the taskbar button's progress (yellow when paused)
    /// and the thumbnail buttons' state.
    fn show_on_taskbar(&mut self) {
        use crate::player::Status;
        let status = self.player.status;
        let duration = self.player.duration_ms();
        let progress = (status != Status::Stopped && duration > 0)
            .then(|| (self.player.position_ms().min(duration) * 1000 / duration).min(1000));
        let shown = crate::taskbar::Shown {
            progress,
            paused: status != Status::Playing,
            has_previous: self.has_previous(),
            has_next: self.has_next(),
        };
        if let Some(taskbar) = &mut self.taskbar {
            taskbar.show(shown);
        }
    }

    /// Take the media keys as the setting says: Windows' media controls
    /// made or let go, the window's own keys on or off.
    /// The layout chosen applied to the window: the place of the one before
    /// kept, the new one's put back (else its size at first, centred where
    /// the window was) with its least size; until the window has its size
    /// only the ground is painted (`growing`). The mini player waits.
    fn apply_layout(&mut self, ctx: &egui::Context) {
        let wanted = self.settings.window_layout;
        if self.applied_layout == wanted || self.settings.mini {
            return;
        }
        let Some(hwnd) = self.hwnd else { return };
        if let Some((normal, maximized)) = win::placement(hwnd) {
            let place = Some(WindowPlace { normal, maximized });
            match self.applied_layout {
                WindowLayout::Library => self.settings.window = place,
                WindowLayout::Playlist => self.settings.playlist_window = place,
            }
        }
        self.applied_layout = wanted;
        let least = self.settings.min_size();
        win::set_least(least);
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(least.into()));
        match self.settings.place() {
            Some(w) => win::show_placed(hwnd, w.normal, w.maximized),
            None => {
                let size = egui::Vec2::from(self.settings.default_size()) * ctx.pixels_per_point();
                if let Some(r) = win::window_rect(hwnd) {
                    let (w, h) = (size.x as i32, size.y as i32);
                    let (x, y) = ((r[0] + r[2] - w) / 2, (r[1] + r[3] - h) / 2);
                    let mut rect = [x, y, x + w, y + h];
                    // On the screen as far as it fits.
                    if let Some(work) = win::work_area_at(rect) {
                        let dx = (work[2] - rect[2]).min(0) + (work[0] - rect[0]).max(0);
                        let dy = (work[3] - rect[3]).min(0) + (work[1] - rect[1]).max(0);
                        rect = [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy];
                    }
                    win::set_rect(hwnd, rect);
                }
            }
        }
        self.growing = Some(Instant::now());
        self.place_changed = Some(Instant::now());
    }

    /// "Over the others" and the mini player applied to the window when
    /// they changed. The mini player is a square at its place of the last
    /// time (else in the work area's bottom right corner), always over the
    /// others, its least size and shape kept by `win::set_mini`, not to be
    /// maximized; back, the whole window at its place, maximized if it
    /// was.
    fn apply_modes(&mut self, ctx: &egui::Context) {
        use crate::ui::mini;
        use egui::{ViewportCommand, WindowLevel};
        let Some(hwnd) = self.hwnd else { return };
        let wanted = (
            self.settings.on_top || self.settings.mini,
            self.settings.mini,
        );
        let applied = self.applied_modes;
        if applied == Some(wanted) {
            return;
        }
        if applied.map(|a| a.0) != Some(wanted.0) {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if wanted.0 {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            }));
        }
        let was_mini = applied.is_some_and(|a| a.1);
        if wanted.1 && !was_mini {
            // The whole window's place as it is now, for the way back.
            if applied.is_some()
                && let Some((normal, maximized)) = win::placement(hwnd)
            {
                *self.settings.place_mut() = Some(WindowPlace { normal, maximized });
            }
            let ppp = ctx.pixels_per_point();
            let least = (mini::LEAST * ppp).round() as i32;
            win::set_mini(Some(least));
            ctx.send_viewport_cmd(ViewportCommand::EnableButtons {
                close: true,
                minimized: true,
                maximize: false,
            });
            let saved = self
                .settings
                .mini_rect
                .filter(|r| win::on_a_monitor(*r) && r[2] - r[0] >= least && r[3] - r[1] >= least);
            let rect = saved.or_else(|| {
                let side = (mini::SIDE * ppp).round() as i32;
                let margin = (mini::MARGIN * ppp).round() as i32;
                let work = win::work_area_at(win::window_rect(hwnd)?)?;
                let (right, bottom) = (work[2] - margin, work[3] - margin);
                Some([right - side, bottom - side, right, bottom])
            });
            if let Some(rect) = rect {
                win::set_rect(hwnd, rect);
            }
        } else if !wanted.1 && was_mini {
            if !win::is_minimized(hwnd) {
                self.settings.mini_rect = win::window_rect(hwnd);
            }
            win::set_mini(None);
            self.mini_cover.clear();
            self.growing = Some(Instant::now());
            ctx.send_viewport_cmd(ViewportCommand::EnableButtons {
                close: true,
                minimized: true,
                maximize: true,
            });
            match self.settings.place() {
                Some(w) => win::show_placed(hwnd, w.normal, w.maximized),
                None => {
                    let ppp = ctx.pixels_per_point();
                    let size = egui::Vec2::from(self.settings.default_size()) * ppp;
                    if let Some(work) = win::window_rect(hwnd).and_then(win::work_area_at) {
                        let x = (work[0] + work[2] - size.x as i32) / 2;
                        let y = (work[1] + work[3] - size.y as i32) / 2;
                        win::set_rect(hwnd, [x, y, x + size.x as i32, y + size.y as i32]);
                    }
                }
            }
        }
        self.applied_modes = Some(wanted);
    }

    /// The tray icon made or dropped as the setting says, its tooltip what
    /// plays; minimized with it, the window leaves the taskbar.
    fn follow_tray(&mut self) {
        let Some(hwnd) = self.hwnd else { return };
        if self.settings.tray {
            if self.tray.is_none() {
                self.tray = crate::tray::Tray::new(hwnd);
            }
        } else {
            self.tray = None;
        }
        let off = self.tray.is_some() && win::is_minimized(hwnd);
        if off != self.off_taskbar {
            if let Some(taskbar) = &self.taskbar {
                taskbar.set_tab(!off);
            }
            self.off_taskbar = off;
        }
        if self.tray.is_some() {
            let tip = self.window_title();
            if let Some(tray) = &mut self.tray {
                tray.set_tip(&tip);
            }
        }
    }

    fn follow_media_keys(&mut self) {
        use crate::settings::MediaKeys;
        let wanted = self.settings.media_keys;
        win::set_media_keys(wanted != MediaKeys::Off);
        if wanted == MediaKeys::Always {
            if self.smtc.is_none() && !self.smtc_failed {
                self.smtc = self.hwnd.and_then(crate::smtc::Smtc::new);
                self.smtc_failed = self.smtc.is_none();
            }
        } else if self.smtc.take().is_some() {
            log::debug!("media controls let go");
        }
    }

    /// What plays in Windows' media controls: the state, previous and
    /// next, the title and the artist.
    fn show_in_media_controls(&mut self) {
        if self.smtc.is_none() {
            return;
        }
        use crate::player::Status as Player;
        use crate::smtc::Status;
        let status = match self.player.status {
            Player::Stopped if self.player.queue.is_empty() => Status::Closed,
            Player::Stopped => Status::Stopped,
            Player::Paused => Status::Paused,
            Player::Playing => Status::Playing,
        };
        let (title, _, track) = self.now_playing();
        let artist = track
            .and_then(|t| self.library.tracks.get(t as usize))
            .map(|t| t.artist.clone())
            .filter(|a| !a.is_empty());
        let shown = crate::smtc::Shown {
            status,
            has_previous: self.has_previous(),
            has_next: self.has_next(),
            title: if status == Status::Closed {
                String::new()
            } else {
                title
            },
            artist,
            music: true,
        };
        if let Some(smtc) = &mut self.smtc {
            smtc.show(shown);
        }
    }

    fn title_row(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let icon = crate::ui::app_icon(&ctx, 18.0, &mut self.title_icon);
        let update = self.updates.newer().map(str::to_owned);
        let maximized = frameless::is_maximized(&ctx);
        let title = self.window_title();
        let modes = title::Modes {
            on_top: self.settings.on_top,
        };
        match title::row(ui, &title, maximized, &icon, update.as_deref(), modes) {
            Some(TitleCmd::OnTop) => {
                self.settings.on_top = !self.settings.on_top;
                self.place_changed = Some(Instant::now());
            }
            Some(TitleCmd::Mini) => {
                self.settings.mini = !self.settings.mini;
                self.place_changed = Some(Instant::now());
            }
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
        self.follow_folders(ctx);
        self.library_view.covers.poll(ctx);
        // Minimized, eframe runs a frame only when one is due, not when the
        // window's messages wake it (the tray, the media keys, a close):
        // one is kept due.
        if self.hwnd.is_some_and(win::is_minimized) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.take_hooked(ctx);
        self.take_handed_over();
        self.remember_playlist(false);
        if self.player.poll().track {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));
            self.listening.restart(self.player.playing);
        }
        self.count_play();
        // The visualization moves: 50 frames a second while playing, and
        // after a stop while it falls; paused, it stands. Without one (off,
        // the compact bar, the window minimized) only the time, the seek
        // bar and the taskbar's progress move.
        let shown = self.bar_layout() != crate::settings::BarLayout::Classic
            && self.settings.visual_shown() != crate::settings::Visual::Off
            && !self.settings.mini
            && ctx.input(|i| i.viewport().minimized) != Some(true);
        let moving = match self.player.status {
            crate::player::Status::Playing => true,
            crate::player::Status::Paused => false,
            crate::player::Status::Stopped => self.visual.active(),
        };
        if shown && moving {
            ctx.request_repaint_after(Duration::from_millis(20));
        } else if self.player.status == crate::player::Status::Playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.follow_media_keys();
        self.follow_tray();
        self.apply_layout(ctx);
        self.apply_modes(ctx);
        self.show_on_taskbar();
        self.show_in_media_controls();
        self.remember_place();
    }

    // egui 0.36: `ui` with the root `Ui`, not `update(ctx)`; panels are
    // `egui::Panel::top/left/right/bottom(id).show(ui, …)` and
    // `CentralPanel::default()` / `CentralPanel::no_frame()`.
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.uncloak(&ctx);
        // `METALAMP_INFO=<file>`: the file info at start (GUI checks).
        if let Some(path) = std::env::var_os("METALAMP_INFO")
            && ctx.cumulative_frame_nr() == 1
        {
            self.info_request = Some(PathBuf::from(path));
        }
        // `METALAMP_TAG_EDITOR=<file>`: the tag editor at start (GUI checks).
        if let Some(path) = std::env::var_os("METALAMP_TAG_EDITOR")
            && !self.tag_editor.open
            && ctx.cumulative_frame_nr() == 1
        {
            self.tag_editor.open(&ctx, vec![PathBuf::from(path)]);
        }
        if self.env_view_editor && self.loaded {
            self.env_view_editor = false;
            self.view_editor
                .open_new(&self.library, &self.settings.library_folders);
        }
        // A modal takes the keys while it is open.
        let modal = self.about.open
            || self.file_info.open
            || self.settings_dialog.open
            || self.view_editor.open
            || self.tag_editor.open;
        // Outside the modals only the search fields keep the focus: a
        // button that Tab focused took every key of the player until a
        // click elsewhere (the keys wait while anything has the focus).
        if !modal
            && !ctx.text_edit_focused()
            && let Some(id) = ctx.memory(|m| m.focused())
        {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        let typing = ctx.memory(|m| m.focused().is_some());
        // Letters typed over the artists' pane go to an artist, not to
        // the keys below.
        let letters = !modal
            && !typing
            && !self.settings.library_folders.is_empty()
            && self.settings.window_layout == WindowLayout::Library
            && !self.settings.mini
            && self.library_view.pointer_over_artists(&ctx);
        self.library_view.letters = letters;
        // The keys as the settings' Keys page set them: none while a modal
        // is open; while a text field has the focus only those it does
        // not take (F-keys, Alt and most Ctrl chords); letters typed over
        // the artists go to an artist.
        if !modal {
            let cmds = crate::keymap::take(&ctx, &self.settings.keys, |chord| {
                !(typing && chord.edits_text()) && !(letters && chord.types())
            });
            for cmd in cmds {
                self.run_key(cmd, &ctx, frame);
            }
            // Esc leaves the mini player.
            if self.settings.mini
                && !typing
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                self.settings.mini = false;
                self.place_changed = Some(Instant::now());
            }
        }
        self.take_dropped(&ctx);
        if !modal {
            let over_player = ctx
                .pointer_hover_pos()
                .is_some_and(|p| self.player_area.contains(p));
            self.pointer_keys(&ctx, over_player);
        }
        if !frameless::is_maximized(&ctx) {
            frameless::resize_edges(root_ui);
        }
        // A window or a modal wants the whole window: the mini player
        // ends (it shows in the next frame).
        if self.settings.mini
            && (modal
                || self.info_request.is_some()
                || self.eq_window.open
                || self.about.open
                || self.settings_dialog.open)
        {
            self.settings.mini = false;
            self.place_changed = Some(Instant::now());
        }
        // The mini player: the cover and its controls alone, as long as
        // the window has been made it (`apply_modes`, in this frame's
        // logic).
        let mini = self.settings.mini && self.applied_modes.is_some_and(|m| m.1);
        if let Some(at) = self.growing {
            let least = self.settings.min_size();
            let grown = ctx.content_rect().width() >= least[0] - 1.0
                && ctx.content_rect().height() >= least[1] - 1.0;
            if grown || self.settings.mini || at.elapsed() >= Duration::from_secs(1) {
                self.growing = None;
            } else {
                root_ui
                    .painter()
                    .rect_filled(ctx.content_rect(), 0.0, palette::bar_bg());
                ctx.request_repaint();
                return;
            }
        }
        self.player_area = if mini {
            ctx.content_rect()
        } else {
            let title_rect = egui::Panel::top("title")
                .exact_size(title::HEIGHT)
                .frame(egui::Frame::NONE)
                .show(root_ui, |ui| self.title_row(ui))
                .response
                .rect;
            let bar_rect = egui::Panel::top("player")
                .exact_size(crate::ui::player_bar::height(self.bar_layout()))
                .frame(egui::Frame::NONE)
                .show(root_ui, |ui| self.player_bar(ui))
                .response
                .rect;
            title_rect.union(bar_rect)
        };
        // The wheel anywhere over the title row and the player bar (the
        // mini player) steps the volume by 5.
        if !modal
            && ctx
                .pointer_hover_pos()
                .is_some_and(|p| self.player_area.contains(p))
        {
            let notches = crate::ui::wheel_notches(root_ui);
            if notches != 0 {
                let v = (i32::from(self.player.volume) + notches * 5).clamp(0, 100) as u8;
                self.player.set_volume(v);
                self.place_changed = Some(Instant::now());
            }
        }
        let alone = self.settings.window_layout == WindowLayout::Playlist && !mini;
        self.playlist_view.no_library = alone;
        if alone {
            let cover = self.playing_cover(&ctx);
            let shown_cover = self.settings.playlist_cover;
            let panel = egui::CentralPanel::no_frame().show(root_ui, |ui| {
                crate::ui::playlist::show(
                    ui,
                    &mut self.playlist_view,
                    &mut self.player,
                    &self.library,
                    !modal,
                    cover.as_ref(),
                    &mut self.settings.playlist_cover,
                    &self.settings.hidden_playlists,
                )
            });
            if self.settings.playlist_cover != shown_cover {
                self.place_changed = Some(Instant::now());
            }
            self.playlist_view.focused = true;
            self.playlist_actions(panel.inner, &ctx, frame);
        // Also beside the start screen: files opened from Explorer, Ctrl+O
        // or dropped play without a library.
        } else if self.settings.playlist_visible && !mini {
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
                        &self.settings.hidden_playlists,
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
            self.playlist_actions(panel.inner, &ctx, frame);
        } else {
            self.playlist_view.hidden();
        }
        if !mini && !alone {
            egui::Panel::bottom("status")
                .exact_size(crate::ui::status::HEIGHT)
                .frame(egui::Frame::NONE.fill(palette::title_bg()))
                .show(root_ui, |ui| crate::ui::status::show(ui, self));
        }
        if !alone {
            egui::CentralPanel::no_frame().show(root_ui, |ui| {
                if mini {
                    self.mini_player(ui);
                    return;
                }
                if !self.settings.library_folders.is_empty() {
                    let scanning = self.scanning();
                    let playing = self.playing_track();
                    let (actions, changed) = crate::ui::library::show(
                        ui,
                        &mut self.library_view,
                        &self.library,
                        &mut self.settings,
                        &self.stats,
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
        }
        if let Some(path) = self.info_request.take() {
            self.file_info.open(&ctx, &path);
        }
        crate::ui::paint_dragged_tracks(&ctx);
        if self.tag_editor.show(&ctx).written {
            // The files written: their tags read again.
            self.rescan();
        }
        // The equalizer's window, under the modals.
        let before = (self.settings.dsp.clone(), self.settings.eq_presets.clone());
        self.eq_window.show(
            &ctx,
            &mut self.settings.dsp,
            &mut self.settings.eq_presets,
            !modal,
            self.hwnd,
        );
        if self.settings.dsp != before.0 {
            self.player.set_dsp(&self.settings.dsp);
        }
        if (&self.settings.dsp, &self.settings.eq_presets) != (&before.0, &before.1) {
            self.place_changed = Some(Instant::now());
        }
        self.file_info.show(&ctx, self.hwnd);
        self.about.show(&ctx);
        if self
            .view_editor
            .show(&ctx, &mut self.settings, &self.library, &self.stats, frame)
        {
            self.place_changed = Some(Instant::now());
        }
        let changed =
            self.settings_dialog
                .show(&ctx, self.hwnd, &mut self.settings, &mut self.updates);
        if changed.device {
            self.player.set_device(&self.settings.output_device);
        }
        if changed.dsp {
            self.player.set_dsp(&self.settings.dsp);
        }
        if changed.crossfade {
            self.player
                .set_crossfade(self.settings.crossfade, self.settings.crossfade_in_albums);
        }
        if changed.look {
            palette::set(self.settings.look());
            palette::apply(&ctx);
            if let Some(hwnd) = self.hwnd {
                crate::caption::set_dark(hwnd, self.settings.theme.is_dark());
            }
        }
        if changed.keys {
            crate::keymap::publish(&self.settings.keys);
        }
        if changed.any {
            self.place_changed = Some(Instant::now());
        }
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
                let text = crate::queue::m3u_text(&bytes);
                let folder = f.parent().map(PathBuf::from).unwrap_or_default();
                // Only what plays: not a playlist in a playlist, nor a folder.
                out.extend(
                    crate::queue::m3u_paths(&text, &folder)
                        .into_iter()
                        .filter(|p| crate::filetypes::is_audio(p)),
                );
            }
        } else if crate::filetypes::is_audio(&f) {
            out.push(f);
        }
    }
    out
}

/// Media key `key` at `at`, come by `via`, is the press `last` was, come
/// again another way (metalmpv's): Play, Pause and Play/Pause are one
/// button. Two quick presses the same way are two.
fn same_press(
    last: Option<(win::MediaKey, win::Via, Instant)>,
    key: win::MediaKey,
    via: win::Via,
    at: Instant,
) -> bool {
    use win::MediaKey::{Pause, Play, PlayPause};
    let button = |k: win::MediaKey| match k {
        Play | Pause | PlayPause => PlayPause,
        other => other,
    };
    last.is_some_and(|(k, way, t)| {
        way != via
            && button(k) == button(key)
            && at.saturating_duration_since(t) < Duration::from_millis(300)
    })
}

#[cfg(test)]
mod media_tests {
    use super::*;
    use win::{MediaKey, Via};

    #[test]
    fn one_press_two_ways_counts_once() {
        let t = Instant::now();
        let last = Some((MediaKey::PlayPause, Via::Command, t));
        let soon = t + Duration::from_millis(50);
        assert!(same_press(last, MediaKey::PlayPause, Via::Key, soon));
        assert!(same_press(last, MediaKey::Pause, Via::Controls, soon));
        // The same way twice is two presses; another key is another press.
        assert!(!same_press(last, MediaKey::PlayPause, Via::Command, soon));
        assert!(!same_press(last, MediaKey::Next, Via::Key, soon));
        // Late: a press of its own.
        let late = t + Duration::from_millis(400);
        assert!(!same_press(last, MediaKey::PlayPause, Via::Key, late));
    }
}
