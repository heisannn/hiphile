use std::mem;
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
use hiphile_core::artists::{ArtistDetail, ArtistList, ArtistListStatus, ArtistOrder, ArtistPage};
use hiphile_core::favorites::Favorite;
use hiphile_core::{Shelf, Shown};
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
    /// The list's status last taken, which a new one is compared with.
    status: ArtistListStatus,
    /// What is on show of the list, which follows `artist_list`.
    artists: Shelf<Artist>,
    shown: Option<Shown<Artist>>,
    refresh_error: Option<SharedString>,
    is_refreshing: bool,
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
    _list_changes: Task<()>,
    _detail_changes: Task<()>,
    _subscriptions: Vec<Subscription>,
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
                InputEvent::Change => this.filter_changed(cx),
                InputEvent::PressEnter { .. } => this.list_focus.focus(window, cx),
                _ => {}
            }),
            // Focus changes the selection treatment
            cx.on_focus(&list_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&list_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus(&detail_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&detail_focus, window, |_, _, cx| cx.notify()),
        ];
        let status = artist_list.status();
        let list_changes = gpui_host::observe(artist_list.subscribe(), cx, Self::follow_list);
        let detail_changes = gpui_host::observe(artist_detail.subscribe(), cx, Self::follow_detail);
        let mut this = Self {
            session,
            artist_list,
            artist_detail,
            is_activated: false,
            status: status.clone(),
            artists: status.artists().clone(),
            shown: None,
            refresh_error: None,
            is_refreshing: false,
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
            _list_changes: list_changes,
            _detail_changes: detail_changes,
            _subscriptions: subscriptions,
        };
        this.show_list(status.shown().cloned(), cx);
        this
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        if !self.is_activated {
            self.is_activated = true;
            self.artist_list.open();
            self.sync(cx);
        }
    }

    /// Takes the list's status at once after a call into it, so that what follows does not act
    /// on the old one.
    fn sync(&mut self, cx: &mut Context<Self>) {
        self.follow_list(self.artist_list.status(), cx);
    }

    fn follow_list(&mut self, status: ArtistListStatus, cx: &mut Context<Self>) {
        let old = mem::replace(&mut self.status, status);
        self.refresh_error = self.status.failure().map(|error| error.to_owned().into());
        if old.artists() != self.status.artists() {
            self.artists = self.status.artists().clone();
        }
        if old.shown() != self.status.shown() {
            self.show_list(self.status.shown().cloned(), cx);
        }
        self.set_refreshing(self.status.is_refreshing(), cx);
    }

    fn set_refreshing(&mut self, is_refreshing: bool, cx: &mut Context<Self>) {
        self.is_refreshing = is_refreshing;
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
    /// Artists in a new order keep the selected one in view.
    fn show_list(&mut self, shown: Option<Shown<Artist>>, cx: &mut Context<Self>) {
        let old = mem::replace(&mut self.shown, shown);
        let Some(artists) = self.shown.as_ref().map(|shown| shown.items().clone()) else {
            return;
        };
        if old
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old.items(), &artists))
        {
            self.scroll_to_selected_artist();
            return;
        }
        let is_listed = |list: &[Artist], id: &ArtistId| list.iter().any(|a| a.id() == id);
        let has_gone = self.selected_artist.as_ref().is_some_and(|selected| {
            let was_listed = old
                .as_ref()
                .is_some_and(|old| is_listed(old.items(), selected));
            was_listed && !is_listed(&artists, selected)
        });
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

    fn listed_artists(&self) -> Option<&Arc<[Artist]>> {
        self.shown.as_ref().map(Shown::items)
    }

    fn visible_len(&self) -> usize {
        self.shown.as_ref().map_or(0, |shown| shown.visible().len())
    }

    fn visible_artist(&self, position: usize) -> Option<&Artist> {
        self.shown.as_ref()?.get(position)
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected_artist.as_ref()?;
        self.shown
            .as_ref()?
            .position(|artist| artist.id() == selected)
    }

    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        self.artist_list.set_query(&query);
        self.sync(cx);
    }

    fn choose_order(&mut self, order: ArtistOrder, cx: &mut Context<Self>) {
        self.artist_list.choose_order(order);
        self.sync(cx);
    }

    fn choose_direction(&mut self, is_descending: bool, cx: &mut Context<Self>) {
        self.artist_list.choose_direction(is_descending);
        self.sync(cx);
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
            .listed_artists()
            .and_then(|artists| artists.iter().find(|listed| listed.id() == &artist));
        self.artist_detail.show(&artist, listed);
        self.take_page(self.artist_detail.page());
        cx.notify();
    }

    /// Clears a filter that hides `artist`. One the list does not have, as one who
    /// only appears on songs, shows in the detail all the same.
    pub fn reveal_artist(&mut self, artist: ArtistId, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(cx);
        let is_hidden = self.shown.as_ref().is_some_and(|shown| {
            let position = shown
                .items()
                .iter()
                .position(|listed| listed.id() == &artist);
            position.is_some_and(|position| !shown.visible().contains(&position))
        });
        if is_hidden {
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.filter_changed(cx);
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
        let len = self.visible_len();
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
        let count = self.listed_artists().map(|artists| artists.len());
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
        let current = self.status.sort();
        let order = |label: &'static str, order: ArtistOrder| {
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
        let shown = match (&self.shown, &self.artists) {
            (Some(shown), _) => shown.clone(),
            (None, Shelf::Loading | Shelf::Loaded(_)) => {
                return centered(Spinner::new()).into_any_element();
            }
            (None, Shelf::Failed(error)) => {
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
        };
        if shown.items().is_empty() {
            return centered(muted_text("This server has no artists.", cx)).into_any_element();
        }
        if shown.visible().is_empty() {
            let note = self.session.missing_readings_note();
            return no_matches("No artists match the filter.", note, cx).into_any_element();
        }
        let selected = self.selected_artist.clone();
        let is_focused = self.list_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let list_focus = self.list_focus.clone();
        row_list(
            "artist-list",
            "Artists",
            shown.visible().len(),
            &self.list_scroll,
            move |position| {
                let artist = shown.get(position)?;
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
