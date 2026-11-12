use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::core::access::snapshot;
use anti_webui::core::event::{Key, Modifiers, NamedKey};
use hiphile_core::Hiphile;
use hiphile_core::search::{IndexState, SearchIndex};
use next_ui::cover::install_covers;
use next_ui::testing::{
    NOWHERE, SavedLibrary, TestCore, find, frames, has_text, key, key_with, nodes, press,
    recording_commands, save_library, screen_context, signed_in_core, signed_in_core_with_library,
    start, typed,
};
use serde_json::json;

use super::*;

fn library() -> SavedLibrary {
    SavedLibrary {
        albums: serde_json::from_value(json!([
            { "id": "blue", "name": "Kind of Blue", "artist": "Miles Davis", "artistId": "miles" },
        ]))
        .unwrap(),
        songs: serde_json::from_value(json!([
            { "id": "so-what", "title": "So What", "artist": "Miles Davis", "artistId": "miles",
              "album": "Kind of Blue", "albumId": "blue" },
            { "id": "naima", "title": "Naima", "artist": "John Coltrane", "artistId": "john" },
        ]))
        .unwrap(),
        ..SavedLibrary::default()
    }
}

/// What the library holds after a sync that brought one song in place of the rest.
fn resynced(title: &str) -> SavedLibrary {
    SavedLibrary {
        songs: serde_json::from_value(json!([
            { "id": title, "title": title, "artist": "Someone", "artistId": "s" },
        ]))
        .unwrap(),
        ..SavedLibrary::default()
    }
}

/// The search alone, over nothing, sending its commands where the test reads them.
struct Searching {
    ui: Ui,
    open: Signal<bool>,
    sent: Rc<RefCell<Vec<Command>>>,
    hiphile: Hiphile,
    catalog: Option<catalog::Catalog>,
    _core: TestCore,
}

impl Searching {
    fn new(core: TestCore, hiphile: Hiphile, catalog: Option<catalog::Catalog>) -> Self {
        let (commands, sent) = recording_commands();
        let made = Cell::new(None);
        let ui = start({
            let (hiphile, made) = (hiphile.clone(), &made);
            move || {
                install_covers(hiphile.clone());
                let cx = screen_context(&hiphile, commands);
                let open = signal(false);
                made.set(Some(open));
                div().size_full().relative().child(search(&cx, open))
            }
        });
        Self {
            ui,
            open: made.get().expect("the view to have run"),
            sent,
            hiphile,
            catalog,
            _core: core,
        }
    }

    fn with_library() -> Self {
        let (core, hiphile, catalog) = signed_in_core_with_library(NOWHERE, &library());
        Self::new(core, hiphile, Some(catalog))
    }

    fn open(&mut self) {
        self.open.set(true);
        self.ui.update();
    }

    /// Types `query` in place of what is typed.
    fn ask(&mut self, query: &str) {
        key_with(&mut self.ui, Key::Character("a".into()), Modifiers::PRIMARY);
        typed(&mut self.ui, query);
    }

    fn wait_until(&mut self, done: impl FnMut() -> bool) {
        next_ui::testing::wait_until(&mut self.ui, done);
    }

    fn key(&mut self, named: NamedKey) {
        key(&mut self.ui, named);
    }

    fn index(&self) -> SearchIndex {
        self.hiphile.workspace().unwrap().search().clone()
    }

    /// Stands for a sync that brought `library`.
    fn synced(&self, library: &SavedLibrary) {
        save_library(self.catalog.as_ref().expect("a saved library"), library);
        self.index().library_changed();
    }

    fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    fn is_open(&self) -> bool {
        self.open.get_untracked()
    }
}

fn labels(role: Role) -> Vec<String> {
    nodes(role)
        .into_iter()
        .filter_map(|node| node.label)
        .collect()
}

fn headings() -> Vec<String> {
    labels(Role::Heading)
}

fn found() -> Vec<String> {
    labels(Role::ListOption)
}

fn highlighted() -> Option<String> {
    nodes(Role::ListOption)
        .into_iter()
        .find(|node| node.selected == Some(true))
        .and_then(|node| node.label)
}

fn is_loading() -> bool {
    find(Role::ProgressBar, "Loading Library").is_some()
}

/// What is typed where the focus is, when that is a text field.
fn typed_in_focus() -> Option<String> {
    let tree = snapshot();
    let focus = tree.get(tree.focus?)?;
    (focus.role == Role::TextInput).then(|| focus.value_text.clone().unwrap_or_default())
}

#[test]
fn it_opens_empty_with_the_focus_and_finds_each_kind_as_typed() {
    let mut search = Searching::with_library();
    assert!(nodes(Role::TextInput).is_empty());

    search.open();
    assert_eq!(typed_in_focus().as_deref(), Some(""));
    assert!(found().is_empty());

    search.ask("miles");
    assert_eq!(typed_in_focus().as_deref(), Some("miles"));
    // What an earlier part of the query found may show first
    search.wait_until(|| found() == ["Miles Davis", "Kind of Blue", "So What"]);
    assert_eq!(headings(), ["Artists", "Albums", "Songs"]);

    search.ask("naima");
    search.wait_until(|| found() == ["Naima"]);
    assert_eq!(headings(), ["Songs"]);

    search.ask("nothing like it");
    search.wait_until(|| has_text("No matches for “nothing like it”"));
    assert!(found().is_empty());
}

#[test]
fn enter_plays_the_song_highlighted_and_closes() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("naima");
    search.wait_until(|| found() == ["Naima"]);
    assert_eq!(highlighted().as_deref(), Some("Naima"));

    search.key(NamedKey::Enter);

    assert!(!search.is_open());
    assert!(matches!(
        search.sent().as_slice(),
        [Command::Play { songs, start: Some(0) }]
            if songs.len() == 1 && songs[0].id().as_str() == "naima"
    ));
}

#[test]
fn the_arrows_move_the_highlight_to_the_next_kind_and_no_further_than_the_ends() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("miles");
    search.wait_until(|| found() == ["Miles Davis", "Kind of Blue", "So What"]);

    search.key(NamedKey::ArrowUp);
    assert_eq!(highlighted().as_deref(), Some("Miles Davis"));
    search.key(NamedKey::ArrowDown);
    assert_eq!(highlighted().as_deref(), Some("Kind of Blue"));
    for _ in 0..3 {
        search.key(NamedKey::ArrowDown);
    }
    assert_eq!(highlighted().as_deref(), Some("So What"));
    assert_eq!(typed_in_focus().as_deref(), Some("miles"));

    search.key(NamedKey::ArrowUp);
    search.key(NamedKey::Enter);
    assert!(matches!(
        search.sent().as_slice(),
        [Command::Show(Location::Albums(Some(album)))] if album.as_str() == "blue"
    ));
}

#[test]
fn new_results_highlight_their_first() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("miles");
    search.wait_until(|| found() == ["Miles Davis", "Kind of Blue", "So What"]);
    search.key(NamedKey::ArrowDown);

    search.ask("so what");
    search.wait_until(|| found() == ["So What"]);

    assert_eq!(highlighted().as_deref(), Some("So What"));
}

#[test]
fn choosing_an_artist_goes_to_it() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("coltrane");
    search.wait_until(|| found().contains(&"John Coltrane".to_owned()));

    press(&mut search.ui, Role::ListOption, "John Coltrane");

    assert!(!search.is_open());
    assert!(matches!(
        search.sent().as_slice(),
        [Command::Show(Location::Artists(Some(artist)))] if artist.as_str() == "john"
    ));
}

#[test]
fn clicking_what_is_not_highlighted_chooses_it() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("miles");
    search.wait_until(|| found() == ["Miles Davis", "Kind of Blue", "So What"]);

    press(&mut search.ui, Role::ListOption, "Kind of Blue");

    assert!(matches!(
        search.sent().as_slice(),
        [Command::Show(Location::Albums(Some(album)))] if album.as_str() == "blue"
    ));
}

#[test]
fn escape_closes_without_asking_for_anything() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("miles");
    search.wait_until(|| found() == ["Miles Davis", "Kind of Blue", "So What"]);

    search.key(NamedKey::Escape);

    assert!(!search.is_open());
    frames(&mut search.ui, 1000);
    assert!(nodes(Role::TextInput).is_empty());
    assert!(search.sent().is_empty());
}

#[test]
fn it_opens_again_on_nothing_typed() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("miles");
    search.wait_until(|| found() == ["Miles Davis", "Kind of Blue", "So What"]);
    search.key(NamedKey::Escape);
    frames(&mut search.ui, 1000);

    search.open();

    assert_eq!(typed_in_focus().as_deref(), Some(""));
    assert!(found().is_empty());
}

#[test]
fn a_library_that_changed_while_open_shows_without_opening_it_again() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("a");
    search.wait_until(|| found().contains(&"Naima".to_owned()));

    search.synced(&resynced("A Love Supreme"));

    search.wait_until(|| !found().contains(&"Naima".to_owned()));
    assert!(found().contains(&"A Love Supreme".to_owned()));
}

#[test]
fn what_was_read_stays_searchable_during_a_new_reading() {
    let mut search = Searching::with_library();
    search.open();
    search.ask("naima");
    search.wait_until(|| found() == ["Naima"]);

    search.index().library_changed();

    let status = search.index().status();
    assert!(status.is_reading());
    assert_eq!(*status.library(), IndexState::Read);
    search.ui.update();
    assert!(!is_loading());
    assert_eq!(found(), ["Naima"]);
}

#[test]
fn the_first_reading_shows_and_a_change_during_it_is_read_too() {
    let mut search = Searching::with_library();

    search.open();
    assert!(is_loading());
    search.synced(&resynced("A Love Supreme"));
    let index = search.index();
    search.wait_until(|| !index.status().is_reading());
    search.ask("a");

    search.wait_until(|| found().contains(&"A Love Supreme".to_owned()));
    assert!(!found().contains(&"Naima".to_owned()));
    assert!(!is_loading());
}

#[test]
fn without_a_saved_library_it_says_so_and_how_to_save_one() {
    let (core, hiphile) = signed_in_core(NOWHERE);
    let mut search = Searching::new(core, hiphile, None);

    search.open();

    search.wait_until(|| find(Role::Heading, "Couldn't load the library").is_some());
    assert!(has_text(
        "There's no library saved on this computer to search. Check that the disk has free \
         space, then sign out and sign in again."
    ));
    assert!(find(Role::Button, "Retry").is_some());
    assert!(!is_loading());
}
