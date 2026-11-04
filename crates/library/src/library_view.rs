use std::sync::Arc;

use actions::{AlbumAction, EnqueueSongs, Placement, PlaySongs};
use catalog::{Mode, Outcome};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::{ContextMenuExt as _, DropdownMenu as _},
    notification::NotificationType,
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::Scrollbar,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, AsyncApp, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    UniformListScrollHandle, WeakEntity, Window, div, prelude::FluentBuilder as _, uniform_list,
};
use subsonic::{Album, AlbumId, AlbumWithSongs, PlaylistId, Song, SongId};
use ui::list_navigation::{
    Motion, SelectFirst, SelectLast, SelectNext, SelectPageDown, SelectPageUp, SelectPrevious,
    page_rows,
};
use ui::{CoverArt, format_duration};

use crate::filter::matching_albums;
use crate::{
    ALBUMS_CONTEXT, AddAlbumToPlaylist, AddToPlaylist, AddToQueue, Back, CONTROL_CONTEXT,
    FocusFilter, LIBRARY_CONTEXT, Open, Play, PlayNext, Refresh, SONGS_CONTEXT,
};

/// Pixel size requested for the detail cover: twice its on-screen size, for
/// high-density displays.
const COVER_PIXELS: u32 = 512;
/// The album list's width when first shown, in rem.
const ALBUMS_WIDTH: f32 = 20.;
/// The narrowest the album detail goes, in rem.
const DETAIL_MIN_WIDTH: f32 = 26.;

pub enum LibraryEvent {
    /// A sync brought the album list up to date with the server.
    AlbumsSynced,
}

/// The album browser: a filterable album list beside the selected album's
/// songs. Owns the album data, the selection, and both lists' focus.
pub struct Library {
    session: session::Session,
    albums: Albums,
    sync: SyncState,
    /// Positions in the loaded albums that match the filter.
    visible: Arc<Vec<usize>>,
    selected_album: Option<AlbumId>,
    /// The selected album, when it was asked to show before the list had it.
    /// It stays selected and comes into view once a sync brings it.
    revealing: Option<AlbumId>,
    detail: Detail,
    selected_song: Option<SongId>,
    now_playing: Option<NowPlaying>,
    filter: Entity<InputState>,
    albums_focus: FocusHandle,
    songs_focus: FocusHandle,
    albums_scroll: UniformListScrollHandle,
    songs_scroll: UniformListScrollHandle,
    /// The split between the album list and the detail.
    panes: Entity<ResizableState>,
    _albums_task: Task<()>,
    _album_task: Task<()>,
    _album_action_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

enum Albums {
    Loading,
    Loaded(Arc<Vec<Album>>),
    Failed(SharedString),
}

/// Bringing the library up to date with the server.
enum SyncState {
    Idle,
    /// `again`: a refresh was asked for meanwhile, to run once this ends
    /// rather than cut it short.
    Running {
        again: bool,
    },
    /// Updating albums already on show failed, for this reason. Albums that
    /// never showed fail as [`Albums::Failed`] instead.
    Failed(SharedString),
}

enum Detail {
    None,
    Loading,
    Loaded(LoadedAlbum),
    Failed(SharedString),
}

/// The song the player is on, marked in the song list.
#[derive(Clone, PartialEq, Eq)]
struct NowPlaying {
    song: SongId,
    playing: bool,
}

struct LoadedAlbum {
    album: Album,
    songs: Arc<[Song]>,
    cover_url: Option<SharedString>,
}

impl EventEmitter<LibraryEvent> for Library {}

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
        let mut this = Self {
            session,
            albums: Albums::Loading,
            sync: SyncState::Idle,
            visible: Arc::default(),
            selected_album: None,
            revealing: None,
            detail: Detail::None,
            selected_song: None,
            now_playing: None,
            filter,
            albums_focus,
            songs_focus,
            albums_scroll: UniformListScrollHandle::new(),
            songs_scroll: UniformListScrollHandle::new(),
            panes: cx.new(|_| ResizableState::default()),
            _albums_task: Task::ready(()),
            _album_task: Task::ready(()),
            _album_action_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.sync_library(Mode::IfStale, cx);
        this
    }

    /// Fetches the whole library from the server again.
    pub fn refresh_library(&mut self, cx: &mut Context<Self>) {
        self.sync_library(Mode::Always, cx);
    }

    /// Shows the saved albums, then brings the library up to date: the
    /// albums first, then every song.
    fn sync_library(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if let SyncState::Running { again } = &mut self.sync {
            *again |= mode == Mode::Always;
            return;
        }
        self.sync = SyncState::Running { again: false };
        if self.loaded_albums().is_none() {
            self.albums = Albums::Loading;
            self.visible = Arc::default();
        }
        let session = self.session.clone();
        self._albums_task = cx.spawn(async move |this, cx| {
            let result = run_sync(&this, &session, mode, cx).await;
            this.update(cx, |this, cx| this.finish_sync(result, cx))
                .ok();
        });
        cx.notify();
    }

    fn finish_sync(&mut self, result: Result<(), SharedString>, cx: &mut Context<Self>) {
        let again = matches!(self.sync, SyncState::Running { again: true });
        self.sync = match result {
            Ok(()) => SyncState::Idle,
            Err(error) if self.loaded_albums().is_some() => SyncState::Failed(error),
            Err(error) => {
                self.albums = Albums::Failed(error);
                SyncState::Idle
            }
        };
        if again {
            self.refresh_library(cx);
        } else {
            self.revealing = None;
        }
        cx.notify();
    }

    /// Replaces the album list, keeping the selection while the album is
    /// still there or a sync may yet bring it.
    fn set_albums(&mut self, albums: Vec<Album>, cx: &mut Context<Self>) {
        let keeps_selection = self.revealing.is_some()
            || self
                .selected_album
                .as_ref()
                .is_some_and(|selected| albums.iter().any(|album| album.id() == selected));
        let revealed = self
            .revealing
            .as_ref()
            .is_some_and(|revealing| albums.iter().any(|album| album.id() == revealing));
        self.albums = Albums::Loaded(Arc::new(albums));
        self.apply_filter(cx);
        if !keeps_selection {
            self.selected_album = None;
            self.detail = Detail::None;
            if let Some(first) = self.visible_album(0).map(|album| album.id().clone()) {
                self.select_album(first, cx);
            }
        }
        if revealed {
            self.revealing = None;
            self.scroll_to_selected_album();
        }
        cx.notify();
    }

    fn scroll_to_selected_album(&self) {
        if let Some(position) = self.selected_position() {
            self.albums_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
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
        self.revealing = None;
        if let Some(position) = self.visible.iter().position(|ix| {
            self.loaded_albums()
                .and_then(|albums| albums.get(*ix))
                .is_some_and(|candidate| candidate.id() == &album)
        }) {
            self.albums_scroll
                .scroll_to_item(position, ScrollStrategy::Nearest);
        }
        self.selected_album = Some(album.clone());
        self.selected_song = None;
        self.detail = Detail::Loading;
        self.songs_scroll.scroll_to_item(0, ScrollStrategy::Top);
        let client = self.session.client().clone();
        let catalog = self.session.catalog().cloned();
        self._album_task = cx.spawn(async move |this, cx| {
            // The saved songs show at once; the server's answer replaces them.
            if let Some(catalog) = catalog.clone() {
                let id = album.clone();
                let cached = cx.background_spawn(async move { catalog.album(&id) }).await;
                if let Ok(Some(cached)) = cached {
                    this.update(cx, |this, cx| this.show_album(&album, cached, cx))
                        .ok();
                }
            }
            let result = {
                let album = album.clone();
                cx.background_spawn(async move { client.album(&album).await })
                    .await
            };
            if let (Ok(fetched), Some(catalog)) = (&result, catalog) {
                let fetched = fetched.clone();
                cx.background_spawn(async move { catalog.put_album(&fetched).ok() })
                    .detach();
            }
            this.update(cx, |this, cx| match result {
                Ok(fetched) => this.show_album(&album, fetched, cx),
                // Saved songs on show stay there.
                Err(error)
                    if this.selected_album.as_ref() == Some(&album)
                        && !matches!(this.detail, Detail::Loaded(_)) =>
                {
                    this.detail = Detail::Failed(session::describe(&error).into());
                    cx.notify();
                }
                Err(_) => {}
            })
            .ok();
        });
        cx.notify();
    }

    /// Selects `album` and brings it into view in the album list, clearing a
    /// filter that hides it. An album the list does not have yet, as during
    /// the first sync, comes into view once a sync brings it.
    pub fn reveal_album(&mut self, album: AlbumId, window: &mut Window, cx: &mut Context<Self>) {
        let position = self
            .loaded_albums()
            .and_then(|albums| albums.iter().position(|listed| listed.id() == &album));
        if position.is_some_and(|position| !self.visible.contains(&position)) {
            self.filter
                .update(cx, |filter, cx| filter.set_value("", window, cx));
            self.apply_filter(cx);
        }
        self.select_album(album.clone(), cx);
        self.revealing = position.is_none().then_some(album);
        // Selecting the album already selected does not scroll.
        self.scroll_to_selected_album();
        self.albums_focus.focus(window, cx);
    }

    fn show_album(&mut self, id: &AlbumId, loaded: AlbumWithSongs, cx: &mut Context<Self>) {
        if self.selected_album.as_ref() != Some(id) {
            return;
        }
        let (album, songs) = loaded.into_parts();
        let cover_url = album
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, COVER_PIXELS));
        self.detail = Detail::Loaded(LoadedAlbum {
            album,
            songs: songs.into(),
            cover_url,
        });
        cx.notify();
    }

    fn retry_album(&mut self, cx: &mut Context<Self>) {
        if let Some(album) = self.selected_album.take() {
            self.select_album(album, cx);
        }
    }

    /// The narrowest the library can be while showing its album list at its
    /// current width beside the detail.
    pub fn min_width(&self, window: &Window, cx: &gpui_kit::App) -> gpui_kit::Pixels {
        let rem = window.rem_size();
        let albums = self
            .panes
            .read(cx)
            .sizes()
            .first()
            .copied()
            .filter(|width| !gpui_kit::IsZero::is_zero(width))
            .unwrap_or(rem * ALBUMS_WIDTH);
        albums + rem * DETAIL_MIN_WIDTH
    }

    /// Marks `song` in the song list, or clears the mark.
    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = song.map(|song| NowPlaying { song, playing });
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    fn move_album(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let page = page_rows(&self.albums_scroll);
        let Some(position) = motion.apply(self.selected_position(), self.visible.len(), page)
        else {
            return;
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
        self.songs_scroll
            .scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn move_song(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let len = self.loaded_songs().map_or(0, |songs| songs.len());
        let page = page_rows(&self.songs_scroll);
        if let Some(ix) = motion.apply(self.selected_song_ix(), len, page) {
            self.select_song(ix, cx);
        }
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

    /// Does `action` to the whole of the selected album, once its songs are
    /// there if they are still loading. Replaces an action still waiting for
    /// its songs: the latest one counts.
    fn act_on_album(&mut self, action: AlbumAction, window: &mut Window, cx: &mut Context<Self>) {
        self._album_action_task = Task::ready(());
        if let Some(songs) = self.loaded_songs() {
            if let Some(command) = action.command(songs.clone()) {
                window.dispatch_action(command, cx);
            }
            return;
        }
        let Some(album) = self.selected_album.clone() else {
            return;
        };
        let session = self.session.clone();
        self._album_action_task = cx.spawn_in(window, async move |_, cx| {
            let loaded = cx
                .background_spawn(async move { session.album(&album).await })
                .await;
            cx.update(|window, cx| match loaded {
                Ok(loaded) => {
                    if let Some(command) = action.command(loaded.into_parts().1.into()) {
                        window.dispatch_action(command, cx);
                    }
                }
                Err(error) => {
                    let message =
                        format!("Couldn't load this album. {}", session::describe(&error));
                    window.push_notification((NotificationType::Error, message), cx);
                }
            })
            .ok();
        });
    }

    fn enqueue_selected_song(
        &self,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let song = self
            .selected_song_ix()
            .and_then(|ix| self.loaded_songs()?.get(ix).cloned());
        if let Some(song) = song {
            window.dispatch_action(
                Box::new(EnqueueSongs::new(Arc::from([song]), placement)),
                cx,
            );
        }
    }

    fn choose_playlist_for_selected_song(&self, window: &mut Window, cx: &mut Context<Self>) {
        let song = self
            .selected_song_ix()
            .and_then(|ix| self.loaded_songs()?.get(ix).cloned());
        if let Some(song) = song {
            window.dispatch_action(
                Box::new(actions::ChoosePlaylist::new(Arc::from([song]))),
                cx,
            );
        }
    }

    fn render_albums(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.loaded_albums().map(|albums| albums.len());
        // While nothing shows, the list itself shows a spinner.
        let updating = count.is_some() && matches!(self.sync, SyncState::Running { .. });
        let error = match &self.sync {
            SyncState::Failed(error) => Some(error.clone()),
            SyncState::Idle | SyncState::Running { .. } => None,
        };
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
                    })
                    .when(updating, |this| {
                        this.child(
                            div()
                                .id("library-updating")
                                .role(gpui_kit::Role::ProgressIndicator)
                                .aria_label("Updating the library")
                                .child(Spinner::new().small().color(cx.theme().muted_foreground)),
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
            .when_some(error, |this, error| {
                this.child(
                    v_flex()
                        .flex_none()
                        .px_4()
                        .pb_2()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_medium()
                                .child("Couldn't refresh the library"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(error),
                        )
                        .child(
                            h_flex().pt_1().child(
                                Button::new("retry-refresh")
                                    .label("Retry")
                                    .small()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.refresh_library(cx)),
                                    ),
                            ),
                        ),
                )
            })
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
            .on_action(
                cx.listener(|this, _: &SelectNext, _, cx| this.move_album(Motion::By(1), cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectPrevious, _, cx| this.move_album(Motion::By(-1), cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectFirst, _, cx| this.move_album(Motion::First, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.move_album(Motion::Last, cx)))
            .on_action(
                cx.listener(|this, _: &SelectPageDown, _, cx| {
                    this.move_album(Motion::Pages(1), cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SelectPageUp, _, cx| this.move_album(Motion::Pages(-1), cx)),
            )
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open_album(window, cx)))
            .on_action(cx.listener(|this, _: &Play, window, cx| {
                this.act_on_album(AlbumAction::Play, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.act_on_album(AlbumAction::Enqueue(Placement::Next), window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.act_on_album(AlbumAction::Enqueue(Placement::Last), window, cx)
            }))
            .on_action(
                cx.listener(|this, action: &AddAlbumToPlaylist, window, cx| {
                    let action = AlbumAction::AddToPlaylist(action.playlist.clone());
                    this.act_on_album(action, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.act_on_album(AlbumAction::ChoosePlaylist, window, cx)
            }))
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
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.albums_focus.focus(window, cx);
                            this.refresh_library(cx);
                        })),
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
        let albums_focus = self.albums_focus.clone();
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
                            let on_click = owner.clone();
                            let on_right_click = owner.clone();
                            let click_id = id.clone();
                            let albums_focus = albums_focus.clone();
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
                                        on_click
                                            .update(cx, |this, cx| {
                                                this.albums_focus.focus(window, cx);
                                                this.select_album(click_id.clone(), cx);
                                            })
                                            .ok();
                                    })
                                    // The menu acts on the selection, so the
                                    // row under the pointer becomes it first.
                                    .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                                        on_right_click
                                            .update(cx, |this, cx| {
                                                this.albums_focus.focus(window, cx);
                                                this.select_album(id.clone(), cx);
                                            })
                                            .ok();
                                    })
                                    .context_menu(move |menu, window, cx| {
                                        let menu = menu
                                            .action_context(albums_focus.clone())
                                            .menu("Play", Box::new(Play))
                                            .menu("Play Next", Box::new(PlayNext))
                                            .menu("Add to Queue", Box::new(AddToQueue))
                                            .separator();
                                        actions::add_to_playlist_submenu(
                                            menu,
                                            add_album_command,
                                            window,
                                            cx,
                                        )
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
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_song(Motion::By(1), cx)))
            .on_action(
                cx.listener(|this, _: &SelectPrevious, _, cx| this.move_song(Motion::By(-1), cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectFirst, _, cx| this.move_song(Motion::First, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.move_song(Motion::Last, cx)))
            .on_action(
                cx.listener(|this, _: &SelectPageDown, _, cx| this.move_song(Motion::Pages(1), cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectPageUp, _, cx| this.move_song(Motion::Pages(-1), cx)),
            )
            .on_action(cx.listener(|this, _: &Open, window, cx| {
                if let Some(ix) = this.selected_song_ix() {
                    this.play_from(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PlayNext, window, cx| {
                this.enqueue_selected_song(Placement::Next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToQueue, window, cx| {
                this.enqueue_selected_song(Placement::Last, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToPlaylist, window, cx| {
                this.choose_playlist_for_selected_song(window, cx)
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
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.songs_focus.focus(window, cx);
                            this.retry_album(cx);
                        })),
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
                                h_flex()
                                    .key_context(CONTROL_CONTEXT)
                                    .pt_3()
                                    .gap_2()
                                    .child(
                                        Button::new("play-album")
                                            .icon(IconName::Play)
                                            .label("Play")
                                            .disabled(loaded.songs.is_empty())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.play_from(0, window, cx)
                                            })),
                                    )
                                    .child(self.render_album_menu(loaded)),
                            ),
                    ),
            )
            .child(self.render_songs(loaded, window, cx))
            .into_any_element()
    }

    /// The album's commands besides Play.
    ///
    /// The album list's commands act on the album on show, so the menu
    /// dispatches them there, whichever list has focus.
    fn render_album_menu(&self, loaded: &LoadedAlbum) -> impl IntoElement {
        let albums_focus = self.albums_focus.clone();
        let songs = loaded.songs.clone();
        Button::new("album-menu")
            .ghost()
            .icon(IconName::Ellipsis)
            .accessibility_label("More")
            .disabled(loaded.songs.is_empty())
            .dropdown_menu(move |menu, window, cx| {
                let menu = menu
                    .action_context(albums_focus.clone())
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator();
                actions::add_to_playlist_submenu(
                    menu,
                    actions::add_songs_command(songs.clone()),
                    window,
                    cx,
                )
            })
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
        let now_playing = self.now_playing.clone();
        let focused = self.songs_focus.is_focused(window);
        let muted_foreground = cx.theme().muted_foreground;
        let primary = cx.theme().primary;
        let owner = cx.entity().downgrade();
        let songs_focus = self.songs_focus.clone();
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
                            // The playing song shows a sound icon in place of its
                            // number, dimmed while paused.
                            let marker = now_playing
                                .as_ref()
                                .filter(|now| now.song == id)
                                .map(|now| now.playing);
                            let label = match marker {
                                Some(true) => format!("{title}, playing").into(),
                                Some(false) => format!("{title}, paused").into(),
                                None => title.clone(),
                            };
                            let owner = owner.clone();
                            let songs_focus = songs_focus.clone();
                            let menu_song = song.clone();
                            Some(
                                ListItem::new(ElementId::Name(format!("song-{id}").into()))
                                    .selected(is_selected)
                                    .secondary_selected(focused && is_selected)
                                    .h_10()
                                    .px_6()
                                    .w_full()
                                    .accessibility_label(label)
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .min_w_0()
                                            .gap_4()
                                            .text_sm()
                                            .child(
                                                h_flex()
                                                    .w_6()
                                                    .flex_none()
                                                    .justify_end()
                                                    .text_color(muted_foreground)
                                                    .map(|this| match marker {
                                                        Some(playing) => this.child(
                                                            Icon::new(IconName::AudioLines)
                                                                .small()
                                                                .text_color(if playing {
                                                                    primary
                                                                } else {
                                                                    muted_foreground
                                                                }),
                                                        ),
                                                        None => this.child(number),
                                                    }),
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
                                    .on_click({
                                        let owner = owner.clone();
                                        move |event, window, cx| {
                                            owner
                                                .update(cx, |this, cx| {
                                                    this.songs_focus.focus(window, cx);
                                                    this.select_song(ix, cx);
                                                    if event.click_count() >= 2 {
                                                        this.play_from(ix, window, cx);
                                                    }
                                                })
                                                .ok();
                                        }
                                    })
                                    // The menu acts on the selection, so the
                                    // row under the pointer becomes it first.
                                    .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                                        owner
                                            .update(cx, |this, cx| {
                                                this.songs_focus.focus(window, cx);
                                                this.select_song(ix, cx);
                                            })
                                            .ok();
                                    })
                                    .context_menu(move |menu, window, cx| {
                                        let menu = menu
                                            .action_context(songs_focus.clone())
                                            .menu("Play", Box::new(Open))
                                            .menu("Play Next", Box::new(PlayNext))
                                            .menu("Add to Queue", Box::new(AddToQueue))
                                            .separator();
                                        actions::add_to_playlist_submenu(
                                            menu,
                                            actions::add_songs_command(Arc::from([
                                                menu_song.clone()
                                            ])),
                                            window,
                                            cx,
                                        )
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

impl Focusable for Library {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.albums_focus.clone()
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
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh_library(cx)))
            .child(
                h_resizable("library-panes")
                    .with_state(&self.panes)
                    .child(
                        resizable_panel()
                            .size(rem * ALBUMS_WIDTH)
                            .size_range(rem * 14. ..rem * 32.)
                            .fixed()
                            .child(self.render_albums(window, cx)),
                    )
                    .child(
                        resizable_panel()
                            .size_range(rem * DETAIL_MIN_WIDTH..rem * 200.)
                            .child(self.render_detail(window, cx)),
                    ),
            )
    }
}

/// What a choice in an album row's Add to Playlist menu runs: the album's
/// songs may still be loading when the menu opens.
fn add_album_command(playlist: Option<&PlaylistId>) -> Box<dyn gpui_kit::Action> {
    Box::new(AddAlbumToPlaylist {
        playlist: playlist.cloned(),
    })
}

/// Brings `this` up to date with the server, through the saved library
/// when there is one. Errors are sentences for the user.
async fn run_sync(
    this: &WeakEntity<Library>,
    session: &session::Session,
    mode: Mode,
    cx: &mut AsyncApp,
) -> Result<(), SharedString> {
    let client = session.client().clone();
    let describe = |error: client::Error| SharedString::from(session::describe(&error));
    let describe_catalog =
        |error: catalog::Error| SharedString::from(session::describe_catalog(&error));
    let Some(catalog) = session.catalog().cloned() else {
        let albums = cx
            .background_spawn(async move { client.albums_by_name().await })
            .await
            .map_err(describe)?;
        this.update(cx, |this, cx| {
            this.set_albums(albums, cx);
            cx.emit(LibraryEvent::AlbumsSynced);
        })
        .ok();
        return Ok(());
    };

    let saved = |cx: &mut AsyncApp| {
        let catalog = catalog.clone();
        cx.background_spawn(async move { catalog.albums() })
    };
    let mut shown = this
        .read_with(cx, |this, _| this.loaded_albums().is_some())
        .unwrap_or_default();
    if !shown {
        // A copy that can't be read is replaced by the sync below.
        if let Ok(albums) = saved(cx).await
            && !albums.is_empty()
        {
            this.update(cx, |this, cx| this.set_albums(albums, cx)).ok();
            shown = true;
        }
    }

    let outcome = {
        let (catalog, client) = (catalog.clone(), client.clone());
        cx.background_spawn(async move { catalog::sync_albums(&catalog, &client, mode).await })
            .await
            .map_err(describe_catalog)?
    };
    let albums = match outcome {
        Outcome::Synced { .. } | Outcome::Unsettled { .. } => {
            Some(saved(cx).await.map_err(describe_catalog)?)
        }
        Outcome::UpToDate if !shown => Some(saved(cx).await.map_err(describe_catalog)?),
        // The CLI may be filling an empty copy; ask the server directly.
        Outcome::Busy if !shown => {
            let client = client.clone();
            Some(
                cx.background_spawn(async move { client.albums_by_name().await })
                    .await
                    .map_err(describe)?,
            )
        }
        Outcome::UpToDate | Outcome::Busy => None,
    };
    if let Some(albums) = albums {
        this.update(cx, |this, cx| {
            this.set_albums(albums, cx);
            if matches!(outcome, Outcome::Synced { .. } | Outcome::Unsettled { .. }) {
                cx.emit(LibraryEvent::AlbumsSynced);
            }
        })
        .ok();
    }

    cx.background_spawn(async move { catalog::sync_songs(&catalog, &client, mode).await })
        .await
        .map_err(describe_catalog)?;
    Ok(())
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
        .child(h_flex().key_context(CONTROL_CONTEXT).child(retry))
        .into_any_element()
}

#[cfg(test)]
mod tests;
