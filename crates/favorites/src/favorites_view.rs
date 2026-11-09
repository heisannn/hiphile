use std::sync::Arc;

use actions::{
    AlbumAction, GoToArtist, Placement, PlaySongs, ShowAlbum, ShowArtist, ToggleFavorite,
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
    AnyElement, App, Context, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollStrategy, SharedString, Styled as _, Subscription, Task,
    UniformListScrollHandle, Window, div, prelude::FluentBuilder as _,
};
use hiphile_core::favorites::{
    Favorite, FavoriteLists, FavoritesLoad, FavoritesNotice, FavoritesSnapshot, FavoritesState,
    Section,
};
use subsonic::{AlbumId, CoverArtId, PlaylistId, Song, SongId, StarTarget};
use ui::list_navigation::{ListNavigationExt as _, Motion, page_rows};
use ui::{ListPaneHeader, LoadFailure, NowPlaying, RefreshFailure, centered, muted_text, row_list};

mod rows;

use rows::{AlbumRow, ArtistRow, RowContext, SongRow};

use crate::{
    AddSelectionToPlaylist, AddToPlaylist, AddToQueue, FAVORITES_CONTEXT, GoToAlbum, NextSection,
    Open, Play, PlayNext, PreviousSection,
};

/// The same as the player's.
const ROW_COVER_PIXELS: u32 = 48;

pub struct Favorites {
    session: session::Session,
    favorites: FavoritesState,
    /// What is on show, which follows `favorites`.
    shown: FavoritesSnapshot,
    section: Section,
    /// What is selected of each kind, by [`Section::ix`].
    selected: [Option<StarTarget>; 3],
    now_playing: Option<NowPlaying>,
    focus: FocusHandle,
    /// The scroll of each kind, by [`Section::ix`].
    scrolls: [UniformListScrollHandle; 3],
    _album_task: Task<()>,
    _changes: Task<()>,
    _notices: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Favorites {
    pub fn new(
        session: session::Session,
        favorites: FavoritesState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            // Focus changes the selection treatment of the rows
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        let changes = gpui_host::observe(favorites.subscribe(), cx, Self::show);
        let notices =
            gpui_host::subscribe_in(favorites.notices(), window, cx, |_, notice, window, cx| {
                if let FavoritesNotice::WriteFailed(message) = notice {
                    window.push_notification((NotificationType::Error, message), cx);
                }
            });
        let mut this = Self {
            session,
            shown: favorites.snapshot(),
            favorites,
            section: Section::Songs,
            selected: [None, None, None],
            now_playing: None,
            focus,
            scrolls: std::array::from_fn(|_| UniformListScrollHandle::new()),
            _album_task: Task::ready(()),
            _changes: changes,
            _notices: notices,
            _subscriptions: subscriptions,
        };
        for section in Section::ALL {
            this.keep_selection(section, 0);
        }
        this
    }

    fn lists(&self) -> &FavoriteLists {
        self.shown.lists()
    }

    /// Selects the row where what was selected was, when it is gone.
    fn show(&mut self, favorites: FavoritesSnapshot, cx: &mut Context<Self>) {
        let was_at = Section::ALL.map(|section| {
            self.selected[section.ix()]
                .as_ref()
                .and_then(|target| self.lists().position(target))
        });
        self.shown = favorites;
        for section in Section::ALL {
            self.keep_selection(section, was_at[section.ix()].unwrap_or_default());
        }
        cx.notify();
    }

    /// Selects the row at `fallback_ix`, or the last, when nothing is selected
    /// of `section` or what was is gone.
    fn keep_selection(&mut self, section: Section, fallback_ix: usize) {
        let lists = self.shown.lists();
        let selected = &self.selected[section.ix()];
        if selected
            .as_ref()
            .is_some_and(|target| lists.contains(target))
        {
            return;
        }
        let last = lists.len(section).saturating_sub(1);
        self.selected[section.ix()] = lists
            .favorite_at(section, fallback_ix.min(last))
            .map(|favorite| favorite.target());
    }

    fn selected_position(&self) -> Option<usize> {
        self.lists()
            .position(self.selected[self.section.ix()].as_ref()?)
    }

    fn selected_favorite(&self) -> Option<Favorite> {
        self.lists()
            .favorite_at(self.section, self.selected_position()?)
    }

    fn select(&mut self, target: StarTarget, cx: &mut Context<Self>) {
        let section = Section::of(&target);
        self.selected[section.ix()] = Some(target);
        cx.notify();
    }

    fn move_selection(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let len = self.lists().len(self.section);
        let scroll = &self.scrolls[self.section.ix()];
        let page = page_rows(scroll, len);
        let Some(ix) = motion.apply(self.selected_position(), len, page) else {
            return;
        };
        scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        if let Some(favorite) = self.lists().favorite_at(self.section, ix) {
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
        let songs: Arc<[Song]> = self.lists().songs.iter().cloned().collect();
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
        self._album_task = action.start(album, &self.session, window, cx);
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

    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = NowPlaying::new(song, playing);
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self
            .shown
            .is_loaded()
            .then(|| self.lists().len(self.section));
        let is_updating = count.is_some() && self.shown.is_reloading();
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
        match self.shown.load() {
            FavoritesLoad::Loading => return centered(Spinner::new()).into_any_element(),
            FavoritesLoad::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load favorites",
                    SharedString::from(error.clone()),
                    Button::new("retry-favorites")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.focus.focus(window, cx);
                            this.favorites.refresh();
                        })),
                )
                .into_any_element();
            }
            FavoritesLoad::Loaded => {}
        }
        if self.lists().len(self.section) == 0 {
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
            cover_art.map(|cover_art| session.cover_art_url(cover_art, ROW_COVER_PIXELS).into())
        };
        let scroll = &self.scrolls[self.section.ix()];
        match self.section {
            Section::Songs => {
                let songs = self.lists().songs.clone();
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
                let albums = self.lists().albums.clone();
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
                let artists = self.lists().artists.clone();
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
            .when_some(self.shown.refresh_error(), |this, error| {
                this.child(RefreshFailure::new(
                    "Couldn't refresh the favorites",
                    SharedString::from(error.to_owned()),
                    Button::new("retry-refresh-favorites")
                        .label("Retry")
                        .small()
                        .on_click(cx.listener(|this, _, _, _| this.favorites.refresh())),
                ))
            })
            .child(self.render_list(window, cx))
    }
}

#[cfg(test)]
mod tests;
