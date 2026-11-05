use std::borrow::Cow;
use std::cmp::Ordering;
use std::fs;
use std::path::Path;

use lindera::dictionary::load_dictionary_with_options;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;

use crate::Analyzer;
use crate::searchable::is_kanji;

/// The field of both dictionaries that says how a word reads.
const READING: &str = "reading";
/// The dictionaries a bundle holds, in the order they are asked.
const DICTIONARIES: [&str; 2] = ["ipadic", "sudachi"];
/// The table of people's names in a bundle.
const NAMES: &str = "names.tsv";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read {path}: {source}")]
    Dictionary {
        path: String,
        #[source]
        source: lindera::error::LinderaError,
    },
    #[error("could not read the names at {path}: {source}")]
    Names {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Works out how Japanese text reads, from a bundle installed by
/// [`crate::bundle::install`].
///
/// Morphological analysis alone reads titles well but misreads names, and a
/// dictionary of names misreads titles, so a text keeps every reading they
/// give. Opening one is quick and maps the dictionaries rather than reading
/// them. Those it touches stay in memory until it is dropped, so drop it once
/// an [`crate::Index`] is built.
pub struct Reader {
    segmenters: Vec<Segmenter>,
    names: Names,
}

impl Reader {
    /// Raise when a text would read or be spelled differently from the same
    /// bundle: both are saved, and this says which ones still hold.
    pub const VERSION: u32 = 2;

    /// The bundle in `dir`. Fails for one written by another version of the
    /// dictionary format.
    pub fn open(dir: &Path) -> Result<Self, Error> {
        let segmenters = DICTIONARIES
            .iter()
            .map(|name| {
                let path = dir.join(name).to_string_lossy().into_owned();
                let dictionary = load_dictionary_with_options(&path, true)
                    .map_err(|source| Error::Dictionary { path, source })?;
                Ok(Segmenter::new(Mode::Normal, dictionary, None))
            })
            .collect::<Result<_, Error>>()?;
        let path = dir.join(NAMES);
        let names = fs::read(&path).map_err(|source| Error::Names {
            path: path.display().to_string(),
            source,
        })?;
        Ok(Self {
            segmenters,
            names: Names(names),
        })
    }

    fn analyzed(segmenter: &Segmenter, text: &str) -> Option<String> {
        let mut tokens = segmenter.segment(Cow::Borrowed(text)).ok()?;
        let mut reading = String::with_capacity(text.len() * 2);
        for token in &mut tokens {
            if !reading.is_empty() {
                reading.push(' ');
            }
            let surface = token.surface.clone();
            match token.get(READING).filter(|read| !matches!(*read, "" | "*")) {
                Some(read) => reading.push_str(read),
                None => reading.push_str(&surface),
            }
        }
        Some(reading)
    }
}

impl Analyzer for Reader {
    fn readings(&self, text: &str) -> Vec<String> {
        let mut readings: Vec<String> = self
            .segmenters
            .iter()
            .filter_map(|segmenter| Self::analyzed(segmenter, text))
            .collect();
        readings.extend(
            names_in(text)
                .filter_map(|name| self.names.reading(name))
                .map(str::to_owned),
        );
        readings
    }
}

/// The parts of `text` that may be someone's name: all of it, and each name of
/// a list like `A、B feat. C`. Only those with kanji, whose reading is in doubt.
fn names_in(text: &str) -> impl Iterator<Item = &str> {
    let parts = text
        .split(|c: char| {
            matches!(
                c,
                '、' | ','
                    | '，'
                    | '・'
                    | '/'
                    | '／'
                    | '&'
                    | '＆'
                    | '×'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '［'
                    | '］'
            )
        })
        .flat_map(|part| {
            part.split_whitespace().filter(|word| {
                !word.eq_ignore_ascii_case("feat.")
                    && !word.eq_ignore_ascii_case("feat")
                    && !word.eq_ignore_ascii_case("ft.")
                    && !word.eq_ignore_ascii_case("cv")
                    && !word.eq_ignore_ascii_case("cv.")
            })
        });
    std::iter::once(text.trim())
        .chain(parts)
        .filter(|part| part.chars().any(is_kanji))
}

/// People's names and how they read: a line each of `name<TAB>reading`, in
/// order of their bytes, searched as it lies rather than loaded into a map.
struct Names(Vec<u8>);

impl Names {
    fn reading(&self, name: &str) -> Option<&str> {
        let bytes = &self.0;
        let (mut low, mut high) = (0, bytes.len());
        while low < high {
            let middle = low + (high - low) / 2;
            let start = bytes[..middle]
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |newline| newline + 1);
            let end = bytes[start..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |len| start + len);
            let line = &bytes[start..end];
            let tab = line.iter().position(|byte| *byte == b'\t')?;
            match line[..tab].cmp(name.as_bytes()) {
                Ordering::Equal => return std::str::from_utf8(&line[tab + 1..]).ok(),
                Ordering::Less => low = end + 1,
                Ordering::Greater => high = start,
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Index;

    #[test]
    fn names_are_found_wherever_a_line_falls() {
        let names = Names(b"a\t1\nbb\t22\nccc\t333\nd\t4\n".to_vec());
        assert_eq!(names.reading("a"), Some("1"));
        assert_eq!(names.reading("bb"), Some("22"));
        assert_eq!(names.reading("ccc"), Some("333"));
        assert_eq!(names.reading("d"), Some("4"));
        assert_eq!(names.reading("b"), None);
        assert_eq!(names.reading("cc"), None);
        assert_eq!(names.reading("e"), None);
        assert_eq!(Names(Vec::new()).reading("a"), None);
    }

    #[test]
    fn names_are_found_in_lists_of_them() {
        let found: Vec<_> = names_in("花澤香菜、竹達彩奈 feat. 伊藤美来").collect();
        assert_eq!(
            found,
            [
                "花澤香菜、竹達彩奈 feat. 伊藤美来",
                "花澤香菜",
                "竹達彩奈",
                "伊藤美来"
            ]
        );
        assert_eq!(names_in("Miles Davis").count(), 0);
    }

    /// Against a real bundle, which is far too large to keep with the code.
    /// `HIPHILE_READINGS_DIR=… cargo test -p fuzzy -- --ignored`
    #[test]
    #[ignore = "needs a bundle, at HIPHILE_READINGS_DIR"]
    fn a_real_bundle_finds_kanji_by_romaji() {
        let dir = std::env::var("HIPHILE_READINGS_DIR").unwrap();
        let reader = Reader::open(Path::new(&dir)).unwrap();
        let index = Index::new(
            [
                ["夜に駆ける", "YOASOBI"],
                ["迷える羊", "米津玄師"],
                ["残酷な天使のテーゼ", "高橋洋子"],
                ["東京事変", "椎名林檎"],
            ],
            Some(&reader),
        );
        assert_eq!(index.search("yorunikakeru"), [0]);
        assert_eq!(index.search("yoru ni kakeru"), [0]);
        assert_eq!(index.search("yonezu"), [1]);
        assert_eq!(index.search("zankoku"), [2]);
        assert_eq!(index.search("shiinaringo"), [3]);
        assert_eq!(index.search("sinaringo"), [3]);
        assert_eq!(index.search("tokyo"), [3]);
    }
}
