use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use actions::{AlbumAction, EnqueueSongs, GoToArtist, Placement, PlaySongs, ShowAlbum, ShowArtist};
use futures::future::join_all;
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
    AnyElement, AnyWindowHandle, App, AppContext as _, ClickEvent, Context, ElementId, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, Role,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, prelude::FluentBuilder as _,
};
use hiphile_core::favorites::Favorite;
use hiphile_core::home::{
    AlbumShelf, Item, Recommendations, fetch_recommended, most_recent, today,
};
use subsonic::{Album, AlbumId, PlaylistId, Song, SongId, StarTarget};
use ui::card_grid::{GridMotion as Motion, card_row, columns_for, media_card, step};
use ui::list_navigation::{
    SelectFirst, SelectLast, SelectNext, SelectPageDown, SelectPageUp, SelectPrevious,
};
use ui::{
    ArtistLine, CoverArt, NowPlaying, Shelf, format_duration, section_failure, section_heading,
    section_heading_with_action, section_loading, skippable_song, song_row_label, song_with_cover,
};

use crate::{
    AddSelectionToPlaylist, AddToQueue, ChoosePlaylist, HOME_CONTEXT, Open, Play, PlayNext,
    SelectLeft, SelectRight, ShowInLibrary,
};

const RECENT_SONGS: usize = 8;
/// How many to fetch, for the widest window.
const MOST_PLAYED: u32 = 12;
const NEW_ALBUM_ROWS: usize = 2;
/// How many to fetch, for the widest window.
const NEW_ALBUMS: u32 = 24;
/// The same as the library's album cover, so that opening the album needs no
/// second image.
const CARD_COVER_PIXELS: u32 = 256;
/// The same as the player's.
const ROW_COVER_PIXELS: u32 = 48;
/// A pick takes the server's outside service several seconds per album, and
/// windows can come to the front often.
const RECOMMENDED_RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// The lists come from the server, which alone knows every play. An item stays
/// on show while a reload that replaces it is on its way or fails.
///
/// The albums to try stay the same all day, unless asked to reload, so they are
/// kept in the catalog and picked again on the first occasion of another day:
/// opening, coming back to the screen or the window, a play or a library sync.
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
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            // Focus changes the selection treatment
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
                        // The albums can be picked again
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
        self.start_pick(None, cx);
    }

    /// Picks new albums now, whichever day the ones on show are from. A pick
    /// that leaves them there says why in `window`, as the shelf looks the same.
    fn reload_recommended(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.start_pick(Some(window.window_handle()), cx);
    }

    fn start_pick(&mut self, report_to: Option<AnyWindowHandle>, cx: &mut Context<Self>) {
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
            let kept = this
                .update(cx, |this, cx| {
                    let result = result.map_err(|error| session::describe(&error).into());
                    this.finish_recommended(result, day, cx)
                })
                .ok()
                .flatten();
            if let (Some(window), Some(message)) = (report_to, kept) {
                cx.update_window(window, |_, window, cx| {
                    window.push_notification((NotificationType::Error, message), cx);
                })
                .ok();
            }
        });
        cx.notify();
    }

    /// Takes in a pick made for `day`. Only a complete one settles the day; the
    /// next occasion tries again after one that went wrong. Albums on show stay
    /// there unless a complete pick found others. Returns why, when something
    /// went wrong and they did.
    fn finish_recommended(
        &mut self,
        result: Result<Recommendations, SharedString>,
        day: String,
        cx: &mut Context<Self>,
    ) -> Option<SharedString> {
        self.recommending = false;
        let kept = (!self.recommended.items().is_empty())
            .then(|| match &result {
                Ok(found) if found.complete => None,
                Ok(_) => Some("Couldn't get new recommendations. Try again later.".into()),
                Err(error) => Some(format!("Couldn't get new recommendations. {error}").into()),
            })
            .flatten();
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
        kept
    }

    fn keep_recommended(&self, day: &str, albums: &[Album], cx: &mut Context<Self>) {
        let Some(catalog) = self.session.catalog().cloned() else {
            return;
        };
        let user = self.session.username().to_owned();
        let (day, albums) = (day.to_owned(), albums.to_vec());
        // Failing to keep them only means another pick next time
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

    /// Songs on show stay there when the load missed some of the albums, as it may
    /// have missed the latest.
    fn finish_recently_played(&mut self, result: Result<RecentlyPlayed, SharedString>) {
        match result {
            Ok(found) if !found.complete && matches!(self.recent, Shelf::Loaded(_)) => {}
            result => self.recent.finish(result.map(|found| found.songs)),
        }
    }

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

    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = NowPlaying::new(song, playing);
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    /// A shelf other than the new albums with nothing to show goes away, as a server
    /// with no plays or albums yet has nothing to say about it.
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

    /// Runs while the frame is prepainted, so a change takes effect in the next one.
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

    /// The heading, with a way to reload once there are albums to replace:
    /// until then the status below it shows the wait or offers to retry.
    fn render_recommended_heading(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.recommended.items().is_empty() {
            return section_heading("Recommended").into_any_element();
        }
        section_heading_with_action(
            "Recommended",
            Button::new("reload-recommended")
                .ghost()
                .small()
                .icon(IconName::RefreshCw)
                .accessibility_label("Reload recommendations")
                .loading(self.recommending)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.reload_recommended(window, cx);
                })),
        )
        .into_any_element()
    }

    fn render_recommended_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.recommended {
            Shelf::Failed(error) => section_failure(
                error.clone(),
                Button::new("retry-recommended")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.pick_recommended(cx))),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => section_loading("Loading recommendations", cx),
        }
    }

    fn render_recent_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.recent {
            Shelf::Failed(error) => section_failure(
                error.clone(),
                Button::new("retry-recent")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.reload_recently_played(cx))),
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
                error.clone(),
                Button::new("retry-most-played")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.reload_most_played(cx))),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => section_loading("Loading most played albums", cx),
        }
    }

    fn render_new_albums_status(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.new_albums {
            Shelf::Failed(error) => section_failure(
                error.clone(),
                Button::new("retry-new-albums")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.reload_new_albums(cx))),
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

#[cfg(test)]
mod tests;
