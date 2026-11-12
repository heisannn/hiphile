//! The home screen: the albums to try today, the songs played last, the albums played most and the
//! newest ones, on one page that the keys move a selection through.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    IconButtonStyle, MenuItem, Snackbar, indeterminate_circular_progress, styled_icon_button,
};
use anti_webui_md3e::icons;
use hiphile_core::Shelf;
use hiphile_core::favorites::Favorite;
use hiphile_core::home::{AlbumShelf, HomeSection, Item, MAX_COLUMNS, ReloadFailed};
use hiphile_core::navigation::Destination;
use next_host::{follow, notices};
use next_ui::card_grid::{Card, GridMotion, card, card_row, columns_for, step};
use next_ui::context_menu::{ContextMenu, pressed_at};
use next_ui::cover::{CoverKind, cover};
use next_ui::keymap::{KeyBinding, KeyContext, bind_keys, key_context};
use next_ui::link::{album_artist_link, artist_line, song_line};
use next_ui::load_state::{
    HOME, MOST_PLAYED, NEW_ALBUMS, NONE_ON_SERVER, RECENTLY_PLAYED, RECOMMENDATIONS, load_failure,
};
use next_ui::menus::{album_menu, show_in_albums_item, song_menu};
use next_ui::now_playing::{NowPlaying, follow_now_playing, song_row_label};
use next_ui::page_row::{PageRow, page_row};
use next_ui::pane::pane;
use next_ui::row_buttons::RowButtons;
use next_ui::row_cover::row_cover;
use next_ui::row_list::is_context_click;
use next_ui::section::{
    section_failure, section_heading, section_heading_with, section_loading, section_note,
};
use next_ui::song_row::{SongRow, song_words};
use next_ui::{Command, Screen, ScreenContext};
use subsonic::{Album, Song};

use crate::model::{Asked, Intent, Shelves, Target, section_of, section_with, shelf_of, shown};

const CARD_COVER_PIXELS: u32 = 256;
/// Below the last section, which ends in a row of cards or in what stands for one.
const BOTTOM: f32 = 8.0;
/// Where a reload waits, as wide as the button it stands for.
const RELOADING: f32 = 40.0;
const RELOADING_INDICATOR: f32 = 20.0;

/// Where Home's keys work.
pub const HOME_CONTEXT: &str = "Home";

thread_local! {
    static BOUND: Cell<bool> = const { Cell::new(false) };
}

/// Home, which takes the focus when it comes on show.
pub fn home(cx: &ScreenContext) -> Screen {
    bind();
    let node = node_ref();
    Screen::new(Destination::Home.label(), pane(page(cx, node))).home(node)
}

/// Binds Home's keys once on this thread, as the keymap is the thread's.
fn bind() {
    if BOUND.with(|bound| bound.replace(true)) {
        return;
    }
    let bindings = Asked::strokes()
        .into_iter()
        .map(|(stroke, asked)| KeyBinding::new(stroke, asked, HOME_CONTEXT));
    bind_keys(bindings);
}

/// What a selection or a heading is scrolled to by.
#[derive(Clone, Debug, PartialEq)]
enum Place {
    Item(Item),
    Heading(HomeSection),
}

/// Where the items and headings on show are. Cheap to clone.
#[derive(Clone, Default)]
struct Places(Rc<RefCell<Vec<(Place, NodeRef)>>>);

impl Places {
    /// Knows `place` by `node` for as long as the current owner lives.
    fn keep(&self, place: Place, node: NodeRef) {
        self.0.borrow_mut().push((place, node));
        let places = self.clone();
        on_cleanup(move || places.0.borrow_mut().retain(|(_, kept)| *kept != node));
    }

    fn reveal(&self, place: &Place) {
        let node = self
            .0
            .borrow()
            .iter()
            .find(|(kept, _)| kept == place)
            .map(|(_, node)| *node);
        if let Some(node) = node {
            node.scroll_into_view();
        }
    }
}

/// What the parts of the page share. Cheap to clone.
#[derive(Clone)]
struct Page {
    cx: ScreenContext,
    shelves: Memo<Shelves>,
    is_reloadable: Memo<bool>,
    is_recommending: Memo<bool>,
    /// How many cards fit across.
    columns: Memo<usize>,
    /// Kept by identity, so that it stays through a reload and when it is off the screen for a
    /// while.
    selection: Signal<Option<Item>>,
    /// Whether the page itself has the focus, rather than a button on it.
    focused: Signal<bool>,
    node: NodeRef,
    menu: ContextMenu,
    places: Places,
    now_playing: Memo<Option<NowPlaying>>,
}

fn page(cx: &ScreenContext, node: NodeRef) -> Div {
    let workspace = &cx.workspace;
    let recommended = follow(workspace.recommendations().subscribe());
    let shelved = follow(workspace.shelves().subscribe());
    let snackbars = cx.snackbars;
    notices(
        workspace.recommendations().notices(),
        move |failed: ReloadFailed| snackbars.show(Snackbar::new(failed.message()).long()),
    );
    let width = signal(0.0f32);
    let page = Page {
        cx: cx.clone(),
        shelves: memo(move || {
            recommended
                .with(|recommended| shelved.with(|shelved| Shelves::of(recommended, shelved)))
        }),
        is_reloadable: memo(move || recommended.with(|status| status.is_reloadable())),
        is_recommending: memo(move || recommended.with(|status| status.is_recommending())),
        columns: memo(move || columns_for(width.get()).min(MAX_COLUMNS)),
        selection: signal(None),
        focused: signal(false),
        node,
        menu: ContextMenu::new(cx.covers),
        places: Places::default(),
        now_playing: follow_now_playing(workspace),
    };
    let shelves = page.shelves;
    let sections = memo(move || shelves.with(Shelves::sections));
    let common_failure = memo(move || shelves.with(Shelves::common_failure));
    let (focused, node) = (page.focused, page.node);
    let shown = Cell::new(false);
    let view = div()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .pb(BOTTOM)
        .node_ref(node)
        .focusable()
        .access(|a| a.role(Role::ListBox).label("Home"))
        .on_focus_change(move |on| focused.set(on))
        .on_resized(move |w, _| {
            width.set_if_changed(w);
            if !shown.replace(true) {
                node.focus();
            }
        })
        .child(dyn_view({
            let page = page.clone();
            move || {
                let page = page.clone();
                match common_failure.get() {
                    Some(reason) => untrack(|| page.failed_page(&reason).into_view()),
                    None => untrack(|| {
                        for_each(
                            move || sections.get(),
                            |section| *section as u8,
                            move |section| page.section(section),
                        )
                        .into_view()
                    }),
                }
            }
        }))
        .child(page.menu.view());
    let context = {
        let (asked, taken) = (page.clone(), page);
        KeyContext::new(
            HOME_CONTEXT,
            move |key| asked.is_enabled(*key),
            move |key| taken.take(key),
        )
    };
    key_context(view, &context)
}

impl Page {
    fn section(&self, section: HomeSection) -> Div {
        let heading = node_ref();
        self.places.keep(Place::Heading(section), heading);
        let body = match shelf_of(section) {
            Some(shelf) => self.albums(shelf),
            None => self.songs(),
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .node_ref(heading)
                    .child(self.heading(section)),
            )
            .child(body)
    }

    fn heading(&self, section: HomeSection) -> View {
        let title = match section {
            HomeSection::Recommended => return self.recommended_heading(),
            HomeSection::RecentlyPlayed => "Recently played",
            HomeSection::MostPlayed => "Most played",
            HomeSection::NewAlbums => "New albums",
        };
        section_heading(title).into_view()
    }

    /// Recommended, which reloads the albums on show. Until there are some, the shelf itself says
    /// what it waits for or offers to try again.
    fn recommended_heading(&self) -> View {
        let (is_reloadable, is_recommending) = (self.is_reloadable, self.is_recommending);
        let this = self.clone();
        dyn_view(move || {
            if !is_reloadable.get() {
                return untrack(|| section_heading("Recommended"));
            }
            let this = this.clone();
            untrack(move || {
                let reload = dyn_view(move || {
                    if is_recommending.get() {
                        untrack(reloading)
                    } else {
                        untrack(|| this.reload_button())
                    }
                });
                section_heading_with("Recommended", div().flex().child(reload))
            })
        })
    }

    fn reload_button(&self) -> Div {
        let recommendations = self.cx.workspace.recommendations().clone();
        let node = self.node;
        styled_icon_button(icons::REFRESH, IconButtonStyle::standard())
            .access(|a| a.label("Reload Recommendations"))
            .on_click(move |_| {
                recommendations.reload();
                // It gives way to the wait, which would leave nothing with the focus
                node.focus();
            })
    }

    fn albums(&self, shelf: AlbumShelf) -> View {
        let shelves = self.shelves;
        let columns = self.columns;
        let albums = memo(move || shelves.with(|shelves| shelves.albums(shelf).clone()));
        let this = self.clone();
        dyn_view(move || {
            let columns = columns.get();
            match albums.get() {
                Shelf::Loaded(albums) if !albums.is_empty() => {
                    untrack(|| this.cards(shelf, &albums, columns))
                }
                status => untrack(|| this.albums_status(shelf, &status)),
            }
        })
    }

    fn cards(&self, shelf: AlbumShelf, albums: &[Album], columns: usize) -> Div {
        let rows: Vec<Div> = shown(albums, shelf, columns)
            .chunks(columns.max(1))
            .map(|row| {
                // A card is scrolled to by its row, which brings the gap below it along
                let node = node_ref();
                let cards = row
                    .iter()
                    .map(|album| self.album_card(shelf, album, node))
                    .collect();
                card_row(columns, cards).node_ref(node)
            })
            .collect();
        div().flex().flex_col().flex_none().children(rows)
    }

    fn album_card(&self, shelf: AlbumShelf, album: &Album, row: NodeRef) -> Div {
        let item = Item::Album(shelf, album.id().clone());
        self.places.keep(Place::Item(item.clone()), row);
        let url = album.cover_art().map(|cover| {
            self.cx
                .workspace
                .session()
                .cover_art_url(cover, CARD_COVER_PIXELS)
        });
        let line = artist_line(album_artist_link(album, &self.cx.commands), None);
        let look = Card::new(cover(url, CoverKind::Album), album.name())
            .line(line)
            .selected_when(self.is_selected(&item))
            .focused_when(self.is_focused());
        let open = {
            let (this, item) = (self.clone(), item.clone());
            move |e: &PointerEvent| {
                if !is_context_click(e) {
                    this.selection.set(Some(item.clone()));
                    this.run(Intent::Open);
                }
            }
        };
        card(look)
            .on_click(open)
            .on_pointer_down(self.menu_on_press(item))
    }

    fn albums_status(&self, shelf: AlbumShelf, status: &Shelf<Album>) -> Div {
        match (shelf, status) {
            (_, Shelf::Failed(reason)) => self.failure(reason, section_with(shelf)),
            (AlbumShelf::New, Shelf::Loaded(_)) => section_note(NONE_ON_SERVER),
            (AlbumShelf::Recommended, _) => section_loading(RECOMMENDATIONS.loading()),
            (AlbumShelf::MostPlayed, _) => section_loading(MOST_PLAYED.loading()),
            (AlbumShelf::New, _) => section_loading(NEW_ALBUMS.loading()),
        }
    }

    fn songs(&self) -> View {
        let shelves = self.shelves;
        let recent = memo(move || shelves.with(|shelves| shelves.recent.clone()));
        let this = self.clone();
        dyn_view(move || match recent.get() {
            Shelf::Loaded(songs) => untrack(|| {
                let rows: Vec<Div> = (0..songs.len())
                    .map(|ix| this.song_row(&songs, ix))
                    .collect();
                div().flex().flex_col().flex_none().children(rows)
            }),
            Shelf::Failed(reason) => untrack(|| this.failure(&reason, HomeSection::RecentlyPlayed)),
            Shelf::Loading => untrack(|| section_loading(RECENTLY_PLAYED.loading())),
        })
    }

    fn song_row(&self, songs: &Arc<[Song]>, ix: usize) -> Div {
        let song = &songs[ix];
        let item = Item::Song(song.id().clone());
        let node = node_ref();
        self.places.keep(Place::Item(item.clone()), node);
        let state = {
            let (now_playing, id) = (self.now_playing, song.id().clone());
            move || now_playing.with(|now| now.as_ref()?.state_of(&id))
        };
        let skipped = {
            let (known, song) = (self.cx.known, song.clone());
            move || known.is_skipped(&song)
        };
        let label = {
            let (title, state, skipped) = (song.title().to_owned(), state.clone(), skipped.clone());
            move || song_row_label(&title, state(), skipped())
        };
        let groups = {
            let (this, item) = (self.clone(), item.clone());
            move || {
                this.target(&item)
                    .map(|target| this.menu_of(&target))
                    .unwrap_or_default()
            }
        };
        let (commands, known) = (&self.cx.commands, self.cx.known);
        let buttons = RowButtons::new(
            Favorite::Song(song.clone()),
            commands,
            known,
            self.menu,
            groups,
        )
        .selected_when(self.is_selected(&item));
        let words = song_words(song, Some(song_line(song, commands)));
        let content = next_ui::song_row::song_row(
            SongRow::new(song, words, state)
                .cover(row_cover(
                    &self.cx.workspace,
                    song.cover_art(),
                    CoverKind::Album,
                ))
                .skipped_when(skipped)
                .buttons(buttons),
        );
        let row = PageRow::new(content)
            .label(label)
            .selected_when(self.is_selected(&item))
            .focused_when(self.is_focused());
        let click = {
            let (this, item, songs) = (self.clone(), item.clone(), songs.clone());
            move |e: &PointerEvent| {
                if is_context_click(e) {
                    return;
                }
                this.selection.set(Some(item.clone()));
                if e.click_count == 2 {
                    let target = Target::Song {
                        songs: songs.clone(),
                        ix,
                    };
                    this.send(target.play());
                }
            }
        };
        page_row(row)
            .node_ref(node)
            .on_click(click)
            .on_pointer_down(self.menu_on_press(item))
    }

    /// Every shelf that failed for `reason`, said once, with Retry, which tries each again and
    /// leaves the focus on the page as it gives way to the wait.
    fn failed_page(&self, reason: &str) -> Div {
        let shelves = self.cx.workspace.shelves().clone();
        let node = self.node;
        load_failure(HOME.load_failed(), reason.to_owned(), move || {
            for section in [
                HomeSection::Recommended,
                HomeSection::RecentlyPlayed,
                HomeSection::MostPlayed,
                HomeSection::NewAlbums,
            ] {
                shelves.retry(section);
            }
            node.focus();
        })
    }

    /// What a shelf that could not be had says, with Retry, which leaves the focus on the page as
    /// it gives way to the wait.
    fn failure(&self, reason: &str, section: HomeSection) -> Div {
        let shelves = self.cx.workspace.shelves().clone();
        let node = self.node;
        section_failure(reason.to_owned(), move || {
            shelves.retry(section);
            node.focus();
        })
    }

    fn is_selected(&self, item: &Item) -> impl Fn() -> bool + 'static {
        let (selection, item) = (self.selection, item.clone());
        move || selection.with(|selected| selected.as_ref() == Some(&item))
    }

    fn is_focused(&self) -> impl Fn() -> bool + 'static {
        let focused = self.focused;
        move || focused.get()
    }

    /// Opens the menu of `item` when it is pressed with the secondary button, which selects it
    /// first: the keys then act on what the menu is for.
    fn menu_on_press(&self, item: Item) -> impl Fn(&PointerEvent) + 'static {
        let this = self.clone();
        move |e| {
            if !is_context_click(e) {
                return;
            }
            this.selection.set(Some(item.clone()));
            this.node.focus();
            let Some(target) = this.target(&item) else {
                return;
            };
            this.menu.open_at(pressed_at(e), this.menu_of(&target));
        }
    }

    fn menu_of(&self, target: &Target) -> Vec<Vec<MenuItem>> {
        let (commands, known) = (&self.cx.commands, self.cx.known);
        match target {
            Target::Album(album) => {
                let mut groups = album_menu(commands, known, album);
                if let Some(last) = groups.last_mut() {
                    last.push(show_in_albums_item(commands, album.id()));
                }
                groups
            }
            Target::Song { songs, ix } => song_menu(commands, known, &songs[*ix], target.play()),
        }
    }

    /// Whether `asked` does something now. While a button on the page has the focus, Enter and its
    /// modified forms are the button's.
    fn is_enabled(&self, asked: Asked) -> bool {
        if asked.is_for_buttons() && !self.focused.get_untracked() {
            return false;
        }
        match asked {
            Asked::Move(_) => true,
            Asked::Do(intent) => self.command(intent).is_some(),
        }
    }

    fn take(&self, asked: Asked) {
        match asked {
            Asked::Move(motion) => self.move_selection(motion),
            Asked::Do(intent) => self.run(intent),
        }
    }

    fn move_selection(&self, motion: GridMotion) {
        let rows = untrack(|| {
            let columns = self.columns.get();
            self.shelves.with(|shelves| shelves.rows(columns))
        });
        let current = self.selection.get_untracked();
        let Some(to) = step(&rows, current.as_ref(), motion) else {
            return;
        };
        let section = section_of(&to);
        let is_first_row = rows
            .iter()
            .find(|row| row.first().is_some_and(|item| section_of(item) == section))
            .is_some_and(|row| row.contains(&to));
        // Going up a shelf brings its heading into view too
        if is_first_row
            && matches!(
                motion,
                GridMotion::Up | GridMotion::PageUp | GridMotion::First
            )
        {
            self.places.reveal(&Place::Heading(section));
        }
        self.places.reveal(&Place::Item(to.clone()));
        self.selection.set(Some(to));
    }

    fn run(&self, intent: Intent) {
        self.send(self.command(intent));
    }

    /// What doing `intent` to the selection sends, which it does only while the selection shows.
    fn command(&self, intent: Intent) -> Option<Command> {
        let target = self
            .selection
            .get_untracked()
            .and_then(|item| self.target(&item))?;
        let known = self.cx.known;
        // Which way it goes is not known yet
        if intent == Intent::ToggleFavorite
            && untrack(|| known.is_favorite(&target.favorite().target())).is_none()
        {
            return None;
        }
        target.command(intent)
    }

    fn target(&self, item: &Item) -> Option<Target> {
        untrack(|| {
            let columns = self.columns.get();
            self.shelves.with(|shelves| shelves.target(item, columns))
        })
    }

    fn send(&self, command: Option<Command>) {
        if let Some(command) = command {
            self.cx.commands.send(command);
        }
    }
}

/// What stands for the reload while it is under way.
fn reloading() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(RELOADING)
        .child(
            indeterminate_circular_progress()
                .size(RELOADING_INDICATOR)
                .access(|a| a.role(Role::ProgressBar).label("Reloading Recommendations")),
        )
}

#[cfg(test)]
mod tests;
