use std::sync::Arc;

use actions::{AlbumAction, Placement};
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
use hiphile_core::Shelf;
use hiphile_core::artists::{
    ArtistDates, ArtistDetail, ArtistList, ArtistListStatus, ArtistOrder, ArtistPage, ArtistSort,
    arrange, artist_index,
};
use hiphile_core::favorites::Favorite;
use subsonic::{Artist, ArtistId, PlaylistId, SongId};
use ui::card_grid::GridMotion;
use ui::list_navigation::{ListNavigationExt as _, Motion, page_rows};
use ui::{
    ListFilter, ListPaneHeader, LoadFailure, MasterDetail, NowPlaying, RefreshFailure, SortOption,
    centered, muted_text, no_matches, row_list, sort_menu,
};

mod artist_row;
mod detail;

use artist_row::ArtistRow;
use detail::{Item, ListedDetail};

use crate::{
    ARTISTS_CONTEXT, AddSelectionToPlaylist, AddToPlaylist, AddToQueue, FocusFilter, Open, Play,
    PlayNext, ROOT_CONTEXT,
};

const TOP_SONGS_COLLAPSED: usize = 5;
/// In pixels: about a row of cards, so that scrolling does not show them arriving.
const DETAIL_OVERDRAW: f32 = 300.;

pub enum ArtistsEvent {
    /// The user chose another artist over this one.
    LeftArtist(ArtistId),
}

pub struct Artists {
    session: session::Session,
    artist_list: ArtistList,
    artist_detail: ArtistDetail,
    is_activated: bool,
    /// What is on show of the list, which follows `artist_list`.
    artists: Shelf<Artist>,
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
    /// What is on show of the detail, which follows `artist_detail`.
    detail: Option<ArtistPage>,
    /// The page last taken from `artist_detail`, which a page it sends again is compared with.
    taken_page: Option<ArtistPage>,
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
    _action_task: Task<()>,
    _index_task: Task<()>,
    _list_changes: Task<()>,
    _detail_changes: Task<()>,
    _subscriptions: Vec<Subscription>,
    _readings_changes: Option<Task<()>>,
}

impl EventEmitter<ArtistsEvent> for Artists {}

impl Artists {
    pub fn new(
        session: session::Session,
        artist_list: ArtistList,
        artist_detail: ArtistDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let list_changes = gpui_host::observe(artist_list.subscribe(), cx, Self::follow_list);
        let detail_changes = gpui_host::observe(artist_detail.subscribe(), cx, Self::follow_detail);
        Self {
            session,
            artist_list,
            artist_detail,
            is_activated: false,
            artists: Shelf::Loading,
            refresh_error: None,
            is_refreshing: false,
            index: Arc::default(),
            visible: Arc::default(),
            sort: ArtistSort::default(),
            dates: Arc::default(),
            selected_artist: None,
            revealing: false,
            detail: None,
            taken_page: None,
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
            _action_task: Task::ready(()),
            _index_task: Task::ready(()),
            _list_changes: list_changes,
            _detail_changes: detail_changes,
            _subscriptions: subscriptions,
            _readings_changes: readings_changes,
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        if !self.is_activated {
            self.is_activated = true;
            self.artist_list.open();
            self.follow_list(self.artist_list.status(), cx);
        }
    }

    fn follow_list(&mut self, status: ArtistListStatus, cx: &mut Context<Self>) {
        self.refresh_error = status.failure().map(|error| error.to_owned().into());
        let has_new_dates = self.dates != *status.dates();
        self.dates = status.dates().clone();
        if has_new_dates && *status.artists() == self.artists {
            self.apply_filter(cx);
        }
        self.show_artists(status.artists().clone(), status.is_refreshing(), cx);
    }

    fn show_artists(
        &mut self,
        artists: Shelf<Artist>,
        is_refreshing: bool,
        cx: &mut Context<Self>,
    ) {
        self.is_refreshing = is_refreshing;
        if artists != self.artists {
            match artists {
                Shelf::Loaded(artists) => self.set_artists(artists, cx),
                artists => self.artists = artists,
            }
        }
        if !is_refreshing {
            self.revealing = false;
        }
        cx.notify();
    }

    fn follow_detail(&mut self, page: Option<ArtistPage>, cx: &mut Context<Self>) {
        if page != self.taken_page {
            self.take_page(page);
            cx.notify();
        }
    }

    fn take_page(&mut self, page: Option<ArtistPage>) {
        self.taken_page.clone_from(&page);
        self.detail = page;
    }

    /// The selection stays, also on an artist the list does not have, as one who
    /// only appears on songs, unless the artist was on the list and has gone.
    fn set_artists(&mut self, artists: impl Into<Arc<[Artist]>>, cx: &mut Context<Self>) {
        let artists = artists.into();
        let is_listed = |list: &[Artist], id: &ArtistId| list.iter().any(|a| a.id() == id);
        let has_gone = self.selected_artist.as_ref().is_some_and(|selected| {
            let was_listed = self
                .loaded_artists()
                .is_some_and(|old| is_listed(old, selected));
            was_listed && !is_listed(&artists, selected)
        });
        self.artists = Shelf::Loaded(artists);
        self.index_artists(cx);
        if has_gone {
            self.selected_artist = None;
            self.artist_detail.close();
            self.take_page(None);
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

    fn loaded_artists(&self) -> Option<&Arc<[Artist]>> {
        match &self.artists {
            Shelf::Loaded(artists) => Some(artists),
            Shelf::Loading | Shelf::Failed(_) => None,
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
        self.artist_detail.show(&artist, listed);
        self.take_page(self.artist_detail.page());
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
        let songs = self.artist_detail.songs(&artist);
        self.artist_being_fetched = Some(artist);
        cx.notify();
        self._action_task = cx.spawn_in(window, async move |this, cx| {
            let songs = songs.await;
            cx.update(|window, cx| match songs {
                Ok(songs) => {
                    if let Some(command) = action.command(songs.into()) {
                        window.dispatch_action(command, cx);
                    }
                }
                Err(message) => {
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
                        .on_click(cx.listener(|this, _, _, _| this.artist_list.reload())),
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
            Shelf::Loading => return centered(Spinner::new()).into_any_element(),
            Shelf::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load artists",
                    error.clone().into(),
                    Button::new("retry-artists")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.list_focus.focus(window, cx);
                            this.artist_list.reload();
                        })),
                )
                .into_any_element();
            }
            Shelf::Loaded(artists) => artists.clone(),
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

#[cfg(test)]
mod tests;
