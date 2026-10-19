//! Views of the library (Winamp's "smart views"): a named [`Filter`] over
//! the tracks, chosen under the artists' pane. The tracks it leaves are
//! what the three panes, the search and the status line work on, so an
//! artist with soundtracks and other albums is in both a "Soundtracks"
//! view and a view of everything else, with the counts of each.
//!
//! A filter is a list of [`Rule`]s, all of which must hold (or any one,
//! `any`). A rule tests one field of a track: a text (the genre, the
//! artist, the album, the title) against a value without regard to case,
//! a number (the year, the bit rate, the length in minutes, the days since
//! the file was added) against one or two, the folder the file is in, or
//! whether its tag holds a cover. The filters are kept in the settings
//! (`views`) and compiled ([`Filter::compile`]) once per view built.

use super::{Track, search_form};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    pub name: String,
    /// One rule holding is enough; else every rule must hold.
    pub any: bool,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub field: Field,
    pub op: Op,
    pub value: String,
    /// The end of the range of [`Op::Between`].
    pub to: String,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            field: Field::Genre,
            op: Op::Contains,
            value: String::new(),
            to: String::new(),
        }
    }
}

/// What a rule tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    #[default]
    Genre,
    /// The track's artist, or the album artist it is filed under.
    Artist,
    AlbumArtist,
    Album,
    Title,
    Year,
    /// The folder the file is in, or one above it.
    Folder,
    /// Kilobits per second.
    Bitrate,
    /// Minutes.
    Length,
    /// Days since the file was last written.
    Added,
    /// The tag holds a picture.
    Cover,
}

/// The kind of a field's values: which operations fit it and what the
/// editor shows for the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Number,
    Folder,
    Flag,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Is,
    IsNot,
    #[default]
    Contains,
    NotContains,
    StartsWith,
    /// Also "no" for a flag.
    Empty,
    /// Also "yes" for a flag.
    NotEmpty,
    Less,
    Greater,
    Between,
}

impl Field {
    pub const ALL: [Field; 11] = [
        Field::Genre,
        Field::Artist,
        Field::AlbumArtist,
        Field::Album,
        Field::Title,
        Field::Year,
        Field::Folder,
        Field::Bitrate,
        Field::Length,
        Field::Added,
        Field::Cover,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Genre => tr!("Genre", "Жанр"),
            Field::Artist => tr!("Artist", "Исполнитель"),
            Field::AlbumArtist => tr!("Album artist", "Исполнитель альбома"),
            Field::Album => tr!("Album", "Альбом"),
            Field::Title => tr!("Title", "Название"),
            Field::Year => tr!("Year", "Год"),
            Field::Folder => tr!("Folder", "Папка"),
            Field::Bitrate => tr!("Bit rate, kbps", "Битрейт, кбит/с"),
            Field::Length => tr!("Length, minutes", "Длительность, минут"),
            Field::Added => tr!("Added, days ago", "Добавлен, дней назад"),
            Field::Cover => tr!("Cover in the tag", "Обложка в теге"),
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            Field::Genre | Field::Artist | Field::AlbumArtist | Field::Album | Field::Title => {
                Kind::Text
            }
            Field::Year | Field::Bitrate | Field::Length | Field::Added => Kind::Number,
            Field::Folder => Kind::Folder,
            Field::Cover => Kind::Flag,
        }
    }

    /// The operations that fit the field, the first the usual one.
    pub fn ops(self) -> &'static [Op] {
        match self.kind() {
            Kind::Text => &[
                Op::Contains,
                Op::NotContains,
                Op::Is,
                Op::IsNot,
                Op::StartsWith,
                Op::Empty,
                Op::NotEmpty,
            ],
            Kind::Number => &[Op::Is, Op::IsNot, Op::Less, Op::Greater, Op::Between],
            Kind::Folder => &[Op::Is, Op::IsNot],
            Kind::Flag => &[Op::NotEmpty, Op::Empty],
        }
    }

    /// The texts of a text field (the artist has two: the track's and the
    /// album's).
    fn texts(self, t: &Track) -> [&str; 2] {
        match self {
            Field::Genre => [&t.genre, ""],
            Field::Artist => [&t.artist, &t.album_artist],
            Field::AlbumArtist => [&t.album_artist, ""],
            Field::Album => [&t.album, ""],
            Field::Title => [&t.title, ""],
            _ => ["", ""],
        }
    }

    /// The value of a number field; `None` when the file does not say.
    fn number(self, t: &Track, now: u64) -> Option<f64> {
        match self {
            Field::Year => (t.year > 0).then_some(f64::from(t.year)),
            Field::Bitrate => (t.bitrate > 0).then_some(f64::from(t.bitrate)),
            Field::Length => Some(f64::from(t.duration_ms) / 60_000.0),
            Field::Added => Some(now.saturating_sub(t.mtime) as f64 / 86_400.0),
            _ => None,
        }
    }
}

impl Op {
    /// The operation's name for a field of `kind`.
    pub fn label(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Op::Is, Kind::Folder) => tr!("inside", "внутри"),
            (Op::IsNot, Kind::Folder) => tr!("not inside", "не внутри"),
            (Op::NotEmpty, Kind::Flag) => tr!("yes", "есть"),
            (Op::Empty, Kind::Flag) => tr!("no", "нет"),
            (Op::Is, _) => tr!("is", "равно"),
            (Op::IsNot, _) => tr!("is not", "не равно"),
            (Op::Contains, _) => tr!("contains", "содержит"),
            (Op::NotContains, _) => tr!("does not contain", "не содержит"),
            (Op::StartsWith, _) => tr!("starts with", "начинается с"),
            (Op::Empty, _) => tr!("is empty", "пусто"),
            (Op::NotEmpty, _) => tr!("is not empty", "не пусто"),
            (Op::Less, _) => tr!("less than", "меньше"),
            (Op::Greater, _) => tr!("greater than", "больше"),
            (Op::Between, _) => tr!("between", "в диапазоне"),
        }
    }

    /// How many values the operation takes.
    pub fn values(self) -> usize {
        match self {
            Op::Empty | Op::NotEmpty => 0,
            Op::Between => 2,
            _ => 1,
        }
    }

    /// The operation as a positive one and whether the result is to be
    /// inverted: "is not" is the inverse of "is".
    fn positive(self) -> (Op, bool) {
        match self {
            Op::IsNot => (Op::Is, true),
            Op::NotContains => (Op::Contains, true),
            Op::Empty => (Op::NotEmpty, true),
            op => (op, false),
        }
    }
}

impl Filter {
    /// Whether the filter can be saved: a name, and every rule complete.
    pub fn is_complete(&self) -> bool {
        !self.name.trim().is_empty()
            && self.rules.iter().all(|r| match r.field.kind() {
                Kind::Number => parse_number(&r.value).is_some()
                    && (r.op != Op::Between || parse_number(&r.to).is_some()),
                Kind::Text | Kind::Folder => r.op.values() == 0 || !r.value.trim().is_empty(),
                Kind::Flag => true,
            })
    }

    /// The filter ready to test tracks: the texts in the search's form,
    /// the numbers parsed, `now` (Unix seconds) for the days since a file
    /// was added.
    pub fn compile(&self, now: u64) -> Compiled {
        let rules = self
            .rules
            .iter()
            .map(|r| {
                let (op, inverted) = r.op.positive();
                let test = match r.field.kind() {
                    Kind::Text => Test::Text {
                        op,
                        value: search_form(r.value.trim()),
                    },
                    Kind::Number => {
                        let a = parse_number(&r.value).unwrap_or(f64::NAN);
                        let b = if op == Op::Between {
                            parse_number(&r.to).unwrap_or(f64::NAN)
                        } else {
                            a
                        };
                        Test::Number {
                            op,
                            from: a.min(b),
                            to: a.max(b),
                        }
                    }
                    Kind::Folder => Test::Folder(folder_form(r.value.trim())),
                    Kind::Flag => Test::Flag,
                };
                CompiledRule {
                    field: r.field,
                    test,
                    inverted,
                }
            })
            .collect();
        Compiled {
            any: self.any,
            rules,
            now,
        }
    }
}

/// A [`Filter`] ready to test tracks.
#[derive(Clone, Debug)]
pub struct Compiled {
    any: bool,
    rules: Vec<CompiledRule>,
    now: u64,
}

#[derive(Clone, Debug)]
struct CompiledRule {
    field: Field,
    test: Test,
    inverted: bool,
}

#[derive(Clone, Debug)]
enum Test {
    /// `op` is Is, Contains, StartsWith or NotEmpty.
    Text { op: Op, value: String },
    /// `op` is Is, Less, Greater or Between; NaN never holds.
    Number { op: Op, from: f64, to: f64 },
    /// The folder in [`folder_form`].
    Folder(String),
    /// The flag is set.
    Flag,
}

impl Compiled {
    /// Whether `t` is in the view. A filter without rules takes every
    /// track.
    pub fn matches(&self, t: &Track) -> bool {
        if self.rules.is_empty() {
            return true;
        }
        let holds = |r: &CompiledRule| r.holds(t, self.now) != r.inverted;
        if self.any {
            self.rules.iter().any(holds)
        } else {
            self.rules.iter().all(holds)
        }
    }
}

impl CompiledRule {
    /// The positive test, before the inversion.
    fn holds(&self, t: &Track, now: u64) -> bool {
        match &self.test {
            Test::Text { op, value } => {
                let texts = self.field.texts(t);
                texts.iter().filter(|s| !s.is_empty()).any(|s| {
                    let s = search_form(s.trim());
                    match op {
                        Op::Is => s == *value,
                        Op::Contains => s.contains(value.as_str()),
                        Op::StartsWith => s.starts_with(value.as_str()),
                        Op::NotEmpty => true,
                        _ => false,
                    }
                })
            }
            Test::Number { op, from, to } => {
                let Some(x) = self.field.number(t, now) else {
                    return false;
                };
                match op {
                    Op::Is => (x - from).abs() < 0.5,
                    Op::Less => x < *from,
                    Op::Greater => x > *from,
                    Op::Between => *from <= x && x <= *to,
                    _ => false,
                }
            }
            Test::Folder(prefix) => {
                if prefix.is_empty() {
                    return false;
                }
                let folder = t.path.parent().map(folder_of).unwrap_or_default();
                folder == *prefix
                    || (folder.starts_with(prefix.as_str())
                        && folder[prefix.len()..].starts_with('\\'))
            }
            Test::Flag => t.has_cover,
        }
    }
}

/// A number as typed: a decimal comma is a point too; nothing else.
pub fn parse_number(text: &str) -> Option<f64> {
    let text = text.trim().replace(',', ".");
    text.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// A folder as typed, for comparing: lower case, `\` for `/`, no
/// separator at the end.
fn folder_form(text: &str) -> String {
    let mut s = text.replace('/', "\\").to_lowercase();
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    s
}

fn folder_of(path: &Path) -> String {
    folder_form(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn track(genre: &str, artist: &str, year: u16) -> Track {
        Track {
            path: PathBuf::from(r"D:\Music\Soundtracks\Dune\01 - Paul.mp3"),
            genre: genre.into(),
            artist: artist.into(),
            album_artist: "Hans Zimmer".into(),
            album: "Dune".into(),
            title: "Paul".into(),
            year,
            bitrate: 320,
            duration_ms: 4 * 60_000 + 30_000,
            mtime: 1_000_000,
            has_cover: true,
            ..Track::default()
        }
    }

    fn rule(field: Field, op: Op, value: &str) -> Rule {
        Rule {
            field,
            op,
            value: value.into(),
            to: String::new(),
        }
    }

    fn filter(rules: Vec<Rule>) -> Compiled {
        Filter {
            name: "test".into(),
            any: false,
            rules,
        }
        .compile(1_000_000 + 10 * 86_400)
    }

    #[test]
    fn text_rules_ignore_case_and_negate() {
        let t = track("Soundtrack; Score", "Hans Zimmer", 2021);
        assert!(filter(vec![rule(Field::Genre, Op::Contains, "soundtrack")]).matches(&t));
        assert!(!filter(vec![rule(Field::Genre, Op::NotContains, "SOUNDTRACK")]).matches(&t));
        assert!(filter(vec![rule(Field::Genre, Op::Is, "soundtrack; score")]).matches(&t));
        assert!(!filter(vec![rule(Field::Genre, Op::Is, "soundtrack")]).matches(&t));
        assert!(filter(vec![rule(Field::Album, Op::StartsWith, "du")]).matches(&t));
        assert!(filter(vec![rule(Field::Title, Op::NotEmpty, "")]).matches(&t));
        let no_genre = track("", "x", 0);
        assert!(filter(vec![rule(Field::Genre, Op::Empty, "")]).matches(&no_genre));
        assert!(!filter(vec![rule(Field::Genre, Op::NotEmpty, "")]).matches(&no_genre));
        // The search's "ё" as "е".
        let ru = track("Саундтрёк", "x", 0);
        assert!(filter(vec![rule(Field::Genre, Op::Is, "саундтрек")]).matches(&ru));
    }

    #[test]
    fn artist_is_the_tracks_or_the_albums() {
        let t = track("", "Zimmer feat. Someone", 2021);
        assert!(filter(vec![rule(Field::Artist, Op::Is, "hans zimmer")]).matches(&t));
        assert!(filter(vec![rule(Field::Artist, Op::Contains, "someone")]).matches(&t));
        assert!(!filter(vec![rule(Field::Artist, Op::IsNot, "hans zimmer")]).matches(&t));
        assert!(!filter(vec![rule(Field::AlbumArtist, Op::Contains, "someone")]).matches(&t));
    }

    #[test]
    fn number_rules() {
        let t = track("", "x", 1995);
        assert!(filter(vec![rule(Field::Year, Op::Is, "1995")]).matches(&t));
        assert!(filter(vec![rule(Field::Year, Op::Less, "2000")]).matches(&t));
        assert!(!filter(vec![rule(Field::Year, Op::Greater, "2000")]).matches(&t));
        let mut between = rule(Field::Year, Op::Between, "1999");
        between.to = "1990".into(); // the ends in either order
        assert!(filter(vec![between]).matches(&t));
        assert!(filter(vec![rule(Field::Length, Op::Greater, "4,25")]).matches(&t));
        assert!(filter(vec![rule(Field::Bitrate, Op::Is, "320")]).matches(&t));
        assert!(filter(vec![rule(Field::Added, Op::Less, "30")]).matches(&t));
        assert!(!filter(vec![rule(Field::Added, Op::Less, "5")]).matches(&t));
        // A number the file does not have holds for nothing but "is not".
        let no_year = track("", "x", 0);
        assert!(!filter(vec![rule(Field::Year, Op::Less, "2000")]).matches(&no_year));
        assert!(filter(vec![rule(Field::Year, Op::IsNot, "1995")]).matches(&no_year));
        // Not a number: never.
        assert!(!filter(vec![rule(Field::Year, Op::Less, "abc")]).matches(&t));
    }

    #[test]
    fn folder_rules_take_the_folder_or_one_above() {
        let t = track("", "x", 0);
        for folder in [
            r"D:\Music\Soundtracks\Dune",
            r"d:\music\soundtracks\",
            r"D:/Music",
        ] {
            assert!(
                filter(vec![rule(Field::Folder, Op::Is, folder)]).matches(&t),
                "{folder}"
            );
        }
        assert!(!filter(vec![rule(Field::Folder, Op::Is, r"D:\Music\Sound")]).matches(&t));
        assert!(filter(vec![rule(Field::Folder, Op::IsNot, r"D:\Music\Rock")]).matches(&t));
        assert!(!filter(vec![rule(Field::Folder, Op::Is, "")]).matches(&t));
    }

    #[test]
    fn flags_all_and_any() {
        let t = track("Rock", "x", 1980);
        assert!(filter(vec![rule(Field::Cover, Op::NotEmpty, "")]).matches(&t));
        assert!(!filter(vec![rule(Field::Cover, Op::Empty, "")]).matches(&t));
        let rules = vec![
            rule(Field::Genre, Op::Is, "rock"),
            rule(Field::Year, Op::Greater, "2000"),
        ];
        assert!(!filter(rules.clone()).matches(&t));
        let any = Filter {
            name: "any".into(),
            any: true,
            rules,
        }
        .compile(0);
        assert!(any.matches(&t));
        assert!(filter(vec![]).matches(&t));
    }

    #[test]
    fn completeness() {
        let mut f = Filter {
            name: "x".into(),
            any: false,
            rules: vec![rule(Field::Genre, Op::Contains, "")],
        };
        assert!(!f.is_complete());
        f.rules[0].value = "rock".into();
        assert!(f.is_complete());
        f.rules.push(rule(Field::Year, Op::Between, "1990"));
        assert!(!f.is_complete());
        f.rules[1].to = "1999".into();
        assert!(f.is_complete());
        f.rules.push(rule(Field::Cover, Op::Empty, ""));
        assert!(f.is_complete());
        f.name = " ".into();
        assert!(!f.is_complete());
    }

    #[test]
    fn json_names() {
        let f = Filter {
            name: "Soundtracks".into(),
            any: false,
            rules: vec![rule(Field::AlbumArtist, Op::NotContains, "x")],
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains(r#""field":"album_artist""#), "{json}");
        assert!(json.contains(r#""op":"not_contains""#), "{json}");
        let back: Filter = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }
}
