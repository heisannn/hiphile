use std::sync::Arc;

use actions::PlaySongs;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    resizable::{h_resizable, resizable_panel},
    scroll::Scrollbar,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, UniformListScrollHandle,
    Window, div, prelude::FluentBuilder as _, uniform_list,
};
use subsonic::{Album, AlbumId, Song, SongId};
use ui::{CoverArt, format_duration};

use crate::filter::matching_albums;
use crate::{
    ALBUMS_CONTEXT, Back, FocusFilter, LIBRARY_CONTEXT, Open, SONGS_CONTEXT, SelectNext,
    SelectPrevious,
};

/// Pixel size requested for the detail cover: twice its on-screen size, for
/// high-density displays.
const COVER_PIXELS: u32 = 512;

/// The album browser: a filterable album list beside the selected album's
/// songs. Owns the album data, the selection, and both lists' focus.
pub struct Library {
    session: session::Session,
    albums: Albums,
    /// Positions in the loaded albums that match the filter.
    visible: Arc<Vec<usize>>,
    selected_album: Option<AlbumId>,
    detail: Detail,
    selected_song: Option<SongId>,
    filter: Entity<InputState>,
    albums_focus: FocusHandle,
    songs_focus: FocusHandle,
    albums_scroll: UniformListScrollHandle,
    songs_scroll: UniformListScrollHandle,
    _albums_task: Task<()>,
    _album_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

enum Albums {
    Loading,
    Loaded(Arc<Vec<Album>>),
    Failed(SharedString),
}

enum Detail {
    None,
    Loading,
    Loaded(LoadedAlbum),
    Failed(SharedString),
}

struct LoadedAlbum {
    album: Album,
    songs: Arc<[Song]>,
    cover_url: Option<SharedString>,
}

impl Library {
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter albums"));
        let albums_focus = cx.focus_handle().tab_stop(true);
        let songs_focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.apply_filter(cx),
                InputEvent::PressEnter { .. } => this.albums_focus.focus(window, cx),
                _ => {}
            }),
            // Focus changes the selection treatment of the rows.
            cx.on_focus(&albums_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&albums_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus(&songs_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&songs_focus, window, |_, _, cx| cx.notify()),
        ];
        albums_focus.focus(window, cx);
        let mut this = Self {
            session,
            albums: Albums::Loading,
            visible: Arc::default(),
            selected_album: None,
            detail: Detail::None,
            selected_song: None,
            filter,
            albums_focus,
            songs_focus,
            albums_scroll: UniformListScrollHandle::new(),
            songs_scroll: UniformListScrollHandle::new(),
            _albums_task: Task::ready(()),
            _album_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.load_albums(cx);
        this
    }

    fn load_albums(&mut self, cx: &mut Context<Self>) {
        self.albums = Albums::Loading;
        self.visible = Arc::default();
        let client = self.session.client().clone();
        // Replacing the task drops a previous load, so results never race.
        self._albums_task = cx.spawn(async move |this, cx| {
            let result = client.albums_by_name().await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(albums) => {
                        this.albums = Albums::Loaded(Arc::new(albums));
                        this.apply_filter(cx);
                        if this.selected_album.is_none()
                            && let Some(first) =
                                this.visible_album(0).map(|album| album.id().clone())
                        {
                            this.select_album(first, cx);
                        }
                    }
                    Err(error) => {
                        this.albums = Albums::Failed(format!("{error}.").into());
                    }
                }
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    fn loaded_albums(&self) -> Option<&Arc<Vec<Album>>> {
        match &self.albums {
            Albums::Loaded(albums) => Some(albums),
            _ => None,
        }
    }

    fn visible_album(&self, position: usize) -> Option<&Album> {
        let albums = self.loaded_albums()?;
        albums.get(*self.visible.get(position)?)
    }

    fn selected_position(&self) -> Option<usize> {
        let selected = self.selected_album.as_ref()?;
        (0..self.visible.len()).find(|position| {
            self.visible_album(*position)
                .is_some_and(|album| album.id() == selected)
        })
    }

    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        self.visible = Arc::new(
            self.loaded_albums()
                .map(|albums| matching_albums(albums, &query))
                .unwrap_or_default(),
        );
        cx.notify();
    }

    fn select_album(&mut self, album: AlbumId, cx: &mut Context<Self>) {
        if self.selected_album.as_ref() == Some(&album) {
            return;
        }
        if let Some(position) = self.visible.iter().position(|ix| {
            self.loaded_albums()
                .and_then(|albums| albums.get(*ix))
                .is_some_and(|candidate| candidate.id() == &album)
        }) {
            self.albums_scroll
                .scroll_to_item(position, ScrollStrategy::Center);
        }
        self.selected_album = Some(album.clone());
        self.selected_song = None;
        self.detail = Detail::Loading;
        self.songs_scroll.scroll_to_item(0, ScrollStrategy::Top);
        let client = self.session.client().clone();
        self._album_task = cx.spawn(async move |this, cx| {
            let result = client.album(&album).await;
            this.update(cx, |this, cx| {
                if this.selected_album.as_ref() != Some(&album) {
                    return;
                }
                this.detail = match result {
                    Ok(loaded) => {
                        let (album, songs) = loaded.into_parts();
                        let cover_url = album
                            .cover_art()
                            .map(|cover_art| this.session.cover_art_url(cover_art, COVER_PIXELS));
                        Detail::Loaded(LoadedAlbum {
                            album,
                            songs: songs.into(),
                            cover_url,
                        })
                    }
                    Err(error) => Detail::Failed(format!("{error}.").into()),
                };
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    fn retry_album(&mut self, cx: &mut Context<Self>) {
        if let Some(album) = self.selected_album.take() {
            self.select_album(album, cx);
        }
    }

    fn move_album(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.visible.is_empty() {
            return;
        }
        let position = match self.selected_position() {
            Some(position) => position
                .saturating_add_signed(delta)
                .min(self.visible.len() - 1),
            None => 0,
        };
        if let Some(album) = self.visible_album(position).map(|album| album.id().clone()) {
            self.select_album(album, cx);
        }
    }

    fn loaded_songs(&self) -> Option<&Arc<[Song]>> {
        match &self.detail {
            Detail::Loaded(loaded) => Some(&loaded.songs),
            _ => None,
        }
    }

    fn selected_song_ix(&self) -> Option<usize> {
        let selected = self.selected_song.as_ref()?;
        self.loaded_songs()?
            .iter()
            .position(|song| song.id() == selected)
    }

    fn select_song(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(song) = self.loaded_songs().and_then(|songs| songs.get(ix)) else {
            return;
        };
        self.selected_song = Some(song.id().clone());
        self.songs_scroll.scroll_to_item(ix, ScrollStrategy::Center);
        cx.notify();
    }

    fn move_song(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(len) = self.loaded_songs().map(|songs| songs.len()) else {
            return;
        };
        if len == 0 {
            return;
        }
        let ix = match self.selected_song_ix() {
            Some(ix) => ix.saturating_add_signed(delta).min(len - 1),
            None => 0,
        };
        self.select_song(ix, cx);
    }

    /// Moves keyboard focus to the songs, selecting the first one if none is.
    fn open_album(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded_songs().is_none_or(|songs| songs.is_empty()) {
            return;
        }
        if self.selected_song_ix().is_none() {
            self.select_song(0, cx);
        }
        self.songs_focus.focus(window, cx);
    }

    fn play_from(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(songs) = self.loaded_songs().filter(|songs| ix < songs.len()) {
            window.dispatch_action(Box::new(PlaySongs::new(songs.clone(), ix)), cx);
        }
    }

    fn render_albums(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.loaded_albums().map(|albums| albums.len());
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .h_10()
                    .flex_none()
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_medium().child("Albums"))
                    .when_some(count, |this, count| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(count.to_string()),
                        )
                    }),
            )
            .child(
                div().flex_none().px_3().py_2().child(
                    Input::new(&self.filter)
                        .small()
                        .prefix(Icon::new(IconName::Search).small())
                        .cleanable(true),
                ),
            )
            .child(self.render_album_list(window, cx))
            .into_any_element()
    }

    /// The album list's focus target, rendered in every state so focus and
    /// the commands bound above it never land on an element that is gone.
    fn render_album_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("albums")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .track_focus(&self.albums_focus)
            .key_context(ALBUMS_CONTEXT)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_album(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.move_album(-1, cx)))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open_album(window, cx)))
            .child(self.render_album_rows(window, cx))
            .into_any_element()
    }

    fn render_album_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let albums = match &self.albums {
            Albums::Loading => return centered(Spinner::new()).into_any_element(),
            Albums::Failed(error) => {
                return failure(
                    "Couldn't load albums",
                    error.clone(),
                    Button::new("retry-albums")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.load_albums(cx))),
                    cx,
                );
            }
            Albums::Loaded(albums) => albums.clone(),
        };
        if albums.is_empty() {
            return centered(muted("This server has no albums.", cx)).into_any_element();
        }
        if self.visible.is_empty() {
            return centered(muted("No albums match the filter.", cx)).into_any_element();
        }

        let visible = self.visible.clone();
        let selected = self.selected_album.clone();
        let focused = self.albums_focus.is_focused(window);
        let muted_foreground = cx.theme().muted_foreground;
        let owner = cx.entity().downgrade();
        div()
            .id("album-list")
            .relative()
            .size_full()
            .role(gpui_kit::Role::List)
            .aria_label("Albums")
            .child(
                uniform_list("album-rows", visible.len(), move |range, _, _| {
                    range
                        .filter_map(|position| {
                            let album = albums.get(*visible.get(position)?)?;
                            let id = album.id().clone();
                            let is_selected = selected.as_ref() == Some(&id);
                            let name = SharedString::from(album.name().to_owned());
                            let details = album_details(album);
                            let owner = owner.clone();
                            Some(
                                ListItem::new(ElementId::Name(format!("album-{id}").into()))
                                    .selected(is_selected)
                                    .secondary_selected(focused && is_selected)
                                    .h_12()
                                    .px_4()
                                    .w_full()
                                    .accessibility_label(name.clone())
                                    .child(
                                        v_flex()
                                            .min_w_0()
                                            .flex_1()
                                            .child(div().text_sm().truncate().child(name))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .truncate()
                                                    .text_color(muted_foreground)
                                                    .child(details),
                                            ),
                                    )
                                    .on_click(move |_, window, cx| {
                                        owner
                                            .update(cx, |this, cx| {
                                                this.albums_focus.focus(window, cx);
                                                this.select_album(id.clone(), cx);
                                            })
                                            .ok();
                                    }),
                            )
                        })
                        .collect()
                })
                .size_full()
                .track_scroll(&self.albums_scroll),
            )
            .child(Scrollbar::vertical(&self.albums_scroll))
            .into_any_element()
    }

    /// The detail pane is the songs' focus target in every state, for the
    /// same reason as [`Self::render_album_list`].
    fn render_detail(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("detail")
            .size_full()
            .min_w_0()
            .min_h_0()
            .track_focus(&self.songs_focus)
            .key_context(SONGS_CONTEXT)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_song(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.move_song(-1, cx)))
            .on_action(cx.listener(|this, _: &Open, window, cx| {
                if let Some(ix) = this.selected_song_ix() {
                    this.play_from(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Back, window, cx| {
                this.albums_focus.focus(window, cx);
            }))
            .child(self.render_detail_content(window, cx))
            .into_any_element()
    }

    fn render_detail_content(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let loaded = match &self.detail {
            Detail::None => return div().size_full().into_any_element(),
            Detail::Loading => return centered(Spinner::new()).into_any_element(),
            Detail::Failed(error) => {
                return failure(
                    "Couldn't load this album",
                    error.clone(),
                    Button::new("retry-album")
                        .label("Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.retry_album(cx))),
                    cx,
                );
            }
            Detail::Loaded(loaded) => loaded,
        };
        let album = &loaded.album;
        let total = album
            .duration()
            .or_else(|| loaded.songs.iter().map(Song::duration).sum());
        let summary = [
            album.year().map(|year| year.to_string()),
            Some(match loaded.songs.len() {
                1 => "1 song".to_owned(),
                count => format!("{count} songs"),
            }),
            total.map(format_duration),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                h_flex()
                    .flex_none()
                    .items_end()
                    .gap_6()
                    .p_6()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(CoverArt::new(loaded.cover_url.clone()).large())
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_semibold()
                                    .child(SharedString::from(album.name().to_owned())),
                            )
                            .when_some(album.artist(), |this, artist| {
                                this.child(div().child(SharedString::from(artist.to_owned())))
                            })
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(summary),
                            )
                            .child(
                                h_flex().pt_3().child(
                                    Button::new("play-album")
                                        .icon(IconName::Play)
                                        .label("Play")
                                        .disabled(loaded.songs.is_empty())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.play_from(0, window, cx)
                                        })),
                                ),
                            ),
                    ),
            )
            .child(self.render_songs(loaded, window, cx))
            .into_any_element()
    }

    fn render_songs(
        &self,
        loaded: &LoadedAlbum,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if loaded.songs.is_empty() {
            return centered(muted("This album has no songs.", cx)).into_any_element();
        }
        let songs = loaded.songs.clone();
        let album_artist = loaded.album.artist().map(str::to_owned);
        let selected = self.selected_song.clone();
        let focused = self.songs_focus.is_focused(window);
        let muted_foreground = cx.theme().muted_foreground;
        let owner = cx.entity().downgrade();
        div()
            .id("songs")
            .relative()
            .flex_1()
            .min_h_0()
            .role(gpui_kit::Role::List)
            .aria_label("Songs")
            .child(
                uniform_list("song-rows", songs.len(), move |range, _, _| {
                    range
                        .filter_map(|ix| {
                            let song = songs.get(ix)?;
                            let id = song.id().clone();
                            let is_selected = selected.as_ref() == Some(&id);
                            let title = SharedString::from(song.title().to_owned());
                            // Name the performer only where it differs from the album's.
                            let artist = song
                                .artist()
                                .filter(|artist| album_artist.as_deref() != Some(*artist))
                                .map(|artist| SharedString::from(artist.to_owned()));
                            let number = song
                                .track()
                                .map(|track| track.to_string())
                                .unwrap_or_default();
                            let duration = song.duration().map(format_duration).unwrap_or_default();
                            let owner = owner.clone();
                            Some(
                                ListItem::new(ElementId::Name(format!("song-{id}").into()))
                                    .selected(is_selected)
                                    .secondary_selected(focused && is_selected)
                                    .h_10()
                                    .px_6()
                                    .w_full()
                                    .accessibility_label(title.clone())
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .min_w_0()
                                            .gap_4()
                                            .text_sm()
                                            .child(
                                                div()
                                                    .w_6()
                                                    .flex_none()
                                                    .text_right()
                                                    .text_color(muted_foreground)
                                                    .child(number),
                                            )
                                            .child(
                                                h_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .gap_2()
                                                    .child(div().truncate().child(title))
                                                    .when_some(artist, |this, artist| {
                                                        this.child(
                                                            div()
                                                                .truncate()
                                                                .text_color(muted_foreground)
                                                                .child(artist),
                                                        )
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .text_color(muted_foreground)
                                                    .child(duration),
                                            ),
                                    )
                                    .on_click(move |event, window, cx| {
                                        owner
                                            .update(cx, |this, cx| {
                                                this.songs_focus.focus(window, cx);
                                                this.select_song(ix, cx);
                                                if event.click_count() >= 2 {
                                                    this.play_from(ix, window, cx);
                                                }
                                            })
                                            .ok();
                                    }),
                            )
                        })
                        .collect()
                })
                .size_full()
                .track_scroll(&self.songs_scroll),
            )
            .child(Scrollbar::vertical(&self.songs_scroll))
            .into_any_element()
    }
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        div()
            .id("library")
            .key_context(LIBRARY_CONTEXT)
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter.update(cx, |input, cx| input.focus(window, cx));
            }))
            .child(
                h_resizable("library-panes")
                    .child(
                        resizable_panel()
                            .size(rem * 20.)
                            .size_range(rem * 14. ..rem * 32.)
                            .fixed()
                            .child(self.render_albums(window, cx)),
                    )
                    .child(
                        resizable_panel()
                            .size_range(rem * 30. ..rem * 200.)
                            .child(self.render_detail(window, cx)),
                    ),
            )
    }
}

/// The second line of an album row.
fn album_details(album: &Album) -> SharedString {
    match (album.artist(), album.year()) {
        (Some(artist), Some(year)) => format!("{artist} · {year}").into(),
        (Some(artist), None) => artist.to_owned().into(),
        (None, Some(year)) => year.to_string().into(),
        (None, None) => SharedString::default(),
    }
}

fn centered(child: impl IntoElement) -> gpui_kit::Div {
    div()
        .flex_1()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .child(child)
}

fn muted(text: &'static str, cx: &Context<Library>) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// A failed load: what failed, why, and a way to try again.
fn failure(
    title: &'static str,
    reason: SharedString,
    retry: Button,
    cx: &Context<Library>,
) -> AnyElement {
    v_flex()
        .size_full()
        .p_6()
        .gap_3()
        .child(div().font_medium().child(title))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(reason),
        )
        .child(h_flex().child(retry))
        .into_any_element()
}
