use std::ops::Range;
use std::sync::Arc;

use actions::{AlbumAction, EnqueueSongs, GoToArtist, Placement, PlaySongs, ShowAlbum, ShowArtist};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    list::ListItem,
    menu::ContextMenuExt as _,
    notification::NotificationType,
    scroll::Scrollbar,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, Role,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, prelude::FluentBuilder as _,
};
use hiphile_core::Shelf;
use hiphile_core::favorites::Favorite;
use hiphile_core::home::{
    AlbumShelf, DailyRecommendations, HomeSection, HomeShelves, Item, MAX_COLUMNS,
    RecommendedStatus, ReloadFailed, ShelvesStatus, sections,
};
use subsonic::{Album, AlbumId, PlaylistId, Song, SongId, StarTarget};
use ui::card_grid::{GridMotion as Motion, card_row, columns_for, media_card, step};
use ui::list_navigation::{
    SelectFirst, SelectLast, SelectNext, SelectPageDown, SelectPageUp, SelectPrevious,
};
use ui::{
    ArtistLine, CoverArt, NowPlaying, format_duration, section_failure, section_heading,
    section_heading_with_action, section_loading, skippable_song, song_row_label, song_with_cover,
};

use crate::{
    AddSelectionToPlaylist, AddToQueue, ChoosePlaylist, HOME_CONTEXT, Open, Play, PlayNext,
    SelectLeft, SelectRight, ShowInLibrary,
};

/// The same as the library's album cover, so that opening the album needs no
/// second image.
const CARD_COVER_PIXELS: u32 = 256;
/// The same as the player's.
const ROW_COVER_PIXELS: u32 = 48;

/// The albums to try are picked again on the first occasion of another day:
/// opening, coming back to the screen or the window, a play or a library sync.
pub struct Home {
    session: session::Session,
    focus: FocusHandle,
    scroll: ScrollHandle,
    recommendations: DailyRecommendations,
    /// What the recommendations showed last.
    recommended_status: RecommendedStatus,
    recommended: Shelf<Album>,
    shelves: HomeShelves,
    /// What the shelves showed last.
    shelves_status: ShelvesStatus,
    recent: Shelf<Song>,
    most_played: Shelf<Album>,
    new_albums: Shelf<Album>,
    selection: Option<Item>,
    now_playing: Option<NowPlaying>,
    /// How many album cards fit across, from the last layout.
    columns: usize,
    _album_task: Task<()>,
    _subscriptions: Vec<Subscription>,
    _recommended_changes: Task<()>,
    _recommended_failures: Task<()>,
    _shelves_changes: Task<()>,
}

/// One child of the scrolling list, in order. Each is a direct child so that
/// the selection's can be scrolled to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    RecommendedHeading,
    RecommendedStatus,
    RecentHeading,
    RecentStatus,
    Song(usize),
    MostPlayedHeading,
    MostPlayedStatus,
    NewAlbumsHeading,
    NewAlbumsStatus,
    AlbumRow(AlbumShelf, Range<usize>),
}

impl Home {
    /// A reload of `recommendations` that fails says so in `window`.
    pub fn new(
        session: session::Session,
        shelves: HomeShelves,
        recommendations: DailyRecommendations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            // Focus changes the selection treatment
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        let recommended_changes =
            gpui_host::observe(recommendations.subscribe(), cx, Self::follow_recommended);
        let recommended_failures = gpui_host::subscribe_in(
            recommendations.notices(),
            window,
            cx,
            |_, failed: ReloadFailed, window, cx| {
                let message = failed.message().to_owned();
                window.push_notification((NotificationType::Error, message), cx);
            },
        );
        let shelves_changes = gpui_host::observe(shelves.subscribe(), cx, Self::follow_shelves);
        let recommended_status = recommendations.status();
        let shelves_status = shelves.status();
        Self {
            session,
            focus,
            scroll: ScrollHandle::new(),
            recommendations,
            recommended_status: recommended_status.clone(),
            recommended: recommended_status.albums().clone(),
            shelves,
            recent: shelves_status.recently_played().clone(),
            most_played: shelves_status.most_played().clone(),
            new_albums: shelves_status.new_albums().clone(),
            shelves_status,
            selection: None,
            now_playing: None,
            columns: 4,
            _album_task: Task::ready(()),
            _subscriptions: subscriptions,
            _recommended_changes: recommended_changes,
            _recommended_failures: recommended_failures,
            _shelves_changes: shelves_changes,
        }
    }

    fn follow_recommended(&mut self, status: RecommendedStatus, cx: &mut Context<Self>) {
        if status.albums() != self.recommended_status.albums() {
            self.recommended = status.albums().clone();
        }
        self.recommended_status = status;
        cx.notify();
    }

    fn follow_shelves(&mut self, status: ShelvesStatus, cx: &mut Context<Self>) {
        let last = &self.shelves_status;
        if status.recently_played() != last.recently_played() {
            self.recent = status.recently_played().clone();
        }
        if status.most_played() != last.most_played() {
            self.most_played = status.most_played().clone();
        }
        if status.new_albums() != last.new_albums() {
            self.new_albums = status.new_albums().clone();
        }
        self.shelves_status = status;
        cx.notify();
    }

    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = NowPlaying::new(song, playing);
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    fn parts(&self) -> Vec<Part> {
        let mut parts = Vec::new();
        for section in sections(&self.recommended, &self.recent, &self.most_played) {
            match section {
                HomeSection::Recommended => {
                    parts.push(Part::RecommendedHeading);
                    parts
                        .extend(self.album_parts(AlbumShelf::Recommended, Part::RecommendedStatus));
                }
                HomeSection::RecentlyPlayed => {
                    parts.push(Part::RecentHeading);
                    match self.recent.items().len() {
                        0 => parts.push(Part::RecentStatus),
                        songs => parts.extend((0..songs).map(Part::Song)),
                    }
                }
                HomeSection::MostPlayed => {
                    parts.push(Part::MostPlayedHeading);
                    parts.extend(self.album_parts(AlbumShelf::MostPlayed, Part::MostPlayedStatus));
                }
                HomeSection::NewAlbums => {
                    parts.push(Part::NewAlbumsHeading);
                    parts.extend(self.album_parts(AlbumShelf::New, Part::NewAlbumsStatus));
                }
            }
        }
        parts
    }

    /// The shelf's rows of as many cards as fit, or `status` while it has none.
    fn album_parts(&self, shelf: AlbumShelf, status: Part) -> Vec<Part> {
        let albums = self.albums(shelf).len();
        if albums == 0 {
            return vec![status];
        }
        let columns = self.columns.max(1);
        let shown = albums.min(columns * shelf.rows());
        (0..shown)
            .step_by(columns)
            .map(|start| Part::AlbumRow(shelf, start..(start + columns).min(shown)))
            .collect()
    }

    fn albums(&self, shelf: AlbumShelf) -> &[Album] {
        match shelf {
            AlbumShelf::Recommended => self.recommended.items(),
            AlbumShelf::MostPlayed => self.most_played.items(),
            AlbumShelf::New => self.new_albums.items(),
        }
    }

    /// What can be selected, row by row from the top.
    fn rows(&self, parts: &[Part]) -> Vec<Vec<Item>> {
        parts
            .iter()
            .filter_map(|part| match part {
                Part::Song(ix) => {
                    let song = self.recent.items().get(*ix)?;
                    Some(vec![Item::Song(song.id().clone())])
                }
                Part::AlbumRow(shelf, range) => {
                    let albums = self.albums(*shelf).get(range.clone())?;
                    let item = |album: &Album| Item::Album(*shelf, album.id().clone());
                    Some(albums.iter().map(item).collect())
                }
                _ => None,
            })
            .collect()
    }

    fn move_selection(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let parts = self.parts();
        let rows = self.rows(&parts);
        let Some(item) = step(&rows, self.selection.as_ref(), motion) else {
            return;
        };
        if let Some(ix) = self.part_of(&item, &parts) {
            // Going up a shelf brings its heading into view too
            let heading_above = ix > 0
                && matches!(motion, Motion::Up | Motion::PageUp | Motion::First)
                && matches!(
                    parts[ix - 1],
                    Part::RecommendedHeading
                        | Part::RecentHeading
                        | Part::MostPlayedHeading
                        | Part::NewAlbumsHeading
                );
            self.scroll
                .scroll_to_item(if heading_above { ix - 1 } else { ix });
        }
        self.selection = Some(item);
        cx.notify();
    }

    fn part_of(&self, item: &Item, parts: &[Part]) -> Option<usize> {
        parts.iter().position(|part| match (part, item) {
            (Part::Song(ix), Item::Song(id)) => self
                .recent
                .items()
                .get(*ix)
                .is_some_and(|song| song.id() == id),
            (Part::AlbumRow(part_shelf, range), Item::Album(shelf, id)) if part_shelf == shelf => {
                self.albums(*shelf)
                    .get(range.clone())
                    .is_some_and(|albums| albums.iter().any(|album| album.id() == id))
            }
            _ => false,
        })
    }

    fn select(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.selection = Some(item);
        cx.notify();
    }

    /// The selection, while it shows. A narrower window or a reload can take
    /// it off the screen, and a command must not act on what cannot be seen.
    /// It stays selected, for when it shows again.
    fn selection_on_screen(&self) -> Option<Item> {
        let selection = self.selection.as_ref()?;
        self.part_of(selection, &self.parts())?;
        Some(selection.clone())
    }

    fn song_ix(&self, id: &SongId) -> Option<usize> {
        self.recent.items().iter().position(|song| song.id() == id)
    }

    fn play_from(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Shelf::Loaded(songs) = &self.recent
            && ix < songs.len()
        {
            window.dispatch_action(Box::new(PlaySongs::new(songs.clone(), ix)), cx);
        }
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selection_on_screen() {
            Some(Item::Album(_, album)) => {
                window.dispatch_action(Box::new(ShowAlbum::new(album)), cx);
            }
            Some(Item::Song(song)) => {
                if let Some(ix) = self.song_ix(&song) {
                    self.play_from(ix, window, cx);
                }
            }
            None => {}
        }
    }

    fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selection_on_screen() {
            Some(Item::Album(_, album)) => self.act_on_album(album, AlbumAction::Play, window, cx),
            Some(Item::Song(song)) => {
                if let Some(ix) = self.song_ix(&song) {
                    self.play_from(ix, window, cx);
                }
            }
            None => {}
        }
    }

    fn enqueue(&mut self, placement: Placement, window: &mut Window, cx: &mut Context<Self>) {
        match self.selection_on_screen() {
            Some(Item::Album(_, album)) => {
                self.act_on_album(album, AlbumAction::Enqueue(placement), window, cx);
            }
            Some(Item::Song(id)) => {
                let song = self.song_ix(&id).map(|ix| self.recent.items()[ix].clone());
                if let Some(song) = song {
                    let songs = Arc::from([song]);
                    window.dispatch_action(Box::new(EnqueueSongs::new(songs, placement)), cx);
                }
            }
            None => {}
        }
    }

    fn selected_song(&self) -> Option<Arc<[Song]>> {
        let Some(Item::Song(id)) = self.selection_on_screen() else {
            return None;
        };
        let song = self.recent.items().get(self.song_ix(&id)?)?.clone();
        Some(Arc::from([song]))
    }

    fn add_to_playlist(
        &mut self,
        playlist: Option<PlaylistId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(Item::Album(_, album)) = self.selection_on_screen() {
            self.act_on_album(album, AlbumAction::AddToPlaylist(playlist), window, cx);
        } else if let Some(songs) = self.selected_song() {
            let command = actions::add_songs_command(songs)(playlist.as_ref());
            window.dispatch_action(command, cx);
        }
    }

    fn choose_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Item::Album(_, album)) = self.selection_on_screen() {
            self.act_on_album(album, AlbumAction::ChoosePlaylist, window, cx);
        } else if let Some(songs) = self.selected_song() {
            window.dispatch_action(Box::new(actions::ChoosePlaylist::new(songs)), cx);
        }
    }

    fn show_in_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let album = match self.selection_on_screen() {
            Some(Item::Album(_, album)) => Some(album),
            Some(Item::Song(id)) => self
                .song_ix(&id)
                .and_then(|ix| self.recent.items()[ix].album_id().cloned()),
            None => None,
        };
        if let Some(album) = album {
            window.dispatch_action(Box::new(ShowAlbum::new(album)), cx);
        }
    }

    fn go_to_artist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let artist = match self.selection_on_screen() {
            Some(Item::Album(shelf, id)) => self
                .albums(shelf)
                .iter()
                .find(|album| album.id() == &id)
                .and_then(|album| album.artist_id().cloned()),
            Some(Item::Song(id)) => self
                .song_ix(&id)
                .and_then(|ix| self.recent.items()[ix].artist_id().cloned()),
            None => None,
        };
        if let Some(artist) = artist {
            window.dispatch_action(Box::new(ShowArtist::new(artist)), cx);
        }
    }

    fn toggle_favorite(&self, window: &mut Window, cx: &mut Context<Self>) {
        let favorite = match self.selection_on_screen() {
            Some(Item::Album(shelf, id)) => self
                .albums(shelf)
                .iter()
                .find(|album| album.id() == &id)
                .cloned()
                .map(Favorite::Album),
            Some(Item::Song(id)) => self
                .song_ix(&id)
                .map(|ix| Favorite::Song(self.recent.items()[ix].clone())),
            None => None,
        };
        if let Some(favorite) = favorite {
            actions::toggle_favorite(favorite, window, cx);
        }
    }

    /// Plays or queues `album`, through its saved songs when there are any.
    /// Replaces an action still waiting for its songs: the latest one counts.
    fn act_on_album(
        &mut self,
        album: AlbumId,
        action: AlbumAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._album_task = action.start(album, &self.session, window, cx);
    }

    /// Runs while the frame is prepainted, so a change takes effect in the next one.
    fn observe_width(
        this: &gpui_kit::WeakEntity<Self>,
        width: gpui_kit::Pixels,
        window: &Window,
        cx: &mut App,
    ) {
        let columns = columns_for(width / window.rem_size()).min(MAX_COLUMNS);
        let Some(home) = this.upgrade() else {
            return;
        };
        if home.read(cx).columns == columns {
            return;
        }
        cx.defer(move |cx| {
            home.update(cx, |home, cx| {
                home.columns = columns;
                cx.notify();
            });
        });
    }

    fn render_part(&self, part: &Part, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match part {
            Part::RecommendedHeading => self.render_recommended_heading(cx),
            Part::RecommendedStatus => self.render_recommended_status(cx),
            Part::RecentHeading => section_heading("Recently played").into_any_element(),
            Part::MostPlayedHeading => section_heading("Most played").into_any_element(),
            Part::NewAlbumsHeading => section_heading("New albums").into_any_element(),
            Part::RecentStatus => self.render_recent_status(cx),
            Part::MostPlayedStatus => self.render_most_played_status(cx),
            Part::NewAlbumsStatus => self.render_new_albums_status(cx),
            Part::Song(ix) => self.render_song(*ix, window, cx),
            Part::AlbumRow(shelf, range) => {
                self.render_album_row(*shelf, range.clone(), window, cx)
            }
        }
    }

    fn render_recommended_heading(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.recommended_status.is_reloadable() {
            return section_heading("Recommended").into_any_element();
        }
        section_heading_with_action(
            "Recommended",
            Button::new("reload-recommended")
                .ghost()
                .small()
                .icon(IconName::RefreshCw)
                .accessibility_label("Reload recommendations")
                .loading(self.recommended_status.is_recommending())
                .on_click(cx.listener(|this, _, _, _| this.recommendations.reload())),
        )
        .into_any_element()
    }

    fn render_recommended_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.recommended {
            Shelf::Failed(error) => section_failure(
                error.clone().into(),
                Button::new("retry-recommended")
                    .label("Retry")
                    .small()
                    .on_click(
                        cx.listener(|this, _, _, _| this.shelves.retry(HomeSection::Recommended)),
                    ),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => section_loading("Loading recommendations", cx),
        }
    }

    fn render_recent_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.recent {
            Shelf::Failed(error) => section_failure(
                error.clone().into(),
                Button::new("retry-recent")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, _| {
                        this.shelves.retry(HomeSection::RecentlyPlayed);
                    })),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => {
                section_loading("Loading recently played songs", cx)
            }
        }
    }

    fn render_most_played_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.most_played {
            Shelf::Failed(error) => section_failure(
                error.clone().into(),
                Button::new("retry-most-played")
                    .label("Retry")
                    .small()
                    .on_click(
                        cx.listener(|this, _, _, _| this.shelves.retry(HomeSection::MostPlayed)),
                    ),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => section_loading("Loading most played albums", cx),
        }
    }

    fn render_new_albums_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.new_albums {
            Shelf::Failed(error) => section_failure(
                error.clone().into(),
                Button::new("retry-new-albums")
                    .label("Retry")
                    .small()
                    .on_click(
                        cx.listener(|this, _, _, _| this.shelves.retry(HomeSection::NewAlbums)),
                    ),
                cx,
            ),
            Shelf::Loading => section_loading("Loading new albums", cx),
            Shelf::Loaded(_) => div()
                .flex_none()
                .px_6()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("This server has no albums.")
                .into_any_element(),
        }
    }

    fn render_song(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(song) = self.recent.items().get(ix) else {
            return div().into_any_element();
        };
        let id = song.id().clone();
        let target = StarTarget::Song(id.clone());
        let is_selected = self.selection == Some(Item::Song(id.clone()));
        let focused = self.focus.is_focused(window);
        let title = SharedString::from(song.title().to_owned());
        let details = ArtistLine::of_song(song);
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let cover = song.cover_art().map(|cover_art| {
            self.session
                .cover_art_url(cover_art, ROW_COVER_PIXELS)
                .into()
        });
        let marker = self.now_playing.as_ref().and_then(|now| now.state_of(&id));
        let label = song_row_label(song, &title, marker, cx);
        let focus = self.focus.clone();
        let has_album = song.album_id().is_some();
        let has_artist = song.artist_id().is_some();
        let row_song = song.clone();
        ListItem::new(ElementId::Name(format!("home-song-{id}").into()))
            .selected(is_selected)
            .secondary_selected(focused && is_selected)
            .flex_none()
            .h_12()
            .px_6()
            .w_full()
            .accessibility_label(label)
            .child(skippable_song(
                &row_song,
                song_with_cover(cover, title, details, marker, duration.into(), cx),
                cx,
            ))
            .on_click(cx.listener({
                let id = id.clone();
                move |this, event: &ClickEvent, window, cx| {
                    this.select(Item::Song(id.clone()), window, cx);
                    if event.click_count() >= 2 {
                        this.play_from(ix, window, cx);
                    }
                }
            }))
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.select(Item::Song(id.clone()), window, cx);
                }),
            )
            .context_menu(move |menu, window, cx| {
                let menu = menu.action_context(focus.clone());
                let menu =
                    actions::queue_menu_items(menu, &row_song, Play, PlayNext, AddToQueue, cx)
                        .separator();
                let menu =
                    actions::add_to_playlist_submenu(menu, add_selection_command, window, cx);
                actions::favorite_menu_item(menu, &target, cx)
                    .when(has_album, |menu| {
                        menu.menu("Go to Album", Box::new(ShowInLibrary))
                    })
                    .when(has_artist, |menu| {
                        menu.menu("Go to Artist", Box::new(GoToArtist))
                    })
            })
            .into_any_element()
    }

    fn render_album_row(
        &self,
        shelf: AlbumShelf,
        range: Range<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let albums = self.albums(shelf).get(range).unwrap_or_default();
        let cards = albums
            .iter()
            .map(|album| self.render_album_card(shelf, album, window, cx))
            .collect::<Vec<_>>();
        card_row(self.columns, cards).into_any_element()
    }

    fn render_album_card(
        &self,
        shelf: AlbumShelf,
        album: &Album,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = album.id().clone();
        let target = StarTarget::Album(id.clone());
        let is_selected = self.selection == Some(Item::Album(shelf, id.clone()));
        let focused = self.focus.is_focused(window);
        let name = SharedString::from(album.name().to_owned());
        let cover = album.cover_art().map(|cover_art| {
            self.session
                .cover_art_url(cover_art, CARD_COVER_PIXELS)
                .into()
        });
        let focus = self.focus.clone();
        let has_artist = album.artist_id().is_some();
        let artist = ui::ArtistLine::new(ui::album_artist_link(album), None);
        media_card(
            ElementId::Name(format!("home-album-{shelf:?}-{id}").into()),
            CoverArt::new(cover),
            name,
            artist,
            is_selected,
            focused,
            cx,
        )
        .on_click(cx.listener({
            let id = id.clone();
            move |this, _, window, cx| {
                this.select(Item::Album(shelf, id.clone()), window, cx);
                this.open(window, cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _, window, cx| {
                this.select(Item::Album(shelf, id.clone()), window, cx);
            }),
        )
        .context_menu(move |menu, window, cx| {
            let menu = menu
                .action_context(focus.clone())
                .menu("Play", Box::new(Play))
                .menu("Play Next", Box::new(PlayNext))
                .menu("Add to Queue", Box::new(AddToQueue))
                .separator();
            let menu = actions::add_to_playlist_submenu(menu, add_selection_command, window, cx);
            actions::favorite_menu_item(menu, &target, cx)
                .menu("Show in Albums", Box::new(ShowInLibrary))
                .when(has_artist, |menu| {
                    menu.menu("Go to Artist", Box::new(GoToArtist))
                })
        })
        .into_any_element()
    }
}

impl Focusable for Home {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let parts = self.parts();
        let children = parts
            .iter()
            .map(|part| self.render_part(part, window, cx))
            .collect::<Vec<_>>();
        let owner = cx.entity().downgrade();
        let scroll = self.scroll.clone();
        div()
            .id("home")
            .key_context(HOME_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                this.move_selection(Motion::Down, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| {
                this.move_selection(Motion::Up, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| {
                this.move_selection(Motion::Left, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| {
                this.move_selection(Motion::Right, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPageUp, _, cx| {
                this.move_selection(Motion::PageUp, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPageDown, _, cx| {
                this.move_selection(Motion::PageDown, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| {
                this.move_selection(Motion::First, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| {
                this.move_selection(Motion::Last, cx);
            }))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &Play, window, cx| this.play(window, cx)))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.enqueue(Placement::Next, window, cx);
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.enqueue(Placement::Last, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowInLibrary, window, cx| {
                this.show_in_library(window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoToArtist, window, cx| {
                this.go_to_artist(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &actions::ToggleFavorite, window, cx| {
                    this.toggle_favorite(window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, action: &AddSelectionToPlaylist, window, cx| {
                    this.add_to_playlist(action.playlist.clone(), window, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &ChoosePlaylist, window, cx| {
                this.choose_playlist(window, cx);
            }))
            .child(
                v_flex()
                    // An element with a scroll handle lists no bounds here:
                    // the handle has them.
                    .on_children_prepainted(move |_, window, cx| {
                        Home::observe_width(&owner, scroll.bounds().size.width, window, cx);
                    })
                    .id("home-parts")
                    .role(Role::List)
                    .aria_label("Home")
                    .size_full()
                    .pb_4()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(children),
            )
            .child(Scrollbar::vertical(&self.scroll))
    }
}

/// What a choice in an Add to Playlist menu runs: an album's songs are only
/// fetched once the choice is made.
fn add_selection_command(playlist: Option<&PlaylistId>) -> Box<dyn gpui_kit::Action> {
    Box::new(AddSelectionToPlaylist {
        playlist: playlist.cloned(),
    })
}

#[cfg(test)]
mod tests;
