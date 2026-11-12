use anti_webui::prelude::{Signal, signal};
use hiphile_core::navigation::{Destination, History, Location};

/// Where the window is, and where it has been. Cheap to copy.
#[derive(Clone, Copy)]
pub(crate) struct Navigation {
    location: Signal<Location>,
    history: Signal<History<Location>>,
    /// Where the window was last in each destination, in the rail's order.
    last: Signal<Vec<Location>>,
}

impl Navigation {
    pub(crate) fn new() -> Self {
        Self {
            location: signal(Location::Home),
            history: signal(History::new()),
            last: signal(Destination::ALL.map(location_of).to_vec()),
        }
    }

    /// Where the window is, which also says what is selected there.
    pub(crate) fn location(&self) -> Signal<Location> {
        self.location
    }

    pub(crate) fn destination(&self) -> Destination {
        self.location.with(Location::destination)
    }

    /// Goes to `to`, or, when `to` names only a destination, back to where the window was last
    /// there.
    pub(crate) fn go(&self, to: Location) {
        let to = if to == location_of(to.destination()) {
            self.last
                .with_untracked(|last| last[index_of(to.destination())].clone())
        } else {
            to
        };
        let now = self.location.get_untracked();
        if now == to {
            return;
        }
        if now.is_worth_returning_to_from(&to) {
            self.history.update(|history| history.visit(now));
        }
        self.set(to);
    }

    /// Goes to `to` without making where the window is somewhere to go back to.
    pub(crate) fn show_in_place(&self, to: Location) {
        if self.location.with_untracked(|now| *now != to) {
            self.set(to);
        }
    }

    pub(crate) fn back(&self) {
        let now = self.location.get_untracked();
        if let Some(previous) = self.history.update(|history| history.back(now)) {
            self.set(previous);
        }
    }

    pub(crate) fn forward(&self) {
        let now = self.location.get_untracked();
        if let Some(next) = self.history.update(|history| history.forward(now)) {
            self.set(next);
        }
    }

    fn set(&self, to: Location) {
        let ix = index_of(to.destination());
        self.last.update_untracked(|last| last[ix] = to.clone());
        self.location.set(to);
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.history.with(History::can_go_back)
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        self.history.with(History::can_go_forward)
    }
}

/// What names `destination` alone: its list, with nothing chosen.
pub(crate) fn location_of(destination: Destination) -> Location {
    match destination {
        Destination::Home => Location::Home,
        Destination::Artists => Location::Artists(None),
        Destination::Albums => Location::Albums(None),
        Destination::Songs => Location::Songs,
        Destination::Playlists => Location::Playlists(None),
        Destination::Favorites => Location::Favorites,
    }
}

/// Its place in the rail.
pub(crate) fn index_of(destination: Destination) -> usize {
    Destination::ALL
        .iter()
        .position(|shown| *shown == destination)
        .expect("every destination is in the rail")
}
