//! The library's statistics, for the statistics window (`ui/statistics.rs`):
//! what it holds (tracks, albums, artists, genres, length, size), in what
//! quality (bit rates, formats, sample rates), from when (albums by year,
//! the tracks added by month), how it is tagged, and how it is listened to
//! (plays, ratings, the last plays). Worked out in one pass over the tracks
//! when the library or the plays change; nothing here depends on the
//! language, the window names things as it draws them.

use std::collections::HashMap;

use super::{AlbumId, Library, TrackId};
use crate::stats::Stats;

/// The bit rate classes' lower bounds in kbps, lossy; then the lossless
/// files (`BITRATE_LOSSLESS`).
pub const BITRATES: [u32; 7] = [0, 128, 160, 192, 224, 256, 320];
/// The bit rate class of the lossless files (FLAC, WAV, and a bit rate a
/// lossy codec does not reach: ALAC, WMA Lossless).
pub const BITRATE_LOSSLESS: usize = BITRATES.len();
/// A bit rate past this is not of a lossy codec.
const LOSSY_MAX: u32 = 500;

/// The track lengths' classes' lower bounds in minutes.
pub const LENGTHS: [u32; 9] = [0, 2, 3, 4, 5, 6, 8, 10, 20];

/// How long ago the last play was: this week, month, year, before, never.
pub const RECENCY: usize = 5;
const WEEK: u64 = 7 * 86_400;
const MONTH: u64 = 30 * 86_400;
const YEAR: u64 = 365 * 86_400;

/// The longest lists kept (top artists, tracks played most).
const TOP: usize = 10;

/// An artist among the top ones.
#[derive(Clone, Debug, PartialEq)]
pub struct TopArtist {
    pub name: String,
    pub albums: u32,
    pub tracks: u32,
    pub plays: u64,
    pub length_ms: u64,
    /// Its first album, to go to.
    pub album: AlbumId,
}

/// A format (the file's extension in capitals) with its tracks and bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Format {
    pub name: String,
    pub tracks: u64,
    pub bytes: u64,
}

/// How many tracks have each of the tags that matter to a library.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tagged {
    pub year: u64,
    pub genre: u64,
    pub album_artist: u64,
    pub number: u64,
    pub cover: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
    pub length_ms: u64,
    pub bytes: u64,
    /// The plays counted of the library's tracks, the tracks played at
    /// least once, the time heard (each play the track's length), the
    /// tracks rated.
    pub plays: u64,
    pub played: u64,
    pub heard_ms: u64,
    pub rated: u64,
    /// Tracks by bit rate class (`BITRATES`, then the lossless ones); the
    /// tracks whose bit rate is not known are left out (`no_bitrate`).
    pub bitrates: [u64; BITRATES.len() + 1],
    pub no_bitrate: u64,
    /// The formats, the most tracks first.
    pub formats: Vec<Format>,
    /// Every genre (as most of its tracks spell it) with its tracks, the
    /// most first; the tracks without one apart.
    pub genres: Vec<(String, u64)>,
    pub no_genre: u64,
    /// Albums by year, each year from the first to the last (0 albums
    /// where none); the albums without a year apart.
    pub years: Vec<(u16, u64)>,
    pub no_year: u64,
    /// Tracks added each month (`year * 12 + month - 1`), each month from
    /// the first to the last.
    pub added: Vec<(i32, u64)>,
    /// Tracks by length class (`LENGTHS`).
    pub lengths: [u64; LENGTHS.len()],
    /// The artists with the most tracks, and those played most (played
    /// at all).
    pub top_artists: Vec<TopArtist>,
    pub played_artists: Vec<TopArtist>,
    /// The tracks played most, with their plays.
    pub top_tracks: Vec<(TrackId, u32)>,
    /// Tracks by rating, 0 (none) to 5.
    pub ratings: [u64; 6],
    /// Tracks by their last play (`RECENCY`).
    pub recency: [u64; RECENCY],
    pub tagged: Tagged,
    /// Sample rates in Hz with their tracks, the most first.
    pub sample_rates: Vec<(u32, u64)>,
    /// Mono, stereo and more channels.
    pub channels: [u64; 3],
}

/// The bit rate class of a track of `kbps` in a file with `extension`.
pub fn bitrate_class(kbps: u32, extension: &str) -> usize {
    if matches!(extension, "flac" | "wav") || kbps >= LOSSY_MAX {
        return BITRATE_LOSSLESS;
    }
    BITRATES.iter().rposition(|&from| kbps >= from).unwrap_or(0)
}

/// The length class of a track of `ms`.
pub fn length_class(ms: u32) -> usize {
    let minutes = ms / 60_000;
    LENGTHS
        .iter()
        .rposition(|&from| minutes >= from)
        .unwrap_or(0)
}

/// The class of a last play at `last` (0: never) at `now`.
pub fn recency_class(last: u64, now: u64) -> usize {
    if last == 0 {
        return 4;
    }
    let ago = now.saturating_sub(last);
    if ago < WEEK {
        0
    } else if ago < MONTH {
        1
    } else if ago < YEAR {
        2
    } else {
        3
    }
}

/// The month of a time (Unix seconds, UTC) as `year * 12 + month - 1`.
pub fn month_of(secs: u64) -> i32 {
    let (y, m, _) = civil((secs / 86_400) as i64);
    y * 12 + m as i32 - 1
}

/// The date of a day since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + i64::from(m <= 2)) as i32;
    (y, m, d)
}

/// Years outside these are a tag's mistake, not an album's year.
const FIRST_YEAR: u16 = 1900;
/// The most years the albums' chart spans (the latest ones).
const MOST_YEARS: u16 = 120;

impl Summary {
    /// The statistics of `library` with the plays and ratings of `stats`,
    /// at `now` (Unix seconds).
    pub fn of(library: &Library, stats: &Stats, now: u64) -> Summary {
        let mut s = Summary {
            tracks: library.totals.tracks,
            albums: library.albums.len() as u64,
            artists: library.artists.len() as u64,
            length_ms: library.totals.length_ms,
            bytes: library.totals.bytes,
            ..Summary::default()
        };
        let mut formats: HashMap<String, Format> = HashMap::new();
        // Lower-case genre: its tracks and each spelling's.
        let mut genres: HashMap<String, (u64, HashMap<String, u64>)> = HashMap::new();
        let mut added: HashMap<i32, u64> = HashMap::new();
        let mut rates: HashMap<u32, u64> = HashMap::new();
        let mut plays_of: Vec<u32> = Vec::with_capacity(library.tracks.len());
        for t in &library.tracks {
            let stat = stats.get(&t.path);
            plays_of.push(stat.plays);
            s.plays += u64::from(stat.plays);
            s.heard_ms += u64::from(stat.plays) * u64::from(t.duration_ms);
            if stat.plays > 0 {
                s.played += 1;
            }
            if stat.rating > 0 {
                s.rated += 1;
            }
            s.ratings[usize::from(stat.rating.min(5))] += 1;
            s.recency[recency_class(if stat.plays > 0 { stat.last } else { 0 }, now)] += 1;

            let extension = t
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if t.bitrate == 0 && !matches!(extension.as_str(), "flac" | "wav") {
                s.no_bitrate += 1;
            } else {
                s.bitrates[bitrate_class(t.bitrate, &extension)] += 1;
            }
            let name = if extension.is_empty() {
                "?".to_owned()
            } else {
                extension.to_uppercase()
            };
            let f = formats.entry(name.clone()).or_insert(Format {
                name,
                tracks: 0,
                bytes: 0,
            });
            f.tracks += 1;
            f.bytes += t.size;

            let mut any = false;
            for genre in t.genre.split([';', '\0']) {
                let genre = genre.trim();
                if genre.is_empty() {
                    continue;
                }
                any = true;
                let g = genres.entry(genre.to_lowercase()).or_default();
                g.0 += 1;
                *g.1.entry(genre.to_owned()).or_default() += 1;
            }
            if !any {
                s.no_genre += 1;
            }

            let came = t.added_or_mtime();
            if came > 0 {
                *added.entry(month_of(came)).or_default() += 1;
            }
            s.lengths[length_class(t.duration_ms)] += 1;
            if t.sample_rate > 0 {
                *rates.entry(t.sample_rate).or_default() += 1;
            }
            match t.channels {
                0 => {}
                1 => s.channels[0] += 1,
                2 => s.channels[1] += 1,
                _ => s.channels[2] += 1,
            }
            let tagged = &mut s.tagged;
            tagged.year += u64::from(t.year > 0);
            tagged.genre += u64::from(any);
            tagged.album_artist += u64::from(!t.album_artist.trim().is_empty());
            tagged.number += u64::from(t.track > 0);
            tagged.cover += u64::from(t.has_cover);
        }

        s.formats = formats.into_values().collect();
        s.formats
            .sort_by(|a, b| b.tracks.cmp(&a.tracks).then_with(|| a.name.cmp(&b.name)));
        s.genres = genres
            .into_values()
            .map(|(n, spellings)| {
                let name = spellings
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                    .map(|(name, _)| name)
                    .unwrap_or_default();
                (name, n)
            })
            .collect();
        s.genres.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| crate::format::natural_cmp(&a.0, &b.0))
        });
        s.sample_rates = rates.into_iter().collect();
        s.sample_rates
            .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        // Each month from the first to the last, none left out.
        if let (Some(&first), Some(&last)) = (added.keys().min(), added.keys().max()) {
            s.added = (first..=last)
                .map(|m| (m, added.get(&m).copied().unwrap_or(0)))
                .collect();
        }

        let mut years: HashMap<u16, u64> = HashMap::new();
        let latest = u16::try_from(civil((now / 86_400) as i64).0 + 1).unwrap_or(u16::MAX);
        for a in &library.albums {
            if (FIRST_YEAR..=latest).contains(&a.year) {
                *years.entry(a.year).or_default() += 1;
            } else {
                s.no_year += 1;
            }
        }
        if let (Some(&first), Some(&last)) = (years.keys().min(), years.keys().max()) {
            let first = first.max(last.saturating_sub(MOST_YEARS - 1));
            s.no_year += years
                .iter()
                .filter(|(y, _)| **y < first)
                .map(|(_, n)| n)
                .sum::<u64>();
            s.years = (first..=last)
                .map(|y| (y, years.get(&y).copied().unwrap_or(0)))
                .collect();
        }

        let artists: Vec<TopArtist> = library
            .artists
            .iter()
            .filter(|a| !a.albums.is_empty())
            .map(|a| {
                let tracks = a
                    .albums
                    .iter()
                    .flat_map(|&al| library.albums[al as usize].tracks.iter());
                let (plays, length_ms) = tracks.fold((0u64, 0u64), |(p, l), &t| {
                    (
                        p + u64::from(plays_of[t as usize]),
                        l + u64::from(library.tracks[t as usize].duration_ms),
                    )
                });
                TopArtist {
                    name: a.name.clone(),
                    albums: a.albums.len() as u32,
                    tracks: a.tracks,
                    plays,
                    length_ms,
                    album: a.albums[0],
                }
            })
            .collect();
        let mut by_tracks = artists.clone();
        by_tracks.sort_by(|a, b| {
            b.tracks
                .cmp(&a.tracks)
                .then_with(|| crate::format::natural_cmp(&a.name, &b.name))
        });
        by_tracks.truncate(TOP);
        s.top_artists = by_tracks;
        let mut by_plays: Vec<TopArtist> = artists.into_iter().filter(|a| a.plays > 0).collect();
        by_plays.sort_by(|a, b| {
            b.plays
                .cmp(&a.plays)
                .then_with(|| crate::format::natural_cmp(&a.name, &b.name))
        });
        by_plays.truncate(TOP);
        s.played_artists = by_plays;

        let mut top: Vec<(TrackId, u32)> = plays_of
            .iter()
            .enumerate()
            .filter(|(_, p)| **p > 0)
            .map(|(i, &p)| (i as TrackId, p))
            .collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top.truncate(TOP);
        s.top_tracks = top;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Track;
    use std::path::PathBuf;

    #[test]
    fn classes() {
        assert_eq!(bitrate_class(96, "mp3"), 0);
        assert_eq!(bitrate_class(128, "mp3"), 1);
        assert_eq!(bitrate_class(191, "mp3"), 2);
        assert_eq!(bitrate_class(245, "mp3"), 4);
        assert_eq!(bitrate_class(320, "mp3"), 6);
        assert_eq!(bitrate_class(900, "flac"), BITRATE_LOSSLESS);
        assert_eq!(bitrate_class(0, "wav"), BITRATE_LOSSLESS);
        assert_eq!(bitrate_class(780, "m4a"), BITRATE_LOSSLESS);
        assert_eq!(length_class(59_000), 0);
        assert_eq!(length_class(3 * 60_000 + 5), 2);
        assert_eq!(length_class(7 * 60_000), 5);
        assert_eq!(length_class(45 * 60_000), 8);
        let now = 1_000_000_000;
        assert_eq!(recency_class(0, now), 4);
        assert_eq!(recency_class(now - 3600, now), 0);
        assert_eq!(recency_class(now - 10 * 86_400, now), 1);
        assert_eq!(recency_class(now - 100 * 86_400, now), 2);
        assert_eq!(recency_class(now - 400 * 86_400, now), 3);
    }

    #[test]
    fn months() {
        assert_eq!(civil(0), (1970, 1, 1));
        // 2024-02-29 and 2026-11-05.
        assert_eq!(civil(19_782), (2024, 2, 29));
        assert_eq!(civil(20_762), (2026, 11, 5));
        assert_eq!(month_of(0), 1970 * 12);
        assert_eq!(month_of(20_762 * 86_400 + 3600), 2026 * 12 + 10);
    }

    fn t(path: &str, artist: &str, album: &str, year: u16, genre: &str) -> Track {
        Track {
            path: PathBuf::from(path),
            artist: artist.into(),
            album: album.into(),
            title: path.into(),
            year,
            genre: genre.into(),
            duration_ms: 200_000,
            bitrate: 320,
            sample_rate: 44_100,
            channels: 2,
            size: 8_000_000,
            // 2020-01-15 and the months after, by the album.
            added: 1_579_046_400 + u64::from(year % 3) * 40 * 86_400,
            ..Track::default()
        }
    }

    #[test]
    fn a_library_summed_up() {
        let library = Library::new(vec![
            t(r"D:\M\a\1.mp3", "Ария", "Химера", 2001, "Heavy Metal"),
            t(r"D:\M\a\2.mp3", "Ария", "Химера", 2001, "heavy metal"),
            t(
                r"D:\M\b\1.flac",
                "Ария",
                "Армагеддон",
                2006,
                "Heavy Metal; Rock",
            ),
            t(r"D:\M\c\1.mp3", "Abba", "Gold", 1992, ""),
            t(r"D:\M\d\1.mp3", "X", "Y", 0, "Pop"),
        ]);
        let mut stats = Stats::default();
        let now = 1_700_000_000;
        stats.played(&PathBuf::from(r"D:\M\c\1.mp3"), now - 60);
        stats.played(&PathBuf::from(r"D:\M\c\1.mp3"), now - 30);
        stats.played(&PathBuf::from(r"D:\M\a\2.mp3"), now - 200 * 86_400);
        stats.rate(&[PathBuf::from(r"D:\M\a\1.mp3")], 4);
        let s = Summary::of(&library, &stats, now);
        assert_eq!((s.tracks, s.albums, s.artists), (5, 4, 3));
        assert_eq!((s.plays, s.played, s.rated), (3, 2, 1));
        assert_eq!(s.heard_ms, 3 * 200_000);
        assert_eq!(s.bitrates[6], 4);
        assert_eq!(s.bitrates[BITRATE_LOSSLESS], 1);
        assert_eq!(s.formats[0].name, "MP3");
        assert_eq!(s.formats[0].tracks, 4);
        assert_eq!(s.formats[1].name, "FLAC");
        // Genres whatever their case, as most spell them; one with two.
        assert_eq!(s.genres[0], ("Heavy Metal".to_owned(), 3));
        assert_eq!(s.genres.len(), 3);
        assert_eq!(s.no_genre, 1);
        // Every year from the first to the last.
        assert_eq!(s.years.first(), Some(&(1992, 1)));
        assert_eq!(s.years.last(), Some(&(2006, 1)));
        assert_eq!(s.years.len(), 15);
        assert_eq!(s.no_year, 1);
        assert_eq!(s.added.iter().map(|(_, n)| n).sum::<u64>(), 5);
        assert_eq!(s.added[0].0, 2020 * 12);
        assert_eq!(s.lengths[2], 5);
        assert_eq!(s.top_artists[0].name, "Ария");
        assert_eq!(s.top_artists[0].tracks, 3);
        assert_eq!(s.top_artists[0].albums, 2);
        assert_eq!(s.played_artists[0].name, "Abba");
        assert_eq!(s.played_artists.len(), 2);
        assert_eq!(library.tracks[s.top_tracks[0].0 as usize].artist, "Abba");
        assert_eq!(s.top_tracks[0].1, 2);
        assert_eq!(s.ratings[4], 1);
        assert_eq!(s.ratings[0], 4);
        assert_eq!(s.recency, [1, 0, 1, 0, 3]);
        assert_eq!(s.tagged.year, 4);
        assert_eq!(s.tagged.genre, 4);
        assert_eq!(s.sample_rates, [(44_100, 5)]);
        assert_eq!(s.channels, [0, 5, 0]);
    }
}
