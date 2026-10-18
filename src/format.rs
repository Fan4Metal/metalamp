//! Numbers as the interface shows them: track times, the length and size
//! of the whole library, bit rates. The `_in` variants take the language,
//! so that tests do not depend on the global one.

use crate::i18n::{Lang, lang};
use std::cmp::Ordering;

/// A track's time: `4:07`, `1:02:03` past an hour.
pub fn time(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The length of many tracks, as Winamp's status line shows it:
/// `16:40:20`, `15 days+16:40:20` / `15 дн.+16:40:20`.
pub fn long_duration_in(lang: Lang, ms: u64) -> String {
    let s = ms / 1000;
    let (d, h, m, s) = (s / 86_400, s / 3600 % 24, s / 60 % 60, s % 60);
    let clock = format!("{h}:{m:02}:{s:02}");
    if d == 0 {
        return clock;
    }
    match lang {
        Lang::En => format!("{d} {}+{clock}", if d == 1 { "day" } else { "days" }),
        Lang::Ru => format!("{d} дн.+{clock}"),
    }
}

pub fn long_duration(ms: u64) -> String {
    long_duration_in(lang(), ms)
}

/// A size in binary units with two decimals past a kilobyte, as Windows
/// shows it: `49.02 GB` / `49,02 ГБ`.
pub fn size_in(lang: Lang, bytes: u64) -> String {
    let units: [&str; 5] = match lang {
        Lang::En => ["bytes", "KB", "MB", "GB", "TB"],
        Lang::Ru => ["байт", "КБ", "МБ", "ГБ", "ТБ"],
    };
    if bytes < 1024 {
        return format!("{bytes} {}", units[0]);
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let number = format!("{value:.2}");
    let number = match lang {
        Lang::En => number,
        Lang::Ru => number.replace('.', ","),
    };
    format!("{number} {}", units[unit])
}

pub fn size(bytes: u64) -> String {
    size_in(lang(), bytes)
}

/// A short time in milliseconds with one decimal: `0.4 ms` / `0,4 мс`.
pub fn millis_in(lang: Lang, secs: f64) -> String {
    let text = format!("{:.1}", secs * 1000.0);
    match lang {
        Lang::En => format!("{text} ms"),
        Lang::Ru => format!("{} мс", text.replace('.', ",")),
    }
}

/// A sample rate: `44.1 kHz` / `44,1 кГц`, `48 kHz`.
pub fn sample_rate(hz: u32) -> String {
    let khz = hz as f32 / 1000.0;
    let khz = if khz.fract() == 0.0 {
        format!("{khz:.0}")
    } else {
        format!("{khz:.1}")
    };
    tr!(
        format!("{khz} kHz"),
        format!("{} кГц", khz.replace('.', ","))
    )
}

/// The channels in words; `None` for 0.
pub fn channels(n: u8) -> Option<String> {
    match n {
        0 => None,
        1 => Some(tr!("Mono", "Моно").to_owned()),
        2 => Some(tr!("Stereo", "Стерео").to_owned()),
        n => Some(tr!(format!("{n} channels"), format!("{n} кан."))),
    }
}

/// A bit rate in kilobits per second: `192 kbps` / `192 кбит/с`.
pub fn bitrate(kbps: u32) -> String {
    if kbps == 0 {
        return String::new();
    }
    tr!(format!("{kbps} kbps"), format!("{kbps} кбит/с"))
}

/// Compare as Explorer does, ignoring case: runs of digits by their value,
/// the rest by character (metalmpv's `natural_cmp`, made case-blind for
/// names of artists and albums).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let lower = |s: &str| {
        s.chars()
            .flat_map(char::to_lowercase)
            .collect::<Vec<char>>()
    };
    let (a, b) = (lower(a), lower(b));
    let (mut i, mut j) = (0, 0);
    loop {
        match (a.get(i).copied(), b.get(j).copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let start = (i, j);
                while a.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
                while b.get(j).is_some_and(char::is_ascii_digit) {
                    j += 1;
                }
                let da: String = a[start.0..i].iter().collect();
                let db: String = b[start.1..j].iter().collect();
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let order = ta
                    .len()
                    .cmp(&tb.len())
                    .then_with(|| ta.cmp(tb))
                    .then_with(|| da.len().cmp(&db.len()));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                i += 1;
                j += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_times() {
        assert_eq!(time(0), "0:00");
        assert_eq!(time(247_900), "4:07");
        assert_eq!(time(3_723_000), "1:02:03");
    }

    #[test]
    fn library_length_in_both_languages() {
        let ms = ((15 * 86_400 + 16 * 3600 + 40 * 60 + 20) * 1000) as u64;
        assert_eq!(long_duration_in(Lang::En, ms), "15 days+16:40:20");
        assert_eq!(long_duration_in(Lang::Ru, ms), "15 дн.+16:40:20");
        assert_eq!(long_duration_in(Lang::En, 86_400_000), "1 day+0:00:00");
        assert_eq!(long_duration_in(Lang::En, 60_000), "0:01:00");
    }

    #[test]
    fn sizes_in_both_languages() {
        let bytes = (49.02 * 1024.0 * 1024.0 * 1024.0) as u64;
        assert_eq!(size_in(Lang::En, bytes), "49.02 GB");
        assert_eq!(size_in(Lang::Ru, bytes), "49,02 ГБ");
        assert_eq!(size_in(Lang::En, 512), "512 bytes");
        assert_eq!(size_in(Lang::En, 1536), "1.50 KB");
        assert_eq!(millis_in(Lang::Ru, 0.00042), "0,4 мс");
        assert_eq!(millis_in(Lang::En, 0.036), "36.0 ms");
    }

    #[test]
    fn natural_order_ignores_case() {
        assert_eq!(natural_cmp("ep2", "ep10"), Ordering::Less);
        assert_eq!(natural_cmp("abba", "ABBA"), Ordering::Equal);
        assert_eq!(natural_cmp("ac/dc", "Accept"), Ordering::Less);
        assert_eq!(natural_cmp("Ария", "ария"), Ordering::Equal);
        assert_eq!(natural_cmp("a1", "a01"), Ordering::Less);
        assert_eq!(natural_cmp("ab", "abc"), Ordering::Less);
    }
}
