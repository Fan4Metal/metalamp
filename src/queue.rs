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
    /// it is kept.
    pub fn remove(&mut self, indices: &[usize]) {
        let mut keep = vec![true; self.entries.len()];
        for &i in indices {
            if i < keep.len() {
                keep[i] = false;
            }
        }
        let current_id = self.current_entry().map(|e| e.id);
        let mut k = keep.iter();
        self.entries.retain(|_| *k.next().unwrap_or(&true));
        self.current = current_id.and_then(|id| self.index_of(id));
        self.reshuffle();
        self.revision += 1;
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
        self.reshuffle();
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
        let order: Vec<usize> = if self.shuffle {
            self.order.clone()
        } else {
            (0..n).collect()
        };
        let pos = from.and_then(|i| order.iter().position(|&o| o == i));
        match pos {
            None => order.first().copied(),
            Some(p) if p + 1 < order.len() => Some(order[p + 1]),
            Some(_) if self.repeat == Repeat::All => order.first().copied(),
            Some(_) => None,
        }
    }

    /// The entry before the one at `from` (Previous), as `after` goes.
    pub fn before(&self, from: Option<usize>) -> Option<usize> {
        let n = self.entries.len();
        if n == 0 {
            return None;
        }
        let order: Vec<usize> = if self.shuffle {
            self.order.clone()
        } else {
            (0..n).collect()
        };
        let pos = from.and_then(|i| order.iter().position(|&o| o == i));
        match pos {
            Some(p) if p > 0 => Some(order[p - 1]),
            Some(_) if self.repeat == Repeat::All => order.last().copied(),
            Some(p) => Some(order[p]),
            None => order.first().copied(),
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        self.reshuffle();
        self.revision += 1;
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
/// from the list's folder; addresses (`scheme://`) are left out.
pub fn m3u_paths(text: &str, folder: &Path) -> Vec<PathBuf> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| !l.contains("://"))
        .map(|l| {
            let p = PathBuf::from(l);
            if p.is_absolute() { p } else { folder.join(p) }
        })
        .collect()
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
}
