use std::sync::{Arc, Weak};
use std::time::Duration;

use executor::{Executor, Publish, Published, Task};
use session::Session;
use subsonic::{Album, AlbumId, AlbumWithSongs, Song};
use tokio::sync::watch;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AlbumDetailSnapshot {
    #[default]
    None,
    Loading(AlbumId),
    Loaded(LoadedAlbum),
    /// Nothing of the album showed, for this reason.
    Failed {
        album: AlbumId,
        error: String,
    },
}

impl AlbumDetailSnapshot {
    pub fn album(&self) -> Option<&AlbumId> {
        match self {
            Self::None => None,
            Self::Loading(album) | Self::Failed { album, .. } => Some(album),
            Self::Loaded(loaded) => Some(loaded.album.id()),
        }
    }

    pub fn loaded(&self) -> Option<&LoadedAlbum> {
        match self {
            Self::Loaded(loaded) => Some(loaded),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedAlbum {
    album: Album,
    songs: Arc<[Song]>,
}

impl LoadedAlbum {
    pub fn album(&self) -> &Album {
        &self.album
    }

    pub fn songs(&self) -> &Arc<[Song]> {
        &self.songs
    }

    /// The album's, or else the sum of its songs', unless one of them does not say.
    pub fn duration(&self) -> Option<Duration> {
        self.album
            .duration()
            .or_else(|| self.songs.iter().map(Song::duration).sum())
    }

    /// Whether `song` is by another artist than the album, as on a compilation, so that its own
    /// is worth naming.
    pub fn is_by_another_artist(&self, song: &Song) -> bool {
        song.artist() != self.album.artist()
    }
}

impl From<AlbumWithSongs> for LoadedAlbum {
    fn from(loaded: AlbumWithSongs) -> Self {
        let (album, songs) = loaded.into_parts();
        Self {
            album,
            songs: songs.into(),
        }
    }
}

/// One album and its songs on show: the saved ones at once, then the server's, which are saved
/// in their place. Dropping the last handle stops what is under way.
#[derive(Clone)]
pub struct AlbumDetail(Arc<Inner>);

struct WeakDetail(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
}

struct State {
    detail: AlbumDetailSnapshot,
    /// Counts the albums asked for, so that what one replaced brings on another thread cannot
    /// change what its successor shows.
    work_id: u64,
    _work: Task<()>,
}

impl Publish for State {
    type Snapshot = AlbumDetailSnapshot;

    fn snapshot(&self) -> &AlbumDetailSnapshot {
        &self.detail
    }
}

impl AlbumDetail {
    pub fn new(executor: Executor, session: Session) -> Self {
        Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                detail: AlbumDetailSnapshot::None,
                work_id: 0,
                _work: Task::ready(()),
            }),
        }))
    }

    pub fn snapshot(&self) -> AlbumDetailSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling the detail, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<AlbumDetailSnapshot> {
        self.0.state.subscribe()
    }

    /// Loads `album` afresh, even when it is the one on show.
    pub fn show(&self, album: AlbumId) {
        self.0.state.update(|state| self.start(state, album));
    }

    pub fn clear(&self) {
        self.0.state.update(|state| {
            state.work_id += 1;
            state._work = Task::ready(());
            state.detail = AlbumDetailSnapshot::None;
        });
    }

    fn start(&self, state: &mut State, album: AlbumId) {
        state.work_id += 1;
        let work_id = state.work_id;
        state.detail = AlbumDetailSnapshot::Loading(album.clone());
        let this = self.downgrade();
        let executor = self.0.executor.clone();
        let session = self.0.session.clone();
        state._work = self.0.executor.spawn(async move {
            let catalog = session.catalog().cloned();
            if let Some(saved) = saved_album(&executor, catalog.clone(), album.clone()).await {
                this.change(work_id, |state| {
                    state.detail = AlbumDetailSnapshot::Loaded(saved.into());
                });
            }
            let result = session.client().album(&album).await;
            if let (Ok(fetched), Some(catalog)) = (&result, catalog) {
                let fetched = fetched.clone();
                executor
                    .spawn_blocking(move || catalog.put_album(&fetched).ok())
                    .detach();
            }
            let result = result.map_err(|error| session::describe(&error));
            this.change(work_id, |state| finish(state, album, result));
        });
    }

    fn downgrade(&self) -> WeakDetail {
        WeakDetail(Arc::downgrade(&self.0))
    }
}

impl WeakDetail {
    /// Changes the state unless the work `work_id` has been replaced.
    fn change(&self, work_id: u64, change: impl FnOnce(&mut State)) {
        if let Some(inner) = self.0.upgrade() {
            inner.state.update(|state| {
                if state.work_id == work_id {
                    change(state);
                }
            });
        }
    }
}

fn finish(state: &mut State, album: AlbumId, result: Result<AlbumWithSongs, String>) {
    match result {
        Ok(fetched) => state.detail = AlbumDetailSnapshot::Loaded(fetched.into()),
        // Saved songs on show stay there
        Err(_) if matches!(state.detail, AlbumDetailSnapshot::Loaded(_)) => {}
        Err(error) => state.detail = AlbumDetailSnapshot::Failed { album, error },
    }
}

/// The songs of `album` to act on: the saved ones when there are any, otherwise the server's.
/// The error is a sentence for the user.
pub fn album_songs(
    executor: &Executor,
    session: &Session,
    album: AlbumId,
) -> Task<Result<Arc<[Song]>, String>> {
    let (executor, session) = (executor.clone(), session.clone());
    executor.clone().spawn(async move {
        let found = match saved_album(&executor, session.catalog().cloned(), album.clone()).await {
            Some(saved) => Ok(saved),
            None => session.client().album(&album).await,
        };
        found
            .map(|found| found.into_parts().1.into())
            .map_err(|error| format!("Couldn't load this album. {}", session::describe(&error)))
    })
}

/// `None` when nothing of it is saved or the saved library can't be read.
async fn saved_album(
    executor: &Executor,
    catalog: Option<catalog::Catalog>,
    album: AlbumId,
) -> Option<AlbumWithSongs> {
    let catalog = catalog?;
    executor
        .spawn_blocking(move || catalog.album(&album).ok().flatten())
        .await
}

#[cfg(test)]
mod tests;
