use std::sync::Arc;

use subsonic::{Song, SongId};

/// A row of a playlist. A song can be in a playlist more than once, so rows
/// keep their selection and identity by this rather than by song or position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryId(u64);

impl EntryId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    id: EntryId,
    song: Song,
}

impl Entry {
    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn song(&self) -> &Song {
        &self.song
    }
}

/// The songs of the playlist on show, in order, which edits change first and
/// the server after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entries {
    /// Shared with the list's row renderer.
    entries: Arc<Vec<Entry>>,
    next_id: u64,
}

impl Entries {
    pub fn new(songs: Vec<Song>) -> Self {
        let mut this = Self {
            entries: Arc::default(),
            next_id: 0,
        };
        this.extend(songs);
        this
    }

    /// Takes `songs` in place of those listed, as new entries: none takes the id of one
    /// listed before.
    pub fn replace(&mut self, songs: Vec<Song>) {
        self.entries = Arc::default();
        self.extend(songs);
    }

    pub fn shared(&self) -> Arc<Vec<Entry>> {
        self.entries.clone()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, ix: usize) -> Option<&Entry> {
        self.entries.get(ix)
    }

    pub fn position(&self, id: EntryId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id == id)
    }

    pub fn songs(&self) -> Arc<[Song]> {
        self.entries
            .iter()
            .map(|entry| entry.song.clone())
            .collect()
    }

    pub fn song_ids(&self) -> Vec<SongId> {
        self.entries
            .iter()
            .map(|entry| entry.song.id().clone())
            .collect()
    }

    pub fn has_songs(&self, songs: &[Song]) -> bool {
        self.entries.len() == songs.len()
            && self
                .entries
                .iter()
                .zip(songs)
                .all(|(entry, song)| entry.song.id() == song.id())
    }

    pub fn extend(&mut self, songs: impl IntoIterator<Item = Song>) {
        let added: Vec<Entry> = songs.into_iter().map(|song| self.entry(song)).collect();
        Arc::make_mut(&mut self.entries).extend(added);
    }

    /// Puts the entry at position `to_ix`, or the last if past the end.
    /// Whether that moved it.
    pub fn move_entry(&mut self, id: EntryId, to_ix: usize) -> bool {
        let Some(from_ix) = self.position(id) else {
            return false;
        };
        let to_ix = to_ix.min(self.entries.len() - 1);
        if from_ix == to_ix {
            return false;
        }
        let entries = Arc::make_mut(&mut self.entries);
        let entry = entries.remove(from_ix);
        entries.insert(to_ix, entry);
        true
    }

    pub fn remove(&mut self, id: EntryId) -> Option<(usize, Song)> {
        let ix = self.position(id)?;
        let entry = Arc::make_mut(&mut self.entries).remove(ix);
        Some((ix, entry.song))
    }

    /// Puts `song` at position `ix`, or the last if past the end.
    pub fn insert(&mut self, ix: usize, song: Song) -> EntryId {
        let entry = self.entry(song);
        let id = entry.id;
        let ix = ix.min(self.entries.len());
        Arc::make_mut(&mut self.entries).insert(ix, entry);
        id
    }

    fn entry(&mut self, song: Song) -> Entry {
        let id = EntryId(self.next_id);
        self.next_id += 1;
        Entry { id, song }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn song(id: &str) -> Song {
        serde_json::from_value(json!({ "id": id, "title": id })).unwrap()
    }

    fn entries(ids: &[&str]) -> Entries {
        Entries::new(ids.iter().map(|id| song(id)).collect())
    }

    fn ids(entries: &Entries) -> Vec<String> {
        entries
            .song_ids()
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect()
    }

    fn id_at(entries: &Entries, ix: usize) -> EntryId {
        entries.get(ix).unwrap().id()
    }

    #[test]
    fn the_same_song_twice_is_two_entries() {
        let entries = entries(&["a", "a"]);
        assert_ne!(id_at(&entries, 0), id_at(&entries, 1));
    }

    #[test]
    fn moving_an_entry_lands_it_on_the_position_asked_for() {
        let mut list = entries(&["a", "b", "c"]);
        let a = id_at(&list, 0);

        assert!(list.move_entry(a, 2));
        assert_eq!(ids(&list), ["b", "c", "a"]);
        assert_eq!(list.position(a), Some(2));

        assert!(list.move_entry(a, 0));
        assert_eq!(ids(&list), ["a", "b", "c"]);
    }

    #[test]
    fn moving_an_entry_nowhere_says_so() {
        let mut list = entries(&["a", "b"]);
        let b = id_at(&list, 1);
        assert!(!list.move_entry(b, 1));
        assert!(!list.move_entry(b, 9));
        assert_eq!(ids(&list), ["a", "b"]);
    }

    #[test]
    fn removing_one_of_two_copies_leaves_the_other() {
        let mut list = entries(&["a", "b", "a"]);
        let first = id_at(&list, 0);
        let last = id_at(&list, 2);

        let (ix, removed) = list.remove(last).unwrap();

        assert_eq!(ix, 2);
        assert_eq!(removed.id().as_str(), "a");
        assert_eq!(ids(&list), ["a", "b"]);
        assert_eq!(list.position(first), Some(0));
    }

    #[test]
    fn a_removed_song_goes_back_where_it_was() {
        let mut list = entries(&["a", "b", "c"]);
        let (ix, song) = list.remove(id_at(&list, 1)).unwrap();
        list.insert(ix, song);
        assert_eq!(ids(&list), ["a", "b", "c"]);
    }

    #[test]
    fn a_song_put_back_past_the_end_goes_last() {
        let mut list = entries(&["a"]);
        list.insert(9, song("b"));
        assert_eq!(ids(&list), ["a", "b"]);
    }

    #[test]
    fn songs_put_in_place_of_others_are_new_entries() {
        let mut list = entries(&["a", "b"]);
        let a = id_at(&list, 0);
        list.replace(vec![song("a"), song("b")]);
        assert_eq!(list.position(a), None);
        assert_eq!(ids(&list), ["a", "b"]);
    }

    #[test]
    fn compares_songs_in_order() {
        let list = entries(&["a", "b"]);
        assert!(list.has_songs(&[song("a"), song("b")]));
        assert!(!list.has_songs(&[song("b"), song("a")]));
        assert!(!list.has_songs(&[song("a")]));
    }
}
