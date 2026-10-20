//! The playlist's line of a track, by a template as foobar2000's: fields
//! between percent signs (`%artist%`), and a part in square brackets shown
//! only when a field in it has something (`[%artist% – ]%title%`: the
//! title alone when the artist is not known). A name between percent
//! signs that is no field, and a lone percent sign, stay as typed.

use crate::library::Track;

/// The line as it was before the template: "Artist – Title".
pub const DEFAULT: &str = "[%artist% – ]%title%";

/// The templates offered.
pub const PRESETS: [&str; 5] = [
    DEFAULT,
    "%title%",
    "%title%[ – %artist%]",
    "[%artist% – ][%album% – ]%title%",
    "[%artist% – ]%title%[ (%year%)]",
];

/// The fields, as they are named in a template.
pub const FIELDS: [&str; 9] = [
    "artist",
    "title",
    "album",
    "albumartist",
    "year",
    "track",
    "disc",
    "genre",
    "filename",
];

/// A field's name as people read it, for the presets' labels.
pub fn field_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "artist" => tr!("Artist", "Исполнитель"),
        "title" => tr!("Title", "Название"),
        "album" => tr!("Album", "Альбом"),
        "albumartist" => tr!("Album artist", "Исполнитель альбома"),
        "year" => tr!("Year", "Год"),
        "track" => tr!("Track", "Номер"),
        "disc" => tr!("Disc", "Диск"),
        "genre" => tr!("Genre", "Жанр"),
        "filename" => tr!("File name", "Имя файла"),
        _ => return None,
    })
}

/// `template` with its fields filled by `field` (none: no such field).
pub fn render(template: &str, field: &dyn Fn(&str) -> Option<String>) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut at = 0;
    render_part(&chars, &mut at, field, false).0
}

/// A track's line by `template`; the file's name when the line would be
/// empty.
pub fn line(template: &str, track: &Track) -> String {
    let file = || {
        track
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let number = |n: u32| if n == 0 { String::new() } else { n.to_string() };
    let text = render(template, &|name| {
        Some(match name {
            "artist" => track.artist.clone(),
            "title" => track.title.clone(),
            "album" => track.album.clone(),
            "albumartist" => track.album_artist.clone(),
            "year" => number(u32::from(track.year)),
            "track" => number(u32::from(track.track)),
            "disc" => number(u32::from(track.disc)),
            "genre" => track.genre.clone(),
            "filename" => file(),
            _ => return None,
        })
    });
    if text.trim().is_empty() { file() } else { text }
}

/// From `at` to the end, or to the `]` closing a part (`inner`): the text,
/// and whether a field in it had something.
fn render_part(
    chars: &[char],
    at: &mut usize,
    field: &dyn Fn(&str) -> Option<String>,
    inner: bool,
) -> (String, bool) {
    let mut out = String::new();
    let mut filled = false;
    while *at < chars.len() {
        let c = chars[*at];
        *at += 1;
        match c {
            ']' if inner => return (out, filled),
            '[' => {
                let (text, any) = render_part(chars, at, field, true);
                if any {
                    out.push_str(&text);
                    filled = true;
                }
            }
            '%' => {
                // A field's name up to the next percent sign.
                let end = chars[*at..].iter().position(|&c| c == '%');
                let name: Option<String> = end.map(|e| chars[*at..*at + e].iter().collect());
                match name.as_deref().and_then(|n| field(n).map(|v| (n.len(), v))) {
                    Some((len, value)) => {
                        *at += len + 1;
                        filled |= !value.trim().is_empty();
                        out.push_str(&value);
                    }
                    None => out.push('%'),
                }
            }
            c => out.push(c),
        }
    }
    (out, filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(name: &str) -> Option<String> {
        Some(
            match name {
                "artist" => "Ария",
                "title" => "Беспечный ангел",
                "album" => "",
                "year" => "1998",
                _ => return None,
            }
            .to_owned(),
        )
    }

    #[test]
    fn fields_and_parts() {
        assert_eq!(render(DEFAULT, &fields), "Ария – Беспечный ангел");
        assert_eq!(render("%title%[ (%year%)]", &fields), "Беспечный ангел (1998)");
        // The album is empty: its part goes.
        assert_eq!(
            render("[%artist% – ][%album% – ]%title%", &fields),
            "Ария – Беспечный ангел"
        );
        // Nested parts; a part without fields is never shown.
        assert_eq!(render("[[%album%]x]%title%", &fields), "Беспечный ангел");
        assert_eq!(render("[plain]%title%", &fields), "Беспечный ангел");
        // Not fields: as typed.
        assert_eq!(render("%nope% 100% %title", &fields), "%nope% 100% %title");
        // An unclosed part runs to the end.
        assert_eq!(render("%title%[ – %artist%", &fields), "Беспечный ангел – Ария");
    }

    #[test]
    fn presets_render_their_labels() {
        let label = |t: &str| render(t, &|n| field_label(n).map(str::to_owned));
        for p in PRESETS {
            assert!(!label(p).contains('%'), "{p}");
        }
        assert!(FIELDS.iter().all(|f| field_label(f).is_some()));
    }
}
