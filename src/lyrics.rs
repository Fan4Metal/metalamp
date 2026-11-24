//! The lyrics of the track heard, for the panel under the playlist: an
//! `.lrc` file of the track's name beside it, else its tag's lyrics (USLT,
//! a FLAC's `UNSYNCEDLYRICS`/`LYRICS`), else, when the setting allows,
//! LRCLIB (lrclib.net, an open database of lyrics, many of them synced):
//! `/api/get` by the artist, the title, the album and the length, then
//! `/api/search` by the artist and the title without a bracketed tail.
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

#[derive(Debug, PartialEq)]
pub struct Text {
    pub lines: Vec<Line>,
    /// Every line has its time, in order.
    pub synced: bool,
    pub source: Source,
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
}

struct Job {
    number: u64,
    ask: Ask,
    /// LRCLIB asked again whatever the cache holds.
    fresh: bool,
}

pub struct Lyrics {
    asks: Option<Sender<Job>>,
    answers: Option<Receiver<(u64, State)>>,
    asked: Option<Ask>,
    number: u64,
    fresh: bool,
    state: State,
}

impl Default for Lyrics {
    fn default() -> Self {
        Lyrics {
            asks: None,
            answers: None,
            asked: None,
            number: 0,
            fresh: false,
            state: State::Idle,
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
            fresh: std::mem::take(&mut self.fresh),
        };
        if let Some(asks) = &self.asks {
            let _ = asks.send(job);
        }
        self.asked = Some(ask);
        self.state = State::Looking;
    }

    /// Nothing to show: no track. An answer still coming is not taken.
    pub fn clear(&mut self) {
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

    /// LRCLIB asked again at the next `follow`, whatever the cache holds.
    pub fn search_again(&mut self) {
        self.asked = None;
        self.fresh = true;
    }
}

fn lyrics_thread(asks: &Receiver<Job>, answers: &Sender<(u64, State)>, ctx: &egui::Context) {
    while let Ok(mut job) = asks.recv() {
        // Only the last of the asks waiting.
        for later in asks.try_iter() {
            job = later;
        }
        let started = std::time::Instant::now();
        let state = look(&job.ask, job.fresh);
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

/// The lyrics of `ask`: its `.lrc` file, its tag, the cache, LRCLIB.
fn look(ask: &Ask, fresh: bool) -> State {
    if let Some(text) = lrc_beside(&ask.path) {
        return found(&text, Source::File);
    }
    if let Some(text) = crate::guard::catch(&ask.path, || tag_lyrics(&ask.path)).flatten() {
        return found(&text, Source::Tag);
    }
    if !ask.online || ask.artist.trim().is_empty() || ask.title.trim().is_empty() {
        return State::None(None);
    }
    let file = crate::settings::data_dir().map(|d| d.join(FOLDER).join(cache_name(ask)));
    if !fresh
        && let Some(cached) = file.as_deref().and_then(read_cached)
        && (cached.found || now().saturating_sub(cached.at) < RETRY_NONE)
    {
        return cached.state();
    }
    match ask_lrclib(ask) {
        Ok(record) => {
            let cached = Cached::of(record.as_ref());
            if let Some(file) = &file {
                write_cached(file, &cached);
            }
            cached.state()
        }
        Err(e) => {
            log::warn!("lyrics: LRCLIB: {e}");
            State::None(Some(e))
        }
    }
}

fn found(text: &str, source: Source) -> State {
    let (lines, synced) = parse(text);
    if lines.iter().all(|l| l.text.trim().is_empty()) {
        return State::None(None);
    }
    State::Found(Arc::new(Text {
        lines,
        synced,
        source,
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

/// The lines of `text` and whether they are synced: LRC's time tags
/// (`[mm:ss]`, `[mm:ss.xx]`, `[mm:ss.xxx]`, `[mm:ss:xx]`, several on a line
/// for a line sung more than once) give the lines their times, in order,
/// moved by an `[offset:±ms]` tag (positive: the lyrics come sooner); the
/// other tags (`[ar:…]`) and the word times of enhanced LRC (`<mm:ss.xx>`)
/// are left out. Text with no time tag is plain, its blank lines at either
/// end left out.
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
                && !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphabetic())
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

/// LRCLIB's record of the track: `get` by all it knows, else the best of
/// a search by the artist and the title; `None` when it has none.
fn ask_lrclib(ask: &Ask) -> Result<Option<Record>, String> {
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
    let (status, body) = get(&format!("/api/get?{}", query_string(&query)))?;
    match status {
        200 => {
            let record: Record = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
            if record.has_lyrics() || record.instrumental {
                return Ok(Some(record));
            }
        }
        404 => {}
        s => return Err(format!("LRCLIB answered with HTTP {s}")),
    }
    let title = without_tail(ask.title.trim());
    let query = [("artist_name", ask.artist.trim()), ("track_name", title)];
    let (status, body) = get(&format!("/api/search?{}", query_string(&query)))?;
    if status != 200 {
        return Err(format!("LRCLIB answered with HTTP {status}"));
    }
    let records: Vec<Record> = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok(best(records, ask.duration_ms))
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
        }
    }

    fn state(&self) -> State {
        if let Some(text) = self.synced.as_deref().or(self.plain.as_deref()) {
            return found(text, Source::Lrclib);
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

/// Written aside and renamed (only this thread writes them).
fn write_cached(file: &Path, cached: &Cached) {
    let written = (|| -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let bytes = serde_json::to_vec(cached).map_err(std::io::Error::other)?;
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, file)
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
    }

    #[test]
    fn the_current_line_is_the_last_begun() {
        let (lines, synced) = parse("[00:01.00]a\n[00:03.00]b\n[00:05.00]c");
        let text = Text {
            lines,
            synced,
            source: Source::Tag,
        };
        assert_eq!(text.current(0), None);
        assert_eq!(text.current(1000), Some(0));
        assert_eq!(text.current(4999), Some(1));
        assert_eq!(text.current(60_000), Some(2));
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
        match back.state() {
            State::Found(t) => {
                assert!(t.synced);
                assert_eq!(t.source, Source::Lrclib);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(Cached::of(None).state(), State::None(None));
        let instrumental = Record {
            instrumental: true,
            ..Default::default()
        };
        assert_eq!(Cached::of(Some(&instrumental)).state(), State::Instrumental);
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
        };
        let record = ask_lrclib(&ask).unwrap().unwrap();
        println!("{record:?}");
        assert!(record.has_lyrics());
    }
}
