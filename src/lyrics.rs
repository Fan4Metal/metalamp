//! The lyrics of the track heard, for the panel under the playlist: an
//! `.lrc` file of the track's name beside it, else its tag's lyrics (USLT,
//! a FLAC's `UNSYNCEDLYRICS`/`LYRICS`), else, when the setting allows,
//! LRCLIB (lrclib.net, an open database of lyrics, many of them synced):
//! `/api/get` by the artist, the title, the album and the length, then
//! `/api/search` by the artist and the title without a bracketed tail.
//! With the priority `LyricsPriority::Lrclib` LRCLIB's go before the
//! file's own (the own when it has none); a choice made for a track in the
//! panel's menu (`How::Lrclib`, `How::Own`) is kept in its cache file and
//! goes before the priority.
//! What LRCLIB answered is kept in `lyrics\<hash>.json` next to the
//! settings, by the artist, title, album and length (so a track tagged
//! anew is looked up anew); an answer of none is asked again after
//! [`RETRY_NONE`]. A failure (no network) is not kept. Text with LRC time
//! tags is synced (`parse`). One "lyrics" thread, started with the first
//! ask, takes only the last of the asks waiting.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const HOST: &str = "lrclib.net";
/// Resolving, connecting, sending and receiving each give up after this.
const TIMEOUT_MS: i32 = 10_000;
/// A search's answer lists up to 20 records with their lyrics.
const MAX_ANSWER: usize = 4 << 20;
/// The cache's folder next to the settings, and its files' version.
const FOLDER: &str = "lyrics";
const VERSION: u32 = 1;
/// LRCLIB's "none" is asked again after this many seconds (30 days):
/// lyrics are added to it all the time.
const RETRY_NONE: u64 = 30 * 24 * 60 * 60;
/// How far a record's length may be from the track's to be taken from a
/// search, in seconds (LRCLIB's `get` allows 2).
const LENGTH_SLACK: f64 = 3.0;
/// The largest `.lrc` file read, in bytes.
const LRC_MOST: u64 = 1 << 20;

/// Where the lyrics shown came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// An `.lrc` file beside the track.
    File,
    Tag,
    Lrclib,
}

/// A line; `at_ms` its time when the lyrics are synced.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub at_ms: Option<u64>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub lines: Vec<Line>,
    /// Every line has its time, in order.
    pub synced: bool,
    pub source: Source,
    /// LRCLIB's lyrics shown over the file's own: where those are.
    pub own: Option<Source>,
    /// Why the file's own lyrics are shown though LRCLIB's were asked for,
    /// or why they could not be written into the tag.
    pub note: Note,
    /// The file's tag can be written (`edit::writable`): LRCLIB's lyrics
    /// may be saved into it.
    pub writable: bool,
}

/// Something to say over the lyrics shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Note {
    #[default]
    None,
    /// LRCLIB's lyrics were asked for: it has none of the track.
    LrclibHasNone,
    /// LRCLIB's lyrics were asked for: it could not be asked (technical,
    /// in English).
    LrclibFailed(String),
    /// LRCLIB's lyrics could not be written into the tag: why.
    SaveFailed(String),
}

/// How the lyrics are looked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum How {
    /// The file's own first, unless LRCLIB's were chosen for the track;
    /// then the cache; then LRCLIB.
    #[default]
    Usual,
    /// LRCLIB asked again whatever the cache holds.
    Fresh,
    /// LRCLIB's lyrics shown over the file's own, and so from then on
    /// (kept in the cache).
    Lrclib,
    /// The file's own lyrics shown again.
    Own,
}

impl Text {
    /// The line sung at `ms`: the last one whose time has come.
    pub fn current(&self, ms: u64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let n = self
            .lines
            .partition_point(|l| l.at_ms.is_some_and(|at| at <= ms));
        n.checked_sub(1)
    }

    /// The lines as a tag keeps them: synced ones as LRC (`[mm:ss.xx]`
    /// before each line, the times as shown, any offset applied), else as
    /// they are.
    pub fn lrc(&self) -> String {
        if !self.synced {
            return self.plain();
        }
        self.lines
            .iter()
            .map(|l| {
                let ms = l.at_ms.unwrap_or(0);
                format!(
                    "[{:02}:{:02}.{:02}]{}",
                    ms / 60_000,
                    ms / 1000 % 60,
                    ms % 1000 / 10,
                    l.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The lines as text, for the clipboard.
    pub fn plain(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Nothing asked yet.
    Idle,
    Looking,
    Found(Arc<Text>),
    /// LRCLIB knows the track as an instrumental.
    Instrumental,
    /// None found; with LRCLIB's failure when it could not be asked.
    None(Option<String>),
    /// None in the file, and LRCLIB not asked: the track has no artist or
    /// no title to look for (so asking again would change nothing).
    Unnamed,
}

/// What is looked for: the file, and its tags for LRCLIB (`online`).
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub path: PathBuf,
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration_ms: u32,
    pub online: bool,
    /// LRCLIB's lyrics before the file's own (the settings' priority).
    pub lrclib_first: bool,
}

struct Job {
    number: u64,
    ask: Ask,
    how: How,
}

pub struct Lyrics {
    asks: Option<Sender<Job>>,
    answers: Option<Receiver<(u64, State)>>,
    asked: Option<Ask>,
    number: u64,
    /// How the next ask of that file looks (then back to the usual way);
    /// an ask of another file goes the usual way and drops it, so a choice
    /// left over (a save finished after the window closed) is not another
    /// track's.
    how: Option<(PathBuf, How)>,
    state: State,
    /// LRCLIB's lyrics being written into a file's tag: the file, and
    /// what came of it.
    saving: Option<(PathBuf, Receiver<Result<(), String>>)>,
}

impl Default for Lyrics {
    fn default() -> Self {
        Lyrics {
            asks: None,
            answers: None,
            asked: None,
            number: 0,
            how: None,
            state: State::Idle,
            saving: None,
        }
    }
}

impl Lyrics {
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Looks for the lyrics of `ask` when it is not what was asked last
    /// (`None`: nothing new to look for), and takes the answers.
    pub fn follow(&mut self, ctx: &egui::Context, ask: Option<Ask>) {
        if let Some(answers) = &self.answers {
            for (number, state) in answers.try_iter() {
                if number == self.number {
                    self.state = state;
                }
            }
        }
        let Some(ask) = ask else {
            return;
        };
        if self.asked.as_ref() == Some(&ask) {
            return;
        }
        if self.asks.is_none() {
            let (asks, rx) = channel::<Job>();
            let (tx, answers) = channel();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("lyrics".into())
                .spawn(move || lyrics_thread(&rx, &tx, &ctx));
            if let Err(e) = spawned {
                log::error!("cannot start the lyrics thread: {e}");
                self.state = State::None(None);
                self.asked = Some(ask);
                return;
            }
            self.asks = Some(asks);
            self.answers = Some(answers);
        }
        self.number += 1;
        let job = Job {
            number: self.number,
            ask: ask.clone(),
            how: self.take_how(&ask.path),
        };
        if let Some(asks) = &self.asks {
            let _ = asks.send(job);
        }
        self.asked = Some(ask);
        self.state = State::Looking;
    }

    /// Nothing to show: no track. An answer still coming is not taken.
    pub fn clear(&mut self) {
        self.how = None;
        if self.asked.take().is_some() || self.state != State::Idle {
            self.number += 1;
            self.state = State::Idle;
        }
    }

    /// The track's lyrics looked for again at the next `follow` (its tags
    /// were written).
    pub fn again(&mut self) {
        self.asked = None;
    }

    /// The track's lyrics looked for again `how` at the next `follow`, if
    /// it asks for the same file.
    pub fn ask_again(&mut self, how: How) {
        self.how = self.asked.take().map(|a| (a.path, how));
    }

    /// How an ask of `path` looks: as asked for that file, else the usual
    /// way; either way the choice is used up.
    fn take_how(&mut self, path: &Path) -> How {
        match self.how.take() {
            Some((p, how)) if p == path => how,
            _ => How::Usual,
        }
    }

    /// Writes LRCLIB's lyrics shown into the file's tag (as LRC when
    /// synced) on a thread, as the file info writes tags: the player lets
    /// go of the file meanwhile (`releaser`), the MP3's other tags as `how`.
    pub fn save_to_tag(
        &mut self,
        ctx: &egui::Context,
        releaser: crate::audio::Releaser,
        how: crate::library::edit::WriteTags,
    ) {
        let (Some(ask), State::Found(text)) = (&self.asked, &self.state) else {
            return;
        };
        if text.source != Source::Lrclib || !text.writable || self.saving.is_some() {
            return;
        }
        let path = ask.path.clone();
        let changes = crate::library::edit::Changes {
            lyrics: Some(text.lrc()),
            ..Default::default()
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let file = path.clone();
        let spawned = std::thread::Builder::new()
            .name("lyrics tag".into())
            .spawn(move || {
                let held = releaser.hold(vec![file.clone()]);
                let result = crate::guard::catch(&file, || {
                    crate::library::edit::write(&file, &changes, how)
                })
                .unwrap_or_else(|| Err("cannot be written".into()));
                drop(held);
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.saving = Some((path, rx)),
            Err(e) => self.save_failed(e.to_string()),
        }
    }

    /// Whether LRCLIB's lyrics were written into a file since the last
    /// call (its tags are to be read again). Written, the track's own
    /// lyrics are shown (now the same); not, why over them.
    pub fn take_written(&mut self) -> bool {
        let Some((path, rx)) = &self.saving else {
            return false;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("not written".into()),
        };
        let path = path.clone();
        self.saving = None;
        match result {
            Ok(()) => {
                log::debug!("lyrics: written into {}", path.display());
                if self.asked.as_ref().is_some_and(|a| a.path == path) {
                    self.ask_again(How::Own);
                }
                true
            }
            Err(e) => {
                log::warn!("lyrics: cannot write {}: {e}", path.display());
                if self.asked.as_ref().is_some_and(|a| a.path == path) {
                    self.save_failed(e);
                }
                false
            }
        }
    }

    /// The lyrics shown with why they were not written.
    fn save_failed(&mut self, why: String) {
        if let State::Found(text) = &self.state {
            let mut text = (**text).clone();
            text.note = Note::SaveFailed(why);
            self.state = State::Found(Arc::new(text));
        }
    }
}

fn lyrics_thread(asks: &Receiver<Job>, answers: &Sender<(u64, State)>, ctx: &egui::Context) {
    while let Ok(mut job) = asks.recv() {
        // Only the last of the asks waiting.
        for later in asks.try_iter() {
            job = later;
        }
        let started = std::time::Instant::now();
        let mut state = look(&job.ask, job.how);
        if let State::Found(text) = &mut state
            && let Some(text) = Arc::get_mut(text)
        {
            text.writable = crate::library::edit::writable(&job.ask.path);
        }
        log::debug!(
            "lyrics: {} {} in {} ms",
            job.ask.path.display(),
            match &state {
                State::Found(t) => format!("{:?}, synced {}", t.source, t.synced),
                other => format!("{other:?}"),
            },
            started.elapsed().as_millis()
        );
        if answers.send((job.number, state)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// The lyrics of `ask`, looked for `how`: its `.lrc` file, its tag, the
/// cache, LRCLIB; LRCLIB's first for a track they were chosen for.
fn look(ask: &Ask, how: How) -> State {
    let cache = crate::settings::data_dir().map(|d| d.join(FOLDER));
    look_in(ask, how, cache.as_deref(), &|a| ask_lrclib(a).map_err(|f| f.text))
}

/// What LRCLIB has for a track, for the file info's tag editor.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetched {
    /// Its lyrics as LRCLIB keeps them: synced ones as LRC, else plain.
    Lyrics(String),
    Instrumental,
    None,
    /// No artist or no title to ask by: LRCLIB was not asked.
    Unnamed,
}

/// LRCLIB's lyrics of a track by its tags, from the cache when it has
/// them (not when `fresh`), else asked (blocking; the answer kept, as the
/// panel's). The artist (else the album artist) and the title are needed.
pub fn fetch(
    artist: &str,
    title: &str,
    album: &str,
    duration_ms: u32,
    fresh: bool,
) -> Result<Fetched, Failure> {
    let ask = Ask {
        path: PathBuf::new(),
        artist: artist.to_owned(),
        title: title.to_owned(),
        album: album.to_owned(),
        duration_ms,
        online: true,
        lrclib_first: false,
    };
    if ask.artist.trim().is_empty() || ask.title.trim().is_empty() {
        return Ok(Fetched::Unnamed);
    }
    let file = crate::settings::data_dir().map(|d| d.join(FOLDER).join(cache_name(&ask)));
    let cached = match file.as_deref().and_then(read_cached) {
        Some(c) if !fresh && (c.found || now().saturating_sub(c.at) < RETRY_NONE) => c,
        kept => {
            let record = ask_lrclib(&ask)?;
            let mut answer = Cached::of(record.as_ref());
            // A choice made for the track in the panel stays: as it is
            // now, the panel may have made one while LRCLIB was asked.
            let kept = file.as_deref().and_then(read_cached).or(kept);
            if let Some(kept) = kept {
                answer.chosen = kept.chosen && answer.has_text();
                answer.own_chosen = kept.own_chosen;
            }
            if let Some(file) = &file {
                write_cached(file, &answer);
            }
            answer
        }
    };
    Ok(match cached.text() {
        Some(text) => Fetched::Lyrics(text.replace("\r\n", "\n").trim().to_owned()),
        None if cached.instrumental => Fetched::Instrumental,
        None => Fetched::None,
    })
}

/// Whether the file's tag holds lyrics.
pub fn tag_has_lyrics(path: &Path) -> bool {
    crate::guard::catch(path, || tag_lyrics(path))
        .flatten()
        .is_some()
}

/// What LRCLIB answers for a track.
type Asker<'a> = dyn Fn(&Ask) -> Result<Option<Record>, String> + 'a;

/// `look` with the cache's folder and LRCLIB given (the tests' own).
fn look_in(ask: &Ask, how: How, cache: Option<&Path>, ask_lrclib: &Asker<'_>) -> State {
    let own = lrc_beside(&ask.path)
        .map(|t| (t, Source::File))
        .or_else(|| {
            crate::guard::catch(&ask.path, || tag_lyrics(&ask.path))
                .flatten()
                .map(|t| (t, Source::Tag))
        })
        .filter(|(t, _)| has_words(t));
    let own_source = own.as_ref().map(|(_, s)| *s);
    let unnamed = ask.online && (ask.artist.trim().is_empty() || ask.title.trim().is_empty());
    let show_own = |note: Note| match &own {
        Some((text, source)) => found(text, *source, None, note),
        None if unnamed => State::Unnamed,
        None => match note {
            Note::LrclibFailed(e) => State::None(Some(e)),
            _ => State::None(None),
        },
    };
    // LRCLIB is asked by the artist and the title.
    let file = cache
        .filter(|_| !ask.artist.trim().is_empty() && !ask.title.trim().is_empty())
        .map(|d| d.join(cache_name(ask)));
    let cached = file.as_deref().and_then(read_cached);
    let fetch = |file: &Path, chosen: bool| -> Result<Cached, String> {
        let record = ask_lrclib(ask).inspect_err(|e| log::warn!("lyrics: LRCLIB: {e}"))?;
        let mut answer = Cached::of(record.as_ref());
        answer.chosen = chosen && answer.has_text();
        write_cached(file, &answer);
        Ok(answer)
    };
    // LRCLIB's lyrics go first: chosen for the track, or by the priority
    // unless the own were chosen for it.
    let chosen = cached.as_ref().is_some_and(|c| c.chosen);
    let own_chosen = cached.as_ref().is_some_and(|c| c.own_chosen);
    let lrclib_first = chosen || (ask.online && ask.lrclib_first && !own_chosen);
    match how {
        How::Own => {
            if let Some(file) = &file {
                let mut c = cached.unwrap_or_else(|| Cached::of(None));
                c.chosen = false;
                c.own_chosen = true;
                write_cached(file, &c);
            }
            if own.is_some() {
                show_own(Note::None)
            } else {
                look_in(ask, How::Usual, cache, ask_lrclib)
            }
        }
        How::Lrclib => {
            let Some(file) = &file else {
                return show_own(Note::LrclibHasNone);
            };
            let answer = match cached {
                Some(c) if c.has_text() => Ok(c),
                _ => fetch(file, true),
            };
            match answer {
                Ok(mut c) if c.has_text() => {
                    if !c.chosen || c.own_chosen {
                        c.chosen = true;
                        c.own_chosen = false;
                        write_cached(file, &c);
                    }
                    c.state(own_source)
                }
                Ok(_) => show_own(Note::LrclibHasNone),
                Err(e) => show_own(Note::LrclibFailed(e)),
            }
        }
        How::Fresh => {
            let Some(file) = &file else {
                return show_own(Note::LrclibHasNone);
            };
            let shown = lrclib_first || own.is_none();
            match fetch(file, chosen) {
                Ok(c) if c.has_text() && shown => c.state(own_source),
                Ok(c) if own.is_none() => c.state(None),
                Ok(c) if !c.has_text() => show_own(Note::LrclibHasNone),
                Ok(_) => show_own(Note::None),
                Err(e) => show_own(Note::LrclibFailed(e)),
            }
        }
        How::Usual => {
            if chosen
                && let Some(c) = &cached
                && c.has_text()
            {
                return c.state(own_source);
            }
            if own.is_some() && !lrclib_first {
                return show_own(Note::None);
            }
            let Some(file) = file.filter(|_| ask.online) else {
                return show_own(Note::None);
            };
            let answer = match cached {
                Some(c) if c.found || now().saturating_sub(c.at) < RETRY_NONE => Ok(c),
                _ => fetch(&file, false),
            };
            match answer {
                Ok(c) if c.has_text() => c.state(own_source),
                Ok(c) if own.is_none() => c.state(None),
                // LRCLIB first, without them (or not answering): the own.
                Ok(_) | Err(_) if own.is_some() => show_own(Note::None),
                Ok(_) => State::None(None),
                Err(e) => State::None(Some(e)),
            }
        }
    }
}

fn has_words(text: &str) -> bool {
    parse(text).0.iter().any(|l| !l.text.trim().is_empty())
}

fn found(text: &str, source: Source, own: Option<Source>, note: Note) -> State {
    let (lines, synced) = parse(text);
    if lines.iter().all(|l| l.text.trim().is_empty()) {
        return State::None(None);
    }
    State::Found(Arc::new(Text {
        lines,
        synced,
        source,
        own,
        note,
        writable: false,
    }))
}

/// `<name>.lrc` beside the track, read as the file info reads one.
fn lrc_beside(path: &Path) -> Option<String> {
    let lrc = path.with_extension("lrc");
    let meta = std::fs::metadata(&lrc).ok()?;
    if !meta.is_file() || meta.len() > LRC_MOST {
        return None;
    }
    let bytes = std::fs::read(&lrc).ok()?;
    Some(crate::ui::info::text_of_file(&bytes))
}

/// The lyrics of the file's main tag, if any.
fn tag_lyrics(path: &Path) -> Option<String> {
    use lofty::prelude::*;
    let file =
        crate::library::tags::open(path, lofty::config::ParseOptions::new().read_properties(false))
            .ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    [ItemKey::UnsyncLyrics, ItemKey::Lyrics]
        .iter()
        .find_map(|k| tag.get_strings(*k).next())
        .map(|t| t.replace("\r\n", "\n").replace('\r', "\n"))
        .filter(|t| !t.trim().is_empty())
}

/// LRC's ID tags, the only `[key:value]` taken for one: a section's name
/// in plain lyrics (`[Chorus: Eminem]`) is a line of them.
const ID_TAGS: &[&str] = &[
    "ar", "al", "ti", "au", "lr", "by", "re", "ve", "tool", "length", "offset", "la", "id", "#",
];

/// The lines of `text` and whether they are synced: LRC's time tags
/// (`[mm:ss]`, `[mm:ss.xx]`, `[mm:ss.xxx]`, `[mm:ss:xx]`, several on a line
/// for a line sung more than once) give the lines their times, in order,
/// moved by an `[offset:±ms]` tag (positive: the lyrics come sooner); the
/// other ID tags (`ID_TAGS`: `[ar:…]`) and the word times of enhanced LRC
/// (`<mm:ss.xx>`) are left out. Text with no time tag is plain, its blank
/// lines at either end left out.
pub fn parse(text: &str) -> (Vec<Line>, bool) {
    let mut offset: i64 = 0;
    let mut timed: Vec<Line> = Vec::new();
    let mut plain: Vec<Line> = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        let mut meta = false;
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else {
                break;
            };
            let tag = &inner[..end];
            if let Some(ms) = time_of(tag) {
                times.push(ms);
            } else if let Some((key, value)) = tag.split_once(':')
                && ID_TAGS.iter().any(|t| key.trim().eq_ignore_ascii_case(t))
            {
                meta = true;
                if key.eq_ignore_ascii_case("offset") {
                    offset = value.trim().parse().unwrap_or(0);
                }
            } else {
                break;
            }
            rest = inner[end + 1..].trim_start();
        }
        let words = without_word_times(rest).trim().to_owned();
        if times.is_empty() {
            if !(meta && words.is_empty()) {
                plain.push(Line {
                    at_ms: None,
                    text: words,
                });
            }
        } else {
            for ms in times {
                timed.push(Line {
                    at_ms: Some(ms),
                    text: words.clone(),
                });
            }
        }
    }
    if timed.is_empty() {
        let first = plain.iter().position(|l| !l.text.is_empty());
        let last = plain.iter().rposition(|l| !l.text.is_empty());
        let lines = match (first, last) {
            (Some(a), Some(b)) => plain[a..=b].to_vec(),
            _ => Vec::new(),
        };
        return (lines, false);
    }
    for line in &mut timed {
        line.at_ms = line
            .at_ms
            .map(|at| u64::try_from(at as i64 - offset).unwrap_or(0));
    }
    timed.sort_by_key(|l| l.at_ms);
    (timed, true)
}

/// A time tag's milliseconds: `mm:ss`, `mm:ss.f…` or `mm:ss:ff`.
fn time_of(tag: &str) -> Option<u64> {
    let (minutes, rest) = tag.split_once(':')?;
    let (seconds, fraction) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(minutes) || !digits(seconds) || !(fraction.is_empty() || digits(fraction)) {
        return None;
    }
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: u64 = seconds.parse().ok()?;
    // Hundredths as most write them, tenths or thousandths as some do.
    let ms = match fraction.len() {
        0 => 0,
        n => {
            let f: u64 = fraction[..n.min(3)].parse().ok()?;
            f * 10u64.pow(3 - n.min(3) as u32)
        }
    };
    Some((minutes * 60 + seconds) * 1000 + ms)
}

/// `text` without enhanced LRC's word times (`<01:02.34>`).
fn without_word_times(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        match rest[start + 1..].find('>') {
            Some(end) if time_of(&rest[start + 1..start + 1 + end]).is_some() => {
                out.push_str(&rest[..start]);
                rest = &rest[start + 1 + end + 1..];
            }
            _ => {
                out.push_str(&rest[..=start]);
                rest = &rest[start + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A record of LRCLIB's answers (the fields used).
#[derive(Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Record {
    duration: Option<f64>,
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl Record {
    fn has_lyrics(&self) -> bool {
        let some = |s: &Option<String>| s.as_deref().is_some_and(|s| !s.trim().is_empty());
        some(&self.synced_lyrics) || some(&self.plain_lyrics)
    }
}

/// Why LRCLIB gave no record: its words (technical, English) and what
/// kind of failure it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub text: String,
    pub kind: Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failed {
    /// No answer (no network, a timeout): other tracks asked now fail
    /// alike.
    Unreachable,
    /// LRCLIB busy or down for a moment (HTTP 5xx, 429): asked again.
    Busy,
    /// An answer that asking again would not change (another status, an
    /// answer not read).
    Refused,
}

impl Failure {
    fn unreachable(text: String) -> Failure {
        Failure {
            text,
            kind: Failed::Unreachable,
        }
    }

    fn status(status: u32) -> Failure {
        Failure {
            text: format!("LRCLIB answered with HTTP {status}"),
            kind: if status >= 500 || status == 429 {
                Failed::Busy
            } else {
                Failed::Refused
            },
        }
    }

    fn unread(e: &serde_json::Error) -> Failure {
        Failure {
            text: e.to_string(),
            kind: Failed::Refused,
        }
    }
}

/// `ask_lrclib_once`, made once more after a moment when LRCLIB did not
/// answer or was busy: it is slow at times (the panel's and the file
/// info's asks alike).
fn ask_lrclib(ask: &Ask) -> Result<Option<Record>, Failure> {
    ask_lrclib_once(ask).or_else(|e| {
        if e.kind == Failed::Refused {
            return Err(e);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        ask_lrclib_once(ask)
    })
}

/// LRCLIB's record of the track: `get` by all it knows, else the best of
/// a search by the artist and the title; `None` when it has none.
fn ask_lrclib_once(ask: &Ask) -> Result<Option<Record>, Failure> {
    ask_lrclib_by(ask, &get)
}

/// What LRCLIB answers to a path (status and body, or no answer).
type Getter<'a> = dyn Fn(&str) -> Result<(u32, Vec<u8>), String> + 'a;

/// `ask_lrclib_once` with the way to LRCLIB given (the tests' own). A
/// `get` that fails other than by "none" (404) still leaves the search;
/// its failure is told only when the search finds nothing either.
fn ask_lrclib_by(ask: &Ask, get: &Getter<'_>) -> Result<Option<Record>, Failure> {
    let seconds = (ask.duration_ms as f64 / 1000.0).round() as u64;
    let mut query = vec![
        ("artist_name", ask.artist.trim()),
        ("track_name", ask.title.trim()),
    ];
    if !ask.album.trim().is_empty() {
        query.push(("album_name", ask.album.trim()));
    }
    let length = seconds.to_string();
    if seconds > 0 {
        query.push(("duration", &length));
    }
    let (status, body) =
        get(&format!("/api/get?{}", query_string(&query))).map_err(Failure::unreachable)?;
    let mut failed = None;
    match status {
        200 => match serde_json::from_slice::<Record>(&body) {
            Ok(record) if record.has_lyrics() || record.instrumental => return Ok(Some(record)),
            Ok(_) => {}
            Err(e) => failed = Some(Failure::unread(&e)),
        },
        404 => {}
        s => failed = Some(Failure::status(s)),
    }
    let title = without_tail(ask.title.trim());
    let query = [("artist_name", ask.artist.trim()), ("track_name", title)];
    let (status, body) =
        get(&format!("/api/search?{}", query_string(&query))).map_err(Failure::unreachable)?;
    if status != 200 {
        return Err(Failure::status(status));
    }
    let records: Vec<Record> = serde_json::from_slice(&body).map_err(|e| Failure::unread(&e))?;
    match (best(records, ask.duration_ms), failed) {
        (Some(record), _) => Ok(Some(record)),
        (None, Some(failed)) => Err(failed),
        (None, None) => Ok(None),
    }
}

/// The search's record to take: of the track's length (when known), the
/// synced first, then with lyrics, then an instrumental; the nearest length
/// among them.
fn best(records: Vec<Record>, duration_ms: u32) -> Option<Record> {
    let length = f64::from(duration_ms) / 1000.0;
    let off = |r: &Record| {
        if duration_ms == 0 {
            0.0
        } else {
            r.duration.map_or(f64::INFINITY, |d| (d - length).abs())
        }
    };
    let rank = |r: &Record| {
        if r.synced_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty()) {
            0
        } else if r.has_lyrics() {
            1
        } else {
            2
        }
    };
    records
        .into_iter()
        .filter(|r| (r.has_lyrics() || r.instrumental) && off(r) <= LENGTH_SLACK)
        .min_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then(off(a).total_cmp(&off(b)))
        })
}

/// The title without a bracketed tail: "Wait for Me (Bonus Track)" as
/// "Wait for Me" (LRCLIB's search ranks by words, and such tails are
/// rarely in its titles).
fn without_tail(title: &str) -> &str {
    let mut t = title.trim_end();
    while let Some(open) = [('(', ')'), ('[', ']')]
        .iter()
        .find(|(_, close)| t.ends_with(*close))
        .and_then(|(open, _)| t.rfind(*open))
        .filter(|&i| i > 0)
    {
        t = t[..open].trim_end();
    }
    if t.is_empty() { title } else { t }
}

fn get(path: &str) -> Result<(u32, Vec<u8>), String> {
    let agent = format!(
        "{}/{} ({})",
        crate::APP_NAME,
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY")
    );
    crate::win::https_get(HOST, path, &agent, "", TIMEOUT_MS, MAX_ANSWER)
}

/// `key=value&…`, percent-encoded as UTF-8 (all but the unreserved).
fn query_string(pairs: &[(&str, &str)]) -> String {
    fn encode(text: &str, out: &mut String) {
        for b in text.bytes() {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                out.push(char::from(b));
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    let mut out = String::new();
    for (i, (key, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        encode(key, &mut out);
        out.push('=');
        encode(value, &mut out);
    }
    out
}

/// What LRCLIB answered for a track, as kept.
#[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default)]
struct Cached {
    version: u32,
    /// Unix seconds of the answer.
    at: u64,
    /// LRCLIB had the track (with lyrics or as an instrumental).
    found: bool,
    instrumental: bool,
    synced: Option<String>,
    plain: Option<String>,
    /// Shown over the file's own lyrics (chosen in the panel's menu).
    chosen: bool,
    /// The file's own shown over these (chosen in the panel's menu), when
    /// LRCLIB's come first by the priority.
    own_chosen: bool,
}

impl Cached {
    fn of(record: Option<&Record>) -> Cached {
        Cached {
            version: VERSION,
            at: now(),
            found: record.is_some(),
            instrumental: record.is_some_and(|r| r.instrumental && !r.has_lyrics()),
            synced: record
                .and_then(|r| r.synced_lyrics.clone())
                .filter(|s| !s.trim().is_empty()),
            plain: record
                .and_then(|r| r.plain_lyrics.clone())
                .filter(|s| !s.trim().is_empty()),
            chosen: false,
            own_chosen: false,
        }
    }

    fn text(&self) -> Option<&str> {
        self.synced
            .as_deref()
            .or(self.plain.as_deref())
            .filter(|t| has_words(t))
    }

    fn has_text(&self) -> bool {
        self.text().is_some()
    }

    /// As shown; `own` where the file's own lyrics are, if any.
    fn state(&self, own: Option<Source>) -> State {
        if let Some(text) = self.text() {
            return found(text, Source::Lrclib, own, Note::None);
        }
        if self.instrumental {
            State::Instrumental
        } else {
            State::None(None)
        }
    }
}

/// The cache's file of a track: an FNV-1a hash of its artist, title, album
/// and length in seconds, as LRCLIB is asked by them.
fn cache_name(ask: &Ask) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let seconds = (ask.duration_ms as f64 / 1000.0).round() as u64;
    let key = format!(
        "{}\n{}\n{}\n{seconds}",
        ask.artist.trim().to_lowercase(),
        ask.title.trim().to_lowercase(),
        ask.album.trim().to_lowercase()
    );
    for b in key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}.json")
}

fn read_cached(file: &Path) -> Option<Cached> {
    let bytes = std::fs::read(file).ok()?;
    serde_json::from_slice::<Cached>(&bytes)
        .ok()
        .filter(|c| c.version == VERSION)
}

/// Written aside and renamed. The panel's thread and the tag editor's
/// (several at once) may write a track's at the same time: each write has
/// a file aside of its own, so none is written over half-done.
fn write_cached(file: &Path, cached: &Cached) {
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let written = (|| -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let bytes = serde_json::to_vec(cached).map_err(std::io::Error::other)?;
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = file.with_extension(format!("{}.{n}.tmp", std::process::id()));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, file).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    })();
    if let Err(e) = written {
        log::warn!("cannot write {}: {e}", file.display());
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(lines: &[Line]) -> Vec<Option<u64>> {
        lines.iter().map(|l| l.at_ms).collect()
    }

    #[test]
    fn time_tags() {
        assert_eq!(time_of("01:02"), Some(62_000));
        assert_eq!(time_of("01:02.5"), Some(62_500));
        assert_eq!(time_of("01:02.34"), Some(62_340));
        assert_eq!(time_of("01:02.345"), Some(62_345));
        assert_eq!(time_of("01:02:34"), Some(62_340));
        assert_eq!(time_of("ar:Nightwish"), None);
        assert_eq!(time_of("1:x"), None);
        assert_eq!(time_of(""), None);
    }

    #[test]
    fn synced_lyrics_are_parsed() {
        let text = "[ar:Somebody]\n[ti:Song]\n[offset:+500]\n\
                    [00:12.00]First line\n[00:15.50][01:15.50]Chorus\n\
                    [00:20.00]\n[00:21.00]<00:21.00>Word <00:21.50>times\n";
        let (lines, synced) = parse(text);
        assert!(synced);
        assert_eq!(
            times(&lines),
            vec![
                Some(11_500),
                Some(15_000),
                Some(19_500),
                Some(20_500),
                Some(75_000)
            ]
        );
        assert_eq!(lines[0].text, "First line");
        assert_eq!(lines[2].text, "");
        assert_eq!(lines[3].text, "Word times");
        assert_eq!(lines[4].text, "Chorus");
    }

    #[test]
    fn plain_lyrics_keep_their_blank_lines_inside() {
        let (lines, synced) = parse("\n\nVerse one\nline two\n\nChorus [x2]\n\n");
        assert!(!synced);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Verse one", "line two", "", "Chorus [x2]"]);
        assert!(lines.iter().all(|l| l.at_ms.is_none()));
        // A bracket that is no tag starts a line as it is.
        let (lines, _) = parse("[Chorus]\nla la");
        assert_eq!(lines[0].text, "[Chorus]");
        // Nor is a section's name with a colon; an ID tag is.
        let (lines, _) = parse("[ar:Somebody]\n[Chorus: Eminem]\nla la\n[Intro:]");
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["[Chorus: Eminem]", "la la", "[Intro:]"]);
    }

    #[test]
    fn the_current_line_is_the_last_begun() {
        let (lines, synced) = parse("[00:01.00]a\n[00:03.00]b\n[00:05.00]c");
        let text = Text {
            lines,
            synced,
            source: Source::Tag,
            own: None,
            note: Note::None,
            writable: false,
        };
        assert_eq!(text.current(0), None);
        assert_eq!(text.current(1000), Some(0));
        assert_eq!(text.current(4999), Some(1));
        assert_eq!(text.current(60_000), Some(2));
    }

    #[test]
    fn synced_lyrics_go_back_to_lrc() {
        let (lines, synced) = parse("[offset:+500]\n[00:02.00]a\n[01:02.53]\n[61:00.00]c");
        let text = Text {
            lines,
            synced,
            source: Source::Lrclib,
            own: None,
            note: Note::None,
            writable: true,
        };
        assert_eq!(text.lrc(), "[00:01.50]a\n[01:02.03]\n[60:59.50]c");
        assert_eq!(parse(&text.lrc()), (text.lines.clone(), true));
        let (lines, synced) = parse("one\n\ntwo");
        let plain = Text {
            lines,
            synced,
            ..text
        };
        assert_eq!(plain.lrc(), "one\n\ntwo");
    }

    #[test]
    fn queries_are_percent_encoded() {
        assert_eq!(
            query_string(&[("artist_name", "AC/DC & Co"), ("track_name", "Élan")]),
            "artist_name=AC%2FDC%20%26%20Co&track_name=%C3%89lan"
        );
    }

    #[test]
    fn bracketed_tails_are_cut_for_the_search() {
        assert_eq!(without_tail("Wait for Me (Bonus Track)"), "Wait for Me");
        assert_eq!(
            without_tail("To the Top (Instrumental Version) [Live]"),
            "To the Top"
        );
        assert_eq!(without_tail("(Untitled)"), "(Untitled)");
        assert_eq!(without_tail("Plain"), "Plain");
    }

    #[test]
    fn the_best_record_is_synced_and_of_the_length() {
        let record = |d: f64, synced: bool, plain: bool| Record {
            duration: Some(d),
            instrumental: false,
            plain_lyrics: plain.then(|| "words".into()),
            synced_lyrics: synced.then(|| "[00:01.00]words".into()),
        };
        let chosen = best(
            vec![
                record(200.0, true, true),
                record(241.0, false, true),
                record(240.5, true, true),
                record(239.0, true, true),
            ],
            240_000,
        )
        .unwrap();
        assert_eq!(chosen.duration, Some(240.5));
        assert!(best(vec![record(250.0, true, true)], 240_000).is_none());
        // The length unknown: any.
        assert!(best(vec![record(250.0, false, true)], 0).is_some());
    }

    #[test]
    fn the_cache_reads_back() {
        let record = Record {
            duration: Some(200.0),
            instrumental: false,
            plain_lyrics: Some("a\nb".into()),
            synced_lyrics: Some("[00:01.00]a\n[00:02.00]b".into()),
        };
        let dir = std::env::temp_dir().join(format!("metalamp-lyrics-{}", std::process::id()));
        let file = dir.join("x.json");
        let cached = Cached::of(Some(&record));
        write_cached(&file, &cached);
        let back = read_cached(&file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(back, cached);
        match back.state(Some(Source::Tag)) {
            State::Found(t) => {
                assert!(t.synced);
                assert_eq!(t.source, Source::Lrclib);
                assert_eq!(t.own, Some(Source::Tag));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(Cached::of(None).state(None), State::None(None));
        let instrumental = Record {
            instrumental: true,
            ..Default::default()
        };
        assert_eq!(Cached::of(Some(&instrumental)).state(None), State::Instrumental);
    }

    #[test]
    fn the_cache_name_follows_the_tags() {
        let ask = Ask {
            path: PathBuf::from("a.mp3"),
            artist: "Nightwish".into(),
            title: "Élan".into(),
            album: "Endless Forms Most Beautiful".into(),
            duration_ms: 285_400,
            online: true,
            lrclib_first: false,
        };
        let moved = Ask {
            path: PathBuf::from("b.mp3"),
            artist: "NIGHTWISH ".into(),
            ..ask.clone()
        };
        assert_eq!(cache_name(&ask), cache_name(&moved));
        let retitled = Ask {
            title: "Elan".into(),
            ..ask.clone()
        };
        assert_ne!(cache_name(&ask), cache_name(&retitled));
    }

    /// The ways of looking with an `.lrc` file of its own, a cache in a
    /// temporary folder and an LRCLIB of the test's.
    #[test]
    fn lrclibs_lyrics_are_shown_over_the_files_own_when_chosen() {
        use std::cell::Cell;
        let dir = std::env::temp_dir().join(format!("metalamp-look-{}", std::process::id()));
        let cache = dir.join("cache");
        std::fs::create_dir_all(&dir).unwrap();
        let track = dir.join("song.mp3");
        std::fs::write(dir.join("song.lrc"), "Own words\n").unwrap();
        let ask = Ask {
            path: track,
            artist: "Artist".into(),
            title: "Song".into(),
            album: String::new(),
            duration_ms: 200_000,
            online: true,
            lrclib_first: false,
        };
        let asked = Cell::new(0);
        let has = Cell::new(true);
        let lrclib = |_: &Ask| -> Result<Option<Record>, String> {
            asked.set(asked.get() + 1);
            Ok(has.get().then(|| Record {
                duration: Some(200.0),
                synced_lyrics: Some("[00:01.00]Their words".into()),
                ..Default::default()
            }))
        };
        let look = |how| look_in(&ask, how, Some(&cache), &lrclib);
        let text = |state: State| match state {
            State::Found(t) => t,
            other => panic!("{other:?}"),
        };

        // The own lyrics, LRCLIB not asked.
        let t = text(look(How::Usual));
        assert_eq!((t.source, t.own, &t.note), (Source::File, None, &Note::None));
        assert_eq!(asked.get(), 0);
        // LRCLIB's chosen, and so from then on, from the cache.
        let t = text(look(How::Lrclib));
        assert_eq!((t.source, t.own), (Source::Lrclib, Some(Source::File)));
        assert_eq!(t.lines[0].text, "Their words");
        let t = text(look(How::Usual));
        assert_eq!(t.source, Source::Lrclib);
        assert_eq!(asked.get(), 1);
        // Asked again: still LRCLIB's.
        let t = text(look(How::Fresh));
        assert_eq!(t.source, Source::Lrclib);
        assert_eq!(asked.get(), 2);
        // Back to the own, and so from then on.
        assert_eq!(text(look(How::Own)).source, Source::File);
        assert_eq!(text(look(How::Usual)).source, Source::File);
        assert_eq!(asked.get(), 2);
        // LRCLIB without them: the own, with a note why.
        let _ = std::fs::remove_dir_all(&cache);
        has.set(false);
        let t = text(look(How::Lrclib));
        assert_eq!((t.source, &t.note), (Source::File, &Note::LrclibHasNone));
        assert_eq!(text(look(How::Usual)).source, Source::File);
        // No own lyrics: LRCLIB's at once, none when it has none.
        std::fs::remove_file(dir.join("song.lrc")).unwrap();
        assert_eq!(look(How::Usual), State::None(None));
        has.set(true);
        let _ = std::fs::remove_dir_all(&cache);
        let t = text(look(How::Usual));
        assert_eq!((t.source, t.own), (Source::Lrclib, None));
        // No title: LRCLIB not asked, and nothing to ask again.
        let before = asked.get();
        let unnamed = Ask {
            title: String::new(),
            ..ask.clone()
        };
        for how in [How::Usual, How::Fresh, How::Lrclib] {
            assert_eq!(look_in(&unnamed, how, Some(&cache), &lrclib), State::Unnamed);
        }
        assert_eq!(asked.get(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// LRCLIB first by the priority: its lyrics over the file's own, the
    /// own when it has none or they are chosen for the track.
    /// A choice of how to ask is its own file's: an ask of another file
    /// goes the usual way and the choice is gone; so is it once cleared.
    #[test]
    fn a_choice_of_how_is_its_own_files() {
        let ask = |path: &str| Ask {
            path: PathBuf::from(path),
            artist: "A".into(),
            title: "T".into(),
            album: String::new(),
            duration_ms: 0,
            online: false,
            lrclib_first: false,
        };
        let mut lyrics = Lyrics {
            asked: Some(ask("a.mp3")),
            ..Lyrics::default()
        };
        lyrics.ask_again(How::Own);
        assert_eq!(lyrics.take_how(Path::new("a.mp3")), How::Own);
        assert_eq!(lyrics.take_how(Path::new("a.mp3")), How::Usual);
        // The save finished after the window closed: another track later.
        lyrics.asked = Some(ask("a.mp3"));
        lyrics.ask_again(How::Own);
        assert_eq!(lyrics.take_how(Path::new("c.mp3")), How::Usual);
        assert_eq!(lyrics.take_how(Path::new("a.mp3")), How::Usual);
        lyrics.asked = Some(ask("a.mp3"));
        lyrics.ask_again(How::Lrclib);
        lyrics.clear();
        assert_eq!(lyrics.take_how(Path::new("a.mp3")), How::Usual);
        // Nothing asked: nothing to choose for.
        lyrics.ask_again(How::Fresh);
        assert_eq!(lyrics.how, None);
    }

    #[test]
    fn the_priority_puts_lrclib_first() {
        use std::cell::Cell;
        let dir = std::env::temp_dir().join(format!("metalamp-first-{}", std::process::id()));
        let cache = dir.join("cache");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("song.lrc"), "Own words\n").unwrap();
        let ask = Ask {
            path: dir.join("song.mp3"),
            artist: "Artist".into(),
            title: "Song".into(),
            album: String::new(),
            duration_ms: 200_000,
            online: true,
            lrclib_first: true,
        };
        let has = Cell::new(true);
        let lrclib = |_: &Ask| -> Result<Option<Record>, String> {
            Ok(has.get().then(|| Record {
                duration: Some(200.0),
                plain_lyrics: Some("Their words".into()),
                ..Default::default()
            }))
        };
        let source = |ask: &Ask, how| match look_in(ask, how, Some(&cache), &lrclib) {
            State::Found(t) => t.source,
            other => panic!("{other:?}"),
        };
        assert_eq!(source(&ask, How::Usual), Source::Lrclib);
        // The own chosen for the track, and so from then on.
        assert_eq!(source(&ask, How::Own), Source::File);
        assert_eq!(source(&ask, How::Usual), Source::File);
        // LRCLIB's chosen again.
        assert_eq!(source(&ask, How::Lrclib), Source::Lrclib);
        assert_eq!(source(&ask, How::Usual), Source::Lrclib);
        // Offline: the own.
        let offline = Ask {
            online: false,
            ..ask.clone()
        };
        let _ = std::fs::remove_dir_all(&cache);
        assert_eq!(source(&offline, How::Usual), Source::File);
        // LRCLIB without them: the own.
        has.set(false);
        assert_eq!(source(&ask, How::Usual), Source::File);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With `METALAMP_SAMPLE`: synced lyrics written into a copy of it as
    /// LRC are read back synced, the same.
    #[test]
    #[ignore]
    fn lrc_written_into_a_copy_reads_back_synced() {
        let sample = PathBuf::from(std::env::var("METALAMP_SAMPLE").expect("METALAMP_SAMPLE"));
        let copy = std::env::temp_dir().join(format!(
            "metalamp-lrc-{}.{}",
            std::process::id(),
            sample.extension().unwrap().to_string_lossy()
        ));
        std::fs::copy(&sample, &copy).unwrap();
        // The copy takes a read-only file's attribute.
        let mut permissions = std::fs::metadata(&copy).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&copy, permissions).unwrap();
        let (lines, synced) = parse("[00:01.00]First\n[00:03.25]Second line\n[00:05.00]");
        let text = Text {
            lines,
            synced,
            source: Source::Lrclib,
            own: None,
            note: Note::None,
            writable: true,
        };
        let changes = crate::library::edit::Changes {
            lyrics: Some(text.lrc()),
            ..Default::default()
        };
        let written = crate::library::edit::write(&copy, &changes, Default::default());
        let back = tag_lyrics(&copy);
        let _ = std::fs::remove_file(&copy);
        written.unwrap();
        let back = back.expect("lyrics in the tag");
        println!("{back}");
        assert_eq!(parse(&back), (text.lines, true));
    }

    /// A track's cache written by several threads at once (the panel's,
    /// the tag editor's): each write whole, none left aside.
    #[test]
    fn the_cache_is_written_by_several_at_once() {
        let dir = std::env::temp_dir().join(format!("metalamp-lyrics-{}", std::process::id()));
        let file = dir.join("0123456789abcdef.json");
        std::thread::scope(|s| {
            for t in 0..8 {
                let file = &file;
                s.spawn(move || {
                    for i in 0..40 {
                        let mut c = Cached::of(None);
                        c.chosen = (t + i) % 2 == 0;
                        write_cached(file, &c);
                    }
                });
            }
        });
        assert!(read_cached(&file).is_some());
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(names, ["0123456789abcdef.json"]);
    }

    /// LRCLIB's answers: a `get` failing other than by "none" still
    /// leaves the search, its failure told when the search finds nothing;
    /// a status says whether asking again may help, no answer that LRCLIB
    /// cannot be reached.
    #[test]
    fn lrclib_failures_are_told_apart() {
        let ask = Ask {
            path: PathBuf::new(),
            artist: "A".into(),
            title: "T".into(),
            album: String::new(),
            duration_ms: 200_000,
            online: true,
            lrclib_first: false,
        };
        let found = br#"[{"duration":200.0,"plainLyrics":"la la"}]"#.to_vec();
        let answer = |get: u32, search: (u32, Vec<u8>)| {
            move |path: &str| -> Result<(u32, Vec<u8>), String> {
                Ok(if path.starts_with("/api/get") {
                    (get, b"{}".to_vec())
                } else {
                    search.clone()
                })
            }
        };
        for status in [400, 500, 429] {
            let got = ask_lrclib_by(&ask, &answer(status, (200, found.clone()))).unwrap();
            assert_eq!(got.unwrap().plain_lyrics.as_deref(), Some("la la"), "{status}");
        }
        let kind = |get, search| ask_lrclib_by(&ask, &answer(get, search)).unwrap_err().kind;
        assert_eq!(kind(500, (200, b"[]".to_vec())), Failed::Busy);
        assert_eq!(kind(429, (200, b"[]".to_vec())), Failed::Busy);
        assert_eq!(kind(400, (200, b"[]".to_vec())), Failed::Refused);
        assert_eq!(kind(404, (503, Vec::new())), Failed::Busy);
        assert_eq!(kind(404, (200, b"not json".to_vec())), Failed::Refused);
        assert!(ask_lrclib_by(&ask, &answer(404, (200, b"[]".to_vec()))).unwrap().is_none());
        let none = |_: &str| -> Result<(u32, Vec<u8>), String> { Err("timed out".into()) };
        assert_eq!(ask_lrclib_by(&ask, &none).unwrap_err().kind, Failed::Unreachable);
    }

    /// LRCLIB asked for real, with a well-known song (not the user's).
    #[test]
    #[ignore]
    fn lrclib_answers() {
        let ask = Ask {
            path: PathBuf::from("none.mp3"),
            artist: "Queen".into(),
            title: "Bohemian Rhapsody".into(),
            album: String::new(),
            duration_ms: 355_000,
            online: true,
            lrclib_first: false,
        };
        let record = ask_lrclib(&ask).unwrap().unwrap();
        println!("{record:?}");
        assert!(record.has_lyrics());
    }
}
