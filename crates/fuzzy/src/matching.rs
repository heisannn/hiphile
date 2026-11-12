use nucleo_matcher::{Matcher, Utf32Str};

use crate::romaji::{ends_a_syllable, finished_ways, is_readable, unvoiced};
use crate::searchable::{Key, Searchable, is_kanji, normalized};

/// How well an atom matched, the lower the better.
pub(crate) type Tier = u8;

const EXACT: Tier = 0;
const PREFIX: Tier = 1;
const WORD_START: Tier = 2;
/// Like the three above, by how Japanese reads rather than how it is written.
const READ_EXACT: Tier = 3;
const READ_PREFIX: Tier = 4;
const READ_WORD_START: Tier = 5;
/// Below a reading that starts a word: romaji is half vowels, so a word typed
/// for its reading is inside many a title that has nothing to do with it.
const INSIDE_WORD: Tier = 6;
const READ_INSIDE: Tier = 7;
/// Another spelling of a word that is not Japanese, as `sinjuku` for Shinjuku.
const RESPELLED: Tier = 8;
/// As far as voicing goes, the same as a reading that starts a word.
const UNVOICED_START: Tier = 9;
/// As far as voicing goes, the same as [`READ_INSIDE`].
const UNVOICED_INSIDE: Tier = 10;
const SCATTERED: Tier = 11;
const MISTYPED: Tier = 12;

/// Below this, in romaji, only the start of a word counts. A letter or two
/// matches inside nearly anything, since romaji is half vowels.
const MIN_ANYWHERE: usize = 3;
/// A subsequence must not spread beyond this many times its own length, nor
/// start inside a word, or it would match nearly anything for the same reason.
const SPREAD_NUMERATOR: usize = 3;
const SPREAD_DENOMINATOR: usize = 2;
/// How long a word must be before a mistake in it is forgiven.
const MIN_MISTYPED: usize = 5;
/// How long a word must be before a second mistake is forgiven.
const MIN_TWICE_MISTYPED: usize = 9;

#[derive(Debug)]
pub(crate) struct Atom {
    text: String,
    /// How it reads, when all of it can be read: kanji says nothing by itself.
    spelled: Option<String>,
    unvoiced: Option<String>,
    /// What `spelled` may become once its last syllable is typed to the end.
    unfinished: Vec<String>,
}

/// The words of `query`, every one of which a match needs.
pub(crate) fn atoms(query: &str) -> Vec<Atom> {
    normalized(query)
        .split_whitespace()
        .map(Atom::new)
        .collect()
}

impl Atom {
    fn new(word: &str) -> Self {
        // Punctuation does not get in the way of reading what is around it, as
        // in don't. Kanji does: its reading is not to be guessed at here.
        let reads = !word.chars().any(is_kanji) && word.chars().any(is_readable);
        let spelled = reads
            .then(|| Key::new(word))
            .flatten()
            .map(|key| key.spelled.into_string());
        Self {
            text: word.to_owned(),
            // A sound cut off says too little to overlook its voicing: `ship`
            // is not the ジヘ of ジヘン.
            unvoiced: spelled
                .as_deref()
                .filter(|it| ends_a_syllable(it))
                .map(unvoiced),
            unfinished: spelled.as_deref().map_or_else(Vec::new, finished_ways),
            spelled,
        }
    }
}

pub(crate) struct Scratch {
    pub(crate) matcher: Matcher,
    indices: Vec<u32>,
    needle: Vec<char>,
    haystack: Vec<char>,
}

impl Scratch {
    pub(crate) fn new() -> Self {
        Self {
            matcher: Matcher::default(),
            indices: Vec::new(),
            needle: Vec::new(),
            haystack: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Whole,
    Start,
    WordStart,
    InsideWord,
}

fn contiguous(needle: &str, text: &str, starts: &[u32]) -> Option<Place> {
    if text == needle {
        return Some(Place::Whole);
    }
    if text.starts_with(needle) {
        return Some(Place::Start);
    }
    let mut found = None;
    for (ix, _) in text.match_indices(needle) {
        if u32::try_from(ix).is_ok_and(|ix| starts.binary_search(&ix).is_ok()) {
            return Some(Place::WordStart);
        }
        found = Some(Place::InsideWord);
    }
    found
}

const fn as_written(place: Place) -> Tier {
    match place {
        Place::Whole => EXACT,
        Place::Start => PREFIX,
        Place::WordStart => WORD_START,
        Place::InsideWord => INSIDE_WORD,
    }
}

/// How good a match on a spelling of `searchable` is, if it counts: inside a
/// word, what spelling folds together stops meaning anything unless the text
/// is Japanese, where it is the only way in. `ship` is not in Gossip. Nor does
/// a spelling too short to read, which doubled letters fold to: `zzzz` spells
/// `z`, which is not after John.
fn as_spelled(place: Place, needle: &str, searchable: &Searchable) -> Option<Tier> {
    match (place, searchable.has_japanese) {
        (Place::InsideWord, false) => None,
        (_, false) if needle.len() < MIN_ANYWHERE => None,
        (Place::InsideWord, true) if needle.len() < MIN_ANYWHERE => None,
        (Place::InsideWord, true) => Some(READ_INSIDE),
        (_, false) => Some(RESPELLED),
        (Place::Whole, true) => Some(READ_EXACT),
        (Place::Start, true) => Some(READ_PREFIX),
        (Place::WordStart, true) => Some(READ_WORD_START),
    }
}

fn is_close_subsequence(needle: &str, text: &str, starts: &[u32], scratch: &mut Scratch) -> bool {
    let wanted = needle.chars().count();
    if wanted < MIN_ANYWHERE {
        return false;
    }
    scratch.indices.clear();
    let found = scratch.matcher.fuzzy_indices(
        Utf32Str::new(text, &mut scratch.haystack),
        Utf32Str::new(needle, &mut scratch.needle),
        &mut scratch.indices,
    );
    let (Some(_), Some(first), Some(last)) = (
        found,
        scratch.indices.iter().min().copied(),
        scratch.indices.iter().max().copied(),
    ) else {
        return false;
    };
    let spread = (last - first + 1) as usize;
    if spread * SPREAD_DENOMINATOR > wanted * SPREAD_NUMERATOR {
        return false;
    }
    let byte = text
        .char_indices()
        .nth(first as usize)
        .map_or(0, |(ix, _)| ix);
    u32::try_from(byte).is_ok_and(|byte| starts.binary_search(&byte).is_ok())
}

/// How well `atom` matches `searchable`, short of forgiving a mistake.
pub(crate) fn tier(atom: &Atom, searchable: &Searchable, scratch: &mut Scratch) -> Option<Tier> {
    let mut best = contiguous(&atom.text, &searchable.text, &searchable.starts).map(as_written);
    // Half a syllable says too little of a text that is not Japanese: `c` is
    // not after Tiger, however it may start ち.
    let unfinished: &[String] = if searchable.has_japanese {
        &atom.unfinished
    } else {
        &[]
    };
    for needle in atom.spelled.iter().chain(unfinished) {
        for key in &searchable.keys {
            let found = contiguous(needle, &key.spelled, &key.starts)
                .and_then(|place| as_spelled(place, needle, searchable));
            best = better(best, found);
        }
    }
    if best.is_some_and(|tier| tier < UNVOICED_START) {
        return best;
    }
    if let Some(unvoiced) = atom.unvoiced.as_ref().filter(|it| it.len() >= MIN_ANYWHERE) {
        for key in &searchable.keys {
            let Some(key_unvoiced) = &key.unvoiced else {
                continue;
            };
            let found = contiguous(unvoiced, key_unvoiced, &key.starts).map(|place| {
                if place == Place::InsideWord {
                    UNVOICED_INSIDE
                } else {
                    UNVOICED_START
                }
            });
            best = better(best, found);
        }
    }
    if best.is_some() {
        return best;
    }
    // Scattered letters of a respelled word are too far from what was typed:
    // `fun` is not after Hound.
    let is_scattered =
        is_close_subsequence(&atom.text, &searchable.text, &searchable.starts, scratch)
            || (searchable.has_japanese
                && atom.spelled.as_ref().is_some_and(|spelled| {
                    searchable.keys.iter().any(|key| {
                        is_close_subsequence(spelled, &key.spelled, &key.starts, scratch)
                    })
                }));
    is_scattered.then_some(SCATTERED)
}

fn better(a: Option<Tier>, b: Option<Tier>) -> Option<Tier> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

pub(crate) fn is_mistyped(atom: &Atom, searchable: &Searchable) -> bool {
    let Some(spelled) = atom.spelled.as_ref().filter(|it| it.len() >= MIN_MISTYPED) else {
        return false;
    };
    let allowed = if spelled.len() >= MIN_TWICE_MISTYPED {
        2
    } else {
        1
    };
    searchable
        .keys
        .iter()
        .any(|key| edit_distance_within(spelled.as_bytes(), key.spelled.as_bytes()) <= allowed)
}

pub(crate) const fn mistyped_tier() -> Tier {
    MISTYPED
}

/// The fewest edits that make `needle` appear somewhere in `text`, counting two
/// neighbors swapped as one.
fn edit_distance_within(needle: &[u8], text: &[u8]) -> usize {
    let len = needle.len();
    let mut before_last = vec![0; len + 1];
    let mut last: Vec<usize> = (0..=len).collect();
    let mut current = vec![0; len + 1];
    let mut best = last[len];
    for (at, c) in text.iter().enumerate() {
        current[0] = 0;
        for ix in 1..=len {
            let replaced = last[ix - 1] + usize::from(needle[ix - 1] != *c);
            let mut edits = replaced.min(last[ix] + 1).min(current[ix - 1] + 1);
            if ix > 1 && at > 0 && needle[ix - 1] == text[at - 1] && needle[ix - 2] == *c {
                edits = edits.min(before_last[ix - 2] + 1);
            }
            current[ix] = edits;
        }
        best = best.min(current[len]);
        std::mem::swap(&mut before_last, &mut last);
        std::mem::swap(&mut last, &mut current);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Analyzer;

    struct Reads(&'static str);

    impl Analyzer for Reads {
        fn readings(&self, _: &str) -> Vec<String> {
            vec![self.0.to_owned()]
        }
    }

    fn tier_of(query: &str, text: &str, reading: Option<&'static str>) -> Option<Tier> {
        let reads = reading.map(Reads);
        let searchable = Searchable::new(text, reads.as_ref().map(|it| it as &dyn crate::Speller));
        let atom = &atoms(query)[0];
        tier(atom, &searchable, &mut Scratch::new())
    }

    #[test]
    fn the_better_the_place_the_lower_the_tier() {
        assert_eq!(tier_of("blue", "Blue", None), Some(EXACT));
        assert_eq!(tier_of("blue", "Blue Train", None), Some(PREFIX));
        assert_eq!(tier_of("train", "Blue Train", None), Some(WORD_START));
        assert_eq!(tier_of("rain", "Blue Train", None), Some(INSIDE_WORD));
        assert_eq!(tier_of("bltn", "Blue Train", None), None);
    }

    #[test]
    fn kanji_is_found_by_its_reading_in_any_spelling() {
        let lost = Some("ヨル ニ カケル");
        for typed in ["yorunikakeru", "yoru", "よるにかける", "ヨルニカケル"] {
            assert!(
                tier_of(typed, "夜に駆ける", lost).is_some_and(|it| it <= READ_PREFIX),
                "{typed}"
            );
        }
        assert_eq!(tier_of("kakeru", "夜に駆ける", lost), Some(READ_WORD_START));
        assert_eq!(tier_of("yorunikakeru", "夜に駆ける", None), None);
    }

    #[test]
    fn systems_of_romaji_do_not_matter() {
        let reading = Some("トウキョウ ジヘン");
        for typed in ["toukyoujihen", "tokyojihen", "tookyoozihen", "toukyouzihen"] {
            assert_eq!(
                tier_of(typed, "東京事変", reading),
                Some(READ_EXACT),
                "{typed}"
            );
        }
        let reading = Some("シンジュク");
        for typed in ["shinjuku", "sinzyuku", "shinjyuku"] {
            assert_eq!(tier_of(typed, "新宿", reading), Some(READ_EXACT), "{typed}");
        }
    }

    #[test]
    fn kanji_also_matches_as_written() {
        assert_eq!(tier_of("夜に", "夜に駆ける", None), Some(PREFIX));
        assert_eq!(tier_of("駆ける", "夜に駆ける", None), Some(INSIDE_WORD));
    }

    #[test]
    fn voicing_only_narrowly_misses() {
        let reading = Some("ヨネツ ゲン シ");
        assert_eq!(
            tier_of("yonezukenshi", "米津玄師", reading),
            Some(UNVOICED_START)
        );
        assert_eq!(tier_of("kenshi", "米津玄師", reading), Some(UNVOICED_START));
        assert_eq!(tier_of("nezu", "米津玄師", reading), Some(UNVOICED_INSIDE));
    }

    #[test]
    fn a_short_word_in_romaji_only_matches_where_a_word_starts() {
        let reading = Some("サカイ ミク");
        assert_eq!(tier_of("ai", "酒井未来", reading), None);
        assert_eq!(tier_of("mi", "酒井未来", reading), Some(READ_WORD_START));
    }

    #[test]
    fn a_short_word_as_written_still_matches_anywhere() {
        assert_eq!(tier_of("ai", "Rain Air", None), Some(WORD_START));
        assert_eq!(tier_of("ai", "Rain", None), Some(INSIDE_WORD));
    }

    #[test]
    fn a_subsequence_must_be_close_and_start_on_a_word() {
        let reading = Some("ヨルニカケル");
        // One letter left out, as in a typing slip
        assert_eq!(
            tier_of("yornikakeru", "夜に駆ける", reading),
            Some(SCATTERED)
        );
        // Spread over the whole reading, which any romaji would allow
        assert_eq!(tier_of("yoke", "夜に駆ける", reading), None);
        // Close, but inside a word
        assert_eq!(tier_of("rnk", "夜に駆ける", reading), None);
    }

    #[test]
    fn a_word_that_is_not_japanese_is_respelled_only_from_its_start() {
        assert_eq!(tier_of("sinjuku", "Shinjuku", None), Some(RESPELLED));
        for (typed, title) in [
            ("ship", "Gossip"),
            ("work", "York"),
            ("work", "Fork"),
            ("woman", "Romance"),
        ] {
            assert_eq!(tier_of(typed, title, None), None, "{typed} in {title}");
        }
        assert_eq!(tier_of("fun", "Hound", None), None);
    }

    #[test]
    fn what_is_written_beats_what_is_spelled_alike() {
        for (typed, written, alike) in [
            ("ship", "Ship", "Sip"),
            ("fun", "Funky", "Hun"),
            ("duke", "Duke", "Zuke"),
            ("ship", "Worship", "Sip"),
        ] {
            let written = tier_of(typed, written, None).unwrap();
            let alike = tier_of(typed, alike, None).unwrap();
            assert!(written < alike, "{typed}");
        }
    }

    #[test]
    fn a_reading_that_starts_a_word_beats_a_word_inside_a_title() {
        let reading = tier_of("ai", "愛", Some("アイ")).unwrap();
        let inside = tier_of("ai", "Rain", None).unwrap();
        assert!(reading < inside);
    }

    #[test]
    fn voicing_is_forgiven_only_in_what_a_dictionary_read() {
        assert_eq!(tier_of("pop", "Bob Dylan", None), None);
        assert_eq!(tier_of("bad", "Hat Trick", None), None);
        assert_eq!(tier_of("god", "Got to Go", None), None);
        // Cut off inside a syllable
        assert_eq!(tier_of("ship", "事変", Some("ジヘン")), None);
        // As written in kana, the voicing is known
        assert_eq!(tier_of("kenshi", "げんし", None), None);
    }

    #[test]
    fn half_a_syllable_finds_what_it_may_become() {
        let tests = [
            ("新宿", "シンジュク", "shinjuku"),
            ("新宿", "シンジュク", "sinjuku"),
            ("富士", "フジ", "fuji"),
            ("月", "ツキ", "tsuki"),
            ("地球", "チキュウ", "chikyuu"),
            ("茶", "チャ", "cha"),
            ("抹茶", "マッチャ", "matcha"),
            ("新聞", "シンブン", "shimbun"),
            ("写真", "シャシン", "shashin"),
            ("順", "ジュン", "jun"),
            ("日本", "ニッポン", "nippon"),
            ("京", "キョウ", "kyou"),
        ];
        for (title, reading, typed) in tests {
            for end in 1..=typed.len() {
                assert!(
                    tier_of(&typed[..end], title, Some(reading)).is_some(),
                    "{} of {title}",
                    &typed[..end],
                );
            }
        }
    }

    #[test]
    fn letters_folded_short_say_nothing_of_a_word_that_is_not_japanese() {
        for (typed, title) in [
            ("z", "John Coltrane"),
            ("zzzz", "John Coltrane"),
            ("qqqq", "Queen"),
            ("ssss", "Shinjuku"),
        ] {
            assert_eq!(tier_of(typed, title, None), None, "{typed} in {title}");
        }
        assert_eq!(tier_of("jo", "John Coltrane", None), Some(PREFIX));
    }

    #[test]
    fn half_a_syllable_says_nothing_of_other_syllables() {
        assert_eq!(tier_of("sh", "桜", Some("サクラ")), None);
        assert_eq!(tier_of("ch", "田", Some("タ")), None);
        assert_eq!(tier_of("c", "Tiger", None), None);
    }

    fn mistyped(query: &str, reading: &'static str) -> bool {
        let reads = Reads(reading);
        let searchable = Searchable::new("夜に駆ける", Some(&reads));
        is_mistyped(&atoms(query)[0], &searchable)
    }

    #[test]
    fn a_mistake_is_forgiven_only_in_a_long_enough_word() {
        let reading = "ヨルニカケル";
        assert!(mistyped("yorunikaxeru", reading), "replaced");
        assert!(mistyped("yorunikaekru", reading), "swapped");
        assert!(mistyped("yoxun", reading), "replaced in five letters");
        assert!(!mistyped("yoxu", reading), "too short");
        assert!(!mistyped("xoxun", reading), "two in five letters");
        assert!(mistyped("xorunikaxeru", reading), "two in nine letters");
    }

    #[test]
    fn edits_count_inside_the_text() {
        assert_eq!(edit_distance_within(b"abc", b"xxabcxx"), 0);
        assert_eq!(edit_distance_within(b"abc", b"xxaxcxx"), 1);
        assert_eq!(edit_distance_within(b"abcd", b"xxacbdxx"), 1);
    }
}
