use std::mem;
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
use hiphile_core::alerts;
use hiphile_core::favorites::Favorite;
use hiphile_core::library::{
    AlbumDetail, AlbumDetailSnapshot, AlbumList, AlbumListStatus, AlbumOrder, LibraryNotice,
    LibrarySync, LoadedAlbum,
};
use hiphile_core::{Shelf, Shown};
use subsonic::{Album, AlbumId, PlaylistId, Song, SongId};
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
    LIBRARY_CONTEXT, Open, Play, PlayNext, SONGS_CONTEXT,
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
    album_list: AlbumList,
    /// The list's status last taken, which a new one is compared with.
    status: AlbumListStatus,
    /// What is on show of the list, which follows `album_list`.
    albums: Shelf<Album>,
    shown: Option<Shown<Album>>,
    selected_album: Option<AlbumId>,
    /// The selected album, when it was asked to show before the list had it.
    /// It stays selected and comes into view once a sync brings it.
    revealing: Option<AlbumId>,
    /// A sync ended, so that the album being revealed is given up once what it brought shows.
    gives_up_revealing: bool,
    album: AlbumDetail,
    /// What the album's detail showed last.
    detail: AlbumDetailSnapshot,
    cover_url: Option<SharedString>,
    selected_song: Option<SongId>,
    now_playing: Option<NowPlaying>,
    filter: Entity<InputState>,
    albums_focus: FocusHandle,
    songs_focus: FocusHandle,
    albums_scroll: UniformListScrollHandle,
    songs_scroll: UniformListScrollHandle,
    panes: Entity<ResizableState>,
    _album_changes: Task<()>,
    _album_action_task: Task<()>,
    _subscriptions: Vec<Subscription>,
    _list_changes: Task<()>,
    _sync_notices: Task<()>,
}

impl EventEmitter<LibraryEvent> for Library {}

impl Library {
    pub fn new(
        session: session::Session,
        sync: LibrarySync,
        album_list: AlbumList,
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
                InputEvent::Change => this.filter_changed(cx),
                InputEvent::PressEnter { .. } => this.albums_focus.focus(window, cx),
                _ => {}
            }),
            // Focus changes the selection treatment of the rows
            cx.on_focus(&albums_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&albums_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus(&songs_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&songs_focus, window, |_, _, cx| cx.notify()),
        ];
        let status = album_list.status();
        let list_changes = gpui_host::observe(album_list.subscribe(), cx, Self::follow_list);
        let sync_notices =
            gpui_host::subscribe(sync.notices(), cx, |this: &mut Self, notice, cx| {
                if notice == LibraryNotice::SyncFinished {
                    this.gives_up_revealing = true;
                    this.sync(cx);
                }
            });
        let album = AlbumDetail::new(gpui_host::executor(cx), session.clone());
        let album_changes = gpui_host::observe(album.subscribe(), cx, Self::follow_album);
        let mut this = Self {
            session,
            sync,
            album_list,
            status: status.clone(),
            albums: status.albums().clone(),
            shown: None,
            selected_album: None,
            revealing: None,
            gives_up_revealing: false,
            album,
            detail: AlbumDetailSnapshot::None,
            cover_url: None,
            selected_song: None,
            now_playing: None,
            filter,
            albums_focus,
            songs_focus,
            albums_scroll: UniformListScrollHandle::new(),
            songs_scroll: UniformListScrollHandle::new(),
            panes: cx.new(|_| ResizableState::default()),
            _album_changes: album_changes,
            _album_action_task: Task::ready(()),
            _subscriptions: subscriptions,
            _list_changes: list_changes,
            _sync_notices: sync_notices,
        };
        this.show(status.shown().cloned(), cx);
        this
    }

    /// Fetches the whole library from the server again.
    pub fn refresh_library(&self) {
        self.sync.refresh();
    }

    /// Gives up on an album being revealed once the albums a sync brought show without it.
    fn follow_list(&mut self, status: AlbumListStatus, cx: &mut Context<Self>) {
        let old = mem::replace(&mut self.status, status);
        if old.albums() != self.status.albums() {
            self.albums = self.status.albums().clone();
        }
        if old.shown() != self.status.shown() {
            self.show(self.status.shown().cloned(), cx);
        }
        if self.gives_up_revealing && !self.status.is_updating() {
            self.gives_up_revealing = false;
            self.revealing = None;
        }
        cx.notify();
    }

    /// Takes the list's status at once after a call into it, so that what follows does not act
    /// on the old one.
    fn sync(&mut self, cx: &mut Context<Self>) {
        self.follow_list(self.album_list.status(), cx);
    }

    /// Keeps the selection while the album is still there or a sync may yet bring it. Albums
    /// in a new order keep the selected one in view.
    fn show(&mut self, shown: Option<Shown<Album>>, cx: &mut Context<Self>) {
        let old = mem::replace(&mut self.shown, shown);
        let Some(albums) = self.shown.as_ref().map(|shown| shown.items().clone()) else {
            return;
        };
        if old.is_some_and(|old| Arc::ptr_eq(old.items(), &albums)) {
            self.scroll_to_selected_album();
            return;
        }
        let is_listed = |id: &AlbumId| albums.iter().any(|album| album.id() == id);
        let keeps_selection =
            self.revealing.is_some() || self.selected_album.as_ref().is_some_and(is_listed);
        let revealed = self.revealing.as_ref().is_some_and(is_listed);
        if !keeps_selection {
            self.selected_album = None;
            self.album.clear();
            self.follow_album(self.album.snapshot(), cx);
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

    fn listed_albums(&self) -> Option<&Arc<[Album]>> {
        self.shown.as_ref().map(Shown::items)
    }

    fn visible_len(&self) -> usize {
        self.shown.as_ref().map_or(0, |shown| shown.visible().len())
    }

    fn visible_album(&self, position: usize) -> Option<&Album> {
        self.shown.as_ref()?.get(position)
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected_album.as_ref()?;
        self.shown
            .as_ref()?
            .position(|album| album.id() == selected)
    }

    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        self.album_list.set_query(&query);
        self.sync(cx);
    }

    fn choose_order(&mut self, order: AlbumOrder, cx: &mut Context<Self>) {
        self.album_list.choose_order(order);
        self.sync(cx);
    }

    fn choose_direction(&mut self, is_descending: bool, cx: &mut Context<Self>) {
        self.album_list.choose_direction(is_descending);
        self.sync(cx);
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
        self.selected_album = Some(album.clone());
        self.scroll_to_selected_album();
        self.selected_song = None;
        self.songs_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.album.show(album);
        self.follow_album(self.album.snapshot(), cx);
        cx.notify();
    }

    /// Clears a filter that hides `album`. An album the list does not have yet, as
    /// during the first sync, comes into view once a sync brings it.
    pub fn reveal_album(&mut self, album: AlbumId, window: &mut Window, cx: &mut Context<Self>) {
        let position = self
            .listed_albums()
            .and_then(|albums| albums.iter().position(|listed| listed.id() == &album));
        let is_hidden = position.is_some_and(|position| {
            self.shown
                .as_ref()
                .is_some_and(|shown| !shown.visible().contains(&position))
        });
        if is_hidden {
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.filter_changed(cx);
        }
        self.select_album(album.clone(), cx);
        self.revealing = position.is_none().then_some(album);
        self.gives_up_revealing = false;
        // Selecting the album already selected does not scroll, and an album the filter hid
        // comes into view once the albums show in their new order
        self.scroll_to_selected_album();
        self.albums_focus.focus(window, cx);
    }

    fn follow_album(&mut self, detail: AlbumDetailSnapshot, cx: &mut Context<Self>) {
        if detail == self.detail {
            return;
        }
        self.cover_url = detail
            .loaded()
            .and_then(|loaded| loaded.album().cover_art())
            .map(|cover_art| self.session.cover_art_url(cover_art, COVER_PIXELS).into());
        self.detail = detail;
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
        let len = self.visible_len();
        let page = page_rows(&self.albums_scroll, len);
        let Some(position) = motion.apply(self.selected_position(), len, page) else {
            return;
        };
        if let Some(album) = self.visible_album(position).map(|album| album.id().clone()) {
            self.select_album(album, cx);
        }
    }

    fn loaded_songs(&self) -> Option<&Arc<[Song]>> {
        self.detail.loaded().map(LoadedAlbum::songs)
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
        if let Some(album) = self.selected_album.clone() {
            self._album_action_task = action.start(album, &self.session, window, cx);
        }
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
            .listed_albums()
            .and_then(|albums| albums.iter().find(|album| album.id() == selected));
        listed.or(self
            .detail
            .loaded()
            .map(LoadedAlbum::album)
            .filter(|album| album.id() == selected))
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
        let count = self.listed_albums().map(|albums| albums.len());
        // While nothing shows, the list itself shows a spinner
        let updating = count.is_some() && self.status.is_updating();
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
                    alerts::SYNC_FAILED,
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
        let current = self.status.sort();
        let order = |label: &'static str, order: AlbumOrder| {
            let owner = cx.entity().downgrade();
            Some(SortOption::new(
                label,
                current.order == order,
                move |_, cx| {
                    owner
                        .update(cx, |this, cx| this.choose_order(order, cx))
                        .ok();
                },
            ))
        };
        let direction = |label: &'static str, is_descending: bool| {
            let owner = cx.entity().downgrade();
            let is_checked = current.is_descending == is_descending;
            Some(SortOption::new(label, is_checked, move |_, cx| {
                owner
                    .update(cx, |this, cx| this.choose_direction(is_descending, cx))
                    .ok();
            }))
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
        let shown = match (&self.albums, &self.shown) {
            (Shelf::Loading, _) | (Shelf::Loaded(_), None) => {
                return centered(Spinner::new()).into_any_element();
            }
            (Shelf::Failed(error), _) => {
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
            (Shelf::Loaded(_), Some(shown)) => shown.clone(),
        };
        if shown.items().is_empty() {
            return centered(muted_text("This server has no albums.", cx)).into_any_element();
        }
        if shown.visible().is_empty() {
            let note = self.session.missing_readings_note();
            return no_matches("No albums match the filter.", note, cx).into_any_element();
        }

        let selected = self.selected_album.clone();
        let is_focused = self.albums_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let albums_focus = self.albums_focus.clone();
        row_list(
            "album-list",
            "Albums",
            shown.visible().len(),
            &self.albums_scroll,
            move |position| {
                let album = shown.get(position)?;
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
            AlbumDetailSnapshot::None => return div().size_full().into_any_element(),
            AlbumDetailSnapshot::Loading(_) => return centered(Spinner::new()).into_any_element(),
            AlbumDetailSnapshot::Failed { error, .. } => {
                return LoadFailure::new(
                    "Couldn't load this album",
                    SharedString::from(error.clone()),
                    Button::new("retry-album")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.songs_focus.focus(window, cx);
                            this.retry_album(cx);
                        })),
                )
                .into_any_element();
            }
            AlbumDetailSnapshot::Loaded(loaded) => loaded,
        };
        let album = loaded.album();
        let summary = join_details([
            album.year().map(|year| year.to_string()),
            Some(format_song_count(loaded.songs().len())),
            loaded.duration().map(format_duration),
        ]);

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                DetailHeader::new(
                    self.cover_url.clone(),
                    album.name().to_owned().into(),
                    summary,
                )
                .artist(album_artist_link(album))
                .action(
                    Button::new("play-album")
                        .icon(IconName::Play)
                        .label("Play")
                        .disabled(loaded.songs().is_empty())
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
        let songs = loaded.songs().clone();
        Button::new("album-menu")
            .ghost()
            .icon(IconName::Ellipsis)
            .accessibility_label("More")
            .disabled(songs.is_empty())
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
        if loaded.songs().is_empty() {
            return centered(muted_text("This album has no songs.", cx)).into_any_element();
        }
        let loaded = loaded.clone();
        let selected = self.selected_song.clone();
        let now_playing = self.now_playing.clone();
        let is_focused = self.songs_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let songs_focus = self.songs_focus.clone();
        row_list(
            "songs",
            "Songs",
            loaded.songs().len(),
            &self.songs_scroll,
            move |ix| {
                let song = loaded.songs().get(ix)?;
                Some(SongRow {
                    song: song.clone(),
                    ix,
                    is_by_another_artist: loaded.is_by_another_artist(song),
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
