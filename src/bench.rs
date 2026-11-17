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
//! - `--bench enhancer <file> [from s] [seconds] [bass,clarity,width,warmth,harmonics,air]`:
//!   what each of the enhancer's stages does to a passage of real music,
//!   by third-octave bands (sines told too little: two faults were found
//!   only this way), and what the limiter then takes.

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
        _ => {
            let _ = writeln!(
                out,
                "usage: metalamp --bench scan <folder>... | gapless <file>... | loudness <file>... | enhancer <file> [from s] [seconds] [bass,clarity,width,warmth,harmonics,air]"
            );
            2
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
        crate::ui::spectrum::fft(&mut re, &mut im, &twiddles);
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

/// `--bench enhancer <file> [from s] [seconds] [bass,clarity,width,warmth,harmonics,air]`
/// (the amounts 0 to 10; the settings' when left out): the passage dry, by
/// third-octave bands in dBFS, and what each stage adds to each band in dB:
/// the bass's shelf alone and the whole bass, the air shelf alone and
/// the whole clarity, warmth, all six, and all six through the limiter.
fn bench_enhancer(args: &[String], out: &mut impl Write) {
    use crate::audio::decoder::{Decoder, SECOND};
    use crate::audio::dsp::biquad::{Biquad, Coeffs, db_to_gain, gain_to_db};
    use crate::audio::dsp::enhancer::{
        AIR_SHELF_DB, AIR_SHELF_HZ, Amounts, BASS_SHELF_DB, BASS_SHELF_HZ, Enhancer, LATENCY,
    };
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
            [
                d.bass,
                d.clarity,
                d.width,
                d.warmth,
                d.bass_harmonics,
                d.air,
            ]
            .map(f32::from)
            .to_vec()
        }
    };
    if !matches!(amounts.len(), 4 | 6)
        || amounts
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=10.0).contains(x))
    {
        let _ = writeln!(
            out,
            "the amounts: bass,clarity,width,warmth[,harmonics,air], 0 to 10 each"
        );
        return;
    }
    let a = Amounts {
        bass: amounts[0] / 10.0,
        clarity: amounts[1] / 10.0,
        width: amounts[2] / 10.0,
        warmth: amounts[3] / 10.0,
        bass_harmonics: amounts.get(4).copied().unwrap_or(0.0) / 10.0,
        air: amounts.get(5).copied().unwrap_or(0.0) / 10.0,
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
    if dry.len() < SPECTRUM_N * ch {
        let _ = writeln!(out, "too short a passage");
        return;
    }
    let enhanced = |a: Amounts| {
        let mut s = dry.clone();
        s.resize(s.len() + LATENCY * ch, 0.0);
        Enhancer::new(rate, ch, a).process(&mut s);
        // Analyse the same musical interval. Keeping the initial delay
        // and dropping the end can masquerade as a change in timbre.
        s.drain(..LATENCY * ch);
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
    let mut limited = all.clone();
    let mut limiter = Limiter::new(rate, ch, db_to_gain(CEILING_DB));
    let delay = limiter.latency_frames() * ch;
    limited.resize(limited.len() + delay, 0.0);
    limiter.process(&mut limited);
    limited.drain(..delay);
    // Compare timbre at matched processing levels. The real output's
    // reserved level and limiter loss are reported separately below.
    let matched = |a: Amounts| {
        let mut s = enhanced(a);
        let gain = db_to_gain(a.headroom_db(ch));
        for x in &mut s {
            *x *= gain;
        }
        s
    };
    let gain = db_to_gain(a.headroom_db(ch));
    let columns: [(&str, Vec<f32>); 9] = [
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
            "harm",
            matched(Amounts {
                bass_harmonics: a.bass_harmonics,
                ..none
            }),
        ),
        (
            "clarity",
            matched(Amounts {
                clarity: a.clarity,
                ..none
            }),
        ),
        (
            "a.shelf",
            shelved(Coeffs::high_shelf(rate, AIR_SHELF_HZ, AIR_SHELF_DB * a.air)),
        ),
        ("air", matched(Amounts { air: a.air, ..none })),
        (
            "warmth",
            matched(Amounts {
                warmth: a.warmth,
                ..none
            }),
        ),
        ("all", all.iter().map(|x| x * gain).collect()),
        ("limited", limited.iter().map(|x| x * gain).collect()),
    ];
    let _ = writeln!(
        out,
        "spectral deltas compensate processing reserve: {:.1} dB; harmonics {}, air {}",
        a.headroom_db(ch),
        a.bass_harmonics * 10.0,
        a.air * 10.0
    );
    let db = |p: f64| 10.0 * p.max(1e-20).log10();
    let base = third_octaves(&dry, rate, ch, false);
    let _ = writeln!(
        out,
        "{}: {from} s + {seconds} s, {rate} Hz, {ch} ch; bass {}, clarity {}, width {}, warmth {}",
        file.display(),
        amounts[0],
        amounts[1],
        amounts[2],
        amounts[3]
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
            "side: dry {:.1} dB, all {:+.1} dB (headroom compensated)",
            side(&dry),
            side(&all) - side(&dry) + f64::from(a.headroom_db(ch))
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
