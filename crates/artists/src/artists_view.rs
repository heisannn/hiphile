use std::sync::Arc;
use std::time::Duration;

use actions::{AlbumAction, Placement};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    button::Button,
    input::{InputEvent, InputState},
    notification::NotificationType,
    resizable::ResizableState,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ListAlignment, ListOffset, ListState, ParentElement as _,
    Pixels, Render, ScrollStrategy, SharedString, Styled as _, Subscription, Task,
    UniformListScrollHandle, Window, div, prelude::FluentBuilder as _, px,
};
use hiphile_core::artists::{
    ArtistDates, ArtistOrder, ArtistSort, album_by_album, arrange, artist_index, artists_of_albums,
    newest_first,
};
use hiphile_core::favorites::Favorite;
use subsonic::{Album, Artist, ArtistId, CoverArtId, PlaylistId, Song, SongId};
use ui::card_grid::GridMotion;
use ui::list_navigation::{ListNavigationExt as _, Motion, page_rows};
use ui::{
    ListFilter, ListPaneHeader, LoadFailure, MasterDetail, NowPlaying, RefreshFailure, Shelf,
    SortOption, centered, muted_text, no_matches, row_list, sort_menu,
};

mod artist_row;
mod detail;

use artist_row::ArtistRow;
use detail::{Item, ListedDetail};

use crate::{
    ARTISTS_CONTEXT, AddSelectionToPlaylist, AddToPlaylist, AddToQueue, FocusFilter, Open, Play,
    PlayNext, ROOT_CONTEXT,
};

const TOP_SONGS: u32 = 10;
const TOP_SONGS_COLLAPSED: usize = 5;
const SIMILAR_ARTISTS: u32 = 6;
/// Top songs and similar artists take the server's outside service a while:
/// moving through the list with the arrow keys should not ask about every artist.
const SERVER_DELAY: Duration = Duration::from_millis(250);
const ALBUMS_AT_ONCE: usize = 8;
/// In pixels: about a row of cards, so that scrolling does not show them arriving.
const DETAIL_OVERDRAW: f32 = 300.;

pub enum ArtistsEvent {
    /// The user chose another artist over this one.
    LeftArtist(ArtistId),
}

pub struct Artists {
    session: session::Session,
    is_activated: bool,
    artists: ArtistList,
    refresh_error: Option<SharedString>,
    is_refreshing: bool,
    index: Arc<fuzzy::Index>,
    /// Positions in the loaded artists that match the filter, in the order they show.
    visible: Arc<Vec<usize>>,
    sort: ArtistSort,
    /// From the saved albums: without a saved library, no artist has a date.
    dates: Arc<ArtistDates>,
    selected_artist: Option<ArtistId>,
    /// Whether the list on its way should come to the artist just revealed,
    /// who may not be on the list yet or may move when it arrives.
    revealing: bool,
    detail: Option<ShownArtist>,
    selection: Option<Item>,
    top_songs_expanded: bool,
    now_playing: Option<NowPlaying>,
    /// How many cards fit across the detail, from the last layout.
    columns: usize,
    filter: Entity<InputState>,
    list_focus: FocusHandle,
    detail_focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    detail_list: ListState,
    listed: ListedDetail,
    detail_width: Pixels,
    /// The artist whose songs a command is still waiting for.
    artist_being_fetched: Option<ArtistId>,
    panes: Entity<ResizableState>,
    _list_task: Task<()>,
    _albums_task: Task<()>,
    _top_songs_task: Task<()>,
    _similar_task: Task<()>,
    _action_task: Task<()>,
    _index_task: Task<()>,
    _subscriptions: Vec<Subscription>,
    _readings_changes: Option<Task<()>>,
}

enum ArtistList {
    Loading,
    Loaded(Arc<Vec<Artist>>),
    Failed(SharedString),
}

struct ShownArtist {
    id: ArtistId,
    /// Known once the list, the saved albums or the server has said.
    name: Option<SharedString>,
    cover_art: Option<CoverArtId>,
    albums: Shelf<Album>,
    top_songs: Shelf<Song>,
    similar: Shelf<Artist>,
    /// Whether the artist has no albums of their own, only songs on others',
    /// which `top_songs` then holds in place of what the server ranks.
    is_only_on_songs: bool,
}

impl EventEmitter<ArtistsEvent> for Artists {}

impl Artists {
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter artists"));
        let list_focus = cx.focus_handle().tab_stop(true);
        let detail_focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            // The artist on show has a star that says
            actions::observe_favorites(cx),
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.apply_filter(cx),
                InputEvent::PressEnter { .. } => this.list_focus.focus(window, cx),
                _ => {}
            }),
            // Focus changes the selection treatment
            cx.on_focus(&list_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&list_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus(&detail_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&detail_focus, window, |_, _, cx| cx.notify()),
        ];
        let readings_changes = session.readings().map(|readings| {
            ui::on_readings_changed(readings, cx, |this: &mut Self, cx| this.index_artists(cx))
        });
        Self {
            session,
            is_activated: false,
            artists: ArtistList::Loading,
            refresh_error: None,
            is_refreshing: false,
            index: Arc::default(),
            visible: Arc::default(),
            sort: ArtistSort::default(),
            dates: Arc::default(),
            selected_artist: None,
            revealing: false,
            detail: None,
            selection: None,
            top_songs_expanded: false,
            now_playing: None,
            columns: 3,
            filter,
            list_focus,
            detail_focus,
            list_scroll: UniformListScrollHandle::new(),
            detail_list: ListState::new(0, ListAlignment::Top, px(DETAIL_OVERDRAW)),
            listed: ListedDetail::default(),
            detail_width: px(0.),
            artist_being_fetched: None,
            panes: cx.new(|_| ResizableState::default()),
            _list_task: Task::ready(()),
            _albums_task: Task::ready(()),
            _top_songs_task: Task::ready(()),
            _similar_task: Task::ready(()),
            _action_task: Task::ready(()),
            _index_task: Task::ready(()),
            _subscriptions: subscriptions,
            _readings_changes: readings_changes,
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        if !self.is_activated {
            self.is_activated = true;
            self.load_artists(cx);
        }
    }

    /// Does nothing to a screen that has yet to show.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.is_activated {
            return;
        }
        self.load_artists(cx);
        if let Some(artist) = self.selected_artist.clone() {
            self.load_detail(artist, cx);
        }
    }

    /// Does nothing to a screen that has yet to show.
    pub fn artists_synced(&mut self, cx: &mut Context<Self>) {
        if self.is_activated {
            self.load_artists(cx);
        }
    }

    /// Reads the saved songs again for an artist with no albums of their own,
    /// whose page is made of them.
    pub fn songs_synced(&mut self, cx: &mut Context<Self>) {
        let artist = self
            .detail
            .as_ref()
            .filter(|detail| matches!(&detail.albums, Shelf::Loaded(albums) if albums.is_empty()))
            .map(|detail| detail.id.clone());
        if let Some(artist) = artist {
            self.load_albums(artist, cx);
        }
    }

    /// Shows the saved artists, which a sync keeps up to date. Before there are
    /// any, shows what the saved albums say, then what the server says.
    fn load_artists(&mut self, cx: &mut Context<Self>) {
        self.is_refreshing = true;
        self.refresh_error = None;
        if self.loaded_artists().is_none() {
            self.artists = ArtistList::Loading;
        }
        let client = self.session.client().clone();
        let catalog = self.session.catalog().cloned();
        let has_list = self.loaded_artists().is_some();
        self._list_task = cx.spawn(async move |this, cx| {
            if let Some(catalog) = catalog {
                let saved = cx
                    .background_spawn(async move {
                        // Albums that can't be read only leave the artists without dates
                        let albums = catalog.albums();
                        let dates = albums.as_deref().map(ArtistDates::of_albums);
                        let dates = Arc::new(dates.unwrap_or_default());
                        let artists = catalog.artists()?;
                        if !artists.is_empty() {
                            return Ok((artists, dates, true));
                        }
                        Ok::<_, catalog::Error>((artists_of_albums(&albums?), dates, false))
                    })
                    .await;
                // A copy that can't be read only means waiting for the server
                match saved {
                    Ok((artists, dates, true)) => {
                        this.update(cx, |this, cx| {
                            this.dates = dates;
                            this.finish_loading_artists(Ok(artists), cx);
                        })
                        .ok();
                        return;
                    }
                    Ok((artists, dates, false)) => {
                        this.update(cx, |this, cx| {
                            this.dates = dates;
                            if !has_list && !artists.is_empty() {
                                this.set_artists(artists, cx);
                            }
                        })
                        .ok();
                    }
                    Err(_) => {}
                }
            }
            let result = cx
                .background_spawn(async move { client.artists().await })
                .await
                .map_err(|error| SharedString::from(session::describe(&error)));
            this.update(cx, |this, cx| this.finish_loading_artists(result, cx))
                .ok();
        });
        cx.notify();
    }

    fn finish_loading_artists(
        &mut self,
        result: Result<Vec<Artist>, SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.is_refreshing = false;
        match result {
            Ok(artists) => self.set_artists(artists, cx),
            Err(error) if self.loaded_artists().is_some() => self.refresh_error = Some(error),
            Err(error) => self.artists = ArtistList::Failed(error),
        }
        self.revealing = false;
        cx.notify();
    }

    /// The selection stays, also on an artist the list does not have, as one who
    /// only appears on songs, unless the artist was on the list and has gone.
    fn set_artists(&mut self, artists: Vec<Artist>, cx: &mut Context<Self>) {
        let is_listed = |list: &[Artist], id: &ArtistId| list.iter().any(|a| a.id() == id);
        let has_gone = self.selected_artist.as_ref().is_some_and(|selected| {
            let was_listed = self
                .loaded_artists()
                .is_some_and(|old| is_listed(old, selected));
            was_listed && !is_listed(&artists, selected)
        });
        self.artists = ArtistList::Loaded(Arc::new(artists));
        self.index_artists(cx);
        if has_gone {
            self.selected_artist = None;
            self.detail = None;
        }
        if self.selected_artist.is_none() {
            let first = self.visible_artist(0).map(|artist| artist.id().clone());
            if let Some(first) = first {
                self.select_artist(first, cx);
            }
        }
        if self.revealing {
            self.scroll_to_selected_artist();
        }
        cx.notify();
    }

    fn loaded_artists(&self) -> Option<&Arc<Vec<Artist>>> {
        match &self.artists {
            ArtistList::Loaded(artists) => Some(artists),
            _ => None,
        }
    }

    fn visible_artist(&self, position: usize) -> Option<&Artist> {
        self.loaded_artists()?.get(*self.visible.get(position)?)
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected_artist.as_ref()?;
        (0..self.visible.len()).find(|position| {
            self.visible_artist(*position)
                .is_some_and(|artist| artist.id() == selected)
        })
    }

    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        let found = self.index.search(&query);
        self.visible = Arc::new(match self.loaded_artists() {
            Some(artists) => arrange(artists, found, &self.dates, self.sort),
            None => found,
        });
        cx.notify();
    }

    fn set_sort(&mut self, sort: ArtistSort, cx: &mut Context<Self>) {
        if self.sort != sort {
            self.sort = sort;
            self.apply_filter(cx);
            self.scroll_to_selected_artist();
        }
    }

    /// Makes the artists searchable at once as far as they read without a
    /// dictionary, then, off the UI thread, as far as they read with one.
    fn index_artists(&mut self, cx: &mut Context<Self>) {
        let Some(artists) = self.loaded_artists().cloned() else {
            return;
        };
        self.index = Arc::new(artist_index(&artists, None));
        self._index_task = Task::ready(());
        self.apply_filter(cx);
        let Some(book) = self.session.reading_book() else {
            return;
        };
        self._index_task = cx.spawn(async move |this, cx| {
            let index = cx
                .background_spawn(async move {
                    book.read(|speller| artist_index(&artists, Some(speller)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.index = Arc::new(index);
                this.apply_filter(cx);
            })
            .ok();
        });
    }

    pub fn selected_artist(&self) -> Option<&ArtistId> {
        self.selected_artist.as_ref()
    }

    /// Selects the artist the user chose. Moving through the list does not
    /// count: it would leave every artist it passes to go back to.
    fn choose_artist(&mut self, artist: ArtistId, cx: &mut Context<Self>) {
        if let Some(left) = self.selected_artist.clone().filter(|left| *left != artist) {
            cx.emit(ArtistsEvent::LeftArtist(left));
        }
        self.select_artist(artist, cx);
    }

    fn select_artist(&mut self, artist: ArtistId, cx: &mut Context<Self>) {
        if self.selected_artist.as_ref() == Some(&artist) {
            return;
        }
        self.revealing = false;
        self.selected_artist = Some(artist.clone());
        self.scroll_to_selected_artist();
        self.selection = None;
        self.top_songs_expanded = false;
        self.detail_list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        let listed = self
            .loaded_artists()
            .and_then(|artists| artists.iter().find(|listed| listed.id() == &artist));
        self.detail = Some(ShownArtist {
            id: artist.clone(),
            name: listed.map(|listed| listed.name().to_owned().into()),
            cover_art: listed.and_then(|listed| listed.cover_art().cloned()),
            albums: Shelf::Loading,
            top_songs: Shelf::Loading,
            similar: Shelf::Loading,
            is_only_on_songs: false,
        });
        self.load_detail(artist, cx);
        cx.notify();
    }

    /// Clears a filter that hides `artist`. One the list does not have, as one who
    /// only appears on songs, shows in the detail all the same.
    pub fn reveal_artist(&mut self, artist: ArtistId, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(cx);
        let is_hidden = self
            .loaded_artists()
            .and_then(|artists| artists.iter().position(|listed| listed.id() == &artist))
            .is_some_and(|position| !self.visible.contains(&position));
        if is_hidden {
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.apply_filter(cx);
        }
        self.select_artist(artist, cx);
        self.revealing = self.is_refreshing;
        // Selecting the artist already selected does not scroll
        self.scroll_to_selected_artist();
        self.list_focus.focus(window, cx);
    }

    fn scroll_to_selected_artist(&self) {
        if let Some(position) = self.selected_position() {
            self.list_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
    }

    fn load_detail(&mut self, artist: ArtistId, cx: &mut Context<Self>) {
        self.load_albums(artist.clone(), cx);
        self.load_similar(artist.clone(), cx);
        self.load_top_songs(artist, cx);
    }

    /// Shows the saved albums of `artist` at once. The server's answer replaces
    /// them, and names an artist nothing else had a name for.
    fn load_albums(&mut self, artist: ArtistId, cx: &mut Context<Self>) {
        if let Some(detail) = &mut self.detail {
            detail.albums.begin();
        }
        let client = self.session.client().clone();
        let catalog = self.session.catalog().cloned();
        self._albums_task = cx.spawn(async move |this, cx| {
            if let Some(catalog) = catalog.clone() {
                let id = artist.clone();
                let saved = cx
                    .background_spawn(async move { catalog.albums_by_artist(&id) })
                    .await;
                if let Ok(saved) = saved
                    && !saved.is_empty()
                {
                    this.update(cx, |this, cx| this.show_saved_albums(&artist, saved, cx))
                        .ok();
                }
            }
            cx.background_executor().timer(SERVER_DELAY).await;
            let result = {
                let artist = artist.clone();
                cx.background_spawn(async move { client.artist(&artist).await })
                    .await
            }
            .map(|found| found.into_parts())
            .map_err(|error| SharedString::from(session::describe(&error)));
            // Read now so that "no albums" never shows on the way to the songs
            let has_no_albums = matches!(&result, Ok((_, albums)) if albums.is_empty());
            let songs = match catalog {
                Some(catalog) if has_no_albums => {
                    let artist = artist.clone();
                    cx.background_spawn(async move { saved_songs_naming(&catalog, &artist) })
                        .await
                }
                _ => Vec::new(),
            };
            this.update(cx, |this, cx| {
                this.finish_loading_albums(&artist, result, songs, cx);
            })
            .ok();
        });
        cx.notify();
    }

    fn show_saved_albums(&mut self, id: &ArtistId, albums: Vec<Album>, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut().filter(|detail| &detail.id == id) else {
            return;
        };
        let names_artist = detail.name.is_none();
        if names_artist {
            detail.name = albums
                .first()
                .and_then(Album::artist)
                .map(|name| name.to_owned().into());
        }
        detail.albums = Shelf::Loaded(newest_first(albums).into());
        if names_artist {
            self.load_top_songs(id.clone(), cx);
        }
        cx.notify();
    }

    fn finish_loading_albums(
        &mut self,
        id: &ArtistId,
        result: Result<(Artist, Vec<Album>), SharedString>,
        songs_naming_artist: Vec<Song>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.detail.as_mut().filter(|detail| &detail.id == id) else {
            return;
        };
        let had_name = detail.name.is_some();
        let was_only_on_songs = detail.is_only_on_songs;
        match result {
            Ok((artist, albums)) => {
                detail.name = Some(artist.name().to_owned().into());
                if artist.cover_art().is_some() {
                    detail.cover_art = artist.cover_art().cloned();
                }
                detail.albums.finish(Ok(newest_first(albums)));
                detail.is_only_on_songs = !songs_naming_artist.is_empty();
            }
            Err(error) => detail.albums.finish(Err(error)),
        }
        if detail.is_only_on_songs {
            // A failed refresh brings no songs: those on show stay
            if !songs_naming_artist.is_empty() {
                detail.top_songs = Shelf::Loaded(songs_naming_artist.into());
            }
            self._top_songs_task = Task::ready(());
        } else if was_only_on_songs || (!had_name && detail.name.is_some()) {
            // Top songs are asked for by name, which may only now have arrived
            self.load_top_songs(id.clone(), cx);
        }
        cx.notify();
    }

    fn load_top_songs(&mut self, artist: ArtistId, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut().filter(|detail| detail.id == artist) else {
            return;
        };
        if detail.is_only_on_songs {
            return;
        }
        detail.top_songs.begin();
        let Some(name) = detail.name.clone() else {
            return;
        };
        let client = self.session.client().clone();
        self._top_songs_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SERVER_DELAY).await;
            let result = cx
                .background_spawn(async move { client.top_songs(&name, TOP_SONGS).await })
                .await;
            this.update(cx, |this, cx| {
                let result = result.map_err(|error| {
                    format!("Couldn't load top songs. {}", session::describe(&error)).into()
                });
                this.finish_loading_top_songs(&artist, result, cx);
            })
            .ok();
        });
        cx.notify();
    }

    fn finish_loading_top_songs(
        &mut self,
        id: &ArtistId,
        result: Result<Vec<Song>, SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(detail) = self.detail.as_mut().filter(|detail| &detail.id == id) {
            detail.top_songs.finish(result);
            cx.notify();
        }
    }

    fn load_similar(&mut self, artist: ArtistId, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut().filter(|detail| detail.id == artist) else {
            return;
        };
        detail.similar.begin();
        let client = self.session.client().clone();
        self._similar_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SERVER_DELAY).await;
            let result = {
                let artist = artist.clone();
                cx.background_spawn(async move {
                    client.similar_artists(&artist, SIMILAR_ARTISTS).await
                })
                .await
            };
            this.update(cx, |this, cx| {
                let result = result.map_err(|error| {
                    format!(
                        "Couldn't load similar artists. {}",
                        session::describe(&error)
                    )
                    .into()
                });
                this.finish_loading_similar(&artist, result, cx);
            })
            .ok();
        });
        cx.notify();
    }

    fn finish_loading_similar(
        &mut self,
        id: &ArtistId,
        result: Result<Vec<Artist>, SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(detail) = self.detail.as_mut().filter(|detail| &detail.id == id) {
            detail.similar.finish(result);
            cx.notify();
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

    fn move_artist(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let len = self.visible.len();
        let page = page_rows(&self.list_scroll, len);
        let Some(position) = motion.apply(self.selected_position(), len, page) else {
            return;
        };
        if let Some(artist) = self.visible_artist(position).map(|a| a.id().clone()) {
            self.select_artist(artist, cx);
        }
    }

    /// Moves keyboard focus to the detail, selecting the first thing in it if
    /// nothing is.
    fn open_artist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection_on_screen().is_none() {
            self.move_selection(GridMotion::First, cx);
        }
        self.detail_focus.focus(window, cx);
    }

    /// Does `action` to every song of `artist`, album by album as the detail
    /// shows them. Replaces an action still waiting for its songs: the latest
    /// one counts.
    fn act_on_artist(
        &mut self,
        artist: ArtistId,
        action: AlbumAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.artist_being_fetched = Some(artist.clone());
        cx.notify();
        let session = self.session.clone();
        let shown = self
            .detail
            .as_ref()
            .filter(|detail| detail.id == artist && matches!(detail.albums, Shelf::Loaded(_)))
            .map(|detail| detail.albums.items().to_vec());
        self._action_task = cx.spawn_in(window, async move |this, cx| {
            let songs = cx
                .background_spawn(async move { artist_songs(&session, &artist, shown).await })
                .await
                .map_err(|error| session::describe(&error));
            cx.update(|window, cx| match songs {
                Ok(songs) => {
                    if let Some(command) = action.command(songs.into()) {
                        window.dispatch_action(command, cx);
                    }
                }
                Err(error) => {
                    let message = format!("Couldn't load this artist. {error}");
                    window.push_notification((NotificationType::Error, message), cx);
                }
            })
            .ok();
            this.update(cx, |this, cx| {
                this.artist_being_fetched = None;
                cx.notify();
            })
            .ok();
        });
    }

    fn act_on_selected_artist(
        &mut self,
        action: AlbumAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(artist) = self.selected_artist.clone() {
            self.act_on_artist(artist, action, window, cx);
        }
    }

    fn render_list_pane(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.loaded_artists().map(|artists| artists.len());
        let is_updating = count.is_some() && self.is_refreshing;
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                ListPaneHeader::new("Artists", count)
                    .when(is_updating, |this| this.updating("Updating the artists"))
                    .action(self.render_sort_menu(cx)),
            )
            .child(ListFilter::new(&self.filter))
            .when_some(self.refresh_error.clone(), |this, error| {
                this.child(RefreshFailure::new(
                    "Couldn't refresh the artists",
                    error,
                    Button::new("retry-refresh-artists")
                        .label("Retry")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.load_artists(cx))),
                ))
            })
            .child(self.render_list(window, cx))
            .into_any_element()
    }

    fn render_sort_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.sort;
        let option = |label: &'static str, is_checked: bool, sort: ArtistSort| {
            let owner = cx.entity().downgrade();
            Some(SortOption::new(label, is_checked, move |_, cx| {
                owner.update(cx, |this, cx| this.set_sort(sort, cx)).ok();
            }))
        };
        let order = |label, order| {
            let sort = if current.order == order {
                current
            } else {
                ArtistSort::chosen(order)
            };
            option(label, current.order == order, sort)
        };
        let direction = |label, is_descending| {
            let mut sort = current;
            sort.is_descending = is_descending;
            option(label, current.is_descending == is_descending, sort)
        };
        sort_menu(
            "sort-artists",
            current.is_descending,
            vec![
                order("Name", ArtistOrder::Name),
                order("Recently Added", ArtistOrder::Added),
                order("Release Date", ArtistOrder::Released),
                None,
                direction("Ascending", false),
                direction("Descending", true),
            ],
        )
    }

    /// The list's focus target, rendered in every state so focus and the
    /// commands bound above it never land on an element that is gone.
    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("artists")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .track_focus(&self.list_focus)
            .key_context(ARTISTS_CONTEXT)
            .on_list_navigation(|this, motion, _, cx| this.move_artist(motion, cx), cx)
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open_artist(window, cx)))
            .on_action(cx.listener(|this, _: &Play, window, cx| {
                this.act_on_selected_artist(AlbumAction::Play, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.act_on_selected_artist(AlbumAction::Enqueue(Placement::Next), window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.act_on_selected_artist(AlbumAction::Enqueue(Placement::Last), window, cx)
            }))
            .on_action(
                cx.listener(|this, action: &AddSelectionToPlaylist, window, cx| {
                    let action = AlbumAction::AddToPlaylist(action.playlist.clone());
                    this.act_on_selected_artist(action, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.act_on_selected_artist(AlbumAction::ChoosePlaylist, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &actions::ToggleFavorite, window, cx| {
                    if let Some(artist) = this.selected_artist_whole() {
                        actions::toggle_favorite(Favorite::Artist(artist), window, cx);
                    }
                }),
            )
            .child(self.render_rows(window, cx))
            .into_any_element()
    }

    fn render_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let artists = match &self.artists {
            ArtistList::Loading => return centered(Spinner::new()).into_any_element(),
            ArtistList::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load artists",
                    error.clone(),
                    Button::new("retry-artists")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.list_focus.focus(window, cx);
                            this.load_artists(cx);
                        })),
                )
                .into_any_element();
            }
            ArtistList::Loaded(artists) => artists.clone(),
        };
        if artists.is_empty() {
            return centered(muted_text("This server has no artists.", cx)).into_any_element();
        }
        if self.visible.is_empty() {
            let note = self.session.missing_readings_note();
            return no_matches("No artists match the filter.", note, cx).into_any_element();
        }
        let visible = self.visible.clone();
        let selected = self.selected_artist.clone();
        let is_focused = self.list_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let list_focus = self.list_focus.clone();
        row_list(
            "artist-list",
            "Artists",
            visible.len(),
            &self.list_scroll,
            move |position| {
                let artist = artists.get(*visible.get(position)?)?;
                Some(ArtistRow::new(
                    artist,
                    selected.as_ref() == Some(artist.id()),
                    is_focused,
                    owner.clone(),
                    list_focus.clone(),
                ))
            },
        )
        .size_full()
        .into_any_element()
    }
}

impl Focusable for Artists {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.list_focus.clone()
    }
}

impl Render for Artists {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_detail_list(window);
        div()
            .id("artists-screen")
            .key_context(ROOT_CONTEXT)
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter.update(cx, |input, cx| input.focus(window, cx));
            }))
            .child(MasterDetail::new(
                "artists-panes",
                &self.panes,
                self.render_list_pane(window, cx),
                self.render_detail(cx),
            ))
    }
}

/// What a choice in an Add to Playlist menu runs: the songs are only fetched
/// once the choice is made.
fn add_selection_command(playlist: Option<&PlaylistId>) -> Box<dyn gpui_kit::Action> {
    Box::new(AddSelectionToPlaylist {
        playlist: playlist.cloned(),
    })
}

async fn artist_songs(
    session: &session::Session,
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
