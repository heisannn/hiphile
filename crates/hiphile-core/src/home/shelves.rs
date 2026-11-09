use std::sync::{Arc, Weak};

use executor::{Executor, Publish, Published, Task};
use futures::future::join_all;
use session::Session;
use subsonic::{Album, Song};
use tokio::sync::watch;

use super::daily::DailyRecommendations;
use super::layout::{AlbumShelf, HomeSection};
use super::recent::most_recent;
use crate::Shelf;

/// How many of the most played albums the home screen asks for. The recommendations leave them
/// out.
pub(crate) const MOST_PLAYED: u32 = AlbumShelf::MostPlayed.room() as u32;
/// How many of the new albums the home screen asks for. The recommendations leave them out.
pub(crate) const NEW_ALBUMS: u32 = AlbumShelf::New.room() as u32;
const RECENT_SONGS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelvesStatus {
    recently_played: Shelf<Song>,
    most_played: Shelf<Album>,
    new_albums: Shelf<Album>,
}

impl ShelvesStatus {
    /// Most recent first.
    pub fn recently_played(&self) -> &Shelf<Song> {
        &self.recently_played
    }

    pub fn most_played(&self) -> &Shelf<Album> {
        &self.most_played
    }

    pub fn new_albums(&self) -> &Shelf<Album> {
        &self.new_albums
    }
}

/// The home screen's shelves besides the recommendations: the songs played last, the albums
/// played most and the newest albums, which the recommendations leave out. They come from the
/// server, which alone knows every play, and start loading when made. Dropping the last handle
/// stops the loads under way.
#[derive(Clone)]
pub struct HomeShelves(Arc<Inner>);

#[derive(Clone)]
struct WeakShelves(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    /// Told under the shelves' lock what they show, so that a shelf finishing on another thread
    /// cannot tell it an older list after a newer one. It never calls the shelves.
    recommendations: DailyRecommendations,
    state: Published<State>,
}

struct State {
    status: ShelvesStatus,
    recently_played: Load,
    most_played: Load,
    new_albums: Load,
}

/// One shelf's load under way. Loads are counted, so that one replaced while it was finishing on
/// another thread cannot change the shelf after its successor has.
struct Load {
    id: u64,
    _task: Task<()>,
}

impl HomeShelves {
    pub fn new(
        executor: Executor,
        session: Session,
        recommendations: DailyRecommendations,
    ) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            recommendations,
            state: Published::new(State {
                status: ShelvesStatus {
                    recently_played: Shelf::Loading,
                    most_played: Shelf::Loading,
                    new_albums: Shelf::Loading,
                },
                recently_played: Load::none(),
                most_played: Load::none(),
                new_albums: Load::none(),
            }),
        }));
        this.0.state.update(|state| {
            this.load_recently_played(state);
            this.load_most_played(state);
            this.load_new_albums(state);
        });
        this
    }

    pub fn status(&self) -> ShelvesStatus {
        self.0.state.snapshot()
    }

    /// Sees each new status. A borrow of it must not be held while calling the shelves, which
    /// publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<ShelvesStatus> {
        self.0.state.subscribe()
    }

    /// A play the server has heard of can change what was played last and most.
    pub fn song_played(&self) {
        self.0.state.update(|state| {
            self.load_recently_played(state);
            self.load_most_played(state);
        });
    }

    /// A library sync can change what the server has played most and added last.
    pub fn library_synced(&self) {
        self.0.state.update(|state| {
            self.load_most_played(state);
            self.load_new_albums(state);
        });
    }

    pub fn reload_recently_played(&self) {
        self.0
            .state
            .update(|state| self.load_recently_played(state));
    }

    pub fn reload_most_played(&self) {
        self.0.state.update(|state| self.load_most_played(state));
    }

    pub fn reload_new_albums(&self) {
        self.0.state.update(|state| self.load_new_albums(state));
    }

    /// Loads a shelf that failed again, the recommendations included. Their reload is for
    /// albums on show, which a pick that fails leaves there.
    pub fn retry(&self, section: HomeSection) {
        match section {
            HomeSection::Recommended => self.0.recommendations.pick(),
            HomeSection::RecentlyPlayed => self.reload_recently_played(),
            HomeSection::MostPlayed => self.reload_most_played(),
            HomeSection::NewAlbums => self.reload_new_albums(),
        }
    }

    fn load_recently_played(&self, state: &mut State) {
        state.status.recently_played.begin();
        let client = self.0.session.client().clone();
        let fetch = async move {
            fetch_recently_played(&client)
                .await
                .map_err(|error| session::describe(&error))
        };
        self.start(
            state,
            |state| &mut state.recently_played,
            fetch,
            |_, status, result| {
                finish_recently_played(&mut status.recently_played, result);
            },
        );
    }

    fn load_most_played(&self, state: &mut State) {
        state.status.most_played.begin();
        let client = self.0.session.client().clone();
        let fetch = async move {
            client
                .frequent_albums(MOST_PLAYED)
                .await
                .map_err(|error| session::describe(&error))
        };
        self.start(
            state,
            |state| &mut state.most_played,
            fetch,
            |this, status, result| {
                status.most_played.finish(result);
                this.keep_shelves_off_recommended(status);
            },
        );
    }

    fn load_new_albums(&self, state: &mut State) {
        state.status.new_albums.begin();
        let client = self.0.session.client().clone();
        let fetch = async move {
            client
                .newest_albums(NEW_ALBUMS)
                .await
                .map_err(|error| session::describe(&error))
        };
        self.start(
            state,
            |state| &mut state.new_albums,
            fetch,
            |this, status, result| {
                status.new_albums.finish(result);
                this.keep_shelves_off_recommended(status);
            },
        );
    }

    /// Replaces the load that `load` picks out of the state with `fetch`, whose result `finish`
    /// takes in unless another load has replaced it meanwhile.
    fn start<R: Send + 'static>(
        &self,
        state: &mut State,
        load: fn(&mut State) -> &mut Load,
        fetch: impl Future<Output = R> + Send + 'static,
        finish: impl FnOnce(&Self, &mut ShelvesStatus, R) + Send + 'static,
    ) {
        let id = load(state).id + 1;
        let this = self.downgrade();
        let task = self.0.executor.spawn(async move {
            let result = fetch.await;
            if let Some(this) = this.upgrade() {
                this.0.state.update(|state| {
                    if load(state).id == id {
                        finish(&this, &mut state.status, result);
                    }
                });
            }
        });
        *load(state) = Load { id, _task: task };
    }

    fn keep_shelves_off_recommended(&self, status: &ShelvesStatus) {
        let shelved = [&status.most_played, &status.new_albums]
            .into_iter()
            .flat_map(Shelf::items)
            .map(|album| album.id().clone())
            .collect();
        self.0.recommendations.keep_off(shelved);
    }

    fn downgrade(&self) -> WeakShelves {
        WeakShelves(Arc::downgrade(&self.0))
    }
}

impl Load {
    fn none() -> Self {
        Self {
            id: 0,
            _task: Task::ready(()),
        }
    }
}

impl Publish for State {
    type Snapshot = ShelvesStatus;

    fn snapshot(&self) -> &ShelvesStatus {
        &self.status
    }
}

impl WeakShelves {
    fn upgrade(&self) -> Option<HomeShelves> {
        self.0.upgrade().map(HomeShelves)
    }
}

struct RecentlyPlayed {
    songs: Vec<Song>,
    /// Whether every album asked about came through.
    complete: bool,
}

/// The songs played last. Those are in the albums played last: an album's last play is that of
/// its latest song, so `RECENT_SONGS` songs lie within as many albums. No method returns the
/// songs themselves. An album that fails to load only leaves out its songs.
async fn fetch_recently_played(client: &client::Client) -> Result<RecentlyPlayed, client::Error> {
    let albums = client.recently_played_albums(RECENT_SONGS as u32).await?;
    let loaded = join_all(albums.iter().map(|album| client.album(album.id()))).await;
    let mut songs = Vec::new();
    let mut failure = None;
    for album in loaded {
        match album {
            Ok(album) => songs.extend(album.into_parts().1),
            Err(error) => failure = Some(error),
        }
    }
    match failure {
        Some(error) if songs.is_empty() => Err(error),
        failure => Ok(RecentlyPlayed {
            songs: most_recent(songs, RECENT_SONGS),
            complete: failure.is_none(),
        }),
    }
}

/// Songs on show stay there when the load missed some of the albums, as it may have missed the
/// latest.
fn finish_recently_played(shelf: &mut Shelf<Song>, result: Result<RecentlyPlayed, String>) {
    match result {
        Ok(found) if !found.complete && matches!(shelf, Shelf::Loaded(_)) => {}
        result => shelf.finish(result.map(|found| found.songs)),
    }
}

#[cfg(test)]
mod tests;
