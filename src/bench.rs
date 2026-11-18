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
//! - `--bench enhancer <file> [from s] [seconds] [bass,width,brilliance,adaptive_bass] [gain dB]`:
//!   what each of the enhancer's stages does to a passage of real music,
//!   by third-octave bands (sines told too little: two faults were found
//!   only this way), and what the limiter then takes.
//! - `--bench drive <file> [from s] [seconds] [amount] [gain dB]`: the
//!   drive (and the limiter after it) on a passage: the loudness, peaks
//!   and crest factor before and after, and the gain it gave.
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
        Some("enhancer") if args.len() > 1 => {
            bench_enhancer(&args[1..], &mut out);
            0
        }
        Some("drive") if args.len() > 1 => {
            bench_drive(&args[1..], &mut out);
            0
        }
        Some("render") if args.len() > 2 => {
            bench_render(&args[1..], &mut out);
            0
        }
        _ => {
            let _ = writeln!(
                out,
                "usage: metalamp --bench scan <folder>... | gapless <file>... | loudness <file>... | enhancer <file> [from s] [seconds] [bass,width,brilliance,adaptive_bass] [gain dB] | drive <file> [from s] [seconds] [amount] [gain dB] | render <file> <out.wav> [from s] [seconds] [gain dB] [raw]"
            );
            2
        }
    }
}

/// `--bench drive <file> [from s] [seconds] [amount 0-10] [gain dB]`: the
/// passage through a chain with the enhancer on, its amounts at 0 and the
/// drive at `amount` (the settings' when left out), in blocks of 1024
/// frames as the player would; `gain` is applied first, as normalisation
/// would (`--bench loudness` tells a file's).
fn bench_drive(args: &[String], out: &mut impl Write) {
    use crate::audio::decoder::{Decoder, SECOND};
    use crate::audio::dsp::biquad::{db_to_gain, gain_to_db};
    use crate::audio::dsp::loudness::Meter;
    use crate::audio::dsp::{Chain, DspSettings};
    use crate::audio::mf;
    let file = PathBuf::from(&args[0]);
    let from: f64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let seconds: f64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(30.0);
    let amount: u8 = match args.get(3) {
        Some(v) => match v.parse::<u8>() {
            Ok(a) if a <= 10 => a,
            _ => {
                let _ = writeln!(out, "the amount: 0 to 10");
                return;
            }
        },
        None => crate::settings::Settings::load().dsp.drive,
    };
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
    let _ = decoder.seek((from * SECOND as f64) as i64);
    let (rate, ch) = (decoder.rate, usize::from(decoder.channels));
    let want = (seconds * f64::from(rate)) as usize * ch;
    let mut dry = Vec::with_capacity(want);
    while dry.len() < want {
        match decoder.next() {
            Ok(Some(block)) => dry.extend(block),
            _ => break,
        }
    }
    dry.truncate(want);
    if gain_db != 0.0 {
        let gain = db_to_gain(gain_db);
        dry.iter_mut().for_each(|x| *x *= gain);
    }
    let settings = DspSettings {
        enhancer: true,
        drive: amount,
        limiter: true,
        ..Default::default()
    };
    let mut chain = Chain::new(rate, decoder.channels, &settings);
    let delay = chain.latency_frames() * ch;
    let mut wet = dry.clone();
    wet.resize(wet.len() + delay, 0.0);
    let (mut levels, mut gains) = (Vec::new(), Vec::new());
    let t = Instant::now();
    for block in wet.chunks_mut(1024 * ch) {
        chain.process(block, 1.0);
        let (level, gain) = chain.drive_state();
        levels.push(level);
        gains.push(gain);
    }
    let took = t.elapsed();
    wet.drain(..delay);
    let measure = |s: &[f32]| {
        let mut m = Meter::new(rate, ch);
        m.add(s);
        (
            m.integrated().unwrap_or(-100.0),
            gain_to_db(m.peak().max(1e-9)),
        )
    };
    let (dry_lufs, dry_peak) = measure(&dry);
    let (wet_lufs, wet_peak) = measure(&wet);
    let percentile = |v: &[f32], p: f32| {
        let mut v = v.to_vec();
        v.sort_by(f32::total_cmp);
        v.get(((v.len() as f32 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    let _ = writeln!(
        out,
        "{}: {from} s + {seconds} s, {rate} Hz, {ch} ch; drive {amount}, gain first {gain_db:+.1} dB",
        file.display()
    );
    let _ = writeln!(
        out,
        "dry: {dry_lufs:.1} LUFS, peak {dry_peak:+.1} dBFS, peak over loudness {:.1} dB",
        dry_peak - dry_lufs
    );
    let _ = writeln!(
        out,
        "out: {wet_lufs:.1} LUFS ({:+.1} LU), peak {wet_peak:+.1} dBFS, peak over loudness {:.1} dB",
        wet_lufs - dry_lufs,
        wet_peak - wet_lufs
    );
    let _ = writeln!(
        out,
        "level followed: median {:.1} LUFS, 10 % {:.1}, 90 % {:.1}; gain: median {:.1} dB, 10 % {:.1}, 90 % {:.1}, most {:.1}",
        percentile(&levels, 0.5),
        percentile(&levels, 0.1),
        percentile(&levels, 0.9),
        percentile(&gains, 0.5),
        percentile(&gains, 0.1),
        percentile(&gains, 0.9),
        percentile(&gains, 1.0)
    );
    let _ = writeln!(
        out,
        "the chain took {:.2} % of real time",
        100.0 * took.as_secs_f64() / (dry.len() / ch) as f64 * f64::from(rate)
    );
}

/// `--bench render <file> <out.wav> [from s] [seconds] [gain dB] [raw]`:
/// the passage through the chain with the settings' sound processing, in
/// blocks of 1024 frames as the player would, written as a 32-bit float WAV
/// with the chain's delay taken out (for comparing with a recording of
/// another player); `gain` is applied first, as normalisation would; `raw`:
/// through the measured PowerFX profile alone, without the limiter.
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
    let mut powerfx = crate::audio::dsp::powerfx::PowerFx::new(rate, ch, true);
    let delay = if raw {
        powerfx.latency_frames()
    } else {
        chain.latency_frames()
    } * ch;
    wet.resize(wet.len() + delay, 0.0);
    let t = Instant::now();
    for block in wet.chunks_mut(1024 * ch) {
        if raw {
            powerfx.process(block);
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

/// The points of `third_octaves`' transform: bins of 2.7 Hz at 44.1 kHz.
const SPECTRUM_N: usize = 16384;

/// The power in each third-octave band from 31.5 Hz to 16 kHz of the mean
/// of interleaved `s`'s channels (or of the first less the second, `side`):
/// Hann windows of `SPECTRUM_N` frames, half overlapping, averaged. A
/// full-scale sine reads about 0 dB.
fn third_octaves(s: &[f32], rate: u32, ch: usize, side: bool) -> Vec<(f32, f64)> {
    use std::f32::consts::PI;
    let n = SPECTRUM_N;
    let mono: Vec<f32> = s
        .chunks_exact(ch)
        .map(|f| {
            if side {
                (f[0] - f[ch.min(2) - 1]) / 2.0
            } else {
                f.iter().sum::<f32>() / ch as f32
            }
        })
        .collect();
    let twiddles: Vec<(f32, f32)> = (0..n / 2)
        .map(|k| {
            let a = -2.0 * PI * k as f32 / n as f32;
            (a.cos(), a.sin())
        })
        .collect();
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect();
    let mut power = vec![0f64; n / 2];
    let mut frames = 0;
    let mut at = 0;
    while at + n <= mono.len() {
        let mut re: Vec<f32> = mono[at..at + n]
            .iter()
            .zip(&window)
            .map(|(x, w)| x * w)
            .collect();
        let mut im = vec![0f32; n];
        crate::audio::dsp::fft::fft(&mut re, &mut im, &twiddles);
        for (p, (r, i)) in power.iter_mut().zip(re.iter().zip(&im)) {
            *p += f64::from(r * r + i * i);
        }
        frames += 1;
        at += n / 2;
    }
    // A sine of amplitude 1 has n/4 in its bin (the window's mean is a
    // half) and a little in the two beside it (1.5 times in all).
    let scale = 1.0 / (frames.max(1) as f64 * (n as f64 / 4.0).powi(2) * 1.5);
    let hz = f64::from(rate) / n as f64;
    (0..28)
        .map(|k| {
            let centre = 31.5 * 2f64.powf(f64::from(k) / 3.0);
            let (lo, hi) = (centre / 2f64.powf(1.0 / 6.0), centre * 2f64.powf(1.0 / 6.0));
            let sum: f64 = power
                .iter()
                .enumerate()
                .filter(|(b, _)| (lo..hi).contains(&(*b as f64 * hz)))
                .map(|(_, p)| p)
                .sum();
            (centre as f32, sum * scale)
        })
        .collect()
}

/// `--bench enhancer <file> [from s] [seconds] [bass,width,brilliance,adaptive_bass] [gain dB]`
/// (the amounts 0 to 10, those left out 0; the settings' when the list is
/// left out): the passage dry, by third-octave bands in dBFS, and what each
/// stage adds to each band in dB: the bass's shelf alone and the whole
/// bass, the adaptive bass, brilliance, all of them, and all of them
/// through the limiter; the reserve taken, the masking cut and
/// brilliance's raise. `gain` (dB, 0 when left out) is applied to the
/// passage first, as loudness normalisation would (`--bench loudness`
/// tells a file's).
fn bench_enhancer(args: &[String], out: &mut impl Write) {
    use crate::audio::decoder::{Decoder, SECOND};
    use crate::audio::dsp::biquad::{Biquad, Coeffs, db_to_gain, gain_to_db};
    use crate::audio::dsp::enhancer::{Amounts, BASS_SHELF_DB, BASS_SHELF_HZ, Enhancer};
    use crate::audio::dsp::{CEILING_DB, limiter::Limiter};
    use crate::audio::mf;
    let file = PathBuf::from(&args[0]);
    let from: f64 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let seconds: f64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(15.0);
    let amounts: Vec<f32> = match args.get(3) {
        Some(list) => list
            .split(',')
            .map(|v| v.parse::<f32>().unwrap_or(f32::NAN))
            .collect(),
        None => {
            let d = crate::settings::Settings::load().dsp;
            [d.bass, d.width, d.brilliance, d.adaptive_bass]
                .map(f32::from)
                .to_vec()
        }
    };
    if !(1..=4).contains(&amounts.len())
        || amounts
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=10.0).contains(x))
    {
        let _ = writeln!(
            out,
            "the amounts: bass[,width[,brilliance[,adaptive_bass]]], 0 to 10 each"
        );
        return;
    }
    let amount = |i: usize| amounts.get(i).copied().unwrap_or(0.0);
    let a = Amounts {
        bass: amount(0) / 10.0,
        width: amount(1) / 10.0,
        brilliance: amount(2) / 10.0,
        adaptive_bass: amount(3) / 10.0,
    };
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
    let _ = decoder.seek((from * SECOND as f64) as i64);
    let (rate, ch) = (decoder.rate, usize::from(decoder.channels));
    let want = (seconds * f64::from(rate)) as usize * ch;
    let mut dry = Vec::with_capacity(want);
    while dry.len() < want {
        match decoder.next() {
            Ok(Some(block)) => dry.extend(block),
            _ => break,
        }
    }
    dry.truncate(want);
    let gain_db: f32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    if gain_db != 0.0 {
        let gain = db_to_gain(gain_db);
        dry.iter_mut().for_each(|x| *x *= gain);
        let _ = writeln!(out, "the passage turned by {gain_db:+.1} dB first");
    }
    if dry.len() < SPECTRUM_N * ch {
        let _ = writeln!(out, "too short a passage");
        return;
    }
    let enhanced = |a: Amounts| {
        let mut s = dry.clone();
        Enhancer::new(rate, ch, a).process(&mut s);
        s
    };
    let shelved = |c: Coeffs| {
        let mut f = vec![Biquad::new(c); ch];
        dry.iter()
            .enumerate()
            .map(|(i, &x)| f[i % ch].run(x))
            .collect::<Vec<f32>>()
    };
    let none = Amounts::default();
    let processing = Instant::now();
    let all = enhanced(a);
    let processing = processing.elapsed();
    // The same again in blocks of 1024 frames, noting after each the
    // reserve and the masking cut (its band over the middle and the cut),
    // as the player would see them.
    let mut reserves = Vec::new();
    let mut given = Vec::new();
    let mut bass_shelf = Vec::new();
    let mut masking: (Vec<f32>, Vec<f32>) = Default::default();
    let mut bright: (Vec<f32>, Vec<f32>) = Default::default();
    {
        let mut s = dry.clone();
        let mut e = Enhancer::new(rate, ch, a);
        for block in s.chunks_mut(1024 * ch) {
            e.process(block);
            reserves.push(e.reserve_db());
            given.push(e.bass_given());
            bass_shelf.push(e.bass_shelf_db());
            let (excess, cut) = e.masking();
            masking.0.push(excess);
            masking.1.push(cut);
            let (excess, raise) = e.brilliance();
            bright.0.push(excess);
            bright.1.push(raise);
        }
    }
    let mut limited = all.clone();
    let mut limiter = Limiter::new(rate, ch, db_to_gain(CEILING_DB));
    let delay = limiter.latency_frames() * ch;
    limited.resize(limited.len() + delay, 0.0);
    limiter.process(&mut limited);
    limited.drain(..delay);
    // What is heard: the deltas include the reserve the boosts take.
    let matched = enhanced;
    let columns: [(&str, Vec<f32>); 6] = [
        (
            "b.shelf",
            shelved(Coeffs::low_shelf(
                rate,
                BASS_SHELF_HZ,
                BASS_SHELF_DB * a.bass,
            )),
        ),
        (
            "bass",
            matched(Amounts {
                bass: a.bass,
                ..none
            }),
        ),
        (
            "adapt",
            matched(Amounts {
                adaptive_bass: a.adaptive_bass,
                ..none
            }),
        ),
        (
            "bright",
            matched(Amounts {
                brilliance: a.brilliance,
                ..none
            }),
        ),
        ("all", all.clone()),
        ("limited", limited.clone()),
    ];
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let most = |v: &[f32]| v.iter().fold(0f32, |m, &x| m.max(x));
    let least = |v: &[f32]| v.iter().fold(0f32, |m, &x| m.min(x));
    let share =
        |v: &[f32]| 100.0 * v.iter().filter(|&&x| x < -0.05).count() as f32 / v.len().max(1) as f32;
    let percentile = |v: &[f32], p: f32| {
        let mut v = v.to_vec();
        v.sort_by(f32::total_cmp);
        v.get(((v.len() as f32 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    let _ = writeln!(
        out,
        "reserve (all, the deltas include it): mean {:.2} dB, most {:.2} dB of at most {:.1}",
        mean(&reserves),
        most(&reserves),
        a.most_reserve_db(ch),
    );
    let _ = writeln!(
        out,
        "bass shelf with adaptive bass {}: median {:.2} dB, 10 % {:.2} dB, 90 % {:.2} dB",
        a.adaptive_bass * 10.0,
        percentile(&bass_shelf, 0.5),
        percentile(&bass_shelf, 0.1),
        percentile(&bass_shelf, 0.9),
    );
    let _ = writeln!(
        out,
        "bass boost let through by its guard: mean {:.0} %, under 90 % for {:.0} % of the time, least {:.0} %",
        100.0 * mean(&given),
        100.0 * given.iter().filter(|&&g| g < 0.9).count() as f32 / given.len().max(1) as f32,
        100.0 * given.iter().fold(1f32, |m, &g| m.min(g))
    );
    let (excess, cut) = &masking;
    let _ = writeln!(
        out,
        "250 Hz over the middle: median {:+.1} dB, 90 % {:+.1} dB, most {:+.1} dB; cut {:.0} % of the time, mean {:.2} dB, deepest {:.2} dB",
        percentile(excess, 0.5),
        percentile(excess, 0.9),
        most(excess),
        share(cut),
        mean(cut),
        least(cut)
    );
    let _ = writeln!(
        out,
        "treble (9 kHz) over the middle: median {:+.1} dB, 10 % {:+.1} dB, 90 % {:+.1} dB; brilliance {}: raised by a mean {:.2} dB, median {:.2} dB, most {:.2} dB",
        percentile(&bright.0, 0.5),
        percentile(&bright.0, 0.1),
        percentile(&bright.0, 0.9),
        a.brilliance * 10.0,
        mean(&bright.1),
        percentile(&bright.1, 0.5),
        most(&bright.1)
    );
    let db = |p: f64| 10.0 * p.max(1e-20).log10();
    let base = third_octaves(&dry, rate, ch, false);
    let _ = writeln!(
        out,
        "{}: {from} s + {seconds} s, {rate} Hz, {ch} ch; bass {}, width {}, brilliance {}, adaptive bass {}",
        file.display(),
        amount(0),
        amount(1),
        amount(2),
        amount(3)
    );
    let _ = write!(out, "{:>7} {:>7}", "Hz", "dry dB");
    for (name, _) in &columns {
        let _ = write!(out, " {name:>7}");
    }
    let _ = writeln!(out);
    let spectra: Vec<Vec<(f32, f64)>> = columns
        .iter()
        .map(|(_, s)| third_octaves(s, rate, ch, false))
        .collect();
    for (i, (centre, p)) in base.iter().enumerate() {
        if f64::from(*centre) > f64::from(rate) * 0.45 {
            break;
        }
        let _ = write!(out, "{centre:>7.0} {:>7.1}", db(*p));
        for s in &spectra {
            let _ = write!(out, " {:>+7.1}", db(s[i].1) - db(*p));
        }
        let _ = writeln!(out);
    }
    let rms = |s: &[f32]| {
        gain_to_db(
            (s.iter().map(|&x| f64::from(x * x)).sum::<f64>() / s.len() as f64).sqrt() as f32,
        )
    };
    let peak = |s: &[f32]| gain_to_db(s.iter().fold(0f32, |m, &x| m.max(x.abs())));
    let ceiling = db_to_gain(CEILING_DB);
    let over = |s: &[f32]| {
        100.0
            * s.chunks_exact(ch)
                .filter(|f| f.iter().any(|x| x.abs() > ceiling))
                .count() as f64
            / (s.len() / ch) as f64
    };
    if ch >= 2 {
        let side = |s: &[f32]| db(third_octaves(s, rate, ch, true).iter().map(|b| b.1).sum());
        let _ = writeln!(
            out,
            "side: dry {:.1} dB, all {:+.1} dB",
            side(&dry),
            side(&all) - side(&dry)
        );
        let width = matched(Amounts {
            width: a.width,
            ..none
        });
        let before = third_octaves(&dry, rate, ch, true);
        let after = third_octaves(&width, rate, ch, true);
        let _ = write!(out, "width-only side deltas:");
        for target in [63.0f32, 126.0, 252.0, 1008.0, 4032.0] {
            if let Some(i) = before.iter().position(|(hz, _)| (*hz - target).abs() < 2.0) {
                let _ = write!(
                    out,
                    " {:.0} Hz {:+.2} dB;",
                    before[i].0,
                    db(after[i].1) - db(before[i].1)
                );
            }
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(
        out,
        "dry: RMS {:.1} dB, peak {:.1} dB, {:.2} % of the frames over the ceiling ({CEILING_DB} dB)",
        rms(&dry),
        peak(&dry),
        over(&dry)
    );
    let _ = writeln!(
        out,
        "limiter current gain reduction: {:.2} dB",
        limiter.reduction_db()
    );
    let _ = writeln!(
        out,
        "all: RMS {:+.1} dB, peak {:.1} dB, {:.2} % over the ceiling; the limiter takes {:.1} dB of the RMS",
        rms(&all) - rms(&dry),
        peak(&all),
        over(&all),
        rms(&all) - rms(&limited)
    );
    let _ = writeln!(
        out,
        "crest factor: dry {:.1} dB, enhanced {:.1} dB, limited {:.1} dB; enhancer CPU {:.2}% of real time",
        peak(&dry) - rms(&dry),
        peak(&all) - rms(&all),
        peak(&limited) - rms(&limited),
        100.0 * processing.as_secs_f64() / (dry.len() as f64 / ch as f64 / f64::from(rate))
    );
}
