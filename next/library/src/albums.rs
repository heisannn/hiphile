//! The albums, beside the songs of the one on show.
//!
//! Where the window is says which album is on show. What the screen moves to itself, by the keys,
//! a press of the secondary button or for want of one, it shows in place, so that going back
//! passes over it; an album chosen with the pointer is somewhere to come back to. An album shown
//! from elsewhere, or by going back, is revealed: the filter is cleared if it hides it, the list
//! scrolls to it and takes the focus.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::core::reactive::effect_with;
use anti_webui::prelude::*;
use hiphile_core::Shown;
use hiphile_core::favorites::Favorite;
use hiphile_core::library::{
    AlbumDetail, AlbumDetailSnapshot, AlbumList, AlbumListStatus, AlbumOrder, LibraryNotice,
};
use hiphile_core::navigation::Location;
use next_host::{follow, notices};
use next_ui::context_menu::ContextMenu;
use next_ui::list_detail::{Split, list_detail};
use next_ui::now_playing::NowPlaying;
use next_ui::row_list::{ROW_HEIGHT, Selection, has_room_for_rows, scroll_to_row};
use next_ui::sort_menu::{SortOption, sort_menu};
use next_ui::{AlbumAction, Command, Screen, ScreenContext};
use playback::Placement;
use subsonic::{Album, AlbumId, Song, SongId};

use keys::AlbumsKey;

mod album_detail;
mod album_list;
mod keys;

/// The rows of songs, which hold buttons as high as a row with a cover leaves room for.
const SONG_ROW_HEIGHT: f32 = ROW_HEIGHT;

/// The albums the library keeps, beside the songs of the one that `location` names, with the order
/// of the albums to choose in the bar. With none named, the first is shown in place.
pub fn albums(location: Signal<Location>, cx: &ScreenContext) -> Screen {
    keys::bind();
    let model = Model::new(location, cx);
    let body = list_detail(
        Split::new(),
        album_list::album_list(&model),
        album_detail::album_detail(&model),
    )
    .child(model.menu.view());
    model.follow();
    // As the screen comes, its keys work
    model.wait_for_the_list(Waiting {
        scroll: true,
        focus: true,
    });
    Screen::new("Albums", body)
        .action(order_menu(&model))
        .home(model.album_rows)
}

/// What the albums' screen goes by, and what it does. Cheap to clone.
#[derive(Clone)]
struct Model {
    cx: ScreenContext,
    list: AlbumList,
    status: Signal<AlbumListStatus>,
    detail: AlbumDetail,
    /// What the detail shows of the album on show.
    album: Signal<AlbumDetailSnapshot>,
    /// The album the window names while it is on Albums, `None` while it is elsewhere.
    here: Memo<Option<Option<AlbumId>>>,
    filter: Signal<String>,
    albums: Selection<AlbumId>,
    songs: Selection<SongId>,
    /// The album on show, when it was asked for before the list had it. It stays on show and comes
    /// into view once a sync brings it.
    revealing: Signal<Option<AlbumId>>,
    /// A sync ended, so that the album being revealed is given up once what it brought shows.
    gives_up: Signal<bool>,
    /// The album the screen last asked to show itself, which is not to be revealed when it comes.
    asked: Rc<RefCell<Option<AlbumId>>>,
    album_rows: NodeRef,
    song_rows: NodeRef,
    /// What waits for the album list to be laid out.
    waiting: Rc<Cell<Waiting>>,
    menu: ContextMenu,
    now_playing: Memo<Option<NowPlaying>>,
}

#[derive(Clone, Copy, Default)]
struct Waiting {
    scroll: bool,
    focus: bool,
}

impl Model {
    fn new(location: Signal<Location>, cx: &ScreenContext) -> Self {
        let workspace = &cx.workspace;
        let list = workspace.albums().clone();
        let status = follow(list.subscribe());
        let detail = AlbumDetail::new(next_host::executor(), workspace.session().clone());
        let album = follow(detail.subscribe());
        let player = follow(workspace.player().subscribe());
        let now_playing = memo(move || {
            player.with(|player| {
                NowPlaying::new(
                    player.current_song().map(|song| song.id().clone()),
                    player.is_playing(),
                )
            })
        });
        let here = memo(move || match location.get() {
            Location::Albums(album) => Some(album),
            _ => None,
        });
        Self {
            cx: cx.clone(),
            filter: signal(list.status().query().to_owned()),
            list,
            status,
            detail,
            album,
            here,
            albums: Selection::new(),
            songs: Selection::new(),
            revealing: signal(None),
            gives_up: signal(false),
            asked: Rc::default(),
            album_rows: node_ref(),
            song_rows: node_ref(),
            waiting: Rc::default(),
            menu: ContextMenu::new(),
            now_playing,
        }
    }

    /// Follows the window's location, the filter, the list, the selections and the syncs, for as
    /// long as the current owner lives.
    fn follow(&self) {
        let this = self.clone();
        effect(move || {
            let query = this.filter.get();
            untrack(|| this.list.set_query(&query));
        });
        let this = self.clone();
        effect(move || {
            let Some(wanted) = this.here.get() else {
                return;
            };
            untrack(|| this.follow_location(wanted));
        });
        let this = self.clone();
        effect_with(move |before: Option<Option<Shown<Album>>>| {
            let (shown, is_updating) = this
                .status
                .with(|status| (status.shown().cloned(), status.is_updating()));
            untrack(|| {
                if this.is_here() {
                    this.follow_shown(before.flatten(), shown.as_ref());
                    this.give_up_if_done(is_updating);
                }
            });
            shown
        });
        let this = self.clone();
        effect(move || {
            let cursor = this.albums.cursor();
            let several = this.albums.with(|selected| selected.len() > 1);
            untrack(|| this.follow_album_selection(cursor, several));
        });
        let songs = self.songs;
        effect(move || {
            let cursor = songs.cursor();
            if songs.with(|selected| selected.len() > 1)
                && let Some(cursor) = cursor
            {
                untrack(|| songs.select(cursor));
            }
        });
        let this = self.clone();
        notices(self.cx.workspace.library().notices(), move |notice| {
            if notice == LibraryNotice::SyncFinished {
                this.gives_up.set(true);
                // By what shows rather than by the core, which may be ahead of it: the album being
                // revealed is given up on once what the sync brought is on show
                this.give_up_if_done(this.status.with_untracked(AlbumListStatus::is_updating));
            }
        });
    }

    fn is_here(&self) -> bool {
        untrack(|| self.here.get()).is_some()
    }

    /// The album on show. Tracks it.
    fn selected(&self) -> Option<AlbumId> {
        self.here.with(|here| here.clone().flatten())
    }

    fn follow_location(&self, wanted: Option<AlbumId>) {
        let asked = self.asked.take();
        let Some(album) = wanted else {
            // Coming here with none named keeps what was on show. The selection shows it in place:
            // this runs in the effect that reads the location, which a change made while it runs
            // does not run again
            match self.albums.cursor().or_else(|| self.first_visible()) {
                Some(album) => self.albums.select(album),
                None => {
                    self.albums.clear();
                    self.detail.clear();
                }
            }
            return;
        };
        if asked.as_ref() == Some(&album) {
            self.select(album);
        } else {
            self.reveal(album);
        }
    }

    /// Shows `album`, chosen with the pointer, which leaves the album that was on show to go back
    /// to.
    fn choose(&self, album: AlbumId) {
        self.asked.replace(Some(album.clone()));
        self.cx
            .commands
            .send(Command::Show(Location::Albums(Some(album))));
    }

    fn show_in_place(&self, album: Option<AlbumId>) {
        self.asked.replace(album.clone());
        self.cx
            .commands
            .send(Command::ShowInPlace(Location::Albums(album)));
    }

    fn select(&self, album: AlbumId) {
        self.revealing.set(None);
        if self.albums.cursor().as_ref() != Some(&album) {
            self.albums.select(album.clone());
        }
        self.songs.clear();
        self.song_rows.scroll_to(0.0, 0.0);
        self.detail.show(album);
        self.scroll_to_selected();
    }

    /// Clears a filter that hides `album`. An album the list does not have yet, as during the
    /// first sync, comes into view once a sync brings it.
    fn reveal(&self, album: AlbumId) {
        let (is_listed, is_hidden) = self.status.with_untracked(|status| {
            let Some(shown) = status.shown() else {
                return (false, false);
            };
            let at = shown
                .items()
                .iter()
                .position(|listed| listed.id() == &album);
            let is_hidden = at.is_some_and(|at| !shown.visible().contains(&at));
            (at.is_some(), is_hidden)
        });
        if is_hidden {
            self.filter.set(String::new());
        }
        self.select(album.clone());
        self.revealing.set((!is_listed).then_some(album));
        self.gives_up.set(false);
        self.wait_for_the_list(Waiting {
            scroll: true,
            focus: true,
        });
    }

    /// Keeps the album on show while it is still there or a sync may yet bring it. Albums in a new
    /// order keep it in view.
    fn follow_shown(&self, before: Option<Shown<Album>>, shown: Option<&Shown<Album>>) {
        let Some(shown) = shown else {
            return;
        };
        if before.is_some_and(|before| Arc::ptr_eq(before.items(), shown.items())) {
            self.scroll_to_selected();
            return;
        }
        let is_listed = |id: &AlbumId| shown.items().iter().any(|album| album.id() == id);
        let selected = untrack(|| self.selected());
        let revealing = self.revealing.get_untracked();
        let keeps = revealing.is_some() || selected.as_ref().is_some_and(is_listed);
        if !keeps {
            self.albums.clear();
            match shown.get(0).map(|album| album.id().clone()) {
                Some(first) => self.show_in_place(Some(first)),
                None => {
                    self.detail.clear();
                    if selected.is_some() {
                        self.show_in_place(None);
                    }
                }
            }
        }
        if revealing.as_ref().is_some_and(is_listed) {
            self.revealing.set(None);
            self.scroll_to_selected();
        }
    }

    /// Gives up on an album being revealed once a sync has ended and what it brought shows.
    fn give_up_if_done(&self, is_updating: bool) {
        if self.gives_up.get_untracked() && !is_updating {
            self.gives_up.set(false);
            self.revealing.set(None);
        }
    }

    /// What the keys and the secondary button selected is shown in place. The album list holds
    /// one album: another selected besides takes its place.
    fn follow_album_selection(&self, cursor: Option<AlbumId>, several: bool) {
        let Some(cursor) = cursor else {
            return;
        };
        if several {
            self.albums.select(cursor.clone());
        }
        if self.is_here() && untrack(|| self.selected()).as_ref() != Some(&cursor) {
            self.show_in_place(Some(cursor));
        }
    }

    fn first_visible(&self) -> Option<AlbumId> {
        self.status.with_untracked(|status| {
            status
                .shown()
                .and_then(|shown| shown.get(0))
                .map(|album| album.id().clone())
        })
    }

    fn scroll_to_selected(&self) {
        self.wait_for_the_list(Waiting {
            scroll: true,
            focus: false,
        });
    }

    /// Does what `waiting` asks of the album list at once if it can, or else once it is laid out.
    fn wait_for_the_list(&self, waiting: Waiting) {
        let before = self.waiting.get();
        self.waiting.set(Waiting {
            scroll: before.scroll || waiting.scroll,
            focus: before.focus || waiting.focus,
        });
        self.list_laid_out();
    }

    /// Called as the album list is laid out at a new size.
    fn list_laid_out(&self) {
        if !has_room_for_rows(self.album_rows, ROW_HEIGHT) {
            return;
        }
        let waiting = self.waiting.take();
        if waiting.focus {
            self.album_rows.focus();
        }
        let position = untrack(|| self.selected()).and_then(|selected| {
            self.status
                .with_untracked(|status| status.shown()?.position(|album| album.id() == &selected))
        });
        if waiting.scroll
            && let Some(position) = position
        {
            scroll_to_row(self.album_rows, position, ROW_HEIGHT);
        }
    }

    /// The album on show, from the list, or from the detail when the list has yet to have it.
    fn album_on_show(&self) -> Option<Album> {
        let selected = untrack(|| self.selected())?;
        let listed = self.status.with_untracked(|status| {
            status
                .albums()
                .items()
                .iter()
                .find(|album| album.id() == &selected)
                .cloned()
        });
        listed.or_else(|| {
            self.album.with_untracked(|album| {
                album
                    .loaded()
                    .map(|loaded| loaded.album())
                    .filter(|album| album.id() == &selected)
                    .cloned()
            })
        })
    }

    /// The songs of the album on show, once they are there. Tracks them.
    fn loaded_songs(&self) -> Option<Arc<[Song]>> {
        self.album
            .with(|album| album.loaded().map(|loaded| loaded.songs().clone()))
    }

    /// Does `action` to the album on show: to its songs at once when they are there, or once they
    /// are had.
    fn act(&self, action: AlbumAction) {
        let Some(album) = untrack(|| self.selected()) else {
            return;
        };
        let loaded = self.album.with_untracked(|shown| {
            shown
                .loaded()
                .filter(|loaded| loaded.album().id() == &album)
                .map(|loaded| loaded.songs().clone())
        });
        let command = match loaded {
            Some(songs) => action.command(songs),
            None => Some(Command::ActOnAlbum { album, action }),
        };
        if let Some(command) = command {
            self.cx.commands.send(command);
        }
    }

    /// The song the selection of the songs is on.
    fn song_at_cursor(&self) -> Option<(Arc<[Song]>, usize)> {
        let cursor = self.songs.cursor()?;
        let songs = untrack(|| self.loaded_songs())?;
        let at = songs.iter().position(|song| song.id() == &cursor)?;
        Some((songs, at))
    }

    /// Moves the focus to the songs, selecting the first one if none is.
    fn open(&self) {
        let Some(songs) = untrack(|| self.loaded_songs()).filter(|songs| !songs.is_empty()) else {
            return;
        };
        if self.song_at_cursor().is_none() {
            self.songs.select(songs[0].id().clone());
        }
        self.song_rows.focus();
    }

    fn take_album_key(&self, key: AlbumsKey) {
        let commands = &self.cx.commands;
        match key {
            AlbumsKey::Open => self.open(),
            AlbumsKey::Play => self.act(AlbumAction::Play),
            AlbumsKey::PlayNext => self.act(AlbumAction::Enqueue(Placement::Next)),
            AlbumsKey::AddToQueue => self.act(AlbumAction::Enqueue(Placement::Last)),
            AlbumsKey::AddToPlaylist => self.act(AlbumAction::ChoosePlaylist),
            AlbumsKey::GoToArtist => {
                let artist = self
                    .album_on_show()
                    .and_then(|album| album.artist_id().cloned());
                if let Some(artist) = artist {
                    commands.send(Command::Show(Location::Artists(Some(artist))));
                }
            }
            AlbumsKey::ToggleFavorite => {
                if let Some(album) = self.album_on_show() {
                    commands.send(Command::ToggleFavorite(Favorite::Album(album)));
                }
            }
            AlbumsKey::Back => {}
        }
    }

    fn take_song_key(&self, key: AlbumsKey) {
        if key == AlbumsKey::Back {
            self.album_rows.focus();
            return;
        }
        let Some((songs, at)) = self.song_at_cursor() else {
            return;
        };
        let song = songs[at].clone();
        let commands = &self.cx.commands;
        let one = || Arc::from([song.clone()]);
        match key {
            AlbumsKey::PlayNext => commands.send(Command::Enqueue {
                songs: one(),
                placement: Placement::Next,
            }),
            AlbumsKey::AddToQueue => commands.send(Command::Enqueue {
                songs: one(),
                placement: Placement::Last,
            }),
            AlbumsKey::AddToPlaylist => commands.send(Command::ChoosePlaylist(one())),
            AlbumsKey::GoToArtist => {
                if let Some(artist) = song.artist_id() {
                    commands.send(Command::Show(Location::Artists(Some(artist.clone()))));
                }
            }
            AlbumsKey::ToggleFavorite => {
                commands.send(Command::ToggleFavorite(Favorite::Song(song)));
            }
            AlbumsKey::Open | AlbumsKey::Play | AlbumsKey::Back => {}
        }
    }
}

/// The order of the albums, and which way it runs.
fn order_menu(model: &Model) -> Div {
    let list = model.list.clone();
    let status = model.status;
    let sort = memo(move || status.with(AlbumListStatus::sort));
    let order = |label: &str, order: AlbumOrder| {
        let list = list.clone();
        SortOption::new(
            label,
            move || sort.get().order == order,
            move || list.choose_order(order),
        )
    };
    let direction = |label: &str, is_descending: bool| {
        let list = list.clone();
        SortOption::new(
            label,
            move || sort.get().is_descending == is_descending,
            move || list.choose_direction(is_descending),
        )
    };
    sort_menu(
        move || sort.get().is_descending,
        vec![
            vec![
                order("Name", AlbumOrder::Name),
                order("Artist", AlbumOrder::Artist),
                order("Recently Added", AlbumOrder::Added),
                order("Release Date", AlbumOrder::Released),
            ],
            vec![direction("Ascending", false), direction("Descending", true)],
        ],
    )
}

#[cfg(test)]
mod tests;
