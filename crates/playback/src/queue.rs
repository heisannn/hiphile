#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Ordered entries and the current position in them.
///
/// The same song may appear more than once, so the current entry is a
/// position, not a song ID. `T` is whatever the owner needs per entry: an ID
/// for the CLI, a full `Song` for a view that shows titles.
#[derive(Clone, Debug)]
pub struct Queue<T> {
    songs: Vec<T>,
    current_ix: Option<usize>,
    repeat: RepeatMode,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            songs: Vec::new(),
            current_ix: None,
            repeat: RepeatMode::default(),
        }
    }
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the queue and starts at `start_ix`, clamped to the last song.
    pub fn replace(&mut self, songs: Vec<T>, start_ix: usize) {
        self.current_ix = (!songs.is_empty()).then(|| start_ix.min(songs.len() - 1));
        self.songs = songs;
    }

    pub fn songs(&self) -> &[T] {
        &self.songs
    }

    pub fn current_ix(&self) -> Option<usize> {
        self.current_ix
    }

    pub fn current(&self) -> Option<&T> {
        self.current_ix.map(|ix| &self.songs[ix])
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
            RepeatMode::All => Some((ix + 1) % self.songs.len()),
            RepeatMode::Off => (ix + 1 < self.songs.len()).then_some(ix + 1),
        }
    }

    /// Moves to the next entry; at the end without repeat, playback stops.
    ///
    /// An explicit skip leaves a repeated single song, unlike [`Self::following_ix`].
    pub fn advance(&mut self) -> Option<&T> {
        let ix = self.current_ix?;
        self.current_ix = match self.repeat {
            RepeatMode::Off => (ix + 1 < self.songs.len()).then_some(ix + 1),
            RepeatMode::All | RepeatMode::One => Some((ix + 1) % self.songs.len()),
        };
        self.current()
    }

    /// Moves to the previous entry, staying on the first one.
    pub fn retreat(&mut self) -> Option<&T> {
        let ix = self.current_ix?;
        self.current_ix = Some(match (ix, self.repeat) {
            (0, RepeatMode::All) => self.songs.len() - 1,
            (0, _) => 0,
            (ix, _) => ix - 1,
        });
        self.current()
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
        queue.advance();
        assert_eq!(queue.current_ix(), Some(1));
    }
}
