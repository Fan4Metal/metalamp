//! The playlist: the tracks to play in order, the one playing, repeat and
//! shuffle (after metalmpv's `playlist.rs`, without its IPTV and address
//! parts). Each entry has an id of its own, so the audio thread can say
//! which one it started even after the list changed, and a track can be
//! listed more than once.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    /// The whole list, round again after its end.
    All,
    /// The track playing, again and again.
    One,
}

impl Repeat {
    /// The next in the button's cycle: off, all, one.
    pub fn next(self) -> Repeat {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub path: PathBuf,
}

#[derive(Default)]
pub struct Queue {
    entries: Vec<Entry>,
    current: Option<usize>,
    last_id: u64,
    pub repeat: Repeat,
    shuffle: bool,
    /// With shuffle on: the order of play, as indices into `entries`.
    order: Vec<usize>,
    seed: u64,
    /// Counts the changes, for views made of the list.
    pub revision: u64,
}

impl Queue {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        self.current.map(|i| &self.entries[i])
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    fn new_entry(&mut self, path: PathBuf) -> Entry {
        self.last_id += 1;
        Entry {
            id: self.last_id,
            path,
        }
    }

    /// The list becomes `paths`, nothing current.
    pub fn replace(&mut self, paths: Vec<PathBuf>) {
        self.entries = paths.into_iter().map(|p| self.new_entry(p)).collect();
        self.current = None;
        self.reshuffle();
        self.revision += 1;
    }

    /// `paths` at the list's end; the index of the first added.
    pub fn add(&mut self, paths: Vec<PathBuf>) -> usize {
        let first = self.entries.len();
        let new: Vec<Entry> = paths.into_iter().map(|p| self.new_entry(p)).collect();
        self.entries.extend(new);
        if self.shuffle {
            // The new ones are shuffled into what is still to come.
            let added: Vec<usize> = (first..self.entries.len()).collect();
            let at = self
                .current
                .and_then(|c| self.order.iter().position(|&i| i == c))
                .map_or(0, |p| p + 1);
            let mut rest: Vec<usize> = self.order.split_off(at);
            rest.extend(added);
            self.shuffle_slice(&mut rest);
            self.order.extend(rest);
        }
        self.revision += 1;
        first
    }

    /// Removes the entries at `indices`; the current one stays current if
    /// it is kept. When it is not, returns the kept entry that came after
    /// it in the order of play (round to the first with repeat all), to go
    /// on with. The order of play stays as it was for the rest.
    pub fn remove(&mut self, indices: &[usize]) -> Option<usize> {
        let mut keep = vec![true; self.entries.len()];
        for &i in indices {
            if i < keep.len() {
                keep[i] = false;
            }
        }
        let current_id = self.current_entry().map(|e| e.id);
        let next_id = self.current.filter(|&c| !keep[c]).and_then(|c| {
            let order = self.play_order();
            let p = order.iter().position(|&o| o == c)?;
            let round = if self.repeat == Repeat::All {
                &order[..p]
            } else {
                &[]
            };
            order[p + 1..]
                .iter()
                .chain(round)
                .find(|&&i| keep[i])
                .map(|&i| self.entries[i].id)
        });
        let ids = self.order_ids();
        let mut k = keep.iter();
        self.entries.retain(|_| *k.next().unwrap_or(&true));
        self.current = current_id.and_then(|id| self.index_of(id));
        self.reorder(&ids);
        self.revision += 1;
        next_id.and_then(|id| self.index_of(id))
    }

    pub fn clear(&mut self) {
        self.replace(Vec::new());
    }

    /// Moves the entries at `indices` (in the list's order) to before
    /// `slot` (an index counted before the move; the length for the end).
    /// Returns where they now start.
    pub fn move_to(&mut self, indices: &[usize], slot: usize) -> usize {
        let mut sorted: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| i < self.entries.len())
            .collect();
        sorted.sort_unstable();
        sorted.dedup();
        let current_id = self.current_entry().map(|e| e.id);
        let ids = self.order_ids();
        let before = sorted.iter().filter(|&&i| i < slot).count();
        let moved: Vec<Entry> = sorted.iter().map(|&i| self.entries[i].clone()).collect();
        for &i in sorted.iter().rev() {
            self.entries.remove(i);
        }
        let at = (slot - before).min(self.entries.len());
        for (k, e) in moved.into_iter().enumerate() {
            self.entries.insert(at + k, e);
        }
        self.current = current_id.and_then(|id| self.index_of(id));
        self.reorder(&ids);
        self.revision += 1;
        at
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    pub fn set_current(&mut self, index: Option<usize>) {
        self.current = index.filter(|&i| i < self.entries.len());
    }

    /// The entry to play after the one at `from` (or first, from nothing),
    /// as repeat and shuffle have it; `None` at the end.
    pub fn after(&self, from: Option<usize>) -> Option<usize> {
        let n = self.entries.len();
        if n == 0 {
            return None;
        }
        if self.repeat == Repeat::One
            && let Some(i) = from
        {
            return Some(i);
        }
        let len = self.order_len();
        if len == 0 {
            return None;
        }
        match from.and_then(|i| self.order_pos(i)) {
            None => Some(self.order_at(0)),
            Some(p) if p + 1 < len => Some(self.order_at(p + 1)),
            Some(_) if self.repeat == Repeat::All => Some(self.order_at(0)),
            Some(_) => None,
        }
    }

    /// The entry before the one at `from` (Previous), as `after` goes.
    pub fn before(&self, from: Option<usize>) -> Option<usize> {
        let n = self.entries.len();
        if n == 0 {
            return None;
        }
        let len = self.order_len();
        if len == 0 {
            return None;
        }
        match from.and_then(|i| self.order_pos(i)) {
            Some(p) if p > 0 => Some(self.order_at(p - 1)),
            Some(_) if self.repeat == Repeat::All => Some(self.order_at(len - 1)),
            Some(p) => Some(self.order_at(p)),
            None => Some(self.order_at(0)),
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        self.reshuffle();
        self.revision += 1;
    }

    /// The order of play's length, its entry at place `p`, and the place
    /// of entry `i` in it; no list made (the taskbar and the media
    /// controls ask for the next and the previous every frame).
    fn order_len(&self) -> usize {
        if self.shuffle {
            self.order.len()
        } else {
            self.entries.len()
        }
    }

    fn order_at(&self, p: usize) -> usize {
        if self.shuffle { self.order[p] } else { p }
    }

    fn order_pos(&self, i: usize) -> Option<usize> {
        if self.shuffle {
            self.order.iter().position(|&o| o == i)
        } else {
            (i < self.entries.len()).then_some(i)
        }
    }

    /// The indices in the order of play.
    fn play_order(&self) -> Vec<usize> {
        if self.shuffle {
            self.order.clone()
        } else {
            (0..self.entries.len()).collect()
        }
    }

    /// The shuffled order as the entries' ids, to keep it through a change
    /// of the list ([`Queue::reorder`]).
    fn order_ids(&self) -> Vec<u64> {
        self.order
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| e.id)
            .collect()
    }

    /// The shuffled order of `ids` (from [`Queue::order_ids`]) kept after
    /// entries were removed or moved: the tracks heard stay heard, those to
    /// come keep their turn.
    fn reorder(&mut self, ids: &[u64]) {
        if !self.shuffle {
            self.order.clear();
            return;
        }
        let index: std::collections::HashMap<u64, usize> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id, i))
            .collect();
        self.order = ids.iter().filter_map(|id| index.get(id).copied()).collect();
    }

    /// A new order of play: the current entry first, the rest at random.
    fn reshuffle(&mut self) {
        if !self.shuffle {
            self.order.clear();
            return;
        }
        let mut rest: Vec<usize> = (0..self.entries.len())
            .filter(|&i| Some(i) != self.current)
            .collect();
        self.shuffle_slice(&mut rest);
        self.order = self.current.into_iter().chain(rest).collect();
    }

    fn shuffle_slice(&mut self, items: &mut [usize]) {
        if self.seed == 0 {
            self.seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64)
                | 1;
        }
        for i in (1..items.len()).rev() {
            // xorshift64*
            self.seed ^= self.seed >> 12;
            self.seed ^= self.seed << 25;
            self.seed ^= self.seed >> 27;
            let r = self.seed.wrapping_mul(0x2545_f491_4f6c_dd1d);
            items.swap(i, (r % (i as u64 + 1)) as usize);
        }
    }

    /// The list as an M3U8 playlist: `#EXTM3U`, then for each entry its
    /// `#EXTINF` line (seconds and "Artist - Title", from `describe`) and
    /// its path. UTF-8 without a BOM, CRLF, as players write it.
    pub fn m3u8(&self, describe: &dyn Fn(&Path) -> Option<(u64, String)>) -> String {
        let mut out = String::from("#EXTM3U\r\n");
        for e in &self.entries {
            if let Some((secs, title)) = describe(&e.path) {
                out += &format!("#EXTINF:{secs},{title}\r\n");
            }
            out += &format!("{}\r\n", e.path.display());
        }
        out
    }
}

/// The paths of an M3U or M3U8 playlist: comments skipped, relative paths
/// from the list's folder, `file:` addresses (VLC's, foobar2000's) as the
/// paths they name; other addresses (`http://`) are left out.
pub fn m3u_paths(text: &str, folder: &Path) -> Vec<PathBuf> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| match l.get(..7) {
            Some(scheme) if scheme.eq_ignore_ascii_case("file://") => file_url_path(&l[7..]),
            _ if l.contains("://") => None,
            _ => {
                let p = PathBuf::from(l);
                Some(if p.is_absolute() { p } else { folder.join(p) })
            }
        })
        .collect()
}

/// The path of a `file:` address after its `file://`: `/D:/Music/a%20b.mp3`
/// or a share's `server/folder/x.mp3`, percent-decoded as UTF-8.
fn file_url_path(rest: &str) -> Option<PathBuf> {
    let mut bytes = Vec::with_capacity(rest.len());
    let mut it = rest.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex = [it.next()?, it.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    let text = String::from_utf8_lossy(&bytes).replace('/', "\\");
    let path = match text.strip_prefix('\\') {
        // file:///D:/x: a drive.
        Some(local) => PathBuf::from(local),
        // file://server/share/x: a share.
        None => PathBuf::from(format!("\\\\{text}")),
    };
    path.is_absolute().then_some(path)
}

/// The text of a playlist file: UTF-8 (with or without a BOM), else the
/// ANSI code page of Windows, which older players wrote `.m3u` in.
pub fn m3u_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => crate::win::from_ansi(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(n: usize) -> Queue {
        let mut q = Queue::default();
        q.replace((0..n).map(|i| PathBuf::from(format!("{i}.mp3"))).collect());
        q
    }

    #[test]
    fn removing_the_current_entry_names_the_one_after_it() {
        let mut q = queue(10);
        q.set_current(Some(5));
        // The current one and one before it: the old 6 goes on, now 4.
        assert_eq!(q.remove(&[1, 5]), Some(4));
        assert_eq!(q.entries()[4].path, PathBuf::from("6.mp3"));
        assert_eq!(q.current(), None);
        // The current one kept: nothing to go on with.
        q.set_current(Some(2));
        assert_eq!(q.remove(&[0]), None);
        assert_eq!(q.current(), Some(1));
        // The last removed: none after it, or the first with repeat all.
        let mut q = queue(3);
        q.set_current(Some(2));
        assert_eq!(q.remove(&[2]), None);
        q.repeat = Repeat::All;
        q.set_current(Some(1));
        assert_eq!(q.remove(&[1]), Some(0));
    }

    #[test]
    fn the_shuffled_order_stays_through_removes_and_moves() {
        let mut q = queue(12);
        q.set_current(Some(0));
        q.set_shuffle(true);
        let names = |q: &Queue| -> Vec<PathBuf> {
            q.order
                .iter()
                .map(|&i| q.entries()[i].path.clone())
                .collect()
        };
        let before = names(&q);
        let gone = q.entries()[q.order[3]].path.clone();
        let third = q.order[3];
        q.remove(&[third]);
        let mut expected = before.clone();
        expected.retain(|p| *p != gone);
        assert_eq!(names(&q), expected);
        q.move_to(&[0, 1], 8);
        assert_eq!(names(&q), expected);
        assert_eq!(q.order.len(), q.len());
    }

    #[test]
    fn plays_in_order_and_repeats() {
        let mut q = queue(3);
        assert_eq!(q.after(None), Some(0));
        assert_eq!(q.after(Some(1)), Some(2));
        assert_eq!(q.after(Some(2)), None);
        assert_eq!(q.before(Some(0)), Some(0));
        q.repeat = Repeat::All;
        assert_eq!(q.after(Some(2)), Some(0));
        assert_eq!(q.before(Some(0)), Some(2));
        q.repeat = Repeat::One;
        assert_eq!(q.after(Some(1)), Some(1));
    }

    #[test]
    fn shuffle_plays_each_once() {
        let mut q = queue(20);
        q.set_current(Some(5));
        q.set_shuffle(true);
        let mut seen = vec![5];
        let mut at = Some(5);
        while let Some(next) = q.after(at) {
            seen.push(next);
            at = Some(next);
        }
        seen.sort_unstable();
        assert_eq!(seen, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn ids_follow_entries_through_changes() {
        let mut q = queue(4);
        q.set_current(Some(2));
        let id = q.current_entry().unwrap().id;
        q.remove(&[0]);
        assert_eq!(q.current(), Some(1));
        assert_eq!(q.current_entry().unwrap().id, id);
        let first = q.add(vec!["x.mp3".into(), "1.mp3".into()]);
        assert_eq!(first, 3);
        assert_eq!(q.len(), 5);
        // The same file twice: two entries with ids of their own.
        assert_ne!(q.entries()[0].id, q.entries()[4].id);
        // Moved to the front: "x.mp3" and the current one.
        let at = q.move_to(&[3, 1], 0);
        assert_eq!(at, 0);
        assert_eq!(q.entries()[0].path, PathBuf::from("2.mp3"));
        assert_eq!(q.entries()[1].path, PathBuf::from("x.mp3"));
        assert_eq!(q.current(), Some(0));
    }

    #[test]
    fn m3u8_round_trip() {
        let mut q = Queue::default();
        q.replace(vec![PathBuf::from(r"D:\Music\Ария\01 - Патриот.mp3")]);
        let text = q.m3u8(&|_| Some((301, "Ария - Патриот".into())));
        assert_eq!(
            text,
            "#EXTM3U\r\n#EXTINF:301,Ария - Патриот\r\nD:\\Music\\Ария\\01 - Патриот.mp3\r\n"
        );
        let paths = m3u_paths(&text, Path::new(r"C:\x"));
        assert_eq!(paths, [PathBuf::from(r"D:\Music\Ария\01 - Патриот.mp3")]);
        let relative = m3u_paths(
            "\u{feff}#EXTM3U\nsub\\a.mp3\nhttp://x/y.mp3\n",
            Path::new(r"C:\x"),
        );
        assert_eq!(relative, [PathBuf::from(r"C:\x\sub\a.mp3")]);
    }

    #[test]
    fn a_playlist_not_in_utf8_is_read_in_the_ansi_code_page() {
        assert_eq!(m3u_text("D:\\Ария.mp3".as_bytes()), "D:\\Ария.mp3");
        assert_eq!(m3u_text(b"a.mp3"), "a.mp3");
        // "Ария" in Windows-1251, where that is the code page.
        if unsafe { windows_sys::Win32::Globalization::GetACP() } == 1251 {
            assert_eq!(m3u_text(b"\xc0\xf0\xe8\xff.mp3"), "Ария.mp3");
        }
    }

    #[test]
    fn file_addresses_are_paths() {
        let paths = m3u_paths(
            "file:///D:/Music/%D0%90%D1%80%D0%B8%D1%8F/01%20-%20x.mp3\nFILE://nas/music/b.flac\nfile:///bad%2",
            Path::new(r"C:\x"),
        );
        assert_eq!(
            paths,
            [
                PathBuf::from(r"D:\Music\Ария\01 - x.mp3"),
                PathBuf::from(r"\\nas\music\b.flac"),
            ]
        );
    }
}
