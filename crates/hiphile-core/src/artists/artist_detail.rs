use std::sync::{Arc, Weak};
use std::time::Duration;

use executor::{Executor, Publish, Published, Task};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use session::Session;
use subsonic::{Album, Artist, ArtistId, CoverArtId, Song};
use tokio::sync::watch;

use super::list::{album_by_album, newest_first};
use crate::Shelf;
use crate::library::on_blocking_thread;

const TOP_SONGS: u32 = 10;
const SIMILAR_ARTISTS: u32 = 6;
/// Top songs and similar artists take the server's outside service a while: moving through the
/// list with the arrow keys should not ask about every artist.
const SERVER_DELAY: Duration = Duration::from_millis(250);
const ALBUMS_AT_ONCE: usize = 8;
/// How many top songs show until a host asks for all of them.
const TOP_SONGS_COLLAPSED: usize = 5;

/// One artist's page.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ArtistPage {
    pub id: ArtistId,
    /// Known once the list, the saved albums or the server has said.
    pub name: Option<String>,
    pub cover_art: Option<CoverArtId>,
    /// Newest first.
    pub albums: Shelf<Album>,
    pub top_songs: Shelf<Song>,
    pub similar: Shelf<Artist>,
    /// Whether the artist has no albums of their own, only songs on others', which `top_songs`
    /// then holds in place of what the server ranks.
    pub is_only_on_songs: bool,
}

/// A part of an artist's page, in the order they show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtistSection {
    TopSongs,
    /// The songs of an artist with no albums of their own, in place of their top songs and their
    /// albums.
    Songs,
    Albums,
    Similar,
}

impl ArtistPage {
    /// Top songs and similar artists with nothing to show go away: a server without an outside
    /// service to ask has nothing to say about them. A section loading or failed keeps its place.
    pub fn sections(&self) -> Vec<ArtistSection> {
        let mut sections = Vec::new();
        if !self.top_songs.has_loaded_nothing() {
            sections.push(if self.is_only_on_songs {
                ArtistSection::Songs
            } else {
                ArtistSection::TopSongs
            });
        }
        if !self.is_only_on_songs {
            sections.push(ArtistSection::Albums);
        }
        if !self.similar.has_loaded_nothing() {
            sections.push(ArtistSection::Similar);
        }
        sections
    }

    /// The top songs that show: the first few, unless `is_expanded`.
    pub fn top_songs_on_show(&self, is_expanded: bool) -> &[Song] {
        let songs = self.top_songs.items();
        if is_expanded {
            songs
        } else {
            &songs[..songs.len().min(TOP_SONGS_COLLAPSED)]
        }
    }

    /// Whether more top songs show expanded than collapsed.
    pub fn has_more_top_songs(&self) -> bool {
        self.top_songs.items().len() > TOP_SONGS_COLLAPSED
    }

    /// The artist's own picture, or else the cover of their first album or their first song.
    pub fn picture(&self) -> Option<&CoverArtId> {
        self.cover_art
            .as_ref()
            .or_else(|| self.albums.items().first().and_then(Album::cover_art))
            .or_else(|| self.top_songs.items().first().and_then(Song::cover_art))
    }

    /// How many albums of their own, once they are loaded.
    pub fn album_count(&self) -> Option<usize> {
        match &self.albums {
            _ if self.is_only_on_songs => None,
            Shelf::Loaded(albums) => Some(albums.len()),
            Shelf::Loading | Shelf::Failed(_) => None,
        }
    }

    /// How many songs, from the albums' counts, unless one of them does not say. An artist with
    /// no albums has the songs they are on.
    pub fn song_count(&self) -> Option<usize> {
        if self.is_only_on_songs {
            return Some(self.top_songs.items().len());
        }
        let Shelf::Loaded(albums) = &self.albums else {
            return None;
        };
        let songs: Option<u32> = albums.iter().map(Album::song_count).sum();
        songs.map(|songs| songs as usize)
    }

    /// Whether there is anything to play.
    pub fn has_songs(&self) -> bool {
        !self.albums.items().is_empty() || self.is_only_on_songs
    }

    /// Whether `song` is by another artist than this one, so that its own is worth naming.
    pub fn is_by_another_artist(&self, song: &Song) -> bool {
        song.artist_id() != Some(&self.id)
    }

    /// The artist as their page has them, once their name is known, as a favorite needs a name.
    pub fn as_artist(&self) -> Option<Artist> {
        let name = self.name.clone()?;
        let albums = self
            .album_count()
            .and_then(|count| u32::try_from(count).ok());
        Some(Artist::new(
            self.id.clone(),
            name,
            self.cover_art.clone(),
            albums,
        ))
    }
}

/// The page of the artist on show, if any: their albums, from the saved library and then the
/// server, their top songs and similar artists. Dropping the last handle stops what is loading.
#[derive(Clone)]
pub struct ArtistDetail(Arc<Inner>);

#[derive(Clone)]
struct WeakDetail(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: Session,
    state: Published<State>,
}

struct State {
    page: Option<ArtistPage>,
    /// Counts the work of every section, so that work replaced while it was finishing on another
    /// thread cannot change its successor's section, or another artist's.
    work_id: u64,
    albums: Work,
    top_songs: Work,
    similar: Work,
}

#[derive(Clone, Copy)]
enum Section {
    Albums,
    TopSongs,
    Similar,
}

struct Work {
    id: u64,
    _task: Task<()>,
}

impl ArtistDetail {
    pub fn new(executor: Executor, session: Session) -> Self {
        let idle = || Work {
            id: 0,
            _task: Task::ready(()),
        };
        Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                page: None,
                work_id: 0,
                albums: idle(),
                top_songs: idle(),
                similar: idle(),
            }),
        }))
    }

    pub fn page(&self) -> Option<ArtistPage> {
        self.0.state.snapshot()
    }

    /// Sees each new page. A borrow of it must not be held while calling the detail, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<Option<ArtistPage>> {
        self.0.state.subscribe()
    }

    /// Shows `artist`, named and pictured as `listed` has them until the server says. Does
    /// nothing to the artist already on show.
    pub fn show(&self, artist: &ArtistId, listed: Option<&Artist>) {
        self.0.state.update(|state| {
            if state.page.as_ref().is_some_and(|page| page.id == *artist) {
                return;
            }
            state.page = Some(ArtistPage {
                id: artist.clone(),
                name: listed.map(|listed| listed.name().to_owned()),
                cover_art: listed.and_then(|listed| listed.cover_art().cloned()),
                albums: Shelf::Loading,
                top_songs: Shelf::Loading,
                similar: Shelf::Loading,
                is_only_on_songs: false,
            });
            self.load(state);
        });
    }

    /// Shows no artist.
    pub fn close(&self) {
        self.0.state.update(|state| {
            state.page = None;
            for section in [Section::Albums, Section::TopSongs, Section::Similar] {
                state.replace(section, Task::ready(()));
            }
        });
    }

    /// Loads every section of the artist on show again.
    pub fn reload(&self) {
        self.0.state.update(|state| self.load(state));
    }

    pub fn reload_albums(&self) {
        self.0.state.update(|state| self.load_albums(state));
    }

    pub fn reload_top_songs(&self) {
        self.0.state.update(|state| self.load_top_songs(state));
    }

    pub fn reload_similar(&self) {
        self.0.state.update(|state| self.load_similar(state));
    }

    /// Reads the saved songs again for an artist with no albums of their own, whose page is made
    /// of them.
    pub fn songs_synced(&self) {
        self.0.state.update(|state| {
            let has_no_albums = state.page.as_ref().is_some_and(
                |page| matches!(&page.albums, Shelf::Loaded(albums) if albums.is_empty()),
            );
            if has_no_albums {
                self.load_albums(state);
            }
        });
    }

    /// Every song of `artist`, album by album as their page shows them, or their saved songs
    /// when they have no albums. Errors are sentences for the user.
    pub fn songs(&self, artist: &ArtistId) -> Task<Result<Vec<Song>, String>> {
        let shown = self.0.state.read(|state| {
            state
                .page
                .as_ref()
                .filter(|page| page.id == *artist && page.albums.is_loaded())
                .map(|page| page.albums.items().to_vec())
        });
        let executor = self.0.executor.clone();
        let session = self.0.session.clone();
        let artist = artist.clone();
        self.0.executor.spawn(async move {
            // Reading an album blocks on the saved library
            let work = async move { artist_songs(&session, &artist, shown).await };
            on_blocking_thread(&executor, work).await.map_err(|error| {
                format!("Couldn't load this artist. {}", session::describe(&error))
            })
        })
    }

    fn load(&self, state: &mut State) {
        self.load_albums(state);
        self.load_similar(state);
        self.load_top_songs(state);
    }

    /// Shows the saved albums at once. The server's answer replaces them, and names an artist
    /// nothing else had a name for.
    fn load_albums(&self, state: &mut State) {
        let Some(page) = &mut state.page else {
            return;
        };
        page.albums.begin();
        let artist = page.id.clone();
        let work_id = state.next_work();
        let client = self.0.session.client().clone();
        let catalog = self.0.session.catalog().cloned();
        let executor = self.0.executor.clone();
        let this = self.downgrade();
        let task = self.0.executor.spawn(async move {
            if let Some(catalog) = catalog.clone() {
                let id = artist.clone();
                let saved = executor
                    .spawn_blocking(move || catalog.albums_by_artist(&id))
                    .await;
                if let Ok(saved) = saved
                    && !saved.is_empty()
                {
                    this.change(Section::Albums, work_id, |this, state| {
                        this.show_saved_albums(state, saved);
                    });
                }
            }
            executor.timer(SERVER_DELAY).await;
            let result = client
                .artist(&artist)
                .await
                .map(|found| found.into_parts())
                .map_err(|error| session::describe(&error));
            // Read now so that "no albums" never shows on the way to the songs
            let has_no_albums = matches!(&result, Ok((_, albums)) if albums.is_empty());
            let songs = match catalog {
                Some(catalog) if has_no_albums => {
                    executor
                        .spawn_blocking(move || saved_songs_naming(&catalog, &artist))
                        .await
                }
                _ => Vec::new(),
            };
            this.change(Section::Albums, work_id, |this, state| {
                this.finish_albums(state, result, songs);
            });
        });
        state.albums = Work {
            id: work_id,
            _task: task,
        };
    }

    fn show_saved_albums(&self, state: &mut State, albums: Vec<Album>) {
        let Some(page) = &mut state.page else {
            return;
        };
        let names_artist = page.name.is_none();
        if names_artist {
            page.name = albums.first().and_then(Album::artist).map(str::to_owned);
        }
        page.albums = Shelf::Loaded(newest_first(albums).into());
        if names_artist {
            self.load_top_songs(state);
        }
    }

    fn finish_albums(
        &self,
        state: &mut State,
        result: Result<(Artist, Vec<Album>), String>,
        songs_naming_artist: Vec<Song>,
    ) {
        let Some(page) = &mut state.page else {
            return;
        };
        let had_name = page.name.is_some();
        let was_only_on_songs = page.is_only_on_songs;
        match result {
            Ok((artist, albums)) => {
                page.name = Some(artist.name().to_owned());
                if artist.cover_art().is_some() {
                    page.cover_art = artist.cover_art().cloned();
                }
                page.albums.finish(Ok(newest_first(albums)));
                page.is_only_on_songs = !songs_naming_artist.is_empty();
            }
            Err(error) => page.albums.finish(Err(error)),
        }
        if page.is_only_on_songs {
            // A failed refresh brings no songs: those on show stay
            if !songs_naming_artist.is_empty() {
                page.top_songs = Shelf::Loaded(songs_naming_artist.into());
            }
            state.replace(Section::TopSongs, Task::ready(()));
        } else if was_only_on_songs || (!had_name && page.name.is_some()) {
            // Top songs are asked for by name, which may only now have arrived
            self.load_top_songs(state);
        }
    }

    fn load_top_songs(&self, state: &mut State) {
        let Some(page) = &mut state.page else {
            return;
        };
        if page.is_only_on_songs {
            return;
        }
        page.top_songs.begin();
        let name = page.name.clone();
        // Whatever was asked before was for another name, or another artist
        let Some(name) = name else {
            state.replace(Section::TopSongs, Task::ready(()));
            return;
        };
        let work_id = state.next_work();
        let client = self.0.session.client().clone();
        let executor = self.0.executor.clone();
        let this = self.downgrade();
        let task = self.0.executor.spawn(async move {
            executor.timer(SERVER_DELAY).await;
            let result = client
                .top_songs(&name, TOP_SONGS)
                .await
                .map_err(|error| format!("Couldn't load top songs. {}", session::describe(&error)));
            this.change(Section::TopSongs, work_id, |_, state| {
                if let Some(page) = &mut state.page {
                    page.top_songs.finish(result);
                }
            });
        });
        state.top_songs = Work {
            id: work_id,
            _task: task,
        };
    }

    fn load_similar(&self, state: &mut State) {
        let Some(page) = &mut state.page else {
            return;
        };
        page.similar.begin();
        let artist = page.id.clone();
        let work_id = state.next_work();
        let client = self.0.session.client().clone();
        let executor = self.0.executor.clone();
        let this = self.downgrade();
        let task = self.0.executor.spawn(async move {
            executor.timer(SERVER_DELAY).await;
            let result = client
                .similar_artists(&artist, SIMILAR_ARTISTS)
                .await
                .map_err(|error| {
                    format!(
                        "Couldn't load similar artists. {}",
                        session::describe(&error)
                    )
                });
            this.change(Section::Similar, work_id, |_, state| {
                if let Some(page) = &mut state.page {
                    page.similar.finish(result);
                }
            });
        });
        state.similar = Work {
            id: work_id,
            _task: task,
        };
    }

    fn downgrade(&self) -> WeakDetail {
        WeakDetail(Arc::downgrade(&self.0))
    }
}

impl State {
    fn next_work(&mut self) -> u64 {
        self.work_id += 1;
        self.work_id
    }

    fn work(&self, section: Section) -> &Work {
        match section {
            Section::Albums => &self.albums,
            Section::TopSongs => &self.top_songs,
            Section::Similar => &self.similar,
        }
    }

    /// Drops the work under way for `section`, so that its result is not taken.
    fn replace(&mut self, section: Section, task: Task<()>) {
        let work = Work {
            id: self.next_work(),
            _task: task,
        };
        match section {
            Section::Albums => self.albums = work,
            Section::TopSongs => self.top_songs = work,
            Section::Similar => self.similar = work,
        }
    }
}

impl Publish for State {
    type Snapshot = Option<ArtistPage>;

    fn snapshot(&self) -> &Option<ArtistPage> {
        &self.page
    }
}

impl WeakDetail {
    /// Changes the state unless the work `work_id` for `section` has been replaced.
    fn change(
        &self,
        section: Section,
        work_id: u64,
        change: impl FnOnce(&ArtistDetail, &mut State),
    ) {
        if let Some(this) = self.0.upgrade().map(ArtistDetail) {
            this.0.state.update(|state| {
                if state.work(section).id == work_id {
                    change(&this, state);
                }
            });
        }
    }
}

async fn artist_songs(
    session: &Session,
    artist: &ArtistId,
    albums: Option<Vec<Album>>,
) -> Result<Vec<Song>, client::Error> {
    let albums = match albums {
        Some(albums) => albums,
        None => {
            let (_, albums) = session.client().artist(artist).await?.into_parts();
            newest_first(albums)
        }
    };
    let fetches: Vec<_> = albums
        .iter()
        .map(|album| session.album(album.id()))
        .collect();
    let albums: Vec<_> = stream::iter(fetches)
        .buffered(ALBUMS_AT_ONCE)
        .try_collect()
        .await?;
    let mut songs: Vec<Song> = albums
        .into_iter()
        .flat_map(|album| album.into_parts().1)
        .collect();
    if songs.is_empty()
        && let Some(catalog) = session.catalog()
    {
        songs = saved_songs_naming(catalog, artist);
    }
    Ok(songs)
}

/// A copy that can't be read has nothing to add to what the server said.
fn saved_songs_naming(catalog: &catalog::Catalog, artist: &ArtistId) -> Vec<Song> {
    album_by_album(catalog.songs_by_artist(artist).unwrap_or_default())
}

#[cfg(test)]
mod tests;
