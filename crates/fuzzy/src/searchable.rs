use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization as _;

use crate::Speller;
use crate::romaji::{is_kana, is_readable, spelling, to_katakana, unvoiced};

/// `text` as it is compared: full-width and half-width forms in one form,
/// lowercase, and katakana in place of hiragana.
pub(crate) fn normalized(text: &str) -> String {
    text.nfkc()
        .flat_map(char::to_lowercase)
        .map(to_katakana)
        .collect()
}

/// One way a text reads, spelled by [`crate::romaji::canonical`].
#[derive(Debug, Deserialize, Serialize)]
pub struct Key {
    pub(crate) spelled: Box<str>,
    /// `spelled` without voicing, and just as long. Only on a dictionary's
    /// reading, whose voicing may be wrong; what is written out means what it
    /// says, as `pop` is not Bob.
    pub(crate) unvoiced: Option<Box<str>>,
    /// Where each word starts in both, in bytes.
    pub(crate) starts: Box<[u32]>,
}

impl Key {
    /// `keys` in the form to keep them in.
    pub fn encode(keys: &[Self]) -> Result<String, serde_json::Error> {
        serde_json::to_string(keys)
    }

    /// What [`Self::encode`] made.
    pub fn decode(encoded: &str) -> Result<Vec<Self>, serde_json::Error> {
        serde_json::from_str(encoded)
    }

    /// The key of `reading`, whose words a space or anything unreadable splits.
    /// `None` when nothing in it can be read.
    pub(crate) fn new(reading: &str) -> Option<Self> {
        let mut spelled = String::with_capacity(reading.len());
        let mut starts = Vec::new();
        for word in reading.split(|c| !is_readable(c)) {
            let word = spelling(word);
            if !word.is_empty() {
                starts.push(u32::try_from(spelled.len()).unwrap_or(u32::MAX));
                spelled.push_str(&word);
            }
        }
        (!spelled.is_empty()).then(|| Self {
            unvoiced: None,
            spelled: spelled.into(),
            starts: starts.into(),
        })
    }

    /// The key of what a dictionary says `reading` is.
    fn read(reading: &str) -> Option<Self> {
        let mut key = Self::new(reading)?;
        key.unvoiced = Some(unvoiced(&key.spelled).into());
        Some(key)
    }
}

/// A text ready to be matched: as written, and as it reads.
#[derive(Debug)]
pub(crate) struct Searchable {
    pub(crate) text: Box<str>,
    /// Where each word starts in `text`, in bytes.
    pub(crate) starts: Box<[u32]>,
    /// How long the text as written is, in characters.
    pub(crate) len: u32,
    pub(crate) keys: Box<[Key]>,
    /// Whether any of it is Japanese, which its keys then read rather than just
    /// respell.
    pub(crate) has_japanese: bool,
}

impl Searchable {
    pub(crate) fn new(text: &str, speller: Option<&dyn Speller>) -> Self {
        let keys = speller
            .filter(|_| needs_reading(text))
            .map_or_else(|| keys_of(text, &[]), |speller| speller.keys(text));
        let normalized = normalized(text);
        let starts = word_starts(&normalized);
        Self {
            len: u32::try_from(text.chars().count()).unwrap_or(u32::MAX),
            has_japanese: has_japanese(&normalized),
            text: normalized.into(),
            starts: starts.into(),
            keys: keys.into(),
        }
    }
}

/// The ways `text` is matched, given how a dictionary says it reads.
pub fn keys_of(text: &str, readings: &[String]) -> Vec<Key> {
    let normalized = normalized(text);
    let mut keys: Vec<Key> = Vec::new();
    let mut add = |key: Option<Key>| {
        if let Some(key) = key
            && !keys.iter().any(|known| known.spelled == key.spelled)
        {
            keys.push(key);
        }
    };
    // Spelled the way it is written, which needs no dictionary and holds for
    // Latin letters and kana whatever a dictionary makes of them. With kanji
    // left out it is no reading at all, so only the lack of one excuses it.
    if readings.is_empty() || !normalized.chars().any(is_kanji) {
        add(Key::new(&normalized));
    }
    for reading in readings {
        let reading = normalized_reading(reading);
        add(Key::read(&reading));
        if let Some(spoken) = spoken_particles(&reading) {
            add(Key::read(&spoken));
        }
    }
    keys
}

/// A dictionary's reading in the form [`Key::new`] takes, keeping its spaces.
fn normalized_reading(reading: &str) -> String {
    reading
        .split(' ')
        .map(normalized)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `reading` with the particles は and へ as they are said, or `None` when it
/// has neither. Dictionaries spell them as written; people type either.
fn spoken_particles(reading: &str) -> Option<String> {
    let words: Vec<&str> = reading.split(' ').collect();
    words
        .iter()
        .any(|word| matches!(*word, "ハ" | "ヘ"))
        .then(|| {
            words
                .iter()
                .map(|word| match *word {
                    "ハ" => "ワ",
                    "ヘ" => "エ",
                    word => word,
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
}

/// Whether an [`Analyzer`] gets asked how `text` reads. The rest reads as written.
pub fn needs_reading(text: &str) -> bool {
    has_japanese(&normalized(text))
}

/// Whether a dictionary can say more of `normalized` than its letters do.
fn has_japanese(normalized: &str) -> bool {
    normalized.chars().any(|c| is_kana(c) || is_kanji(c))
}

pub(crate) fn is_kanji(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{20000}'..='\u{3ffff}' | '々')
}

/// Where a word starts in `text`, in bytes: after anything that is not part of
/// a word. Japanese has nothing to tell its words apart by here; only readings
/// do that.
fn word_starts(text: &str) -> Vec<u32> {
    let mut starts = Vec::new();
    let mut in_word = false;
    for (ix, c) in text.char_indices() {
        let is_word = c.is_alphanumeric() || c == 'ー';
        if is_word && !in_word {
            starts.push(u32::try_from(ix).unwrap_or(u32::MAX));
        }
        in_word = is_word;
    }
    starts
}

#[cfg(test)]
mod tests {
    use crate::Analyzer;

    use super::*;

    struct Fixed(&'static [&'static str]);

    impl Analyzer for Fixed {
        fn readings(&self, _: &str) -> Vec<String> {
            self.0.iter().map(|reading| (*reading).to_owned()).collect()
        }
    }

    fn spellings(searchable: &Searchable) -> Vec<&str> {
        searchable.keys.iter().map(|key| &*key.spelled).collect()
    }

    #[test]
    fn widths_cases_and_kana_compare_as_one() {
        assert_eq!(normalized("ＡＢＣ"), "abc");
        assert_eq!(normalized("ｶﾞﾗｽ"), "ガラス");
        assert_eq!(normalized("がらす"), "ガラス");
        assert_eq!(normalized("Blue"), "blue");
    }

    #[test]
    fn latin_letters_and_kana_read_without_a_dictionary() {
        let found = Searchable::new("ほほえみDiary", None);
        assert_eq!(spellings(&found), ["hohoemiziary"]);
        // Kanji says nothing without one, and splits the words around it.
        let found = Searchable::new("夜に駆ける", None);
        assert_eq!(spellings(&found), ["nikeru"]);
        assert_eq!(&*found.keys[0].starts, [0, 2]);
        assert!(Searchable::new("夜", None).keys.is_empty());
    }

    #[test]
    fn a_reading_keeps_its_words_and_each_reading_is_kept_once() {
        let readings = Fixed(&["ヨル ニ カケル", "ヨル ニ カケル", "ヤ ニ カケル"]);
        let found = Searchable::new("夜に駆ける", Some(&readings));
        assert_eq!(spellings(&found), ["yorunikakeru", "yanikakeru"]);
    }

    #[test]
    fn particles_read_both_as_written_and_as_said() {
        let found = Searchable::new("君は", Some(&Fixed(&["キミ ハ"])));
        assert_eq!(spellings(&found), ["kimiha", "kimiwa"]);
    }

    #[test]
    fn kana_keeps_its_own_spelling_beside_the_dictionary_s() {
        let found = Searchable::new("ねこ", Some(&Fixed(&["ニャンコ"])));
        assert_eq!(spellings(&found), ["neko", "nyanko"]);
    }

    #[test]
    fn kanji_with_no_reading_falls_back_to_what_can_be_read() {
        let found = Searchable::new("夜に", Some(&Fixed(&[])));
        assert_eq!(spellings(&found), ["ni"]);
    }

    #[test]
    fn latin_text_never_troubles_the_dictionary() {
        struct Refuses;
        impl Analyzer for Refuses {
            fn readings(&self, text: &str) -> Vec<String> {
                panic!("asked to read {text}");
            }
        }
        Searchable::new("Kind of Blue", Some(&Refuses));
    }

    #[test]
    fn words_start_after_anything_that_is_not_part_of_one() {
        assert_eq!(word_starts("kind of blue"), [0, 5, 8]);
        assert_eq!(word_starts("(a) b"), [1, 4]);
        assert_eq!(word_starts("夜に駆ける"), [0]);
    }
}
