use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::Snackbar;
use hiphile_core::Shown;
use hiphile_core::artists::{
    ArtistDetail, ArtistList, ArtistListStatus, ArtistOrder, ArtistPage, ArtistSection,
};
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::Location;
use next_host::{Pending, Scope, follow};
use next_ui::card_grid::{GridMotion, columns_for, step};
use next_ui::context_menu::ContextMenu;
use next_ui::keymap::{KeyBinding, Keystroke, bind_keys, is_bound};
use next_ui::list_detail::{Split, list_detail};
use next_ui::now_playing::NowPlaying;
use next_ui::row_list::{ROW_HEIGHT, Selection, scroll_to_row};
use next_ui::selection::KeysShown;
use next_ui::sort_menu::{SortOption, sort_menu};
use next_ui::{AlbumAction, Command, Screen, ScreenContext};
use playback::Placement;
use subsonic::{Album, AlbumId, Artist, ArtistId, Song, SongId};

mod list;
mod menus;
mod page;

/// Where the keys of [`ArtistsCommand`] work on the list.
pub const ARTISTS_CONTEXT: &str = "Artists";
/// Where they work on the page of the artist on show.
pub const DETAIL_CONTEXT: &str = "ArtistDetail";

/// What the keys do on the screen, on top of what the list does itself. On the list, they act on
/// the artist on show; on the page, on what is selected there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtistsCommand {
    /// The list: on to the page. The page: play the song from it, or show the album or the
    /// artist.
    Open,
    Play,
    PlayNext,
    AddToQueue,
    AddToPlaylist,
    ToggleFavorite,
    GoToAlbum,
    GoToArtist,
    /// From the page back to the list.
    Back,
    /// Left at the start of a row goes back to the list too.
    Move(GridMotion),
}

/// Adds the keys of the screen to this thread's keymap, once.
fn bind_artists_keys() {
    if is_bound::<ArtistsCommand>() {
        return;
    }
    use ArtistsCommand::*;
    let enter = || Keystroke::named(NamedKey::Enter);
    let on_either = [
        (enter().with_primary(), Play),
        (enter().with_alt(), PlayNext),
        (enter().with_shift(), AddToQueue),
        (Keystroke::primary('p'), AddToPlaylist),
        (Keystroke::primary('d'), ToggleFavorite),
    ];
    let on_list = on_either
        .clone()
        .into_iter()
        .chain([(Keystroke::named(NamedKey::ArrowRight), Open)])
        .map(|(stroke, command)| KeyBinding::new(stroke, command, ARTISTS_CONTEXT));
    let moves = GridMotion::strokes().map(|(stroke, motion)| (stroke, Move(motion)));
    let on_page = on_either
        .into_iter()
        .chain([
            (enter(), Open),
            (Keystroke::named(NamedKey::Escape), Back),
            (Keystroke::primary('l'), GoToAlbum),
            (Keystroke::primary('a').with_shift(), GoToArtist),
        ])
        .chain(moves)
        .map(|(stroke, command)| KeyBinding::new(stroke, command, DETAIL_CONTEXT));
    bind_keys(on_list.chain(on_page));
}

/// What can be selected on an artist's page, by identity, so that the selection outlasts a reload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Item {
    Song(SongId),
    Album(AlbumId),
    Artist(ArtistId),
}

/// The parts of a page that a selection moves through, in their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Songs,
    Albums,
    Similar,
}

/// Where the page's parts are, as the page lays them out, which brings what is selected into view.
pub(crate) struct PageLayout {
    pub(crate) width: Signal<f32>,
    pub(crate) height: Signal<f32>,
    pub(crate) scrolled: Signal<f32>,
    /// Where the albums' cards start on the page.
    pub(crate) grid_top: Signal<f32>,
    pub(crate) album_row: Signal<f32>,
    pub(crate) columns: Memo<usize>,
    /// The headings of the parts, by [`Part`].
    pub(crate) headings: [NodeRef; 3],
    /// The songs and the similar artists on show. The albums have places of their own.
    pub(crate) nodes: RefCell<HashMap<Item, NodeRef>>,
}

impl PageLayout {
    fn new() -> Self {
        let width = signal(0.0f32);
        Self {
            width,
            height: signal(0.0),
            scrolled: signal(0.0),
            grid_top: signal(0.0),
            album_row: signal(0.0),
            columns: memo(move || columns_for(width.get())),
            headings: [node_ref(), node_ref(), node_ref()],
            nodes: RefCell::default(),
        }
    }

    pub(crate) fn heading(&self, part: Part) -> NodeRef {
        self.headings[part as usize]
    }

    /// Keeps `node` as where `item` is, until the current owner is gone.
    pub(crate) fn keep(self: &Rc<Self>, item: Item, node: NodeRef) {
        self.nodes.borrow_mut().insert(item.clone(), node);
        let layout = Rc::downgrade(self);
        on_cleanup(move || {
            let Some(layout) = layout.upgrade() else {
                return;
            };
            let mut nodes = layout.nodes.borrow_mut();
            if nodes.get(&item) == Some(&node) {
                nodes.remove(&item);
            }
        });
    }
}

/// What the screen and its parts share. What is on show follows `status` and `page`, which follow
/// the core; the artist on show is where the window is.
pub(crate) struct View {
    pub(crate) cx: ScreenContext,
    /// The artist the window is at, `Some(None)` for none yet, and `None` once it is elsewhere.
    location: Memo<Option<Option<ArtistId>>>,
    pub(crate) list: ArtistList,
    pub(crate) detail: ArtistDetail,
    pub(crate) status: Signal<ArtistListStatus>,
    pub(crate) page: Signal<Option<ArtistPage>>,
    pub(crate) filter: Signal<String>,
    /// The list's, whose cursor is the artist on show.
    pub(crate) selection: Selection<ArtistId>,
    /// What is selected on the page.
    pub(crate) item: Signal<Option<Item>>,
    pub(crate) top_songs_expanded: Signal<bool>,
    /// The artist whose songs are being had for an action.
    pub(crate) fetching: Signal<Option<ArtistId>>,
    action: RefCell<Option<Pending>>,
    pub(crate) menu: ContextMenu,
    pub(crate) now_playing: Memo<Option<NowPlaying>>,
    pub(crate) list_node: NodeRef,
    pub(crate) page_node: NodeRef,
    pub(crate) page_focused: Signal<bool>,
    pub(crate) page_keys: KeysShown,
    pub(crate) layout: Rc<PageLayout>,
    /// The artist the page was last made to show.
    shown_artist: RefCell<Option<ArtistId>>,
    /// The list as last taken, which a new one is compared with.
    listed: RefCell<Option<Shown<Artist>>>,
    /// Whether the list comes to the artist on show once the refresh on its way brings them.
    revealing: Cell<bool>,
    /// Whether the list comes to the artist on show once it is laid out.
    scroll_list: Cell<bool>,
    /// Whether the list takes the focus once it is there.
    focus_list: Cell<bool>,
    scope: Scope,
}

/// The artists, beside the page of the one where the window is, which `location` says. Choosing
/// another sends [`Command::Show`], and moving through the list [`Command::ShowInPlace`]. The list
/// is as wide as `split` says.
pub fn artists(location: Signal<Location>, split: Split, cx: &ScreenContext) -> Screen {
    artists_view(location, split, cx).1
}

pub(crate) fn artists_view(
    location: Signal<Location>,
    split: Split,
    cx: &ScreenContext,
) -> (Rc<View>, Screen) {
    bind_artists_keys();
    let list = cx.workspace.artists().clone();
    let detail = cx.workspace.artist_detail().clone();
    list.open();
    let status = follow(list.subscribe());
    let page = follow(detail.subscribe());
    let filter = signal(untrack(|| status.with(|status| status.query().to_owned())));
    let view = Rc::new(View {
        cx: cx.clone(),
        location: memo(move || {
            location.with(|location| match location {
                Location::Artists(artist) => Some(artist.clone()),
                _ => None,
            })
        }),
        list,
        detail,
        status,
        page,
        filter,
        selection: Selection::new(),
        item: signal(None),
        top_songs_expanded: signal(false),
        fetching: signal(None),
        action: RefCell::default(),
        menu: ContextMenu::new(cx.overlays),
        now_playing: cx.known.now_playing(),
        list_node: node_ref(),
        page_node: node_ref(),
        page_focused: signal(false),
        page_keys: KeysShown::new(),
        layout: Rc::new(PageLayout::new()),
        shown_artist: RefCell::default(),
        listed: RefCell::default(),
        revealing: Cell::new(false),
        scroll_list: Cell::new(true),
        focus_list: Cell::new(false),
        scope: Scope::here(),
    });
    effect({
        let list = view.list.clone();
        move || {
            let query = filter.get();
            untrack(|| list.set_query(&query));
        }
    });
    effect({
        let view = view.clone();
        move || {
            status.with(|_| ());
            untrack(|| view.follow_list());
        }
    });
    effect({
        let view = view.clone();
        move || {
            if let Some(artist) = view.location.get() {
                untrack(|| view.go_to(artist));
            }
        }
    });
    effect({
        let view = view.clone();
        move || {
            let cursor = view.selection.cursor();
            untrack(|| view.follow_cursor(cursor));
        }
    });
    effect({
        let view = view.clone();
        move || {
            if view.list_node.get().is_some() && view.focus_list.take() {
                view.list_node.focus();
            }
        }
    });
    view.focus_list();
    let body =
        list_detail(split, list::list_pane(&view), page::page_pane(&view)).child(view.menu.view());
    let screen = Screen::new("Artists", body)
        .action(sort(&view))
        .home(view.list_node);
    (view, screen)
}

/// The order of the list and which way it runs.
fn sort(view: &Rc<View>) -> Div {
    let status = view.status;
    let sort = memo(move || status.with(ArtistListStatus::sort));
    let order = |label: &str, order: ArtistOrder| {
        let list = view.list.clone();
        SortOption::new(
            label,
            move || sort.get().order == order,
            move || list.choose_order(order),
        )
    };
    let direction = |label: &str, is_descending: bool| {
        let list = view.list.clone();
        SortOption::new(
            label,
            move || sort.get().is_descending == is_descending,
            move || list.choose_direction(is_descending),
        )
    };
    sort_menu(
        view.cx.overlays,
        move || sort.get().is_descending,
        vec![
            vec![
                order("Name", ArtistOrder::Name),
                order("Recently Added", ArtistOrder::Added),
                order("Release Date", ArtistOrder::Released),
            ],
            vec![direction("Ascending", false), direction("Descending", true)],
        ],
    )
}

impl View {
    pub(crate) fn send(&self, command: Command) {
        self.cx.commands.send(command);
    }

    fn artist_at_location(&self) -> Option<ArtistId> {
        untrack(|| self.location.get()).flatten()
    }

    /// Shows what the window is at: the artist, or one to start with when it is at none.
    fn go_to(&self, artist: Option<ArtistId>) {
        let Some(artist) = artist else {
            self.pick();
            return;
        };
        if untrack(|| self.selection.cursor()).as_ref() == Some(&artist) {
            self.show_artist(&artist);
        } else {
            self.reveal(artist);
        }
    }

    /// The artist the page showed last, or else the first of the list.
    fn pick(&self) {
        let shown = self.detail.page().map(|page| page.id);
        if let Some(artist) = shown.or_else(|| self.first_visible()) {
            self.selection.select(artist);
        }
    }

    fn first_visible(&self) -> Option<ArtistId> {
        untrack(|| {
            self.status.with(|status| {
                let first = status.shown()?.get(0)?;
                Some(first.id().clone())
            })
        })
    }

    /// The window goes where the list's cursor went, as the keys or a click took it.
    fn follow_cursor(&self, cursor: Option<ArtistId>) {
        let Some(cursor) = cursor else {
            return;
        };
        if self.artist_at_location().as_ref() != Some(&cursor) {
            self.send(Command::ShowInPlace(Location::Artists(Some(cursor))));
        }
    }

    /// Clears a filter that hides `artist`, selects them and brings them into view. One the list
    /// does not have, as one who is only on songs, shows on the page all the same.
    fn reveal(&self, artist: ArtistId) {
        let is_hidden = untrack(|| {
            self.status.with(|status| {
                status.shown().is_some_and(|shown| {
                    let position = shown.items().iter().position(|it| it.id() == &artist);
                    position.is_some_and(|position| !shown.visible().contains(&position))
                })
            })
        });
        if is_hidden {
            self.filter.set(String::new());
        }
        self.selection.select(artist.clone());
        self.show_artist(&artist);
        self.revealing.set(untrack(|| {
            self.status.with(ArtistListStatus::is_refreshing)
        }));
        self.scroll_list_to_cursor();
        self.focus_list();
    }

    /// Makes the page show `artist`, from the top, with nothing on it selected.
    fn show_artist(&self, artist: &ArtistId) {
        self.revealing.set(false);
        if self.shown_artist.borrow().as_ref() == Some(artist) {
            return;
        }
        self.shown_artist.replace(Some(artist.clone()));
        let listed = untrack(|| {
            self.status.with(|status| {
                let shown = status.shown()?;
                shown.items().iter().find(|it| it.id() == artist).cloned()
            })
        });
        self.detail.show(artist, listed.as_ref());
        self.item.set(None);
        self.top_songs_expanded.set(false);
        self.page_node.scroll_to(0.0, 0.0);
    }

    /// The selection stays, also on an artist the list does not have, unless the artist was on
    /// it and has gone. Artists in a new order keep the one on show in view.
    fn follow_list(&self) {
        let (shown, is_refreshing) = untrack(|| {
            self.status
                .with(|status| (status.shown().cloned(), status.is_refreshing()))
        });
        let old = self.listed.replace(shown.clone());
        if old != shown
            && let Some(shown) = &shown
        {
            let is_same_list = old
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old.items(), shown.items()));
            if is_same_list {
                self.scroll_list_to_cursor();
            } else {
                self.keep_selection(old.as_ref(), shown);
                if self.revealing.get() {
                    self.scroll_list_to_cursor();
                }
            }
        }
        if !is_refreshing {
            self.revealing.set(false);
        }
    }

    fn keep_selection(&self, old: Option<&Shown<Artist>>, new: &Shown<Artist>) {
        let on_show = self.artist_at_location();
        let is_listed = |list: &[Artist], id: &ArtistId| list.iter().any(|it| it.id() == id);
        let has_gone = on_show.as_ref().is_some_and(|artist| {
            old.is_some_and(|old| is_listed(old.items(), artist)) && !is_listed(new.items(), artist)
        });
        if has_gone {
            self.detail.close();
            self.shown_artist.replace(None);
            self.selection.clear();
        }
        if on_show.is_none() || has_gone {
            match new.get(0) {
                Some(first) => self.selection.select(first.id().clone()),
                None => self.send(Command::ShowInPlace(Location::Artists(None))),
            }
        }
    }

    /// Scrolls the list to the artist on show, now or once it is laid out.
    fn scroll_list_to_cursor(&self) {
        let Some(cursor) = untrack(|| self.selection.cursor()) else {
            return;
        };
        let position = untrack(|| {
            self.status
                .with(|status| status.shown()?.position(|artist| artist.id() == &cursor))
        });
        let Some(position) = position else {
            return;
        };
        if !scroll_to_row(self.list_node, position, ROW_HEIGHT) {
            self.scroll_list.set(true);
        }
    }

    /// The list has been laid out, which it may have had to be to come to the artist on show.
    pub(crate) fn list_laid_out(&self) {
        if self.scroll_list.take() {
            self.scroll_list_to_cursor();
        }
    }

    pub(crate) fn focus_list(&self) {
        if untrack(|| self.list_node.get()).is_some() {
            self.list_node.focus();
        } else {
            self.focus_list.set(true);
        }
    }

    /// The artist on show, as the list has them or as the page does once it has their name.
    pub(crate) fn artist_on_show(&self) -> Option<Artist> {
        self.whole_artist(&self.artist_at_location()?)
    }

    /// `artist` as the list has them, or as the page does once it has their name, as a favorite
    /// needs one.
    pub(crate) fn whole_artist(&self, artist: &ArtistId) -> Option<Artist> {
        untrack(|| {
            let listed = self.status.with(|status| {
                let shown = status.shown()?;
                shown.items().iter().find(|it| it.id() == artist).cloned()
            });
            listed.or_else(|| {
                self.page.with(|page| {
                    page.as_ref()
                        .filter(|page| page.id == *artist)
                        .and_then(ArtistPage::as_artist)
                })
            })
        })
    }

    /// Has every song of `artist`, album by album as their page shows them, and sends what `then`
    /// makes of them. It replaces an action still waiting for its songs: the latest one counts.
    pub(crate) fn act_on_artist(
        &self,
        artist: ArtistId,
        then: impl FnOnce(Arc<[Song]>) -> Option<Command> + 'static,
    ) {
        self.cancel_action();
        let songs = self.detail.songs(&artist);
        self.fetching.set(Some(artist));
        let (fetching, commands, snackbars) =
            (self.fetching, self.cx.commands.clone(), self.cx.snackbars);
        let pending = self.scope.on_ui(songs, move |songs| {
            fetching.set(None);
            match songs {
                Ok(songs) => {
                    if let Some(command) = then(songs.into()) {
                        commands.send(command);
                    }
                }
                Err(error) => snackbars.show(Snackbar::new(error).long()),
            }
        });
        *self.action.borrow_mut() = Some(pending);
    }

    fn cancel_action(&self) {
        if let Some(before) = self.action.borrow_mut().take() {
            before.cancel();
        }
        self.fetching.set(None);
    }

    pub(crate) fn act_with(&self, artist: ArtistId, action: AlbumAction) {
        self.act_on_artist(artist, move |songs| action.command(songs));
    }

    pub(crate) fn shuffle(&self, artist: ArtistId) {
        self.act_on_artist(artist, |songs| {
            (!songs.is_empty()).then_some(Command::Shuffle(songs))
        });
    }

    fn act_on_album(&self, album: AlbumId, action: AlbumAction) {
        self.cancel_action();
        self.send(Command::ActOnAlbum { album, action });
    }

    /// The page of the artist on show, as it shows.
    pub(crate) fn page_on_show(&self) -> Option<ArtistPage> {
        let artist = self.artist_at_location()?;
        untrack(|| self.page.get()).filter(|page| page.id == artist)
    }

    /// What can be selected on the page, row by row from the top, and the part each row is in.
    fn rows(&self) -> Vec<(Part, Vec<Item>)> {
        let Some(page) = self.page_on_show() else {
            return Vec::new();
        };
        let columns = untrack(|| self.layout.columns.get()).max(1);
        let expanded = untrack(|| self.top_songs_expanded.get());
        let mut rows = Vec::new();
        for section in page.sections() {
            match section {
                ArtistSection::TopSongs | ArtistSection::Songs => {
                    let songs = page.top_songs_on_show(expanded);
                    rows.extend(
                        songs
                            .iter()
                            .map(|song| (Part::Songs, vec![Item::Song(song.id().clone())])),
                    );
                }
                ArtistSection::Albums => {
                    let albums = page.albums.items();
                    rows.extend(albums.chunks(columns).map(|row| {
                        let row = row.iter().map(|album| Item::Album(album.id().clone()));
                        (Part::Albums, row.collect())
                    }));
                }
                ArtistSection::Similar => {
                    let similar = page.similar.items();
                    rows.extend(similar.chunks(columns).map(|row| {
                        let row = row.iter().map(|artist| Item::Artist(artist.id().clone()));
                        (Part::Similar, row.collect())
                    }));
                }
            }
        }
        rows
    }

    /// The selection, while it shows. Show less or a reload can take it off the page, and a
    /// command must not act on what cannot be seen.
    pub(crate) fn item_on_show(&self) -> Option<Item> {
        let item = untrack(|| self.item.get())?;
        let rows = self.rows();
        rows.iter()
            .any(|(_, row)| row.contains(&item))
            .then_some(item)
    }

    /// Whether the keys are on `item` while the page's focus shows: the selected one, or the first
    /// with none.
    pub(crate) fn has_keys_on(&self, item: &Item) -> bool {
        if !self.page_keys.get() || !self.page_focused.get() {
            return false;
        }
        match self.item.get() {
            Some(selected) => selected == *item,
            None => self
                .rows()
                .first()
                .and_then(|(_, row)| row.first())
                .is_some_and(|first| first == item),
        }
    }

    fn is_at_row_start(&self) -> bool {
        let Some(item) = self.item_on_show() else {
            return true;
        };
        self.rows()
            .iter()
            .any(|(_, row)| row.first() == Some(&item))
    }

    fn move_item(&self, motion: GridMotion) {
        let rows = self.rows();
        let items: Vec<Vec<Item>> = rows.iter().map(|(_, row)| row.clone()).collect();
        let current = untrack(|| self.item.get());
        let Some(to) = step(&items, current.as_ref(), motion) else {
            return;
        };
        let at = rows.iter().position(|(_, row)| row.contains(&to));
        // Going up to a part brings its heading into view too
        let heading = at.and_then(|at| {
            let part = rows[at].0;
            let starts_part = at == 0 || rows[at - 1].0 != part;
            let is_up = matches!(
                motion,
                GridMotion::Up | GridMotion::PageUp | GridMotion::First
            );
            (starts_part && is_up).then_some(part)
        });
        self.item.set(Some(to.clone()));
        self.reveal_item(&to, heading);
    }

    /// Scrolls the page just far enough to show `item`, and the heading of `with` above it.
    fn reveal_item(&self, item: &Item, with: Option<Part>) {
        let node = self.page_node;
        let layout = &self.layout;
        let span = match item {
            Item::Album(album) => self.page_on_show().and_then(|page| {
                let position = page.albums.items().iter().position(|it| it.id() == album)?;
                let columns = untrack(|| layout.columns.get()).max(1);
                let row = untrack(|| layout.album_row.get());
                let top = untrack(|| layout.grid_top.get()) + (position / columns) as f32 * row;
                Some((top, top + row))
            }),
            _ => {
                let shown = layout.nodes.borrow().get(item).copied();
                shown.and_then(|shown| {
                    let top = shown.offset_in(node)?.y;
                    Some((top, top + shown.logical_rect()?.height))
                })
            }
        };
        let Some((top, bottom)) = span else {
            return;
        };
        let top = with
            .and_then(|part| layout.heading(part).offset_in(node))
            .map_or(top, |heading| heading.y.min(top));
        let Some(rect) = node.logical_rect() else {
            return;
        };
        let at = node.scroll_offset();
        let y = if top < at.y {
            top
        } else if bottom > at.y + rect.height {
            (bottom - rect.height).min(top)
        } else {
            return;
        };
        node.scroll_to(at.x, y);
    }

    pub(crate) fn select_item(&self, item: Item) {
        self.item.set(Some(item));
        self.page_node.focus();
    }

    /// Moves the focus to the page, selecting the first thing on it if nothing is.
    pub(crate) fn open_page(&self) {
        if self.item_on_show().is_none() {
            self.move_item(GridMotion::First);
        }
        self.page_node.focus();
    }

    pub(crate) fn top_song(&self, song: &SongId) -> Option<(usize, Song)> {
        let page = self.page_on_show()?;
        let songs = page.top_songs.items();
        let position = songs.iter().position(|it| it.id() == song)?;
        Some((position, songs[position].clone()))
    }

    pub(crate) fn album(&self, album: &AlbumId) -> Option<Album> {
        let page = self.page_on_show()?;
        page.albums
            .items()
            .iter()
            .find(|it| it.id() == album)
            .cloned()
    }

    pub(crate) fn similar_artist(&self, artist: &ArtistId) -> Option<Artist> {
        let page = self.page_on_show()?;
        page.similar
            .items()
            .iter()
            .find(|it| it.id() == artist)
            .cloned()
    }

    /// Plays the top songs on show from the one at `position`.
    pub(crate) fn play_top_songs_from(&self, position: usize) -> Option<Command> {
        let page = self.page_on_show()?;
        let songs = page.top_songs_on_show(untrack(|| self.top_songs_expanded.get()));
        (position < songs.len()).then(|| Command::Play {
            songs: songs.into(),
            start: Some(position),
        })
    }

    fn open_item(&self) {
        let command = match self.item_on_show() {
            Some(Item::Song(song)) => self
                .top_song(&song)
                .and_then(|(position, _)| self.play_top_songs_from(position)),
            Some(Item::Album(album)) => Some(Command::Show(Location::Albums(Some(album)))),
            Some(Item::Artist(artist)) => Some(Command::Show(Location::Artists(Some(artist)))),
            None => None,
        };
        if let Some(command) = command {
            self.send(command);
        }
    }

    fn act_on_item(&self, action: AlbumAction) {
        match self.item_on_show() {
            Some(Item::Song(song)) => {
                let command = self
                    .top_song(&song)
                    .and_then(|(_, song)| action.command(Arc::from([song])));
                if let Some(command) = command {
                    self.send(command);
                }
            }
            Some(Item::Album(album)) => self.act_on_album(album, action),
            Some(Item::Artist(artist)) => self.act_with(artist, action),
            None => {}
        }
    }

    fn play_item(&self) {
        if matches!(self.item_on_show(), Some(Item::Song(_))) {
            self.open_item();
        } else {
            self.act_on_item(AlbumAction::Play);
        }
    }

    fn favorite_of_item(&self) -> Option<Favorite> {
        match self.item_on_show()? {
            Item::Song(song) => self.top_song(&song).map(|(_, song)| Favorite::Song(song)),
            Item::Album(album) => self.album(&album).map(Favorite::Album),
            Item::Artist(artist) => self.similar_artist(&artist).map(Favorite::Artist),
        }
    }

    fn album_of_item(&self) -> Option<AlbumId> {
        match self.item_on_show()? {
            Item::Album(album) => Some(album),
            Item::Song(song) => self.top_song(&song)?.1.album_id().cloned(),
            Item::Artist(_) => None,
        }
    }

    fn artist_of_item(&self) -> Option<ArtistId> {
        match self.item_on_show()? {
            Item::Artist(artist) => Some(artist),
            Item::Album(album) => self.album(&album)?.artist_id().cloned(),
            Item::Song(song) => self.top_song(&song)?.1.artist_id().cloned(),
        }
    }

    fn is_known(&self, favorite: &Favorite) -> bool {
        self.cx.known.is_favorite(&favorite.target()).is_some()
    }

    pub(crate) fn is_enabled_on_list(&self, command: ArtistsCommand) -> bool {
        use ArtistsCommand::*;
        let artist = self.artist_at_location();
        match command {
            Open | Play | PlayNext | AddToQueue | AddToPlaylist => artist.is_some(),
            ToggleFavorite => self
                .artist_on_show()
                .is_some_and(|artist| self.is_known(&Favorite::Artist(artist))),
            GoToAlbum | GoToArtist | Back | Move(_) => false,
        }
    }

    pub(crate) fn run_on_list(&self, command: ArtistsCommand) {
        use ArtistsCommand::*;
        let Some(artist) = self.artist_at_location() else {
            return;
        };
        match command {
            Open => self.open_page(),
            Play => self.act_with(artist, AlbumAction::Play),
            PlayNext => self.act_with(artist, AlbumAction::Enqueue(Placement::Next)),
            AddToQueue => self.act_with(artist, AlbumAction::Enqueue(Placement::Last)),
            AddToPlaylist => self.act_with(artist, AlbumAction::ChoosePlaylist),
            ToggleFavorite => {
                if let Some(artist) = self.artist_on_show() {
                    self.send(Command::ToggleFavorite(Favorite::Artist(artist)));
                }
            }
            GoToAlbum | GoToArtist | Back | Move(_) => {}
        }
    }

    pub(crate) fn is_enabled_on_page(&self, command: ArtistsCommand) -> bool {
        use ArtistsCommand::*;
        match command {
            Move(_) | Back => true,
            Open | Play | PlayNext | AddToQueue | AddToPlaylist => self.item_on_show().is_some(),
            ToggleFavorite => self
                .favorite_of_item()
                .is_some_and(|favorite| self.is_known(&favorite)),
            GoToAlbum => self.album_of_item().is_some(),
            GoToArtist => self.artist_of_item().is_some(),
        }
    }

    pub(crate) fn run_on_page(&self, command: ArtistsCommand) {
        use ArtistsCommand::*;
        self.page_keys.used_a_key();
        match command {
            Move(GridMotion::Left) if self.is_at_row_start() => self.focus_list(),
            Move(motion) => self.move_item(motion),
            Back => self.focus_list(),
            Open => self.open_item(),
            Play => self.play_item(),
            PlayNext => self.enqueue_item(Placement::Next),
            AddToQueue => self.enqueue_item(Placement::Last),
            AddToPlaylist => self.act_on_item(AlbumAction::ChoosePlaylist),
            ToggleFavorite => {
                if let Some(favorite) = self.favorite_of_item() {
                    self.send(Command::ToggleFavorite(favorite));
                }
            }
            GoToAlbum => {
                if let Some(album) = self.album_of_item() {
                    self.send(Command::Show(Location::Albums(Some(album))));
                }
            }
            GoToArtist => {
                if let Some(artist) = self.artist_of_item() {
                    self.send(Command::Show(Location::Artists(Some(artist))));
                }
            }
        }
    }

    fn enqueue_item(&self, placement: Placement) {
        self.act_on_item(AlbumAction::Enqueue(placement));
    }
}

#[cfg(test)]
mod tests;
