use std::sync::Arc;

use actions::{
    AlbumAction, EnqueueSongs, GoToArtist, Placement, PlaySongs, ShowArtist, ToggleFavorite,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{InputEvent, InputState},
    menu::DropdownMenu as _,
    resizable::ResizableState,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollStrategy, SharedString,
    Styled as _, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _,
};
use hiphile_core::favorites::Favorite;
use hiphile_core::library::{
    AlbumOrder, AlbumSort, Albums, LibraryNotice, LibrarySync, SyncStatus, album_index, arrange,
};
use subsonic::{Album, AlbumId, AlbumWithSongs, PlaylistId, Song, SongId};
use ui::list_navigation::{ListNavigationExt as _, Motion, page_rows};
use ui::{
    DetailHeader, ListFilter, ListPaneHeader, LoadFailure, MasterDetail, NowPlaying,
    RefreshFailure, SortOption, album_artist_link, centered, format_duration, format_song_count,
    join_details, muted_text, no_matches, row_list, sort_menu,
};

mod album_row;
mod song_row;

use album_row::AlbumRow;
use song_row::SongRow;

use crate::{
    ALBUMS_CONTEXT, AddAlbumToPlaylist, AddToPlaylist, AddToQueue, Back, FocusFilter,
    LIBRARY_CONTEXT, Open, Play, PlayNext, Refresh, SONGS_CONTEXT,
};

/// The renderer aliases an image drawn at less than half its size, so this stays
/// close to the on-screen size.
const COVER_PIXELS: u32 = 256;

pub enum LibraryEvent {
    /// The user chose another album over this one.
    LeftAlbum(AlbumId),
}

pub struct Library {
    session: session::Session,
    sync: LibrarySync,
    /// What the sync showed last.
    status: SyncStatus,
    /// The albums the list shows, which follow the sync's.
    albums: Albums,
    index: Arc<fuzzy::Index>,
    /// Positions in the loaded albums that match the filter, in the order they show.
    visible: Arc<Vec<usize>>,
    sort: AlbumSort,
    selected_album: Option<AlbumId>,
    /// The selected album, when it was asked to show before the list had it.
    /// It stays selected and comes into view once a sync brings it.
    revealing: Option<AlbumId>,
    detail: Detail,
    selected_song: Option<SongId>,
    now_playing: Option<NowPlaying>,
    filter: Entity<InputState>,
    albums_focus: FocusHandle,
    songs_focus: FocusHandle,
    albums_scroll: UniformListScrollHandle,
    songs_scroll: UniformListScrollHandle,
    panes: Entity<ResizableState>,
    _album_task: Task<()>,
    _album_action_task: Task<()>,
    _index_task: Task<()>,
    _subscriptions: Vec<Subscription>,
    _sync_changes: Task<()>,
    _sync_notices: Task<()>,
    _readings_changes: Option<Task<()>>,
}

enum Detail {
    None,
    Loading,
    Loaded(Box<LoadedAlbum>),
    Failed(SharedString),
}

struct LoadedAlbum {
    album: Album,
    songs: Arc<[Song]>,
    cover_url: Option<SharedString>,
}

impl EventEmitter<LibraryEvent> for Library {}

impl Library {
    pub fn new(
        session: session::Session,
        sync: LibrarySync,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter albums"));
        let albums_focus = cx.focus_handle().tab_stop(true);
        let songs_focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            // The album on show has a star that says
            actions::observe_favorites(cx),
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.apply_filter(cx),
                InputEvent::PressEnter { .. } => this.albums_focus.focus(window, cx),
                _ => {}
            }),
            // Focus changes the selection treatment of the rows
            cx.on_focus(&albums_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&albums_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus(&songs_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&songs_focus, window, |_, _, cx| cx.notify()),
        ];
        let status = sync.status();
        let sync_changes = gpui_host::observe(sync.subscribe(), cx, Self::follow_sync);
        let sync_notices =
            gpui_host::subscribe(sync.notices(), cx, |this: &mut Self, notice, cx| {
                if notice == LibraryNotice::SyncFinished {
                    this.sync_finished(cx);
                }
            });
        let mut this = Self {
            session,
            sync,
            status: status.clone(),
            albums: Albums::Loading,
            index: Arc::default(),
            visible: Arc::default(),
            sort: AlbumSort::default(),
            selected_album: None,
            revealing: None,
            detail: Detail::None,
            selected_song: None,
            now_playing: None,
            filter,
            albums_focus,
            songs_focus,
            albums_scroll: UniformListScrollHandle::new(),
            songs_scroll: UniformListScrollHandle::new(),
            panes: cx.new(|_| ResizableState::default()),
            _album_task: Task::ready(()),
            _album_action_task: Task::ready(()),
            _index_task: Task::ready(()),
            _subscriptions: subscriptions,
            _sync_changes: sync_changes,
            _sync_notices: sync_notices,
            _readings_changes: None,
        };
        this._readings_changes = this.session.readings().map(|readings| {
            ui::on_readings_changed(readings, cx, |this: &mut Self, cx| this.index_albums(cx))
        });
        this.show_albums(status.albums().clone(), cx);
        this
    }

    /// Fetches the whole library from the server again.
    pub fn refresh_library(&self) {
        self.sync.refresh();
    }

    fn follow_sync(&mut self, status: SyncStatus, cx: &mut Context<Self>) {
        if status.albums() != self.status.albums() {
            self.show_albums(status.albums().clone(), cx);
        }
        self.status = status;
        cx.notify();
    }

    /// Gives up on an album being revealed that the sync did not bring. The albums it brought
    /// come first, as their change may not have been followed yet.
    fn sync_finished(&mut self, cx: &mut Context<Self>) {
        self.follow_sync(self.sync.status(), cx);
        self.revealing = None;
    }

    fn show_albums(&mut self, albums: Albums, cx: &mut Context<Self>) {
        match albums {
            Albums::Loaded(albums) => self.set_albums(albums, cx),
            Albums::Loading => {
                self.albums = Albums::Loading;
                self.visible = Arc::default();
            }
            Albums::Failed(error) => self.albums = Albums::Failed(error),
        }
    }

    /// Replaces the album list, keeping the selection while the album is
    /// still there or a sync may yet bring it.
    fn set_albums(&mut self, albums: Arc<[Album]>, cx: &mut Context<Self>) {
        let keeps_selection = self.revealing.is_some()
            || self
                .selected_album
                .as_ref()
                .is_some_and(|selected| albums.iter().any(|album| album.id() == selected));
        let revealed = self
            .revealing
            .as_ref()
            .is_some_and(|revealing| albums.iter().any(|album| album.id() == revealing));
        self.albums = Albums::Loaded(albums);
        self.index_albums(cx);
        if !keeps_selection {
            self.selected_album = None;
            self.detail = Detail::None;
            if let Some(first) = self.visible_album(0).map(|album| album.id().clone()) {
                self.select_album(first, cx);
            }
        }
        if revealed {
            self.revealing = None;
            self.scroll_to_selected_album();
        }
        cx.notify();
    }

    fn scroll_to_selected_album(&self) {
        if let Some(position) = self.selected_position() {
            self.albums_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
    }

    fn loaded_albums(&self) -> Option<&Arc<[Album]>> {
        match &self.albums {
            Albums::Loaded(albums) => Some(albums),
            _ => None,
        }
    }

    fn visible_album(&self, position: usize) -> Option<&Album> {
        let albums = self.loaded_albums()?;
        albums.get(*self.visible.get(position)?)
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected_album.as_ref()?;
        (0..self.visible.len()).find(|position| {
            self.visible_album(*position)
                .is_some_and(|album| album.id() == selected)
        })
    }

    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        let found = self.index.search(&query);
        self.visible = Arc::new(match self.loaded_albums() {
            Some(albums) => arrange(albums, found, self.sort),
            None => found,
        });
        cx.notify();
    }

    fn set_sort(&mut self, sort: AlbumSort, cx: &mut Context<Self>) {
        if self.sort != sort {
            self.sort = sort;
            self.apply_filter(cx);
            self.scroll_to_selected_album();
        }
    }

    /// Makes the albums searchable at once as far as they read without a
    /// dictionary, then, off the UI thread, as far as they read with one.
    fn index_albums(&mut self, cx: &mut Context<Self>) {
        let Some(albums) = self.loaded_albums().cloned() else {
            return;
        };
        self.index = Arc::new(album_index(&albums, None));
        self._index_task = Task::ready(());
        self.apply_filter(cx);
        let Some(book) = self.session.reading_book() else {
            return;
        };
        self._index_task = cx.spawn(async move |this, cx| {
            let index = cx
                .background_spawn(async move {
                    book.read(|speller| album_index(&albums, Some(speller)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.index = Arc::new(index);
                this.apply_filter(cx);
            })
            .ok();
        });
    }

    pub fn selected_album(&self) -> Option<&AlbumId> {
        self.selected_album.as_ref()
    }

    /// Selects the album the user chose. Moving through the list does not
    /// count: it would leave every album it passes to go back to.
    fn choose_album(&mut self, album: AlbumId, cx: &mut Context<Self>) {
        if let Some(left) = self.selected_album.clone().filter(|left| *left != album) {
            cx.emit(LibraryEvent::LeftAlbum(left));
        }
        self.select_album(album, cx);
    }

    fn select_album(&mut self, album: AlbumId, cx: &mut Context<Self>) {
        if self.selected_album.as_ref() == Some(&album) {
            return;
        }
        self.revealing = None;
        if let Some(position) = self.visible.iter().position(|ix| {
            self.loaded_albums()
                .and_then(|albums| albums.get(*ix))
                .is_some_and(|candidate| candidate.id() == &album)
        }) {
            self.albums_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
        self.selected_album = Some(album.clone());
        self.selected_song = None;
        self.detail = Detail::Loading;
        self.songs_scroll.scroll_to_item(0, ScrollStrategy::Top);
        let client = self.session.client().clone();
        let catalog = self.session.catalog().cloned();
        self._album_task = cx.spawn(async move |this, cx| {
            // The saved songs show at once, and the server's answer replaces them
            if let Some(catalog) = catalog.clone() {
                let id = album.clone();
                let cached = cx.background_spawn(async move { catalog.album(&id) }).await;
                if let Ok(Some(cached)) = cached {
                    this.update(cx, |this, cx| this.show_album(&album, cached, cx))
                        .ok();
                }
            }
            let result = {
                let album = album.clone();
                cx.background_spawn(async move { client.album(&album).await })
                    .await
            };
            if let (Ok(fetched), Some(catalog)) = (&result, catalog) {
                let fetched = fetched.clone();
                cx.background_spawn(async move { catalog.put_album(&fetched).ok() })
                    .detach();
            }
            this.update(cx, |this, cx| match result {
                Ok(fetched) => this.show_album(&album, fetched, cx),
                // Saved songs on show stay there
                Err(error)
                    if this.selected_album.as_ref() == Some(&album)
                        && !matches!(this.detail, Detail::Loaded(_)) =>
                {
                    this.detail = Detail::Failed(session::describe(&error).into());
                    cx.notify();
                }
                Err(_) => {}
            })
            .ok();
        });
        cx.notify();
    }

    /// Clears a filter that hides `album`. An album the list does not have yet, as
    /// during the first sync, comes into view once a sync brings it.
    pub fn reveal_album(&mut self, album: AlbumId, window: &mut Window, cx: &mut Context<Self>) {
        let position = self
            .loaded_albums()
            .and_then(|albums| albums.iter().position(|listed| listed.id() == &album));
        if position.is_some_and(|position| !self.visible.contains(&position)) {
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.apply_filter(cx);
        }
        self.select_album(album.clone(), cx);
        self.revealing = position.is_none().then_some(album);
        // Selecting the album already selected does not scroll
        self.scroll_to_selected_album();
        self.albums_focus.focus(window, cx);
    }

    fn show_album(&mut self, id: &AlbumId, loaded: AlbumWithSongs, cx: &mut Context<Self>) {
        if self.selected_album.as_ref() != Some(id) {
            return;
        }
        let (album, songs) = loaded.into_parts();
        let cover_url = album
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, COVER_PIXELS).into());
        self.detail = Detail::Loaded(Box::new(LoadedAlbum {
            album,
            songs: songs.into(),
            cover_url,
        }));
        cx.notify();
    }

    fn retry_album(&mut self, cx: &mut Context<Self>) {
        if let Some(album) = self.selected_album.take() {
            self.select_album(album, cx);
        }
    }

    pub fn min_width(&self, window: &Window, cx: &gpui_kit::App) -> gpui_kit::Pixels {
        MasterDetail::min_width(&self.panes, window, cx)
    }

    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = NowPlaying::new(song, playing);
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    fn move_album(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let len = self.visible.len();
        let page = page_rows(&self.albums_scroll, len);
        let Some(position) = motion.apply(self.selected_position(), len, page) else {
            return;
        };
        if let Some(album) = self.visible_album(position).map(|album| album.id().clone()) {
            self.select_album(album, cx);
        }
    }

    fn loaded_songs(&self) -> Option<&Arc<[Song]>> {
        match &self.detail {
            Detail::Loaded(loaded) => Some(&loaded.songs),
            _ => None,
        }
    }

    fn selected_song_ix(&self) -> Option<usize> {
        let selected = self.selected_song.as_ref()?;
        self.loaded_songs()?
            .iter()
            .position(|song| song.id() == selected)
    }

    fn select_song(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(song) = self.loaded_songs().and_then(|songs| songs.get(ix)) else {
            return;
        };
        self.selected_song = Some(song.id().clone());
        self.songs_scroll
            .scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn move_song(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let len = self.loaded_songs().map_or(0, |songs| songs.len());
        let page = page_rows(&self.songs_scroll, len);
        if let Some(ix) = motion.apply(self.selected_song_ix(), len, page) {
            self.select_song(ix, cx);
        }
    }

    /// Moves keyboard focus to the songs, selecting the first one if none is.
    fn open_album(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded_songs().is_none_or(|songs| songs.is_empty()) {
            return;
        }
        if self.selected_song_ix().is_none() {
            self.select_song(0, cx);
        }
        self.songs_focus.focus(window, cx);
    }

    fn play_from(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(songs) = self.loaded_songs().filter(|songs| ix < songs.len()) {
            window.dispatch_action(Box::new(PlaySongs::new(songs.clone(), ix)), cx);
        }
    }

    fn play_whole(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(songs) = self.loaded_songs().filter(|songs| !songs.is_empty()) {
            window.dispatch_action(Box::new(PlaySongs::whole(songs.clone())), cx);
        }
    }

    /// Does `action` to the whole of the selected album, once its songs are
    /// there if they are still loading. Replaces an action still waiting for
    /// its songs: the latest one counts.
    fn act_on_album(&mut self, action: AlbumAction, window: &mut Window, cx: &mut Context<Self>) {
        self._album_action_task = Task::ready(());
        if let Some(songs) = self.loaded_songs() {
            if let Some(command) = action.command(songs.clone()) {
                window.dispatch_action(command, cx);
            }
            return;
        }
        let Some(album) = self.selected_album.clone() else {
            return;
        };
        let session = self.session.clone();
        self._album_action_task = cx.spawn_in(window, async move |_, cx| {
            let songs = cx
                .background_spawn(async move { session.album(&album).await })
                .await
                .map(|loaded| Arc::from(loaded.into_parts().1))
                .map_err(|error| session::describe(&error));
            cx.update(|window, cx| action.finish(songs, window, cx))
                .ok();
        });
    }

    fn enqueue_selected_song(
        &self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let song = self
            .selected_song_ix()
            .and_then(|ix| self.loaded_songs()?.get(ix).cloned());
        if let Some(song) = song {
            window.dispatch_action(
                Box::new(EnqueueSongs::new(Arc::from([song]), placement)),
                cx,
            );
        }
    }

    fn choose_playlist_for_selected_song(&self, window: &mut Window, cx: &mut Context<Self>) {
        let song = self
            .selected_song_ix()
            .and_then(|ix| self.loaded_songs()?.get(ix).cloned());
        if let Some(song) = song {
            window.dispatch_action(
                Box::new(actions::ChoosePlaylist::new(Arc::from([song]))),
                cx,
            );
        }
    }

    fn go_to_album_artist(&self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected_position();
        let artist = selected
            .and_then(|position| self.visible_album(position))
            .and_then(|album| album.artist_id().cloned());
        if let Some(artist) = artist {
            window.dispatch_action(Box::new(ShowArtist::new(artist)), cx);
        }
    }

    /// The selected album, from the detail when it is on show there and it has
    /// yet to reach the list.
    fn selected_album_listing(&self) -> Option<&Album> {
        let selected = self.selected_album.as_ref()?;
        let listed = self
            .loaded_albums()
            .and_then(|albums| albums.iter().find(|album| album.id() == selected));
        listed.or(match &self.detail {
            Detail::Loaded(loaded) if loaded.album.id() == selected => Some(&loaded.album),
            _ => None,
        })
    }

    fn toggle_album_favorite(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(album) = self.selected_album_listing() {
            actions::toggle_favorite(Favorite::Album(album.clone()), window, cx);
        }
    }

    fn toggle_song_favorite(&self, window: &mut Window, cx: &mut Context<Self>) {
        let song = self
            .selected_song_ix()
            .and_then(|ix| self.loaded_songs()?.get(ix));
        if let Some(song) = song {
            actions::toggle_favorite(Favorite::Song(song.clone()), window, cx);
        }
    }

    fn go_to_song_artist(&self, window: &mut Window, cx: &mut Context<Self>) {
        let artist = self
            .selected_song_ix()
            .and_then(|ix| self.loaded_songs()?.get(ix)?.artist_id().cloned());
        if let Some(artist) = artist {
            window.dispatch_action(Box::new(ShowArtist::new(artist)), cx);
        }
    }

    fn render_albums(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.loaded_albums().map(|albums| albums.len());
        // While nothing shows, the list itself shows a spinner
        let updating = count.is_some() && self.status.is_running();
        let error = self.status.failure().map(SharedString::from);
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                ListPaneHeader::new("Albums", count)
                    .when(updating, |this| this.updating("Updating the library"))
                    .action(self.render_sort_menu(cx)),
            )
            .child(ListFilter::new(&self.filter))
            .when_some(error, |this, error| {
                this.child(RefreshFailure::new(
                    "Couldn't refresh the library",
                    error,
                    Button::new("retry-refresh")
                        .label("Retry")
                        .small()
                        .on_click(cx.listener(|this, _, _, _| this.refresh_library())),
                ))
            })
            .child(self.render_album_list(window, cx))
            .into_any_element()
    }

    fn render_sort_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.sort;
        let option = |label: &'static str, is_checked: bool, sort: AlbumSort| {
            let owner = cx.entity().downgrade();
            Some(SortOption::new(label, is_checked, move |_, cx| {
                owner.update(cx, |this, cx| this.set_sort(sort, cx)).ok();
            }))
        };
        let order = |label, order| {
            let sort = if current.order == order {
                current
            } else {
                AlbumSort::chosen(order)
            };
            option(label, current.order == order, sort)
        };
        let direction = |label, is_descending| {
            let mut sort = current;
            sort.is_descending = is_descending;
            option(label, current.is_descending == is_descending, sort)
        };
        sort_menu(
            "sort-albums",
            current.is_descending,
            vec![
                order("Name", AlbumOrder::Name),
                order("Artist", AlbumOrder::Artist),
                order("Recently Added", AlbumOrder::Added),
                order("Release Date", AlbumOrder::Released),
                None,
                direction("Ascending", false),
                direction("Descending", true),
            ],
        )
    }

    /// The album list's focus target, rendered in every state so focus and
    /// the commands bound above it never land on an element that is gone.
    fn render_album_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("albums")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .track_focus(&self.albums_focus)
            .key_context(ALBUMS_CONTEXT)
            .on_list_navigation(|this, motion, _, cx| this.move_album(motion, cx), cx)
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open_album(window, cx)))
            .on_action(cx.listener(|this, _: &Play, window, cx| {
                this.act_on_album(AlbumAction::Play, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.act_on_album(AlbumAction::Enqueue(Placement::Next), window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.act_on_album(AlbumAction::Enqueue(Placement::Last), window, cx)
            }))
            .on_action(
                cx.listener(|this, action: &AddAlbumToPlaylist, window, cx| {
                    let action = AlbumAction::AddToPlaylist(action.playlist.clone());
                    this.act_on_album(action, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.act_on_album(AlbumAction::ChoosePlaylist, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &GoToArtist, window, cx| this.go_to_album_artist(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFavorite, window, cx| {
                this.toggle_album_favorite(window, cx)
            }))
            .child(self.render_album_rows(window, cx))
            .into_any_element()
    }

    fn render_album_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let albums = match &self.albums {
            Albums::Loading => return centered(Spinner::new()).into_any_element(),
            Albums::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load albums",
                    SharedString::from(error.clone()),
                    Button::new("retry-albums")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.albums_focus.focus(window, cx);
                            this.refresh_library();
                        })),
                )
                .into_any_element();
            }
            Albums::Loaded(albums) => albums.clone(),
        };
        if albums.is_empty() {
            return centered(muted_text("This server has no albums.", cx)).into_any_element();
        }
        if self.visible.is_empty() {
            let note = self.session.missing_readings_note();
            return no_matches("No albums match the filter.", note, cx).into_any_element();
        }

        let visible = self.visible.clone();
        let selected = self.selected_album.clone();
        let is_focused = self.albums_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let albums_focus = self.albums_focus.clone();
        row_list(
            "album-list",
            "Albums",
            visible.len(),
            &self.albums_scroll,
            move |position| {
                let album = albums.get(*visible.get(position)?)?;
                Some(AlbumRow::new(
                    album,
                    selected.as_ref() == Some(album.id()),
                    is_focused,
                    owner.clone(),
                    albums_focus.clone(),
                ))
            },
        )
        .size_full()
        .into_any_element()
    }

    /// The detail pane is the songs' focus target in every state, for the
    /// same reason as [`Self::render_album_list`].
    fn render_detail(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("detail")
            .size_full()
            .min_w_0()
            .min_h_0()
            .track_focus(&self.songs_focus)
            .key_context(SONGS_CONTEXT)
            .on_list_navigation(|this, motion, _, cx| this.move_song(motion, cx), cx)
            .on_action(cx.listener(|this, _: &Open, window, cx| {
                if let Some(ix) = this.selected_song_ix() {
                    this.play_from(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.enqueue_selected_song(Placement::Next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.enqueue_selected_song(Placement::Last, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.choose_playlist_for_selected_song(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &GoToArtist, window, cx| this.go_to_song_artist(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFavorite, window, cx| {
                this.toggle_song_favorite(window, cx)
            }))
            .on_action(cx.listener(|this, _: &Back, window, cx| {
                this.albums_focus.focus(window, cx);
            }))
            .child(self.render_detail_content(window, cx))
            .into_any_element()
    }

    fn render_detail_content(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let loaded = match &self.detail {
            Detail::None => return div().size_full().into_any_element(),
            Detail::Loading => return centered(Spinner::new()).into_any_element(),
            Detail::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load this album",
                    error.clone(),
                    Button::new("retry-album")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.songs_focus.focus(window, cx);
                            this.retry_album(cx);
                        })),
                )
                .into_any_element();
            }
            Detail::Loaded(loaded) => loaded,
        };
        let album = &loaded.album;
        let total = album
            .duration()
            .or_else(|| loaded.songs.iter().map(Song::duration).sum());
        let summary = join_details([
            album.year().map(|year| year.to_string()),
            Some(format_song_count(loaded.songs.len())),
            total.map(format_duration),
        ]);

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                DetailHeader::new(
                    loaded.cover_url.clone(),
                    album.name().to_owned().into(),
                    summary,
                )
                .artist(album_artist_link(album))
                .action(
                    Button::new("play-album")
                        .icon(IconName::Play)
                        .label("Play")
                        .disabled(loaded.songs.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.play_whole(window, cx))),
                )
                .action(ui::favorite_button(
                    "favorite-album",
                    Favorite::Album(album.clone()),
                    cx,
                ))
                .action(self.render_album_menu(loaded)),
            )
            .child(self.render_songs(loaded, window, cx))
            .into_any_element()
    }

    /// The album list's commands act on the album on show, so the menu dispatches
    /// them there, whichever list has focus.
    fn render_album_menu(&self, loaded: &LoadedAlbum) -> impl IntoElement {
        let albums_focus = self.albums_focus.clone();
        let songs = loaded.songs.clone();
        Button::new("album-menu")
            .ghost()
            .icon(IconName::Ellipsis)
            .accessibility_label("More")
            .disabled(loaded.songs.is_empty())
            .dropdown_menu(move |menu, window, cx| {
                let menu = menu
                    .action_context(albums_focus.clone())
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                actions::add_to_playlist_submenu(
                    menu,
                    actions::add_songs_command(songs.clone()),
                    window,
                    cx,
                )
            })
    }

    fn render_songs(
        &self,
        loaded: &LoadedAlbum,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if loaded.songs.is_empty() {
            return centered(muted_text("This album has no songs.", cx)).into_any_element();
        }
        let songs = loaded.songs.clone();
        let album_artist = loaded.album.artist().map(Arc::from);
        let selected = self.selected_song.clone();
        let now_playing = self.now_playing.clone();
        let is_focused = self.songs_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let songs_focus = self.songs_focus.clone();
        row_list(
            "songs",
            "Songs",
            songs.len(),
            &self.songs_scroll,
            move |ix| {
                let song = songs.get(ix)?;
                Some(SongRow {
                    song: song.clone(),
                    ix,
                    album_artist: album_artist.clone(),
                    state: now_playing.as_ref().and_then(|now| now.state_of(song.id())),
                    is_selected: selected.as_ref() == Some(song.id()),
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

impl Focusable for Library {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.albums_focus.clone()
    }
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("library")
            .key_context(LIBRARY_CONTEXT)
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter.update(cx, |input, cx| input.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &Refresh, _, _| this.refresh_library()))
            .child(MasterDetail::new(
                "library-panes",
                &self.panes,
                self.render_albums(window, cx),
                self.render_detail(window, cx),
            ))
    }
}

/// What a choice in an album row's Add to Playlist menu runs: the album's
/// songs may still be loading when the menu opens.
fn add_album_command(playlist: Option<&PlaylistId>) -> Box<dyn gpui_kit::Action> {
    Box::new(AddAlbumToPlaylist {
        playlist: playlist.cloned(),
    })
}

#[cfg(test)]
mod tests;
