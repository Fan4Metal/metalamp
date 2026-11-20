//! `--bench`: measurements printed to the console, for CLAUDE.md and for
//! checking a change before claiming it is faster (the first, cold run
//! is thrown away).
//!
//! - `--bench scan <folder>…`: a full scan without the index: the walk,
//!   the tags (on the scan's threads), the grouping, the index's size.
//! - `--bench gapless <file>…`: an MP3 decoded whole against its LAME tag.
//! - `--bench loudness <file>…`: each file's loudness (EBU R128), peak and
//!   the gain normalisation would give it, with the time the measuring
//!   took; `ffmpeg -af ebur128 -f null -` gives the figure to compare.
//! - `--bench render <file> <out.wav> [from s] [seconds] [gain dB] [raw]`:
//!   the passage through the chain with the settings' sound processing,
//!   written as a WAV to compare with a recording of another player.

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
        Some("gapless") if args.len() > 1 => {
            let files: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
            bench_gapless(&files, &mut out);
            0
        }
        Some("loudness") if args.len() > 1 => {
            let files: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
            bench_loudness(&files, &mut out);
            0
        }
        Some("render") if args.len() > 2 => {
            bench_render(&args[1..], &mut out);
            0
        }
        _ => {
            let _ = writeln!(
                out,
                "usage: metalamp --bench scan <folder>... | gapless <file>... | loudness <file>... | render <file> <out.wav> [from s] [seconds] [gain dB] [raw]"
            );
            2
        }
    }
}

/// `--bench render <file> <out.wav> [from s] [seconds] [gain dB] [raw]`:
/// the passage through the chain with the settings' sound processing, in
/// blocks of 1024 frames as the player would, written as a 32-bit float WAV
/// with the chain's delay taken out (for comparing with a recording of
/// another player); `gain` is applied first, as normalisation would; `raw`:
/// through the enhancer alone (the settings' controls), without the
/// limiter.
fn bench_render(args: &[String], out: &mut impl Write) {
    use crate::audio::decoder::{Decoder, SECOND};
    use crate::audio::dsp::Chain;
    use crate::audio::dsp::biquad::db_to_gain;
    use crate::audio::mf;
    let file = PathBuf::from(&args[0]);
    let target = PathBuf::from(&args[1]);
    let from: f64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let seconds: f64 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(30.0);
    let gain_db: f32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let _com = mf::com_init_mta();
    let _mf = match mf::Mf::start() {
        Ok(m) => m,
        Err(e) => {
            let _ = writeln!(out, "Media Foundation: {}", mf::describe(&e));
            return;
        }
    };
    let mut decoder = match Decoder::open(&file) {
        Ok(d) => d,
        Err(e) => {
            let _ = writeln!(out, "{}: {}", file.display(), mf::describe(&e));
            return;
        }
    };
    if from > 0.0 {
        let _ = decoder.seek((from * SECOND as f64) as i64);
    }
    let (rate, ch) = (decoder.rate, usize::from(decoder.channels));
    let want = (seconds * f64::from(rate)) as usize * ch;
    let mut wet = Vec::with_capacity(want);
    while wet.len() < want {
        match decoder.next() {
            Ok(Some(block)) => wet.extend(block),
            _ => break,
        }
    }
    wet.truncate(want);
    if gain_db != 0.0 {
        let gain = db_to_gain(gain_db);
        wet.iter_mut().for_each(|x| *x *= gain);
    }
    let raw = args.get(5).is_some_and(|a| a == "raw");
    let settings = crate::settings::Settings::load().dsp;
    let mut chain = Chain::new(rate, decoder.channels, &settings);
    let mut enhancer =
        crate::audio::dsp::enhancer::Enhancer::new(rate, ch, true, &settings.enhancement);
    let delay = if raw {
        enhancer.latency_frames()
    } else {
        chain.latency_frames()
    } * ch;
    wet.resize(wet.len() + delay, 0.0);
    let t = Instant::now();
    for block in wet.chunks_mut(1024 * ch) {
        if raw {
            enhancer.process(block);
        } else {
            chain.process(block, 1.0);
        }
    }
    let took = t.elapsed();
    wet.drain(..delay);
    let mut bytes = Vec::with_capacity(44 + wet.len() * 4);
    let data = (wet.len() * 4) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes());
    bytes.extend_from_slice(&(ch as u16).to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * ch as u32 * 4).to_le_bytes());
    bytes.extend_from_slice(&(ch as u16 * 4).to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for x in &wet {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    match std::fs::write(&target, bytes) {
        Ok(()) => {
            let _ = writeln!(
                out,
                "{}: {} s, {rate} Hz, {ch} ch, in {:.0} ms",
                target.display(),
                wet.len() / ch / rate as usize,
                took.as_secs_f64() * 1000.0
            );
        }
        Err(e) => {
            let _ = writeln!(out, "{}: {e}", target.display());
        }
    }
}

fn bench_scan(folders: &[PathBuf], out: &mut impl Write) {
    let cancel = AtomicBool::new(false);
    let t = Instant::now();
    let found = scan::walk(folders, &cancel).found;
    let walk = t.elapsed();
    let t = Instant::now();
    let (tracks, read) =
        scan::read_tracks(&found, &HashMap::new(), false, &cancel, &|_, _| {}, &|_| {});
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

/// `--bench loudness <file>…`: the loudness measured as the player does
/// it before a track's first play.
fn bench_loudness(files: &[PathBuf], out: &mut impl Write) {
    use crate::audio::{dsp, mf};
    let _com = mf::com_init_mta();
    let _mf = match mf::Mf::start() {
        Ok(m) => m,
        Err(e) => {
            let _ = writeln!(out, "Media Foundation: {}", mf::describe(&e));
            return;
        }
    };
    for file in files {
        let started = Instant::now();
        match crate::loudness::measure_file(file) {
            Some(m) => {
                let gain = dsp::biquad::gain_to_db(dsp::track_gain(m.lufs, m.peak, true));
                let _ = writeln!(
                    out,
                    "{}: {:.1} LUFS, peak {:.2} dBFS, gain {gain:+.1} dB, {} ms",
                    file.display(),
                    m.lufs,
                    dsp::biquad::gain_to_db(m.peak),
                    started.elapsed().as_millis()
                );
            }
            None => {
                let _ = writeln!(out, "{}: cannot measure", file.display());
            }
        }
    }
}

/// `--bench gapless <file>…`: what Media Foundation's decoder gives for an
/// MP3 against what its LAME tag says: the silent samples at the start
/// (exactly zero), the samples decoded and the frames counted.
fn bench_gapless(files: &[PathBuf], out: &mut impl Write) {
    use crate::audio::{decoder::Decoder, mf, mp3gap};
    let _com = mf::com_init_mta();
    let _mf = match mf::Mf::start() {
        Ok(m) => m,
        Err(e) => {
            let _ = writeln!(out, "Media Foundation: {}", mf::describe(&e));
            return;
        }
    };
    for file in files {
        let head = mp3gap::read_head(file);
        let info = mp3gap::parse(&head);
        let mut decoder = match Decoder::open(file) {
            Ok(d) => d,
            Err(e) => {
                let _ = writeln!(out, "{}: {}", file.display(), mf::describe(&e));
                continue;
            }
        };
        let channels = usize::from(decoder.channels.max(1));
        let (mut total, mut leading, mut sound) = (0u64, 0u64, false);
        let mut trailing = 0u64;
        while let Ok(Some(block)) = decoder.next() {
            for frame in block.chunks(channels) {
                let silent = frame.iter().all(|&s| s == 0.0);
                if !sound {
                    if silent {
                        leading += 1;
                    } else {
                        sound = true;
                    }
                }
                trailing = if silent { trailing + 1 } else { 0 };
                total += 1;
            }
        }
        let _ = writeln!(out, "{}", file.display());
        let _ = writeln!(
            out,
            "  decoded {total} samples, {leading} silent at the start, {trailing} at the end ({} Hz)",
            decoder.rate
        );
        match info {
            Some(i) => {
                let frames_total = i
                    .frames
                    .map(|f| u64::from(f) * u64::from(i.samples_per_frame));
                let _ = writeln!(
                    out,
                    "  tag: {:?} frames x {}, delay {}, padding {}; frames' samples {:?}, music {:?}",
                    i.frames,
                    i.samples_per_frame,
                    i.delay,
                    i.padding,
                    frames_total,
                    i.music_samples()
                );
                if let Some(f) = frames_total {
                    let _ = writeln!(
                        out,
                        "  decoded - frames' samples = {}",
                        total as i64 - f as i64
                    );
                }
            }
            None => {
                let _ = writeln!(out, "  no Xing/Info header with a LAME tag");
            }
        }
    }
}
