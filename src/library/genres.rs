//! The genres offered while editing tags: the library's own, then a
//! ready list of the usual ones (`COMMON`: the ID3v1 and Winamp genres and
//! those of taggers such as Tag&Rename, the exotic ones left out).

use super::Library;
use crate::format::natural_cmp;

/// The usual genres, alphabetically.
pub const COMMON: &[&str] = &[
    "Acid Jazz",
    "Acoustic",
    "Alternative",
    "Alternative Metal",
    "Alternative Rock",
    "Ambient",
    "Art Rock",
    "Audiobook",
    "Avant-garde",
    "Baroque",
    "Big Band",
    "Black Metal",
    "Bluegrass",
    "Blues",
    "Blues Rock",
    "Bossa Nova",
    "Breakbeat",
    "Britpop",
    "Celtic",
    "Chamber Music",
    "Chanson",
    "Children's",
    "Chillout",
    "Christian Rock",
    "Classic Rock",
    "Classical",
    "Comedy",
    "Country",
    "Country Rock",
    "Dance",
    "Dark Ambient",
    "Darkwave",
    "Death Metal",
    "Disco",
    "Doom Metal",
    "Downtempo",
    "Drum & Bass",
    "Dubstep",
    "Easy Listening",
    "EBM",
    "Electro",
    "Electronic",
    "Emo",
    "Ethnic",
    "Eurodance",
    "Experimental",
    "Folk",
    "Folk Metal",
    "Folk Rock",
    "Funk",
    "Fusion",
    "Garage Rock",
    "Glam Rock",
    "Gospel",
    "Gothic",
    "Gothic Metal",
    "Gothic Rock",
    "Grindcore",
    "Groove Metal",
    "Grunge",
    "Hard Rock",
    "Hardcore",
    "Heavy Metal",
    "Hip-Hop",
    "House",
    "IDM",
    "Indie",
    "Indie Pop",
    "Indie Rock",
    "Industrial",
    "Industrial Metal",
    "Instrumental",
    "Instrumental Rock",
    "J-Pop",
    "Jazz",
    "Jazz Fusion",
    "Latin",
    "Lounge",
    "Melodic Death Metal",
    "Metal",
    "Metalcore",
    "Modern Metal",
    "Musical",
    "Neoclassical",
    "New Age",
    "New Wave",
    "Noise",
    "Nu Metal",
    "Opera",
    "Pop",
    "Pop Punk",
    "Pop Rock",
    "Post-Hardcore",
    "Post-Metal",
    "Post-Punk",
    "Post-Rock",
    "Power Metal",
    "Progressive Metal",
    "Progressive Rock",
    "Psychedelic Rock",
    "Psytrance",
    "Punk",
    "Punk Rock",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Rock & Roll",
    "Shoegaze",
    "Singer-Songwriter",
    "Ska",
    "Soft Rock",
    "Soul",
    "Soundtrack",
    "Southern Rock",
    "Speed Metal",
    "Spoken Word",
    "Stoner Rock",
    "Symphonic Metal",
    "Symphonic Rock",
    "Synth-pop",
    "Synthwave",
    "Techno",
    "Thrash Metal",
    "Trance",
    "Trip-Hop",
    "Viking Metal",
    "World",
];

/// The genres the library's tracks have (a tag's several, split at `;`),
/// each once whatever its case (the spelling most of its tracks have), in
/// natural order.
pub fn of_library(library: &Library) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, std::collections::HashMap<&str, usize>> =
        std::collections::HashMap::new();
    for t in &library.tracks {
        for g in t.genre.split(';').map(str::trim).filter(|g| !g.is_empty()) {
            *seen
                .entry(g.to_lowercase())
                .or_default()
                .entry(g)
                .or_default() += 1;
        }
    }
    let mut genres: Vec<String> = seen
        .into_values()
        .filter_map(|spellings| {
            spellings
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(s, _)| s.to_owned())
        })
        .collect();
    genres.sort_by(|a, b| natural_cmp(a, b));
    genres
}

/// The usual genres the library does not have already (any case).
pub fn common_besides(library: &[String]) -> Vec<&'static str> {
    COMMON
        .iter()
        .copied()
        .filter(|g| !library.iter().any(|l| l.eq_ignore_ascii_case(g)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_common_genres_are_sorted_and_each_once() {
        for pair in COMMON.windows(2) {
            assert!(
                pair[0].to_lowercase() < pair[1].to_lowercase(),
                "{} {}",
                pair[0],
                pair[1]
            );
        }
        assert!(COMMON.iter().all(|g| g.trim() == *g && !g.contains(';')));
    }

    #[test]
    fn the_librarys_genres_once_in_their_usual_spelling() {
        let mut library = Library::default();
        for g in ["Rock", "rock", "Rock", "Metal; Rock", "", "Heavy Metal"] {
            library.tracks.push(crate::library::Track {
                genre: g.to_owned(),
                ..Default::default()
            });
        }
        assert_eq!(of_library(&library), ["Heavy Metal", "Metal", "Rock"]);
        let common = common_besides(&of_library(&library));
        assert!(!common.contains(&"Rock") && !common.contains(&"Metal"));
        assert!(common.contains(&"Jazz"));
    }
}
