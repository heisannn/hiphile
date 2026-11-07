use std::ops::Range;
use std::sync::Arc;

use actions::{
    AlbumAction, EnqueueSongs, Favorite, GoToArtist, Placement, PlaySongs, ShowAlbum, ShowArtist,
    ToggleFavorite,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    list::ListItem,
    menu::{ContextMenuExt as _, DropdownMenu as _},
    scroll::Scrollbar,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, InteractiveElement as _,
    IntoElement, ListOffset, MouseButton, ParentElement as _, Pixels, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, list, prelude::FluentBuilder as _,
    px,
};
use subsonic::{Album, AlbumId, Artist, ArtistId, Song, SongId, StarTarget};
use ui::card_grid::{GridMotion, card_row, card_row_height, columns_for, media_card, step};
use ui::list_navigation::{
    CONTROL_CONTEXT, SelectFirst, SelectLast, SelectNext, SelectPageDown, SelectPageUp,
    SelectPrevious,
};
use ui::{
    ArtistLine, CoverArt, DetailHeader, LoadFailure, Shelf, centered, format_album_count,
    format_duration, format_song_count, join_details, muted_text, section_failure, section_heading,
    section_loading, skippable_song, song_artist_link, song_row_label, song_with_cover,
};

use super::{Artists, ShownArtist, TOP_SONGS_COLLAPSED, add_selection_command};
use crate::{
    AddSelectionToPlaylist, AddToPlaylist, AddToQueue, Back, DETAIL_CONTEXT, GoToAlbum, Open, Play,
    PlayNext, SelectLeft, SelectRight,
};

/// The same as the albums' screen's, so that opening an album needs no second image.
const CARD_COVER_PIXELS: u32 = 256;
/// The same as the player's.
const ROW_COVER_PIXELS: u32 = 48;

/// By identity, so that the selection survives a reload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Item {
    Song(SongId),
    Album(AlbumId),
    Artist(ArtistId),
}

/// One item of the detail's list, in order. Only those near the viewport exist:
/// an artist can have hundreds of albums, whose covers should not all load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Header,
    TopSongsHeading,
    TopSongsStatus,
    Song(usize),
    TopSongsToggle,
    AlbumsHeading,
    AlbumsStatus,
    AlbumRow(Range<usize>),
    SimilarHeading,
    SimilarStatus,
    SimilarRow(Range<usize>),
}

impl Part {
    /// Whether this part shows what `other` starts with: rows rearrange when the
    /// number of columns changes.
    fn continues(&self, other: &Part) -> bool {
        match (self, other) {
            (Part::AlbumRow(row), Part::AlbumRow(other))
            | (Part::SimilarRow(row), Part::SimilarRow(other)) => row.contains(&other.start),
            _ => self == other,
        }
    }
}

/// What the detail's list was last told it holds.
#[derive(Default, PartialEq)]
pub(super) struct ListedDetail {
    artist: Option<ArtistId>,
    pub(super) parts: Vec<Part>,
    width: Pixels,
    rem: Pixels,
}

/// Where in `new` the part at `top` of `old` went. One that is gone gives way to
/// the next that stays.
pub(super) fn same_place(old: &[Part], top: usize, new: &[Part]) -> usize {
    old.iter()
        .skip(top)
        .find_map(|part| new.iter().position(|kept| kept.continues(part)))
        .unwrap_or(top.min(new.len().saturating_sub(1)))
}

impl Artists {
    fn top_songs_shown(&self, detail: &ShownArtist) -> usize {
        let len = detail.top_songs.items().len();
        if self.top_songs_expanded {
            len
        } else {
            len.min(TOP_SONGS_COLLAPSED)
        }
    }

    /// Top songs and similar artists with nothing to show go away: a server without
    /// an outside service to ask has nothing to say about them.
    pub(super) fn parts(&self) -> Vec<Part> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let columns = self.columns.max(1);
        let rows_of = |len: usize| {
            (0..len)
                .step_by(columns)
                .map(move |start| start..(start + columns).min(len))
        };
        let mut parts = vec![Part::Header];
        match &detail.top_songs {
            Shelf::Loaded(songs) if songs.is_empty() => {}
            Shelf::Loaded(songs) => {
                parts.push(Part::TopSongsHeading);
                parts.extend((0..self.top_songs_shown(detail)).map(Part::Song));
                if songs.len() > TOP_SONGS_COLLAPSED {
                    parts.push(Part::TopSongsToggle);
                }
            }
            Shelf::Loading | Shelf::Failed(_) => {
                parts.extend([Part::TopSongsHeading, Part::TopSongsStatus]);
            }
        }
        if !detail.is_only_on_songs {
            parts.push(Part::AlbumsHeading);
            match &detail.albums {
                Shelf::Loaded(albums) if !albums.is_empty() => {
                    parts.extend(rows_of(albums.len()).map(Part::AlbumRow));
                }
                _ => parts.push(Part::AlbumsStatus),
            }
        }
        match &detail.similar {
            Shelf::Loaded(artists) if artists.is_empty() => {}
            Shelf::Loaded(artists) => {
                parts.push(Part::SimilarHeading);
                parts.extend(rows_of(artists.len()).map(Part::SimilarRow));
            }
            Shelf::Loading | Shelf::Failed(_) => {
                parts.extend([Part::SimilarHeading, Part::SimilarStatus]);
            }
        }
        parts
    }

    /// What can be selected, row by row from the top.
    fn rows(&self, parts: &[Part]) -> Vec<Vec<Item>> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        parts
            .iter()
            .filter_map(|part| match part {
                Part::Song(ix) => {
                    let song = detail.top_songs.items().get(*ix)?;
                    Some(vec![Item::Song(song.id().clone())])
                }
                Part::AlbumRow(range) => {
                    let albums = detail.albums.items().get(range.clone())?;
                    let item = |album: &Album| Item::Album(album.id().clone());
                    Some(albums.iter().map(item).collect())
                }
                Part::SimilarRow(range) => {
                    let artists = detail.similar.items().get(range.clone())?;
                    let item = |artist: &Artist| Item::Artist(artist.id().clone());
                    Some(artists.iter().map(item).collect())
                }
                _ => None,
            })
            .collect()
    }

    fn part_of(&self, item: &Item, parts: &[Part]) -> Option<usize> {
        let detail = self.detail.as_ref()?;
        parts.iter().position(|part| match (part, item) {
            (Part::Song(ix), Item::Song(id)) => detail
                .top_songs
                .items()
                .get(*ix)
                .is_some_and(|song| song.id() == id),
            (Part::AlbumRow(range), Item::Album(id)) => detail
                .albums
                .items()
                .get(range.clone())
                .is_some_and(|albums| albums.iter().any(|album| album.id() == id)),
            (Part::SimilarRow(range), Item::Artist(id)) => detail
                .similar
                .items()
                .get(range.clone())
                .is_some_and(|artists| artists.iter().any(|artist| artist.id() == id)),
            _ => false,
        })
    }

    /// The selection, while it shows. Show less or a reload can take it off the
    /// screen, and a command must not act on what cannot be seen.
    pub(super) fn selection_on_screen(&self) -> Option<Item> {
        let selection = self.selection.as_ref()?;
        self.part_of(selection, &self.parts())?;
        Some(selection.clone())
    }

    fn is_selection_at_row_start(&self) -> bool {
        let Some(selection) = self.selection_on_screen() else {
            return true;
        };
        let parts = self.parts();
        let rows = self.rows(&parts);
        rows.iter().any(|row| row.first() == Some(&selection))
    }

    pub(super) fn move_selection(&mut self, motion: GridMotion, cx: &mut Context<Self>) {
        let parts = self.parts();
        let rows = self.rows(&parts);
        let Some(item) = step(&rows, self.selection.as_ref(), motion) else {
            return;
        };
        if let Some(ix) = self.part_of(&item, &parts) {
            // Going up to a section brings its heading into view too
            let heading_above = ix > 0
                && matches!(
                    motion,
                    GridMotion::Up | GridMotion::PageUp | GridMotion::First
                )
                && matches!(
                    parts[ix - 1],
                    Part::TopSongsHeading | Part::AlbumsHeading | Part::SimilarHeading
                );
            self.reveal_part(if heading_above { ix - 1 } else { ix });
        }
        self.selection = Some(item);
        cx.notify();
    }

    /// One not yet laid out has no height to go by, so it goes to the top of the
    /// viewport.
    fn reveal_part(&self, ix: usize) {
        if self.detail_list.bounds_for_item(ix).is_some() {
            self.detail_list.scroll_to_reveal_item(ix);
        } else {
            self.detail_list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
        }
    }

    /// Keeps what is at the top of the viewport there. Heights start over: a part
    /// counts as a row of cards, as most of a long list is, until it is laid out,
    /// which sizes the scrollbar right from the start.
    pub(super) fn sync_detail_list(&mut self, window: &Window) {
        let listed = ListedDetail {
            artist: self.detail.as_ref().map(|detail| detail.id.clone()),
            parts: self.parts(),
            width: self.detail_width,
            rem: window.rem_size(),
        };
        if listed == self.listed {
            return;
        }
        let top = self.detail_list.logical_scroll_top();
        let item_ix = same_place(&self.listed.parts, top.item_ix, &listed.parts);
        let unmoved = self.listed.parts.get(top.item_ix) == listed.parts.get(item_ix);
        let row_height = card_row_height(listed.width / listed.rem, self.columns);
        self.detail_list
            .reset_with_uniform_height(listed.parts.len(), listed.rem * row_height);
        self.detail_list.scroll_to(ListOffset {
            item_ix,
            offset_in_item: if unmoved { top.offset_in_item } else { px(0.) },
        });
        self.listed = listed;
    }

    /// Runs while the frame is prepainted, so a change takes effect in the next one.
    fn observe_width(
        this: &gpui_kit::WeakEntity<Self>,
        width: Pixels,
        window: &Window,
        cx: &mut App,
    ) {
        let columns = columns_for(width / window.rem_size());
        let Some(artists) = this.upgrade() else {
            return;
        };
        if artists.read(cx).detail_width == width {
            return;
        }
        cx.defer(move |cx| {
            artists.update(cx, |artists, cx| {
                artists.detail_width = width;
                artists.columns = columns;
                cx.notify();
            });
        });
    }

    fn select_item(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.detail_focus.focus(window, cx);
        self.selection = Some(item);
        cx.notify();
    }

    fn top_song(&self, id: &SongId) -> Option<(usize, &Song)> {
        let songs = self.detail.as_ref()?.top_songs.items();
        let ix = songs.iter().position(|song| song.id() == id)?;
        Some((ix, songs.get(ix)?))
    }

    fn album(&self, id: &AlbumId) -> Option<&Album> {
        let albums = self.detail.as_ref()?.albums.items();
        albums.iter().find(|album| album.id() == id)
    }

    fn play_top_songs_from(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else {
            return;
        };
        let shown = self.top_songs_shown(detail);
        if let Some(songs) = detail.top_songs.items().get(..shown)
            && ix < songs.len()
        {
            let songs: Arc<[Song]> = songs.into();
            window.dispatch_action(Box::new(PlaySongs::new(songs, ix)), cx);
        }
    }

    fn open_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selection_on_screen() {
            Some(Item::Song(id)) => {
                if let Some((ix, _)) = self.top_song(&id) {
                    self.play_top_songs_from(ix, window, cx);
                }
            }
            Some(Item::Album(album)) => {
                window.dispatch_action(Box::new(ShowAlbum::new(album)), cx);
            }
            Some(Item::Artist(artist)) => {
                window.dispatch_action(Box::new(ShowArtist::new(artist)), cx);
            }
            None => {}
        }
    }

    fn play_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.selection_on_screen(), Some(Item::Song(_))) {
            self.open_selection(window, cx);
        } else {
            self.act_on_selection(AlbumAction::Play, window, cx);
        }
    }

    fn act_on_selection(
        &mut self,
        action: AlbumAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.selection_on_screen() {
            Some(Item::Song(id)) => {
                let song = self.top_song(&id).map(|(_, song)| song.clone());
                if let Some(command) = song.and_then(|song| action.command(Arc::from([song]))) {
                    window.dispatch_action(command, cx);
                }
            }
            Some(Item::Album(album)) => self.act_on_album(album, action, window, cx),
            Some(Item::Artist(artist)) => self.act_on_artist(artist, action, window, cx),
            None => {}
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
        self.artist_being_fetched = None;
        cx.notify();
        let session = self.session.clone();
        self._action_task = cx.spawn_in(window, async move |_, cx| {
            let songs = cx
                .background_spawn(async move { session.album(&album).await })
                .await
                .map(|loaded| Arc::from(loaded.into_parts().1))
                .map_err(|error| session::describe(&error));
            cx.update(|window, cx| action.finish(songs, window, cx))
                .ok();
        });
    }

    fn enqueue_selection(
        &mut self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(Item::Song(id)) = self.selection_on_screen() {
            if let Some((_, song)) = self.top_song(&id) {
                let songs = Arc::from([song.clone()]);
                window.dispatch_action(Box::new(EnqueueSongs::new(songs, placement)), cx);
            }
        } else {
            self.act_on_selection(AlbumAction::Enqueue(placement), window, cx);
        }
    }

    /// The selected artist, as the list has them or as the detail does once it
    /// has their name.
    pub(super) fn selected_artist_whole(&self) -> Option<Artist> {
        let selected = self.selected_artist.as_ref()?;
        let listed = self
            .loaded_artists()
            .and_then(|artists| artists.iter().find(|artist| artist.id() == selected));
        match (listed, &self.detail) {
            (Some(artist), _) => Some(artist.clone()),
            (None, Some(detail)) if detail.id == *selected && detail.name.is_some() => {
                Some(shown_as_artist(detail))
            }
            (None, _) => None,
        }
    }

    fn toggle_selection_favorite(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else {
            return;
        };
        let favorite = match self.selection_on_screen() {
            Some(Item::Song(id)) => self
                .top_song(&id)
                .map(|(_, song)| Favorite::Song(song.clone())),
            Some(Item::Album(id)) => self.album(&id).cloned().map(Favorite::Album),
            Some(Item::Artist(id)) => detail
                .similar
                .items()
                .iter()
                .find(|artist| *artist.id() == id)
                .cloned()
                .map(Favorite::Artist),
            None => None,
        };
        if let Some(favorite) = favorite {
            actions::toggle_favorite(favorite, window, cx);
        }
    }

    fn go_to_album(&self, window: &mut Window, cx: &mut Context<Self>) {
        let album = match self.selection_on_screen() {
            Some(Item::Album(album)) => Some(album),
            Some(Item::Song(id)) => self
                .top_song(&id)
                .and_then(|(_, song)| song.album_id().cloned()),
            Some(Item::Artist(_)) | None => None,
        };
        if let Some(album) = album {
            window.dispatch_action(Box::new(ShowAlbum::new(album)), cx);
        }
    }

    fn go_to_artist(&self, window: &mut Window, cx: &mut Context<Self>) {
        let artist = match self.selection_on_screen() {
            Some(Item::Artist(artist)) => Some(artist),
            Some(Item::Album(id)) => self.album(&id).and_then(|a| a.artist_id().cloned()),
            Some(Item::Song(id)) => self
                .top_song(&id)
                .and_then(|(_, song)| song.artist_id().cloned()),
            None => None,
        };
        if let Some(artist) = artist {
            window.dispatch_action(Box::new(ShowArtist::new(artist)), cx);
        }
    }

    pub(super) fn toggle_top_songs(&mut self, cx: &mut Context<Self>) {
        self.top_songs_expanded = !self.top_songs_expanded;
        cx.notify();
    }

    /// The detail is its keyboard target in every state, so focus and the
    /// commands bound above it never land on an element that is gone.
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("artist-detail")
            .key_context(DETAIL_CONTEXT)
            .track_focus(&self.detail_focus)
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                this.move_selection(GridMotion::Down, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| {
                this.move_selection(GridMotion::Up, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLeft, window, cx| {
                if this.is_selection_at_row_start() {
                    this.list_focus.focus(window, cx);
                } else {
                    this.move_selection(GridMotion::Left, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| {
                this.move_selection(GridMotion::Right, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPageUp, _, cx| {
                this.move_selection(GridMotion::PageUp, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPageDown, _, cx| {
                this.move_selection(GridMotion::PageDown, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| {
                this.move_selection(GridMotion::First, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| {
                this.move_selection(GridMotion::Last, cx);
            }))
            .on_action(cx.listener(|this, _: &Back, window, cx| {
                this.list_focus.focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open_selection(window, cx)))
            .on_action(cx.listener(|this, _: &Play, window, cx| this.play_selection(window, cx)))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.enqueue_selection(Placement::Next, window, cx);
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.enqueue_selection(Placement::Last, window, cx);
            }))
            .on_action(
                cx.listener(|this, action: &AddSelectionToPlaylist, window, cx| {
                    let action = AlbumAction::AddToPlaylist(action.playlist.clone());
                    this.act_on_selection(action, window, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.act_on_selection(AlbumAction::ChoosePlaylist, window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoToAlbum, window, cx| this.go_to_album(window, cx)))
            .on_action(
                cx.listener(|this, _: &GoToArtist, window, cx| this.go_to_artist(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFavorite, window, cx| {
                this.toggle_selection_favorite(window, cx)
            }))
            .child(self.render_detail_content(cx))
            .into_any_element()
    }

    fn render_detail_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(detail) = &self.detail else {
            return div().size_full().into_any_element();
        };
        // An artist not on the list has nothing to show until the server names
        // them.
        if detail.name.is_none() {
            return match &detail.albums {
                Shelf::Failed(error) => LoadFailure::new(
                    "Couldn't load this artist",
                    error.clone(),
                    Button::new("retry-artist")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.detail_focus.focus(window, cx);
                            if let Some(artist) = this.selected_artist.clone() {
                                this.load_detail(artist, cx);
                            }
                        })),
                )
                .into_any_element(),
                _ => centered(Spinner::new()).into_any_element(),
            };
        }
        let owner = cx.entity().downgrade();
        let render_item = cx.processor(|this: &mut Self, ix: usize, window, cx| {
            this.render_listed_part(ix, window, cx)
        });
        div()
            .relative()
            .size_full()
            .child(
                v_flex()
                    .on_children_prepainted(move |bounds, window, cx| {
                        if let Some(list) = bounds.first() {
                            Artists::observe_width(&owner, list.size.width, window, cx);
                        }
                    })
                    .id("artist-parts")
                    .role(Role::List)
                    .aria_label("Artist")
                    .size_full()
                    .child(
                        list(self.detail_list.clone(), render_item)
                            .size_full()
                            .pb_4(),
                    ),
            )
            .child(Scrollbar::vertical(&self.detail_list))
            .into_any_element()
    }

    fn render_listed_part(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match (self.listed.parts.get(ix), &self.detail) {
            // A list lays each item out on its own, as wide as its content
            // unless told otherwise.
            (Some(part), Some(detail)) => v_flex()
                .debug_selector(|| format!("artist-part-{ix}"))
                .w_full()
                .child(self.render_part(part, detail, window, cx))
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    fn render_part(
        &self,
        part: &Part,
        detail: &ShownArtist,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match part {
            Part::Header => self.render_header(detail, cx),
            Part::TopSongsHeading => section_heading(if detail.is_only_on_songs {
                "Songs"
            } else {
                "Top songs"
            })
            .into_any_element(),
            Part::TopSongsStatus => self.render_top_songs_status(detail, cx),
            Part::Song(ix) => self.render_top_song(*ix, detail, window, cx),
            Part::TopSongsToggle => self.render_top_songs_toggle(cx),
            Part::AlbumsHeading => section_heading("Albums").into_any_element(),
            Part::AlbumsStatus => self.render_albums_status(detail, cx),
            Part::AlbumRow(range) => {
                let albums = detail.albums.items().get(range.clone()).unwrap_or_default();
                let cards = albums
                    .iter()
                    .map(|album| self.render_album_card(album, window, cx))
                    .collect();
                card_row(self.columns, cards).into_any_element()
            }
            Part::SimilarHeading => section_heading("Similar artists").into_any_element(),
            Part::SimilarStatus => self.render_similar_status(detail, cx),
            Part::SimilarRow(range) => {
                let artists = detail.similar.items();
                let artists = artists.get(range.clone()).unwrap_or_default();
                let cards = artists
                    .iter()
                    .map(|artist| self.render_artist_card(artist, window, cx))
                    .collect();
                card_row(self.columns, cards).into_any_element()
            }
        }
    }

    fn render_header(&self, detail: &ShownArtist, cx: &mut Context<Self>) -> AnyElement {
        let albums = detail.albums.items();
        let cover = detail
            .cover_art
            .as_ref()
            .or_else(|| albums.first().and_then(Album::cover_art))
            .or_else(|| detail.top_songs.items().first().and_then(Song::cover_art))
            .map(|cover_art| self.session.cover_art_url(cover_art, CARD_COVER_PIXELS));
        let songs: Option<u32> = albums.iter().map(Album::song_count).sum();
        let summary = match &detail.albums {
            _ if detail.is_only_on_songs => {
                format_song_count(detail.top_songs.items().len()).into()
            }
            Shelf::Loaded(_) => join_details([
                Some(format_album_count(albums.len())),
                songs.map(|songs| format_song_count(songs as usize)),
            ]),
            Shelf::Loading | Shelf::Failed(_) => SharedString::default(),
        };
        let name = detail.name.clone().unwrap_or_default();
        let has_songs = !albums.is_empty() || detail.is_only_on_songs;
        DetailHeader::new(cover, name, summary)
            .artist_picture()
            .action(
                Button::new("play-artist")
                    .icon(IconName::Play)
                    .label("Play")
                    .disabled(!has_songs)
                    .loading(self.artist_being_fetched.as_ref() == Some(&detail.id))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.act_on_selected_artist(AlbumAction::Play, window, cx);
                    })),
            )
            .action({
                let artist = self
                    .selected_artist_whole()
                    .filter(|artist| *artist.id() == detail.id);
                let is_nameless = artist.is_none();
                let artist = artist.unwrap_or_else(|| shown_as_artist(detail));
                ui::favorite_button("favorite-artist", Favorite::Artist(artist), cx)
                    .when(is_nameless, |button| button.disabled(true))
            })
            .action(self.render_artist_menu(has_songs))
            .into_any_element()
    }

    /// The list's commands act on the artist on show, so the menu dispatches them
    /// there, whichever part of the screen has focus.
    fn render_artist_menu(&self, has_songs: bool) -> impl IntoElement {
        let list_focus = self.list_focus.clone();
        Button::new("artist-menu")
            .ghost()
            .icon(IconName::Ellipsis)
            .accessibility_label("More")
            .disabled(!has_songs)
            .dropdown_menu(move |menu, window, cx| {
                let menu = menu
                    .action_context(list_focus.clone())
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                actions::add_to_playlist_submenu(menu, add_selection_command, window, cx)
            })
    }

    fn render_top_songs_status(&self, detail: &ShownArtist, cx: &mut Context<Self>) -> AnyElement {
        match &detail.top_songs {
            Shelf::Failed(error) => section_failure(
                error.clone(),
                Button::new("retry-top-songs")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(artist) = this.selected_artist.clone() {
                            this.load_top_songs(artist, cx);
                        }
                    })),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => section_loading("Loading top songs", cx),
        }
    }

    fn render_similar_status(&self, detail: &ShownArtist, cx: &mut Context<Self>) -> AnyElement {
        match &detail.similar {
            Shelf::Failed(error) => section_failure(
                error.clone(),
                Button::new("retry-similar-artists")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(artist) = this.selected_artist.clone() {
                            this.load_similar(artist, cx);
                        }
                    })),
                cx,
            ),
            Shelf::Loading | Shelf::Loaded(_) => section_loading("Loading similar artists", cx),
        }
    }

    fn render_albums_status(&self, detail: &ShownArtist, cx: &mut Context<Self>) -> AnyElement {
        match &detail.albums {
            Shelf::Failed(error) => section_failure(
                error.clone(),
                Button::new("retry-artist-albums")
                    .label("Retry")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(artist) = this.selected_artist.clone() {
                            this.load_albums(artist, cx);
                        }
                    })),
                cx,
            ),
            Shelf::Loading => section_loading("Loading albums", cx),
            Shelf::Loaded(_) => div()
                .flex_none()
                .px_6()
                .child(muted_text("This artist has no albums.", cx))
                .into_any_element(),
        }
    }

    fn render_top_songs_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.top_songs_expanded {
            "Show less"
        } else {
            "See more"
        };
        // A button brings its own padding, so it starts closer to the edge for
        // its label to line up with the rows above.
        h_flex()
            .flex_none()
            .px_4()
            .pt_1()
            .key_context(CONTROL_CONTEXT)
            .child(
                Button::new("top-songs-toggle")
                    .ghost()
                    .small()
                    .text_color(cx.theme().muted_foreground)
                    .label(label)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_top_songs(cx))),
            )
            .into_any_element()
    }

    fn render_top_song(
        &self,
        ix: usize,
        detail: &ShownArtist,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(song) = detail.top_songs.items().get(ix) else {
            return div().into_any_element();
        };
        let id = song.id().clone();
        let target = StarTarget::Song(id.clone());
        let is_selected = self.selection == Some(Item::Song(id.clone()));
        let is_focused = self.detail_focus.is_focused(window);
        let title = SharedString::from(song.title().to_owned());
        // The artist on show goes without saying
        let artist = song_artist_link(song).filter(|_| song.artist_id() != Some(&detail.id));
        let details = ArtistLine::of_song_by(artist, song);
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let cover = song
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, ROW_COVER_PIXELS));
        let state = self.now_playing.as_ref().and_then(|now| now.state_of(&id));
        let label = song_row_label(song, &title, state, cx);
        let focus = self.detail_focus.clone();
        let has_album = song.album_id().is_some();
        let has_artist = song.artist_id().is_some();
        let row_song = song.clone();
        ListItem::new(ElementId::Name(format!("top-song-{id}").into()))
            .selected(is_selected)
            .secondary_selected(is_focused && is_selected)
            .flex_none()
            .h_12()
            .px_6()
            .w_full()
            .accessibility_label(label)
            .child(skippable_song(
                &row_song,
                song_with_cover(cover, title, details, state, duration.into(), cx),
                cx,
            ))
            .on_click(cx.listener({
                let id = id.clone();
                move |this, event: &ClickEvent, window, cx| {
                    this.select_item(Item::Song(id.clone()), window, cx);
                    if event.click_count() >= 2 {
                        this.play_top_songs_from(ix, window, cx);
                    }
                }
            }))
            // The menu acts on the selection, so the row under the pointer
            // becomes it first.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.select_item(Item::Song(id.clone()), window, cx);
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
                        menu.menu("Go to Album", Box::new(GoToAlbum))
                    })
                    .when(has_artist, |menu| {
                        menu.menu("Go to Artist", Box::new(GoToArtist))
                    })
            })
            .into_any_element()
    }

    fn render_album_card(
        &self,
        album: &Album,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = album.id().clone();
        let target = StarTarget::Album(id.clone());
        let item = Item::Album(id.clone());
        let is_selected = self.selection.as_ref() == Some(&item);
        let cover = album
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, CARD_COVER_PIXELS));
        let year = album
            .year()
            .map(|year| year.to_string())
            .unwrap_or_default();
        let details = div()
            .text_xs()
            .truncate()
            .text_color(cx.theme().muted_foreground)
            .child(year);
        let focus = self.detail_focus.clone();
        media_card(
            ElementId::Name(format!("artist-album-{id}").into()),
            CoverArt::new(cover),
            album.name().to_owned().into(),
            details,
            is_selected,
            self.detail_focus.is_focused(window),
            cx,
        )
        .on_click(cx.listener({
            let item = item.clone();
            move |this, _, window, cx| {
                this.select_item(item.clone(), window, cx);
                this.open_selection(window, cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _, window, cx| {
                this.select_item(item.clone(), window, cx);
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
                .menu("Show in Albums", Box::new(GoToAlbum))
        })
        .into_any_element()
    }

    fn render_artist_card(
        &self,
        artist: &Artist,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = artist.id().clone();
        let target = StarTarget::Artist(id.clone());
        let item = Item::Artist(id.clone());
        let is_selected = self.selection.as_ref() == Some(&item);
        let cover = artist
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, CARD_COVER_PIXELS));
        // Zero says nothing of an artist the server knows for their songs
        let albums = artist
            .album_count()
            .filter(|count| *count > 0)
            .map(|count| format_album_count(count as usize))
            .unwrap_or_default();
        let details = div()
            .text_xs()
            .truncate()
            .text_color(cx.theme().muted_foreground)
            .child(albums);
        let focus = self.detail_focus.clone();
        media_card(
            ElementId::Name(format!("similar-artist-{id}").into()),
            CoverArt::artist(cover),
            artist.name().to_owned().into(),
            details,
            is_selected,
            self.detail_focus.is_focused(window),
            cx,
        )
        .on_click(cx.listener({
            let item = item.clone();
            move |this, _, window, cx| {
                this.select_item(item.clone(), window, cx);
                this.open_selection(window, cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _, window, cx| {
                this.select_item(item.clone(), window, cx);
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
                .menu("Go to Artist", Box::new(GoToArtist))
        })
        .into_any_element()
    }
}

fn shown_as_artist(detail: &ShownArtist) -> Artist {
    let albums = match &detail.albums {
        Shelf::Loaded(albums) => u32::try_from(albums.len()).ok(),
        Shelf::Loading | Shelf::Failed(_) => None,
    };
    Artist::new(
        detail.id.clone(),
        detail.name.clone().unwrap_or_default().to_string(),
        detail.cover_art.clone(),
        albums,
    )
}
