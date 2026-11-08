use subsonic::{AlbumId, ArtistId, PlaylistId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Home,
    Artists,
    Albums,
    Songs,
    Playlists,
    Favorites,
}

impl Destination {
    /// In the order the title bar shows them.
    pub const ALL: [Self; 6] = [
        Self::Home,
        Self::Artists,
        Self::Albums,
        Self::Songs,
        Self::Playlists,
        Self::Favorites,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Songs => "Songs",
            Self::Playlists => "Playlists",
            Self::Favorites => "Favorites",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    Home,
    Artists(Option<ArtistId>),
    Albums(Option<AlbumId>),
    Songs,
    Playlists(Option<PlaylistId>),
    Favorites,
}

impl Location {
    pub fn destination(&self) -> Destination {
        match self {
            Self::Home => Destination::Home,
            Self::Artists(_) => Destination::Artists,
            Self::Albums(_) => Destination::Albums,
            Self::Songs => Destination::Songs,
            Self::Playlists(_) => Destination::Playlists,
            Self::Favorites => Destination::Favorites,
        }
    }

    /// Whether this is somewhere to come back to after going to `next`. A
    /// screen with nothing selected is not, once `next` selects something on
    /// it: going back could not show it again.
    pub fn is_worth_returning_to_from(&self, next: &Self) -> bool {
        let has_selection = matches!(
            self,
            Self::Artists(Some(_)) | Self::Albums(Some(_)) | Self::Playlists(Some(_))
        );
        self != next && (has_selection || self.destination() != next.destination())
    }
}

/// The places behind and ahead of the current one, which its owner knows.
pub struct History<T> {
    behind: Vec<T>,
    ahead: Vec<T>,
}

impl<T> History<T> {
    pub fn new() -> Self {
        Self {
            behind: Vec::new(),
            ahead: Vec::new(),
        }
    }

    /// Records going somewhere new from `left`, which drops what was ahead.
    pub fn visit(&mut self, left: T) {
        self.behind.push(left);
        self.ahead.clear();
    }

    pub fn back(&mut self, current: T) -> Option<T> {
        let previous = self.behind.pop()?;
        self.ahead.push(current);
        Some(previous)
    }

    pub fn forward(&mut self, current: T) -> Option<T> {
        let next = self.ahead.pop()?;
        self.behind.push(current);
        Some(next)
    }

    pub fn can_go_back(&self) -> bool {
        !self.behind.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.ahead.is_empty()
    }
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(id: &str) -> Location {
        Location::Albums(Some(AlbumId::new(id)))
    }

    #[test]
    fn nowhere_to_go_at_first() {
        let mut history = History::new();
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
        assert_eq!(history.back(1), None);
        assert_eq!(history.forward(1), None);
    }

    #[test]
    fn going_back_and_forward_retraces_the_places_visited() {
        let mut history = History::new();
        history.visit(1);
        history.visit(2);

        assert_eq!(history.back(3), Some(2));
        assert_eq!(history.back(2), Some(1));
        assert_eq!(history.back(1), None);
        assert_eq!(history.forward(1), Some(2));
        assert_eq!(history.forward(2), Some(3));
        assert_eq!(history.forward(3), None);
    }

    #[test]
    fn a_place_that_went_nowhere_stays_current() {
        let mut history = History::new();
        history.visit(1);
        assert_eq!(history.forward(2), None);
        assert_eq!(history.back(2), Some(1));
        assert_eq!(history.forward(1), Some(2));
    }

    #[test]
    fn visiting_somewhere_new_drops_what_was_ahead() {
        let mut history = History::new();
        history.visit(1);
        assert_eq!(history.back(2), Some(1));
        assert!(history.can_go_forward());

        history.visit(1);

        assert!(!history.can_go_forward());
        assert!(history.can_go_back());
    }

    #[test]
    fn another_screen_is_worth_returning_to() {
        assert!(Location::Home.is_worth_returning_to_from(&album("a")));
        assert!(Location::Albums(None).is_worth_returning_to_from(&Location::Songs));
    }

    #[test]
    fn another_selection_on_the_screen_is_worth_returning_to() {
        assert!(album("a").is_worth_returning_to_from(&album("b")));
    }

    #[test]
    fn the_place_itself_is_not_worth_returning_to() {
        assert!(!album("a").is_worth_returning_to_from(&album("a")));
    }

    #[test]
    fn a_screen_with_nothing_selected_is_not_worth_returning_to_from_itself() {
        assert!(!Location::Albums(None).is_worth_returning_to_from(&album("a")));
    }
}
