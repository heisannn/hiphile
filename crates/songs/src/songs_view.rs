use std::sync::Arc;

use actions::{EnqueueSongs, GoToArtist, Placement, PlaySongs, ShowAlbum, ShowArtist};
use fuzzy::Index;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollStrategy, SharedString,
    Styled as _, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _,
};
use hiphile_core::favorites::Favorite;
use hiphile_core::songs::{Column, Sort, arrange, song_index};
use session::ReadingBook;
use subsonic::{Song, SongId};
use ui::list_navigation::{ListNavigationExt as _, Motion, page_rows};
use ui::{
    ListPaneHeader, LoadFailure, NowPlaying, RefreshFailure, SortOption, centered, filter_input,
    muted_text, no_matches, row_list, sort_menu,
};

mod song_row;

use song_row::SongRow;

use crate::{
    AddToPlaylist, AddToQueue, FocusFilter, GoToAlbum, Open, PlayNext, ROOT_CONTEXT, SONGS_CONTEXT,
};

pub struct Songs {
    session: session::Session,
    /// Whether the screen has shown, which is when it starts loading. Reading
    /// every song is too much to do for a screen that may never show.
    is_activated: bool,
    songs: SongList,
    is_reloading: bool,
    refresh_error: Option<SharedString>,
    index: Arc<Index>,
    /// Positions in the songs that match the filter, in the order they show.
    visible: Arc<Vec<usize>>,
    sort: Sort,
    /// Whether a filter ranks what it finds rather than following `sort`. So it
    /// does until a heading is clicked; the next filter from empty does again.
    by_relevance: bool,
    /// The filter as last arranged, to tell when a new one starts.
    query: SharedString,
    /// Kept by song, so it follows the song as the order changes.
    selected: Option<SongId>,
    /// Whether the user chose the selection. Until then it is simply the first
    /// song on show, which changing the order should not chase down the list.
    has_chosen_selection: bool,
    now_playing: Option<NowPlaying>,
    filter: Entity<InputState>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    _load_task: Task<()>,
    _arrange_task: Task<()>,
    _index_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

enum SongList {
    Loading,
    Loaded(Arc<Vec<Song>>),
    Failed(SharedString),
}

/// Songs just read, and which of them to show as `query` and `sort` had it when
/// the reading started.
struct Arranged {
    songs: Vec<Song>,
    index: Index,
    visible: Vec<usize>,
    query: SharedString,
    sort: Sort,
    by_relevance: bool,
    /// Whether there were dictionaries to read it with.
    had_book: bool,
}

impl Arranged {
    fn new(
        songs: Vec<Song>,
        book: Option<&ReadingBook>,
        query: SharedString,
        sort: Sort,
        by_relevance: bool,
    ) -> Self {
        let index = read_songs(&songs, book);
        let visible = arrange(&songs, &index, &query, sort, by_relevance);
        Self {
            songs,
            index,
            visible,
            query,
            sort,
            by_relevance,
            had_book: book.is_some(),
        }
    }
}

/// `songs` made searchable, as far as they read without a dictionary when there
/// is no `book` to read them with. Takes a while: call it off the UI thread.
fn read_songs(songs: &[Song], book: Option<&ReadingBook>) -> Index {
    match book {
        Some(book) => book.read(|speller| song_index(songs, Some(speller))),
        None => song_index(songs, None),
    }
}

impl Songs {
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter songs"));
        let focus = cx.focus_handle().tab_stop(true);
        let mut subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.filter_changed(cx),
                InputEvent::PressEnter { .. } => this.focus.focus(window, cx),
                _ => {}
            }),
            // Focus changes the selection treatment of the rows
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        if let Some(readings) = session::readings(cx) {
            subscriptions.push(
                cx.subscribe(&readings, |this, _, _: &session::ReadingsChanged, cx| {
                    this.read_again(cx)
                }),
            );
        }
        Self {
            session,
            is_activated: false,
            songs: SongList::Loading,
            is_reloading: false,
            refresh_error: None,
            index: Arc::default(),
            visible: Arc::default(),
            sort: Sort::DEFAULT,
            by_relevance: true,
            query: SharedString::default(),
            selected: None,
            has_chosen_selection: false,
            now_playing: None,
            filter,
            focus,
            scroll: UniformListScrollHandle::new(),
            _load_task: Task::ready(()),
            _arrange_task: Task::ready(()),
            _index_task: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        if !self.is_activated {
            self.is_activated = true;
            self.load(cx);
        }
    }

    /// Does nothing to a screen that has yet to show.
    pub fn songs_synced(&mut self, cx: &mut Context<Self>) {
        if self.is_activated {
            self.load(cx);
        }
    }

    /// Shows the saved songs; with none saved yet, as during the first sync,
    /// asks the server for them.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.is_reloading = true;
        self.refresh_error = None;
        if !matches!(self.songs, SongList::Loaded(_)) {
            self.songs = SongList::Loading;
        }
        let client = self.session.client().clone();
        let catalog = self.session.catalog().cloned();
        let query = self.filter.read(cx).value();
        let sort = self.sort;
        let by_relevance = self.by_relevance;
        let book = self.session.reading_book(cx);
        self._load_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    // A copy that can't be read is the same as none
                    let saved = catalog.and_then(|catalog| catalog.songs().ok());
                    let songs = match saved {
                        Some(songs) if !songs.is_empty() => songs,
                        _ => client
                            .all_songs()
                            .await
                            .map_err(|error| SharedString::from(session::describe(&error)))?,
                    };
                    Ok(Arranged::new(
                        songs,
                        book.as_ref(),
                        query,
                        sort,
                        by_relevance,
                    ))
                })
                .await;
            this.update(cx, |this, cx| this.finish_loading(result, cx))
                .ok();
        });
        cx.notify();
    }

    fn finish_loading(&mut self, result: Result<Arranged, SharedString>, cx: &mut Context<Self>) {
        self.is_reloading = false;
        match result {
            Ok(arranged) => {
                // Positions worked out for the songs going away mean nothing
                // for these: `visible` only ever indexes the songs on show.
                self._arrange_task = Task::ready(());
                self._index_task = Task::ready(());
                self.songs = SongList::Loaded(Arc::new(arranged.songs));
                self.index = Arc::new(arranged.index);
                self.visible = Arc::new(arranged.visible);
                self.select_first_unless_chosen();
                // Dictionaries that came or went during the reading had no songs
                // yet to read again.
                if arranged.had_book != self.session.reading_book(cx).is_some() {
                    self.read_again(cx);
                }
                // The filter or the sort may have changed during the reading
                if arranged.query != self.filter.read(cx).value()
                    || arranged.sort != self.sort
                    || arranged.by_relevance != self.by_relevance
                {
                    self.rearrange(cx);
                }
            }
            // The songs on show stay there, with word of what went wrong
            Err(error) if matches!(self.songs, SongList::Loaded(_)) => {
                self.refresh_error = Some(error);
            }
            Err(error) => self.songs = SongList::Failed(error),
        }
        cx.notify();
    }

    fn loaded_songs(&self) -> Option<&Arc<Vec<Song>>> {
        match &self.songs {
            SongList::Loaded(songs) => Some(songs),
            _ => None,
        }
    }

    fn visible_song(&self, position: usize) -> Option<&Song> {
        self.loaded_songs()?.get(*self.visible.get(position)?)
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        (0..self.visible.len()).find(|position| {
            self.visible_song(*position)
                .is_some_and(|song| song.id() == selected)
        })
    }

    fn selected_song(&self) -> Option<&Song> {
        self.visible_song(self.selected_position()?)
    }

    /// Off the UI thread: a large library takes a moment to sort. A newer change replaces one still
    /// under way.
    fn rearrange(&mut self, cx: &mut Context<Self>) {
        let Some(songs) = self.loaded_songs().cloned() else {
            return;
        };
        let index = self.index.clone();
        let query = self.filter.read(cx).value();
        let sort = self.sort;
        let by_relevance = self.by_relevance;
        self._arrange_task = cx.spawn(async move |this, cx| {
            let visible = cx
                .background_spawn(
                    async move { arrange(&songs, &index, &query, sort, by_relevance) },
                )
                .await;
            this.update(cx, |this, cx| {
                this.visible = Arc::new(visible);
                this.select_first_unless_chosen();
                this.bring_selection_into_view();
                cx.notify();
            })
            .ok();
        });
    }

    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        if self.query.trim().is_empty() {
            self.by_relevance = true;
        }
        self.query = query;
        self.rearrange(cx);
    }

    /// The songs on show stay as they are until that is done.
    fn read_again(&mut self, cx: &mut Context<Self>) {
        let Some(songs) = self.loaded_songs().cloned() else {
            return;
        };
        let book = self.session.reading_book(cx);
        self._index_task = cx.spawn(async move |this, cx| {
            let index = cx
                .background_spawn(async move { read_songs(&songs, book.as_ref()) })
                .await;
            this.update(cx, |this, cx| {
                this.index = Arc::new(index);
                this.rearrange(cx);
            })
            .ok();
        });
    }

    /// Choosing the column already sorted by keeps its direction.
    fn choose_sort(&mut self, column: Column, cx: &mut Context<Self>) {
        if self.is_ranked(cx) || self.sort.column != column {
            self.sort = Sort::chosen(column);
        }
        self.by_relevance = false;
        self.rearrange(cx);
        cx.notify();
    }

    fn choose_direction(&mut self, is_descending: bool, cx: &mut Context<Self>) {
        self.sort.is_descending = is_descending;
        self.by_relevance = false;
        self.rearrange(cx);
        cx.notify();
    }

    fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        // A heading clicked while the rows follow relevance shows as not sorted,
        // so it starts from the top rather than turning its column around.
        if self.is_ranked(cx) {
            self.sort.column = column;
            self.sort.is_descending = false;
        } else {
            self.sort = self.sort.clicked(column);
        }
        self.by_relevance = false;
        self.rearrange(cx);
        cx.notify();
    }

    fn is_ranked(&self, cx: &App) -> bool {
        self.by_relevance && !self.filter.read(cx).value().trim().is_empty()
    }

    fn bring_selection_into_view(&self) {
        let position = self.selected_position().unwrap_or_default();
        self.scroll
            .scroll_to_item(position, ScrollStrategy::Nearest);
    }

    fn select_first_unless_chosen(&mut self) {
        if !self.has_chosen_selection {
            self.selected = self.visible_song(0).map(|song| song.id().clone());
        }
    }

    fn select_song(&mut self, song: SongId, cx: &mut Context<Self>) {
        self.selected = Some(song);
        self.has_chosen_selection = true;
        cx.notify();
    }

    fn move_selection(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let len = self.visible.len();
        let page = page_rows(&self.scroll, len);
        let Some(position) = motion.apply(self.selected_position(), len, page) else {
            return;
        };
        if let Some(song) = self.visible_song(position).map(|song| song.id().clone()) {
            self.scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
            self.select_song(song, cx);
        }
    }

    fn play_selected(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(start_ix), Some(songs)) = (self.selected_position(), self.loaded_songs()) else {
            return;
        };
        let queue: Arc<[Song]> = self.visible.iter().map(|ix| songs[*ix].clone()).collect();
        window.dispatch_action(Box::new(PlaySongs::new(queue, start_ix)), cx);
    }

    fn enqueue_selected(&self, placement: Placement, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(song) = self.selected_song() {
            let songs = Arc::from([song.clone()]);
            window.dispatch_action(Box::new(EnqueueSongs::new(songs, placement)), cx);
        }
    }

    fn choose_playlist_for_selected(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(song) = self.selected_song() {
            let songs = Arc::from([song.clone()]);
            window.dispatch_action(Box::new(actions::ChoosePlaylist::new(songs)), cx);
        }
    }

    fn toggle_selected_favorite(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(song) = self.selected_song() {
            actions::toggle_favorite(Favorite::Song(song.clone()), window, cx);
        }
    }

    fn go_to_album(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(album) = self.selected_song().and_then(Song::album_id) {
            window.dispatch_action(Box::new(ShowAlbum::new(album.clone())), cx);
        }
    }

    fn go_to_artist(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(artist) = self.selected_song().and_then(Song::artist_id) {
            window.dispatch_action(Box::new(ShowArtist::new(artist.clone())), cx);
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
        let count = self.loaded_songs().map(|songs| songs.len());
        let is_updating = count.is_some() && self.is_reloading;
        ListPaneHeader::new("Songs", count)
            .when(is_updating, |this| this.updating("Updating the songs"))
            .action(
                h_flex()
                    .gap_2()
                    .child(self.render_sort_menu(cx))
                    .child(div().w_56().child(filter_input(&self.filter))),
            )
            .into_any_element()
    }

    fn render_sort_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_ranked = self.is_ranked(cx);
        let option = |label: &'static str, column: Column| {
            let owner = cx.entity().downgrade();
            let is_checked = self.sort.column == column && !is_ranked;
            Some(SortOption::new(label, is_checked, move |_, cx| {
                owner
                    .update(cx, |this, cx| this.choose_sort(column, cx))
                    .ok();
            }))
        };
        let direction = |label: &'static str, is_descending: bool| {
            let owner = cx.entity().downgrade();
            let is_checked = self.sort.is_descending == is_descending && !is_ranked;
            Some(SortOption::new(label, is_checked, move |_, cx| {
                owner
                    .update(cx, |this, cx| this.choose_direction(is_descending, cx))
                    .ok();
            }))
        };
        sort_menu(
            "sort-songs",
            self.sort.is_descending && !is_ranked,
            vec![
                option("Title", Column::Title),
                option("Artist", Column::Artist),
                option("Album", Column::Album),
                option("Time", Column::Time),
                None,
                option("Recently Added", Column::Added),
                option("Release Date", Column::Released),
                None,
                direction("Ascending", false),
                direction("Descending", true),
            ],
        )
    }

    fn render_heading(
        &self,
        id: &'static str,
        label: &'static str,
        column: Column,
        cx: &mut Context<Self>,
    ) -> Button {
        let sorted =
            (self.sort.column == column && !self.is_ranked(cx)).then_some(self.sort.is_descending);
        let (icon, description) = match sorted {
            Some(false) => (
                Some(IconName::ArrowUp),
                format!("{label}, sorted from the start"),
            ),
            Some(true) => (
                Some(IconName::ArrowDown),
                format!("{label}, sorted from the end"),
            ),
            None => (None, label.to_owned()),
        };
        let color = if sorted.is_some() {
            cx.theme().foreground
        } else {
            cx.theme().muted_foreground
        };
        Button::new(id)
            .text()
            .xsmall()
            .text_color(color)
            .accessibility_label(description)
            .child(label)
            .when_some(icon, |this, icon| this.child(Icon::new(icon).xsmall()))
            .on_click(cx.listener(move |this, _, _, cx| this.sort_by(column, cx)))
    }

    fn render_headings(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_none()
            .h_8()
            .px_4()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(columns(
                None::<Div>,
                self.render_heading("sort-title", "Title", Column::Title, cx),
                Some(self.render_heading("sort-artist", "Artist", Column::Artist, cx)),
                Some(self.render_heading("sort-album", "Album", Column::Album, cx)),
                self.render_heading("sort-time", "Time", Column::Time, cx),
                cx,
            ))
            .into_any_element()
    }

    /// Rendered in every state so focus and the commands bound above it never land on an element
    /// that is gone.
    fn render_table(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("songs")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .key_context(SONGS_CONTEXT)
            .on_list_navigation(|this, motion, _, cx| this.move_selection(motion, cx), cx)
            .on_action(cx.listener(|this, _: &Open, window, cx| this.play_selected(window, cx)))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.enqueue_selected(Placement::Next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.enqueue_selected(Placement::Last, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.choose_playlist_for_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToAlbum, window, cx| this.go_to_album(window, cx)))
            .on_action(
                cx.listener(|this, _: &GoToArtist, window, cx| this.go_to_artist(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &actions::ToggleFavorite, window, cx| {
                    this.toggle_selected_favorite(window, cx)
                }),
            )
            .child(self.render_rows(window, cx))
            .into_any_element()
    }

    fn render_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let songs = match &self.songs {
            SongList::Loading => return centered(Spinner::new()).into_any_element(),
            SongList::Failed(error) => {
                return LoadFailure::new(
                    "Couldn't load songs",
                    error.clone(),
                    Button::new("retry-songs")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.focus.focus(window, cx);
                            this.load(cx);
                        })),
                )
                .into_any_element();
            }
            SongList::Loaded(songs) => songs.clone(),
        };
        if songs.is_empty() {
            return centered(muted_text("This server has no songs.", cx)).into_any_element();
        }
        if self.visible.is_empty() {
            let note = session::missing_readings_note(cx);
            return no_matches("No songs match the filter.", note, cx).into_any_element();
        }
        let visible = self.visible.clone();
        let selected = self.selected.clone();
        let now_playing = self.now_playing.clone();
        let is_focused = self.focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let focus = self.focus.clone();
        row_list(
            "song-list",
            "Songs",
            visible.len(),
            &self.scroll,
            move |position| {
                let song = songs.get(*visible.get(position)?)?;
                Some(SongRow {
                    song: song.clone(),
                    state: now_playing.as_ref().and_then(|now| now.state_of(song.id())),
                    is_selected: selected.as_ref() == Some(song.id()),
                    is_focused,
                    owner: owner.clone(),
                    focus: focus.clone(),
                })
            },
        )
        .size_full()
        .into_any_element()
    }
}

fn columns(
    marker: Option<impl IntoElement>,
    title: impl IntoElement,
    artist: Option<impl IntoElement>,
    album: Option<impl IntoElement>,
    time: impl IntoElement,
    cx: &App,
) -> Div {
    let muted_foreground = cx.theme().muted_foreground;
    h_flex()
        .w_full()
        .min_w_0()
        .gap_3()
        // A lane of its own, so the titles stay in line
        .child(h_flex().w_4().flex_none().justify_center().children(marker))
        .child(
            h_flex()
                .flex_grow(3.)
                .flex_shrink(3.)
                .flex_basis(gpui_kit::relative(0.))
                .min_w_0()
                .child(title),
        )
        .child(
            h_flex()
                .flex_grow(2.)
                .flex_shrink(2.)
                .flex_basis(gpui_kit::relative(0.))
                .min_w_0()
                .text_color(muted_foreground)
                .children(artist),
        )
        .child(
            h_flex()
                .flex_grow(2.)
                .flex_shrink(2.)
                .flex_basis(gpui_kit::relative(0.))
                .min_w_0()
                .text_color(muted_foreground)
                .children(album),
        )
        .child(
            h_flex()
                .w_16()
                .flex_none()
                .justify_end()
                .text_color(muted_foreground)
                .child(time),
        )
}

impl Focusable for Songs {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Songs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("songs-screen")
            .key_context(ROOT_CONTEXT)
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter.update(cx, |input, cx| input.focus(window, cx));
            }))
            .child(self.render_header(cx))
            .when_some(self.refresh_error.clone(), |this, error| {
                this.child(RefreshFailure::new(
                    "Couldn't refresh the songs",
                    error,
                    Button::new("retry-refresh-songs")
                        .label("Retry")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                ))
            })
            .child(self.render_headings(cx))
            .child(self.render_table(window, cx))
    }
}

#[cfg(test)]
mod tests;
