//! The file info window, which edits the tags too: a click on the player
//! bar's cover opens it for the track heard, I, Alt+3 or the context menus
//! of the tracks and the playlist for the track selected; Ctrl+E and "Edit
//! tags…" open it editing the files selected. The cover, the title, the
//! artist and the album on top with the audio properties as chips; the
//! main fields in the usual order of taggers (the short ones two a line,
//! in boxes as while editing), the tag's other fields under them (only
//! those it has), the lyrics, the file's path and the rest; the text can be
//! selected and copied. Several files are shown one at a time (‹ ›) or all
//! at once: the main fields with the value the files share or
//! "(different)", and the files' names.
//!
//! Editing ("Edit" or Ctrl+E, `Form`): the main fields and the other fields
//! the tag has become text fields, read with `library::edit` on a thread;
//! a field the files differ in is left alone unless typed in; Save (Enter
//! with no field focused) writes only the fields typed in, on a thread; a
//! file that cannot be written (in use, a format lofty does not write) is
//! named under the fields and the others are written; Cancel or Esc gives
//! up. Going to another file saves first. The library reads the files
//! written again (`App`).
//!
//! The cover (a click or the right button opens its menu): a picture
//! chosen in a dialog, the folder's picture or one dropped on the window
//! becomes the files' front cover, or the cover is removed; written with
//! the fields (`edit::CoverChange`), made smaller first when the setting
//! says (`covers::for_tag`), and shown meanwhile. Not editing, these start
//! it. "Save as folder.jpg" writes the cover shown into the folder at once.
//!
//! The cross, Esc, Enter or a click outside closes it; while editing only
//! the cross (typing is not lost by a slip). When the program's window
//! cannot hold it (the player and playlist layout), it is a window of its
//! own, as the settings are. The file is read on a thread of its own
//! (`library::details`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};

use egui::{Key, RichText, TextureHandle, Ui, vec2};
use lofty::picture::PictureType;
use lofty::prelude::ItemKey;

use crate::audio::Releaser;
use crate::format;
use crate::keymap::{Action, Cmd, Keymap};
use crate::library::details::{self, Details};
use crate::library::edit::{self, Changes, CoverChange, Values};
use crate::palette;
use crate::settings::{CoverSide, Id3Version};

type Read = (PathBuf, Result<Details, String>);

const WIDTH: f32 = 720.0;
/// The cover's side in the window, in points.
const COVER: f32 = 150.0;

/// The main fields in the order taggers show them; the short ones two a
/// line.
enum Line {
    One(edit::Key),
    Two(edit::Key, edit::Key),
}
const MAIN: [Line; 8] = [
    Line::One(edit::Key::Title),
    Line::One(edit::Key::Artist),
    Line::One(edit::Key::Album),
    Line::One(edit::Key::AlbumArtist),
    Line::One(edit::Key::Composer),
    Line::Two(edit::Key::Year, edit::Key::Genre),
    Line::Two(edit::Key::Track, edit::Key::Disc),
    Line::One(edit::Key::Comment),
];
/// The width of the first value of a line of two, in points.
const SHORT: f32 = 150.0;
/// The names' column at most, in points; a longer name is cut.
const MAX_NAME: f32 = 190.0;
/// The row of Cancel and Save under the fields while editing, in points.
const FOOTER: f32 = 44.0;
/// The files that could not be read or written, at most, in points.
const ERRORS: f32 = 70.0;

/// The window of its own: its title (by which it is found) and height.
fn window_title() -> &'static str {
    tr!("File info", "Сведения о файле")
}
const WINDOW_HEIGHT: f32 = 640.0;

#[derive(Default)]
pub struct InfoView {
    pub open: bool,
    /// Opened this frame: the key that opened it must not close it.
    fresh: bool,
    /// The files opened (the selection), at least one while open.
    paths: Vec<PathBuf>,
    /// The one shown, or none: all of them at once (several only).
    at: Option<usize>,
    /// What LRCLIB gave each file while all of them were edited, kept while
    /// one of them is shown (in `paths`' order; empty: nothing asked or
    /// it is the form's): a file's lyrics found go into its field when it
    /// is edited, to be looked at before they are written.
    lrclib_found: Vec<Found>,
    /// The file whose details are asked for: the one shown (with all of
    /// them, the first, for its cover).
    wanted: Option<PathBuf>,
    /// What was read of it (or of the file before, until it is read).
    shown: Option<Read>,
    cover: Option<(PathBuf, TextureHandle)>,
    /// To the reading thread, started with the first file asked for.
    requests: Option<Sender<PathBuf>>,
    results: Option<Receiver<Read>>,
    /// The fields as `edit` reads and writes them: of all the files shown
    /// at once, or of the one edited.
    form: Option<Form>,
    /// Forms still writing when another file was opened (the main window
    /// stays interactive beside a window of its own): polled for what
    /// they wrote, then dropped.
    finishing: Vec<Form>,
    editing: bool,
    /// Where to go once the fields typed in are written (`go`).
    then: Option<Option<usize>>,
    /// In a window of its own (decided as it opens), and its handle once
    /// found.
    windowed: bool,
    window: super::OwnWindow,
    /// Has the player close the files written meanwhile (`new`).
    releaser: Option<Releaser>,
    /// The largest side of a cover put into the tags: the setting, set by
    /// the app each frame and chosen in the cover's menu, which the app
    /// keeps.
    pub cover_side: CoverSide,
    /// How an MP3's tags are written: the settings', set by the app each
    /// frame.
    pub write_tags: edit::WriteTags,
    /// LRCLIB may be asked for lyrics while editing (the setting, set by
    /// the app each frame).
    pub lrclib: bool,
    /// The library's genres, offered by the genre's field (`set_library`),
    /// and the library they are of.
    genres: Vec<String>,
    genres_of: Option<u64>,
    /// A picture is to be chosen for the cover: the app shows the dialog
    /// and hands its answer to `cover_from`.
    pub pick_cover: bool,
    /// The picture chosen in that dialog, taken in `show`.
    cover_file: Option<PathBuf>,
    /// A text file is to be chosen for the lyrics: the app shows the
    /// dialog and hands its answer to `lyrics_from`.
    pub pick_lyrics: bool,
    /// The text file chosen in that dialog, taken in `show`.
    lyrics_file: Option<PathBuf>,
    /// A `folder.jpg` was saved into this folder: the app makes its
    /// albums' covers again.
    pub folder_saved: Option<PathBuf>,
    /// A picture made ready for the tags, or a `folder.jpg` saved, on a
    /// thread.
    cover_job: Option<Receiver<CoverDone>>,
    /// What became of the last of them: a text and whether it is an error.
    cover_note: Option<(String, bool)>,
    /// The cover shown large over the window (a click on it).
    zoom: Option<Zoom>,
    /// The track's spectrum in a window of its own (the "Spectrum" chip),
    /// which stays open when this one closes.
    spectrum: super::track_spectrum::SpectrumWindow,
}

/// The cover shown large: the window's preview at first, then the picture
/// itself, read on a thread.
struct Zoom {
    small: Option<TextureHandle>,
    big: Option<(TextureHandle, String)>,
    job: Option<Receiver<Option<(egui::ColorImage, String)>>>,
}

/// The largest side of a cover shown large, in pixels.
const ZOOM_SIDE: u32 = 2560;

/// What a cover's thread did.
enum CoverDone {
    /// The picture (from its file) as it is to be written, its preview
    /// and sizes.
    Made(PathBuf, Result<crate::covers::ForTag, String>),
    /// The `folder.jpg` written.
    Saved(Result<PathBuf, String>),
}

/// What the cover's menu (or a picture dropped) asks for.
enum CoverAct {
    /// A picture chosen in a dialog.
    Choose,
    /// This picture (the folder's, or one dropped).
    File(PathBuf),
    Remove,
    /// The cover as it was (the change given up).
    Keep,
    /// The cover shown saved as the folder's `folder.jpg`.
    SaveToFolder,
    /// The largest side of a cover put into tags; a new one waiting is
    /// made again for it.
    Side(CoverSide),
}

/// A change of the cover waiting to be written with the fields, and the
/// new one's picture.
struct NewCover {
    change: CoverChange,
    preview: Option<TextureHandle>,
    /// The file it was made from.
    source: Option<PathBuf>,
}

/// What the window was asked to do this frame.
enum Act {
    Close,
    Edit,
    Cancel,
    Save,
    /// To a file, or to all of them.
    Go(Option<usize>),
    /// Editing one file: its changes written, then to the file given.
    SaveAndGo(usize),
    Cover(CoverAct),
    /// The cover shown large.
    Zoom,
    /// The lyrics from a text file chosen in a dialog.
    PickLyrics,
    /// LRCLIB's lyrics: of the file edited into its field, or of every
    /// file at once; `again`: asked anew (`Form::fetch_lyrics`).
    Lrclib { again: bool },
    /// The file's spectrum in a window of its own.
    Spectrum,
}

#[derive(Default, PartialEq)]
enum State {
    #[default]
    Reading,
    Ready,
    Saving,
}

enum Msg {
    Read(Vec<Result<Values, String>>),
    /// The files written, and the errors.
    Written(usize, Vec<String>),
}

#[derive(Default)]
struct Field {
    value: String,
    /// The files differ in it; left alone unless typed in.
    mixed: bool,
    /// The value shown at first (empty when mixed).
    shown: String,
    /// Typed in and not as it was shown: written.
    touched: bool,
}

impl Field {
    /// The field of files whose values are `values`.
    fn of<'a>(mut values: impl Iterator<Item = &'a String>) -> Field {
        let first = values.next().cloned().unwrap_or_default();
        let mixed = values.any(|v| *v != first);
        let value = if mixed { String::new() } else { first };
        Field {
            shown: value.clone(),
            value,
            mixed,
            touched: false,
        }
    }
}

/// The other fields of the files read: each key any of them has, in the
/// order they come, a file without it counting as empty (so a field some
/// of them lack shows "(different)").
fn other_fields(read: &[&Values]) -> Vec<(ItemKey, Field)> {
    let mut keys: Vec<ItemKey> = Vec::new();
    for v in read {
        for (k, _) in &v.other {
            if !keys.contains(k) {
                keys.push(*k);
            }
        }
    }
    let none = String::new();
    keys.into_iter()
        .map(|k| {
            let values = read.iter().map(|v| {
                v.other
                    .iter()
                    .find(|(o, _)| *o == k)
                    .map_or(&none, |(_, s)| s)
            });
            (k, Field::of(values))
        })
        .collect()
}

/// The text of a lyrics file: UTF-8 or UTF-16 by its BOM, else UTF-8,
/// else the ANSI code page of Windows (an old `.txt` or `.lrc`); its lines
/// ending with `\n`.
pub(crate) fn text_of_file(bytes: &[u8]) -> String {
    let utf16 = |bytes: &[u8], read: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| read(*c)).collect();
        String::from_utf16_lossy(&units)
    };
    let text = match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => crate::win::from_ansi(bytes),
        },
    };
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Why a file's lyrics were not asked for: its tags cannot be read
/// (LRCLIB's own failures are other words).
const UNREADABLE: &str = "the file cannot be read";

/// How many files' lyrics LRCLIB is asked for at once (all the files at
/// once): one after another, a dozen files took ten seconds and more.
const LRCLIB_AT_ONCE: usize = 4;

/// LRCLIB's lyrics of `path` by its tags (`typed`: the artist, title and
/// album typed in for it), its length from its file; `fresh`: LRCLIB asked
/// again, the cache passed by.
fn lrclib_of(path: &Path, typed: Option<&(String, String, String)>, fresh: bool) -> Found {
    use crate::lyrics::Fetched;
    let track = std::fs::metadata(path).ok().and_then(|m| {
        let mtime = m
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        crate::guard::catch(path, || crate::library::tags::read(path, m.len(), mtime)).flatten()
    });
    let Some(track) = track else {
        return Found::Failed(UNREADABLE.into());
    };
    let (artist, title, album) = match typed {
        Some(typed) => typed.clone(),
        None => {
            let artist = if track.artist.trim().is_empty() {
                track.album_artist.clone()
            } else {
                track.artist.clone()
            };
            (artist, track.title.clone(), track.album.clone())
        }
    };
    match crate::lyrics::fetch(&artist, &title, &album, track.duration_ms, fresh) {
        Ok(Fetched::Lyrics(text)) => Found::Lyrics(text),
        Ok(Fetched::Instrumental) => Found::Instrumental,
        Ok(Fetched::None) => Found::None,
        Err(e) => Found::Failed(e),
    }
}

/// The largest lyrics file read, in bytes.
const LYRICS_MOST: u64 = 1 << 20;

/// The tags of the files shown as `edit` has them: read on a thread,
/// changed in the text fields, written on a thread.
struct Form {
    paths: Vec<PathBuf>,
    state: State,
    rx: Receiver<Msg>,
    /// In `edit::Key::ALL`'s order.
    main: Vec<Field>,
    /// The other fields: of one file, or of all the files at once (each
    /// key any of them has, a file without it counting as empty).
    other: Vec<(ItemKey, Field)>,
    /// The usual fields every file's tag can hold, by group
    /// (`edit::Values::standard`); those not in `other` can be added.
    addable: Vec<(edit::Group, ItemKey)>,
    /// The lyrics of one file (all the files at once: none).
    lyrics: Option<Field>,
    /// A field just added takes the keys.
    focus_other: Option<ItemKey>,
    /// The files that could not be read or written, with why.
    errors: Vec<String>,
    /// The fields came while editing: the first one takes the keys.
    focus_first: bool,
    /// The cover put or removed, written with the fields.
    cover: Option<NewCover>,
    /// LRCLIB may be asked for lyrics (the setting, set by the window each
    /// frame).
    lrclib_on: bool,
    /// LRCLIB's lyrics looked for: of the one file, into its field; of
    /// every file at once, each written into its own file with the fields.
    lrclib: Option<Lrclib>,
    /// LRCLIB's lyrics found for this one file while all the files were
    /// shown: put into its field once it is read, as if typed in.
    prefill: Option<String>,
}

/// LRCLIB asked for the files' lyrics on a thread, `LRCLIB_AT_ONCE` files
/// at a time.
struct Lrclib {
    rx: Receiver<(usize, Found)>,
    /// Each file's, by its place in the form's files.
    found: Vec<Found>,
    /// Asked while editing one file: its lyrics go into the field.
    one: bool,
}

/// What came of a file's lyrics asked of LRCLIB.
#[derive(Clone, PartialEq, Debug)]
enum Found {
    Waiting,
    Lyrics(String),
    /// Its tag has lyrics already: left as they are (all the files at
    /// once).
    Has,
    Instrumental,
    None,
    /// LRCLIB could not be asked, or the file read (technical, English).
    Failed(String),
    /// Its lyrics were written, looked at on their own (`InfoView::go`).
    Written,
    /// Its lyrics were given up while looked at on their own.
    Declined,
}

impl Lrclib {
    fn busy(&self) -> bool {
        self.found.contains(&Found::Waiting)
    }

    /// The lyrics to write into each file (all the files at once).
    fn per_file(&self) -> Vec<Option<String>> {
        if self.one {
            return Vec::new();
        }
        self.found
            .iter()
            .map(|f| match f {
                Found::Lyrics(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// "Found 9 of 12; 2 have lyrics already; …" (all the files at once).
    fn summary(&self) -> String {
        let count = |f: fn(&Found) -> bool| self.found.iter().filter(|x| f(x)).count();
        let total = self.found.len();
        let found = count(|f| matches!(f, Found::Lyrics(_)));
        let has = count(|f| *f == Found::Has);
        let failed = count(|f| matches!(f, Found::Failed(_)));
        let done = total - count(|f| *f == Found::Waiting);
        let mut parts = vec![if self.busy() {
            tr!(
                format!("Looking: {done} of {total}"),
                format!("Поиск: {done} из {total}")
            )
        } else {
            tr!(
                format!("Found: {found} of {total}"),
                format!("Найдено: {found} из {total}")
            )
        }];
        let written = count(|f| *f == Found::Written);
        if written > 0 {
            parts.push(tr!(
                format!("{written} written"),
                format!("записано: {written}")
            ));
        }
        let declined = count(|f| *f == Found::Declined);
        if declined > 0 {
            parts.push(tr!(
                format!("{declined} given up"),
                format!("отменено: {declined}")
            ));
        }
        if has > 0 {
            parts.push(tr!(
                format!("{has} have lyrics already"),
                format!("уже с текстом: {has}")
            ));
        }
        if failed > 0 {
            parts.push(tr!(
                format!("{failed} not asked"),
                format!("не удалось: {failed}")
            ));
        }
        parts.join(" · ")
    }
}

impl Found {
    /// A word for the file's line in the list of files.
    fn word(&self) -> Option<&'static str> {
        Some(match self {
            Found::Waiting => return None,
            Found::Lyrics(_) => tr!("lyrics found", "текст найден"),
            Found::Has => tr!("has lyrics", "уже есть текст"),
            Found::Instrumental => tr!("an instrumental", "инструментальная"),
            Found::None => tr!("not in LRCLIB", "нет в LRCLIB"),
            Found::Failed(_) => tr!("not asked", "не удалось"),
            Found::Written => tr!("lyrics written", "текст записан"),
            Found::Declined => tr!("given up", "отменён"),
        })
    }
}

impl Lrclib {
    /// What came of each file before, kept while one of them was looked
    /// at on its own.
    fn kept(found: Vec<Found>) -> Lrclib {
        let (_, rx) = channel();
        Lrclib {
            rx,
            found,
            one: false,
        }
    }
}

/// What came to a form.
enum Polled {
    Nothing,
    /// The files were written (`any`: some of them), without an error
    /// (`ok`).
    Written {
        any: bool,
        ok: bool,
    },
}

impl Form {
    fn read(ctx: &egui::Context, paths: Vec<PathBuf>, version: Id3Version) -> Form {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let to_read = paths.clone();
        std::thread::spawn(move || {
            let read = to_read
                .iter()
                .map(|p| {
                    crate::guard::catch(p, || edit::read(p, version))
                        .unwrap_or_else(|| {
                            Err(tr!("cannot be read", "не удалось прочитать").to_owned())
                        })
                        .map_err(|e| format!("{}: {e}", p.display()))
                })
                .collect();
            let _ = tx.send(Msg::Read(read));
            ctx.request_repaint();
        });
        Form {
            paths,
            state: State::Reading,
            rx,
            main: Vec::new(),
            other: Vec::new(),
            addable: Vec::new(),
            lyrics: None,
            focus_other: None,
            errors: Vec::new(),
            focus_first: false,
            cover: None,
            lrclib_on: false,
            lrclib: None,
            prefill: None,
        }
    }

    /// LRCLIB asked for the lyrics on a thread: of the one file edited, by
    /// its fields as typed (its length from its tags), into its field; of
    /// all the files at once, each by its own tags, those whose tag has
    /// lyrics left alone. `again`: LRCLIB asked anew, the cache passed by,
    /// and of all the files at once only for those it had none for or did
    /// not answer, the rest kept as they came.
    fn fetch_lyrics(&mut self, ctx: &egui::Context, again: bool) {
        if self.state != State::Ready || self.lrclib.as_ref().is_some_and(Lrclib::busy) {
            return;
        }
        let one = self.paths.len() == 1 && self.lyrics.is_some();
        let before: Vec<Found> = match &self.lrclib {
            Some(l) if again && !one && !l.one => l.found.clone(),
            _ => Vec::new(),
        };
        let kept = |i: usize| {
            before
                .get(i)
                .filter(|f| !matches!(f, Found::None | Found::Failed(_)))
                .cloned()
        };
        let found: Vec<Found> = (0..self.paths.len())
            .map(|i| kept(i).unwrap_or(Found::Waiting))
            .collect();
        let asked: Vec<bool> = found.iter().map(|f| *f == Found::Waiting).collect();
        let typed = one.then(|| {
            let value = |k: edit::Key| self.main.get(k as usize).map(|f| f.value.trim().to_owned());
            let artist = value(edit::Key::Artist)
                .filter(|a| !a.is_empty())
                .or_else(|| value(edit::Key::AlbumArtist))
                .unwrap_or_default();
            (
                artist,
                value(edit::Key::Title).unwrap_or_default(),
                value(edit::Key::Album).unwrap_or_default(),
            )
        });
        let paths = self.paths.clone();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("lyrics lrclib".into())
            .spawn(move || {
                let queue: Vec<usize> = (0..paths.len()).filter(|&i| asked[i]).collect();
                let next = AtomicUsize::new(0);
                // LRCLIB not answering (no network, a stalled connection):
                // the files left are not asked, each would wait out its
                // time, but fail with its error; the refresh sign asks again.
                let down: Mutex<Option<String>> = Mutex::new(None);
                let work = || {
                    while let Some(&i) = queue.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let path = &paths[i];
                        let failed = down.lock().ok().and_then(|d| d.clone());
                        let found = if let Some(e) = failed {
                            Found::Failed(e)
                        } else if !one && crate::lyrics::tag_has_lyrics(path) {
                            Found::Has
                        } else {
                            let found = lrclib_of(path, typed.as_ref(), again);
                            if let Found::Failed(e) = &found
                                && e != UNREADABLE
                                && let Ok(mut d) = down.lock()
                            {
                                d.get_or_insert_with(|| e.clone());
                            }
                            found
                        };
                        if tx.send((i, found)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                };
                // Several at once, this thread one of them.
                std::thread::scope(|s| {
                    for _ in 1..queue.len().min(LRCLIB_AT_ONCE) {
                        let _ = std::thread::Builder::new()
                            .name("lyrics lrclib".into())
                            .spawn_scoped(s, work);
                    }
                    work();
                });
            });
        if let Err(e) = spawned {
            self.errors.push(e.to_string());
            return;
        }
        self.lrclib = Some(Lrclib { rx, found, one });
    }

    fn poll(&mut self) -> Polled {
        if let Some(lrclib) = &mut self.lrclib {
            for (i, found) in lrclib.rx.try_iter() {
                if lrclib.one
                    && let (Found::Lyrics(text), Some(field)) = (&found, self.lyrics.as_mut())
                {
                    field.value.clone_from(text);
                    field.touched = field.value != field.shown;
                }
                if let Some(f) = lrclib.found.get_mut(i) {
                    *f = found;
                }
            }
        }
        let mut out = Polled::Nothing;
        for msg in self.rx.try_iter().collect::<Vec<_>>() {
            match msg {
                Msg::Read(read) => {
                    let ok: Vec<&Values> = read.iter().filter_map(|r| r.as_ref().ok()).collect();
                    self.errors = read.iter().filter_map(|r| r.clone().err()).collect();
                    self.main = (0..edit::MAIN)
                        .map(|i| Field::of(ok.iter().map(|v| &v.main[i])))
                        .collect();
                    self.other = other_fields(&ok);
                    self.addable = match ok.first() {
                        Some(first) => first
                            .standard
                            .iter()
                            .filter(|(_, k)| {
                                ok.iter().all(|v| v.standard.iter().any(|(_, s)| s == k))
                            })
                            .copied()
                            .collect(),
                        None => Vec::new(),
                    };
                    self.lyrics = match ok.as_slice() {
                        [one] if self.paths.len() == 1 => {
                            Some(Field::of(std::iter::once(&one.lyrics)))
                        }
                        _ => None,
                    };
                    if let (Some(text), Some(field)) = (self.prefill.take(), self.lyrics.as_mut()) {
                        field.value = text.clone();
                        field.touched = field.value != field.shown;
                        let (_, rx) = channel();
                        self.lrclib = Some(Lrclib {
                            rx,
                            found: vec![Found::Lyrics(text)],
                            one: true,
                        });
                    }
                    self.state = State::Ready;
                }
                Msg::Written(written, errors) => {
                    self.state = State::Ready;
                    if errors.is_empty() {
                        self.lrclib = None;
                    }
                    out = Polled::Written {
                        any: written > 0,
                        ok: errors.is_empty(),
                    };
                    self.errors = errors;
                }
            }
        }
        out
    }

    fn fields(&self) -> impl Iterator<Item = &Field> {
        self.main
            .iter()
            .chain(self.other.iter().map(|(_, f)| f))
            .chain(self.lyrics.iter())
    }

    /// LRCLIB is being asked: nothing is saved nor another file gone to
    /// meanwhile, so no answer is lost.
    fn looking(&self) -> bool {
        self.lrclib.as_ref().is_some_and(Lrclib::busy)
    }

    /// Why another file cannot be gone to now (`all`: all the files are
    /// shown at once), while editing: LRCLIB is being asked, or changes
    /// wait (of all the files, those typed in: LRCLIB's lyrics of all the
    /// files go with the window, `InfoView::go`); going does not write
    /// them, "Save and next" does (`InfoView::save_and_go`).
    fn held(&self, all: bool) -> Option<&'static str> {
        if self.looking() {
            Some(tr!(
                "Looking on LRCLIB: wait for it to end",
                "Идёт поиск в LRCLIB: дождитесь его окончания"
            ))
        } else if all && self.typed() {
            Some(tr!(
                "Save or cancel the changes first",
                "Сначала сохраните или отмените изменения"
            ))
        } else if !all && self.touched() {
            Some(tr!(
                "Save or cancel the changes first, or use \"Save and next\" below",
                "Сначала сохраните или отмените изменения, или нажмите «Сохранить и далее» внизу"
            ))
        } else {
            None
        }
    }

    /// Fields typed in or a cover chosen.
    fn typed(&self) -> bool {
        self.fields().any(|f| f.touched) || self.cover.is_some()
    }

    fn touched(&self) -> bool {
        self.typed()
            || self
                .lrclib
                .as_ref()
                .is_some_and(|l| l.per_file().iter().any(Option::is_some))
    }

    /// Every number typed in is one.
    fn valid(&self) -> bool {
        edit::Key::ALL
            .iter()
            .zip(&self.main)
            .all(|(k, f)| !f.touched || k.takes(&f.value))
    }

    /// The fields as they were read.
    fn reset(&mut self) {
        let other = self.other.iter_mut().map(|(_, f)| f);
        for f in self.main.iter_mut().chain(other).chain(self.lyrics.iter_mut()) {
            f.value = f.shown.clone();
            f.touched = false;
        }
        self.cover = None;
        self.lrclib = None;
        self.errors.clear();
    }

    /// The fields typed in.
    fn changes(&self) -> Changes {
        let mut changes = Changes::default();
        for (c, f) in changes.main.iter_mut().zip(&self.main) {
            if f.touched {
                *c = Some(f.value.trim().to_owned());
            }
        }
        changes.other = self
            .other
            .iter()
            .filter(|(_, f)| f.touched)
            .map(|(k, f)| (*k, f.value.trim().to_owned()))
            .collect();
        changes.lyrics = self
            .lyrics
            .as_ref()
            .filter(|f| f.touched)
            .map(|f| f.value.clone());
        changes.cover = self.cover.as_ref().map(|c| c.change.clone());
        changes
    }

    /// The fields typed in written on a thread; the player lets go of each
    /// file while it is written (`releaser`: the track playing, the next).
    fn save(&mut self, ctx: &egui::Context, releaser: Option<Releaser>, how: edit::WriteTags) {
        let changes = self.changes();
        // LRCLIB's lyrics of each file (all the files at once).
        let per_file = self.lrclib.as_ref().map(Lrclib::per_file).unwrap_or_default();
        let paths = self.paths.clone();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut written = 0;
            let mut errors = Vec::new();
            for (i, p) in paths.iter().enumerate() {
                let mut changes = changes.clone();
                if let Some(Some(lyrics)) = per_file.get(i) {
                    changes.lyrics = Some(lyrics.clone());
                }
                // One file at a time: the ring covers a second.
                let held = releaser.as_ref().and_then(|r| r.hold(vec![p.clone()]));
                let result = crate::guard::catch(p, || edit::write(p, &changes, how))
                    .unwrap_or_else(|| {
                        Err(tr!("cannot be written", "не удалось записать").to_owned())
                    });
                drop(held);
                match result {
                    Ok(()) => written += 1,
                    Err(e) => errors.push(format!("{}: {e}", p.display())),
                }
            }
            let _ = tx.send(Msg::Written(written, errors));
            ctx.request_repaint();
        });
        self.rx = rx;
        self.errors.clear();
        self.state = State::Saving;
    }

    /// The value every file has, else empty.
    fn common(&self, key: edit::Key) -> &str {
        self.main
            .get(key as usize)
            .filter(|f| !f.mixed)
            .map_or("", |f| f.shown.as_str())
    }
}

impl InfoView {
    /// The window, writing with the player's `releaser`.
    pub fn new(releaser: Releaser) -> InfoView {
        InfoView {
            releaser: Some(releaser),
            ..Default::default()
        }
    }

    /// Opens the window for `paths` (the first of them, or, editing
    /// several, all of them at once), editing when `edit`; the files are
    /// read again (the tags may have been changed since).
    pub fn open(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, edit: bool) {
        if paths.is_empty() {
            return;
        }
        self.open = true;
        self.fresh = true;
        self.editing = edit;
        self.then = None;
        self.cover_note = None;
        // The cover shown large was another file's.
        self.zoom = None;
        let to = if edit && paths.len() > 1 {
            None
        } else {
            Some(0)
        };
        self.paths = paths;
        self.lrclib_found.clear();
        // Another file's details are not shown while these are read.
        self.shown = None;
        self.cover = None;
        if let Some(form) = self.form.take_if(|f| f.state == State::Saving) {
            self.finishing.push(form);
        }
        self.switch(ctx, to);
    }

    /// Shows the file `to` (none: all of them): its details asked for, the
    /// fields read for all of them or while editing.
    fn switch(&mut self, ctx: &egui::Context, to: Option<usize>) {
        self.at = to;
        let path = self.paths[to.unwrap_or(0)].clone();
        if to.is_some() && !edit::writable(&path) {
            self.editing = false;
        }
        self.form = match to {
            None => {
                let mut form = Form::read(ctx, self.paths.clone(), self.write_tags.id3v2);
                // LRCLIB's lyrics of all the files come back with them.
                if self.editing && !self.lrclib_found.is_empty() {
                    form.lrclib = Some(Lrclib::kept(std::mem::take(&mut self.lrclib_found)));
                }
                Some(form)
            }
            Some(i) if self.editing => {
                let mut form = Form::read(ctx, vec![path.clone()], self.write_tags.id3v2);
                form.prefill = self.found_lyrics(i);
                Some(form)
            }
            Some(_) => None,
        };
        if let Some(form) = &mut self.form {
            form.focus_first = self.editing;
        }
        self.request(ctx, &path);
    }

    /// Asks the "info" thread (started with the first file) for `path`.
    fn request(&mut self, ctx: &egui::Context, path: &Path) {
        self.wanted = Some(path.to_path_buf());
        if self.requests.is_none() {
            let (req_tx, req_rx) = channel::<PathBuf>();
            let (res_tx, res_rx) = channel();
            let ctx = ctx.clone();
            let started = std::thread::Builder::new()
                .name("info".into())
                .spawn(move || {
                    while let Ok(mut path) = req_rx.recv() {
                        // Only the last of the files asked for meanwhile.
                        while let Ok(next) = req_rx.try_recv() {
                            path = next;
                        }
                        let read = crate::guard::catch(&path, || details::read(&path))
                            .unwrap_or_else(|| Err("the file could not be read".into()));
                        if res_tx.send((path, read)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                });
            if let Err(e) = started {
                log::warn!("no info thread: {e}");
                return;
            }
            self.requests = Some(req_tx);
            self.results = Some(res_rx);
        }
        if let Some(tx) = &self.requests {
            let _ = tx.send(path.to_path_buf());
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.results else { return };
        while let Ok(mut read) = rx.try_recv() {
            if self.wanted.as_deref() != Some(read.0.as_path()) {
                continue;
            }
            self.cover = match &mut read.1 {
                Ok(d) => d.cover.take().map(|image| {
                    let texture = ctx.load_texture("file_info_cover", image, Default::default());
                    (read.0.clone(), texture)
                }),
                Err(_) => None,
            };
            self.shown = Some(read);
        }
    }

    /// What came to the form; whether files were written. Written without
    /// an error, the window goes where it was going, or stops editing; the
    /// files are read again either way.
    fn poll_form(&mut self, ctx: &egui::Context) -> bool {
        let mut written = false;
        self.finishing.retain_mut(|f| match f.poll() {
            Polled::Written { any, .. } => {
                written |= any;
                false
            }
            Polled::Nothing => true,
        });
        let Some(form) = &mut self.form else {
            return written;
        };
        let Polled::Written { any, ok } = form.poll() else {
            return written;
        };
        // One file's lyrics written (or cleared) while LRCLIB's of all the
        // files are kept: its line in the list says so.
        let lyrics = form
            .lyrics
            .as_ref()
            .filter(|f| f.touched)
            .map(|f| f.value.trim().is_empty());
        if ok
            && let (Some(i), Some(cleared)) = (self.at, lyrics)
            && let Some(found) = self.lrclib_found.get_mut(i)
        {
            *found = if cleared {
                Found::Declined
            } else {
                Found::Written
            };
        }
        if ok {
            let to = match self.then.take() {
                Some(to) => to,
                None => {
                    self.editing = false;
                    self.at
                }
            };
            if self.open {
                self.switch(ctx, to);
            } else {
                self.form = None;
            }
        } else {
            self.then = None;
        }
        written || any
    }

    /// Goes to the file `to` (or all of them), writing nothing: not while
    /// changes typed in wait (`held`; "Save and next" writes and goes).
    /// From all the files at once, LRCLIB's lyrics found go with the
    /// window, not written: the file gone to has its own in its field (the
    /// lyrics' tab shown), to be looked at, and the others are kept.
    fn go(&mut self, ctx: &egui::Context, to: Option<usize>) {
        if self.stuck() || self.held().is_some() {
            return;
        }
        if self.editing
            && self.at.is_none()
            && let Some(form) = &mut self.form
            && let Some(lrclib) = form.lrclib.take_if(|l| !l.one)
        {
            self.lrclib_found = lrclib.found;
            if let Some(i) = to
                && self.found_lyrics(i).is_some()
            {
                set_tab(ctx, Tab::Lyrics);
            }
        }
        // Changes typed in hold going (`held`): it writes nothing.
        self.switch(ctx, to);
    }

    /// "Save and next" / "Save and back": the file edited written when it
    /// holds changes, then the file `to` edited (not when writing failed:
    /// the error stays shown).
    fn save_and_go(&mut self, ctx: &egui::Context, to: usize) {
        if self.stuck() || self.at.is_none() || to >= self.paths.len() {
            return;
        }
        let Some(form) = &self.form else { return };
        if form.looking() || form.state != State::Ready {
            return;
        }
        if form.touched() {
            self.save(ctx, Some(Some(to)));
        } else {
            self.switch(ctx, Some(to));
        }
    }

    /// LRCLIB's lyrics found for the file `i` while all the files were
    /// edited, not yet written nor given up.
    fn found_lyrics(&self, i: usize) -> Option<String> {
        match self.lrclib_found.get(i) {
            Some(Found::Lyrics(text)) => Some(text.clone()),
            _ => None,
        }
    }

    /// Why another file cannot be gone to while editing (`Form::held`).
    fn held(&self) -> Option<&'static str> {
        let form = self.form.as_ref().filter(|_| self.editing)?;
        form.held(self.at.is_none())
    }

    /// A field typed in holds what cannot be written (a number that is
    /// not one, red): nothing is saved and no other file is gone to.
    fn stuck(&self) -> bool {
        self.editing
            && self
                .form
                .as_ref()
                .is_some_and(|f| f.touched() && !f.valid())
    }

    fn save(&mut self, ctx: &egui::Context, then: Option<Option<usize>>) {
        let Some(form) = &mut self.form else { return };
        if self.editing
            && form.state == State::Ready
            && form.touched()
            && form.valid()
            && !form.looking()
        {
            form.save(ctx, self.releaser.clone(), self.write_tags);
            self.then = then;
        }
    }

    fn start_editing(&mut self, ctx: &egui::Context) {
        if self.editing {
            return;
        }
        match self.at {
            Some(i) => {
                let path = self.paths[i].clone();
                if !edit::writable(&path) {
                    return;
                }
                let mut form = Form::read(ctx, vec![path], self.write_tags.id3v2);
                form.focus_first = true;
                form.prefill = self.found_lyrics(i);
                self.form = Some(form);
            }
            None => {
                let Some(form) = &mut self.form else { return };
                if form.state != State::Ready {
                    return;
                }
                form.reset();
                form.focus_first = true;
                if !self.lrclib_found.is_empty() {
                    form.lrclib = Some(Lrclib::kept(std::mem::take(&mut self.lrclib_found)));
                }
            }
        }
        self.editing = true;
    }

    fn stop_editing(&mut self) {
        if self.form.as_ref().is_some_and(|f| f.state == State::Saving) {
            return;
        }
        self.editing = false;
        self.then = None;
        match self.at {
            Some(i) => {
                // LRCLIB's lyrics put in the field are given up with it.
                let prefilled = self
                    .form
                    .as_ref()
                    .is_some_and(|f| f.lrclib.as_ref().is_some_and(|l| l.one));
                if prefilled
                    && let Some(found @ Found::Lyrics(_)) = self.lrclib_found.get_mut(i)
                {
                    *found = Found::Declined;
                }
                self.form = None;
            }
            None => {
                if let Some(form) = &mut self.form {
                    form.reset();
                }
            }
        }
    }

    fn act(&mut self, ctx: &egui::Context, act: Act) {
        match act {
            Act::Close => {
                self.open = false;
                self.zoom = None;
                self.editing = false;
                self.then = None;
                // A write under way goes on and is reported.
                if self
                    .form
                    .as_ref()
                    .is_some_and(|f| f.state != State::Saving)
                {
                    self.form = None;
                }
            }
            Act::Edit => self.start_editing(ctx),
            Act::Cancel => self.stop_editing(),
            Act::Save => self.save(ctx, None),
            Act::Go(to) => self.go(ctx, to),
            Act::SaveAndGo(to) => self.save_and_go(ctx, to),
            Act::Cover(act) => self.cover_act(ctx, act),
            Act::Zoom => self.zoom_in(ctx),
            Act::PickLyrics => self.pick_lyrics = true,
            Act::Lrclib { again } => {
                if let Some(form) = self.form.as_mut() {
                    form.fetch_lyrics(ctx, again);
                }
            }
            Act::Spectrum => {
                if let Some((path, Ok(d))) = &self.shown {
                    let top = Top::of(d);
                    let heading = match top.artist.as_str() {
                        "" => top.title,
                        artist => format!("{artist} – {}", top.title),
                    };
                    let facts = top.chips.join("  ·  ");
                    self.spectrum.open(ctx, path, heading, facts);
                }
            }
        }
    }

    /// `path`'s spectrum in its window (the key of "Spectrum" outside
    /// this window), with `heading` and `facts` over it.
    pub fn open_spectrum(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        heading: String,
        facts: String,
    ) {
        self.spectrum.open(ctx, path, heading, facts);
    }

    /// The spectrum's window asked for its picture to be saved: the file's
    /// name to offer and the window to own the dialog.
    pub fn spectrum_save_asked(&mut self) -> Option<(String, Option<isize>)> {
        std::mem::take(&mut self.spectrum.ask_save)
            .then(|| (self.spectrum.file_name(), self.spectrum.hwnd()))
    }

    /// The file chosen for the spectrum's picture.
    pub fn spectrum_to(&mut self, path: PathBuf) {
        self.spectrum.save_to(path);
    }

    /// The text file chosen in the dialog `pick_lyrics` asked for.
    pub fn lyrics_from(&mut self, path: PathBuf) {
        self.lyrics_file = Some(path);
    }

    /// The lyrics read from `path` into the field (a file too large or
    /// that cannot be read: an error under the fields).
    fn take_lyrics_file(&mut self, path: &Path) {
        let Some(form) = self.form.as_mut().filter(|f| f.state == State::Ready) else {
            return;
        };
        let Some(field) = form.lyrics.as_mut() else {
            return;
        };
        let read = std::fs::metadata(path)
            .and_then(|m| {
                if m.len() > LYRICS_MOST {
                    Err(std::io::Error::other(tr!(
                        "the file is larger than 1 MB",
                        "файл больше 1 МБ"
                    )))
                } else {
                    std::fs::read(path)
                }
            })
            .map(|bytes| text_of_file(&bytes));
        match read {
            Ok(text) => {
                field.value = text.trim_end().to_owned();
                field.touched = field.value != field.shown;
            }
            Err(e) => form.errors.push(format!("{}: {e}", path.display())),
        }
    }

    /// The picture of the cover shown, as it is: a new one waiting, else
    /// the tag's, else the folder's (as `details` finds the cover).
    fn cover_bytes(&self) -> Option<CoverSource> {
        let new = self
            .form
            .as_ref()
            .filter(|_| self.editing)
            .and_then(|f| f.cover.as_ref());
        match new.map(|c| &c.change) {
            Some(CoverChange::Set(data)) => return Some(CoverSource::Bytes(Arc::clone(data))),
            Some(CoverChange::Remove) => return None,
            None => {}
        }
        let (path, details) = self.shown.as_ref()?;
        let details = details.as_ref().ok()?;
        match &details.tag_cover {
            Some(bytes) => Some(CoverSource::Bytes(Arc::new(bytes.clone()))),
            None => Some(CoverSource::Folder(path.parent()?.to_path_buf())),
        }
    }

    /// The cover shown large; its picture read on a thread, the preview
    /// shown meanwhile.
    fn zoom_in(&mut self, ctx: &egui::Context) {
        let small = match self
            .form
            .as_ref()
            .filter(|_| self.editing)
            .and_then(|f| f.cover.as_ref())
        {
            Some(new) => new.preview.clone(),
            None => self.cover.as_ref().map(|(_, t)| t.clone()),
        };
        let Some(source) = self.cover_bytes() else {
            return;
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("cover zoom".into())
            .spawn(move || {
                let _ = tx.send(source.picture());
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            log::warn!("no cover thread: {e}");
        }
        self.zoom = Some(Zoom {
            small,
            big: None,
            job: Some(rx),
        });
    }

    /// The cover large over the window while it is open: the picture fit
    /// to the window (a small one at most twice its size), its size and
    /// format under it. A click, Esc or Enter closes it.
    fn show_zoom(&mut self, ctx: &egui::Context) {
        let Some(zoom) = &mut self.zoom else { return };
        if let Some(rx) = &zoom.job {
            match rx.try_recv() {
                Ok(read) => {
                    zoom.job = None;
                    let options = egui::TextureOptions::LINEAR
                        .with_mipmap_mode(Some(egui::TextureFilter::Linear));
                    zoom.big = read.map(|(image, note)| {
                        (ctx.load_texture("file_info_zoom", image, options), note)
                    });
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => zoom.job = None,
            }
        }
        let texture = zoom
            .big
            .as_ref()
            .map(|(t, _)| t)
            .or(zoom.small.as_ref())
            .cloned();
        let Some(texture) = texture else {
            // Nothing could be read: nothing to show.
            if zoom.job.is_none() {
                self.zoom = None;
            }
            return;
        };
        let note = zoom.big.as_ref().map(|(_, n)| n.clone());
        let reading = zoom.job.is_some();
        let room = ctx.content_rect().size() * 0.9 - vec2(0.0, 24.0);
        let natural = texture.size_vec2() / ctx.pixels_per_point();
        // The preview is small: its picture's own size is not known yet.
        let most = if zoom.big.is_some() {
            natural * 2.0
        } else {
            room
        };
        let scale = (room.x / natural.x)
            .min(room.y / natural.y)
            .min(most.x / natural.x)
            .max(0.01);
        let size = natural * scale;
        let modal = egui::Modal::new(egui::Id::new("file_info_zoom"))
            .frame(egui::Frame::NONE)
            .backdrop_color(egui::Color32::from_black_alpha(225))
            .show(ctx, |ui| {
                // The caption's centred layout otherwise keeps the area's
                // previous width, leaving a resized image off-centre.
                ui.set_width(size.x);
                let image = ui.add(
                    egui::Image::new((texture.id(), size))
                        .corner_radius(8.0)
                        .sense(egui::Sense::click()),
                );
                if reading {
                    egui::Spinner::new().size(28.0).paint_at(
                        ui,
                        egui::Rect::from_center_size(image.rect.center(), vec2(28.0, 28.0)),
                    );
                }
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    if let Some(note) = note {
                        egui::Frame::NONE
                            .fill(egui::Color32::from_black_alpha(180))
                            .corner_radius(6.0)
                            .inner_margin(egui::Margin::symmetric(10, 4))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(note)
                                        .color(egui::Color32::from_white_alpha(220)),
                                );
                            });
                    }
                });
                image.clicked()
            });
        let keys = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, Key::Escape)
                | i.consume_key(egui::Modifiers::NONE, Key::Enter)
                | i.consume_key(egui::Modifiers::NONE, Key::Space)
        });
        if modal.inner || modal.backdrop_response.clicked() || keys {
            self.zoom = None;
        }
    }

    /// The picture chosen in the dialog `pick_cover` asked for.
    pub fn cover_from(&mut self, path: PathBuf) {
        self.cover_file = Some(path);
    }

    /// The window of its own, for a dialog to be owned by; none: the
    /// program's.
    pub fn owner(&self) -> Option<isize> {
        self.windowed.then(|| self.window.hwnd()).flatten()
    }

    /// A change of the cover, editing first (a change waits for Save);
    /// saving it into the folder is done at once.
    fn cover_act(&mut self, ctx: &egui::Context, act: CoverAct) {
        self.cover_note = None;
        match act {
            CoverAct::SaveToFolder => {
                self.save_to_folder(ctx);
                return;
            }
            CoverAct::Side(side) => {
                self.cover_side = side;
                let source = self
                    .form
                    .as_ref()
                    .filter(|_| self.editing)
                    .and_then(|f| f.cover.as_ref())
                    .and_then(|c| c.source.clone());
                if let Some(source) = source {
                    self.make_cover(ctx, source);
                }
                return;
            }
            _ => {}
        }
        if !self.editing {
            self.start_editing(ctx);
            if !self.editing {
                return;
            }
        }
        match act {
            CoverAct::Choose => self.pick_cover = true,
            CoverAct::File(path) => self.make_cover(ctx, path),
            CoverAct::Remove | CoverAct::Keep => {
                if let Some(form) = &mut self.form {
                    form.cover = matches!(act, CoverAct::Remove).then_some(NewCover {
                        change: CoverChange::Remove,
                        preview: None,
                        source: None,
                    });
                }
            }
            CoverAct::SaveToFolder | CoverAct::Side(_) => {}
        }
    }

    /// The picture at `path` made ready for the tags on a thread.
    fn make_cover(&mut self, ctx: &egui::Context, path: PathBuf) {
        let side = self.cover_side.pixels();
        self.cover_job = Some(spawn_cover(ctx, move || {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let made = crate::guard::catch(&path, || {
                let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                if size > MAX_PICTURE {
                    return Err(tr!("the picture is too large", "картинка слишком большая").into());
                }
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                crate::covers::for_tag(bytes, side, details::COVER_SIDE)
            })
            .unwrap_or_else(|| Err(tr!("cannot be read", "не удалось прочитать").into()));
            let made = made.map_err(|e| format!("{name}: {e}"));
            CoverDone::Made(path, made)
        }));
    }

    /// The cover shown (a new one, else the tag's) saved as its folder's
    /// `folder.jpg`, on a thread.
    fn save_to_folder(&mut self, ctx: &egui::Context) {
        let Some((folder, bytes)) = self.folder_cover() else {
            return;
        };
        self.cover_job = Some(spawn_cover(ctx, move || {
            CoverDone::Saved(crate::covers::save_folder_jpg(&folder, &bytes))
        }));
    }

    /// The folder of the files shown (all of them in one), and the cover
    /// that would be saved into it: the new one, else the tag's.
    fn folder_cover(&self) -> Option<(PathBuf, Arc<Vec<u8>>)> {
        let folder = self.folder()?;
        let new = self
            .form
            .as_ref()
            .filter(|_| self.editing)
            .and_then(|f| f.cover.as_ref());
        let bytes = match new.map(|c| &c.change) {
            Some(CoverChange::Set(data)) => Arc::clone(data),
            Some(CoverChange::Remove) => return None,
            None => match &self.shown {
                Some((path, Ok(d))) if path.parent() == Some(&folder) => {
                    Arc::new(d.tag_cover.clone()?)
                }
                _ => return None,
            },
        };
        Some((folder, bytes))
    }

    /// The folder of the file shown, or of all the files when they are of
    /// one.
    fn folder(&self) -> Option<PathBuf> {
        let mut paths = match self.at {
            Some(i) => self.paths.get(i..=i)?.iter(),
            None => self.paths.iter(),
        };
        let folder = paths.next()?.parent()?;
        paths
            .all(|p| p.parent() == Some(folder))
            .then(|| folder.to_path_buf())
    }

    /// What came from the cover's thread.
    fn poll_cover(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.cover_job else { return };
        let done = match rx.try_recv() {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.cover_job = None;
                return;
            }
        };
        self.cover_job = None;
        match done {
            CoverDone::Made(source, Ok(made)) => {
                if let Some(form) = self.form.as_mut().filter(|_| self.editing) {
                    self.cover_note = Some((made_note(&made), false));
                    let preview =
                        ctx.load_texture("file_info_new_cover", made.preview, Default::default());
                    form.cover = Some(NewCover {
                        change: CoverChange::Set(Arc::new(made.data)),
                        preview: Some(preview),
                        source: Some(source),
                    });
                }
            }
            CoverDone::Saved(Ok(path)) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.cover_note = Some((
                    tr!(format!("Saved {name}"), format!("Сохранено: {name}")),
                    false,
                ));
                self.folder_saved = path.parent().map(Path::to_path_buf);
                // A file without a cover in its tag shows the folder's.
                if let Some(wanted) = self.wanted.clone() {
                    self.request(ctx, &wanted);
                }
            }
            CoverDone::Made(_, Err(e)) | CoverDone::Saved(Err(e)) => {
                self.cover_note = Some((e, true));
            }
        }
    }

    /// What the cover's menu offers now.
    fn cover_menu(&self, dropping: bool) -> CoverMenu {
        let new = self
            .form
            .as_ref()
            .filter(|_| self.editing)
            .and_then(|f| f.cover.as_ref());
        let in_tag = match self.at {
            Some(_) => self.shown.as_ref().is_some_and(|(p, d)| {
                Some(p) == self.wanted.as_ref() && d.as_ref().is_ok_and(|d| d.tag_cover.is_some())
            }),
            // All the files: some of them may have one.
            None => true,
        };
        CoverMenu {
            writable: self.paths.iter().any(|p| edit::writable(p)),
            folder: self.folder(),
            changed: new.is_some(),
            removable: match new.map(|c| &c.change) {
                Some(CoverChange::Set(_)) => true,
                Some(CoverChange::Remove) => false,
                None => in_tag,
            },
            savable: self.folder_cover().is_some(),
            side: self.cover_side,
            dropping,
            busy: self.cover_job.is_some(),
            note: self.cover_note.clone(),
        }
    }

    /// The window's keys: Esc or Enter closes it, the keys of "Edit
    /// tags" edit; editing, Esc gives up and Enter (no field holding it)
    /// saves.
    fn keys(&self, ctx: &egui::Context, keys: &Keymap, acts: &mut Vec<Act>) {
        // The cover shown large takes the keys.
        if self.zoom.is_some() {
            return;
        }
        if self.editing {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                acts.push(Act::Cancel);
            }
            if ctx.memory(|m| m.focused().is_none())
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
            {
                acts.push(Act::Save);
            }
        } else {
            if ctx.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter)) {
                acts.push(Act::Close);
            }
            let mut chords = keys.chords(Action::EditTags).to_vec();
            chords.extend_from_slice(keys.chords(Action::Spectrum));
            let cmds = crate::keymap::take(ctx, keys, |c| chords.contains(c));
            if cmds.contains(&Cmd::EditTags) {
                acts.push(Act::Edit);
            }
            if cmds.contains(&Cmd::Spectrum) {
                acts.push(Act::Spectrum);
            }
        }
    }

    /// The library whose genres the genre's field offers: they are
    /// gathered again when it is another (each frame while editing).
    pub fn set_library(&mut self, library: &crate::library::Library) {
        if self.editing && self.genres_of != Some(library.generation) {
            self.genres = crate::library::genres::of_library(library);
            self.genres_of = Some(library.generation);
        }
    }

    /// Draws the window while it is open; whether files were written (the
    /// library reads them again).
    pub fn show(&mut self, ctx: &egui::Context, owner: Option<isize>, keys: &Keymap) -> bool {
        self.spectrum.show(ctx, owner);
        let lrclib = self.lrclib;
        if let Some(form) = self.form.as_mut() {
            form.lrclib_on = lrclib;
        }
        self.poll(ctx);
        self.poll_cover(ctx);
        let written = self.poll_form(ctx);
        if !self.open {
            return written;
        }
        if let Some(path) = self.cover_file.take() {
            self.cover_act(ctx, CoverAct::File(path));
        }
        if let Some(path) = self.lyrics_file.take() {
            // The lyrics taken in are shown.
            set_tab(ctx, Tab::Lyrics);
            self.take_lyrics_file(&path);
        }
        // `METALAMP_INFO_ZOOM=1`: the cover large once it is read (GUI
        // checks, with `METALAMP_INFO`).
        if self.zoom.is_none()
            && self.cover.is_some()
            && ctx.cumulative_frame_nr() < 300
            && std::env::var_os("METALAMP_INFO_ZOOM").is_some()
            && !ctx.data(|d| d.get_temp::<bool>(egui::Id::new("info_zoomed")).unwrap_or(false))
        {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("info_zoomed"), true));
            self.zoom_in(ctx);
        }
        let fresh = std::mem::take(&mut self.fresh);
        if fresh {
            let room = ctx.content_rect().size();
            self.windowed = room.x < WIDTH + 64.0 || room.y < 420.0;
            self.window.reset();
        }
        let mut acts = Vec::new();
        if self.windowed {
            self.window(ctx, owner, fresh, keys, &mut acts);
        } else {
            let modal = egui::Modal::new(egui::Id::new("file_info"))
                .frame(
                    egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
                        .fill(palette::panel_bg())
                        .inner_margin(egui::Margin::same(20))
                        .corner_radius(10.0),
                )
                .show(ctx, |ui| {
                    let height = ctx.content_rect().height();
                    self.content(ui, Some(height), &mut acts);
                    self.show_zoom(ctx);
                });
            if !fresh {
                self.keys(ctx, keys, &mut acts);
                if !self.editing && modal.should_close() {
                    acts.push(Act::Close);
                }
            }
        }
        for act in acts {
            self.act(ctx, act);
        }
        written
    }

    /// The info in a window of its own (the settings' way): frameless,
    /// owned by the program's window, centred over it; moved by its top,
    /// closed by the cross, Esc or Enter.
    fn window(
        &mut self,
        ctx: &egui::Context,
        owner: Option<isize>,
        fresh: bool,
        keys: &Keymap,
        acts: &mut Vec<Act>,
    ) {
        let size = vec2(WIDTH + 40.0, WINDOW_HEIGHT);
        let centre = ctx
            .input(|i| i.viewport().outer_rect)
            .unwrap_or(ctx.content_rect())
            .center();
        let builder = super::OwnWindow::builder(egui::ViewportBuilder::default())
            .with_title(window_title())
            .with_decorations(false)
            .with_resizable(true)
            .with_inner_size(size)
            .with_min_inner_size(vec2(WIDTH + 40.0, 360.0))
            .with_position(centre - size / 2.0)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            .with_icon(crate::window_icon());
        let id = egui::ViewportId::from_hash_of("file_info_window");
        let dark = palette::current().dark;
        ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            self.window.follow(&ctx, window_title(), owner, dark);
            if ctx.input(|i| i.viewport().close_requested()) {
                acts.push(Act::Close);
            }
            crate::frameless::resize_edges(ui, true);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(palette::panel_bg())
                        .inner_margin(egui::Margin::same(20)),
                )
                .show(ui, |ui| {
                    // Its top moves it; the widgets on it come later and
                    // take their clicks first.
                    let top = egui::Rect::from_min_size(
                        ctx.content_rect().min,
                        vec2(ctx.content_rect().width(), 20.0 + COVER),
                    );
                    crate::frameless::drag_area(
                        ui,
                        top,
                        ui.id().with("info_drag"),
                        egui::Sense::hover(),
                    );
                    self.content(ui, None, acts);
                });
            self.show_zoom(&ctx);
            if !fresh {
                self.keys(&ctx, keys, acts);
            }
        });
    }

    /// The window's content: the hero and the body in a scroll area for
    /// `height` (the program's window), or the rest of a window of its own
    /// (none); editing, Cancel and Save under it.
    fn content(&mut self, ui: &mut Ui, height: Option<f32>, acts: &mut Vec<Act>) {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
        crate::ui::settings::round_widgets(ui);
        let wanted = self.wanted.clone().unwrap_or_default();
        // Going to another file, the one before stays until it is read: a
        // window shrunk to "Reading..." for a moment and grown back blinked.
        let shown = self.shown.as_ref().map(|(_, d)| d);
        let cover = self.cover.as_ref().map(|(_, t)| t.clone());
        let saving = self
            .form
            .as_ref()
            .is_some_and(|f| f.state == State::Saving);
        let nav = Nav {
            at: self.at,
            count: self.paths.len(),
            enabled: !saving && !self.stuck(),
            held: self.held(),
        };
        let top = match (self.at, shown) {
            (None, _) => Top::all(self.paths.len(), self.form.as_ref()),
            (Some(_), Some(Ok(d))) => Top::of(d),
            (Some(_), _) => Top::named(&wanted),
        };
        // All the files: their cover when they are of one folder.
        let one_folder = self.paths.windows(2).all(|w| w[0].parent() == w[1].parent());
        let cover = cover.filter(|_| self.at.is_some() || one_folder);
        // A new cover waiting for Save is shown in its place (a removed
        // one as none).
        let new = self
            .form
            .as_ref()
            .filter(|_| self.editing)
            .and_then(|f| f.cover.as_ref());
        let cover = match new {
            Some(c) => c.preview.clone(),
            None => cover,
        };
        // A picture dropped on the window becomes the cover.
        let (dropped, dropping) = ui.ctx().input(|i| {
            let dropped = i
                .raw
                .dropped_files
                .iter()
                .map(|f| f.path())
                .find(|p| crate::covers::is_picture(p))
                .map(Path::to_path_buf);
            let dropping = i
                .raw
                .hovered_files
                .iter()
                .any(|f| f.path.as_deref().is_none_or(crate::covers::is_picture));
            (dropped, dropping)
        });
        if let Some(path) = dropped {
            acts.push(Act::Cover(CoverAct::File(path)));
        }
        let menu = self.cover_menu(dropping);
        hero(ui, &top, cover.as_ref(), &menu, &nav, acts);

        let errors = self.form.as_ref().is_some_and(|f| !f.errors.is_empty());
        let footer = match (self.editing, errors) {
            (false, _) => 0.0,
            (true, false) => FOOTER,
            (true, true) => FOOTER + ERRORS + 6.0,
        };
        // A long tag (lyrics, many fields) scrolls within the screen
        // instead of pushing the window off it. The height is forced
        // (`min_scrolled_height`), not left to the room below: a centred
        // modal is placed by its size of the frame before, so an area that
        // only took the room left froze at the height the window had while
        // the file was read. In a window of its own (`height` none) the
        // area takes the rest of it.
        let room = height.map_or(ui.available_height() - footer, |h| {
            (h - 2.0 * 20.0 - COVER - 60.0 - footer).max(120.0)
        });
        let editing = self.editing;
        let genres = &self.genres;
        let reading = |ui: &mut Ui| {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(tr!("Reading…", "Чтение…"));
            });
        };
        match (self.at, shown) {
            (None, _) => match &mut self.form {
                Some(form) => {
                    let paths = &self.paths;
                    scrolled(ui, room, |ui| {
                        ui.add_enabled_ui(!saving, |ui| {
                            body_all(ui, form, paths, editing, genres, acts)
                        });
                    });
                }
                None => reading(ui),
            },
            (Some(_), None) => reading(ui),
            (Some(_), Some(Err(e))) => {
                ui.add_space(16.0);
                ui.colored_label(
                    palette::error(),
                    tr!(
                        format!("Cannot read the file: {e}"),
                        format!("Не удалось прочитать файл: {e}")
                    ),
                );
            }
            (Some(_), Some(Ok(d))) => {
                // Gone to another file, the one before stays shown until
                // it is read: its fields are shown as they are, not in
                // the other file's form.
                let form = self
                    .form
                    .as_mut()
                    .filter(|f| editing && f.paths.first() == Some(&d.path));
                let own = height.is_none();
                tabbed(ui, d, form, genres, room, own, saving, acts);
            }
        }
        if editing {
            // Its right edge where the fields end, left of the scroll bar.
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    right: 14,
                    ..egui::Margin::ZERO
                })
                .show(ui, |ui| {
                    // One of several files: the files before and after it.
                    let steps = self.at.filter(|_| self.paths.len() > 1).map(|i| {
                        let next = Some(i + 1).filter(|&n| n < self.paths.len());
                        (i.checked_sub(1), next)
                    });
                    footer_row(ui, self.form.as_ref(), steps, acts)
                });
        }
    }
}

/// The body in a scroll area of `room` points, with room for the scroll
/// bar at the right edge (`set_max_width` here also shrank the area's
/// height to its minimum); the area's height: its content's, at most
/// `room` (not `inner_rect`'s, which is the area before it shrinks to
/// the content: `room`).
fn scrolled(ui: &mut Ui, room: f32, add: impl FnOnce(&mut Ui)) -> f32 {
    let shown = egui::ScrollArea::vertical()
        .id_salt("file_info_body")
        .min_scrolled_height(room)
        .max_height(room)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            right_margin().show(ui, add);
        });
    shown.content_size.y.min(room)
}

/// One file's body: the tabs' row and the tab shown, in `room` (`own`: the
/// rest of a window of its own).
#[allow(clippy::too_many_arguments)]
fn tabbed(
    ui: &mut Ui,
    d: &Details,
    mut form: Option<&mut Form>,
    genres: &[String],
    room: f32,
    own: bool,
    saving: bool,
    acts: &mut Vec<Act>,
) {
    match tab_row(ui, d, form.as_deref_mut(), saving, acts) {
        Tab::Main => {
            scrolled(ui, room, |ui| {
                ui.add_enabled_ui(!saving, |ui| body_one(ui, d, form, genres));
            });
        }
        Tab::Lyrics => {
            // As tall as the main tab would be, so the window keeps its
            // size between the tabs: that tab laid out unseen, in a child
            // that takes no room. In a window of its own, the rest of it.
            let tall = if own {
                room
            } else {
                let mut probe = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(ui.available_rect_before_wrap())
                        .invisible(),
                );
                scrolled(&mut probe, room, |ui| {
                    body_one(ui, d, form.as_deref_mut(), genres)
                })
            };
            ui.add_enabled_ui(!saving, |ui| lyrics_tab(ui, d, form, tall));
        }
    }
}

/// The body's margin on the right, where the scroll bar goes.
fn right_margin() -> egui::Frame {
    egui::Frame::new().inner_margin(egui::Margin {
        right: 14,
        ..egui::Margin::ZERO
    })
}

/// The tabs of one file's body: the tags (with the file's lines) and the
/// lyrics, apart since a song's lyrics shown whole made the window twice
/// as tall. Kept for the session in egui's memory.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Main,
    Lyrics,
}

fn tab(ctx: &egui::Context) -> Tab {
    ctx.data(|d| d.get_temp(tab_id()).unwrap_or(Tab::Main))
}

fn set_tab(ctx: &egui::Context, tab: Tab) {
    ctx.data_mut(|d| d.insert_temp(tab_id(), tab));
}

fn tab_id() -> egui::Id {
    egui::Id::new("info_tab")
}

/// The tabs' row under the hero, a line under it and the tab shown
/// underlined in the accent; the lyrics' lines counted beside their name,
/// a dot in the accent by a tab not shown that holds changes not saved.
/// On the right what the "Main" section's heading had (the tags in the
/// file and Edit; no heading repeats the tab's name under it), and on the
/// lyrics' tab while editing Clear and From a file. The tab shown.
fn tab_row(
    ui: &mut Ui,
    d: &Details,
    form: Option<&mut Form>,
    saving: bool,
    acts: &mut Vec<Act>,
) -> Tab {
    let mut shown = tab(ui.ctx());
    let editing = form.is_some();
    let (count, touched) = {
        let form = form.as_deref();
        let lyrics = match form.and_then(|f| f.lyrics.as_ref()) {
            Some(f) => f.value.as_str(),
            None => value(d, &[ItemKey::UnsyncLyrics, ItemKey::Lyrics]),
        };
        let touched = |t: Tab| {
            form.is_some_and(|f| match t {
                Tab::Lyrics => f.lyrics.as_ref().is_some_and(|l| l.touched),
                Tab::Main => {
                    f.cover.is_some()
                        || f.main
                            .iter()
                            .chain(f.other.iter().map(|(_, o)| o))
                            .any(|o| o.touched)
                }
            })
        };
        (lyrics_lines(lyrics), [touched(Tab::Main), touched(Tab::Lyrics)])
    };
    ui.add_space(14.0);
    right_margin().show(ui, |ui| {
        let mut underlined = egui::Rect::NOTHING;
        let row = ui.horizontal(|ui| {
            let gap = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = 22.0;
            for (t, name) in [
                (Tab::Main, tr!("General", "Основные")),
                (Tab::Lyrics, tr!("Lyrics", "Текст")),
            ] {
                let on = t == shown;
                let note = (t == Tab::Lyrics && count > 0).then(|| count.to_string());
                let dot = !on && touched[t as usize];
                let mut resp = tab_button(ui, name, note, on, dot);
                if t == Tab::Lyrics {
                    resp = resp.on_hover_text(lyrics_note(count));
                }
                if resp.clicked() && !on {
                    set_tab(ui.ctx(), t);
                    shown = t;
                }
                if t == shown {
                    underlined = resp.rect;
                }
            }
            ui.spacing_mut().item_spacing.x = gap;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!saving, |ui| {
                    // Editing, until the fields are read: nothing to clear.
                    let form = form.filter(|f| f.state != State::Reading && f.lyrics.is_some());
                    match (shown, form) {
                        (Tab::Lyrics, Some(form)) => lyrics_tools(ui, form, acts),
                        _ => {
                            if !editing {
                                let why_not = (!edit::writable(&d.path)).then_some(tr!(
                                    "Tags of this format cannot be written",
                                    "Теги этого формата записать нельзя"
                                ));
                                edit_button(ui, why_not, acts);
                                ui.add_space(8.0);
                            }
                            ui.label(
                                RichText::new(d.tags.join(" · "))
                                    .size(11.0)
                                    .color(palette::text_weak()),
                            );
                        }
                    }
                });
            });
        });
        let r = ui.available_rect_before_wrap();
        let y = row.response.rect.bottom() + 4.0;
        let painter = ui.painter();
        painter.hline(r.left()..=r.right(), y, egui::Stroke::new(1.0, palette::line()));
        painter.hline(
            underlined.left()..=underlined.right(),
            y,
            egui::Stroke::new(2.0, palette::accent()),
        );
        ui.add_space(6.0);
    });
    shown
}

/// Clear, From LRCLIB (while the setting allows it) and From a file, right
/// to left, for the lyrics being edited; what came of LRCLIB left of them.
fn lyrics_tools(ui: &mut Ui, form: &mut Form, acts: &mut Vec<Act>) {
    if ui
        .small_button(tr!("From a file…", "Из файла…"))
        .on_hover_text(tr!(
            "The lyrics from a text file (.txt, .lrc)",
            "Текст из текстового файла (.txt, .lrc)"
        ))
        .clicked()
    {
        acts.push(Act::PickLyrics);
    }
    let lrclib = form.lrclib.as_ref().filter(|l| l.one);
    if form.lrclib_on {
        let busy = lrclib.is_some_and(Lrclib::busy);
        let button = egui::Button::new(tr!("From LRCLIB", "Из LRCLIB")).small();
        if ui
            .add_enabled(!busy, button)
            .on_hover_text(tr!(
                "The lyrics from LRCLIB (lrclib.net) by the artist, title and album typed in and the track's length, synced ones with their times; written with the fields",
                "Текст из LRCLIB (lrclib.net) по введённым исполнителю, названию и альбому и длительности трека, синхронизированный — с метками времени; записывается вместе с полями"
            ))
            .clicked()
        {
            acts.push(Act::Lrclib { again: false });
        }
    }
    if let Some(field) = form.lyrics.as_mut() {
        let clear = egui::Button::new(tr!("Clear", "Очистить")).small();
        if ui.add_enabled(!field.value.is_empty(), clear).clicked() {
            field.value.clear();
            field.touched = field.value != field.shown;
        }
    }
    let answered = lrclib.and_then(|l| l.found.first());
    if form.lrclib_on
        && answered.is_some_and(|f| *f != Found::Waiting)
        && ask_again_button(
            ui,
            tr!(
                "Ask LRCLIB again, not taking its answer kept",
                "Запросить в LRCLIB заново, не беря сохранённый ответ"
            ),
        )
    {
        acts.push(Act::Lrclib { again: true });
    }
    match answered {
        Some(Found::Waiting) => {
            ui.spinner();
        }
        Some(Found::Failed(e)) => {
            ui.label(RichText::new(tr!("LRCLIB did not answer", "LRCLIB не ответил")).color(palette::error()))
                .on_hover_text(e.as_str());
        }
        Some(other) => {
            let word = match other {
                Found::Lyrics(_) => tr!("from LRCLIB", "из LRCLIB"),
                Found::Instrumental => tr!("LRCLIB: an instrumental", "LRCLIB: инструментальная"),
                _ => tr!("not in LRCLIB", "нет в LRCLIB"),
            };
            ui.label(RichText::new(word).color(palette::text_weak()));
        }
        None => {}
    }
}

/// The refresh sign as a button of a small button's height, LRCLIB asked
/// again.
fn ask_again_button(ui: &mut Ui, tip: &str) -> bool {
    use crate::ui::icons::{self, Icon};
    let height = ui.spacing().interact_size.y;
    let (rect, resp) = ui.allocate_exact_size(vec2(height + 4.0, height), egui::Sense::click());
    let fill = if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, palette::hover());
        palette::hover()
    } else {
        palette::panel_bg()
    };
    let colour = if resp.hovered() {
        palette::text_bright()
    } else {
        palette::text()
    };
    icons::paint(ui.painter(), Icon::Refresh, rect.center(), 13.0, colour, fill);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
        .clicked()
}

/// All the files at once, while editing: LRCLIB asked for every file's
/// lyrics (those with lyrics left alone), what came of it, written with
/// the fields.
fn lyrics_all(ui: &mut Ui, form: &Form, acts: &mut Vec<Act>) {
    ui.add_space(14.0);
    let lrclib = form.lrclib.as_ref().filter(|l| !l.one);
    let busy = lrclib.is_some_and(Lrclib::busy);
    section(ui, tr!("Lyrics", "Тексты песен"), |ui| {
        let button = egui::Button::new(tr!("Look on LRCLIB", "Найти в LRCLIB")).small();
        if ui
            .add_enabled(!busy, button)
            .on_hover_text(tr!(
                "Each file's lyrics from LRCLIB (lrclib.net) by its artist, title, album and length, synced ones with their times; files whose tag has lyrics are left as they are. Written with the fields.",
                "Текст каждого файла из LRCLIB (lrclib.net) по его исполнителю, названию, альбому и длительности, синхронизированный — с метками времени; файлы, в теге которых текст уже есть, не меняются. Записывается вместе с полями."
            ))
            .clicked()
        {
            acts.push(Act::Lrclib { again: false });
        }
    });
    if let Some(lrclib) = lrclib {
        ui.horizontal(|ui| {
            if busy {
                ui.spinner();
            }
            ui.label(RichText::new(lrclib.summary()).color(palette::text_weak()));
            let missing = lrclib
                .found
                .iter()
                .any(|f| matches!(f, Found::None | Found::Failed(_)));
            if !busy
                && missing
                && ask_again_button(
                    ui,
                    tr!(
                        "Ask LRCLIB again for the files it had no lyrics for or did not answer",
                        "Запросить заново в LRCLIB тексты файлов, для которых их не нашлось или LRCLIB не ответил"
                    ),
                )
            {
                acts.push(Act::Lrclib { again: true });
            }
        });
    }
}

/// A tab's name (bright when `on`; its underline is the row's), a weak
/// `note` after it and a `dot` in the accent.
fn tab_button(ui: &mut Ui, name: &str, note: Option<String>, on: bool, dot: bool) -> egui::Response {
    let font = egui::FontId::proportional(14.0);
    let small = egui::FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(name.to_owned(), font, palette::text());
    let note = note.map(|n| ui.painter().layout_no_wrap(n, small, palette::text_weak()));
    let gap = 5.0;
    let mut size = galley.size();
    if let Some(n) = &note {
        size.x += gap + n.size().x;
    }
    if dot {
        size.x += gap + 6.0;
    }
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let colour = if on {
        palette::text_bright()
    } else if resp.hovered() {
        palette::text()
    } else {
        palette::text_weak()
    };
    let painter = ui.painter();
    let mut x = rect.left();
    let text_w = galley.size().x;
    painter.galley_with_override_text_color(rect.left_top(), galley, colour);
    x += text_w;
    if let Some(n) = note {
        x += gap;
        let y = rect.bottom() - n.size().y - 1.0;
        painter.galley(egui::pos2(x, y), n.clone(), palette::text_weak());
        x += n.size().x;
    }
    if dot {
        x += gap;
        painter.circle_filled(egui::pos2(x + 3.0, rect.center().y), 3.0, palette::accent());
    }
    resp
}

/// The lyrics' tab, `tall` points high: the lyrics in a box filling it,
/// scrolled within it (its tools are in the tabs' row).
fn lyrics_tab(ui: &mut Ui, d: &Details, form: Option<&mut Form>, tall: f32) {
    // Editing, until the fields are read: as shown.
    let field = form
        .filter(|f| f.state != State::Reading)
        .and_then(|f| f.lyrics.as_mut());
    let width = ui.available_width();
    ui.allocate_ui(vec2(width, tall), |ui| {
        ui.set_min_height(tall);
        right_margin().show(ui, |ui| {
            let top = ui.cursor().top();
            // As the general tab's first field under the row.
            ui.add_space(8.0);
            let rest = (tall - (ui.cursor().top() - top)).max(60.0);
            match field {
                Some(field) => lyrics_field(ui, field, &d.path, rest),
                None => {
                    let lyrics = value(d, &[ItemKey::UnsyncLyrics, ItemKey::Lyrics]);
                    lyrics_box(ui, &d.path, rest, |ui, h| {
                        let mut text = lyrics;
                        ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .frame(egui::Frame::NONE)
                                .hint_text(tr!("No lyrics", "Текста нет"))
                                .desired_width(ui.available_width())
                                .min_size(vec2(0.0, h)),
                        )
                    });
                }
            }
        });
    });
}

/// What the top of the window says: the title, the artist, the album with
/// its year and genre, the chips.
struct Top {
    title: String,
    artist: String,
    album: String,
    chips: Vec<String>,
    /// The "Spectrum" chip after them (one file, read).
    spectrum: bool,
}

impl Top {
    fn of(d: &Details) -> Top {
        let title = match value(d, &[ItemKey::TrackTitle]) {
            "" => stem(&d.path),
            t => t.to_owned(),
        };
        let album = [
            value(d, edit::Key::Album.items()),
            value(d, edit::Key::Year.items()),
            value(d, edit::Key::Genre.items()),
        ];
        Top {
            title,
            artist: value(d, &[ItemKey::TrackArtist, ItemKey::AlbumArtist]).to_owned(),
            album: joined(&album),
            chips: chips(d),
            spectrum: true,
        }
    }

    /// While the file is read, or when it cannot be: its name alone.
    fn named(path: &Path) -> Top {
        Top {
            title: stem(path),
            artist: String::new(),
            album: String::new(),
            chips: Vec::new(),
            spectrum: false,
        }
    }

    /// All the files: their number, and what they share.
    fn all(count: usize, form: Option<&Form>) -> Top {
        let title =
            crate::i18n::count(count as u64, ["file", "files"], ["файл", "файла", "файлов"]);
        let Some(form) = form.filter(|f| f.state != State::Reading) else {
            return Top {
                title,
                ..Top::named(Path::new(""))
            };
        };
        let artist = match form.common(edit::Key::Artist) {
            "" => form.common(edit::Key::AlbumArtist),
            a => a,
        };
        let album = [
            form.common(edit::Key::Album),
            form.common(edit::Key::Year),
            form.common(edit::Key::Genre),
        ];
        Top {
            title,
            artist: artist.to_owned(),
            album: joined(&album),
            chips: Vec::new(),
            spectrum: false,
        }
    }
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The parts that are not empty, with dots between them.
fn joined(parts: &[&str]) -> String {
    let parts: Vec<&str> = parts.iter().copied().filter(|s| !s.is_empty()).collect();
    parts.join("  ·  ")
}

/// Going through several files: the one shown (none: all of them), how
/// many, and whether it can be done now (not while writing).
struct Nav {
    at: Option<usize>,
    count: usize,
    enabled: bool,
    /// Why another file cannot be gone to now (`InfoView::held`), the
    /// disabled buttons' tip.
    held: Option<&'static str>,
}

/// The top of the window: the cover, the title, the artist, the album
/// line and the chips; in the corner the cross and, for several files,
/// "All" and the arrows.
fn hero(
    ui: &mut Ui,
    top: &Top,
    cover: Option<&TextureHandle>,
    menu: &CoverMenu,
    nav: &Nav,
    acts: &mut Vec<Act>,
) {
    ui.horizontal_top(|ui| {
        cover_button(ui, cover, menu, acts);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if super::close_cross(ui) {
                        acts.push(Act::Close);
                    }
                    if nav.count > 1 {
                        ui.add_space(6.0);
                        ui.add_enabled_ui(nav.enabled && nav.held.is_none(), |ui| {
                            nav_buttons(ui, nav, acts)
                        });
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&top.title)
                                    .size(22.0)
                                    .strong()
                                    .color(palette::text_bright()),
                            )
                            .truncate(),
                        );
                    });
                });
            });
            if !top.artist.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(&top.artist).size(16.0).color(palette::text()))
                        .truncate(),
                );
            }
            if !top.album.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(&top.album)
                            .size(13.5)
                            .color(palette::text_weak()),
                    )
                    .truncate(),
                );
            }
            if !top.chips.is_empty() {
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for text in &top.chips {
                        chip(ui, text);
                    }
                    if top.spectrum && spectrum_chip(ui) {
                        acts.push(Act::Spectrum);
                    }
                });
            }
            if let Some((text, error)) = &menu.note {
                ui.add_space(8.0);
                let colour = if *error {
                    palette::error()
                } else {
                    palette::text_weak()
                };
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(colour)).wrap());
            }
        });
    });
}

/// "All N", the previous file, "3 / 12", the next one (laid out from the
/// right).
fn nav_buttons(ui: &mut Ui, nav: &Nav, acts: &mut Vec<Act>) {
    use crate::ui::icons::Icon;
    let n = nav.count;
    let next = match nav.at {
        None => Some(0),
        Some(i) if i + 1 < n => Some(i + 1),
        Some(_) => None,
    };
    let previous = nav.at.and_then(|i| i.checked_sub(1));
    let held = |r: egui::Response| match nav.held {
        Some(why) => r.on_disabled_hover_text(why),
        None => r,
    };
    let tip = tr!("Next file", "Следующий файл");
    if held(
        ui.add_enabled_ui(next.is_some(), |ui| super::icon_button(ui, Icon::Next, false, tip))
            .inner,
    )
    .clicked()
    {
        acts.push(Act::Go(next));
    }
    if let Some(i) = nav.at {
        ui.label(RichText::new(format!("{} / {n}", i + 1)).color(palette::text_weak()));
    }
    let tip = tr!("Previous file", "Предыдущий файл");
    if held(
        ui.add_enabled_ui(previous.is_some(), |ui| {
            super::icon_button(ui, Icon::Prev, false, tip)
        })
        .inner,
    )
    .clicked()
    {
        acts.push(Act::Go(previous));
    }
    ui.add_space(4.0);
    let all = nav.at.is_none();
    if held(
        ui.add(egui::Button::new(tr!(format!("All {n}"), format!("Все {n}"))).selected(all))
            .on_hover_text(tr!(
                "The main fields of all the files at once",
                "Основные поля всех файлов сразу"
            )),
    )
    .clicked()
        && !all
    {
        acts.push(Act::Go(None));
    }
}

/// How the main fields are shown: as the file's details have them, as
/// all the files share them, or in text fields.
enum Mode<'a> {
    Read(&'a Details),
    All(&'a Form),
    Edit(&'a mut Form),
}

/// One file's sections (the tab `Tab::Main`): the main fields, the tag's
/// other fields (only those it has), then the file. The names of every section
/// in one column. Editing (`form`), the fields of `edit` in text fields.
fn body_one(ui: &mut Ui, d: &Details, form: Option<&mut Form>, genres: &[String]) {
    // Under the tabs' row.
    ui.add_space(8.0);
    ui.spacing_mut().item_spacing.y = 6.0;
    // Editing: the fields `edit` read (in the order shown), the others as
    // they are.
    let form = form.filter(|f| f.state != State::Reading);
    let mut more: Vec<Other> = d
        .fields
        .iter()
        .filter(|(k, _)| edit::is_other(*k))
        .map(|(k, v)| {
            let edited = form
                .as_ref()
                .and_then(|f| f.other.iter().position(|(o, _)| o == k));
            match edited {
                Some(i) => Other::Edited(*k, i),
                None => Other::Shown(*k, v.as_str()),
            }
        })
        .collect();
    if let Some(f) = &form {
        for (i, (k, _)) in f.other.iter().enumerate() {
            if !more.iter().any(|o| o.key() == *k) {
                more.push(Other::Edited(*k, i));
            }
        }
    }
    // The usual fields the tag can hold and does not show yet.
    let addable: Vec<(edit::Group, ItemKey)> = form
        .as_ref()
        .filter(|f| f.state == State::Ready)
        .map(|f| {
            f.addable
                .iter()
                .filter(|(_, k)| !more.iter().any(|o| o.key() == *k))
                .copied()
                .collect()
        })
        .unwrap_or_default();
    let file = file_lines(d);
    let names = main_names()
        .chain(more.iter().map(|o| field_name(o.key())))
        .chain(file.iter().map(|(n, _)| n.clone()));
    let width = names_width(ui, names);
    let pad = EDIT_PAD;

    // No "Main" heading: the tab is named so, and its row holds the tags
    // in the file and Edit (`tab_row`).
    let mut form = form;
    match form.as_mut() {
        Some(f) => main_lines(ui, width, pad, &mut Mode::Edit(f), genres),
        // Editing, until the fields are read: as shown.
        None => main_lines(ui, width, pad, &mut Mode::Read(d), genres),
    }

    if !more.is_empty() || !addable.is_empty() {
        ui.add_space(14.0);
        let mut added = None;
        section(ui, tr!("Additional", "Дополнительно"), |ui| {
            if !addable.is_empty() {
                added = add_field_button(ui, &addable);
            }
        });
        if let (Some(key), Some(f)) = (added, form.as_mut()) {
            f.other.push((key, Field::default()));
            f.focus_other = Some(key);
            ui.ctx().request_repaint();
        }
        for o in &more {
            let n = field_name(o.key());
            match (o, &mut form) {
                (Other::Edited(key, i), Some(f)) => line(ui, width, &n, pad, |ui| {
                    let focus = f.focus_other == Some(*key);
                    if focus {
                        f.focus_other = None;
                    }
                    if let Some((_, field)) = f.other.get_mut(*i) {
                        text_field(ui, field, false, true, None, focus, None);
                    }
                }),
                (Other::Shown(_, text), _) => line(ui, width, &n, pad, |ui| {
                    boxed(ui, text, "", Some(1), None);
                }),
                (Other::Edited(..), None) => {}
            }
        }
    }

    // The lyrics are a tab of their own (`Tab::Lyrics`).
    ui.add_space(14.0);
    section(ui, tr!("File", "Файл"), |ui| {
        if ui
            .small_button(tr!("Show in folder", "Показать в папке"))
            .clicked()
        {
            crate::win::show_in_folder(&d.path);
        }
    });
    for (n, text) in &file {
        line(ui, width, n, 0.0, |ui| shown_value(ui, text, true));
    }
}

/// A field of the section "Additional": in a text field (its place in
/// `Form::other`) or as it is.
enum Other<'a> {
    Edited(ItemKey, usize),
    Shown(ItemKey, &'a str),
}

impl Other<'_> {
    fn key(&self) -> ItemKey {
        match self {
            Other::Edited(k, _) | Other::Shown(k, _) => *k,
        }
    }
}

/// All the files at once: the main fields (the value they share, or
/// "(different)"), the files that could not be read, the files' names
/// (a click shows that file).
fn body_all(
    ui: &mut Ui,
    form: &mut Form,
    paths: &[PathBuf],
    editing: bool,
    genres: &[String],
    acts: &mut Vec<Act>,
) {
    ui.add_space(18.0);
    ui.spacing_mut().item_spacing.y = 6.0;
    let others = form.other.iter().map(|(k, _)| field_name(*k));
    let width = names_width(ui, main_names().chain(others));
    section(ui, tr!("Main", "Основное"), |ui| {
        if !editing {
            let reading = form.state != State::Ready;
            edit_button(ui, reading.then_some(tr!("Reading…", "Чтение…")), acts);
        }
    });
    // Until the files are read, empty boxes of the same size.
    if editing && form.state != State::Reading {
        main_lines(ui, width, EDIT_PAD, &mut Mode::Edit(form), genres);
    } else {
        main_lines(ui, width, EDIT_PAD, &mut Mode::All(form), genres);
        errors(ui, &form.errors);
    }
    if form.state != State::Reading {
        other_lines_all(ui, form, width, editing);
    }
    if editing && form.state == State::Ready && form.lrclib_on {
        lyrics_all(ui, form, acts);
    }
    ui.add_space(14.0);
    section(ui, tr!("Files", "Файлы"), |_| {});
    let found = form.lrclib.as_ref().filter(|l| !l.one).map(|l| &l.found);
    let held = if editing { form.held(true) } else { None };
    for (i, path) in paths.iter().enumerate() {
        let text = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // What LRCLIB gave the file, after its name.
        let mut job = egui::text::LayoutJob::default();
        let font = egui::FontId::proportional(14.0);
        job.append(&text, 0.0, egui::TextFormat::simple(font.clone(), palette::text()));
        let state = found.and_then(|f| f.get(i));
        if let Some(word) = state.and_then(Found::word) {
            let colour = match state {
                Some(Found::Lyrics(_)) => palette::accent(),
                Some(Found::Failed(_)) => palette::error(),
                _ => palette::text_weak(),
            };
            job.append(
                &format!("  ·  {word}"),
                0.0,
                egui::TextFormat::simple(egui::FontId::proportional(12.0), colour),
            );
        }
        let mut tip = path.display().to_string();
        if let Some(Found::Failed(e)) = state {
            tip = format!("{tip}\n{e}");
        }
        // Going to the file is not offered while it cannot be done.
        if let Some(why) = held {
            ui.add(egui::Label::new(job).truncate())
                .on_hover_text(format!("{tip}\n\n{why}"));
            continue;
        }
        let resp = ui
            .add(
                egui::Label::new(job)
                    .truncate()
                    .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tip);
        if resp.clicked() {
            acts.push(Act::Go(Some(i)));
        }
    }
}

/// All the files' other fields ("(different)" where they differ or some
/// lack it); editing, in text fields, a value typed in written to every
/// file, and the usual fields all of them can hold to be added.
fn other_lines_all(ui: &mut Ui, form: &mut Form, width: f32, editing: bool) {
    let addable: Vec<(edit::Group, ItemKey)> = if editing {
        form.addable
            .iter()
            .filter(|(_, k)| !form.other.iter().any(|(o, _)| o == k))
            .copied()
            .collect()
    } else {
        Vec::new()
    };
    if form.other.is_empty() && addable.is_empty() {
        return;
    }
    ui.add_space(14.0);
    let mut added = None;
    section(ui, tr!("Additional", "Дополнительно"), |ui| {
        if !addable.is_empty() {
            added = add_field_button(ui, &addable);
        }
    });
    if let Some(key) = added {
        form.other.push((key, Field::default()));
        form.focus_other = Some(key);
        ui.ctx().request_repaint();
    }
    let Form {
        other, focus_other, ..
    } = form;
    for (key, field) in other.iter_mut() {
        line(ui, width, &field_name(*key), EDIT_PAD, |ui| {
            if editing {
                let focus = *focus_other == Some(*key);
                if focus {
                    *focus_other = None;
                }
                text_field(ui, field, false, true, None, focus, None);
            } else if field.mixed {
                boxed(ui, "", tr!("(different)", "(разные)"), Some(1), None);
            } else {
                boxed(ui, &field.shown, "", Some(1), None);
            }
        });
    }
}

/// The names of the main fields, in `MAIN`'s order.
fn main_names() -> impl Iterator<Item = String> {
    MAIN.iter().map(|l| match l {
        Line::One(k) | Line::Two(k, _) => main_name(*k),
    })
}

fn main_name(key: edit::Key) -> String {
    field_name(key.items()[0])
}

/// A name beside a text field is lowered to the text in it, in points.
const EDIT_PAD: f32 = 3.0;

/// The main fields, `pad` the names lowered; `genres` the library's, for
/// the genre's menu.
fn main_lines(ui: &mut Ui, width: f32, pad: f32, mode: &mut Mode, genres: &[String]) {
    let second = names_width(
        ui,
        MAIN.iter().filter_map(|l| match l {
            Line::Two(_, k) => Some(main_name(*k)),
            Line::One(_) => None,
        }),
    );
    for l in &MAIN {
        match l {
            Line::One(k) => line(ui, width, &main_name(*k), pad, |ui| {
                main_cell(ui, mode, *k, None, genres);
            }),
            Line::Two(first, then) => line(ui, width, &main_name(*first), pad, |ui| {
                cell(ui, SHORT, |ui| {
                    main_cell(ui, mode, *first, Some(SHORT - 12.0), genres)
                });
                cell(ui, second, |ui| {
                    ui.vertical(|ui| {
                        ui.add_space(pad);
                        name(ui, &main_name(*then));
                    });
                });
                main_cell(ui, mode, *then, None, genres);
            }),
        }
    }
}

/// A main field's value, `width` points wide in a text field (else the
/// room left). The genre's, while editing, has a menu of genres beside it.
fn main_cell(
    ui: &mut Ui,
    mode: &mut Mode,
    key: edit::Key,
    width: Option<f32>,
    genres: &[String],
) {
    let comment = key == edit::Key::Comment;
    let rows = comment.then_some(2);
    match mode {
        Mode::Read(d) => boxed(ui, value(d, key.items()), "", rows, width),
        Mode::All(form) => match form.main.get(key as usize) {
            Some(f) if f.mixed => boxed(ui, "", tr!("(different)", "(разные)"), rows, width),
            Some(f) => boxed(ui, &f.shown, "", rows, width),
            None => boxed(ui, "", "", rows, width),
        },
        Mode::Edit(form) => {
            // The first field takes the keys once the fields came.
            let focus = key == edit::Key::Title && std::mem::take(&mut form.focus_first);
            if let Some(f) = form.main.get_mut(key as usize) {
                let valid = !f.touched || key.takes(&f.value);
                if key == edit::Key::Genre {
                    genre_field(ui, f, valid, genres);
                } else {
                    text_field(ui, f, comment, valid, width, focus, None);
                }
            }
        }
    }
}

/// A field in a text field: "(different)" as a hint while the files differ
/// in it and nothing was typed; red while not what the field takes. `id`:
/// its own, else egui's.
fn text_field(
    ui: &mut Ui,
    f: &mut Field,
    multiline: bool,
    valid: bool,
    width: Option<f32>,
    focus: bool,
    id: Option<egui::Id>,
) -> egui::Response {
    let hint = if f.mixed && !f.touched {
        tr!("(different)", "(разные)")
    } else {
        ""
    };
    let edit = if multiline {
        egui::TextEdit::multiline(&mut f.value).desired_rows(2)
    } else {
        egui::TextEdit::singleline(&mut f.value)
    };
    let mut edit = edit
        .hint_text(hint)
        .desired_width(width.unwrap_or(ui.available_width()));
    if !valid {
        edit = edit.text_color(palette::error());
    }
    if let Some(id) = id {
        edit = edit.id(id);
    }
    let resp = ui.add(edit);
    // Typed back to what it was (a mixed field emptied again): left
    // alone, not written empty to every file.
    if resp.changed() {
        f.touched = f.value != f.shown;
    }
    if focus {
        resp.request_focus();
        resp.scroll_to_me(Some(egui::Align::Center));
    }
    super::take_leaving_key(ui, &resp);
    resp
}

/// The most genres suggested under the genre's field as it is typed in.
const SUGGESTIONS: usize = 8;

/// What the genre's field remembers between frames: the suggestion chosen
/// with the arrows, the text the suggestions were hidden for (Esc, or one
/// taken), where they were shown (a click on them takes the field's
/// focus away before it is a click).
#[derive(Clone, Default)]
struct Suggest {
    chosen: Option<usize>,
    hidden: Option<String>,
    rect: Option<egui::Rect>,
}

/// The genre's field while editing: under it, as it is typed in, the
/// genres that start with the text, then those with a word starting with
/// it, then those containing it (`suggestions`); Up and Down choose one,
/// Enter or a click takes it, Esc hides them. Beside it, the menu of all
/// of them (`genre_menu`).
fn genre_field(ui: &mut Ui, f: &mut Field, valid: bool, genres: &[String]) {
    let room = ui.available_width() - MENU_BUTTON - ui.spacing().item_spacing.x;
    let id = ui.make_persistent_id("genre_field");
    let memory = id.with("suggest");
    let mut state: Suggest = ui.data(|d| d.get_temp(memory)).unwrap_or_default();
    let focused = ui.memory(|m| m.has_focus(id));
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let over = state
        .rect
        .zip(pointer)
        .is_some_and(|(r, p)| r.contains(p));
    let offered = |f: &Field, state: &Suggest| -> Vec<&str> {
        let hidden = state.hidden.as_deref() == Some(f.value.as_str());
        if f.touched && !hidden {
            suggestions(&f.value, genres)
        } else {
            Vec::new()
        }
    };
    let list = if focused || over {
        offered(f, &state)
    } else {
        Vec::new()
    };
    if state.chosen.is_some_and(|c| c >= list.len()) {
        state.chosen = None;
    }
    let mut take = None;
    // Taken before the field sees them: the arrows would move its cursor,
    // Enter and Esc take its focus away (and Esc would end the editing).
    if focused && !list.is_empty() {
        let n = list.len();
        ui.input_mut(|i| {
            let none = egui::Modifiers::NONE;
            if i.consume_key(none, Key::ArrowDown) {
                state.chosen = Some(state.chosen.map_or(0, |c| (c + 1) % n));
            }
            if i.consume_key(none, Key::ArrowUp) {
                state.chosen = Some(state.chosen.map_or(n - 1, |c| (c + n - 1) % n));
            }
            if let Some(c) = state.chosen
                && i.consume_key(none, Key::Enter)
            {
                take = Some(list[c].to_owned());
            }
            if i.consume_key(none, Key::Escape) {
                state.hidden = Some(f.value.clone());
            }
        });
    }
    let resp = text_field(ui, f, false, valid, Some(room), false, Some(id));
    if resp.changed() {
        state.chosen = None;
    }
    genre_menu(ui, f, genres);

    let list = if take.is_none() && (focused || over || resp.has_focus()) {
        offered(f, &state)
    } else {
        Vec::new()
    };
    state.rect = None;
    if !list.is_empty() {
        let width = resp.rect.width();
        let shown = egui::Area::new(memory.with("list"))
            // Over the modal (a foreground area) without being its
            // sublayer.
            .order(egui::Order::Tooltip)
            .fixed_pos(resp.rect.left_bottom() + vec2(0.0, 2.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width - 12.0);
                    for (i, g) in list.iter().enumerate() {
                        let item = egui::Button::selectable(state.chosen == Some(i), *g)
                            .min_size(vec2(ui.available_width(), 0.0));
                        if ui.add(item).clicked() {
                            take = Some((*g).to_owned());
                        }
                    }
                });
            });
        state.rect = Some(shown.response.rect);
    }
    if let Some(g) = take {
        f.value = g;
        f.touched = f.value != f.shown;
        state.chosen = None;
        state.hidden = Some(f.value.clone());
        state.rect = None;
        // Back to the field, its cursor at the end.
        ui.memory_mut(|m| m.request_focus(id));
        if let Some(mut edit) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
            let end = egui::text::CCursor::new(f.value.chars().count());
            edit.cursor
                .set_char_range(Some(egui::text::CCursorRange::one(end)));
            edit.store(ui.ctx(), id);
        }
    }
    ui.data_mut(|d| d.insert_temp(memory, state));
}

/// The genres suggested for `typed`: those starting with it, then those
/// with a word starting with it, then those containing it, the library's
/// before the usual ones, at most `SUGGESTIONS`; not the one typed itself.
fn suggestions<'a>(typed: &str, library: &'a [String]) -> Vec<&'a str> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return Vec::new();
    }
    let common = crate::library::genres::common_besides(library);
    let mut found: Vec<(u8, usize, &str)> = library
        .iter()
        .map(String::as_str)
        .chain(common)
        .enumerate()
        .filter_map(|(order, g)| {
            let lower = g.to_lowercase();
            let rank = if lower == typed {
                return None;
            } else if lower.starts_with(&typed) {
                0
            } else if lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w.starts_with(&typed))
            {
                1
            } else if lower.contains(&typed) {
                2
            } else {
                return None;
            };
            Some((rank, order, g))
        })
        .collect();
    found.sort();
    found.truncate(SUGGESTIONS);
    found.into_iter().map(|(_, _, g)| g).collect()
}

/// The width of the button beside the genre's field.
const MENU_BUTTON: f32 = 24.0;

/// The button beside the genre's field: a menu of the library's genres,
/// then the usual ones it lacks (`genres::COMMON`), narrowed to those
/// containing what was typed (unless it is one of them); the one chosen
/// goes into the field.
fn genre_menu(ui: &mut Ui, f: &mut Field, library: &[String]) {
    let height = ui.spacing().interact_size.y;
    let (rect, resp) = ui.allocate_exact_size(vec2(MENU_BUTTON, height), egui::Sense::click());
    let colour = if resp.hovered() {
        palette::text_bright()
    } else {
        palette::text()
    };
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, palette::hover());
    }
    let c = rect.center();
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(c.x - 4.0, c.y - 2.0),
            egui::pos2(c.x + 4.0, c.y - 2.0),
            egui::pos2(c.x, c.y + 2.5),
        ],
        colour,
        egui::Stroke::NONE,
    ));
    let resp = resp.on_hover_text(tr!("Genres", "Жанры"));

    let value = f.value.trim();
    let (ours, usual) = genre_lists(value, f.touched, library);
    let mut chosen = None;
    // An id of its own (see Conventions: the tooltip's area takes the
    // default one).
    let shown = egui::Popup::menu(&resp)
        .id(resp.id.with("genre_menu"))
        .show(|ui| {
            ui.set_min_width(220.0);
            let most = (ui.ctx().content_rect().height() - 120.0).clamp(160.0, 420.0);
            egui::ScrollArea::vertical()
                .max_height(most)
                .show(ui, |ui| {
                    let lists = [
                        (tr!("In the library", "В библиотеке"), ours),
                        (tr!("Common", "Распространённые"), usual),
                    ];
                    let mut first = true;
                    for (heading, list) in lists {
                        if list.is_empty() {
                            continue;
                        }
                        if !first {
                            ui.separator();
                        }
                        first = false;
                        ui.label(
                            RichText::new(heading.to_uppercase())
                                .size(10.5)
                                .strong()
                                .color(palette::text_weak()),
                        );
                        for g in list {
                            let current = g.to_lowercase() == value.to_lowercase();
                            if ui.selectable_label(current, g).clicked() {
                                chosen = Some(g.to_owned());
                            }
                        }
                    }
                });
        });
    // Kept over the modal, as the cover's menu (see Conventions).
    if let Some(shown) = shown {
        ui.ctx()
            .set_sublayer(ui.layer_id(), shown.response.layer_id);
    }
    if let Some(g) = chosen {
        f.value = g;
        f.touched = f.value != f.shown;
    }
}

/// The genre menu's lists: the library's genres and the usual ones it
/// lacks, those containing `value` when it was typed (`touched`) and is
/// none of them; all when none contains it.
fn genre_lists<'a>(
    value: &str,
    touched: bool,
    library: &'a [String],
) -> (Vec<&'a str>, Vec<&'static str>) {
    let common = crate::library::genres::common_besides(library);
    let ours: Vec<&str> = library.iter().map(String::as_str).collect();
    // Any case, Cyrillic too (the library's genres are grouped so).
    let typed = value.to_lowercase();
    let known = ours
        .iter()
        .chain(&common)
        .any(|g| g.to_lowercase() == typed);
    if !touched || known || value.is_empty() {
        return (ours, common);
    }
    let has = |g: &&str| g.to_lowercase().contains(&typed);
    let narrowed = (
        ours.iter().copied().filter(has).collect::<Vec<_>>(),
        common.iter().copied().filter(has).collect::<Vec<_>>(),
    );
    if narrowed.0.is_empty() && narrowed.1.is_empty() {
        (ours, common)
    } else {
        narrowed
    }
}

/// The lyrics in a text field of several lines (Enter starts a line), in
/// [`lyrics_box`] `tall` points high.
fn lyrics_field(ui: &mut Ui, f: &mut Field, salt: &Path, tall: f32) {
    let resp = lyrics_box(ui, salt, tall, |ui, h| {
        ui.add(
            egui::TextEdit::multiline(&mut f.value)
                .frame(egui::Frame::NONE)
                .hint_text(tr!("No lyrics", "Текста нет"))
                .desired_width(ui.available_width())
                .min_size(vec2(0.0, h)),
        )
    });
    if resp.changed() {
        f.touched = f.value != f.shown;
    }
    super::take_leaving_key(ui, &resp);
}

/// The lyrics' box, `tall` points high: `text` (a frameless text field,
/// editable or not, given the height inside to fill, so a click anywhere
/// in the box reaches it) in a scroll area, with the ground and border of
/// a text field painted around it, so the box stays put while its text
/// scrolls. `salt` (the file) keeps each file's place apart. The text's
/// response.
fn lyrics_box(
    ui: &mut Ui,
    salt: &Path,
    tall: f32,
    text: impl FnOnce(&mut Ui, f32) -> egui::Response,
) -> egui::Response {
    let ground = ui.painter().add(egui::Shape::Noop);
    let margin = egui::Margin::symmetric(6, 4);
    let inside = (tall - margin.sum().y).max(20.0);
    let shown = egui::Frame::new()
        .inner_margin(margin)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("file_info_lyrics", salt))
                .min_scrolled_height(inside)
                .max_height(inside)
                .auto_shrink([false, false])
                .show(ui, |ui| text(ui, inside))
                .inner
        });
    let resp = shown.inner;
    let visuals = ui.visuals();
    let stroke = if resp.has_focus() {
        visuals.selection.stroke
    } else {
        ui.style().interact(&resp).bg_stroke
    };
    let radius = visuals.widgets.inactive.corner_radius;
    ui.painter().set(
        ground,
        egui::epaint::RectShape::new(
            shown.response.rect,
            radius,
            visuals.text_edit_bg_color(),
            stroke,
            egui::StrokeKind::Inside,
        ),
    );
    resp
}

/// "Add a field" in the heading of "Additional" while editing: a menu of
/// the usual fields the tag can hold and lacks (`addable`), by group;
/// the one chosen.
fn add_field_button(ui: &mut Ui, addable: &[(edit::Group, ItemKey)]) -> Option<ItemKey> {
    let resp = ui
        .small_button(tr!("+ Add a field", "+ Добавить поле"))
        .on_hover_text(tr!(
            "Fields the tag does not have yet, of those its format holds",
            "Поля, которых в теге ещё нет, из тех, что поддерживает его формат"
        ));
    let mut chosen = None;
    // An id of its own (see Conventions: the tooltip's area takes the
    // default one).
    let shown = egui::Popup::menu(&resp)
        .id(resp.id.with("add_field_menu"))
        .show(|ui| {
            ui.set_min_width(220.0);
            let most = (ui.ctx().content_rect().height() - 120.0).clamp(160.0, 520.0);
            egui::ScrollArea::vertical()
                .max_height(most)
                .show(ui, |ui| {
                    for group in edit::Group::ALL {
                        let keys: Vec<ItemKey> = addable
                            .iter()
                            .filter(|(g, _)| *g == group)
                            .map(|(_, k)| *k)
                            .collect();
                        if keys.is_empty() {
                            continue;
                        }
                        if group != edit::Group::ALL[0] {
                            ui.separator();
                        }
                        ui.label(
                            RichText::new(group.label().to_uppercase())
                                .size(10.5)
                                .strong()
                                .color(palette::text_weak()),
                        );
                        for key in keys {
                            if ui.button(field_name(key)).clicked() {
                                chosen = Some(key);
                            }
                        }
                    }
                });
        });
    // Kept over the modal, as the cover's menu (see Conventions).
    if let Some(shown) = shown {
        ui.ctx()
            .set_sublayer(ui.layer_id(), shown.response.layer_id);
    }
    chosen
}

/// A value shown in a box as the text fields' while editing, but not
/// editable (its text can be selected and copied); `rows`: on as many
/// lines at least, a longer value wrapped, else on one line.
fn boxed(ui: &mut Ui, text: &str, hint: &str, rows: Option<usize>, width: Option<f32>) {
    let mut text = text;
    let edit = match rows {
        Some(rows) => egui::TextEdit::multiline(&mut text).desired_rows(rows),
        None => egui::TextEdit::singleline(&mut text),
    };
    // egui draws a field that cannot be edited without its ground: the
    // editable one's, painted under it.
    let ground = ui.painter().add(egui::Shape::Noop);
    let resp = ui.add(
        edit.hint_text(hint)
            .desired_width(width.unwrap_or(ui.available_width())),
    );
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = ui.visuals().extreme_bg_color;
    ui.painter()
        .set(ground, egui::epaint::RectShape::filled(resp.rect, radius, fill));
}

/// "Edit" in a section's heading, disabled with `why_not`.
fn edit_button(ui: &mut Ui, why_not: Option<&str>, acts: &mut Vec<Act>) {
    let resp = ui
        .add_enabled(
            why_not.is_none(),
            egui::Button::new(tr!("Edit", "Изменить")).small(),
        )
        .on_hover_text(crate::keymap::with_key(
            tr!("Edit the tags", "Изменить теги"),
            Cmd::EditTags,
        ))
        .on_disabled_hover_text(why_not.unwrap_or_default());
    if resp.clicked() {
        acts.push(Act::Edit);
    }
}

/// The files that could not be read or written, in red.
fn errors(ui: &mut Ui, errors: &[String]) {
    if errors.is_empty() {
        return;
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt("file_info_errors")
        .max_height(ERRORS)
        .show(ui, |ui| {
            for e in errors {
                ui.label(RichText::new(e).size(12.5).color(palette::error()));
            }
        });
}

/// Under the fields while editing: what could not be read or written,
/// "Writing…", Cancel and Save; editing one of several files (`steps`:
/// the files before and after it), "Save and back" and "Save and next"
/// either side of Save, as Tag&Rename has them (the arrows in the corner
/// wait while changes do).
fn footer_row(
    ui: &mut Ui,
    form: Option<&Form>,
    steps: Option<(Option<usize>, Option<usize>)>,
    acts: &mut Vec<Act>,
) {
    ui.add_space(6.0);
    if let Some(f) = form {
        errors(ui, &f.errors);
    }
    ui.add_space(6.0);
    let state = form.map(|f| &f.state);
    let can_save = form.is_some_and(|f| {
        f.state == State::Ready && f.touched() && f.valid() && !f.looking()
    });
    let can_step = form.is_some_and(|f| f.state == State::Ready && f.valid() && !f.looking());
    let looking = form.is_some_and(Form::looking);
    let looking_tip = |r: egui::Response| {
        if looking {
            r.on_disabled_hover_text(tr!(
                "Looking on LRCLIB: wait for it to end",
                "Идёт поиск в LRCLIB: дождитесь его окончания"
            ))
        } else {
            r
        }
    };
    let step = |ui: &mut Ui, to: Option<usize>, label: &str, tip: &str| {
        looking_tip(
            ui.add_enabled(
                can_step && to.is_some(),
                egui::Button::new(label).min_size(vec2(100.0, 0.0)),
            )
            .on_hover_text(tip),
        )
        .clicked()
        .then_some(to)
        .flatten()
    };
    ui.horizontal(|ui| {
        if state == Some(&State::Saving) {
            ui.spinner();
            ui.weak(tr!("Writing…", "Запись…"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some((_, next)) = steps
                && let Some(to) = step(
                    ui,
                    next,
                    tr!("Save and next", "Сохранить и далее"),
                    tr!(
                        "Writes this file's changes (if any) and edits the next file",
                        "Записывает изменения этого файла (если они есть) и переходит к правке следующего"
                    ),
                )
            {
                acts.push(Act::SaveAndGo(to));
            }
            let save = looking_tip(ui.add_enabled(
                can_save,
                egui::Button::new(tr!("Save", "Сохранить")).min_size(vec2(100.0, 0.0)),
            ));
            if save.clicked() {
                acts.push(Act::Save);
            }
            if let Some((previous, _)) = steps
                && let Some(to) = step(
                    ui,
                    previous,
                    tr!("Save and back", "Сохранить и назад"),
                    tr!(
                        "Writes this file's changes (if any) and edits the file before it",
                        "Записывает изменения этого файла (если они есть) и переходит к правке предыдущего"
                    ),
                )
            {
                acts.push(Act::SaveAndGo(to));
            }
            if ui
                .add_enabled(
                    state != Some(&State::Saving),
                    egui::Button::new(tr!("Cancel", "Отмена")).min_size(vec2(90.0, 0.0)),
                )
                .clicked()
            {
                acts.push(Act::Cancel);
            }
        });
    });
}

/// The lines of the section "File": the path, an MP3's encoder from its
/// LAME tag (LAME's or FFmpeg's: the encoder with its settings, the delay
/// and the padding on one line; the encoder's name alone without a tag),
/// the pictures.
fn file_lines(d: &Details) -> Vec<(String, String)> {
    let mut out = vec![(tr!("Path", "Путь").to_owned(), d.path.display().to_string())];
    if d.gapless.is_some() || d.encoder.is_some() {
        // Not "Кодировщик": the tag's "Encoded by" (TENC) is named so.
        out.push((
            tr!("Encoder", "Кодек").to_owned(),
            encoder_line(d.encoder.as_ref(), d.gapless),
        ));
    }
    for p in &d.pictures {
        let mut text = p.format.clone();
        if let Some((w, h)) = p.pixels {
            text += &format!(" {w}×{h}");
        }
        text += &format!(", {}", format::size(p.bytes as u64));
        out.push((picture_name(p.kind), text));
    }
    out
}

/// The encoder's line: `LAME3.99r (-m j -V 4 -q 3 -lowpass 20.5), delay
/// 576, padding 924 samples`; FFmpeg's as `FFmpeg (Lavc60.3)`; the
/// encoder or the delay and padding left out when the tag lacks them.
fn encoder_line(
    encoder: Option<&crate::audio::mp3gap::Encoder>,
    gapless: Option<(u32, u32)>,
) -> String {
    let name = encoder.map(|e| {
        if e.name.starts_with("Lav") {
            format!("FFmpeg ({})", e.name)
        } else if e.settings.is_empty() {
            e.name.clone()
        } else {
            format!("{} ({})", e.name, e.settings)
        }
    });
    let gap = gapless.map(|(delay, padding)| {
        tr!(
            format!("delay {delay}, padding {padding} samples"),
            format!("задержка {delay}, добивка {padding} сэмплов")
        )
    });
    match (name, gap) {
        (Some(name), Some(gap)) => format!("{name}, {gap}"),
        (Some(one), None) | (None, Some(one)) => one,
        (None, None) => String::new(),
    }
}

/// A section's heading: small capitals, `right` (a note or a button) at
/// the right end, a line under them.
fn section(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .size(11.0)
                .strong()
                .color(palette::text_weak()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(
        r.left()..=r.right(),
        r.top() + 2.0,
        egui::Stroke::new(1.0, palette::line()),
    );
    ui.add_space(10.0);
}

/// The lyrics' lines that are not empty.
fn lyrics_lines(text: &str) -> usize {
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// The lyrics tab's tip: their lines counted, or none.
fn lyrics_note(lines: usize) -> String {
    if lines == 0 {
        tr!("No lyrics", "Текста нет").to_owned()
    } else {
        crate::i18n::count(lines as u64, ["line", "lines"], ["строка", "строки", "строк"])
    }
}

/// The names' column: as wide as the widest of `names`, at most
/// `MAX_NAME` (measured, not left to a grid, which sizes its columns by
/// the frame before).
fn names_width(ui: &Ui, names: impl Iterator<Item = String>) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    names
        .map(|n| {
            ui.painter()
                .layout_no_wrap(n, font.clone(), palette::text())
                .size()
                .x
        })
        .fold(0.0, f32::max)
        .min(MAX_NAME)
        .ceil()
}

/// A line of a section: `label` in the names' column (`width`, lowered by
/// `pad` beside a text field), then `value` in the room right of it.
fn line(ui: &mut Ui, width: f32, label: &str, pad: f32, value: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        cell(ui, width, |ui| {
            ui.vertical(|ui| {
                ui.add_space(pad);
                name(ui, label);
            });
        });
        value(ui);
    });
}

/// A cell `width` points wide whatever it holds.
fn cell(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui)) {
    let size = vec2(width, ui.spacing().interact_size.y);
    let layout = egui::Layout::left_to_right(egui::Align::Min);
    ui.allocate_ui_with_layout(size, layout, |ui| {
        ui.set_width(width);
        add(ui);
    });
}

/// A field's name, in the weak colour; cut (whole in a tooltip) when
/// longer than its cell.
fn name(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(palette::text_weak())).truncate());
}

/// A field's value, a dash when empty; `wrap`: a long value takes more
/// lines, else it is cut.
fn shown_value(ui: &mut Ui, text: &str, wrap: bool) {
    if text.is_empty() {
        ui.label(RichText::new("—").color(palette::line()));
    } else {
        let label = egui::Label::new(RichText::new(text).color(palette::text()));
        ui.add(if wrap { label.wrap() } else { label.truncate() });
    }
}

/// What the cover's menu offers.
struct CoverMenu {
    /// Some of the files' tags can be written.
    writable: bool,
    /// The folder of the files shown, when they are of one.
    folder: Option<PathBuf>,
    /// A change waits for Save.
    changed: bool,
    /// There is a cover to remove (all the files at once: maybe).
    removable: bool,
    /// There is a cover to save as `folder.jpg`.
    savable: bool,
    /// Pictures are dragged over the window.
    dropping: bool,
    /// A picture is being read or saved.
    busy: bool,
    note: Option<(String, bool)>,
    /// The largest side of a cover put into tags.
    side: CoverSide,
}

/// What became of a picture made ready: its size, and the size written.
fn made_note(made: &crate::covers::ForTag) -> String {
    let ((w, h), (tw, th)) = (made.from, made.to);
    let size = format::size(made.data.len() as u64);
    if made.from == made.to {
        tr!(
            format!("{w}×{h}, written as it is ({size})"),
            format!("{w}×{h}, запишется как есть ({size})")
        )
    } else {
        tr!(
            format!("{w}×{h}, made {tw}×{th} (JPEG, {size})"),
            format!("{w}×{h}, уменьшена до {tw}×{th} (JPEG, {size})")
        )
    }
}

/// A picture file this large is not taken for a cover.
const MAX_PICTURE: u64 = 64 << 20;

/// Where the cover shown large comes from.
enum CoverSource {
    /// The picture as it is (the tag's, or a new one's as written).
    Bytes(Arc<Vec<u8>>),
    /// The picture of this folder (`covers::in_folder`).
    Folder(PathBuf),
}

impl CoverSource {
    /// The picture at most `ZOOM_SIDE` a side, and a line of its size,
    /// format and bytes: `1500×1500 · JPEG · 512 KB`.
    fn picture(self) -> Option<(egui::ColorImage, String)> {
        let bytes = match self {
            CoverSource::Bytes(b) => b,
            CoverSource::Folder(folder) => {
                let path = crate::covers::in_folder(&folder)?;
                if std::fs::metadata(&path).ok()?.len() > MAX_PICTURE {
                    return None;
                }
                Arc::new(std::fs::read(path).ok()?)
            }
        };
        let picture = crate::guard::catch(Path::new("cover"), || crate::covers::decode(&bytes))??;
        let (w, h) = (picture.width(), picture.height());
        let format = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Png) => "PNG",
            _ => "JPEG",
        };
        let note = format!("{w}×{h} · {format} · {}", format::size(bytes.len() as u64));
        let picture = if w.max(h) > ZOOM_SIDE {
            picture.resize(ZOOM_SIDE, ZOOM_SIDE, image::imageops::FilterType::Triangle)
        } else {
            picture
        };
        Some((crate::covers::color_image(&picture), note))
    }
}

/// `work` on a thread of its own, its answer woken for.
fn spawn_cover(
    ctx: &egui::Context,
    work: impl FnOnce() -> CoverDone + Send + 'static,
) -> Receiver<CoverDone> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("cover".into())
        .spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    if let Err(e) = spawned {
        log::warn!("no cover thread: {e}");
    }
    rx
}

/// The hero's cover: a click shows it large (without one, opens the
/// menu), the right button opens its menu; darkened under the pointer,
/// outlined in the accent while pictures are dragged over the window,
/// with a spinner while one is read.
fn cover_button(ui: &mut Ui, cover: Option<&TextureHandle>, menu: &CoverMenu, acts: &mut Vec<Act>) {
    use crate::ui::icons::{self, Icon};
    let resp = cover_box(ui, cover, COVER, egui::Sense::click());
    let rect = resp.rect;
    let shade = egui::Color32::from_black_alpha(110);
    let painter = ui.painter();
    if menu.busy {
        painter.rect_filled(rect, 8.0, shade);
        egui::Spinner::new().size(24.0).paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), vec2(24.0, 24.0)),
        );
    } else if menu.dropping && menu.writable {
        painter.rect_filled(rect, 8.0, shade);
        let mark = egui::Color32::from_white_alpha(230);
        icons::paint(painter, Icon::Image, rect.center(), 32.0, mark, shade);
        painter.rect_stroke(
            rect.shrink(1.0),
            8.0,
            egui::Stroke::new(2.0, palette::accent()),
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        // A cover: a lighter shade with a magnifier, as a picture to look
        // at; none: the picture's mark, as a place to put one.
        let mark = egui::Color32::from_white_alpha(230);
        if cover.is_some() {
            let shade = egui::Color32::from_black_alpha(60);
            painter.rect_filled(rect, 8.0, shade);
            icons::paint(painter, Icon::ZoomIn, rect.center(), 32.0, mark, shade);
        } else {
            painter.rect_filled(rect, 8.0, shade);
            icons::paint(painter, Icon::Image, rect.center(), 32.0, mark, shade);
        }
    }
    let resp = resp.on_hover_text(if cover.is_some() {
        tr!(
            "A click shows the cover large; the right button opens its menu; a picture can be dropped here",
            "Щелчок показывает обложку крупно; правая кнопка открывает её меню; сюда можно перетащить картинку"
        )
    } else {
        tr!(
            "A click opens the cover's menu; a picture can be dropped here",
            "Щелчок открывает меню обложки; сюда можно перетащить картинку"
        )
    });
    if resp.clicked() && cover.is_some() && !menu.busy {
        acts.push(Act::Zoom);
    }
    // The right button (a click without a cover) opens the menu at the
    // pointer, as a context menu; a click while it is open closes it (a
    // click outside). Its id is its own: `Popup::menu`'s is the one the
    // tooltip's area takes (`Popup::default_response_id`), and with the
    // tooltip shown the menu counted as open the frame before, so the
    // click that opened it closed it at once.
    let open = resp.secondary_clicked() || (resp.clicked() && cover.is_none());
    let menu_id = resp.id.with("cover_menu");
    let shown = egui::Popup::menu(&resp)
        .id(menu_id)
        .open_memory(open.then_some(egui::containers::SetOpenCommand::Bool(true)))
        .at_pointer_fixed()
        // The size is chosen with the menu kept open; the other items
        // close it.
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| cover_items(ui, menu, acts));
    // Kept right over the window, as its sublayer (set each frame: egui
    // forgets sublayers at the end of a pass). Both are foreground areas;
    // a press on the window raises it, and egui keeps the order of those
    // raised together, so the right button's press, which reaches egui in
    // the frame the menu opens (`App::raw_input_hook`), left a menu opened
    // before under the window after any click on it: open, not seen.
    if let Some(shown) = shown {
        ui.ctx()
            .set_sublayer(ui.layer_id(), shown.response.layer_id);
    }
}

/// The cover's menu: another picture (a file, the folder's), none, the one
/// before; the cover saved as the folder's `folder.jpg`.
fn cover_items(ui: &mut Ui, menu: &CoverMenu, acts: &mut Vec<Act>) {
    ui.set_min_width(240.0);
    let mut act = None;
    ui.add_enabled_ui(menu.writable && !menu.busy, |ui| {
        if ui
            .button(tr!("Choose a picture…", "Выбрать картинку…"))
            .clicked()
        {
            act = Some(CoverAct::Choose);
        }
        if let Some(found) = menu.folder.as_deref().and_then(crate::covers::in_folder) {
            let name = found
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let label = tr!(
                format!("The folder's picture ({name})"),
                format!("Картинка папки ({name})")
            );
            if ui.button(label).clicked() {
                act = Some(CoverAct::File(found));
            }
        }
        let remove = egui::Button::new(tr!("Remove the cover", "Удалить обложку"));
        if ui.add_enabled(menu.removable, remove).clicked() {
            act = Some(CoverAct::Remove);
        }
        if menu.changed
            && ui
                .button(tr!("The cover as it was", "Прежняя обложка"))
                .clicked()
        {
            act = Some(CoverAct::Keep);
        }
    })
    .response
    .on_disabled_hover_text(tr!(
        "Tags of this format cannot be written",
        "Теги этого формата записать нельзя"
    ));
    ui.separator();
    ui.label(
        RichText::new(tr!(
            "The largest side in the tag, pixels",
            "Наибольшая сторона в теге, пикс."
        ))
        .size(12.0)
        .color(palette::text_weak()),
    )
    .on_hover_text(tr!(
        "A larger picture is made this large and written as a JPEG, so each track of an album does not carry megabytes of it; a JPEG or PNG within it is written as it is.",
        "Картинка крупнее уменьшается до этого размера и пишется в JPEG, чтобы каждый трек альбома не нёс по нескольку мегабайт; JPEG или PNG в этих пределах пишется как есть."
    ));
    ui.add_enabled_ui(!menu.busy, |ui| {
        ui.horizontal(|ui| {
            for side in CoverSide::ALL {
                if ui
                    .selectable_label(menu.side == side, side.label())
                    .clicked()
                    && menu.side != side
                {
                    acts.push(Act::Cover(CoverAct::Side(side)));
                }
            }
        });
    });
    if menu.savable
        && let Some(folder) = &menu.folder
    {
        ui.separator();
        let (_, exists) = crate::covers::folder_jpg(folder);
        let label = if exists {
            tr!("Replace folder.jpg", "Заменить folder.jpg")
        } else {
            tr!("Save as folder.jpg", "Сохранить как folder.jpg")
        };
        let saved = ui
            .add_enabled(!menu.busy, egui::Button::new(label))
            .on_hover_text(tr!(
                "The cover shown into the files' folder, at once: the library's album grid shows a folder's picture first.",
                "Показанная обложка сразу записывается в папку файлов: сетка альбомов библиотеки в первую очередь показывает картинку папки."
            ));
        if saved.clicked() {
            act = Some(CoverAct::SaveToFolder);
        }
    }
    if let Some(act) = act {
        acts.push(Act::Cover(act));
        ui.close();
    }
}

/// The cover at `side` points with rounded corners, or a placeholder.
fn cover_box(
    ui: &mut Ui,
    cover: Option<&TextureHandle>,
    side: f32,
    sense: egui::Sense,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), sense);
    let painter = ui.painter();
    match cover {
        Some(t) => {
            let [w, h] = t.size();
            let scale = side / w.max(h) as f32;
            let r = egui::Rect::from_center_size(
                rect.center(),
                vec2(w as f32 * scale, h as f32 * scale),
            );
            painter.rect_filled(rect, 8.0, palette::list_bg());
            egui::Image::new(egui::load::SizedTexture::new(t.id(), r.size()))
                .corner_radius(8.0)
                .paint_at(ui, r);
        }
        None => crate::ui::no_cover(painter, rect, 8.0),
    }
    resp
}

/// A small rounded label with a fact (`320 kbps`).
fn chip(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(12.0),
        palette::text(),
    );
    let size = galley.size() + vec2(18.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, size.y / 2.0, palette::card_bg());
    ui.painter()
        .galley(rect.min + vec2(9.0, 4.0), galley, palette::text());
}

/// The chip that opens the spectrum: a chip in the accent, lit under the
/// pointer; whether it was clicked.
fn spectrum_chip(ui: &mut Ui) -> bool {
    let text = tr!("Spectrum", "Спектр");
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(12.0),
        palette::accent(),
    );
    let size = galley.size() + vec2(18.0, 8.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if response.hovered() {
        palette::hover()
    } else {
        palette::card_bg()
    };
    ui.painter().rect_filled(rect, size.y / 2.0, fill);
    ui.painter().rect_stroke(
        rect,
        size.y / 2.0,
        egui::Stroke::new(1.0, palette::accent().gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.min + vec2(9.0, 4.0), galley, palette::accent());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(crate::keymap::with_key(
            tr!(
                "The spectrum of the whole track in a window of its own",
                "Спектр всего трека в отдельном окне"
            ),
            Cmd::Spectrum,
        ))
        .clicked()
}

/// The chips of the hero: the format, the length, the bit rate, the
/// sample rate and depth, the channels, the size.
fn chips(d: &Details) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ext) = d.path.extension() {
        out.push(ext.to_string_lossy().to_uppercase());
    }
    out.push(format::time(d.duration_ms));
    if d.bitrate > 0 {
        out.push(format::bitrate(d.bitrate));
    }
    if d.sample_rate > 0 {
        out.push(format::sample_rate(d.sample_rate));
    }
    if let Some(bits) = d.bit_depth {
        out.push(tr!(format!("{bits} bit"), format!("{bits} бит")));
    }
    if let Some(c) = format::channels(d.channels) {
        out.push(c);
    }
    out.push(format::size(d.size));
    out
}

/// The value of the first of `keys` the tag has, else empty.
fn value<'a>(d: &'a Details, keys: &[ItemKey]) -> &'a str {
    keys.iter()
        .find_map(|k| d.fields.iter().find(|(f, _)| f == k))
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

/// A field's name as players and taggers show it; the others by
/// lofty's name for them, in words.
fn field_name(key: ItemKey) -> String {
    let name = match key {
        ItemKey::TrackNumber => tr!("Track #", "Трек №"),
        ItemKey::DiscNumber => tr!("Disc #", "Диск №"),
        ItemKey::TrackTitle => tr!("Title", "Название"),
        ItemKey::TrackArtist => tr!("Artist", "Исполнитель"),
        ItemKey::AlbumTitle => tr!("Album", "Альбом"),
        // ID3v2.3's TYER is read as ID3v2.4's TDRC.
        ItemKey::Year | ItemKey::RecordingDate => tr!("Year", "Год"),
        ItemKey::Genre => tr!("Genre", "Жанр"),
        ItemKey::Comment => tr!("Comment", "Комментарий"),
        ItemKey::AlbumArtist => tr!("Album artist", "Исп. альбома"),
        ItemKey::Composer => tr!("Composer", "Композитор"),
        ItemKey::Publisher => tr!("Publisher", "Издатель"),
        ItemKey::Label => tr!("Label", "Лейбл"),
        ItemKey::InternetRadioStationName => tr!("Radio station", "Радиостанция"),
        ItemKey::OriginalArtist => tr!("Orig. artist", "Ориг. исп."),
        ItemKey::CopyrightMessage => tr!("Copyright", "Копирайт"),
        ItemKey::AudioFileUrl => "URL",
        ItemKey::EncodedBy => tr!("Encoded by", "Кодировщик"),
        ItemKey::EncoderSoftware => tr!("Encoder", "Программа"),
        ItemKey::Bpm | ItemKey::IntegerBpm => "BPM",
        ItemKey::ReplayGainTrackGain => tr!("Track gain", "Усил. трека"),
        ItemKey::ReplayGainAlbumGain => tr!("Album gain", "Усил. альбома"),
        ItemKey::ReplayGainTrackPeak => tr!("Track peak", "Пик трека"),
        ItemKey::ReplayGainAlbumPeak => tr!("Album peak", "Пик альбома"),
        ItemKey::Lyricist => tr!("Lyricist", "Автор текста"),
        ItemKey::Conductor => tr!("Conductor", "Дирижёр"),
        ItemKey::Lyrics | ItemKey::UnsyncLyrics => tr!("Lyrics", "Текст"),
        ItemKey::CommercialInformationUrl => tr!("Shop URL", "URL магазина"),
        ItemKey::CopyrightUrl => tr!("Copyright URL", "URL копирайта"),
        ItemKey::TrackArtistUrl => tr!("Artist URL", "URL исполнителя"),
        ItemKey::PublisherUrl => tr!("Publisher URL", "URL издателя"),
        ItemKey::AudioSourceUrl => tr!("Source URL", "URL источника"),
        ItemKey::RadioStationUrl => tr!("Station URL", "URL станции"),
        ItemKey::PaymentUrl => tr!("Payment URL", "URL оплаты"),
        ItemKey::EncoderSettings => tr!("Encoder settings", "Настройки кодировщика"),
        ItemKey::EncodingTime => tr!("Encoded on", "Дата кодирования"),
        ItemKey::TaggingTime => tr!("Tagged on", "Дата тега"),
        ItemKey::OriginalReleaseDate => tr!("Original release", "Первый выпуск"),
        ItemKey::ReleaseDate => tr!("Released", "Выпущен"),
        ItemKey::Isrc => "ISRC",
        ItemKey::CatalogNumber => tr!("Catalogue number", "Номер в каталоге"),
        ItemKey::InitialKey => tr!("Key", "Тональность"),
        ItemKey::Mood => tr!("Mood", "Настроение"),
        ItemKey::FlagCompilation => tr!("Compilation", "Сборник"),
        ItemKey::TrackSubtitle => tr!("Subtitle", "Подзаголовок"),
        ItemKey::SetSubtitle => tr!("Disc subtitle", "Название диска"),
        ItemKey::ContentGroup => tr!("Grouping", "Группа"),
        ItemKey::OriginalAlbumTitle => tr!("Original album", "Ориг. альбом"),
        ItemKey::OriginalLyricist => tr!("Original lyricist", "Ориг. автор текста"),
        ItemKey::Arranger => tr!("Arranger", "Аранжировщик"),
        ItemKey::Writer => tr!("Writer", "Автор"),
        ItemKey::Director => tr!("Director", "Режиссёр"),
        ItemKey::Engineer => tr!("Engineer", "Звукоинженер"),
        ItemKey::MixDj => tr!("Mix DJ", "Диджей сведения"),
        ItemKey::MixEngineer => tr!("Mix engineer", "Звукорежиссёр сведения"),
        ItemKey::Performer => tr!("Performer", "Исполнители"),
        ItemKey::Producer => tr!("Producer", "Продюсер"),
        ItemKey::Remixer => tr!("Remixer", "Ремикс"),
        ItemKey::Language => tr!("Language", "Язык"),
        ItemKey::Script => tr!("Script", "Письменность"),
        ItemKey::Barcode => tr!("Barcode", "Штрихкод"),
        ItemKey::Description => tr!("Description", "Описание"),
        ItemKey::AlbumTitleSortOrder => tr!("Album sort", "Сорт. альбома"),
        ItemKey::AlbumArtistSortOrder => tr!("Album artist sort", "Сорт. исп. альбома"),
        ItemKey::TrackTitleSortOrder => tr!("Title sort", "Сорт. названия"),
        ItemKey::TrackArtistSortOrder => tr!("Artist sort", "Сорт. исполнителя"),
        ItemKey::ComposerSortOrder => tr!("Composer sort", "Сорт. композитора"),
        ItemKey::Popularimeter => tr!("Rating", "Рейтинг"),
        // MusicBrainz, AcoustID and the like: names of services, the same
        // in either language.
        other => return words(&format!("{other:?}")),
    };
    name.to_owned()
}

/// `MusicBrainzTrackId` as `MusicBrainz Track Id`: a word before each
/// capital, except within the names of services.
fn words(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out.replace("Music Brainz", "MusicBrainz")
        .replace("Acoust Id", "AcoustID")
        .replace(" Id", " ID")
        .replace(" Url", " URL")
}

fn picture_name(kind: PictureType) -> String {
    let name = match kind {
        PictureType::CoverFront => tr!("Front cover", "Обложка"),
        PictureType::CoverBack => tr!("Back cover", "Задняя обложка"),
        PictureType::Leaflet => tr!("Leaflet", "Буклет"),
        PictureType::Media => tr!("Disc", "Диск"),
        PictureType::Artist | PictureType::LeadArtist => tr!("Artist", "Исполнитель"),
        PictureType::Band => tr!("Band", "Группа"),
        PictureType::Icon => tr!("Icon", "Значок"),
        PictureType::OtherIcon => tr!("Other icon", "Другой значок"),
        PictureType::Conductor => tr!("Conductor", "Дирижёр"),
        PictureType::Composer => tr!("Composer", "Композитор"),
        PictureType::Lyricist => tr!("Lyricist", "Автор текста"),
        PictureType::RecordingLocation => tr!("Recording location", "Место записи"),
        PictureType::DuringRecording => tr!("During recording", "Во время записи"),
        PictureType::DuringPerformance => tr!("During performance", "Во время выступления"),
        PictureType::ScreenCapture => tr!("Screen capture", "Кадр из видео"),
        PictureType::BrightFish => tr!("Bright fish", "Яркая рыбка"),
        PictureType::Illustration => tr!("Illustration", "Иллюстрация"),
        PictureType::BandLogo => tr!("Band logo", "Логотип группы"),
        PictureType::PublisherLogo => tr!("Publisher logo", "Логотип издателя"),
        _ => tr!("Picture", "Картинка"),
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoders_line() {
        use crate::audio::mp3gap::Encoder;
        let lame = Encoder {
            name: "LAME3.99r".into(),
            settings: "-m j -V 4 -q 3 -lowpass 20.5".into(),
        };
        assert_eq!(
            encoder_line(Some(&lame), Some((576, 924))),
            "LAME3.99r (-m j -V 4 -q 3 -lowpass 20.5), delay 576, padding 924 samples"
        );
        let ffmpeg = Encoder {
            name: "Lavc60.3".into(),
            settings: String::new(),
        };
        assert_eq!(
            encoder_line(Some(&ffmpeg), Some((576, 576))),
            "FFmpeg (Lavc60.3), delay 576, padding 576 samples"
        );
        let old = Encoder {
            name: "LAME3.85".into(),
            settings: String::new(),
        };
        assert_eq!(encoder_line(Some(&old), None), "LAME3.85");
        assert_eq!(encoder_line(None, Some((576, 0))), "delay 576, padding 0 samples");
    }

    /// What LRCLIB answers for a folder's files (`METALAMP_MUSIC`), with
    /// the errors.
    #[test]
    #[ignore]
    fn lrclib_of_a_folder() {
        let Ok(dir) = std::env::var("METALAMP_MUSIC") else {
            return;
        };
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| crate::filetypes::is_audio(p))
            .collect();
        paths.sort();
        for path in paths {
            let t = std::time::Instant::now();
            let found = lrclib_of(&path, None, false);
            let what = match &found {
                Found::Lyrics(_) => "lyrics".to_owned(),
                Found::Failed(e) => format!("FAILED {e}"),
                f => f.word().unwrap_or("").to_owned(),
            };
            println!("{:?} {} ms: {what}", path.file_name().unwrap(), t.elapsed().as_millis());
        }
    }

    /// No other file is gone to while LRCLIB is asked, nor from all the
    /// files at once while they hold changes (going would write them into
    /// every file unasked), nor from one file holding changes.
    #[test]
    fn going_waits_for_lrclib_and_for_all_the_files_changes() {
        let ctx = egui::Context::default();
        let paths = vec![PathBuf::from("a.mp3"), PathBuf::from("b.mp3")];
        let mut form = Form::read(&ctx, paths, Id3Version::default());
        assert_eq!(form.held(true), None);
        let (tx, rx) = channel();
        form.lrclib = Some(Lrclib {
            rx,
            found: vec![Found::Lyrics("one".into()), Found::Waiting],
            one: false,
        });
        assert!(form.looking());
        assert!(form.held(false).is_some());
        assert!(form.held(true).is_some());
        if let Some(l) = form.lrclib.as_mut() {
            l.found[1] = Found::None;
        }
        drop(tx);
        assert!(!form.looking());
        // LRCLIB's lyrics go with the window, not written: going is free.
        assert!(form.touched());
        assert_eq!(form.held(true), None);
        // A cover chosen for all the files holds it.
        form.cover = Some(NewCover {
            change: CoverChange::Remove,
            preview: None,
            source: None,
        });
        assert!(form.held(true).is_some());
        // One file holding a change: the arrows wait too ("Save and next"
        // writes and goes).
        let mut one = Form::read(&ctx, vec![PathBuf::from("a.mp3")], Id3Version::default());
        assert_eq!(one.held(false), None);
        one.cover = form.cover.take();
        assert!(one.held(false).is_some());
    }

    /// From all the files at once, LRCLIB's lyrics go with the window: the
    /// file gone to gets its own into its field, nothing written, and
    /// going back to all of them brings them back.
    #[test]
    fn lrclibs_lyrics_go_with_the_file_looked_at() {
        let ctx = egui::Context::default();
        let mut view = InfoView {
            paths: vec![PathBuf::from("a.mp3"), PathBuf::from("b.mp3")],
            editing: true,
            ..Default::default()
        };
        let mut form = Form::read(&ctx, view.paths.clone(), Id3Version::default());
        form.lrclib = Some(Lrclib::kept(vec![Found::Lyrics("one".into()), Found::None]));
        view.form = Some(form);
        view.go(&ctx, Some(0));
        assert_eq!(view.at, Some(0));
        assert!(view.editing);
        let form = view.form.as_ref().unwrap();
        assert_eq!(form.paths, [PathBuf::from("a.mp3")]);
        assert_eq!(form.prefill.as_deref(), Some("one"));
        assert!(form.state == State::Reading);
        assert_eq!(view.lrclib_found.len(), 2);
        // The second file has none: its field is left as it is.
        view.go(&ctx, Some(1));
        assert_eq!(view.form.as_ref().unwrap().prefill, None);
        view.go(&ctx, None);
        let back = view.form.as_ref().unwrap().lrclib.as_ref().unwrap();
        assert_eq!(back.found, [Found::Lyrics("one".into()), Found::None]);
        assert!(view.lrclib_found.is_empty());
    }

    /// LRCLIB's lyrics of all the files at once: only those found are
    /// written, each into its own file, and the summary counts them.
    #[test]
    fn lrclibs_lyrics_go_to_their_files() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let (_, rx) = channel();
        let mut lrclib = Lrclib {
            rx,
            found: vec![
                Found::Lyrics("[00:01.00]one".into()),
                Found::Has,
                Found::None,
                Found::Waiting,
            ],
            one: false,
        };
        assert!(lrclib.busy());
        assert_eq!(lrclib.summary(), "Looking: 3 of 4 · 1 have lyrics already");
        lrclib.found[3] = Found::Lyrics("four".into());
        assert!(!lrclib.busy());
        assert_eq!(
            lrclib.per_file(),
            [Some("[00:01.00]one".into()), None, None, Some("four".into())]
        );
        assert_eq!(lrclib.summary(), "Found: 2 of 4 · 1 have lyrics already");
        // One file's go into its field, not per file.
        lrclib.one = true;
        assert!(lrclib.per_file().is_empty());
    }

    /// A lyrics file in UTF-8 (with or without a BOM) or UTF-16 (either
    /// order), its lines ending with `\n`.
    #[test]
    fn lyrics_files_read() {
        let lines = "Первая\nВторая";
        assert_eq!(text_of_file("Первая\r\nВторая".as_bytes()), lines);
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("Первая\rВторая".as_bytes());
        assert_eq!(text_of_file(&bom), lines);
        let mut le = vec![0xFF, 0xFE];
        let mut be = vec![0xFE, 0xFF];
        for unit in "Первая\r\nВторая".encode_utf16() {
            le.extend_from_slice(&unit.to_le_bytes());
            be.extend_from_slice(&unit.to_be_bytes());
        }
        assert_eq!(text_of_file(&le), lines);
        assert_eq!(text_of_file(&be), lines);
    }

    /// All the files' other fields: each key once; one all share shown,
    /// one they differ in or some lack "(different)".
    #[test]
    fn other_fields_of_several_files() {
        let values = |other: &[(ItemKey, &str)]| Values {
            other: other.iter().map(|(k, v)| (*k, v.to_string())).collect(),
            ..Default::default()
        };
        let a = values(&[(ItemKey::Publisher, "Hansa"), (ItemKey::Mood, "Calm")]);
        let b = values(&[(ItemKey::Publisher, "Hansa"), (ItemKey::Isrc, "X")]);
        let fields = other_fields(&[&a, &b]);
        let keys: Vec<ItemKey> = fields.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, [ItemKey::Publisher, ItemKey::Mood, ItemKey::Isrc]);
        assert!(!fields[0].1.mixed && fields[0].1.value == "Hansa");
        assert!(fields[1].1.mixed && fields[1].1.value.is_empty());
        assert!(fields[2].1.mixed);
    }

    #[test]
    fn names_in_words() {
        assert_eq!(words("MusicBrainzTrackId"), "MusicBrainz Track ID");
        assert_eq!(words("AcoustIdFingerprint"), "AcoustID Fingerprint");
        assert_eq!(words("Barcode"), "Barcode");
    }

    /// The zoomed artwork and its caption share the window's centre,
    /// including after a resize and for portrait and landscape covers.
    #[test]
    fn zoomed_cover_and_caption_are_centred() {
        for pixels in [[600, 600], [300, 900], [900, 300], [80, 80]] {
            let ctx = egui::Context::default();
            let texture = ctx.load_texture(
                "zoom_test",
                egui::ColorImage::filled(pixels, egui::Color32::RED),
                Default::default(),
            );
            let note = "600×600 · JPEG · 140.31 KB";
            let mut info = InfoView {
                zoom: Some(Zoom {
                    small: Some(texture.clone()),
                    big: None,
                    job: None,
                }),
                ..Default::default()
            };
            for (loaded, window) in [
                (false, vec2(2227.0, 1622.0)),
                (true, vec2(2227.0, 1622.0)),
                (true, vec2(900.0, 700.0)),
                (true, vec2(760.0, 400.0)),
            ] {
                if loaded {
                    info.zoom.as_mut().unwrap().big = Some((texture.clone(), note.into()));
                }
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, window);
                let mut out = egui::FullOutput::default();
                for _ in 0..4 {
                    out = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            ..Default::default()
                        },
                        |ui| {
                            egui::Modal::new(egui::Id::new("file_info")).show(ui.ctx(), |ui| {
                                ui.set_width(WIDTH);
                                ui.add_space(400.0);
                                info.show_zoom(ui.ctx());
                            });
                        },
                    );
                    out.textures_delta.clear();
                }
                if !loaded {
                    continue;
                }
                let image = out
                    .shapes
                    .iter()
                    .find(|s| s.shape.texture_id() == texture.id())
                    .expect("the cover was painted")
                    .shape
                    .visual_bounding_rect();
                let caption = out
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == note => {
                            Some(t.galley.rect.translate(t.pos.to_vec2()))
                        }
                        _ => None,
                    })
                    .expect("the caption was painted");
                for rect in [image, caption] {
                    assert!(
                        (rect.center().x - screen.center().x).abs() <= 1.0,
                        "{pixels:?} in {window:?}: {rect:?}"
                    );
                    assert!(screen.contains_rect(rect), "{rect:?} outside {screen:?}");
                }
                assert!(caption.top() >= image.bottom());
            }
        }
    }

    /// The hero's cover in the file info's modal, driven headless frame by
    /// frame; the right button's press comes with its release in one
    /// frame, as `App::raw_input_hook` gives it to egui.
    struct Cover {
        ctx: egui::Context,
        time: f64,
        rect: egui::Rect,
        texture: Option<TextureHandle>,
    }

    impl Cover {
        fn new() -> Cover {
            let mut c = Cover {
                ctx: egui::Context::default(),
                time: 0.0,
                rect: egui::Rect::NOTHING,
                texture: None,
            };
            c.rest(3);
            c
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1000.0, 800.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (mut rect, mut texture) = (self.rect, self.texture.clone());
            let mut out = self.ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                let t = texture
                    .get_or_insert_with(|| {
                        let image = egui::ColorImage::filled([8, 8], egui::Color32::RED);
                        ctx.load_texture("cover", image, Default::default())
                    })
                    .clone();
                egui::Modal::new(egui::Id::new("file_info")).show(&ctx, |ui| {
                    ui.set_width(WIDTH);
                    let menu = CoverMenu {
                        writable: true,
                        folder: None,
                        changed: false,
                        removable: true,
                        savable: false,
                        dropping: false,
                        busy: false,
                        note: None,
                        side: CoverSide::Original,
                    };
                    let row = ui.horizontal_top(|ui| {
                        cover_button(ui, Some(&t), &menu, &mut Vec::new());
                    });
                    rect = egui::Rect::from_min_size(row.response.rect.min, vec2(COVER, COVER));
                    ui.add_space(300.0);
                });
            });
            out.textures_delta.clear();
            (self.rect, self.texture) = (rect, texture);
        }

        fn rest(&mut self, frames: usize) {
            for _ in 0..frames {
                self.frame(vec![]);
            }
        }

        fn click(&mut self, at: egui::Pos2, button: egui::PointerButton) {
            let event = |pressed| egui::Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: Default::default(),
            };
            if button == egui::PointerButton::Secondary {
                self.frame(vec![
                    egui::Event::PointerMoved(at),
                    event(true),
                    event(false),
                ]);
            } else {
                self.frame(vec![egui::Event::PointerMoved(at), event(true)]);
                self.frame(vec![event(false)]);
            }
            self.rest(4);
        }

        /// The menu is open and on top where it opens (right of and under
        /// the pointer at `at`), not under the window.
        fn menu_seen(&self, at: egui::Pos2) -> bool {
            egui::Popup::is_any_open(&self.ctx)
                && self
                    .ctx
                    .layer_id_at(at + vec2(30.0, 15.0))
                    .is_some_and(|l| l.id != egui::Id::new("file_info"))
        }
    }

    #[test]
    fn the_covers_menu_opens_over_the_window_every_time() {
        let mut c = Cover::new();
        let at = c.rect.center();
        // The pointer resting on the cover: its tooltip shows.
        c.frame(vec![egui::Event::PointerMoved(at)]);
        c.rest(20);
        for _ in 0..3 {
            c.click(at, egui::PointerButton::Secondary);
            assert!(c.menu_seen(at), "opened by the right button");
            c.rest(20);
        }
        // A left click on the window closes the menu and raises the window
        // over the menu's layer; opened again, the menu is over it.
        for _ in 0..3 {
            c.click(at + vec2(300.0, 200.0), egui::PointerButton::Primary);
            assert!(
                !egui::Popup::is_any_open(&c.ctx),
                "closed by a click outside"
            );
            c.frame(vec![egui::Event::PointerMoved(at)]);
            c.rest(20);
            c.click(at, egui::PointerButton::Secondary);
            assert!(c.menu_seen(at), "opened again over the window");
        }
    }

    /// A file's details with the usual fields and `lyrics`.
    fn details_with(lyrics: &str) -> Details {
        let fields = [
            (ItemKey::TrackTitle, "Stargazers"),
            (ItemKey::TrackArtist, "Nightwish"),
            (ItemKey::AlbumTitle, "Oceanborn"),
            (ItemKey::AlbumArtist, "Nightwish"),
            (ItemKey::RecordingDate, "1998"),
            (ItemKey::Genre, "Symphonic Power Metal"),
            (ItemKey::TrackNumber, "01"),
            (ItemKey::UnsyncLyrics, lyrics),
        ];
        Details {
            path: PathBuf::from(r"D:\Music\Nightwish\01 - Stargazers.mp3"),
            size: 10_000_000,
            tags: vec!["ID3v2.3".to_owned()],
            fields: fields.iter().map(|(k, v)| (*k, (*v).to_owned())).collect(),
            pictures: Vec::new(),
            duration_ms: 268_000,
            bitrate: 320,
            sample_rate: 44_100,
            bit_depth: None,
            channels: 2,
            gapless: None,
            encoder: None,
            cover: None,
            tag_cover: None,
        }
    }

    /// The file info's modal around one file's tabs, laid out headless for
    /// a few frames: its height on `tab`.
    fn modal_height(d: &Details, tab: Tab) -> f32 {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        for frame in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1000.0, 1400.0),
                )),
                time: Some(frame as f64 * 0.05),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                set_tab(&ctx, tab);
                let shown = egui::Modal::new(egui::Id::new("file_info")).show(&ctx, |ui| {
                    ui.set_width(WIDTH);
                    let room = 1000.0;
                    tabbed(ui, d, None, &[], room, false, false, &mut Vec::new());
                });
                height = shown.response.rect.height();
            });
            out.textures_delta.clear();
        }
        height
    }

    #[test]
    fn the_lyrics_tab_is_as_tall_as_the_general_one() {
        let long = "A grand oasis in the vastness of gloom\n".repeat(80);
        for lyrics in [long.as_str(), "One line", ""] {
            let d = details_with(lyrics);
            let main = modal_height(&d, Tab::Main);
            let text = modal_height(&d, Tab::Lyrics);
            assert!(main > 300.0 && main < 900.0, "{main}");
            assert!((main - text).abs() < 1.0, "{main} against {text}");
        }
    }

    #[test]
    fn the_genre_lists_narrow_to_what_was_typed() {
        let library = vec!["Heavy Metal".to_owned(), "Rock".to_owned()];
        // Shown as read, or one of the genres chosen: all of them.
        for (value, touched) in [("Rock", false), ("rock", true), ("Jazz", true), ("", true)] {
            let (ours, usual) = genre_lists(value, touched, &library);
            assert_eq!(ours, ["Heavy Metal", "Rock"], "{value}");
            assert!(usual.contains(&"Jazz") && !usual.contains(&"Rock"));
        }
        let (ours, usual) = genre_lists("meta", true, &library);
        assert_eq!(ours, ["Heavy Metal"]);
        assert!(usual.contains(&"Death Metal") && !usual.contains(&"Jazz"));
        // Nothing contains it: all.
        let (ours, usual) = genre_lists("zzz", true, &library);
        assert_eq!(ours.len(), 2);
        assert!(usual.len() > 100);
        // A Cyrillic genre in another case is one of them.
        let library = vec!["Русский рок".to_owned(), "Шансон".to_owned()];
        let (ours, _) = genre_lists("русский РОК", true, &library);
        assert_eq!(ours, ["Русский рок", "Шансон"]);
    }

    /// The genre's field and its button in a modal, as `main_cell` lays
    /// them out: the line's rect.
    fn genre_frame(
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        field: &mut Field,
    ) -> egui::Rect {
        let library = vec!["Rock".to_owned()];
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(1000.0, 800.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut rect = egui::Rect::NOTHING;
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            egui::Modal::new(egui::Id::new("file_info")).show(&ctx, |ui| {
                ui.set_width(WIDTH);
                rect = ui
                    .horizontal_top(|ui| genre_field(ui, field, true, &library))
                    .response
                    .rect;
                ui.add_space(300.0);
            });
        });
        out.textures_delta.clear();
        rect
    }

    #[test]
    fn the_genre_menu_opens_over_the_window() {
        let ctx = egui::Context::default();
        let mut field = Field::of([String::from("Rock")].iter());
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.05;
            genre_frame(&ctx, time, events, &mut field)
        };
        for _ in 0..3 {
            frame(vec![]);
        }
        let line = frame(vec![]);
        // The button beside the field, on its line.
        assert!(line.height() < 30.0, "{line:?}");
        let at = egui::pos2(line.right() - MENU_BUTTON / 2.0, line.center().y);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![egui::Event::PointerMoved(at), press(true)]);
        frame(vec![press(false)]);
        for _ in 0..4 {
            frame(vec![]);
        }
        assert!(egui::Popup::is_any_open(&ctx));
        let under = egui::pos2(at.x - 40.0, line.bottom() + 40.0);
        assert!(
            ctx.layer_id_at(under)
                .is_some_and(|l| l.id != egui::Id::new("file_info")),
            "over the window"
        );
    }

    #[test]
    fn genres_are_suggested_as_they_are_typed() {
        let library = vec!["Death Metal | Doom Metal".to_owned(), "Rock".to_owned()];
        // Starting with it, then a word starting with it, then containing
        // it; the library's first.
        let s = suggestions("death", &library);
        assert_eq!(s[0], "Death Metal | Doom Metal");
        assert_eq!(s[1], "Death Metal");
        assert!(s.contains(&"Melodic Death Metal"));
        assert_eq!(suggestions("roc", &library)[0], "Rock");
        assert!(suggestions("rock", &library).iter().all(|g| *g != "Rock"));
        assert!(suggestions("  ", &library).is_empty());
        assert!(suggestions("metal", &library).len() <= SUGGESTIONS);

        // Typed in the field: Down chooses the first, Enter takes it.
        let ctx = egui::Context::default();
        let mut field = Field::of([String::new()].iter());
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.05;
            genre_frame(&ctx, time, events, &mut field)
        };
        for _ in 0..3 {
            frame(vec![]);
        }
        let line = frame(vec![]);
        let at = egui::pos2(line.left() + 40.0, line.center().y);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![egui::Event::PointerMoved(at), press(true)]);
        frame(vec![press(false)]);
        frame(vec![egui::Event::Text("roc".to_owned())]);
        let key = |key, pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(vec![key(Key::ArrowDown, true), key(Key::ArrowDown, false)]);
        frame(vec![key(Key::Enter, true), key(Key::Enter, false)]);
        frame(vec![]);
        drop(frame);
        assert_eq!(field.value, "Rock");
        assert!(field.touched);

        // A click on a suggestion takes it, though the press takes the
        // field's focus away first.
        let mut field = Field::of([String::new()].iter());
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.05;
            genre_frame(&ctx, time, events, &mut field)
        };
        frame(vec![egui::Event::PointerMoved(at), press(true)]);
        frame(vec![press(false)]);
        frame(vec![egui::Event::Text("roc".to_owned())]);
        frame(vec![]);
        let item = egui::pos2(line.left() + 40.0, line.bottom() + 16.0);
        let click = |pressed| egui::Event::PointerButton {
            pos: item,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![egui::Event::PointerMoved(item)]);
        frame(vec![click(true)]);
        frame(vec![click(false)]);
        frame(vec![]);
        drop(frame);
        assert_eq!(field.value, "Rock");
    }
}
