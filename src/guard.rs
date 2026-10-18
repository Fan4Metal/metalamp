//! Files read by other crates' parsers (lofty's tags, the JPEG and PNG
//! decoders) under `catch_unwind`: a panic in one on a damaged file is that
//! file left out, not the program ended, at every start again (the scan
//! meets the file each time). The release build unwinds for this; any
//! other panic still ends the program (`main::install_panic_hook`).

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

thread_local! {
    static GUARDED: Cell<bool> = const { Cell::new(false) };
}

/// Whether this thread is inside [`catch`] now: the panic hook only logs.
pub fn guarded() -> bool {
    GUARDED.with(Cell::get)
}

/// `read` of `path`; `None` if it panicked (logged).
pub fn catch<T>(path: &Path, read: impl FnOnce() -> T) -> Option<T> {
    let was = GUARDED.with(|g| g.replace(true));
    let result = catch_unwind(AssertUnwindSafe(read));
    GUARDED.with(|g| g.set(was));
    match result {
        Ok(value) => Some(value),
        Err(_) => {
            log::error!("{}: a panic while reading it; left out", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_inside_is_none_and_the_flag_goes_back() {
        let path = Path::new("x.mp3");
        assert_eq!(catch(path, || 5), Some(5));
        let none: Option<i32> = catch(path, || panic!("a damaged file"));
        assert_eq!(none, None);
        assert!(!guarded());
    }
}
