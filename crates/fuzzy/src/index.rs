use std::collections::HashMap;

use crate::Speller;
use crate::matching::{Atom, Scratch, Tier, atoms, is_mistyped, mistyped_tier, tier};
use crate::searchable::Searchable;

const NO_MATCH: Tier = Tier::MAX;

/// The items of a list, ready to be searched: each one is the texts it can be
/// found by, the likeliest first, as a song's title before its artist.
///
/// Texts that many items share are matched once, so a query over a library takes
/// no longer than a pass over its distinct titles, artists and albums.
#[derive(Debug, Default)]
pub struct Index {
    texts: Vec<Searchable>,
    /// For each item, its texts as positions in `texts`.
    items: Vec<Box<[u32]>>,
}

/// How an item matched, the lesser the better.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    /// How bad the worst match among the query's words was.
    worst: Tier,
    /// How bad all of them were together.
    total: u32,
    /// How far down the item's texts they were found.
    depth: u32,
    /// How long those texts are: an artist alone beats a duet with them.
    len: u32,
}

impl Index {
    /// Reads `items` with `speller`, or as far as their kana and Latin letters
    /// go without one. Takes a while with one: call it off the UI thread.
    pub fn new<I, T, S>(items: I, speller: Option<&dyn Speller>) -> Self
    where
        I: IntoIterator<Item = T>,
        T: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut positions: HashMap<String, u32> = HashMap::new();
        let mut texts = Vec::new();
        let items = items
            .into_iter()
            .map(|item| {
                item.into_iter()
                    .map(|text| {
                        let text = text.as_ref();
                        if let Some(position) = positions.get(text) {
                            return *position;
                        }
                        let position = u32::try_from(texts.len()).unwrap_or(u32::MAX);
                        texts.push(Searchable::new(text, speller));
                        positions.insert(text.to_owned(), position);
                        position
                    })
                    .collect()
            })
            .collect();
        Self { texts, items }
    }

    /// Positions of the items that match every word of `query`, the best first
    /// and those alike in their original order. An empty query matches every
    /// item, in that order.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let atoms = atoms(query);
        if atoms.is_empty() {
            return (0..self.items.len()).collect();
        }
        let mut tiers = self.tiers_of_each(&atoms);
        let mut found = self.ranked(&tiers);
        if found.is_empty() {
            // Only now: forgiving mistakes in every query would bury what was
            // meant under what is merely close to it.
            for (atom, tiers) in atoms.iter().zip(&mut tiers) {
                if tiers.iter().all(|tier| *tier == NO_MATCH) {
                    self.forgive_mistakes(atom, tiers);
                }
            }
            found = self.ranked(&tiers);
        }
        found
    }

    /// Like [`Self::search`] without forgiving mistakes, for searching several
    /// indexes together: whether to forgive depends on what all of them find.
    pub fn search_as_spelled(&self, query: &str) -> Vec<usize> {
        let atoms = atoms(query);
        if atoms.is_empty() {
            return (0..self.items.len()).collect();
        }
        self.ranked(&self.tiers_of_each(&atoms))
    }

    fn tiers_of_each(&self, atoms: &[Atom]) -> Vec<Vec<Tier>> {
        let mut scratch = Scratch::new();
        atoms
            .iter()
            .map(|atom| self.tiers(atom, &mut scratch))
            .collect()
    }

    fn tiers(&self, atom: &Atom, scratch: &mut Scratch) -> Vec<Tier> {
        self.texts
            .iter()
            .map(|text| tier(atom, text, scratch).unwrap_or(NO_MATCH))
            .collect()
    }

    fn forgive_mistakes(&self, atom: &Atom, tiers: &mut [Tier]) {
        for (text, tier) in self.texts.iter().zip(tiers) {
            if is_mistyped(atom, text) {
                *tier = mistyped_tier();
            }
        }
    }

    fn ranked(&self, tiers: &[Vec<Tier>]) -> Vec<usize> {
        let mut found: Vec<(Rank, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(ix, item)| Some((self.rank(item, tiers)?, ix)))
            .collect();
        found.sort_unstable();
        found.into_iter().map(|(_, ix)| ix).collect()
    }

    /// How `item` matched, if every word did somewhere in it.
    fn rank(&self, item: &[u32], tiers: &[Vec<Tier>]) -> Option<Rank> {
        let mut rank = Rank {
            worst: 0,
            total: 0,
            depth: 0,
            len: 0,
        };
        for tiers in tiers {
            let (tier, depth, text) = item
                .iter()
                .enumerate()
                .map(|(depth, text)| (tiers[*text as usize], depth, *text))
                .min_by_key(|(tier, depth, _)| (*tier, *depth))?;
            if tier == NO_MATCH {
                return None;
            }
            rank.worst = rank.worst.max(tier);
            rank.total += u32::from(tier);
            rank.depth += u32::try_from(depth).unwrap_or(u32::MAX);
            rank.len += self.texts[text as usize].len;
        }
        Some(rank)
    }
}

#[cfg(test)]
mod tests {
    use crate::Analyzer;

    use super::*;

    /// Reads the texts given, and nothing else.
    struct Readings(&'static [(&'static str, &'static str)]);

    impl Analyzer for Readings {
        fn readings(&self, text: &str) -> Vec<String> {
            self.0
                .iter()
                .filter(|(known, _)| *known == text)
                .map(|(_, reading)| (*reading).to_owned())
                .collect()
        }
    }

    const READINGS: Readings = Readings(&[
        ("米津玄師", "ヨネヅケンシ"),
        ("夜に駆ける", "ヨル ニ カケル"),
        ("迷える羊", "マヨエ ル ヒツジ"),
        ("残響散歌", "ザンキョウ サンカ"),
    ]);

    fn songs() -> Index {
        Index::new(
            [
                ["Lemon", "米津玄師", "STRAY SHEEP"],
                ["迷える羊", "米津玄師", "STRAY SHEEP"],
                ["夜に駆ける", "YOASOBI", "THE BOOK"],
                ["Blue Train", "John Coltrane", "Blue Train"],
                ["Kind of Blue", "Miles Davis", "Kind of Blue"],
            ],
            Some(&READINGS),
        )
    }

    #[test]
    fn an_empty_query_keeps_every_item_in_order() {
        assert_eq!(songs().search("  "), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn romaji_finds_kanji_in_any_of_an_item_s_texts() {
        assert_eq!(songs().search("yorunikakeru"), [2]);
        assert_eq!(songs().search("mayoeru"), [1]);
        assert_eq!(songs().search("yonezu"), [0, 1]);
    }

    #[test]
    fn every_word_must_match_but_not_in_the_same_text() {
        assert_eq!(songs().search("yonezu lemon"), [0]);
        assert_eq!(songs().search("lemon yonezu"), [0]);
        assert!(songs().search("yonezu blue").is_empty());
    }

    #[test]
    fn the_better_match_comes_first_whatever_the_order() {
        // A prefix before a word further in.
        assert_eq!(songs().search("blue"), [3, 4]);
    }

    #[test]
    fn a_title_comes_before_an_artist_or_an_album_matching_as_well() {
        let index = Index::new(
            [
                ["a", "shared", "b"],
                ["c", "d", "shared"],
                ["shared", "e", "f"],
            ],
            None,
        );
        assert_eq!(index.search("shared"), [2, 0, 1]);
    }

    #[test]
    fn the_shorter_text_comes_first_among_matches_alike() {
        let index = Index::new([["Miles Davis Quintet"], ["Miles Davis"]], None);
        assert_eq!(index.search("miles"), [1, 0]);
    }

    #[test]
    fn matches_alike_keep_their_order() {
        let index = Index::new([["Same"], ["Same"], ["Same"]], None);
        assert_eq!(index.search("same"), [0, 1, 2]);
    }

    #[test]
    fn as_spelled_never_forgives_a_mistake() {
        let index = Index::new([["Lemon"]], None);
        assert_eq!(index.search("lemmon"), [0]);
        assert!(index.search_as_spelled("lemmon").is_empty());
        assert_eq!(index.search_as_spelled("lemon"), [0]);
    }

    #[test]
    fn mistakes_are_forgiven_only_when_nothing_else_matches() {
        // Nothing reads quite like this, so what is closest will do.
        assert_eq!(songs().search("yorunikaxeru"), [2]);
        // Coltrane matches, so nothing more loosely matched joins it.
        assert_eq!(songs().search("coltrane"), [3]);
    }

    #[test]
    fn a_word_that_matches_is_not_loosened_for_one_that_does_not() {
        assert_eq!(songs().search("coltrane yorunikaxeru"), Vec::<usize>::new());
    }

    #[test]
    fn as_written_comes_before_spelled_alike_and_gossip_is_not_ship() {
        let titles = ["Sip", "Gossip", "Worship", "Ship of Fools", "Ship"];
        let index = Index::new(titles.map(|title| vec![title.to_owned()]), None);
        let found: Vec<_> = index.search("ship").iter().map(|ix| titles[*ix]).collect();
        assert_eq!(found, ["Ship", "Ship of Fools", "Worship", "Sip"]);
    }

    #[test]
    fn nothing_matches_when_nothing_is_close() {
        assert!(songs().search("nothing like it").is_empty());
    }

    #[test]
    fn an_item_with_no_texts_never_matches() {
        let index = Index::new([Vec::<&str>::new(), vec!["found"]], None);
        assert_eq!(index.search("found"), [1]);
        assert_eq!(index.search(""), [0, 1]);
    }

    #[test]
    fn texts_shared_by_items_are_read_once() {
        let index = songs();
        // Five titles, four artists, and two albums not named like a title.
        assert_eq!(index.texts.len(), 5 + 4 + 2);
        assert_eq!(index.items.len(), 5);
    }
}
