//! Whether a song a service offers is the one asked for. Showing another song's
//! lyrics is worse than showing none, so a name has to agree word by word: "Fire"
//! is not held by "Firestarter", and a name with nothing to compare agrees with
//! nothing.

use std::time::Duration;

use crate::TrackQuery;

/// How far the length of a track found may be from the song's for its times to
/// be trusted. A track further off is another version, whose words may still be
/// right but whose times are not. Services are often some way off for the very
/// version asked for: 275 seconds for a song of 261, with its last word at 244.
const MAX_LENGTH_DIFFERENCE: Duration = Duration::from_secs(30);

/// How close the lengths must be to take the word of a service whose matching
/// looked at the artist, or for a song with no artist to compare.
const NEAR_LENGTH_DIFFERENCE: Duration = Duration::from_secs(10);

/// How close the lengths must be to make up for what is more likely another song.
/// Songs of about the same length are common, so this is little more than rounding.
const CLOSE_LENGTH_DIFFERENCE: Duration = Duration::from_secs(3);

/// What follows a title's name when it says which version it is.
const VERSION_MARKS: [char; 8] = ['(', '（', '[', '［', '【', '~', '〜', '～'];
const VERSION_WORDS: [&str; 5] = [" feat.", " feat ", " ft.", " ft ", " - "];

/// Names that say nothing about who a song is by, simplified: what servers and
/// taggers call the artist of a compilation.
const NOT_ANYONE: [&str; 2] = ["variousartists", "va"];

/// A song a service offers, as far as it says.
pub(crate) struct Offered<'a> {
    pub(crate) title: Option<&'a str>,
    pub(crate) artist: Option<&'a str>,
    pub(crate) length: Option<Duration>,
}

impl Offered<'_> {
    pub(crate) fn is_another_length(&self, query: &TrackQuery) -> bool {
        self.length_difference(query)
            .is_some_and(|difference| difference > MAX_LENGTH_DIFFERENCE)
    }

    /// Whether its title names the song asked for, in any version of it.
    pub(crate) fn has_title_of(&self, query: &TrackQuery) -> bool {
        matches!(
            self.title_match(query),
            TitleMatch::Same | TitleMatch::OtherVersion
        )
    }

    /// Whether this is the song asked for, from a service whose search goes by the
    /// title alone. The less the names say, the more the lengths have to. Another
    /// artist's name takes the same length to agree, as tags may write it in another
    /// script: "Kenshi Yonezu" for "米津玄師".
    pub(crate) fn is_the_song(&self, query: &TrackQuery) -> bool {
        let title_agrees = match self.title_match(query) {
            TitleMatch::Same => true,
            TitleMatch::OtherVersion => self.is_length_within(MAX_LENGTH_DIFFERENCE, query),
            TitleMatch::Partly => self.is_length_within(CLOSE_LENGTH_DIFFERENCE, query),
            TitleMatch::Other => false,
        };
        title_agrees
            && match self.artist_match(query) {
                ArtistMatch::Same => true,
                ArtistMatch::Partly => !self.is_another_length(query),
                ArtistMatch::Unknown => self.is_length_within(NEAR_LENGTH_DIFFERENCE, query),
                ArtistMatch::Other => {
                    self.has_title_of(query)
                        && self.is_length_within(CLOSE_LENGTH_DIFFERENCE, query)
                }
            }
    }

    /// Whether this may be the song asked for, from a service whose matching looks
    /// at both names but may write them in another script or language. One name that
    /// does not agree can be made up for by the lengths, and both cannot. An artist's
    /// name often differs in writing, and a title rarely does.
    pub(crate) fn may_be_the_song(&self, query: &TrackQuery) -> bool {
        let title_agrees = self.title.is_none() || self.has_title_of(query);
        let artist_agrees = match self.artist_match(query) {
            ArtistMatch::Same | ArtistMatch::Unknown => true,
            ArtistMatch::Partly => !self.is_another_length(query),
            ArtistMatch::Other => false,
        };
        match (title_agrees, artist_agrees) {
            (true, true) => true,
            (true, false) => self.is_length_within(NEAR_LENGTH_DIFFERENCE, query),
            (false, true) => self.is_length_within(CLOSE_LENGTH_DIFFERENCE, query),
            (false, false) => false,
        }
    }

    fn length_difference(&self, query: &TrackQuery) -> Option<Duration> {
        Some(self.length?.abs_diff(query.duration?))
    }

    fn is_length_within(&self, most: Duration, query: &TrackQuery) -> bool {
        self.length_difference(query)
            .is_some_and(|difference| difference <= most)
    }

    fn title_match(&self, query: &TrackQuery) -> TitleMatch {
        self.title
            .map_or(TitleMatch::Partly, |title| title_match(title, &query.title))
    }

    fn artist_match(&self, query: &TrackQuery) -> ArtistMatch {
        artist_match(self.artist, query.artist.as_deref())
    }
}

/// Whether `artist` can tell a song by them from another.
pub(crate) fn says_who(artist: &str) -> bool {
    let simplified = simplified(artist);
    !simplified.is_empty() && !NOT_ANYONE.contains(&simplified.as_str())
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TitleMatch {
    Same,
    /// The same but for what says which version it is.
    OtherVersion,
    /// One holds the other's words, or there is no telling.
    Partly,
    Other,
}

fn title_match(found: &str, asked: &str) -> TitleMatch {
    if same_name(found, asked) {
        TitleMatch::Same
    } else if same_name(without_version(found), without_version(asked)) {
        TitleMatch::OtherVersion
    } else if holds_words(found, asked) {
        TitleMatch::Partly
    } else {
        TitleMatch::Other
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ArtistMatch {
    Same,
    /// One holds the other's words: "Kaneee" and "Kaneee feat. STUTS".
    Partly,
    /// One of them does not say who it is.
    Unknown,
    Other,
}

fn artist_match(found: Option<&str>, asked: Option<&str>) -> ArtistMatch {
    let (Some(found), Some(asked)) = (
        found.filter(|name| says_who(name)),
        asked.filter(|name| says_who(name)),
    ) else {
        return ArtistMatch::Unknown;
    };
    if same_name(found, asked) {
        ArtistMatch::Same
    } else if holds_words(found, asked) {
        ArtistMatch::Partly
    } else {
        ArtistMatch::Other
    }
}

/// `title` up to what says which version it is, unless that is all there is:
/// "(I Can't Get No) Satisfaction".
pub(crate) fn without_version(title: &str) -> &str {
    let lower = title.to_ascii_lowercase();
    let cut = VERSION_WORDS
        .iter()
        .filter_map(|word| lower.find(word))
        .chain(title.find(VERSION_MARKS))
        .min()
        .unwrap_or(title.len());
    match title[..cut].trim() {
        "" => title,
        name => name,
    }
}

/// Whether both are one name, whatever their case and punctuation. Names of
/// punctuation alone have to be written alike.
fn same_name(a: &str, b: &str) -> bool {
    let simplified_a = simplified(a);
    simplified_a == simplified(b) && (!simplified_a.is_empty() || a.trim() == b.trim())
}

/// Whether the words of one come in a row in the other.
fn holds_words(a: &str, b: &str) -> bool {
    let (a, b) = (words(a), words(b));
    let (longer, shorter) = if a.len() < b.len() { (b, a) } else { (a, b) };
    !shorter.is_empty()
        && longer
            .windows(shorter.len())
            .any(|words| words == shorter.as_slice())
}

fn words(text: &str) -> Vec<String> {
    text.split(|char: char| !char.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn simplified(text: &str) -> String {
    text.chars()
        .filter(|char| char.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn offered<'a>(title: &'a str, artist: &'a str, length: u64) -> Offered<'a> {
        Offered {
            title: Some(title),
            artist: Some(artist),
            length: Some(secs(length)),
        }
    }

    fn asked(title: &str, artist: &str, length: u64) -> TrackQuery {
        TrackQuery::new(title)
            .artist(Some(artist))
            .duration(Some(secs(length)))
    }

    #[test]
    fn titles_compare_by_name_then_by_version_then_by_words() {
        for (found, asked, expected) in [
            ("A Song!", "a song", TitleMatch::Same),
            ("†", "†", TitleMatch::Same),
            ("A Song (feat. Someone)", "a song", TitleMatch::OtherVersion),
            ("A Song feat. Someone", "A Song", TitleMatch::OtherVersion),
            ("A Song - Remastered", "A Song", TitleMatch::OtherVersion),
            ("歌 〜副題〜", "歌", TitleMatch::OtherVersion),
            ("歌（Live）", "歌 [Remix]", TitleMatch::OtherVersion),
            ("Lost Children", "Lost", TitleMatch::Partly),
            ("Lostprophets", "Lost", TitleMatch::Other),
            ("恋の歌", "恋", TitleMatch::Other),
            ("Whatever Else", "†", TitleMatch::Other),
            ("★", "†", TitleMatch::Other),
        ] {
            assert_eq!(title_match(found, asked), expected, "{found} and {asked}");
        }
    }

    #[test]
    fn a_title_that_starts_with_its_brackets_keeps_them() {
        assert_eq!(without_version("(No) Satisfaction"), "(No) Satisfaction");
        assert_eq!(without_version("A Song (Live)"), "A Song");
        assert_eq!(without_version("Left FEAT. Right"), "Left");
    }

    #[test]
    fn artists_agree_by_whole_words_and_only_if_they_say_who() {
        for (found, asked, expected) in [
            ("Mr. Children", "mr.children", ArtistMatch::Same),
            ("KAT-TUN", "KATTUN", ArtistMatch::Same),
            ("Kaneee & STUTS", "Kaneee", ArtistMatch::Partly),
            ("Kaneee feat. STUTS", "STUTS", ArtistMatch::Partly),
            ("Kaneee", "Kaneee & STUTS", ArtistMatch::Partly),
            ("Arcade Fire", "Fire", ArtistMatch::Partly),
            ("Kaito", "AI", ArtistMatch::Other),
            ("Nirvana", "V.A.", ArtistMatch::Unknown),
            ("Someone", "★", ArtistMatch::Unknown),
            ("★", "Someone", ArtistMatch::Unknown),
            ("Various Artists", "Someone", ArtistMatch::Unknown),
        ] {
            let matched = artist_match(Some(found), Some(asked));
            assert_eq!(matched, expected, "{found} and {asked}");
        }
        assert_eq!(artist_match(None, Some("A")), ArtistMatch::Unknown);
        assert_eq!(artist_match(Some("A"), None), ArtistMatch::Unknown);
    }

    #[test]
    fn an_artist_who_partly_agrees_is_held_to_the_length() {
        let query = asked("A Song", "Fire", 200);
        assert!(offered("A Song", "Arcade Fire", 205).is_the_song(&query));
        assert!(!offered("A Song", "Arcade Fire", 300).is_the_song(&query));
    }

    #[test]
    fn the_less_a_title_says_the_closer_the_lengths_must_be() {
        let query = asked("Love", "One", 240);
        assert!(offered("Love (Live)", "One", 247).is_the_song(&query));
        assert!(offered("Love (Live)", "One", 265).is_the_song(&query));
        assert!(!offered("Love (Live)", "One", 280).is_the_song(&query));
        assert!(offered("Love Story", "One", 242).is_the_song(&query));
        assert!(!offered("Love Story", "One", 247).is_the_song(&query));
    }

    #[test]
    fn an_unknown_artist_is_held_to_the_length() {
        let query = TrackQuery::new("Story").duration(Some(secs(240)));
        assert!(offered("Story", "Someone", 245).is_the_song(&query));
        assert!(!offered("Story", "Someone", 100).is_the_song(&query));

        let no_length = TrackQuery::new("Story");
        assert!(!offered("Story", "Someone", 240).is_the_song(&no_length));
    }

    #[test]
    fn a_name_in_another_script_can_be_made_up_for_by_the_lengths() {
        let query = asked("Lemon", "米津玄師", 256);
        assert!(offered("Lemon", "Kenshi Yonezu", 256).may_be_the_song(&query));
        assert!(offered("Lemon", "Kenshi Yonezu", 265).may_be_the_song(&query));
        assert!(!offered("Lemon", "Someone Else", 200).may_be_the_song(&query));

        assert!(offered("Lemon", "Kenshi Yonezu", 255).is_the_song(&query));
        assert!(!offered("Lemon", "Kenshi Yonezu", 265).is_the_song(&query));
        assert!(!offered("Lemon (Live)", "Someone Else", 200).is_the_song(&query));

        let query = asked("夜に駆ける", "YOASOBI", 261);
        assert!(offered("Racing Into The Night", "YOASOBI", 260).may_be_the_song(&query));
        assert!(!offered("Racing Into The Night", "YOASOBI", 270).may_be_the_song(&query));
    }

    #[test]
    fn a_length_some_way_off_is_still_the_same_version() {
        let query = asked("夜に駆ける", "YOASOBI", 261);
        assert!(!offered("夜に駆ける", "YOASOBI", 275).is_another_length(&query));
        assert!(!offered("夜に駆ける", "YOASOBI", 291).is_another_length(&query));
        assert!(offered("夜に駆ける", "YOASOBI", 292).is_another_length(&query));
    }

    #[test]
    fn neither_name_agreeing_is_another_song_whatever_its_length() {
        let query = asked("Lemon", "米津玄師", 256);
        assert!(!offered("Orange", "Someone Else", 256).may_be_the_song(&query));
    }

    #[test]
    fn a_track_that_gives_no_names_may_be_the_song() {
        let offered = Offered {
            title: None,
            artist: None,
            length: None,
        };
        assert!(offered.may_be_the_song(&asked("Lemon", "米津玄師", 256)));
    }

    #[test]
    fn a_name_of_nothing_says_nobody() {
        for name in ["", "★", "Various Artists", "various artists", "V.A."] {
            assert!(!says_who(name), "{name}");
        }
        for name in ["One", "嵐", "Fire"] {
            assert!(says_who(name), "{name}");
        }
    }
}
