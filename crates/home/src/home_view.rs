use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use actions::{AlbumAction, EnqueueSongs, Placement, PlaySongs, ShowAlbum};
use futures::future::join_all;
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, button::Button, h_flex,
    list::ListItem, menu::ContextMenuExt as _, scroll::Scrollbar, spinner::Spinner, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, Role,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, prelude::FluentBuilder as _,
};
use subsonic::{Album, AlbumId, PlaylistId, Song, SongId};
use ui::list_navigation::{
    CONTROL_CONTEXT, SelectFirst, SelectLast, SelectNext, SelectPageDown, SelectPageUp,
    SelectPrevious,
};
use ui::{CoverArt, NowPlaying, PlayState, PlayStateIcon, RowText, format_duration, join_details};

use crate::layout::{AlbumShelf, Item, Motion, columns_for, step};
use crate::recent::most_recent;
use crate::recommend::{Recommendations, fetch_recommended, today};
use crate::{
    AddSelectionToPlaylist, AddToQueue, ChoosePlaylist, HOME_CONTEXT, Open, Play, PlayNext,
    SelectLeft, SelectRight, ShowInLibrary,
};

/// How many of the songs played last show.
const RECENT_SONGS: usize = 8;
/// How many of the albums played most to fetch, for the widest window.
const MOST_PLAYED: u32 = 12;
/// How many rows of the albums added last show.
const NEW_ALBUM_ROWS: usize = 2;
/// How many of the albums added last to fetch, for the widest window.
const NEW_ALBUMS: u32 = 24;
/// Pixel size requested for an album card's cover, the same as the library's
/// album cover, so that opening the album needs no second image.
const CARD_COVER_PIXELS: u32 = 512;
/// Pixel size requested for a song row's cover, the same as the player's.
const ROW_COVER_PIXELS: u32 = 96;
/// How long to wait before picking again after a pick that went wrong. A pick
/// takes the server's outside service several seconds per album, and windows
/// can come to the front often.
const RECOMMENDED_RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
/// The group of an album card, for the hover outline over its cover.
const CARD_GROUP: &str = "home-album-card";

/// The home screen: albums to try, the songs played last, the albums played
/// most and the albums added last. Owns those lists, the selection among them,
/// and the focus.
///
/// The lists come from the server, which alone knows every play. An item stays
/// on show while a reload that replaces it is on its way or fails.
///
/// The albums to try stay the same all day, so they are kept in the catalog
/// and picked again on the first occasion of another day: opening, coming back
/// to the screen or the window, a play or a library sync.
pub struct Home {
    session: session::Session,
    focus: FocusHandle,
    scroll: ScrollHandle,
    recommended: Shelf<Album>,
    /// The day the albums on show were picked for.
    recommended_day: Option<String>,
    /// Whether the kept albums are being read or new ones picked.
    recommending: bool,
    /// When a pick that went wrong may be made again.
    recommended_retry: Option<Instant>,
    recent: Shelf<Song>,
    most_played: Shelf<Album>,
    new_albums: Shelf<Album>,
    selection: Option<Item>,
    now_playing: Option<NowPlaying>,
    /// How many album cards fit across, from the last layout.
    columns: usize,
    _recommended_task: Task<()>,
    _recent_task: Task<()>,
    _most_played_task: Task<()>,
    _new_albums_task: Task<()>,
    _album_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

enum Shelf<T> {
    Loading,
    Loaded(Arc<[T]>),
    Failed(SharedString),
}

impl<T> Shelf<T> {
    fn items(&self) -> &[T] {
        match self {
            Self::Loaded(items) => items,
            Self::Loading | Self::Failed(_) => &[],
        }
    }

    /// Takes in what a load found. A failure leaves items on show in place.
    fn finish(&mut self, result: Result<Vec<T>, SharedString>) {
        match result {
            Ok(items) => *self = Self::Loaded(items.into()),
            Err(_) if matches!(self, Self::Loaded(_)) => {}
            Err(error) => *self = Self::Failed(error),
        }
    }

    /// Shows the load starting, unless there are items to keep on show.
    fn begin(&mut self) {
        if !matches!(self, Self::Loaded(_)) {
            *self = Self::Loading;
        }
    }
}

/// One child of the scrolling list, in order. Each is a direct child so that
/// the selection's can be scrolled to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    RecommendedHeading,
    /// A spinner or a failure.
    RecommendedStatus,
    RecentHeading,
    /// A spinner or a failure.
    RecentStatus,
    /// The song at this position.
    Song(usize),
    MostPlayedHeading,
    /// A spinner or a failure.
    MostPlayedStatus,
    NewAlbumsHeading,
    /// A spinner, a failure, or word of there being no albums.
    NewAlbumsStatus,
    /// The albums at these positions on this shelf.
    AlbumRow(AlbumShelf, Range<usize>),
}

impl Home {
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            // Focus changes the selection treatment.
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.renew_recommended(cx);
                }
            }),
        ];
        let mut this = Self {
            session,
            focus,
            scroll: ScrollHandle::new(),
            recommended: Shelf::Loading,
            recommended_day: None,
            recommending: false,
            recommended_retry: None,
            recent: Shelf::Loading,
            most_played: Shelf::Loading,
            new_albums: Shelf::Loading,
            selection: None,
            now_playing: None,
            columns: 4,
            _recommended_task: Task::ready(()),
            _recent_task: Task::ready(()),
            _most_played_task: Task::ready(()),
            _new_albums_task: Task::ready(()),
            _album_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.load_recommended(cx);
        this.reload_recently_played(cx);
        this.reload_most_played(cx);
        this.reload_new_albums(cx);
        this
    }

    /// Shows the albums kept from the last pick, whichever day that was, and
    /// picks new ones if it was not today.
    fn load_recommended(&mut self, cx: &mut Context<Self>) {
        self.recommending = true;
        let catalog = self.session.catalog().cloned();
        let user = self.session.username().to_owned();
        self._recommended_task = cx.spawn(async move |this, cx| {
            let kept = match catalog {
                Some(catalog) => {
                    cx.background_spawn(async move { catalog.recommended(&user) })
                        .await
                        // The albums can be picked again.
                        .ok()
                        .flatten()
                }
                None => None,
            };
            this.update(cx, |this, cx| {
                this.recommending = false;
                if let Some(kept) = kept {
                    this.recommended_day = Some(kept.day);
                    this.recommended.finish(Ok(kept.albums));
                    this.keep_shelves_off_recommended();
                    cx.notify();
                }
                this.renew_recommended(cx);
            })
            .ok();
        });
    }

    /// Picks new albums to try if those on show are from another day. Call
    /// when something happens, for an app left open overnight.
    pub fn renew_recommended(&mut self, cx: &mut Context<Self>) {
        let waiting = self
            .recommended_retry
            .is_some_and(|retry| Instant::now() < retry);
        if !self.recommending
            && !waiting
            && self.recommended_day.as_deref() != Some(today().as_str())
        {
            self.pick_recommended(cx);
        }
    }

    fn pick_recommended(&mut self, cx: &mut Context<Self>) {
        self.recommending = true;
        self.recommended_retry = None;
        self.recommended.begin();
        let client = self.session.client().clone();
        let day = today();
        self._recommended_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    fetch_recommended(&client, NEW_ALBUMS, MOST_PLAYED).await
                })
                .await;
            this.update(cx, |this, cx| {
                let result = result.map_err(|error| session::describe(&error).into());
                this.finish_recommended(result, day, cx);
            })
            .ok();
        });
        cx.notify();
    }

    /// Takes in a pick made for `day`. Only a complete one settles the day; the
    /// next occasion tries again after one that went wrong. Albums on show stay
    /// there unless a complete pick found others.
    fn finish_recommended(
        &mut self,
        result: Result<Recommendations, SharedString>,
        day: String,
        cx: &mut Context<Self>,
    ) {
        self.recommending = false;
        match result {
            Ok(found) => {
                if found.complete {
                    if !found.albums.is_empty() {
                        self.keep_recommended(&day, &found.albums, cx);
                    }
                    self.recommended_day = Some(day);
                } else {
                    self.recommended_retry = Some(Instant::now() + RECOMMENDED_RETRY_AFTER);
                }
                let better = found.complete && !found.albums.is_empty();
                if better || self.recommended.items().is_empty() {
                    self.recommended.finish(Ok(found.albums));
                }
            }
            Err(error) => {
                self.recommended_retry = Some(Instant::now() + RECOMMENDED_RETRY_AFTER);
                self.recommended.finish(Err(error));
            }
        }
        self.keep_shelves_off_recommended();
        cx.notify();
    }

    fn keep_recommended(&self, day: &str, albums: &[Album], cx: &mut Context<Self>) {
        let Some(catalog) = self.session.catalog().cloned() else {
            return;
        };
        let user = self.session.username().to_owned();
        let (day, albums) = (day.to_owned(), albums.to_vec());
        // Failing to keep them only means another pick next time.
        cx.background_spawn(async move { catalog.put_recommended(&user, &day, &albums).ok() })
            .detach();
    }

    /// Asks again what the server has after a library sync, which may also
    /// give a library that had nothing to pick from something to recommend.
    pub fn library_synced(&mut self, cx: &mut Context<Self>) {
        self.reload_most_played(cx);
        self.reload_new_albums(cx);
        if self.recommended.items().is_empty() {
            self.recommended_day = None;
        }
        self.renew_recommended(cx);
    }

    /// A recommendation is something the other album shelves do not show,
    /// also when an album gets there after the day's pick.
    fn keep_shelves_off_recommended(&mut self) {
        let is_shelved = |album: &Album| {
            [&self.most_played, &self.new_albums]
                .iter()
                .flat_map(|shelf| shelf.items())
                .any(|shelved| shelved.id() == album.id())
        };
        if let Shelf::Loaded(albums) = &self.recommended
            && albums.iter().any(is_shelved)
        {
            let others = albums.iter().filter(|album| !is_shelved(album)).cloned();
            self.recommended = Shelf::Loaded(others.collect());
        }
    }

    /// Takes in a play the server has heard of, which can change what was
    /// played last and most.
    pub fn song_played(&mut self, cx: &mut Context<Self>) {
        self.reload_recently_played(cx);
        self.reload_most_played(cx);
        self.renew_recommended(cx);
    }

    /// Asks the server again what was played most.
    fn reload_most_played(&mut self, cx: &mut Context<Self>) {
        self.most_played.begin();
        let client = self.session.client().clone();
        self._most_played_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.frequent_albums(MOST_PLAYED).await })
                .await;
            this.update(cx, |this, cx| {
                this.most_played
                    .finish(result.map_err(|error| session::describe(&error).into()));
                this.keep_shelves_off_recommended();
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// Asks the server again what was played last, as after a play.
    fn reload_recently_played(&mut self, cx: &mut Context<Self>) {
        self.recent.begin();
        let client = self.session.client().clone();
        self._recent_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { fetch_recently_played(&client).await })
                .await;
            this.update(cx, |this, cx| {
                this.finish_recently_played(
                    result.map_err(|error| session::describe(&error).into()),
                );
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// Takes in a load of what was played last. Songs on show stay there when
    /// the load missed some of the albums, as it may have missed the latest.
    fn finish_recently_played(&mut self, result: Result<RecentlyPlayed, SharedString>) {
        match result {
            Ok(found) if !found.complete && matches!(self.recent, Shelf::Loaded(_)) => {}
            result => self.recent.finish(result.map(|found| found.songs)),
        }
    }

    /// Asks the server again what was added last.
    fn reload_new_albums(&mut self, cx: &mut Context<Self>) {
        self.new_albums.begin();
        let client = self.session.client().clone();
        self._new_albums_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.newest_albums(NEW_ALBUMS).await })
                .await;
            this.update(cx, |this, cx| {
                this.new_albums
                    .finish(result.map_err(|error| session::describe(&error).into()));
                this.keep_shelves_off_recommended();
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// Marks `song` in the list, or clears the mark.
    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = NowPlaying::new(song, playing);
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    /// The children of the scrolling list. A shelf other than the new albums
    /// with nothing to show goes away, as a server with no plays or albums yet
    /// has nothing to say about it.
    fn parts(&self) -> Vec<Part> {
        let mut parts = Vec::new();
        match &self.recommended {
            Shelf::Loaded(albums) if albums.is_empty() => {}
            Shelf::Loaded(albums) => {
                parts.push(Part::RecommendedHeading);
                parts.push(Part::AlbumRow(
                    AlbumShelf::Recommended,
                    0..albums.len().min(self.columns.max(1)),
                ));
            }
            Shelf::Loading | Shelf::Failed(_) => {
                parts.extend([Part::RecommendedHeading, Part::RecommendedStatus]);
            }
        }
        match &self.recent {
            Shelf::Loaded(songs) if songs.is_empty() => {}
            Shelf::Loaded(songs) => {
                parts.push(Part::RecentHeading);
                parts.extend((0..songs.len()).map(Part::Song));
            }
            Shelf::Loading | Shelf::Failed(_) => {
                parts.extend([Part::RecentHeading, Part::RecentStatus]);
            }
        }
        match &self.most_played {
            Shelf::Loaded(albums) if albums.is_empty() => {}
            Shelf::Loaded(albums) => {
                parts.push(Part::MostPlayedHeading);
                parts.push(Part::AlbumRow(
                    AlbumShelf::MostPlayed,
                    0..albums.len().min(self.columns.max(1)),
                ));
            }
            Shelf::Loading | Shelf::Failed(_) => {
                parts.extend([Part::MostPlayedHeading, Part::MostPlayedStatus]);
            }
        }
        parts.push(Part::NewAlbumsHeading);
        let albums = self.new_albums.items().len();
        if albums == 0 {
            parts.push(Part::NewAlbumsStatus);
        } else {
            let columns = self.columns.max(1);
            let shown = albums.min(columns * NEW_ALBUM_ROWS);
            parts.extend(
                (0..shown).step_by(columns).map(|start| {
                    Part::AlbumRow(AlbumShelf::New, start..(start + columns).min(shown))
                }),
            );
        }
        parts
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
            // Going up a shelf brings its heading into view too.
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

    /// Where in `parts` the item shows.
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

    /// Plays the songs on show, from the one at `ix`.
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

    /// The selected song on its own, when a song is what's selected.
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

    /// Plays or queues `album`, through its saved songs when there are any.
    /// Replaces an action still waiting for its songs: the latest one counts.
    fn act_on_album(
        &mut self,
        album: AlbumId,
        action: AlbumAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.clone();
        self._album_task = cx.spawn_in(window, async move |_, cx| {
            let songs = cx
                .background_spawn(async move { session.album(&album).await })
                .await
                .map(|loaded| Arc::from(loaded.into_parts().1))
                .map_err(|error| session::describe(&error));
            cx.update(|window, cx| action.finish(songs, window, cx))
                .ok();
        });
    }

    /// Records how many cards fit when the width changed. Runs while the
    /// frame is prepainted, so a change takes effect in the next one.
    fn observe_width(
        this: &gpui_kit::WeakEntity<Self>,
        width: gpui_kit::Pixels,
        window: &Window,
        cx: &mut App,
    ) {
        let columns = columns_for(width / window.rem_size());
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
            Part::RecommendedHeading => heading("Recommended").into_any_element(),
            Part::RecommendedStatus => self.render_recommended_status(cx),
            Part::RecentHeading => heading("Recently played").into_any_element(),
            Part::MostPlayedHeading => heading("Most played").into_any_element(),
            Part::NewAlbumsHeading => heading("New albums").into_any_element(),
            Part::RecentStatus => self.render_recent_status(cx),
            Part::MostPlayedStatus => self.render_most_played_status(cx),
            Part::NewAlbumsStatus => self.render_new_albums_status(cx),
            Part::Song(ix) => self.render_song(*ix, window, cx),
            Part::AlbumRow(shelf, range) => {
                self.render_album_row(*shelf, range.clone(), window, cx)
            }
        }
    }

    fn render_recommended_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.recommended {
            Shelf::Failed(error) => failure(
                error.clone(),
                Button::new("retry-recommended")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.pick_recommended(cx))),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => loading("Loading recommendations", cx),
        }
    }

    fn render_recent_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.recent {
            Shelf::Failed(error) => failure(
                error.clone(),
                Button::new("retry-recent")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.reload_recently_played(cx))),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => loading("Loading recently played songs", cx),
        }
    }

    fn render_most_played_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.most_played {
            Shelf::Failed(error) => failure(
                error.clone(),
                Button::new("retry-most-played")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.reload_most_played(cx))),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => loading("Loading most played albums", cx),
        }
    }

    fn render_new_albums_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.new_albums {
            Shelf::Failed(error) => failure(
                error.clone(),
                Button::new("retry-new-albums")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.reload_new_albums(cx))),
                cx,
            ),
            Shelf::Loading => loading("Loading new albums", cx),
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
        let is_selected = self.selection == Some(Item::Song(id.clone()));
        let focused = self.focus.is_focused(window);
        let title = SharedString::from(song.title().to_owned());
        let details = join_details([song.artist(), song.album()]);
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let cover = song
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, ROW_COVER_PIXELS));
        let marker = self.now_playing.as_ref().and_then(|now| now.state_of(&id));
        let label = PlayState::row_label(&title, marker);
        let muted_foreground = cx.theme().muted_foreground;
        let focus = self.focus.clone();
        let has_album = song.album_id().is_some();
        ListItem::new(ElementId::Name(format!("home-song-{id}").into()))
            .selected(is_selected)
            .secondary_selected(focused && is_selected)
            .flex_none()
            .h_12()
            .px_6()
            .w_full()
            .accessibility_label(label)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .child(CoverArt::new(cover).small())
                    .child(RowText::new(title).details(details))
                    .child(
                        // A lane of its own, so the durations stay in line.
                        h_flex()
                            .w_4()
                            .flex_none()
                            .justify_end()
                            .when_some(marker, |this, state| this.child(PlayStateIcon::new(state))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_sm()
                            .text_color(muted_foreground)
                            .child(duration),
                    ),
            )
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
                let menu = menu
                    .action_context(focus.clone())
                    .menu("Play", Box::new(Play))
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                actions::add_to_playlist_submenu(menu, add_selection_command, window, cx)
                    .when(has_album, |menu| {
                        menu.menu("Show Album", Box::new(ShowInLibrary))
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
        let columns = u16::try_from(self.columns.max(1)).unwrap_or(u16::MAX);
        let albums = self.albums(shelf).get(range).unwrap_or_default();
        let cards = albums
            .iter()
            .map(|album| self.render_album_card(shelf, album, window, cx))
            .collect::<Vec<_>>();
        div()
            .flex_none()
            .px_6()
            .pb_4()
            .grid()
            .grid_cols(columns)
            .gap_4()
            .children(cards)
            .into_any_element()
    }

    fn render_album_card(
        &self,
        shelf: AlbumShelf,
        album: &Album,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = album.id().clone();
        let is_selected = self.selection == Some(Item::Album(shelf, id.clone()));
        let focused = self.focus.is_focused(window);
        let name = SharedString::from(album.name().to_owned());
        let artist = album
            .artist()
            .map(|artist| SharedString::from(artist.to_owned()));
        let cover = album
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, CARD_COVER_PIXELS));
        // Selected, with or without focus, reads apart from hovered.
        let outline = if is_selected && focused {
            cx.theme().primary
        } else if is_selected {
            cx.theme().muted_foreground
        } else {
            cx.theme().transparent
        };
        let hover_outline = if is_selected {
            outline
        } else {
            cx.theme().border
        };
        let focus = self.focus.clone();
        v_flex()
            .id(ElementId::Name(format!("home-album-{shelf:?}-{id}").into()))
            .group(CARD_GROUP)
            .min_w_0()
            .gap_1()
            .role(Role::ListItem)
            .aria_label(name.clone())
            .aria_selected(is_selected)
            .when(is_selected, |this| this.aria_active_descendant())
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(CoverArt::new(cover).fill())
                    // Over the cover, so that showing it moves nothing. Square
                    // like the cover, whose corners a rounded frame does not clip.
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .border_2()
                            .border_color(outline)
                            .group_hover(CARD_GROUP, |this| this.border_color(hover_outline)),
                    ),
            )
            .child(div().text_sm().truncate().child(name))
            .child(
                div()
                    .text_xs()
                    .truncate()
                    .text_color(cx.theme().muted_foreground)
                    .when_some(artist, |this, artist| this.child(artist)),
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
                actions::add_to_playlist_submenu(menu, add_selection_command, window, cx)
                    .menu("Show in Library", Box::new(ShowInLibrary))
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
                    // An element with a scroll handle lists no bounds here;
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

/// What a load of the songs played last found.
struct RecentlyPlayed {
    songs: Vec<Song>,
    /// Whether every album asked about came through.
    complete: bool,
}

/// The songs played last. Those are in the albums played last: an album's last
/// play is that of its latest song, so `RECENT_SONGS` songs lie within as many
/// albums. No method returns the songs themselves. An album that fails to load
/// only leaves out its songs.
async fn fetch_recently_played(client: &client::Client) -> Result<RecentlyPlayed, client::Error> {
    let albums = client.recently_played_albums(RECENT_SONGS as u32).await?;
    let loaded = join_all(albums.iter().map(|album| client.album(album.id()))).await;
    let mut songs = Vec::new();
    let mut failure = None;
    for album in loaded {
        match album {
            Ok(album) => songs.extend(album.into_parts().1),
            Err(error) => failure = Some(error),
        }
    }
    match failure {
        Some(error) if songs.is_empty() => Err(error),
        failure => Ok(RecentlyPlayed {
            songs: most_recent(songs, RECENT_SONGS),
            complete: failure.is_none(),
        }),
    }
}

fn heading(title: &'static str) -> impl IntoElement {
    div()
        .id(title)
        .role(Role::Heading)
        .aria_level(2)
        .aria_label(title)
        .flex_none()
        .px_6()
        .pt_6()
        .pb_2()
        .font_semibold()
        .child(title)
}

fn loading(label: &'static str, cx: &Context<Home>) -> AnyElement {
    h_flex()
        .id(label)
        .role(Role::ProgressIndicator)
        .aria_label(label)
        .flex_none()
        .h_16()
        .justify_center()
        .child(Spinner::new().color(cx.theme().muted_foreground))
        .into_any_element()
}

/// A failed load: why, and a way to try again.
fn failure(reason: SharedString, retry: Button, cx: &Context<Home>) -> AnyElement {
    v_flex()
        .flex_none()
        .px_6()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(reason),
        )
        .child(h_flex().key_context(CONTROL_CONTEXT).child(retry))
        .into_any_element()
}

#[cfg(test)]
mod tests;
