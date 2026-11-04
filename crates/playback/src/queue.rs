use std::collections::HashMap;
use std::sync::Arc;

use rand::seq::SliceRandom as _;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Where [`Queue::insert`] puts new entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Right after the current entry.
    Next,
    /// After every other entry.
    Last,
}

/// Identifies one entry for as long as it stays in its queue, wherever it
/// moves. The same song added twice is two entries with two IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId(u64);

impl EntryId {
    /// A number unique among the queue's entries, for keys such as element IDs.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// One place in a [`Queue`].
#[derive(Clone, Debug)]
pub struct Entry<T> {
    id: EntryId,
    item: T,
}

impl<T> Entry<T> {
    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn item(&self) -> &T {
        &self.item
    }
}

/// Ordered entries and the current position in them.
///
/// The same song may appear more than once, so the current entry is a
/// position, not a song ID. `T` is whatever the owner needs per entry: an ID
/// for the CLI, a full `Song` for a view that shows titles.
///
/// Every change keeps the current position on the same entry. Entries before
/// it have been played; with no current entry, playback has run off the end
/// and every entry has.
///
/// Shuffling changes the order of the entries themselves, so what they show is
/// what plays, and moving through them works the same either way.
#[derive(Clone, Debug)]
pub struct Queue<T> {
    entries: Arc<Vec<Entry<T>>>,
    current_ix: Option<usize>,
    repeat: RepeatMode,
    /// While shuffled, every entry in the order to go back to.
    unshuffled: Option<Vec<EntryId>>,
    next_id: u64,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            entries: Arc::default(),
            current_ix: None,
            repeat: RepeatMode::default(),
            unshuffled: None,
            next_id: 0,
        }
    }
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[Entry<T>] {
        &self.entries
    }

    /// The entries as they are now, shared rather than copied: the queue
    /// copies them only when it next changes while this is still held.
    pub fn snapshot(&self) -> Arc<Vec<Entry<T>>> {
        self.entries.clone()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn current_ix(&self) -> Option<usize> {
        self.current_ix
    }

    pub fn current(&self) -> Option<&T> {
        self.current_ix.map(|ix| &self.entries[ix].item)
    }

    pub fn current_id(&self) -> Option<EntryId> {
        self.current_ix.map(|ix| self.entries[ix].id)
    }

    /// Where the entry is now.
    pub fn position(&self, id: EntryId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id == id)
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    pub fn is_shuffled(&self) -> bool {
        self.unshuffled.is_some()
    }

    /// The entry that follows the current one when it ends on its own.
    pub fn following_ix(&self) -> Option<usize> {
        let ix = self.current_ix?;
        match self.repeat {
            RepeatMode::One => Some(ix),
            RepeatMode::All => Some((ix + 1) % self.entries.len()),
            RepeatMode::Off => (ix + 1 < self.entries.len()).then_some(ix + 1),
        }
    }

    /// The item of the entry at [`Self::following_ix`].
    pub fn following(&self) -> Option<&T> {
        self.following_ix().map(|ix| &self.entries[ix].item)
    }

    /// The ID of the entry at [`Self::following_ix`].
    pub fn following_id(&self) -> Option<EntryId> {
        self.following_ix().map(|ix| self.entries[ix].id)
    }

    /// Moves to the previous entry, staying on the first one.
    pub fn retreat(&mut self) -> Option<&T> {
        let ix = self.current_ix?;
        self.current_ix = Some(match (ix, self.repeat) {
            (0, RepeatMode::All) => self.entries.len() - 1,
            (0, _) => 0,
            (ix, _) => ix - 1,
        });
        self.current()
    }

    /// Makes the entry the current one.
    pub fn jump_to(&mut self, id: EntryId) -> Option<&T> {
        self.current_ix = Some(self.position(id)?);
        self.current()
    }

    fn entry(&mut self, item: T) -> Entry<T> {
        let id = EntryId(self.next_id);
        self.next_id += 1;
        Entry { id, item }
    }
}

impl<T: Clone> Queue<T> {
    /// Shuffles the entries yet to play, or puts every entry back in the order
    /// from before, keeping the current one. Back in that order, entries moved
    /// by hand in the meantime are where they were.
    pub fn set_shuffled(&mut self, shuffled: bool) {
        if shuffled == self.is_shuffled() {
            return;
        }
        if shuffled {
            self.unshuffled = Some(self.entries.iter().map(Entry::id).collect());
            self.shuffle_upcoming();
        } else if let Some(order) = self.unshuffled.take() {
            let current = self.current_id();
            let ranks: HashMap<_, _> = order.into_iter().zip(0_usize..).collect();
            Arc::make_mut(&mut self.entries).sort_by_key(|entry| ranks[&entry.id]);
            self.current_ix = current.and_then(|id| self.position(id));
        }
    }

    fn shuffle_upcoming(&mut self) {
        let from = self.current_ix.map_or(0, |ix| ix + 1);
        Arc::make_mut(&mut self.entries)[from..].shuffle(&mut rand::rng());
    }

    /// Moves to the entry at [`Self::following_ix`], for when the current one
    /// has ended on its own.
    pub fn follow(&mut self) -> Option<&T> {
        self.go_to(self.following_ix())
    }

    /// Moves to the next entry; at the end without repeat, playback stops.
    ///
    /// An explicit skip leaves a repeated single song, unlike [`Self::following_ix`].
    pub fn advance(&mut self) -> Option<&T> {
        let ix = self.current_ix?;
        self.go_to(match self.repeat {
            RepeatMode::Off => (ix + 1 < self.entries.len()).then_some(ix + 1),
            RepeatMode::All | RepeatMode::One => Some((ix + 1) % self.entries.len()),
        })
    }

    /// Moves to the entry at `ix`. A shuffled queue starting over does so in a
    /// new order, from the entry already known to come next.
    fn go_to(&mut self, ix: Option<usize>) -> Option<&T> {
        let len = self.entries.len();
        let starts_over = len > 1 && ix == Some(0) && self.current_ix == Some(len - 1);
        self.current_ix = ix;
        if starts_over && self.is_shuffled() {
            self.shuffle_upcoming();
        }
        self.current()
    }

    /// Replaces the queue and starts at `start_ix`, clamped to the last song.
    /// While shuffled, every other song is yet to play, in any order.
    pub fn replace(&mut self, items: Vec<T>, start_ix: usize) {
        let entries: Vec<_> = items.into_iter().map(|item| self.entry(item)).collect();
        self.current_ix = (!entries.is_empty()).then(|| start_ix.min(entries.len() - 1));
        self.entries = Arc::new(entries);
        if self.is_shuffled() {
            self.unshuffled = Some(self.entries.iter().map(Entry::id).collect());
            if let Some(ix) = self.current_ix {
                Arc::make_mut(&mut self.entries).swap(0, ix);
                self.current_ix = Some(0);
            }
            self.shuffle_upcoming();
        }
    }

    /// Replaces the queue, starting with no song in particular: the first, or
    /// while shuffled any of them.
    pub fn replace_from_start(&mut self, items: Vec<T>) {
        let start_ix = if self.is_shuffled() {
            rand::random_range(0..items.len().max(1))
        } else {
            0
        };
        self.replace(items, start_ix);
    }

    /// Adds `items` in order and returns the first one's ID, if any.
    ///
    /// With no current entry, they go at the end whatever the placement,
    /// and the first of them becomes current.
    pub fn insert(&mut self, items: Vec<T>, placement: Placement) -> Option<EntryId> {
        let new: Vec<_> = items.into_iter().map(|item| self.entry(item)).collect();
        let first = new.first()?.id;
        let at = match (self.current_ix, placement) {
            (Some(ix), Placement::Next) => ix + 1,
            (Some(_), Placement::Last) | (None, _) => self.entries.len(),
        };
        let current = self.current_id();
        if let Some(order) = &mut self.unshuffled {
            let unshuffled_at = match placement {
                Placement::Next => current
                    .and_then(|current| order.iter().position(|id| *id == current))
                    .map_or(order.len(), |ix| ix + 1),
                Placement::Last => order.len(),
            };
            order.splice(unshuffled_at..unshuffled_at, new.iter().map(Entry::id));
        }
        if self.current_ix.is_none() {
            self.current_ix = Some(at);
        }
        Arc::make_mut(&mut self.entries).splice(at..at, new);
        Some(first)
    }

    /// Removes the entry, unless it is the current one: skip past that
    /// instead. Returns whether it was removed.
    pub fn remove(&mut self, id: EntryId) -> bool {
        let Some(ix) = self.position(id).filter(|ix| Some(*ix) != self.current_ix) else {
            return false;
        };
        Arc::make_mut(&mut self.entries).remove(ix);
        if let Some(order) = &mut self.unshuffled {
            order.retain(|other| *other != id);
        }
        if let Some(current) = &mut self.current_ix
            && ix < *current
        {
            *current -= 1;
        }
        true
    }

    /// Moves the entry so that it ends up at `to_ix`, clamped to the last
    /// position. Returns whether it moved.
    pub fn move_entry(&mut self, id: EntryId, to_ix: usize) -> bool {
        let Some(from) = self.position(id) else {
            return false;
        };
        let to = to_ix.min(self.entries.len() - 1);
        if from == to {
            return false;
        }
        let entries = Arc::make_mut(&mut self.entries);
        let entry = entries.remove(from);
        entries.insert(to, entry);
        self.current_ix = self.current_ix.map(|current| match current {
            current if current == from => to,
            current if from < current && current <= to => current - 1,
            current if to <= current && current < from => current + 1,
            current => current,
        });
        true
    }

    /// Moves the entry to right after the current one. Returns whether it
    /// moved; the current entry itself, or any entry when none is current,
    /// stays where it is.
    pub fn move_to_next(&mut self, id: EntryId) -> bool {
        let (Some(from), Some(current)) = (self.position(id), self.current_ix) else {
            return false;
        };
        match from.cmp(&current) {
            std::cmp::Ordering::Equal => false,
            // Once it is out from before the current entry, that one is a
            // place earlier.
            std::cmp::Ordering::Less => self.move_entry(id, current),
            std::cmp::Ordering::Greater => self.move_entry(id, current + 1),
        }
    }

    /// Removes every entry but the current one; with none current, all of them.
    pub fn clear(&mut self) {
        let kept: Vec<_> = self
            .current_ix
            .map(|ix| self.entries[ix].clone())
            .into_iter()
            .collect();
        self.current_ix = (!kept.is_empty()).then_some(0);
        if self.is_shuffled() {
            self.unshuffled = Some(kept.iter().map(Entry::id).collect());
        }
        self.entries = Arc::new(kept);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(ids: &[&'static str], start_ix: usize) -> Queue<&'static str> {
        let mut queue = Queue::new();
        queue.replace(ids.to_vec(), start_ix);
        queue
    }

    fn items(queue: &Queue<&'static str>) -> Vec<&'static str> {
        queue.entries().iter().map(|entry| *entry.item()).collect()
    }

    fn id_of(queue: &Queue<&'static str>, ix: usize) -> EntryId {
        queue.entries()[ix].id()
    }

    #[test]
    fn replace_clamps_start_and_handles_empty() {
        assert_eq!(queue(&["a", "b"], 9).current_ix(), Some(1));
        assert_eq!(queue(&[], 0).current(), None);
    }

    #[test]
    fn advance_stops_at_end_without_repeat() {
        let mut queue = queue(&["a", "b"], 0);
        assert_eq!(queue.advance().copied(), Some("b"));
        assert_eq!(queue.advance(), None);
        assert_eq!(queue.current_ix(), None);
    }

    #[test]
    fn repeat_all_wraps_both_ways() {
        let mut queue = queue(&["a", "b"], 1);
        queue.set_repeat(RepeatMode::All);
        assert_eq!(queue.following_ix(), Some(0));
        assert_eq!(queue.advance().copied(), Some("a"));
        assert_eq!(queue.retreat().copied(), Some("b"));
    }

    #[test]
    fn repeat_one_replays_on_end_but_skip_moves_on() {
        let mut queue = queue(&["a", "b"], 0);
        queue.set_repeat(RepeatMode::One);
        assert_eq!(queue.following_ix(), Some(0));
        assert_eq!(queue.advance().copied(), Some("b"));
    }

    #[test]
    fn following_goes_where_the_end_of_a_song_leads() {
        let mut queue = queue(&["a", "b"], 0);
        assert_eq!(queue.following_id(), Some(id_of(&queue, 1)));
        assert_eq!(queue.follow().copied(), Some("b"));
        assert_eq!(queue.following_id(), None);
        assert_eq!(queue.follow(), None);
        assert_eq!(queue.current_ix(), None);
        assert_eq!(queue.following_id(), None);

        let mut queue = self::queue(&["a", "b"], 1);
        queue.set_repeat(RepeatMode::One);
        assert_eq!(queue.following_id(), Some(id_of(&queue, 1)));
        assert_eq!(queue.follow().copied(), Some("b"));
        queue.set_repeat(RepeatMode::All);
        assert_eq!(queue.follow().copied(), Some("a"));
    }

    #[test]
    fn duplicate_songs_are_distinct_entries() {
        let mut queue = queue(&["a", "a"], 0);
        assert_ne!(id_of(&queue, 0), id_of(&queue, 1));
        queue.advance();
        assert_eq!(queue.current_ix(), Some(1));
    }

    #[test]
    fn ids_stay_unique_across_replacements() {
        let mut queue = queue(&["a"], 0);
        let old = id_of(&queue, 0);
        queue.replace(vec!["a"], 0);
        assert_ne!(id_of(&queue, 0), old);
        assert_eq!(queue.position(old), None);
    }

    #[test]
    fn insert_next_goes_after_the_current_entry() {
        let mut queue = queue(&["a", "b", "c"], 1);
        let first = queue.insert(vec!["x", "y"], Placement::Next);
        assert_eq!(items(&queue), ["a", "b", "x", "y", "c"]);
        assert_eq!(first, Some(id_of(&queue, 2)));
        assert_eq!(queue.current().copied(), Some("b"));
    }

    #[test]
    fn insert_last_goes_at_the_end() {
        let mut queue = queue(&["a", "b"], 0);
        queue.insert(vec!["x"], Placement::Last);
        assert_eq!(items(&queue), ["a", "b", "x"]);
        assert_eq!(queue.current().copied(), Some("a"));
    }

    #[test]
    fn insert_without_a_current_entry_appends_and_makes_it_current() {
        let mut empty = Queue::new();
        empty.insert(vec!["x", "y"], Placement::Next);
        assert_eq!(empty.current().copied(), Some("x"));

        let mut ended = queue(&["a"], 0);
        ended.advance();
        ended.insert(vec!["x"], Placement::Next);
        assert_eq!(items(&ended), ["a", "x"]);
        assert_eq!(ended.current().copied(), Some("x"));
    }

    #[test]
    fn songs_already_waiting_come_before_songs_added_last() {
        // How the player starts again after the current song failed.
        let mut queue = queue(&["a", "b", "c"], 0);
        queue.insert(vec!["x"], Placement::Last);
        assert_eq!(queue.advance().copied(), Some("b"));

        let mut queue = self::queue(&["a", "b", "c"], 0);
        queue.insert(vec!["x"], Placement::Next);
        assert_eq!(queue.advance().copied(), Some("x"));
    }

    #[test]
    fn insert_next_on_the_last_entry_goes_at_the_end() {
        let mut queue = queue(&["a", "b"], 1);
        queue.insert(vec!["x"], Placement::Next);
        assert_eq!(items(&queue), ["a", "b", "x"]);
        assert_eq!(queue.current().copied(), Some("b"));
    }

    #[test]
    fn without_a_current_entry_any_entry_can_be_removed_or_moved_but_not_put_next() {
        let mut queue = queue(&["a", "b", "c"], 2);
        queue.advance();
        assert!(!queue.move_to_next(id_of(&queue, 0)));
        assert!(queue.move_entry(id_of(&queue, 0), 2));
        assert_eq!(items(&queue), ["b", "c", "a"]);
        assert!(queue.remove(id_of(&queue, 0)));
        assert_eq!(items(&queue), ["c", "a"]);
        assert_eq!(queue.current(), None);
    }

    #[test]
    fn inserting_nothing_changes_nothing() {
        let mut queue = Queue::<&str>::new();
        assert_eq!(queue.insert(Vec::new(), Placement::Last), None);
        assert_eq!(queue.current(), None);
    }

    #[test]
    fn remove_keeps_the_current_entry_and_refuses_it() {
        let mut queue = queue(&["a", "b", "c"], 1);
        assert!(!queue.remove(id_of(&queue, 1)));
        assert!(queue.remove(id_of(&queue, 0)));
        assert_eq!(items(&queue), ["b", "c"]);
        assert_eq!(queue.current().copied(), Some("b"));
        assert!(queue.remove(id_of(&queue, 1)));
        assert_eq!(queue.current_ix(), Some(0));
    }

    #[test]
    fn move_entry_keeps_the_current_entry() {
        // Moving the current entry itself.
        let mut queue = queue(&["a", "b", "c", "d"], 1);
        assert!(queue.move_entry(id_of(&queue, 1), 3));
        assert_eq!(items(&queue), ["a", "c", "d", "b"]);
        assert_eq!(queue.current().copied(), Some("b"));

        // An entry from before it to after it.
        let mut queue = self::queue(&["a", "b", "c", "d"], 1);
        assert!(queue.move_entry(id_of(&queue, 0), 2));
        assert_eq!(items(&queue), ["b", "c", "a", "d"]);
        assert_eq!(queue.current().copied(), Some("b"));

        // An entry from after it to before it.
        let mut queue = self::queue(&["a", "b", "c", "d"], 1);
        assert!(queue.move_entry(id_of(&queue, 3), 0));
        assert_eq!(items(&queue), ["d", "a", "b", "c"]);
        assert_eq!(queue.current().copied(), Some("b"));

        // Past the end, and nowhere.
        let mut queue = self::queue(&["a", "b"], 0);
        assert!(queue.move_entry(id_of(&queue, 0), 9));
        assert_eq!(items(&queue), ["b", "a"]);
        assert!(!queue.move_entry(id_of(&queue, 1), 1));
    }

    #[test]
    fn move_to_next_puts_the_entry_after_the_current_one() {
        let mut queue = queue(&["a", "b", "c", "d"], 1);
        assert!(queue.move_to_next(id_of(&queue, 3)));
        assert_eq!(items(&queue), ["a", "b", "d", "c"]);
        assert!(queue.move_to_next(id_of(&queue, 0)));
        assert_eq!(items(&queue), ["b", "a", "d", "c"]);
        assert_eq!(queue.current().copied(), Some("b"));
        assert!(!queue.move_to_next(id_of(&queue, 0)));
    }

    #[test]
    fn clear_keeps_only_the_current_entry() {
        let mut queue = queue(&["a", "b", "c"], 1);
        let current = queue.current_id();
        queue.clear();
        assert_eq!(items(&queue), ["b"]);
        assert_eq!(queue.current_id(), current);

        queue.advance();
        queue.clear();
        assert!(queue.is_empty());
    }

    #[test]
    fn jump_to_makes_an_entry_current() {
        let mut queue = queue(&["a", "b", "c"], 0);
        assert_eq!(queue.jump_to(id_of(&queue, 2)).copied(), Some("c"));
        let gone = id_of(&queue, 0);
        queue.remove(gone);
        assert_eq!(queue.jump_to(gone), None);
        assert_eq!(queue.current().copied(), Some("c"));
    }

    /// As many items that a shuffle leaving them in order would take a miracle.
    const MANY: [&str; 40] = [
        "00", "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14",
        "15", "16", "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29",
        "30", "31", "32", "33", "34", "35", "36", "37", "38", "39",
    ];

    fn sorted(mut items: Vec<&'static str>) -> Vec<&'static str> {
        items.sort_unstable();
        items
    }

    #[test]
    fn shuffling_leaves_what_has_played_and_mixes_what_has_not() {
        let mut queue = queue(&MANY, 10);
        queue.set_shuffled(true);
        assert!(queue.is_shuffled());
        let shuffled = items(&queue);
        assert_eq!(shuffled[..=10], MANY[..=10]);
        assert_eq!(queue.current().copied(), Some("10"));
        assert_ne!(shuffled[11..], MANY[11..]);
        assert_eq!(sorted(shuffled), MANY);
    }

    #[test]
    fn shuffling_off_goes_back_to_the_order_and_keeps_the_current_entry() {
        let mut queue = queue(&MANY, 10);
        queue.set_shuffled(true);
        queue.advance();
        let current = queue.current().copied();
        queue.set_shuffled(false);
        assert!(!queue.is_shuffled());
        assert_eq!(items(&queue), MANY);
        assert_eq!(queue.current().copied(), current);
    }

    #[test]
    fn shuffling_again_changes_nothing() {
        let mut queue = queue(&MANY, 0);
        queue.set_shuffled(true);
        let shuffled = items(&queue);
        queue.set_shuffled(true);
        assert_eq!(items(&queue), shuffled);
        queue.set_shuffled(false);
        queue.set_shuffled(false);
        assert_eq!(items(&queue), MANY);
    }

    #[test]
    fn shuffling_with_nothing_current_mixes_everything() {
        let mut queue = queue(&MANY, 39);
        queue.advance();
        queue.set_shuffled(true);
        assert_eq!(queue.current(), None);
        assert_ne!(items(&queue), MANY);
        queue.set_shuffled(false);
        assert_eq!(items(&queue), MANY);
        assert_eq!(queue.current(), None);
    }

    #[test]
    fn a_new_queue_while_shuffled_starts_at_its_song_with_all_the_others_to_come() {
        let mut queue = Queue::new();
        queue.set_shuffled(true);
        queue.replace(MANY.to_vec(), 20);
        assert_eq!(queue.current_ix(), Some(0));
        assert_eq!(queue.current().copied(), Some("20"));
        assert_eq!(sorted(items(&queue)), MANY);
        assert_ne!(items(&queue)[1..], MANY[1..]);

        queue.set_shuffled(false);
        assert_eq!(items(&queue), MANY);
        assert_eq!(queue.current_ix(), Some(20));

        let mut empty = Queue::<&str>::new();
        empty.set_shuffled(true);
        empty.replace(Vec::new(), 0);
        assert_eq!(empty.current(), None);
    }

    #[test]
    fn replacing_from_the_start_takes_the_first_song_unless_shuffled() {
        let mut queue = Queue::new();
        queue.replace_from_start(MANY.to_vec());
        assert_eq!(queue.current().copied(), Some("00"));
        assert_eq!(items(&queue), MANY);

        queue.set_shuffled(true);
        let starts_elsewhere = (0..1_000).any(|_| {
            queue.replace_from_start(MANY.to_vec());
            queue.current().copied() != Some("00")
        });
        assert!(starts_elsewhere);
        assert_eq!(queue.current_ix(), Some(0));

        queue.replace_from_start(Vec::new());
        assert_eq!(queue.current(), None);
    }

    #[test]
    fn songs_added_while_shuffled_keep_their_order_and_place() {
        let mut queue = queue(&["a", "b", "c", "d"], 1);
        queue.set_shuffled(true);
        queue.insert(vec!["x", "y"], Placement::Next);
        queue.insert(vec!["z"], Placement::Last);
        let shuffled = items(&queue);
        assert_eq!(shuffled[..4], ["a", "b", "x", "y"]);
        assert_eq!(shuffled.last(), Some(&"z"));

        queue.set_shuffled(false);
        assert_eq!(items(&queue), ["a", "b", "x", "y", "c", "d", "z"]);
        assert_eq!(queue.current().copied(), Some("b"));
    }

    #[test]
    fn songs_added_with_nothing_current_while_shuffled_go_at_the_end() {
        let mut queue = queue(&["a", "b"], 1);
        queue.set_shuffled(true);
        queue.advance();
        queue.insert(vec!["x"], Placement::Next);
        assert_eq!(queue.current().copied(), Some("x"));
        queue.set_shuffled(false);
        assert_eq!(items(&queue), ["a", "b", "x"]);
        assert_eq!(queue.current().copied(), Some("x"));
    }

    #[test]
    fn entries_taken_out_while_shuffled_stay_out() {
        let mut queue = queue(&MANY, 0);
        queue.set_shuffled(true);
        let (id, item) = (id_of(&queue, 39), *queue.entries()[39].item());
        assert!(queue.remove(id));
        queue.set_shuffled(false);
        let mut expected = MANY.to_vec();
        expected.retain(|other| *other != item);
        assert_eq!(items(&queue), expected);

        let mut queue = self::queue(&MANY, 5);
        queue.set_shuffled(true);
        queue.clear();
        queue.set_shuffled(false);
        assert_eq!(items(&queue), ["05"]);
        assert_eq!(queue.current().copied(), Some("05"));
    }

    #[test]
    fn an_entry_moved_while_shuffled_plays_there_and_goes_back_after() {
        let mut queue = queue(&MANY, 0);
        queue.set_shuffled(true);
        let moved = queue.entries()[39].id();
        assert!(queue.move_to_next(moved));
        assert_eq!(queue.following_id(), Some(moved));
        queue.set_shuffled(false);
        assert_eq!(items(&queue), MANY);
    }

    #[test]
    fn a_shuffled_queue_starts_over_from_what_was_to_follow_in_a_new_order() {
        for skip in [false, true] {
            let mut queue = queue(&MANY, 0);
            queue.set_repeat(RepeatMode::All);
            queue.set_shuffled(true);
            queue.jump_to(queue.entries()[39].id());
            let round = items(&queue);
            let following = queue.following_id();
            if skip {
                queue.advance();
            } else {
                queue.follow();
            }
            assert_eq!(queue.current_ix(), Some(0));
            assert_eq!(queue.current_id(), following);
            let next_round = items(&queue);
            assert_eq!(next_round[0], round[0]);
            assert_ne!(next_round[1..], round[1..]);
            assert_eq!(sorted(next_round), MANY);
        }
    }

    #[test]
    fn a_queue_stays_in_its_order_unless_it_starts_over_shuffled() {
        // Not shuffled.
        let mut unshuffled = queue(&MANY, 39);
        unshuffled.set_repeat(RepeatMode::All);
        unshuffled.follow();
        assert_eq!(items(&unshuffled), MANY);

        // A song on repeat, and a step through the middle.
        let mut queue = queue(&MANY, 39);
        queue.set_repeat(RepeatMode::One);
        queue.set_shuffled(true);
        let order = items(&queue);
        queue.follow();
        assert_eq!(items(&queue), order);
        queue.retreat();
        queue.advance();
        assert_eq!(items(&queue), order);
    }

    #[test]
    fn a_snapshot_is_unaffected_by_later_changes() {
        let mut queue = queue(&["a", "b"], 0);
        let snapshot = queue.snapshot();
        queue.remove(id_of(&queue, 1));
        assert_eq!(snapshot.len(), 2);
        assert_eq!(queue.len(), 1);
    }
}
