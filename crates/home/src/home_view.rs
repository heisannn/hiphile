use std::ops::Range;
use std::sync::Arc;

use actions::{EnqueueSongs, Placement, PlaySongs, ShowAlbum};
use futures::future::join_all;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _,
    button::Button, h_flex, list::ListItem, menu::ContextMenuExt as _,
    notification::NotificationType, scroll::Scrollbar, spinner::Spinner, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, Role,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, prelude::FluentBuilder as _,
};
use subsonic::{Album, AlbumId, Song, SongId};
use ui::list_navigation::{
    SelectFirst, SelectLast, SelectNext, SelectPageDown, SelectPageUp, SelectPrevious,
};
use ui::{CoverArt, format_duration};

use crate::layout::{Item, Motion, columns_for, step};
use crate::recent::most_recent;
use crate::{
    AddToQueue, CONTROL_CONTEXT, HOME_CONTEXT, Open, Play, PlayNext, SelectLeft, SelectRight,
    ShowInLibrary,
};

/// How many of the songs played last show.
const RECENT_SONGS: usize = 8;
/// How many rows of the albums added last show.
const NEW_ALBUM_ROWS: usize = 2;
/// How many of the albums added last to fetch, for the widest window.
const NEW_ALBUMS: u32 = 24;
/// Pixel size requested for an album card's cover, the same as the library's
/// album cover, so that opening the album needs no second image.
const CARD_COVER_PIXELS: u32 = 512;
/// Pixel size requested for a song row's cover, the same as the player's.
const ROW_COVER_PIXELS: u32 = 96;
/// The group of an album card, for the hover outline over its cover.
const CARD_GROUP: &str = "home-album-card";

/// The home screen: the songs played last and the albums added last. Owns
/// both lists, the selection among them, and the focus.
///
/// Both lists come from the server, which alone knows every play. A song
/// stays on show while a reload that replaces it is on its way or fails.
pub struct Home {
    session: session::Session,
    focus: FocusHandle,
    scroll: ScrollHandle,
    recent: Shelf<Song>,
    new_albums: Shelf<Album>,
    selection: Option<Item>,
    now_playing: Option<NowPlaying>,
    /// How many album cards fit across, from the last layout.
    columns: usize,
    _recent_task: Task<()>,
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

/// The song the player is on, marked in the list.
#[derive(Clone, PartialEq, Eq)]
struct NowPlaying {
    song: SongId,
    playing: bool,
}

/// One child of the scrolling list, in order. Each is a direct child so that
/// the selection's can be scrolled to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    RecentHeading,
    /// A spinner or a failure.
    RecentStatus,
    /// The song at this position.
    Song(usize),
    NewAlbumsHeading,
    /// A spinner, a failure, or word of there being no albums.
    NewAlbumsStatus,
    /// The albums at these positions.
    AlbumRow(Range<usize>),
}

/// What to do with an album's songs once they have been fetched.
#[derive(Clone, Copy)]
enum AlbumAction {
    Play,
    Enqueue(Placement),
}

impl Home {
    pub fn new(session: session::Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            // Focus changes the selection treatment.
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            session,
            focus,
            scroll: ScrollHandle::new(),
            recent: Shelf::Loading,
            new_albums: Shelf::Loading,
            selection: None,
            now_playing: None,
            columns: 4,
            _recent_task: Task::ready(()),
            _new_albums_task: Task::ready(()),
            _album_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.reload_recently_played(cx);
        this.reload_new_albums(cx);
        this
    }

    /// Asks the server again what was played last, as after a play.
    pub fn reload_recently_played(&mut self, cx: &mut Context<Self>) {
        self.recent.begin();
        let client = self.session.client().clone();
        self._recent_task = cx.spawn(async move |this, cx| {
            let result = fetch_recently_played(&client).await;
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

    /// Asks the server again what was added last, as after a library sync.
    pub fn reload_new_albums(&mut self, cx: &mut Context<Self>) {
        self.new_albums.begin();
        let client = self.session.client().clone();
        self._new_albums_task = cx.spawn(async move |this, cx| {
            let result = client.newest_albums(NEW_ALBUMS).await;
            this.update(cx, |this, cx| {
                this.new_albums
                    .finish(result.map_err(|error| session::describe(&error).into()));
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// Marks `song` in the list, or clears the mark.
    pub fn set_now_playing(&mut self, song: Option<SongId>, playing: bool, cx: &mut Context<Self>) {
        let now_playing = song.map(|song| NowPlaying { song, playing });
        if self.now_playing != now_playing {
            self.now_playing = now_playing;
            cx.notify();
        }
    }

    /// The children of the scrolling list. A song shelf with nothing to show
    /// goes away, as a server with no plays yet has nothing to say about it.
    fn parts(&self) -> Vec<Part> {
        let mut parts = Vec::new();
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
        parts.push(Part::NewAlbumsHeading);
        let albums = self.new_albums.items().len();
        if albums == 0 {
            parts.push(Part::NewAlbumsStatus);
        } else {
            let columns = self.columns.max(1);
            let shown = albums.min(columns * NEW_ALBUM_ROWS);
            parts.extend(
                (0..shown)
                    .step_by(columns)
                    .map(|start| Part::AlbumRow(start..(start + columns).min(shown))),
            );
        }
        parts
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
                Part::AlbumRow(range) => {
                    let albums = self.new_albums.items().get(range.clone())?;
                    let album = |album: &Album| Item::Album(album.id().clone());
                    Some(albums.iter().map(album).collect())
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
                && matches!(parts[ix - 1], Part::RecentHeading | Part::NewAlbumsHeading);
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
            (Part::AlbumRow(range), Item::Album(id)) => self
                .new_albums
                .items()
                .get(range.clone())
                .is_some_and(|albums| albums.iter().any(|album| album.id() == id)),
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
            Some(Item::Album(album)) => {
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
            Some(Item::Album(album)) => self.act_on_album(album, AlbumAction::Play, window, cx),
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
            Some(Item::Album(album)) => {
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

    fn show_in_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let album = match self.selection_on_screen() {
            Some(Item::Album(album)) => Some(album),
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
        let client = self.session.client().clone();
        let catalog = self.session.catalog().cloned();
        self._album_task = cx.spawn_in(window, async move |_, cx| {
            let saved = match catalog {
                Some(catalog) => {
                    let id = album.clone();
                    cx.background_spawn(async move { catalog.album(&id) })
                        .await
                        .ok()
                        .flatten()
                }
                None => None,
            };
            let loaded = match saved {
                Some(saved) => Ok(saved),
                None => client.album(&album).await,
            };
            cx.update(|window, cx| match loaded {
                Ok(loaded) => {
                    let songs: Arc<[Song]> = loaded.into_parts().1.into();
                    if songs.is_empty() {
                        return;
                    }
                    let command: Box<dyn gpui_kit::Action> = match action {
                        AlbumAction::Play => Box::new(PlaySongs::new(songs, 0)),
                        AlbumAction::Enqueue(placement) => {
                            Box::new(EnqueueSongs::new(songs, placement))
                        }
                    };
                    window.dispatch_action(command, cx);
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
            Part::RecentHeading => heading("Recently played").into_any_element(),
            Part::NewAlbumsHeading => heading("New albums").into_any_element(),
            Part::RecentStatus => self.render_recent_status(cx),
            Part::NewAlbumsStatus => self.render_new_albums_status(cx),
            Part::Song(ix) => self.render_song(*ix, window, cx),
            Part::AlbumRow(range) => self.render_album_row(range.clone(), window, cx),
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
        let details: SharedString = [song.artist(), song.album()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
            .into();
        let duration = song.duration().map(format_duration).unwrap_or_default();
        let cover = song
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, ROW_COVER_PIXELS));
        let marker = self
            .now_playing
            .as_ref()
            .filter(|now| now.song == id)
            .map(|now| now.playing);
        let label: SharedString = match marker {
            Some(true) => format!("{title}, playing").into(),
            Some(false) => format!("{title}, paused").into(),
            None => title.clone(),
        };
        let muted_foreground = cx.theme().muted_foreground;
        let primary = cx.theme().primary;
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
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(title))
                            .child(
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(muted_foreground)
                                    .child(details),
                            ),
                    )
                    .child(
                        // A lane of its own, so the durations stay in line.
                        h_flex().w_4().flex_none().justify_end().when_some(
                            marker,
                            |this, playing| {
                                this.child(
                                    Icon::new(IconName::AudioLines).small().text_color(
                                        if playing { primary } else { muted_foreground },
                                    ),
                                )
                            },
                        ),
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
            .context_menu(move |menu, _, _| {
                menu.action_context(focus.clone())
                    .menu("Play", Box::new(Play))
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .when(has_album, |menu| {
                        menu.separator().menu("Show Album", Box::new(ShowInLibrary))
                    })
            })
            .into_any_element()
    }

    fn render_album_row(
        &self,
        range: Range<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let columns = u16::try_from(self.columns.max(1)).unwrap_or(u16::MAX);
        let albums = self.new_albums.items().get(range).unwrap_or_default();
        let cards = albums
            .iter()
            .map(|album| self.render_album_card(album, window, cx))
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
        album: &Album,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = album.id().clone();
        let is_selected = self.selection == Some(Item::Album(id.clone()));
        let focused = self.focus.is_focused(window);
        let name = SharedString::from(album.name().to_owned());
        let artist = album
            .artist()
            .map(|artist| SharedString::from(artist.to_owned()));
        let cover = album
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, CARD_COVER_PIXELS));
        let radius = cx.theme().radius;
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
            .id(ElementId::Name(format!("home-album-{id}").into()))
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
                    // Over the cover, so that showing it moves nothing.
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(radius)
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
                    this.select(Item::Album(id.clone()), window, cx);
                    this.open(window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.select(Item::Album(id.clone()), window, cx);
                }),
            )
            .context_menu(move |menu, _, _| {
                menu.action_context(focus.clone())
                    .menu("Play", Box::new(Play))
                    .menu("Play Next", Box::new(PlayNext))
                    .menu("Add to Queue", Box::new(AddToQueue))
                    .separator()
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
mod tests {
    use gpui_kit::{
        Entity, KeyBinding, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext, actions, px,
        size,
    };
    use serde_json::json;
    use subsonic::Credentials;

    use super::*;

    fn song(id: &str) -> Song {
        serde_json::from_value(json!({
            "id": id, "title": id, "played": "2026-11-03T22:36:15Z", "albumId": "a1",
        }))
        .unwrap()
    }

    fn album(id: &str) -> Album {
        serde_json::from_value(json!({ "id": id, "name": id })).unwrap()
    }

    fn albums(count: usize) -> Vec<Album> {
        (0..count).map(|ix| album(&format!("a{ix}"))).collect()
    }

    /// A home screen whose server cannot be reached.
    fn home(cx: &mut TestAppContext) -> (Entity<Home>, &mut VisualTestContext) {
        // The failing requests wake the test from the network's own thread.
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::init(cx);
        });
        let net = net::Client::new("hiphile-test").unwrap();
        let server = client::Server::new("http://127.0.0.1:9", Credentials::api_key("k")).unwrap();
        let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
        let session = session::Session::new(client::Client::new(net, server), info, None, None);
        cx.add_window_view(|window, cx| Home::new(session, window, cx))
    }

    fn update(
        home: &Entity<Home>,
        cx: &mut VisualTestContext,
        update: impl FnOnce(&mut Home, &mut Context<Home>),
    ) {
        cx.update(|_, cx| home.update(cx, update));
    }

    /// Fills both shelves, cancelling the requests made on opening.
    fn fill(home: &mut Home, songs: &[&str], album_count: usize) {
        home._recent_task = Task::ready(());
        home._new_albums_task = Task::ready(());
        home.recent = Shelf::Loaded(songs.iter().map(|id| song(id)).collect());
        home.new_albums = Shelf::Loaded(albums(album_count).into());
    }

    #[gpui_kit::test]
    fn no_plays_takes_the_song_shelf_away(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            fill(home, &[], 1);
            assert_eq!(home.parts(), [Part::NewAlbumsHeading, Part::AlbumRow(0..1)]);
        });
    }

    #[gpui_kit::test]
    fn loading_and_failing_shelves_keep_their_place(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            home._recent_task = Task::ready(());
            home._new_albums_task = Task::ready(());
            home.recent = Shelf::Loading;
            home.new_albums = Shelf::Failed("offline".into());
            assert_eq!(
                home.parts(),
                [
                    Part::RecentHeading,
                    Part::RecentStatus,
                    Part::NewAlbumsHeading,
                    Part::NewAlbumsStatus,
                ]
            );
        });
    }

    #[gpui_kit::test]
    fn albums_show_in_two_rows_of_as_many_as_fit(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            fill(home, &[], 24);
            home.columns = 3;
            assert_eq!(
                home.parts(),
                [
                    Part::NewAlbumsHeading,
                    Part::AlbumRow(0..3),
                    Part::AlbumRow(3..6),
                ]
            );
            fill(home, &[], 4);
            assert_eq!(
                home.parts(),
                [
                    Part::NewAlbumsHeading,
                    Part::AlbumRow(0..3),
                    Part::AlbumRow(3..4),
                ]
            );
        });
    }

    #[gpui_kit::test]
    fn the_arrow_keys_move_through_the_songs_into_the_albums(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            fill(home, &["s1", "s2"], 4);
        });
        // Room for two cards across.
        cx.simulate_resize(size(px(24. * 16.), px(800.)));
        cx.update(|window, cx| {
            let focus = home.read(cx).focus.clone();
            focus.focus(window, cx);
        });
        cx.run_until_parked();

        let selection = |cx: &mut VisualTestContext| {
            cx.update(|_, cx| home.read(cx).selection.clone()).unwrap()
        };
        cx.simulate_keystrokes("down");
        assert_eq!(selection(cx), Item::Song(SongId::new("s1")));
        cx.simulate_keystrokes("down down");
        assert_eq!(selection(cx), Item::Album(AlbumId::new("a0")));
        cx.simulate_keystrokes("right");
        assert_eq!(selection(cx), Item::Album(AlbumId::new("a1")));
        cx.simulate_keystrokes("down");
        assert_eq!(selection(cx), Item::Album(AlbumId::new("a3")));
        cx.simulate_keystrokes("pageup");
        assert_eq!(selection(cx), Item::Song(SongId::new("s1")));
        cx.simulate_keystrokes("pagedown");
        assert_eq!(selection(cx), Item::Album(AlbumId::new("a2")));
        cx.simulate_keystrokes("home");
        assert_eq!(selection(cx), Item::Song(SongId::new("s1")));
        cx.simulate_keystrokes("end");
        assert_eq!(selection(cx), Item::Album(AlbumId::new("a3")));
    }

    /// Whether `key` presses a focused Retry, which the screen's own use of
    /// the key must not take.
    fn key_presses_retry(key: &str, cx: &mut TestAppContext) -> bool {
        actions!(home_test, [TakenByTheWorkspace]);
        let (home, cx) = home(cx);
        cx.update(|_, cx| {
            cx.bind_keys([KeyBinding::new(
                "space",
                TakenByTheWorkspace,
                Some(HOME_CONTEXT),
            )]);
            cx.on_action(|_: &TakenByTheWorkspace, _| {});
        });
        update(&home, cx, |home, cx| {
            fill(home, &[], 1);
            home.recent = Shelf::Failed("before".into());
            home.selection = Some(Item::Album(AlbumId::new("a0")));
            cx.notify();
        });
        cx.update(|window, cx| {
            let focus = home.read(cx).focus.clone();
            focus.focus(window, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.focus_next(cx));
        cx.run_until_parked();

        // A button acts on the release.
        cx.simulate_keystrokes(key);
        let os = cx.update(|_, cx| cx.platform_info().os);
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse_with_os(key, os).unwrap(),
        });

        // Pressed, it loads again, and whatever that comes to is not this.
        cx.update(
            |_, cx| !matches!(&home.read(cx).recent, Shelf::Failed(error) if error == "before"),
        )
    }

    #[gpui_kit::test]
    fn enter_presses_a_focused_retry(cx: &mut TestAppContext) {
        assert!(key_presses_retry("enter", cx));
    }

    #[gpui_kit::test]
    fn space_presses_a_focused_retry(cx: &mut TestAppContext) {
        assert!(key_presses_retry("space", cx));
    }

    #[gpui_kit::test]
    fn commands_ignore_a_selection_a_narrower_window_took_off_the_screen(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            fill(home, &[], 24);
            home.columns = 7;
            home.selection = Some(Item::Album(AlbumId::new("a13")));
            assert_eq!(home.selection_on_screen(), home.selection);

            home.columns = 2;
            assert_eq!(home.selection_on_screen(), None);
            assert_eq!(home.selection, Some(Item::Album(AlbumId::new("a13"))));

            home.columns = 7;
            assert_eq!(home.selection_on_screen(), home.selection);
        });
    }

    #[gpui_kit::test]
    fn a_reload_in_progress_keeps_what_is_on_show(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, cx| {
            fill(home, &["s1"], 1);
            home.reload_recently_played(cx);
            home.reload_new_albums(cx);
            assert_eq!(home.recent.items().len(), 1);
            assert_eq!(home.new_albums.items().len(), 1);
        });
    }

    #[gpui_kit::test]
    fn an_incomplete_reload_keeps_what_is_on_show(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            fill(home, &["s1", "s2"], 0);
            home.finish_recently_played(Ok(RecentlyPlayed {
                songs: vec![song("s2")],
                complete: false,
            }));
            assert_eq!(home.recent.items().len(), 2);
        });
    }

    #[gpui_kit::test]
    fn an_incomplete_first_load_shows_what_it_found(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            home.recent = Shelf::Loading;
            home.finish_recently_played(Ok(RecentlyPlayed {
                songs: vec![song("s2")],
                complete: false,
            }));
            assert_eq!(home.recent.items().len(), 1);
        });
    }

    #[gpui_kit::test]
    fn the_cards_across_follow_the_width_of_the_screen(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| fill(home, &["s1"], 24));
        let columns = |cx: &mut VisualTestContext, width: f32| {
            cx.simulate_resize(size(px(width), px(800.)));
            cx.run_until_parked();
            cx.update(|_, cx| home.read(cx).columns)
        };

        assert_eq!(columns(cx, 80. * 16.), 7);
        assert_eq!(columns(cx, 46. * 16.), 4);
    }

    #[gpui_kit::test]
    fn a_failed_reload_keeps_what_is_on_show(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            fill(home, &["s1"], 1);
            home.recent.finish(Err("offline".into()));
            assert_eq!(home.recent.items().len(), 1);
        });
    }

    #[gpui_kit::test]
    fn a_failure_with_nothing_on_show_says_so(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            home.recent = Shelf::Loading;
            home.recent.finish(Err("offline".into()));
            assert!(matches!(home.recent, Shelf::Failed(_)));
        });
    }

    #[gpui_kit::test]
    fn a_reload_keeps_the_selection_of_a_song_still_there(cx: &mut TestAppContext) {
        let (home, cx) = home(cx);
        update(&home, cx, |home, _| {
            fill(home, &["s1", "s2"], 0);
            let selection = Some(Item::Song(SongId::new("s2")));
            home.selection = selection.clone();

            home.finish_recently_played(Ok(RecentlyPlayed {
                songs: vec![song("s2"), song("s1")],
                complete: true,
            }));

            assert_eq!(home.selection_on_screen(), selection);
            assert_eq!(home.song_ix(&SongId::new("s2")), Some(0));
        });
    }
}
