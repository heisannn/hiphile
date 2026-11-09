use std::sync::{Arc, Weak};

use executor::{Executor, Publish, Published, Task};
use subsonic::{Playlist, PlaylistId, PlaylistWithSongs, Song, SongId};
use tokio::sync::{broadcast, watch};

use super::{Entries, EntryId, Write, WriteQueue};

const NOTICES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistList {
    Loading,
    /// By name, ignoring case.
    Loaded(Arc<Vec<Playlist>>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistDetail {
    /// No playlist is open.
    None,
    Loading,
    Loaded(OpenPlaylist),
    Failed(String),
}

/// The open playlist and its songs, as edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenPlaylist {
    playlist: Playlist,
    entries: Entries,
}

impl OpenPlaylist {
    pub fn playlist(&self) -> &Playlist {
        &self.playlist
    }

    pub fn entries(&self) -> &Entries {
        &self.entries
    }
}

/// A playlist songs can be added to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistChoice {
    id: PlaylistId,
    name: Arc<str>,
}

impl PlaylistChoice {
    pub fn id(&self) -> &PlaylistId {
        &self.id
    }

    pub fn name(&self) -> &Arc<str> {
        &self.name
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistsSnapshot {
    list: PlaylistList,
    /// Why reading the list again failed, while the one loaded before stays on show.
    refresh_error: Option<String>,
    /// The playlists of the list that the user may edit, in its order.
    choices: Arc<[PlaylistChoice]>,
    detail: PlaylistDetail,
    /// Whether a save failed on the open playlist, which then shows what the server doesn't
    /// have until it has been read again, and takes no edits.
    is_diverged: bool,
    removed: Option<RemovedSong>,
}

impl PlaylistsSnapshot {
    pub fn list(&self) -> &PlaylistList {
        &self.list
    }

    pub fn loaded_list(&self) -> Option<&Arc<Vec<Playlist>>> {
        match &self.list {
            PlaylistList::Loaded(playlists) => Some(playlists),
            PlaylistList::Loading | PlaylistList::Failed(_) => None,
        }
    }

    pub fn refresh_error(&self) -> Option<&str> {
        self.refresh_error.as_deref()
    }

    pub fn choices(&self) -> &Arc<[PlaylistChoice]> {
        &self.choices
    }

    pub fn detail(&self) -> &PlaylistDetail {
        &self.detail
    }

    pub fn open_playlist(&self) -> Option<&OpenPlaylist> {
        match &self.detail {
            PlaylistDetail::Loaded(open) => Some(open),
            PlaylistDetail::None | PlaylistDetail::Loading | PlaylistDetail::Failed(_) => None,
        }
    }

    pub fn is_diverged(&self) -> bool {
        self.is_diverged
    }

    pub fn can_undo_removal(&self) -> bool {
        self.removed.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistsNotice {
    /// A change the server refused or never heard of, with what and why.
    WriteFailed(String),
    /// Songs were added to a playlist, usually not the one on show, named here when it is
    /// listed.
    SongsAdded(Option<Arc<str>>),
}

/// The last song removed, which undoing puts back.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RemovedSong {
    playlist: PlaylistId,
    ix: usize,
    song: Song,
}

/// The user's playlists and the one open, whose changes show at once and are saved one at a time
/// behind. Starts reading the list when made. Dropping the last handle stops what is under way.
#[derive(Clone)]
pub struct PlaylistsState(Arc<Inner>);

struct WeakPlaylists(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: session::Session,
    state: Published<State>,
    notices: broadcast::Sender<PlaylistsNotice>,
}

struct State {
    playlists: PlaylistsSnapshot,
    open: Option<PlaylistId>,
    writes: WriteQueue,
    /// The playlist of the write under way.
    sending: Option<PlaylistId>,
    /// Counts the changes, so that what the server said before the latest of them, which it may
    /// not reflect, is not shown over it.
    edits: u64,
    /// Count the reads of each kind, so that one replaced while it was finishing on another
    /// thread cannot change the state after its successor has.
    list_id: u64,
    detail_id: u64,
    _list: Task<()>,
    _detail: Task<()>,
    _write: Task<()>,
}

impl Publish for State {
    type Snapshot = PlaylistsSnapshot;

    fn snapshot(&self) -> &PlaylistsSnapshot {
        &self.playlists
    }
}

impl State {
    fn has_unsaved(&self, playlist: &PlaylistId) -> bool {
        self.sending.as_ref() == Some(playlist) || self.writes.contains(playlist)
    }

    fn open_playlist_mut(&mut self) -> Option<&mut OpenPlaylist> {
        match &mut self.playlists.detail {
            PlaylistDetail::Loaded(open) => Some(open),
            PlaylistDetail::None | PlaylistDetail::Loading | PlaylistDetail::Failed(_) => None,
        }
    }

    /// The open playlist, if its songs may be changed: they are saved whole.
    fn editable_songs(&mut self, username: &str) -> Option<&mut OpenPlaylist> {
        if self.playlists.is_diverged {
            return None;
        }
        self.open_playlist_mut()
            .filter(|open| open.playlist.is_editable_by(username))
    }

    fn close(&mut self) {
        self.open = None;
        self.playlists.detail = PlaylistDetail::None;
        self.playlists.is_diverged = false;
    }

    /// Replaces the list, sorted by name, and closes the open playlist if it has gone.
    fn set_list(&mut self, mut playlists: Vec<Playlist>, username: &str) {
        playlists.sort_by_cached_key(|playlist| playlist.name().to_lowercase());
        self.playlists.choices = playlists
            .iter()
            .filter(|playlist| playlist.is_editable_by(username))
            .map(|playlist| PlaylistChoice {
                id: playlist.id().clone(),
                name: playlist.name().into(),
            })
            .collect();
        let keeps_open = self
            .open
            .as_ref()
            .is_none_or(|open| playlists.iter().any(|playlist| playlist.id() == open));
        self.playlists.list = PlaylistList::Loaded(Arc::new(playlists));
        if !keeps_open {
            self.close();
        }
    }

    /// Entries that didn't change keep their ids, which views keep their selection by.
    fn show_detail(&mut self, loaded: PlaylistWithSongs) {
        self.playlists.is_diverged = false;
        let (playlist, songs) = loaded.into_parts();
        match &mut self.playlists.detail {
            PlaylistDetail::Loaded(shown)
                if shown.playlist.id() == playlist.id() && shown.entries.has_songs(&songs) =>
            {
                shown.playlist = playlist;
            }
            PlaylistDetail::Loaded(shown) if shown.playlist.id() == playlist.id() => {
                shown.playlist = playlist;
                shown.entries.replace(songs);
            }
            detail => {
                *detail = PlaylistDetail::Loaded(OpenPlaylist {
                    playlist,
                    entries: Entries::new(songs),
                });
            }
        }
    }

    /// After a write to `playlist` failed: what follows was made on top of a change the server
    /// doesn't have.
    fn save_failed(&mut self, playlist: &PlaylistId) {
        self.writes.forget(playlist);
        self.edits += 1;
        if self
            .open_playlist_mut()
            .is_some_and(|open| open.playlist.id() == playlist)
        {
            self.playlists.is_diverged = true;
        }
        // The removal may not have happened, so that putting the song back would make two of it
        if self
            .playlists
            .removed
            .as_ref()
            .is_some_and(|removed| &removed.playlist == playlist)
        {
            self.playlists.removed = None;
        }
    }
}

impl PlaylistsState {
    pub fn new(executor: Executor, session: session::Session) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            session,
            state: Published::new(State {
                playlists: PlaylistsSnapshot {
                    list: PlaylistList::Loading,
                    refresh_error: None,
                    choices: Arc::from([]),
                    detail: PlaylistDetail::None,
                    is_diverged: false,
                    removed: None,
                },
                open: None,
                writes: WriteQueue::default(),
                sending: None,
                edits: 0,
                list_id: 0,
                detail_id: 0,
                _list: Task::ready(()),
                _detail: Task::ready(()),
                _write: Task::ready(()),
            }),
            notices: broadcast::Sender::new(NOTICES),
        }));
        this.0.state.update(|state| this.refresh_in(state));
        this
    }

    pub fn snapshot(&self) -> PlaylistsSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling the playlists,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<PlaylistsSnapshot> {
        self.0.state.subscribe()
    }

    /// Hears each failure from now on.
    pub fn notices(&self) -> broadcast::Receiver<PlaylistsNotice> {
        self.0.notices.subscribe()
    }

    /// Brings the list and the open playlist up to date with the server. Changes still being
    /// saved do that themselves once they are.
    pub fn refresh(&self) {
        self.0.state.update(|state| self.refresh_in(state));
    }

    /// Reads the list again after it failed to load.
    pub fn retry_list(&self) {
        self.0.state.update(|state| {
            state.playlists.list = PlaylistList::Loading;
            self.fetch_list(state);
        });
    }

    /// Reads `playlist`'s songs, showing it as loading until they come.
    pub fn open(&self, playlist: PlaylistId) {
        self.0.state.update(|state| {
            state.open = Some(playlist.clone());
            state.playlists.is_diverged = false;
            state.playlists.detail = PlaylistDetail::Loading;
            self.fetch_detail(state, playlist);
        });
    }

    pub fn close(&self) {
        self.0.state.update(State::close);
    }

    /// Puts the open playlist's entry at position `to_ix`, or the last if past the end. Whether
    /// that moved it.
    pub fn move_entry(&self, id: EntryId, to_ix: usize) -> bool {
        self.0.state.update(|state| {
            let username = self.0.session.username();
            let Some(open) = state.editable_songs(username) else {
                return false;
            };
            if !open.entries.move_entry(id, to_ix) {
                return false;
            }
            self.write_entries(state);
            true
        })
    }

    /// Takes the entry out of the open playlist, and says where it was. Undoing the last removal
    /// puts it back.
    pub fn remove_entry(&self, id: EntryId) -> Option<(usize, Song)> {
        self.0.state.update(|state| {
            let username = self.0.session.username();
            let open = state.editable_songs(username)?;
            let playlist = open.playlist.id().clone();
            let (ix, song) = open.entries.remove(id)?;
            self.write_entries(state);
            state.playlists.removed = Some(RemovedSong {
                playlist,
                ix,
                song: song.clone(),
            });
            Some((ix, song))
        })
    }

    /// Puts the song last removed back where it was. Its entry, when its playlist is open.
    pub fn undo_removal(&self) -> Option<EntryId> {
        self.0.state.update(|state| {
            let RemovedSong { playlist, ix, song } = state.playlists.removed.take()?;
            match state.open_playlist_mut() {
                Some(open) if open.playlist.id() == &playlist => {
                    let id = open.entries.insert(ix, song);
                    self.write_entries(state);
                    Some(id)
                }
                _ => {
                    let song = song.id().clone();
                    self.write(state, Write::InsertSong { playlist, ix, song });
                    None
                }
            }
        })
    }

    /// Adds `songs` after those already in `playlist`, and says so.
    pub fn add_songs(&self, playlist: &PlaylistId, songs: &[Song]) {
        if songs.is_empty() {
            return;
        }
        self.0.state.update(|state| {
            if let Some(open) = state
                .open_playlist_mut()
                .filter(|open| open.playlist.id() == playlist)
            {
                open.entries.extend(songs.iter().cloned());
            }
            let write = Write::AddSongs {
                playlist: playlist.clone(),
                songs: songs.iter().map(|song| song.id().clone()).collect(),
            };
            self.write(state, write);
            let name = state.playlists.loaded_list().and_then(|list| {
                let listed = list.iter().find(|listed| listed.id() == playlist)?;
                Some(Arc::from(listed.name()))
            });
            // Fails only when nobody listens
            self.0.notices.send(PlaylistsNotice::SongsAdded(name)).ok();
        });
    }

    pub fn rename(&self, playlist: PlaylistId, name: String) {
        self.0.state.update(|state| {
            if let Some(open) = state
                .open_playlist_mut()
                .filter(|open| open.playlist.id() == &playlist)
            {
                open.playlist = open.playlist.clone().with_name(&name);
            }
            let renamed = state.playlists.loaded_list().map(|list| {
                list.iter()
                    .map(|listed| {
                        if listed.id() == &playlist {
                            listed.clone().with_name(&name)
                        } else {
                            listed.clone()
                        }
                    })
                    .collect()
            });
            if let Some(renamed) = renamed {
                state.set_list(renamed, self.0.session.username());
            }
            self.write(state, Write::Rename { playlist, name });
        });
    }

    /// Takes the playlist off the list, closing it if it is open.
    pub fn delete(&self, playlist: PlaylistId) {
        self.0.state.update(|state| {
            let remaining = state.playlists.loaded_list().map(|list| {
                list.iter()
                    .filter(|listed| listed.id() != &playlist)
                    .cloned()
                    .collect()
            });
            if state.open.as_ref() == Some(&playlist) {
                state.close();
            }
            if let Some(remaining) = remaining {
                state.set_list(remaining, self.0.session.username());
            }
            self.write(state, Write::Delete { playlist });
        });
    }

    /// Creates a playlist starting with `songs`, which may be none, and lists it once the server
    /// has it: unlike other changes it has no id to show it by until then. The task ends with
    /// the new playlist's id or why it couldn't be made, and dropping it gives up on it.
    pub fn create(&self, name: String, songs: &[Song]) -> Task<Result<PlaylistId, String>> {
        let songs: Vec<SongId> = songs.iter().map(|song| song.id().clone()).collect();
        // A list on its way would not have the new playlist
        self.0.state.update(|state| state.edits += 1);
        let this = self.downgrade();
        let client = self.0.session.client().clone();
        self.0.executor.spawn(async move {
            let created = client
                .create_playlist(&name, &songs)
                .await
                .map(|created| created.playlist().clone())
                .map_err(|error| session::describe(&error));
            if let Some(this) = this.upgrade() {
                this.0
                    .state
                    .update(|state| this.playlist_created(state, created.as_ref()));
            }
            created.map(|created| created.id().clone())
        })
    }

    fn playlist_created(&self, state: &mut State, created: Result<&Playlist, &String>) {
        state.edits += 1;
        self.refresh_in(state);
        // Without a list, the one on its way has the new playlist
        if let (Ok(created), Some(list)) = (created, state.playlists.loaded_list()) {
            let mut playlists = list.to_vec();
            playlists.push(created.clone());
            state.set_list(playlists, self.0.session.username());
        }
    }

    fn refresh_in(&self, state: &mut State) {
        if state.sending.is_some() {
            return;
        }
        self.fetch_list(state);
        if let Some(open) = state.open.clone() {
            self.fetch_detail(state, open);
        }
    }

    fn fetch_list(&self, state: &mut State) {
        state.list_id += 1;
        let (list_id, edits) = (state.list_id, state.edits);
        let this = self.downgrade();
        let client = self.0.session.client().clone();
        state._list = self.0.executor.spawn(async move {
            let result = client.playlists().await;
            let Some(this) = this.upgrade() else {
                return;
            };
            this.0.state.update(|state| {
                if state.list_id != list_id {
                    return;
                }
                match result {
                    // A list from before the latest change would undo it, and the save of that
                    // change fetches another
                    Ok(playlists) if state.edits == edits => {
                        state.playlists.refresh_error = None;
                        state.set_list(playlists, this.0.session.username());
                    }
                    Ok(_) => {}
                    Err(error) => {
                        let error = session::describe(&error);
                        if state.playlists.loaded_list().is_some() {
                            state.playlists.refresh_error = Some(error);
                        } else {
                            state.playlists.list = PlaylistList::Failed(error);
                        }
                    }
                }
            });
        });
    }

    fn fetch_detail(&self, state: &mut State, playlist: PlaylistId) {
        state.detail_id += 1;
        let (detail_id, edits) = (state.detail_id, state.edits);
        let this = self.downgrade();
        let client = self.0.session.client().clone();
        state._detail = self.0.executor.spawn(async move {
            let result = client.playlist(&playlist).await;
            let Some(this) = this.upgrade() else {
                return;
            };
            this.0.state.update(|state| {
                if state.detail_id != detail_id || state.open.as_ref() != Some(&playlist) {
                    return;
                }
                // Edits would go on top of an answer that doesn't have the latest of them, and
                // undo them. Saving fetches another when it ends.
                if state.edits != edits || state.has_unsaved(&playlist) {
                    return;
                }
                match result {
                    Ok(loaded) => state.show_detail(loaded),
                    // Songs on show stay there, if they are what the server has
                    Err(_)
                        if state.playlists.open_playlist().is_some()
                            && !state.playlists.is_diverged => {}
                    Err(error) => {
                        state.playlists.is_diverged = false;
                        state.playlists.detail = PlaylistDetail::Failed(session::describe(&error));
                    }
                }
            });
        });
    }

    fn write_entries(&self, state: &mut State) {
        let Some(open) = state.playlists.open_playlist() else {
            return;
        };
        let write = Write::ReplaceSongs {
            playlist: open.playlist.id().clone(),
            songs: open.entries.song_ids(),
        };
        self.write(state, write);
    }

    fn write(&self, state: &mut State, write: Write) {
        state.edits += 1;
        state.writes.push(write);
        if state.sending.is_none() {
            self.write_next(state);
        }
    }

    /// Sends the next write. Once none is left, fetches what the server made of them, as only it
    /// knows a playlist's new totals and cover.
    fn write_next(&self, state: &mut State) {
        let Some(write) = state.writes.pop() else {
            state.sending = None;
            self.refresh_in(state);
            return;
        };
        state.sending = Some(write.playlist().clone());
        let this = self.downgrade();
        let client = self.0.session.client().clone();
        state._write = self.0.executor.spawn(async move {
            let result = write.clone().send(client).await;
            let Some(this) = this.upgrade() else {
                return;
            };
            this.0.state.update(|state| {
                if let Err(error) = result {
                    state.save_failed(write.playlist());
                    let message = format!("{} {}", write.failure(), session::describe(&error));
                    // Fails only when nobody listens
                    this.0
                        .notices
                        .send(PlaylistsNotice::WriteFailed(message))
                        .ok();
                }
                this.write_next(state);
            });
        });
    }

    fn downgrade(&self) -> WeakPlaylists {
        WeakPlaylists(Arc::downgrade(&self.0))
    }
}

impl WeakPlaylists {
    fn upgrade(&self) -> Option<PlaylistsState> {
        self.0.upgrade().map(PlaylistsState)
    }
}

#[cfg(test)]
mod tests;
