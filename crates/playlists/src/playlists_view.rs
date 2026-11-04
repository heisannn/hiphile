use std::cmp::Ordering;
use std::sync::Arc;

use actions::{EnqueueSongs, Placement, PlaySongs};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
    WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    h_flex,
    input::{InputEvent, InputState},
    list::ListItem,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu},
    notification::{Notification, NotificationType},
    resizable::ResizableState,
    scroll::Scrollbar,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DismissEvent, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, UniformListScrollHandle, Window, anchored, deferred, div,
    prelude::FluentBuilder as _, uniform_list,
};
use subsonic::{Playlist, PlaylistId, PlaylistWithSongs, Song, SongId};
use ui::list_navigation::{CONTROL_CONTEXT, ListNavigationExt as _, Motion, page_rows};
use ui::{
    CoverArt, DetailHeader, DragPreview, ListFilter, ListPaneHeader, LoadFailure, MasterDetail,
    NowPlaying, PlayState, PlayStateIcon, RefreshFailure, RowText, centered, format_duration,
    format_song_count, join_details, muted_text,
};

use crate::entries::{Entries, EntryId};
use crate::filter::matching_playlists;
use crate::name_form::NameForm;
use crate::writes::{Write, WriteQueue};
use crate::{
    AddToPlaylist, AddToQueue, Back, Delete, FocusFilter, MoveDown, MoveUp, Open,
    PLAYLISTS_CONTEXT, Play, PlayNext, ROOT_CONTEXT, Remove, Rename, SONGS_CONTEXT, ShowAlbum,
};

/// Pixel size requested for the detail cover: twice its on-screen size, for
/// high-density displays.
const COVER_PIXELS: u32 = 512;
/// Pixel size requested for a song row's cover, likewise.
const ROW_COVER_PIXELS: u32 = 80;

pub enum PlaylistsEvent {
    /// A playlist was created empty, and selected: it has nothing to show
    /// unless its screen does.
    CreatedEmpty,
    /// The choice of playlist asked for from the keyboard opened or closed.
    ChooserChanged,
}

/// The server's playlists beside the songs of the selected one. Owns the
/// playlists, the selection, both lists' focus, and the writes on their way
/// to the server.
pub struct Playlists {
    session: session::Session,
    list: List,
    /// Why updating a list already on show failed.
    refresh_error: Option<SharedString>,
    /// Positions in the loaded playlists that match the filter.
    visible: Arc<Vec<usize>>,
    selected: Option<PlaylistId>,
    detail: Detail,
    selected_entry: Option<EntryId>,
    now_playing: Option<NowPlaying>,
    /// Waits for the selected playlist's songs.
    after_load: Option<SongsCommand>,
    filter: Entity<InputState>,
    list_focus: FocusHandle,
    songs_focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    songs_scroll: UniformListScrollHandle,
    /// The split between the list and the detail.
    panes: Entity<ResizableState>,
    writes: WriteQueue,
    /// The playlist of the write under way.
    sending: Option<PlaylistId>,
    /// Counts the changes made here, so that what the server said before the
    /// latest of them, which it may not reflect, is not shown over it.
    edits: u64,
    /// Whether a save failed on the playlist on show, which then shows what
    /// the server doesn't have until it has been read again.
    diverged: bool,
    /// What the Undo of the last song removed puts back.
    removed: Option<RemovedSong>,
    chooser: Option<Chooser>,
    _list_task: Task<()>,
    _detail_task: Task<()>,
    _write_task: Task<()>,
    _create_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

enum List {
    Loading,
    Loaded(Arc<Vec<Playlist>>),
    Failed(SharedString),
}

enum Detail {
    None,
    Loading,
    Loaded(LoadedPlaylist),
    Failed(SharedString),
}

struct LoadedPlaylist {
    playlist: Playlist,
    entries: Entries,
    cover_url: Option<SharedString>,
}

#[derive(Clone, Copy)]
enum SongsCommand {
    Play,
    Enqueue(Placement),
}

struct RemovedSong {
    playlist: PlaylistId,
    ix: usize,
    song: Song,
}

/// A menu of playlists, opened from the keyboard. It stands apart from any row
/// and from the screen, as the screen on show may be another feature's.
struct Chooser {
    menu: Entity<PopupMenu>,
    /// Where focus returns when it closes.
    previous_focus: Option<FocusHandle>,
    _dismissal: Subscription,
}

/// Identifies the notification of the last removal, which replaces the one
/// before: only the last removal can be undone.
struct SongRemoved;
/// Identifies the notification of the last addition.
struct SongsAdded;

impl EventEmitter<PlaylistsEvent> for Playlists {}

impl Playlists {
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter playlists"));
        let list_focus = cx.focus_handle().tab_stop(true);
        let songs_focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.apply_filter(cx),
                InputEvent::PressEnter { .. } => this.list_focus.focus(window, cx),
                _ => {}
            }),
            // Focus changes the selection treatment of the rows.
            cx.on_focus(&list_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&list_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus(&songs_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&songs_focus, window, |_, _, cx| cx.notify()),
        ];
        // The last session's choices must not outlive it.
        actions::set_playlist_choices(Vec::new(), cx);
        let mut this = Self {
            session,
            list: List::Loading,
            refresh_error: None,
            visible: Arc::default(),
            selected: None,
            detail: Detail::None,
            selected_entry: None,
            now_playing: None,
            after_load: None,
            filter,
            list_focus,
            songs_focus,
            list_scroll: UniformListScrollHandle::new(),
            songs_scroll: UniformListScrollHandle::new(),
            panes: cx.new(|_| ResizableState::default()),
            writes: WriteQueue::default(),
            sending: None,
            edits: 0,
            diverged: false,
            removed: None,
            chooser: None,
            _list_task: Task::ready(()),
            _detail_task: Task::ready(()),
            _write_task: Task::ready(()),
            _create_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.refresh(window, cx);
        this
    }

    /// Brings the list and the playlist on show up to date with the server.
    /// Changes still being saved do that themselves once they are.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending.is_some() {
            return;
        }
        self.fetch_list(window, cx);
        self.reload_detail(window, cx);
    }

    fn fetch_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.session.client().clone();
        let edits = self.edits;
        self._list_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.playlists().await })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                // A list from before the latest change would undo it on
                // screen; the save of that change fetches another.
                Ok(playlists) if this.edits == edits => {
                    this.refresh_error = None;
                    this.set_list(playlists, window, cx);
                }
                Ok(_) => {}
                Err(error) => {
                    let error = SharedString::from(session::describe(&error));
                    if this.loaded_list().is_some() {
                        this.refresh_error = Some(error);
                    } else {
                        this.list = List::Failed(error);
                    }
                    cx.notify();
                }
            })
            .ok();
        });
    }

    /// Replaces the list, sorted by name, keeping the selection while the
    /// playlist is still there.
    fn set_list(
        &mut self,
        mut playlists: Vec<Playlist>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        playlists.sort_by_cached_key(|playlist| playlist.name().to_lowercase());
        let username = self.session.username();
        actions::set_playlist_choices(
            playlists
                .iter()
                .filter(|playlist| playlist.is_editable_by(username))
                .map(|playlist| {
                    (
                        playlist.id().clone(),
                        SharedString::from(playlist.name().to_owned()),
                    )
                })
                .collect(),
            cx,
        );
        let keeps_selection = self
            .selected
            .as_ref()
            .is_some_and(|selected| playlists.iter().any(|playlist| playlist.id() == selected));
        self.list = List::Loaded(Arc::new(playlists));
        self.apply_filter(cx);
        if !keeps_selection {
            self.clear_selection();
            if let Some(first) = self
                .visible_playlist(0)
                .map(|playlist| playlist.id().clone())
            {
                self.select(first, window, cx);
            }
        }
        cx.notify();
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.selected_entry = None;
        self.detail = Detail::None;
        self.after_load = None;
        self.diverged = false;
    }

    fn loaded_list(&self) -> Option<&Arc<Vec<Playlist>>> {
        match &self.list {
            List::Loaded(playlists) => Some(playlists),
            _ => None,
        }
    }

    fn visible_playlist(&self, position: usize) -> Option<&Playlist> {
        self.loaded_list()?.get(*self.visible.get(position)?)
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        (0..self.visible.len()).find(|position| {
            self.visible_playlist(*position)
                .is_some_and(|playlist| playlist.id() == selected)
        })
    }

    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        self.visible = Arc::new(
            self.loaded_list()
                .map(|playlists| matching_playlists(playlists, &query))
                .unwrap_or_default(),
        );
        cx.notify();
    }

    fn select(&mut self, playlist: PlaylistId, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(&playlist) {
            return;
        }
        self.selected = Some(playlist.clone());
        if let Some(position) = self.selected_position() {
            self.list_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
        self.selected_entry = None;
        self.after_load = None;
        self.diverged = false;
        self.detail = Detail::Loading;
        self.songs_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.fetch_detail(playlist, window, cx);
        cx.notify();
    }

    fn fetch_detail(&mut self, playlist: PlaylistId, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.session.client().clone();
        let edits = self.edits;
        self._detail_task = cx.spawn_in(window, async move |this, cx| {
            let result = {
                let playlist = playlist.clone();
                cx.background_spawn(async move { client.playlist(&playlist).await })
                    .await
            };
            this.update_in(cx, |this, window, cx| {
                if this.selected.as_ref() != Some(&playlist) {
                    return;
                }
                // Edits would go on top of an answer that doesn't have the
                // latest of them, and undo them. Saving fetches another when
                // it ends.
                if this.edits != edits || this.has_unsaved(&playlist) {
                    return;
                }
                match result {
                    Ok(loaded) => this.show_detail(loaded, window, cx),
                    // Songs on show stay there, if they are what the server has.
                    Err(_) if this.loaded_detail().is_some() && !this.diverged => {}
                    Err(error) => {
                        this.after_load = None;
                        this.diverged = false;
                        this.detail = Detail::Failed(session::describe(&error).into());
                        cx.notify();
                    }
                }
            })
            .ok();
        });
    }

    /// Fetches the selected playlist again without leaving what is on show.
    fn reload_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.clone() {
            self.fetch_detail(selected, window, cx);
        }
    }

    fn retry_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected) = self.selected.take() {
            self.select(selected, window, cx);
        }
    }

    /// Shows what the server has. Rows that didn't change keep their identity,
    /// which also keeps their selection and any menu open on them.
    fn show_detail(
        &mut self,
        loaded: PlaylistWithSongs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.diverged = false;
        let (playlist, songs) = loaded.into_parts();
        let cover_url = playlist
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, COVER_PIXELS));
        match &mut self.detail {
            Detail::Loaded(shown)
                if shown.playlist.id() == playlist.id() && shown.entries.has_songs(&songs) =>
            {
                shown.playlist = playlist;
                shown.cover_url = cover_url;
            }
            detail => {
                let selected_song = self.selected_entry.and_then(|selected| {
                    let Detail::Loaded(shown) = &*detail else {
                        return None;
                    };
                    let ix = shown.entries.position(selected)?;
                    Some((ix, shown.entries.get(ix)?.song().id().clone()))
                });
                let entries = Entries::new(songs);
                // Follows the selected song to where it is now.
                self.selected_entry = selected_song.and_then(|(ix, song)| {
                    let same_place = entries
                        .get(ix)
                        .filter(|entry| entry.song().id() == &song)
                        .map(|entry| entry.id());
                    same_place.or_else(|| {
                        (0..entries.len())
                            .filter_map(|ix| entries.get(ix))
                            .find(|entry| entry.song().id() == &song)
                            .map(|entry| entry.id())
                    })
                });
                *detail = Detail::Loaded(LoadedPlaylist {
                    playlist,
                    entries,
                    cover_url,
                });
            }
        }
        if let Some(command) = self.after_load.take() {
            self.run_songs_command(command, window, cx);
        }
        cx.notify();
    }

    fn loaded_detail(&self) -> Option<&LoadedPlaylist> {
        match &self.detail {
            Detail::Loaded(loaded) => Some(loaded),
            _ => None,
        }
    }

    fn loaded_detail_mut(&mut self) -> Option<&mut LoadedPlaylist> {
        match &mut self.detail {
            Detail::Loaded(loaded) => Some(loaded),
            _ => None,
        }
    }

    /// The narrowest the screen can be while showing its list at its current
    /// width beside the detail.
    pub fn min_width(&self, window: &Window, cx: &App) -> gpui_kit::Pixels {
        MasterDetail::min_width(&self.panes, window, cx)
    }

    /// Marks `song` in the song list, or clears the mark.
    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = NowPlaying::new(song, playing);
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    fn move_selection(&mut self, motion: Motion, window: &mut Window, cx: &mut Context<Self>) {
        let page = page_rows(&self.list_scroll);
        let Some(position) = motion.apply(self.selected_position(), self.visible.len(), page)
        else {
            return;
        };
        if let Some(playlist) = self
            .visible_playlist(position)
            .map(|playlist| playlist.id().clone())
        {
            self.select(playlist, window, cx);
        }
    }

    fn selected_entry_ix(&self) -> Option<usize> {
        self.loaded_detail()?.entries.position(self.selected_entry?)
    }

    fn select_entry_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(id) = self
            .loaded_detail()
            .and_then(|loaded| loaded.entries.get(ix))
            .map(|entry| entry.id())
        else {
            return;
        };
        self.selected_entry = Some(id);
        self.songs_scroll
            .scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn select_entry(&mut self, id: EntryId, cx: &mut Context<Self>) {
        let ix = self
            .loaded_detail()
            .and_then(|loaded| loaded.entries.position(id));
        if let Some(ix) = ix {
            self.select_entry_at(ix, cx);
        }
    }

    fn move_entry_selection(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let len = self
            .loaded_detail()
            .map_or(0, |loaded| loaded.entries.len());
        let page = page_rows(&self.songs_scroll);
        if let Some(ix) = motion.apply(self.selected_entry_ix(), len, page) {
            self.select_entry_at(ix, cx);
        }
    }

    /// Moves keyboard focus to the songs, selecting the first one if none is.
    fn open_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .loaded_detail()
            .is_none_or(|loaded| loaded.entries.is_empty())
        {
            return;
        }
        if self.selected_entry_ix().is_none() {
            self.select_entry_at(0, cx);
        }
        self.songs_focus.focus(window, cx);
    }

    fn play_from(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let songs = self
            .loaded_detail()
            .filter(|loaded| ix < loaded.entries.len())
            .map(|loaded| loaded.entries.songs());
        if let Some(songs) = songs {
            window.dispatch_action(Box::new(PlaySongs::new(songs, ix)), cx);
        }
    }

    /// Runs `command` on the whole of the selected playlist, once its songs
    /// are here: a right click selects a row and opens its menu at once.
    fn on_playlist(&mut self, command: SongsCommand, window: &mut Window, cx: &mut Context<Self>) {
        match self.detail {
            Detail::Loaded(_) => self.run_songs_command(command, window, cx),
            Detail::Loading => self.after_load = Some(command),
            Detail::None | Detail::Failed(_) => {}
        }
    }

    fn run_songs_command(
        &self,
        command: SongsCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            SongsCommand::Play => self.play_from(0, window, cx),
            SongsCommand::Enqueue(placement) => {
                let songs = self
                    .loaded_detail()
                    .filter(|loaded| !loaded.entries.is_empty())
                    .map(|loaded| loaded.entries.songs());
                if let Some(songs) = songs {
                    window.dispatch_action(Box::new(EnqueueSongs::new(songs, placement)), cx);
                }
            }
        }
    }

    fn selected_song(&self) -> Option<Song> {
        let ix = self.selected_entry_ix()?;
        Some(self.loaded_detail()?.entries.get(ix)?.song().clone())
    }

    fn enqueue_selected_song(
        &self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(song) = self.selected_song() {
            window.dispatch_action(
                Box::new(EnqueueSongs::new(Arc::from([song]), placement)),
                cx,
            );
        }
    }

    fn show_selected_album(&self, window: &mut Window, cx: &mut Context<Self>) {
        let album = self
            .selected_song()
            .and_then(|song| song.album_id().cloned());
        if let Some(album) = album {
            window.dispatch_action(Box::new(actions::ShowAlbum::new(album)), cx);
        }
    }

    fn choose_playlist_for_selected_song(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(song) = self.selected_song() {
            window.dispatch_action(
                Box::new(actions::ChoosePlaylist::new(Arc::from([song]))),
                cx,
            );
        }
    }

    /// The selected playlist, whose songs may not be here yet.
    fn selected_playlist(&self) -> Option<&Playlist> {
        let selected = self.selected.as_ref()?;
        self.loaded_detail()
            .map(|loaded| &loaded.playlist)
            .or_else(|| {
                self.loaded_list()?
                    .iter()
                    .find(|listed| listed.id() == selected)
            })
    }

    /// Whether the signed-in user can change the selected playlist.
    fn is_editable(&self) -> bool {
        self.selected_playlist()
            .is_some_and(|playlist| playlist.is_editable_by(self.session.username()))
    }

    /// Whether the songs on show can be changed, which saves all of them.
    fn can_edit_songs(&self) -> bool {
        self.is_editable() && !self.diverged
    }

    /// Whether a change to `playlist` has yet to reach the server.
    fn has_unsaved(&self, playlist: &PlaylistId) -> bool {
        self.sending.as_ref() == Some(playlist) || self.writes.contains(playlist)
    }

    /// Queues `write` and starts on it unless another is going.
    fn write(&mut self, write: Write, window: &mut Window, cx: &mut Context<Self>) {
        self.edits += 1;
        self.writes.push(write);
        if self.sending.is_none() {
            self.write_next(window, cx);
        }
    }

    /// Sends the next write. Once none is left, fetches what the server made
    /// of them, as only it knows a playlist's new totals and cover.
    fn write_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(write) = self.writes.pop() else {
            self.sending = None;
            self.refresh(window, cx);
            return;
        };
        self.sending = Some(write.playlist().clone());
        let client = self.session.client().clone();
        self._write_task = cx.spawn_in(window, async move |this, cx| {
            let sent = write.clone();
            let result = cx
                .background_spawn(async move { sent.send(client).await })
                .await;
            this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.save_failed(write.playlist(), window, cx);
                    let message = format!("{} {}", write.failure(), session::describe(&error));
                    window.push_notification((NotificationType::Error, message), cx);
                }
                this.write_next(window, cx);
            })
            .ok();
        });
    }

    fn save_failed(&mut self, playlist: &PlaylistId, window: &mut Window, cx: &mut Context<Self>) {
        self.writes.forget(playlist);
        self.edits += 1;
        if self
            .loaded_detail()
            .is_some_and(|loaded| loaded.playlist.id() == playlist)
        {
            self.diverged = true;
        }
        // The removal may not have happened, so that putting the song back
        // would make two of it.
        if self
            .removed
            .as_ref()
            .is_some_and(|removed| &removed.playlist == playlist)
        {
            self.removed = None;
            window.remove_notification::<SongRemoved>(cx);
        }
    }

    /// Saves the song list on show after an edit to it.
    fn write_entries(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(loaded) = self.loaded_detail() else {
            return;
        };
        let write = Write::ReplaceSongs {
            playlist: loaded.playlist.id().clone(),
            songs: loaded.entries.song_ids(),
        };
        self.write(write, window, cx);
    }

    fn move_entry(
        &mut self,
        id: EntryId,
        to_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit_songs() {
            return;
        }
        let moved = self
            .loaded_detail_mut()
            .is_some_and(|loaded| loaded.entries.move_entry(id, to_ix));
        if moved {
            self.selected_entry = Some(id);
            self.write_entries(window, cx);
            self.select_entry(id, cx);
        }
    }

    fn move_selected_entry(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(id), Some(ix)) = (self.selected_entry, self.selected_entry_ix()) else {
            return;
        };
        if let Some(to_ix) = ix.checked_add_signed(delta) {
            self.move_entry(id, to_ix, window, cx);
        }
    }

    /// Moves the dragged entry to where `target` is: after it when dragged
    /// down, before it when dragged up, as the row's drop line shows.
    fn drop_entry(
        &mut self,
        dragged: EntryId,
        target: EntryId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let to_ix = self
            .loaded_detail()
            .and_then(|loaded| loaded.entries.position(target));
        if let Some(to_ix) = to_ix {
            self.songs_focus.focus(window, cx);
            self.move_entry(dragged, to_ix, window, cx);
        }
    }

    /// Takes the selected song out and offers to put it back.
    fn remove_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_edit_songs() {
            return;
        }
        let Some(id) = self.selected_entry else {
            return;
        };
        let Some(loaded) = self.loaded_detail_mut() else {
            return;
        };
        let playlist = loaded.playlist.id().clone();
        let Some((ix, song)) = loaded.entries.remove(id) else {
            return;
        };
        let next_ix = ix.min(loaded.entries.len().saturating_sub(1));
        self.selected_entry = None;
        self.select_entry_at(next_ix, cx);
        self.write_entries(window, cx);

        let message = format!("Removed “{}”", song.title());
        self.removed = Some(RemovedSong { playlist, ix, song });
        let owner = cx.entity().downgrade();
        window.push_notification(
            Notification::new()
                .id::<SongRemoved>()
                .message(message)
                .action(move |_, _, _| {
                    let owner = owner.clone();
                    Button::new("undo-remove")
                        .label("Undo")
                        .on_click(move |_, window, cx| {
                            owner
                                .update(cx, |this, cx| this.undo_removal(window, cx))
                                .ok();
                            window.remove_notification::<SongRemoved>(cx);
                        })
                })
                // An action alone keeps it up until it is dismissed.
                .autohide(true),
            cx,
        );
        cx.notify();
    }

    fn undo_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(RemovedSong { playlist, ix, song }) = self.removed.take() else {
            return;
        };
        let write = match self.loaded_detail_mut() {
            Some(loaded) if loaded.playlist.id() == &playlist => {
                let id = loaded.entries.insert(ix, song);
                self.selected_entry = Some(id);
                self.write_entries(window, cx);
                self.select_entry(id, cx);
                return;
            }
            _ => Write::InsertSong {
                playlist,
                ix,
                song: song.id().clone(),
            },
        };
        self.write(write, window, cx);
    }

    /// Adds `songs` after those already in `playlist`, and says so: the
    /// playlist is usually not on show.
    pub fn add_songs(
        &mut self,
        playlist: &PlaylistId,
        songs: &[Song],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if songs.is_empty() {
            return;
        }
        if let Some(loaded) = self
            .loaded_detail_mut()
            .filter(|loaded| loaded.playlist.id() == playlist)
        {
            loaded.entries.extend(songs.iter().cloned());
            cx.notify();
        }
        let write = Write::AddSongs {
            playlist: playlist.clone(),
            songs: songs.iter().map(|song| song.id().clone()).collect(),
        };
        self.write(write, window, cx);
        let name = self
            .loaded_list()
            .and_then(|list| list.iter().find(|listed| listed.id() == playlist))
            .map_or_else(
                || "the playlist".to_owned(),
                |listed| listed.name().to_owned(),
            );
        window.push_notification(
            Notification::new()
                .id::<SongsAdded>()
                .message(format!("Added to {name}")),
            cx,
        );
    }

    /// Asks for a name and creates a playlist starting with `songs`.
    pub fn open_new_playlist(
        &mut self,
        songs: Arc<[Song]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Its shortcut still works over a dialog.
        if window.has_active_dialog(cx) {
            return;
        }
        // Before the dialog, which returns focus to where it was opened from.
        self.close_chooser(window, cx);
        let form = self.new_playlist_form(songs, window, cx);
        open_name_dialog("New Playlist", form, window, cx);
    }

    fn new_playlist_form(
        &self,
        songs: Arc<[Song]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<NameForm> {
        let scope = match &*songs {
            [] => None,
            [song] => Some(format!("Starts with “{}”", song.title()).into()),
            songs => Some(format!("Starts with {} songs", songs.len()).into()),
        };
        let owner = cx.entity().downgrade();
        cx.new(|cx| {
            NameForm::new(
                "",
                "Create",
                scope,
                move |name, form, window, cx| {
                    owner
                        .update(cx, |this, cx| {
                            this.create_playlist(name, songs.clone(), form, window, cx);
                        })
                        .ok();
                },
                window,
                cx,
            )
        })
    }

    /// Creates the playlist while its dialog waits: unlike other edits it has
    /// no row to show until the server gives it an id.
    fn create_playlist(
        &mut self,
        name: String,
        songs: Arc<[Song]>,
        form: Entity<NameForm>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A list on its way would not have the new playlist.
        self.edits += 1;
        let client = self.session.client().clone();
        self._create_task = cx.spawn_in(window, async move |this, cx| {
            let ids: Vec<SongId> = songs.iter().map(|song| song.id().clone()).collect();
            let result = {
                let name = name.clone();
                cx.background_spawn(async move { client.create_playlist(&name, &ids).await })
                    .await
            };
            this.update_in(cx, |this, window, cx| {
                this.edits += 1;
                this.refresh(window, cx);
                match result {
                    Ok(created) => {
                        window.close_dialog(cx);
                        this.playlist_created(created, &name, songs.len(), window, cx);
                    }
                    Err(error) => form.update(cx, |form, cx| {
                        form.set_failed(session::describe(&error).into(), cx);
                    }),
                }
            })
            .ok();
        });
    }

    fn playlist_created(
        &mut self,
        created: PlaylistWithSongs,
        name: &str,
        song_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = created.playlist().id().clone();
        // Without a list, the one on its way has the new playlist.
        if let Some(mut playlists) = self.loaded_list().map(|list| list.to_vec()) {
            playlists.push(created.playlist().clone());
            self.set_list(playlists, window, cx);
        }
        if song_count == 0 {
            // It may be hidden by the filter, which it should not stay under.
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.apply_filter(cx);
            self.selected = None;
            self.select(id, window, cx);
            self.list_focus.focus(window, cx);
            cx.emit(PlaylistsEvent::CreatedEmpty);
        } else {
            let count = if song_count == 1 {
                "1 song".to_owned()
            } else {
                format!("{song_count} songs")
            };
            window.push_notification(
                Notification::new().message(format!("Created “{name}” with {count}")),
                cx,
            );
        }
    }

    fn open_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }
        let Some(selected) = self.selected_playlist() else {
            return;
        };
        let playlist = selected.id().clone();
        let name = selected.name().to_owned();
        let owner = cx.entity().downgrade();
        let form = cx.new(|cx| {
            NameForm::new(
                &name,
                "Rename",
                None,
                move |name, _, window, cx| {
                    window.close_dialog(cx);
                    owner
                        .update(cx, |this, cx| {
                            this.rename_playlist(playlist.clone(), name, window, cx);
                        })
                        .ok();
                },
                window,
                cx,
            )
        });
        open_name_dialog("Rename Playlist", form, window, cx);
    }

    fn rename_playlist(
        &mut self,
        playlist: PlaylistId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(loaded) = self
            .loaded_detail_mut()
            .filter(|loaded| loaded.playlist.id() == &playlist)
        {
            loaded.playlist = loaded.playlist.clone().with_name(&name);
        }
        let renamed = self.loaded_list().map(|list| {
            list.iter()
                .map(|listed| {
                    if listed.id() == &playlist {
                        listed.clone().with_name(&name)
                    } else {
                        listed.clone()
                    }
                })
                .collect::<Vec<_>>()
        });
        if let Some(renamed) = renamed {
            self.set_list(renamed, window, cx);
        }
        self.write(Write::Rename { playlist, name }, window, cx);
    }

    fn open_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }
        let Some(selected) = self.selected_playlist() else {
            return;
        };
        let playlist = selected.id().clone();
        let title = SharedString::from(format!("Delete “{}”?", selected.name()));
        let owner = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let playlist = playlist.clone();
            let owner = owner.clone();
            alert
                .title(title.clone())
                .description("The songs stay in your library. This can't be undone.")
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    owner
                        .update(cx, |this, cx| {
                            this.delete_playlist(playlist.clone(), window, cx);
                        })
                        .ok();
                    true
                })
        });
    }

    /// Takes the playlist off the list, selecting the one that takes its place.
    fn delete_playlist(
        &mut self,
        playlist: PlaylistId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.selected_position();
        let remaining = self.loaded_list().map(|list| {
            list.iter()
                .filter(|listed| listed.id() != &playlist)
                .cloned()
                .collect::<Vec<_>>()
        });
        if self.selected.as_ref() == Some(&playlist) {
            self.clear_selection();
        }
        if let Some(remaining) = remaining {
            self.set_list(remaining, window, cx);
        }
        if let Some(next) = position
            .and_then(|position| {
                self.visible_playlist(position)
                    .or_else(|| self.visible_playlist(position.checked_sub(1)?))
            })
            .map(|listed| listed.id().clone())
        {
            self.select(next, window, cx);
        }
        self.write(Write::Delete { playlist }, window, cx);
        self.list_focus.focus(window, cx);
    }

    /// Opens the choice of playlist for `songs`, taking focus until it closes.
    pub fn open_chooser(
        &mut self,
        songs: Arc<[Song]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if songs.is_empty() {
            return;
        }
        let previous_focus = window.focused(cx);
        let command = actions::add_songs_command(songs);
        let menu = PopupMenu::build(window, cx, move |menu, _, cx| {
            actions::playlist_choices(menu, &command, cx)
        });
        menu.focus_handle(cx).focus(window, cx);
        let dismissal = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_chooser(window, cx);
        });
        self.chooser = Some(Chooser {
            menu,
            previous_focus,
            _dismissal: dismissal,
        });
        cx.emit(PlaylistsEvent::ChooserChanged);
    }

    /// Returns focus to where it was, unless it has moved on from the menu.
    fn close_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chooser) = self.chooser.take() else {
            return;
        };
        if chooser.menu.focus_handle(cx).contains_focused(window, cx)
            && let Some(previous) = chooser.previous_focus
        {
            previous.focus(window, cx);
        }
        cx.emit(PlaylistsEvent::ChooserChanged);
    }

    /// The open choice of playlist, to draw over whichever screen is showing.
    pub fn render_chooser(&self, window: &Window) -> Option<AnyElement> {
        let chooser = self.chooser.as_ref()?;
        let viewport = window.viewport_size();
        Some(
            deferred(
                anchored()
                    .position(gpui_kit::point(viewport.width / 2., viewport.height / 4.))
                    .anchor(gpui_kit::Anchor::TopCenter)
                    .snap_to_window()
                    .child(chooser.menu.clone()),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn render_list_pane(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.loaded_list().map(|list| list.len());
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                ListPaneHeader::new("Playlists", count).action(
                    Button::new("new-playlist")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .accessibility_label("New Playlist…")
                        .tooltip_with_action("New Playlist…", &actions::NewPlaylist::empty(), None)
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(actions::NewPlaylist::empty()), cx);
                        }),
                ),
            )
            .when(count.is_some_and(|count| count > 0), |this| {
                this.child(ListFilter::new(&self.filter))
            })
            .when_some(self.refresh_error.clone(), |this, error| {
                this.child(RefreshFailure::new(
                    "Couldn't refresh playlists",
                    error,
                    Button::new("retry-refresh-playlists")
                        .label("Retry")
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.refresh(window, cx);
                        })),
                ))
            })
            .child(self.render_list(window, cx))
            .into_any_element()
    }

    /// The list's focus target, rendered in every state so focus and the
    /// commands bound above it never land on an element that is gone.
    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let editable = self.is_editable();
        div()
            .id("playlists")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .track_focus(&self.list_focus)
            .key_context(PLAYLISTS_CONTEXT)
            .on_list_navigation(
                |this, motion, window, cx| this.move_selection(motion, window, cx),
                cx,
            )
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open_playlist(window, cx)))
            .on_action(cx.listener(|this, _: &Play, window, cx| {
                this.on_playlist(SongsCommand::Play, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.on_playlist(SongsCommand::Enqueue(Placement::Next), window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.on_playlist(SongsCommand::Enqueue(Placement::Last), window, cx)
            }))
            .when(editable, |this| {
                this.on_action(
                    cx.listener(|this, _: &Rename, window, cx| this.open_rename(window, cx)),
                )
                .on_action(cx.listener(|this, _: &Delete, window, cx| this.open_delete(window, cx)))
            })
            .child(self.render_rows(window, cx))
            .into_any_element()
    }

    fn render_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let playlists = match &self.list {
            List::Loading => return centered(Spinner::new()).into_any_element(),
            List::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load playlists",
                    error.clone(),
                    Button::new("retry-playlists")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.list_focus.focus(window, cx);
                            this.list = List::Loading;
                            this.fetch_list(window, cx);
                        })),
                )
                .into_any_element();
            }
            List::Loaded(playlists) => playlists.clone(),
        };
        if playlists.is_empty() {
            return centered(
                v_flex()
                    .items_center()
                    .gap_3()
                    .child(muted_text("No playlists yet.", cx))
                    .child(
                        div().key_context(CONTROL_CONTEXT).child(
                            Button::new("new-first-playlist")
                                .small()
                                .label("New Playlist…")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(actions::NewPlaylist::empty()),
                                        cx,
                                    );
                                }),
                        ),
                    ),
            )
            .into_any_element();
        }
        if self.visible.is_empty() {
            return centered(muted_text("No playlists match the filter.", cx)).into_any_element();
        }

        let visible = self.visible.clone();
        let selected = self.selected.clone();
        let focused = self.list_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let list_focus = self.list_focus.clone();
        let username = self.session.username().to_owned();
        div()
            .id("playlist-list")
            .relative()
            .size_full()
            .role(gpui_kit::Role::List)
            .aria_label("Playlists")
            .child(
                uniform_list("playlist-rows", visible.len(), move |range, _, _| {
                    range
                        .filter_map(|position| {
                            let playlist = playlists.get(*visible.get(position)?)?;
                            let id = playlist.id().clone();
                            let is_selected = selected.as_ref() == Some(&id);
                            let name = SharedString::from(playlist.name().to_owned());
                            let details = playlist_details(playlist, &username);
                            let editable = playlist.is_editable_by(&username);
                            let has_songs = playlist.song_count() != Some(0);
                            let list_focus = list_focus.clone();
                            let on_click = owner.clone();
                            let on_right_click = owner.clone();
                            let click_id = id.clone();
                            Some(
                                ListItem::new(ElementId::Name(format!("playlist-{id}").into()))
                                    .selected(is_selected)
                                    .secondary_selected(focused && is_selected)
                                    .h_12()
                                    .px_4()
                                    .w_full()
                                    .accessibility_label(name.clone())
                                    .child(RowText::new(name).details(details))
                                    .on_click(move |_, window, cx| {
                                        on_click
                                            .update(cx, |this, cx| {
                                                this.list_focus.focus(window, cx);
                                                this.select(click_id.clone(), window, cx);
                                            })
                                            .ok();
                                    })
                                    // The menu acts on the selection, so the
                                    // row under the pointer becomes it first.
                                    .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                                        on_right_click
                                            .update(cx, |this, cx| {
                                                this.list_focus.focus(window, cx);
                                                this.select(id.clone(), window, cx);
                                            })
                                            .ok();
                                    })
                                    .context_menu(move |menu, _, _| {
                                        menu.action_context(list_focus.clone())
                                            .menu_with_disabled("Play", Box::new(Play), !has_songs)
                                            .menu_with_disabled(
                                                "Play Next",
                                                Box::new(PlayNext),
                                                !has_songs,
                                            )
                                            .menu_with_disabled(
                                                "Add to Queue",
                                                Box::new(AddToQueue),
                                                !has_songs,
                                            )
                                            .when(editable, |menu| {
                                                menu.separator()
                                                    .menu("Rename…", Box::new(Rename))
                                                    .menu("Delete…", Box::new(Delete))
                                            })
                                    }),
                            )
                        })
                        .collect()
                })
                .size_full()
                .track_scroll(&self.list_scroll),
            )
            .child(Scrollbar::vertical(&self.list_scroll))
            .into_any_element()
    }

    /// The detail pane is the songs' focus target in every state, for the
    /// same reason as [`Self::render_list`].
    fn render_detail(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let editable = self.is_editable();
        div()
            .id("playlist-detail")
            .size_full()
            .min_w_0()
            .min_h_0()
            .track_focus(&self.songs_focus)
            .key_context(SONGS_CONTEXT)
            .on_list_navigation(
                |this, motion, _, cx| this.move_entry_selection(motion, cx),
                cx,
            )
            .on_action(cx.listener(|this, _: &Open, window, cx| {
                if let Some(ix) = this.selected_entry_ix() {
                    this.play_from(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.enqueue_selected_song(Placement::Next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.enqueue_selected_song(Placement::Last, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ShowAlbum, window, cx| this.show_selected_album(window, cx)),
            )
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.choose_playlist_for_selected_song(window, cx)
            }))
            .on_action(cx.listener(|this, _: &Back, window, cx| {
                this.list_focus.focus(window, cx);
            }))
            .when(editable, |this| {
                this.on_action(cx.listener(|this, _: &MoveUp, window, cx| {
                    this.move_selected_entry(-1, window, cx)
                }))
                .on_action(cx.listener(|this, _: &MoveDown, window, cx| {
                    this.move_selected_entry(1, window, cx)
                }))
                .on_action(cx.listener(|this, _: &Remove, window, cx| {
                    this.remove_selected_entry(window, cx)
                }))
            })
            .child(self.render_detail_content(window, cx))
            .into_any_element()
    }

    fn render_detail_content(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let loaded = match &self.detail {
            Detail::None => return div().size_full().into_any_element(),
            Detail::Loading => return centered(Spinner::new()).into_any_element(),
            Detail::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load this playlist",
                    error.clone(),
                    Button::new("retry-playlist")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.songs_focus.focus(window, cx);
                            this.retry_detail(window, cx);
                        })),
                )
                .into_any_element();
            }
            Detail::Loaded(loaded) => loaded,
        };
        let playlist = &loaded.playlist;
        let username = self.session.username();
        let editable = playlist.is_editable_by(username);
        let songs = loaded.entries.songs();
        let total: Option<std::time::Duration> = songs.iter().map(Song::duration).sum();
        let total = total.or_else(|| playlist.duration());
        let summary = join_details([
            Some(format_song_count(songs.len())),
            total.filter(|_| !songs.is_empty()).map(format_duration),
            (!editable).then(|| "Readonly".to_owned()),
        ]);
        let owner = playlist.owner().filter(|_| !playlist.is_owned_by(username));

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                DetailHeader::new(
                    loaded.cover_url.clone(),
                    playlist.name().to_owned().into(),
                    summary,
                )
                .subtitle(owner.map(|owner| owner.to_owned().into()))
                .comment(playlist.comment().map(|comment| comment.to_owned().into()))
                .action(
                    Button::new("play-playlist")
                        .icon(IconName::Play)
                        .label("Play")
                        .disabled(songs.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.play_from(0, window, cx))),
                )
                .action(self.render_playlist_menu(editable, !songs.is_empty())),
            )
            .child(self.render_songs(loaded, editable, window, cx))
            .into_any_element()
    }

    /// The playlist's commands besides Play.
    ///
    /// The list's commands act on the playlist on show, so the menu dispatches
    /// them there, whichever list has focus.
    fn render_playlist_menu(&self, editable: bool, has_songs: bool) -> impl IntoElement {
        let list_focus = self.list_focus.clone();
        Button::new("playlist-menu")
            .ghost()
            .icon(IconName::Ellipsis)
            .accessibility_label("More")
            .disabled(!editable && !has_songs)
            .dropdown_menu(move |menu, _, _| {
                menu.action_context(list_focus.clone())
                    .menu_with_disabled("Play Next", Box::new(PlayNext), !has_songs)
                    .menu_with_disabled("Add to Queue", Box::new(AddToQueue), !has_songs)
                    .when(editable, |menu| {
                        menu.separator()
                            .menu("Rename…", Box::new(Rename))
                            .menu("Delete…", Box::new(Delete))
                    })
            })
    }

    fn render_songs(
        &self,
        loaded: &LoadedPlaylist,
        editable: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if loaded.entries.is_empty() {
            return centered(
                v_flex()
                    .items_center()
                    .gap_1()
                    .child(div().text_sm().font_medium().child("No songs yet"))
                    .child(muted_text(
                        "Choose Add to Playlist on any song or album.",
                        cx,
                    )),
            )
            .into_any_element();
        }
        let entries = loaded.entries.shared();
        let session = self.session.clone();
        let selected = self.selected_entry;
        let now_playing = self.now_playing.clone();
        let focused = self.songs_focus.is_focused(window);
        let muted_foreground = cx.theme().muted_foreground;
        let owner = cx.entity().downgrade();
        let songs_focus = self.songs_focus.clone();
        div()
            .id("playlist-songs")
            .relative()
            .flex_1()
            .min_h_0()
            .role(gpui_kit::Role::List)
            .aria_label("Songs")
            .child(
                uniform_list("playlist-song-rows", entries.len(), move |range, _, _| {
                    let len = entries.len();
                    range
                        .filter_map(|ix| {
                            let entry = entries.get(ix)?;
                            let id = entry.id();
                            let song = entry.song().clone();
                            let is_selected = selected == Some(id);
                            let title = SharedString::from(song.title().to_owned());
                            let details = join_details([song.artist(), song.album()]);
                            let duration = song.duration().map(format_duration).unwrap_or_default();
                            let cover = song.cover_art().map(|cover_art| {
                                session.cover_art_url(cover_art, ROW_COVER_PIXELS)
                            });
                            // The playing song shows a sound icon in place of
                            // its number.
                            let marker =
                                now_playing.as_ref().and_then(|now| now.state_of(song.id()));
                            let label = PlayState::row_label(&title, marker);
                            let has_album = song.album_id().is_some();
                            let on_click = owner.clone();
                            let on_right_click = owner.clone();
                            let on_drop = owner.clone();
                            let songs_focus = songs_focus.clone();
                            let dragged_title = title.clone();
                            let row = ListItem::new(ElementId::NamedInteger(
                                "playlist-entry".into(),
                                id.get(),
                            ))
                            .selected(is_selected)
                            .secondary_selected(focused && is_selected)
                            .h_12()
                            .px_6()
                            .w_full()
                            .accessibility_label(label)
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .w_6()
                                            .flex_none()
                                            .justify_end()
                                            .text_sm()
                                            .text_color(muted_foreground)
                                            .map(|this| match marker {
                                                Some(state) => {
                                                    this.child(PlayStateIcon::new(state))
                                                }
                                                None => this.child((ix + 1).to_string()),
                                            }),
                                    )
                                    .child(CoverArt::new(cover).small())
                                    .child(RowText::new(title).details(details))
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_sm()
                                            .text_color(muted_foreground)
                                            .child(duration),
                                    ),
                            )
                            .on_click(move |event, window, cx| {
                                on_click
                                    .update(cx, |this, cx| {
                                        this.songs_focus.focus(window, cx);
                                        this.select_entry(id, cx);
                                        if event.click_count() >= 2
                                            && let Some(ix) = this.selected_entry_ix()
                                        {
                                            this.play_from(ix, window, cx);
                                        }
                                    })
                                    .ok();
                            })
                            // The menu acts on the selection, so the row
                            // under the pointer becomes it first.
                            .on_mouse_down(
                                MouseButton::Right,
                                move |_, window, cx| {
                                    on_right_click
                                        .update(cx, |this, cx| {
                                            this.songs_focus.focus(window, cx);
                                            this.select_entry(id, cx);
                                        })
                                        .ok();
                                },
                            );
                            let row = if editable {
                                row.on_drag(
                                    DraggedEntry {
                                        id,
                                        from_ix: ix,
                                        title: dragged_title,
                                    },
                                    |dragged, offset, _, cx| {
                                        cx.new(|_| DragPreview::new(dragged.title.clone(), offset))
                                    },
                                )
                                // A line on the edge the entry would land beside.
                                .drag_over::<DraggedEntry>(move |style, dragged, _, cx| {
                                    let line = cx.theme().drag_border;
                                    match dragged.from_ix.cmp(&ix) {
                                        Ordering::Less => style.border_b_2().border_color(line),
                                        Ordering::Greater => style.border_t_2().border_color(line),
                                        Ordering::Equal => style,
                                    }
                                })
                                .on_drop(
                                    move |dragged: &DraggedEntry, window, cx| {
                                        on_drop
                                            .update(cx, |this, cx| {
                                                this.drop_entry(dragged.id, id, window, cx)
                                            })
                                            .ok();
                                    },
                                )
                            } else {
                                row
                            };
                            Some(row.context_menu(move |menu, window, cx| {
                                let menu = menu
                                    .action_context(songs_focus.clone())
                                    .menu("Play", Box::new(Open))
                                    .menu("Play Next", Box::new(PlayNext))
                                    .menu("Add to Queue", Box::new(AddToQueue))
                                    .when(editable, |menu| {
                                        menu.separator()
                                            .menu_with_disabled(
                                                "Move Up",
                                                Box::new(MoveUp),
                                                ix == 0,
                                            )
                                            .menu_with_disabled(
                                                "Move Down",
                                                Box::new(MoveDown),
                                                ix + 1 == len,
                                            )
                                    })
                                    .separator();
                                let song = song.clone();
                                let menu = actions::add_to_playlist_submenu(
                                    menu,
                                    actions::add_songs_command(Arc::from([song])),
                                    window,
                                    cx,
                                );
                                menu.when(has_album, |menu| {
                                    menu.menu("Show Album", Box::new(ShowAlbum))
                                })
                                .when(editable, |menu| {
                                    menu.separator()
                                        .menu("Remove from Playlist", Box::new(Remove))
                                })
                            }))
                        })
                        .collect()
                })
                .size_full()
                .track_scroll(&self.songs_scroll),
            )
            .child(Scrollbar::vertical(&self.songs_scroll))
            .into_any_element()
    }
}

impl Focusable for Playlists {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.list_focus.clone()
    }
}

impl Render for Playlists {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("playlists-root")
            .key_context(ROOT_CONTEXT)
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter.update(cx, |input, cx| input.focus(window, cx));
            }))
            .child(MasterDetail::new(
                "playlists-panes",
                &self.panes,
                self.render_list_pane(window, cx),
                self.render_detail(window, cx),
            ))
    }
}

/// A row being dragged to a new place in the playlist.
struct DraggedEntry {
    id: EntryId,
    /// Where it was when the drag started, which tells the rows under the
    /// pointer which side it would land on.
    from_ix: usize,
    title: SharedString,
}

fn open_name_dialog(
    title: &'static str,
    form: Entity<NameForm>,
    window: &mut Window,
    cx: &mut App,
) {
    let body = form.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let submitted = body.clone();
        let submitting = body.clone();
        dialog
            .title(title)
            .child(body.clone())
            // Enter reaches the dialog from the input. The form decides whether
            // that closes it: a failure has to stay on show.
            .on_ok(move |_, window, cx| {
                submitted.update(cx, |form, cx| form.submit(window, cx));
                false
            })
            // What is being saved goes through either way, so it can't be
            // cancelled.
            .on_cancel(move |_, _, cx| !submitting.read(cx).is_submitting())
    });
    form.update(cx, |form, cx| form.focus_input(window, cx));
}

/// The second line of a playlist's row.
fn playlist_details(playlist: &Playlist, username: &str) -> SharedString {
    let owner = playlist
        .owner()
        .filter(|_| !playlist.is_owned_by(username))
        .map(str::to_owned);
    let count = playlist
        .song_count()
        .map(|count| format_song_count(count as usize));
    let duration = playlist
        .duration()
        .filter(|_| playlist.song_count() != Some(0))
        .map(format_duration);
    join_details([owner, count, duration])
}

#[cfg(test)]
mod tests;
