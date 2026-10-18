//! The files kept next to the settings (the settings, the library index,
//! the playlist, the loudness measurements), written whole: to a temporary
//! file renamed over the old one, one writer at a time for each, and never
//! an older state over a newer one, whichever thread finishes first.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The writes of one file: numbered when asked for, so that one asked for
/// later wins even if it is written first.
pub struct Kept {
    asked: AtomicU64,
    written: Mutex<u64>,
}

impl Kept {
    pub const fn new() -> Kept {
        Kept {
            asked: AtomicU64::new(0),
            written: Mutex::new(0),
        }
    }

    /// The number of a write asked for now.
    pub fn ask(&self) -> u64 {
        self.asked.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Writes `bytes` to `path` (through `path` + `.tmp`) unless a write
    /// asked for after `number` was written already; several writes of
    /// one number (a long scan saving as it goes) all go through.
    pub fn write(&self, number: u64, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if *written > number {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        *written = number;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_write_never_lands_over_a_newer_one() {
        let dir = std::env::temp_dir().join(format!("metalamp_kept_{}", std::process::id()));
        let file = dir.join("a.json");
        let kept = Kept::new();
        let (first, second) = (kept.ask(), kept.ask());
        kept.write(second, &file, b"new").unwrap();
        kept.write(first, &file, b"old").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new");
        kept.write(second, &file, b"newer, same number").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"newer, same number");
        assert!(!dir.join("a.json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
