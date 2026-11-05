//! Fuzzy matching for lists whose text is often Japanese: a title in kanji can
//! be found by typing how it reads, in any system of romaji or in kana.
//!
//! [`Index`] holds a list's texts and finds and ranks what matches a query. A
//! query word and a text's reading are brought to one spelling first, so that
//! `shinjuku`, `sinzyuku` and `しんじゅく` are all the same word. From there a
//! match is contiguous if at all possible: plain subsequence matching, the
//! usual meaning of fuzzy, matches nearly anything in romaji, which is half
//! vowels. A subsequence must therefore be close and start on a word, and a
//! mistake is forgiven only when nothing else matches.
//!
//! How kanji reads comes from a [`Reader`], over a [`bundle`] of dictionaries
//! that the app fetches when it first runs. Without one, a text can still be
//! found as written, and by the reading of its kana and Latin letters.

pub mod bundle;
mod index;
mod matching;
mod reader;
mod romaji;
mod searchable;

pub use index::Index;
pub use reader::{Error, Reader};
pub use searchable::{Key, keys_of, needs_reading};

pub trait Analyzer {
    /// Each way `text` may read, as kana with spaces between its words. The
    /// words are where a match can start.
    fn readings(&self, text: &str) -> Vec<String>;
}

pub trait Speller {
    fn keys(&self, text: &str) -> Vec<Key>;
}

impl<A: Analyzer> Speller for A {
    fn keys(&self, text: &str) -> Vec<Key> {
        keys_of(text, &self.readings(text))
    }
}
