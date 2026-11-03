use std::sync::Arc;

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
#[derive(Clone, Debug)]
pub struct Queue<T> {
    entries: Arc<Vec<Entry<T>>>,
    current_ix: Option<usize>,
    repeat: RepeatMode,
    next_id: u64,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            entries: Arc::default(),
            current_ix: None,
            repeat: RepeatMode::default(),
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

    /// The entry that follows the current one when it ends on its own.
    pub fn following_ix(&self) -> Option<usize> {
        let ix = self.current_ix?;
        match self.repeat {
            RepeatMode::One => Some(ix),
            RepeatMode::All => Some((ix + 1) % self.entries.len()),
            RepeatMode::Off => (ix + 1 < self.entries.len()).then_some(ix + 1),
        }
    }

    /// Moves to the next entry; at the end without repeat, playback stops.
    ///
    /// An explicit skip leaves a repeated single song, unlike [`Self::following_ix`].
    pub fn advance(&mut self) -> Option<&T> {
        let ix = self.current_ix?;
        self.current_ix = match self.repeat {
            RepeatMode::Off => (ix + 1 < self.entries.len()).then_some(ix + 1),
            RepeatMode::All | RepeatMode::One => Some((ix + 1) % self.entries.len()),
        };
        self.current()
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
    /// Replaces the queue and starts at `start_ix`, clamped to the last song.
    pub fn replace(&mut self, items: Vec<T>, start_ix: usize) {
        let entries: Vec<_> = items.into_iter().map(|item| self.entry(item)).collect();
        self.current_ix = (!entries.is_empty()).then(|| start_ix.min(entries.len() - 1));
        self.entries = Arc::new(entries);
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

    #[test]
    fn a_snapshot_is_unaffected_by_later_changes() {
        let mut queue = queue(&["a", "b"], 0);
        let snapshot = queue.snapshot();
        queue.remove(id_of(&queue, 1));
        assert_eq!(snapshot.len(), 2);
        assert_eq!(queue.len(), 1);
    }
}
