use std::sync::Arc;

use actions::{EnqueueSongs, Placement, PlaySongs};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    input::{InputEvent, InputState},
    menu::DropdownMenu as _,
    resizable::ResizableState,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollStrategy, SharedString,
    Styled as _, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _,
};
use subsonic::{Playlist, PlaylistId, PlaylistWithSongs, Song, SongId};
use ui::list_navigation::{CONTROL_CONTEXT, ListNavigationExt as _, Motion, page_rows};
use ui::{
    DetailHeader, ListFilter, ListPaneHeader, LoadFailure, MasterDetail, NowPlaying,
    RefreshFailure, centered, format_duration, format_song_count, join_details, muted_text,
    no_matches, row_list,
};

mod dialogs;
mod editing;
mod playlist_row;
mod song_row;

use dialogs::Chooser;
use editing::RemovedSong;
use playlist_row::PlaylistRow;
use song_row::SongRow;

use crate::entries::{Entries, EntryId};
use crate::filter::playlist_index;
use crate::writes::WriteQueue;
use crate::{
    AddToPlaylist, AddToQueue, Back, Delete, FocusFilter, MoveDown, MoveUp, Open,
    PLAYLISTS_CONTEXT, Play, PlayNext, ROOT_CONTEXT, Remove, Rename, SONGS_CONTEXT, ShowAlbum,
};

/// The renderer aliases an image drawn at less than half its size, so this stays
/// close to the on-screen size.
const COVER_PIXELS: u32 = 256;
/// The same as the player's.
const ROW_COVER_PIXELS: u32 = 48;

pub enum PlaylistsEvent {
    /// A playlist was created empty, and selected: it has nothing to show
    /// unless its screen does.
    CreatedEmpty,
    /// The choice of playlist asked for from the keyboard opened or closed.
    ChooserChanged,
    /// The user chose another playlist over this one.
    LeftPlaylist(PlaylistId),
}

pub struct Playlists {
    session: session::Session,
    list: List,
    refresh_error: Option<SharedString>,
    index: Arc<fuzzy::Index>,
    /// Positions in the loaded playlists that match the filter, the best first.
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
    _index_task: Task<()>,
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
            // Focus changes the selection treatment of the rows
            cx.on_focus(&list_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&list_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus(&songs_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&songs_focus, window, |_, _, cx| cx.notify()),
        ];
        // The last session's choices must not outlive it
        actions::set_playlist_choices(Vec::new(), cx);
        let mut this = Self {
            session,
            list: List::Loading,
            refresh_error: None,
            index: Arc::default(),
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
            _index_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        if let Some(readings) = session::readings(cx) {
            this._subscriptions.push(
                cx.subscribe(&readings, |this, _, _: &session::ReadingsChanged, cx| {
                    this.index_playlists(cx)
                }),
            );
        }
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
                // screen, and the save of that change fetches another.
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
        self.index_playlists(cx);
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
        self.visible = Arc::new(self.index.search(&query));
        cx.notify();
    }

    /// Makes the playlists searchable at once as far as they read without a
    /// dictionary, then, off the UI thread, as far as they read with one.
    fn index_playlists(&mut self, cx: &mut Context<Self>) {
        let Some(playlists) = self.loaded_list().cloned() else {
            return;
        };
        self.index = Arc::new(playlist_index(&playlists, None));
        self._index_task = Task::ready(());
        self.apply_filter(cx);
        let Some(book) = self.session.reading_book(cx) else {
            return;
        };
        self._index_task = cx.spawn(async move |this, cx| {
            let index = cx
                .background_spawn(async move {
                    book.read(|speller| playlist_index(&playlists, Some(speller)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.index = Arc::new(index);
                this.apply_filter(cx);
            })
            .ok();
        });
    }

    pub fn selected_playlist_id(&self) -> Option<&PlaylistId> {
        self.selected.as_ref()
    }

    /// Clears a filter that hides `playlist`. One that has gone leaves the selection
    /// as it is.
    pub fn reveal_playlist(
        &mut self,
        playlist: PlaylistId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = self
            .loaded_list()
            .and_then(|playlists| playlists.iter().position(|listed| listed.id() == &playlist))
        else {
            return;
        };
        if !self.visible.contains(&position) {
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.apply_filter(cx);
        }
        self.select(playlist, window, cx);
        // Selecting the playlist already selected does not scroll
        self.scroll_to_selected();
        self.list_focus.focus(window, cx);
    }

    fn scroll_to_selected(&self) {
        if let Some(position) = self.selected_position() {
            self.list_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
    }

    /// Selects the playlist the user chose. Moving through the list does not
    /// count: it would leave every playlist it passes to go back to.
    fn choose(&mut self, playlist: PlaylistId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(left) = self.selected.clone().filter(|left| *left != playlist) {
            cx.emit(PlaylistsEvent::LeftPlaylist(left));
        }
        self.select(playlist, window, cx);
    }

    fn select(&mut self, playlist: PlaylistId, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(&playlist) {
            return;
        }
        self.selected = Some(playlist.clone());
        self.scroll_to_selected();
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
                    // Songs on show stay there, if they are what the server has
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

    /// Rows that didn't change keep their identity, which also keeps their selection
    /// and any menu open on them.
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
                // Follows the selected song to where it is now
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

    pub fn min_width(&self, window: &Window, cx: &App) -> gpui_kit::Pixels {
        MasterDetail::min_width(&self.panes, window, cx)
    }

    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = NowPlaying::new(song, playing);
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    fn move_selection(&mut self, motion: Motion, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.visible.len();
        let page = page_rows(&self.list_scroll, len);
        let Some(position) = motion.apply(self.selected_position(), len, page) else {
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
        let page = page_rows(&self.songs_scroll, len);
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

    fn play_whole(&self, window: &mut Window, cx: &mut Context<Self>) {
        let songs = self
            .loaded_detail()
            .filter(|loaded| !loaded.entries.is_empty())
            .map(|loaded| loaded.entries.songs());
        if let Some(songs) = songs {
            window.dispatch_action(Box::new(PlaySongs::whole(songs)), cx);
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
            SongsCommand::Play => self.play_whole(window, cx),
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

    fn show_selected_artist(&self, window: &mut Window, cx: &mut Context<Self>) {
        let artist = self
            .selected_song()
            .and_then(|song| song.artist_id().cloned());
        if let Some(artist) = artist {
            window.dispatch_action(Box::new(actions::ShowArtist::new(artist)), cx);
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

    fn is_editable(&self) -> bool {
        self.selected_playlist()
            .is_some_and(|playlist| playlist.is_editable_by(self.session.username()))
    }

    /// Whether the songs on show can be changed, which saves all of them.
    fn can_edit_songs(&self) -> bool {
        self.is_editable() && !self.diverged
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
            let note = session::missing_readings_note(cx);
            return no_matches("No playlists match the filter.", note, cx).into_any_element();
        }

        let visible = self.visible.clone();
        let selected = self.selected.clone();
        let is_focused = self.list_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let list_focus = self.list_focus.clone();
        let username = self.session.username().to_owned();
        row_list(
            "playlist-list",
            "Playlists",
            visible.len(),
            &self.list_scroll,
            move |position| {
                let playlist = playlists.get(*visible.get(position)?)?;
                Some(PlaylistRow::new(
                    playlist,
                    &username,
                    selected.as_ref() == Some(playlist.id()),
                    is_focused,
                    owner.clone(),
                    list_focus.clone(),
                ))
            },
        )
        .size_full()
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
            .on_action(cx.listener(|this, _: &actions::GoToArtist, window, cx| {
                this.show_selected_artist(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &actions::ToggleFavorite, window, cx| {
                    if let Some(song) = this.selected_song() {
                        actions::toggle_favorite(actions::Favorite::Song(song), window, cx);
                    }
                }),
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
                        .on_click(cx.listener(|this, _, window, cx| this.play_whole(window, cx))),
                )
                .action(self.render_playlist_menu(editable, !songs.is_empty())),
            )
            .child(self.render_songs(loaded, editable, window, cx))
            .into_any_element()
    }

    /// The list's commands act on the playlist on show, so the menu dispatches them
    /// there, whichever list has focus.
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
        let is_focused = self.songs_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let songs_focus = self.songs_focus.clone();
        row_list(
            "playlist-songs",
            "Songs",
            entries.len(),
            &self.songs_scroll,
            move |ix| {
                let entry = entries.get(ix)?;
                let song = entry.song();
                Some(SongRow {
                    id: entry.id(),
                    song: song.clone(),
                    cover: song
                        .cover_art()
                        .map(|cover_art| session.cover_art_url(cover_art, ROW_COVER_PIXELS)),
                    ix,
                    len: entries.len(),
                    state: now_playing.as_ref().and_then(|now| now.state_of(song.id())),
                    is_editable: editable,
                    is_selected: selected == Some(entry.id()),
                    is_focused,
                    owner: owner.clone(),
                    songs_focus: songs_focus.clone(),
                })
            },
        )
        .flex_1()
        .min_h_0()
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

#[cfg(test)]
mod tests;
