//! `--bench`: measurements printed to the console, for CLAUDE.md and for
//! checking a change before claiming it is faster (the first, cold run
//! is thrown away).
//!
//! - `--bench scan <folder>…`: a full scan without the index: the walk,
//!   the tags (on the scan's threads), the grouping, the index's size.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::library::{Library, scan};

pub fn run(args: &[String]) -> i32 {
    let mut out = std::io::stdout();
    match args.first().map(String::as_str) {
        Some("scan") if args.len() > 1 => {
            let folders: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
            bench_scan(&folders, &mut out);
            0
        }
        _ => {
            let _ = writeln!(out, "usage: metalamp --bench scan <folder>...");
            2
        }
    }
}

fn bench_scan(folders: &[PathBuf], out: &mut impl Write) {
    let cancel = AtomicBool::new(false);
    let t = Instant::now();
    let found = scan::walk(folders, &cancel);
    let walk = t.elapsed();
    let t = Instant::now();
    let (tracks, read) = scan::read_tracks(&found, &HashMap::new(), &cancel, &|_, _| {}, &|_| {});
    let tags = t.elapsed();
    let t = Instant::now();
    let library = Library::new(tracks);
    let group = t.elapsed();
    let t = Instant::now();
    let json = serde_json::to_vec(&library.tracks).map_or(0, |v| v.len());
    let serialize = t.elapsed();
    let t = Instant::now();
    let _again = Library::new(library.tracks.clone());
    let regroup = t.elapsed();

    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    let count =
        |f: &dyn Fn(&crate::library::Track) -> bool| library.tracks.iter().filter(|t| f(t)).count();
    let _ = writeln!(out, "files:      {} ({read} read)", found.len());
    let _ = writeln!(out, "walk:       {:.0} ms", ms(walk));
    let _ = writeln!(
        out,
        "tags:       {:.0} ms ({:.2} ms a file)",
        ms(tags),
        ms(tags) / read.max(1) as f64
    );
    let _ = writeln!(
        out,
        "group:      {:.1} ms (again {:.1} ms)",
        ms(group),
        ms(regroup)
    );
    let _ = writeln!(
        out,
        "index:      {} KB, serialised in {:.1} ms",
        json / 1024,
        ms(serialize)
    );
    let _ = writeln!(
        out,
        "library:    {} artists, {} albums, {} tracks, {} s, {} MB",
        library.artists.len(),
        library.albums.len(),
        library.totals.tracks,
        library.totals.length_ms / 1000,
        library.totals.bytes / (1 << 20)
    );
    let _ = writeln!(
        out,
        "no length:  {}; no year: {}; no album artist: {}; no cover in the tag: {}",
        count(&|t| t.duration_ms == 0),
        count(&|t| t.year == 0),
        count(&|t| t.album_artist.is_empty()),
        count(&|t| !t.has_cover)
    );
    // Folders whose tracks went into more than one album (a year or an
    // album artist differing between tracks).
    let mut by_folder: HashMap<&std::path::Path, Vec<&crate::library::Album>> = HashMap::new();
    for album in &library.albums {
        by_folder
            .entry(album.folder.as_path())
            .or_default()
            .push(album);
    }
    let mut split: Vec<_> = by_folder.into_iter().filter(|(_, a)| a.len() > 1).collect();
    split.sort_by(|a, b| a.0.cmp(b.0));
    let _ = writeln!(
        out,
        "split:      {} folders hold more than one album",
        split.len()
    );
    for (folder, albums) in split.iter().take(12) {
        let names: Vec<String> = albums
            .iter()
            .map(|a| {
                format!(
                    "{} / {} / {} ({})",
                    a.artist,
                    a.title,
                    a.year,
                    a.tracks.len()
                )
            })
            .collect();
        let _ = writeln!(out, "  {}: {}", folder.display(), names.join(" | "));
    }
}
