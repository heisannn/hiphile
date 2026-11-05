use std::collections::VecDeque;
use std::mem;
use std::sync::Arc;

use actions::{
    AlbumAction, Favorite, GoToArtist, Placement, PlaySongs, ShowAlbum, ShowArtist, ToggleFavorite,
};
use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    button::Button,
    notification::NotificationType,
    spinner::Spinner,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollStrategy, SharedString,
    Styled as _, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _,
};
use subsonic::{AlbumId, CoverArtId, PlaylistId, Song, SongId, StarTarget, Starred};
use ui::list_navigation::{ListNavigationExt as _, Motion, page_rows};
use ui::{ListPaneHeader, LoadFailure, NowPlaying, RefreshFailure, centered, muted_text, row_list};

mod rows;

use rows::{AlbumRow, ArtistRow, RowContext, SongRow};

use crate::lists::{FavoriteLists, Section};
use crate::{
    AddSelectionToPlaylist, AddToPlaylist, AddToQueue, FAVORITES_CONTEXT, GoToAlbum, NextSection,
    Open, Play, PlayNext, PreviousSection,
};

/// Pixel size requested for a row's cover, the same as the player's.
const ROW_COVER_PIXELS: u32 = 96;

/// The favorites' screen: one kind of favorite at a time. Owns the favorites,
/// the selection of each kind, and the writes on their way to the server, and
/// tells every other feature what the favorites are.
pub struct Favorites {
    session: session::Session,
    state: LoadState,
    lists: FavoriteLists,
    is_reloading: bool,
    /// Why reading the favorites on show again failed.
    refresh_error: Option<SharedString>,
    section: Section,
    /// What is selected of each kind, by [`Section::ix`].
    selected: [Option<StarTarget>; 3],
    now_playing: Option<NowPlaying>,
    writes: VecDeque<Write>,
    is_sending: bool,
    /// Counts the changes made here, so that what the server said before the
    /// latest of them, which it may not reflect, is not shown over it.
    edits: u64,
    /// Whether to read the favorites once the writes have gone, as a read that
    /// overlaps one cannot be trusted to show it.
    reload_after_writes: bool,
    focus: FocusHandle,
    /// The scroll of each kind, by [`Section::ix`].
    scrolls: [UniformListScrollHandle; 3],
    _load_task: Task<()>,
    _write_task: Task<()>,
    _album_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

/// What a [`Favorites`] tells the app shell.
#[derive(Clone, Debug)]
pub enum FavoritesEvent {
    /// The favorites could not be read and nothing else has them, with why.
    LoadFailed(SharedString),
}

impl EventEmitter<FavoritesEvent> for Favorites {}

enum LoadState {
    Loading,
    Loaded,
    Failed(SharedString),
}

/// A change to the favorites on its way to the server, already on show.
struct Write {
    target: StarTarget,
    is_favorite: bool,
    undo: Undo,
}

/// What brings back how the favorites showed before a change.
enum Undo {
    Remove,
    Restore { favorite: Favorite, ix: usize },
}

impl Write {
    fn failure(&self) -> &'static str {
        if self.is_favorite {
            "Couldn't add to favorites."
        } else {
            "Couldn't remove from favorites."
        }
    }
}

impl Favorites {
    /// Starts loading at once, whether or not the screen ever shows: the menus
    /// of every other screen say which way their command goes from this.
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            // Focus changes the selection treatment of the rows.
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        // Those of whoever was signed in before.
        actions::set_known_favorites(None, cx);
        let mut this = Self {
            session,
            state: LoadState::Loading,
            lists: FavoriteLists::default(),
            is_reloading: false,
            refresh_error: None,
            section: Section::Songs,
            selected: [None, None, None],
            now_playing: None,
            writes: VecDeque::new(),
            is_sending: false,
            edits: 0,
            reload_after_writes: false,
            focus,
            scrolls: std::array::from_fn(|_| UniformListScrollHandle::new()),
            _load_task: Task::ready(()),
            _write_task: Task::ready(()),
            _album_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.load(cx);
        this
    }

    /// Reads the favorites again: another device may have changed them.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    /// Reads the favorites again if they never arrived, which every screen's
    /// menus are waiting on.
    pub fn reload_if_failed(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, LoadState::Failed(_)) {
            self.load(cx);
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.is_writing() {
            self.reload_after_writes = true;
            return;
        }
        self.is_reloading = true;
        self.refresh_error = None;
        if !matches!(self.state, LoadState::Loaded) {
            self.state = LoadState::Loading;
        }
        let client = self.session.client().clone();
        let edits = self.edits;
        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.starred().await })
                .await
                .map_err(|error| SharedString::from(session::describe(&error)));
            this.update(cx, |this, cx| this.finish_loading(result, edits, cx))
                .ok();
        });
        cx.notify();
    }

    fn finish_loading(
        &mut self,
        result: Result<Starred, SharedString>,
        edits: u64,
        cx: &mut Context<Self>,
    ) {
        self.is_reloading = false;
        if edits != self.edits {
            self.load(cx);
            return;
        }
        match result {
            Ok(starred) => {
                self.state = LoadState::Loaded;
                self.lists = FavoriteLists::new(starred);
                for section in Section::ALL {
                    self.keep_selection(section, 0);
                }
                self.tell_favorites(cx);
            }
            // The favorites on show stay there, with word of what went wrong.
            Err(error) if matches!(self.state, LoadState::Loaded) => {
                self.refresh_error = Some(error);
            }
            Err(error) => {
                self.state = LoadState::Failed(error.clone());
                cx.emit(FavoritesEvent::LoadFailed(error));
            }
        }
        cx.notify();
    }

    fn tell_favorites(&self, cx: &mut App) {
        actions::set_known_favorites(Some(self.lists.targets()), cx);
    }

    fn is_writing(&self) -> bool {
        self.is_sending || !self.writes.is_empty()
    }

    /// Adds `favorite` to the favorites or takes it out, on show at once and on
    /// the server as soon as what came before it has gone.
    pub fn set_favorite(
        &mut self,
        favorite: Favorite,
        is_favorite: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = favorite.target();
        if !matches!(self.state, LoadState::Loaded) || self.lists.contains(&target) == is_favorite {
            return;
        }
        let section = Section::of(&target);
        let undo = if is_favorite {
            self.lists.add(favorite, 0);
            self.keep_selection(section, 0);
            Undo::Remove
        } else {
            let Some((favorite, ix)) = self.lists.remove(&target) else {
                return;
            };
            self.keep_selection(section, ix);
            Undo::Restore { favorite, ix }
        };
        self.edits += 1;
        self.writes.push_back(Write {
            target,
            is_favorite,
            undo,
        });
        self.tell_favorites(cx);
        self.send_next_write(window, cx);
        cx.notify();
    }

    /// Shows `target` as it was before a change, wherever later changes have
    /// put it since.
    fn undo(&mut self, target: &StarTarget, undo: Undo) {
        let removed_ix = self.lists.remove(target).map(|(_, ix)| ix);
        let fallback_ix = match undo {
            Undo::Remove => removed_ix.unwrap_or_default(),
            Undo::Restore { favorite, ix } => {
                self.lists.add(favorite, ix);
                ix
            }
        };
        self.keep_selection(Section::of(target), fallback_ix);
    }

    /// Selects the row at `fallback_ix`, or the last, when nothing is selected
    /// of `section` or what was is gone.
    fn keep_selection(&mut self, section: Section, fallback_ix: usize) {
        let selected = &self.selected[section.ix()];
        if selected
            .as_ref()
            .is_some_and(|target| self.lists.contains(target))
        {
            return;
        }
        let last = self.lists.len(section).saturating_sub(1);
        self.selected[section.ix()] = self
            .lists
            .favorite_at(section, fallback_ix.min(last))
            .map(|favorite| favorite.target());
    }

    /// Sends what waits, one at a time and in order: a later change overtaking
    /// an earlier one on the network would leave the older on the server.
    fn send_next_write(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_sending {
            return;
        }
        let Some(write) = self.writes.pop_front() else {
            if mem::take(&mut self.reload_after_writes) {
                self.load(cx);
            }
            return;
        };
        self.is_sending = true;
        let client = self.session.client().clone();
        let target = write.target.clone();
        let is_favorite = write.is_favorite;
        self._write_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if is_favorite {
                        client.star(&target).await
                    } else {
                        client.unstar(&target).await
                    }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.finish_write(write, result, window, cx);
            })
            .ok();
        });
    }

    fn finish_write(
        &mut self,
        write: Write,
        result: Result<(), client::Error>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_sending = false;
        if let Err(error) = result {
            let message = format!("{} {}", write.failure(), session::describe(&error));
            window.push_notification((NotificationType::Error, message), cx);
            // The change may have got there with only the answer lost, which
            // reading the favorites back tells.
            self.reload_after_writes = true;
            let later = self
                .writes
                .iter_mut()
                .find(|later| later.target == write.target);
            if let Some(later) = later {
                // It decides what shows. Should it fail too, what it must go
                // back to is what this one started from.
                later.undo = write.undo;
            } else {
                self.undo(&write.target, write.undo);
                self.tell_favorites(cx);
            }
            cx.notify();
        }
        self.send_next_write(window, cx);
    }

    fn selected_position(&self) -> Option<usize> {
        self.lists
            .position(self.selected[self.section.ix()].as_ref()?)
    }

    fn selected_favorite(&self) -> Option<Favorite> {
        self.lists
            .favorite_at(self.section, self.selected_position()?)
    }

    fn select(&mut self, target: StarTarget, cx: &mut Context<Self>) {
        let section = Section::of(&target);
        self.selected[section.ix()] = Some(target);
        cx.notify();
    }

    fn move_selection(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let len = self.lists.len(self.section);
        let scroll = &self.scrolls[self.section.ix()];
        let page = page_rows(scroll, len);
        let Some(ix) = motion.apply(self.selected_position(), len, page) else {
            return;
        };
        scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        if let Some(favorite) = self.lists.favorite_at(self.section, ix) {
            self.select(favorite.target(), cx);
        }
    }

    fn show_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    fn show_adjacent_section(&mut self, delta: isize, cx: &mut Context<Self>) {
        let next = self
            .section
            .ix()
            .checked_add_signed(delta)
            .and_then(|ix| Section::ALL.get(ix));
        if let Some(section) = next {
            self.show_section(*section, cx);
        }
    }

    fn play_songs_from(&self, song: &SongId, window: &mut Window, cx: &mut Context<Self>) {
        let songs: Arc<[Song]> = self.lists.songs.iter().cloned().collect();
        if let Some(ix) = songs.iter().position(|listed| listed.id() == song) {
            window.dispatch_action(Box::new(PlaySongs::new(songs, ix)), cx);
        }
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_favorite() {
            Some(Favorite::Song(song)) => self.play_songs_from(song.id(), window, cx),
            Some(Favorite::Album(album)) => {
                window.dispatch_action(Box::new(ShowAlbum::new(album.id().clone())), cx);
            }
            Some(Favorite::Artist(artist)) => {
                window.dispatch_action(Box::new(ShowArtist::new(artist.id().clone())), cx);
            }
            None => {}
        }
    }

    fn play_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_favorite() {
            Some(Favorite::Song(song)) => self.play_songs_from(song.id(), window, cx),
            Some(Favorite::Album(album)) => {
                self.act_on_album(album.id().clone(), AlbumAction::Play, window, cx);
            }
            Some(Favorite::Artist(_)) | None => {}
        }
    }

    /// Does `action` to the selected song or album, whose songs it fetches.
    fn act_on_selected(
        &mut self,
        action: AlbumAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.selected_favorite() {
            Some(Favorite::Song(song)) => {
                if let Some(command) = action.command(Arc::from([song])) {
                    window.dispatch_action(command, cx);
                }
            }
            Some(Favorite::Album(album)) => {
                self.act_on_album(album.id().clone(), action, window, cx);
            }
            Some(Favorite::Artist(_)) | None => {}
        }
    }

    /// Does `action` to `album`'s songs, through its saved ones when there are
    /// any. Replaces an action still waiting for its songs: the latest counts.
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

    fn go_to_album(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Favorite::Song(song)) = self.selected_favorite()
            && let Some(album) = song.album_id()
        {
            window.dispatch_action(Box::new(ShowAlbum::new(album.clone())), cx);
        }
    }

    fn go_to_artist(&self, window: &mut Window, cx: &mut Context<Self>) {
        let artist = match self.selected_favorite() {
            Some(Favorite::Song(song)) => song.artist_id().cloned(),
            Some(Favorite::Album(album)) => album.artist_id().cloned(),
            Some(Favorite::Artist(artist)) => Some(artist.id().clone()),
            None => None,
        };
        if let Some(artist) = artist {
            window.dispatch_action(Box::new(ShowArtist::new(artist)), cx);
        }
    }

    fn toggle_selected(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(favorite) = self.selected_favorite() {
            actions::toggle_favorite(favorite, window, cx);
        }
    }

    /// Marks `song` in the list, or clears the mark.
    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = NowPlaying::new(song, playing);
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = matches!(self.state, LoadState::Loaded).then(|| self.lists.len(self.section));
        let is_updating = count.is_some() && self.is_reloading;
        ListPaneHeader::new("Favorites", count)
            .when(is_updating, |this| this.updating("Updating the favorites"))
            .action(
                TabBar::new("favorite-sections")
                    .segmented()
                    .small()
                    .selected_index(self.section.ix())
                    .on_click(cx.listener(|this, ix: &usize, window, cx| {
                        if let Some(section) = Section::ALL.get(*ix) {
                            this.show_section(*section, cx);
                            this.focus.focus(window, cx);
                        }
                    }))
                    .children(Section::ALL.map(|section| Tab::new().label(section.label()))),
            )
            .into_any_element()
    }

    /// The list's focus target, rendered in every state so focus and the
    /// commands bound above it never land on an element that is gone.
    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("favorites")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .key_context(FAVORITES_CONTEXT)
            .on_list_navigation(|this, motion, _, cx| this.move_selection(motion, cx), cx)
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open_selected(window, cx)))
            .on_action(cx.listener(|this, _: &Play, window, cx| this.play_selected(window, cx)))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.act_on_selected(AlbumAction::Enqueue(Placement::Next), window, cx);
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.act_on_selected(AlbumAction::Enqueue(Placement::Last), window, cx);
            }))
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.act_on_selected(AlbumAction::ChoosePlaylist, window, cx);
            }))
            .on_action(
                cx.listener(|this, action: &AddSelectionToPlaylist, window, cx| {
                    let action = AlbumAction::AddToPlaylist(action.playlist.clone());
                    this.act_on_selected(action, window, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &GoToAlbum, window, cx| this.go_to_album(window, cx)))
            .on_action(
                cx.listener(|this, _: &GoToArtist, window, cx| this.go_to_artist(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFavorite, window, cx| {
                this.toggle_selected(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousSection, _, cx| {
                this.show_adjacent_section(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &NextSection, _, cx| {
                this.show_adjacent_section(1, cx);
            }))
            .child(self.render_rows(window, cx))
            .into_any_element()
    }

    fn render_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match &self.state {
            LoadState::Loading => return centered(Spinner::new()).into_any_element(),
            LoadState::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load favorites",
                    error.clone(),
                    Button::new("retry-favorites")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.focus.focus(window, cx);
                            this.load(cx);
                        })),
                )
                .into_any_element();
            }
            LoadState::Loaded => {}
        }
        if self.lists.len(self.section) == 0 {
            let message = match self.section {
                Section::Songs => "No favorite songs. Right-click a song to add it.",
                Section::Albums => "No favorite albums. Right-click an album to add it.",
                Section::Artists => "No favorite artists. Right-click an artist to add them.",
            };
            return centered(muted_text(message, cx)).into_any_element();
        }
        let selected = self.selected[self.section.ix()].clone();
        let session = self.session.clone();
        let is_focused = self.focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let focus = self.focus.clone();
        let context = move |target: StarTarget| RowContext {
            is_selected: selected.as_ref() == Some(&target),
            target,
            is_focused,
            owner: owner.clone(),
            focus: focus.clone(),
        };
        let cover = move |cover_art: Option<&CoverArtId>| {
            cover_art.map(|cover_art| session.cover_art_url(cover_art, ROW_COVER_PIXELS))
        };
        let scroll = &self.scrolls[self.section.ix()];
        match self.section {
            Section::Songs => {
                let songs = self.lists.songs.clone();
                let now_playing = self.now_playing.clone();
                row_list(
                    "favorite-songs",
                    "Favorite songs",
                    songs.len(),
                    scroll,
                    move |ix| {
                        let song = songs.get(ix)?;
                        Some(SongRow {
                            song: song.clone(),
                            cover: cover(song.cover_art()),
                            state: now_playing.as_ref().and_then(|now| now.state_of(song.id())),
                            context: context(StarTarget::Song(song.id().clone())),
                        })
                    },
                )
                .size_full()
                .into_any_element()
            }
            Section::Albums => {
                let albums = self.lists.albums.clone();
                row_list(
                    "favorite-albums",
                    "Favorite albums",
                    albums.len(),
                    scroll,
                    move |ix| {
                        let album = albums.get(ix)?;
                        Some(AlbumRow {
                            album: album.clone(),
                            cover: cover(album.cover_art()),
                            context: context(StarTarget::Album(album.id().clone())),
                        })
                    },
                )
                .size_full()
                .into_any_element()
            }
            Section::Artists => {
                let artists = self.lists.artists.clone();
                row_list(
                    "favorite-artists",
                    "Favorite artists",
                    artists.len(),
                    scroll,
                    move |ix| {
                        let artist = artists.get(ix)?;
                        Some(ArtistRow {
                            artist: artist.clone(),
                            cover: cover(artist.cover_art()),
                            context: context(StarTarget::Artist(artist.id().clone())),
                        })
                    },
                )
                .size_full()
                .into_any_element()
            }
        }
    }
}

/// What a choice in an Add to Playlist menu runs: an album's songs are only
/// fetched once the choice is made.
fn add_selection_command(playlist: Option<&PlaylistId>) -> Box<dyn gpui_kit::Action> {
    Box::new(AddSelectionToPlaylist {
        playlist: playlist.cloned(),
    })
}

impl Focusable for Favorites {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Favorites {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("favorites-screen")
            .size_full()
            .min_h_0()
            .child(self.render_header(cx))
            .when_some(self.refresh_error.clone(), |this, error| {
                this.child(RefreshFailure::new(
                    "Couldn't refresh the favorites",
                    error,
                    Button::new("retry-refresh-favorites")
                        .label("Retry")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                ))
            })
            .child(self.render_list(window, cx))
    }
}

#[cfg(test)]
mod tests;
