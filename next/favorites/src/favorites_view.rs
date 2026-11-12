use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{MenuItem, TabsStyle, styled_tabs, tab};
use hiphile_core::favorites::{
    Favorite, FavoriteLists, FavoritesLoad, FavoritesSnapshot, FavoritesState, Section,
};
use hiphile_core::navigation::Location;
use next_host::follow;
use next_ui::context_menu::ContextMenu;
use next_ui::details::{format_album_count, format_artist_count, format_song_count};
use next_ui::keymap::{KeyBinding, KeyContext, Keystroke, bind_keys, is_bound, key_context};
use next_ui::list_pane::{ListHeader, list_header, refresh_failure};
use next_ui::load_state::{
    FAVORITE_ALBUMS, FAVORITE_ARTISTS, FAVORITE_SONGS, FAVORITES, empty_state, load_failure,
    pane_loading,
};
use next_ui::menus::{album_menu, artist_menu, show_in_albums_item, song_menu};
use next_ui::now_playing::NowPlaying;
use next_ui::pane::pane;
use next_ui::row_list::Selection;
use next_ui::{AlbumAction, Command, Screen, ScreenContext};
use playback::Placement;
use subsonic::{Song, StarTarget};

mod rows;

use rows::{album_list, artist_list, song_list};

/// Where the keys of [`FavoritesCommand`] work: in the screen.
pub const FAVORITES_CONTEXT: &str = "Favorites";

/// What the keys do to the favorite selected, on top of what the list does itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FavoritesCommand {
    /// A song from it, among the others; an album's songs.
    Play,
    PlayNext,
    AddToQueue,
    AddToPlaylist,
    GoToAlbum,
    GoToArtist,
    ToggleFavorite,
    PreviousSection,
    NextSection,
}

/// Adds the keys of the screen to this thread's keymap, once.
fn bind_favorites_keys() {
    if is_bound::<FavoritesCommand>() {
        return;
    }
    use FavoritesCommand::*;
    let enter = || Keystroke::named(NamedKey::Enter);
    bind_keys(
        [
            (enter().with_primary(), Play),
            (enter().with_alt(), PlayNext),
            (enter().with_shift(), AddToQueue),
            (Keystroke::primary('p'), AddToPlaylist),
            (Keystroke::primary('l'), GoToAlbum),
            (Keystroke::primary('a').with_shift(), GoToArtist),
            (Keystroke::primary('d'), ToggleFavorite),
            (Keystroke::named(NamedKey::ArrowLeft), PreviousSection),
            (Keystroke::named(NamedKey::ArrowRight), NextSection),
        ]
        .map(|(stroke, command)| KeyBinding::new(stroke, command, FAVORITES_CONTEXT)),
    );
}

/// What the screen and its rows share. The lists on show follow `shown`; what the keys and menus
/// act on is `lists`, which is never behind it.
pub(crate) struct View {
    pub(crate) cx: ScreenContext,
    favorites: FavoritesState,
    pub(crate) shown: Signal<FavoritesSnapshot>,
    section: Signal<Section>,
    /// Each kind's own, by [`Section::ix`].
    pub(crate) selections: [Selection<StarTarget>; 3],
    pub(crate) menu: ContextMenu,
    pub(crate) now_playing: Memo<Option<NowPlaying>>,
    /// What takes the focus and the keys in the list on show.
    pub(crate) list: NodeRef,
    tabs: NodeRef,
    /// Whether the list of the section just chosen takes the focus once it is there.
    focus_list: Cell<bool>,
    /// The favorites as last seen, either from `shown` or right after a change from here, where
    /// what was selected is looked for once it has gone.
    lists: RefCell<FavoriteLists>,
}

/// The favorites, a kind at a time: the songs, the albums and the artists the user marked.
pub fn favorites(cx: &ScreenContext) -> Screen {
    bind_favorites_keys();
    let favorites = cx.workspace.favorites().clone();
    let shown = follow(favorites.subscribe());
    let view = Rc::new(View {
        cx: cx.clone(),
        favorites,
        shown,
        section: signal(Section::Songs),
        selections: [Selection::new(), Selection::new(), Selection::new()],
        menu: ContextMenu::new(cx.overlays),
        now_playing: cx.known.now_playing(),
        list: node_ref(),
        tabs: node_ref(),
        focus_list: Cell::new(false),
        lists: RefCell::default(),
    });
    effect({
        let view = view.clone();
        move || {
            let lists = shown.with(|shown| shown.lists().clone());
            untrack(|| view.settle(lists));
        }
    });
    effect({
        let view = view.clone();
        move || {
            if view.list.get().is_some() && view.focus_list.take() {
                view.list.focus();
            }
        }
    });
    let context = {
        let (enabled, run) = (view.clone(), view.clone());
        KeyContext::new(
            FAVORITES_CONTEXT,
            move |command| enabled.is_enabled(*command),
            move |command| run.run(command),
        )
    };
    let content = div()
        .flex()
        .flex_col()
        .child(tabs(&view))
        .child(header(&view))
        .child(refresh_error(&view))
        .child(body(view.clone()))
        .child(view.menu.view());
    Screen::new("Favorites", key_context(pane(content), &context)).home(view.list)
}

/// What the user goes from kind to kind with.
fn tabs(view: &Rc<View>) -> Div {
    let section = view.section;
    let selected = Binding::new(
        move || section.get().ix(),
        move |ix| {
            if let Some(chosen) = Section::ALL.get(ix) {
                section.set(*chosen);
            }
        },
    );
    let labels = Section::ALL.map(|section| tab(section.label())).to_vec();
    styled_tabs(labels, selected, TabsStyle::secondary())
        .node_ref(view.tabs)
        .flex_none()
        // On the pane rather than the window behind it
        .dyn_style(|s| s.bg(color_scheme().surface_container))
}

/// How many there are of the kind on show, and whether they are being read again.
fn header(view: &Rc<View>) -> Div {
    let (shown, section) = (view.shown, view.section);
    let count = move || {
        shown.with(|shown| {
            // None says so in the pane itself
            let len = shown
                .is_loaded()
                .then(|| shown.lists().len(section.get()))
                .filter(|len| *len > 0)?;
            Some(match section.get() {
                Section::Songs => format_song_count(len),
                Section::Albums => format_album_count(len),
                Section::Artists => format_artist_count(len),
            })
        })
    };
    list_header(
        ListHeader::new(count).updating(FAVORITES.updating(), move || {
            shown.with(|shown| shown.is_loaded() && shown.is_reloading())
        }),
    )
}

/// Why reading the favorites again failed, above those still on show.
fn refresh_error(view: &Rc<View>) -> Div {
    let shown = view.shown;
    let error = memo(move || shown.with(|shown| shown.refresh_error().map(str::to_owned)));
    let favorites = view.favorites.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let favorites = favorites.clone();
        match error.get() {
            None => untrack(|| div().into_view()),
            Some(reason) => untrack(|| {
                refresh_failure(FAVORITES.refresh_failed(), reason, move || {
                    favorites.refresh()
                })
                .into_view()
            }),
        }
    }))
}

/// What the pane shows below the tabs.
#[derive(Clone, Debug, PartialEq)]
enum Shape {
    Loading,
    Failed(String),
    Empty(Section),
    Listed(Section),
}

fn body(view: Rc<View>) -> Div {
    let (shown, section) = (view.shown, view.section);
    let shape = memo(move || {
        shown.with(|shown| match shown.load() {
            FavoritesLoad::Loading => Shape::Loading,
            FavoritesLoad::Failed(error) => Shape::Failed(error.clone()),
            FavoritesLoad::Loaded if shown.lists().len(section.get()) == 0 => {
                Shape::Empty(section.get())
            }
            FavoritesLoad::Loaded => Shape::Listed(section.get()),
        })
    });
    div()
        .flex_1()
        .min_h(0)
        .flex()
        .flex_col()
        .child(dyn_view(move || {
            let shape = shape.get();
            let view = view.clone();
            untrack(move || shaped(&view, shape).flex_1().min_h(0).into_view())
        }))
}

fn shaped(view: &Rc<View>, shape: Shape) -> Div {
    match shape {
        Shape::Loading => pane_loading(FAVORITES.loading()),
        Shape::Failed(reason) => {
            let favorites = view.favorites.clone();
            load_failure(FAVORITES.load_failed(), reason, move || favorites.refresh())
        }
        Shape::Empty(section) => {
            let (subject, note) = match section {
                Section::Songs => (FAVORITE_SONGS, "Right-click a song to add it."),
                Section::Albums => (FAVORITE_ALBUMS, "Right-click an album to add it."),
                Section::Artists => (FAVORITE_ARTISTS, "Right-click an artist to add them."),
            };
            empty_state(&subject.none(), Some(note), None)
        }
        Shape::Listed(Section::Songs) => song_list(view),
        Shape::Listed(Section::Albums) => album_list(view),
        Shape::Listed(Section::Artists) => artist_list(view),
    }
}

impl View {
    /// Takes `lists` as the favorites now. What was selected of each kind stays so while it is
    /// there, and else the one now where it was is.
    fn settle(&self, lists: FavoriteLists) {
        let before = self.lists.replace(lists);
        let lists = self.lists.borrow();
        for section in Section::ALL {
            let selection = self.selections[section.ix()];
            let cursor = untrack(|| selection.cursor());
            let was_ix = cursor
                .as_ref()
                .and_then(|cursor| before.position(cursor))
                .unwrap_or_default();
            let kept = lists.kept_selection(section, cursor.as_ref(), was_ix);
            if kept == cursor {
                continue;
            }
            match kept {
                Some(target) => selection.select(target),
                None => selection.clear(),
            }
        }
    }

    pub(crate) fn favorite(&self, target: &StarTarget) -> Option<Favorite> {
        let lists = self.lists.borrow();
        lists.favorite_at(Section::of(target), lists.position(target)?)
    }

    fn selected(&self) -> Option<Favorite> {
        let section = untrack(|| self.section.get());
        let cursor = untrack(|| self.selections[section.ix()].cursor())?;
        self.favorite(&cursor)
    }

    fn send(&self, command: Command) {
        self.cx.commands.send(command);
    }

    /// Plays the favorite songs from `song`.
    pub(crate) fn play_from(&self, song: &Song) -> Option<Command> {
        let songs: Arc<[Song]> = self.lists.borrow().songs.iter().cloned().collect();
        let start = songs.iter().position(|listed| listed.id() == song.id())?;
        Some(Command::Play {
            songs,
            start: Some(start),
        })
    }

    /// What Enter and a double click do: play a song from it, and show an album or an artist.
    pub(crate) fn open(&self, target: &StarTarget) {
        let command = match self.favorite(target) {
            Some(Favorite::Song(song)) => self.play_from(&song),
            Some(Favorite::Album(album)) => {
                Some(Command::Show(Location::Albums(Some(album.id().clone()))))
            }
            Some(Favorite::Artist(artist)) => {
                Some(Command::Show(Location::Artists(Some(artist.id().clone()))))
            }
            None => None,
        };
        if let Some(command) = command {
            self.send(command);
        }
    }

    /// The menu of `target`, made as it opens.
    pub(crate) fn menu_of(&self, target: &StarTarget) -> Vec<Vec<MenuItem>> {
        let (commands, known) = (&self.cx.commands, self.cx.known);
        match self.favorite(target) {
            Some(Favorite::Song(song)) => song_menu(commands, known, &song, self.play_from(&song)),
            Some(Favorite::Album(album)) => {
                let mut groups = album_menu(commands, known, &album);
                if let Some(last) = groups.last_mut() {
                    last.push(show_in_albums_item(commands, album.id()));
                }
                groups
            }
            Some(Favorite::Artist(artist)) => artist_menu(commands, known, &artist),
            None => Vec::new(),
        }
    }

    /// What `command` does to the selection, or `None` when it does nothing.
    fn command_for(&self, command: FavoritesCommand) -> Option<Command> {
        let selected = self.selected()?;
        let on_songs = |action: AlbumAction| match &selected {
            Favorite::Song(song) => action.command(Arc::from([song.clone()])),
            Favorite::Album(album) => Some(Command::ActOnAlbum {
                album: album.id().clone(),
                action,
            }),
            Favorite::Artist(_) => None,
        };
        match command {
            FavoritesCommand::Play => match &selected {
                Favorite::Song(song) => self.play_from(song),
                _ => on_songs(AlbumAction::Play),
            },
            FavoritesCommand::PlayNext => on_songs(AlbumAction::Enqueue(Placement::Next)),
            FavoritesCommand::AddToQueue => on_songs(AlbumAction::Enqueue(Placement::Last)),
            FavoritesCommand::AddToPlaylist => on_songs(AlbumAction::ChoosePlaylist),
            FavoritesCommand::GoToAlbum => match &selected {
                Favorite::Song(song) => {
                    let album = song.album_id()?.clone();
                    Some(Command::Show(Location::Albums(Some(album))))
                }
                _ => None,
            },
            FavoritesCommand::GoToArtist => {
                let artist = match &selected {
                    Favorite::Song(song) => song.artist_id()?.clone(),
                    Favorite::Album(album) => album.artist_id()?.clone(),
                    Favorite::Artist(artist) => artist.id().clone(),
                };
                Some(Command::Show(Location::Artists(Some(artist))))
            }
            FavoritesCommand::ToggleFavorite => Some(Command::ToggleFavorite(selected)),
            FavoritesCommand::PreviousSection | FavoritesCommand::NextSection => None,
        }
    }

    fn adjacent_section(&self, command: FavoritesCommand) -> Option<Section> {
        let ix = untrack(|| self.section.get()).ix();
        let to = match command {
            FavoritesCommand::PreviousSection => ix.checked_sub(1)?,
            FavoritesCommand::NextSection => ix + 1,
            _ => return None,
        };
        Section::ALL.get(to).copied()
    }

    fn is_enabled(&self, command: FavoritesCommand) -> bool {
        match command {
            FavoritesCommand::PreviousSection | FavoritesCommand::NextSection => {
                self.adjacent_section(command).is_some()
            }
            FavoritesCommand::ToggleFavorite => {
                self.favorites.snapshot().is_loaded() && self.command_for(command).is_some()
            }
            _ => self.command_for(command).is_some(),
        }
    }

    fn run(&self, command: FavoritesCommand) {
        if let Some(section) = self.adjacent_section(command) {
            self.show_section(section);
            return;
        }
        let Some(sent) = self.command_for(command) else {
            return;
        };
        self.send(sent);
        if command == FavoritesCommand::ToggleFavorite {
            // The next key acts on what is selected once this one is out, before the change has
            // come round to `shown`
            self.settle(self.favorites.snapshot().lists().clone());
        }
    }

    /// Shows `section`, and gives its list the focus once it is there, or the tabs when it has
    /// no list.
    fn show_section(&self, section: Section) {
        let has_list = self
            .shown
            .with_untracked(|shown| shown.is_loaded() && shown.lists().len(section) > 0);
        self.focus_list.set(has_list);
        self.section.set(section);
        if !has_list {
            self.tabs.focus();
        }
    }
}

#[cfg(test)]
mod tests;
