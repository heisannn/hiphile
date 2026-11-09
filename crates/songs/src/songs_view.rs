use std::mem;
use std::sync::Arc;

use actions::{EnqueueSongs, GoToArtist, Placement, PlaySongs, ShowAlbum, ShowArtist};
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
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollStrategy, Styled as _,
    Subscription, Task, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _,
};
use hiphile_core::favorites::Favorite;
use hiphile_core::songs::{Column, SongList, SongListStatus};
use hiphile_core::{Shelf, Shown};
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
    song_list: SongList,
    /// What is on show, which follows `song_list`.
    status: SongListStatus,
    /// Kept by song, so it follows the song as the order changes.
    selected: Option<SongId>,
    /// Whether the user chose the selection. Until then it is simply the first
    /// song on show, which changing the order should not chase down the list.
    has_chosen_selection: bool,
    now_playing: Option<NowPlaying>,
    filter: Entity<InputState>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    _changes: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Songs {
    pub fn new(
        session: session::Session,
        song_list: SongList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter songs"));
        let focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.filter_changed(cx),
                InputEvent::PressEnter { .. } => this.focus.focus(window, cx),
                _ => {}
            }),
            // Focus changes the selection treatment of the rows
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        let changes = gpui_host::observe(song_list.subscribe(), cx, Self::follow);
        Self {
            session,
            status: song_list.status(),
            song_list,
            selected: None,
            has_chosen_selection: false,
            now_playing: None,
            filter,
            focus,
            scroll: UniformListScrollHandle::new(),
            _changes: changes,
            _subscriptions: subscriptions,
        }
    }

    /// Reading every song is too much to do for a screen that may never show.
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.song_list.open();
        self.sync(cx);
    }

    /// Takes the list's status at once after a call into it, so that what follows does not act
    /// on the old one.
    fn sync(&mut self, cx: &mut Context<Self>) {
        self.follow(self.song_list.status(), cx);
    }

    /// Keeps the selection on what shows.
    fn follow(&mut self, status: SongListStatus, cx: &mut Context<Self>) {
        let old = mem::replace(&mut self.status, status);
        if old.shown() != self.status.shown() {
            self.select_first_unless_chosen();
            // New songs leave the scroll where it was
            let is_reordered = matches!(
                (old.shown(), self.status.shown()),
                (Some(old), Some(new)) if Arc::ptr_eq(old.items(), new.items())
            );
            if is_reordered {
                self.bring_selection_into_view();
            }
        }
        cx.notify();
    }

    fn shown(&self) -> Option<&Shown<Song>> {
        self.status.shown()
    }

    fn visible_len(&self) -> usize {
        self.shown().map_or(0, |shown| shown.visible().len())
    }

    fn visible_song(&self, position: usize) -> Option<&Song> {
        self.shown()?.get(position)
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.shown()?.position(|song| song.id() == selected)
    }

    fn selected_song(&self) -> Option<&Song> {
        self.visible_song(self.selected_position()?)
    }

    fn filter_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        self.song_list.set_query(&query);
        self.sync(cx);
    }

    fn choose_sort(&mut self, column: Column, cx: &mut Context<Self>) {
        self.song_list.choose_sort(column);
        self.sync(cx);
    }

    fn choose_direction(&mut self, is_descending: bool, cx: &mut Context<Self>) {
        self.song_list.choose_direction(is_descending);
        self.sync(cx);
    }

    fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        self.song_list.sort_by(column);
        self.sync(cx);
    }

    fn is_ranked(&self) -> bool {
        self.status.is_ranked()
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
        let len = self.visible_len();
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
        let queue = self
            .selected_position()
            .and_then(|position| self.shown()?.queue_from(position));
        if let Some((songs, start_ix)) = queue {
            window.dispatch_action(Box::new(PlaySongs::new(songs, start_ix)), cx);
        }
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
        let count = self.shown().map(|shown| shown.items().len());
        let is_updating = count.is_some() && self.status.is_loading();
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
        let is_ranked = self.is_ranked();
        let sort = self.status.sort();
        let option = |label: &'static str, column: Column| {
            let owner = cx.entity().downgrade();
            let is_checked = sort.column == column && !is_ranked;
            Some(SortOption::new(label, is_checked, move |_, cx| {
                owner
                    .update(cx, |this, cx| this.choose_sort(column, cx))
                    .ok();
            }))
        };
        let direction = |label: &'static str, is_descending: bool| {
            let owner = cx.entity().downgrade();
            let is_checked = sort.is_descending == is_descending && !is_ranked;
            Some(SortOption::new(label, is_checked, move |_, cx| {
                owner
                    .update(cx, |this, cx| this.choose_direction(is_descending, cx))
                    .ok();
            }))
        };
        sort_menu(
            "sort-songs",
            sort.is_descending && !is_ranked,
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
        let sort = self.status.sort();
        let sorted = (sort.column == column && !self.is_ranked()).then_some(sort.is_descending);
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
        let shown = match (self.shown(), self.status.songs()) {
            (Some(shown), _) => shown,
            (None, Shelf::Failed(error)) => {
                return LoadFailure::new(
                    "Couldn't load songs",
                    error.clone().into(),
                    Button::new("retry-songs")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.focus.focus(window, cx);
                            this.song_list.reload();
                            this.sync(cx);
                        })),
                )
                .into_any_element();
            }
            (None, Shelf::Loading | Shelf::Loaded(_)) => {
                return centered(Spinner::new()).into_any_element();
            }
        };
        let songs = shown.items().clone();
        if songs.is_empty() {
            return centered(muted_text("This server has no songs.", cx)).into_any_element();
        }
        if shown.visible().is_empty() {
            let note = self.session.missing_readings_note();
            return no_matches("No songs match the filter.", note, cx).into_any_element();
        }
        let visible = shown.visible().clone();
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
            .when_some(self.status.failure(), |this, error| {
                this.child(RefreshFailure::new(
                    "Couldn't refresh the songs",
                    error.to_owned().into(),
                    Button::new("retry-refresh-songs")
                        .label("Retry")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.song_list.reload();
                            this.sync(cx);
                        })),
                ))
            })
            .child(self.render_headings(cx))
            .child(self.render_table(window, cx))
    }
}

#[cfg(test)]
mod tests;
