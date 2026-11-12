use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::core::access::snapshot;
use anti_webui::core::event::WindowEvent;
use anti_webui::core::geometry::{Scale, SizeI};
use anti_webui::prelude::*;
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::{Destination, Location};
use hiphile_core::test_support::LibraryServer;
use hiphile_core::{Hiphile, Workspace};
use next_ui::keymap::Keystroke;
use next_ui::list_detail::Split;
use next_ui::testing::{
    SavedLibrary, TestCore, click_text, find, focus, frames, has_text, key, key_with, middle_of,
    nodes, press, right_click, screen_context, signed_in_core, signed_in_core_with_library, start,
    typed, wait_until,
};
use next_ui::{AlbumAction, Command, Commands};
use playback::Placement;
use serde_json::{Value, json};
use subsonic::{AlbumId, ArtistId, Song};

use super::albums;

/// An album of the artist `ar`, as `getAlbum` gives it, with songs of the titles in order.
fn album(id: &str, name: &str, titles: &[&str]) -> Value {
    let songs: Vec<Value> = titles
        .iter()
        .enumerate()
        .map(|(at, title)| {
            json!({
                "id": format!("{id}{}", at + 1),
                "title": title,
                "track": at + 1,
                "duration": 180,
                "album": name,
                "albumId": id,
                "artist": "Artist",
                "artistId": "ar",
            })
        })
        .collect();
    json!({
        "id": id,
        "name": name,
        "artist": "Artist",
        "artistId": "ar",
        "songCount": songs.len(),
        "song": songs,
    })
}

fn alpha_and_beta() -> LibraryServer {
    LibraryServer::new(vec![
        album("a", "Alpha", &["One", "Two"]),
        album("b", "Beta", &["Three"]),
    ])
}

/// Albums on its own, at `Location`s it sends itself to as the shell does.
struct Albums {
    ui: Ui,
    location: Signal<Location>,
    /// What going back would show, the last first.
    left: Rc<RefCell<Vec<Location>>>,
    sent: Rc<RefCell<Vec<Command>>>,
    /// Whether the screen is there, as while the window shows Albums.
    here: Signal<bool>,
    /// The keys that did nothing in the screen and went on to the window.
    passed: Rc<RefCell<Vec<Keystroke>>>,
    workspace: Workspace,
    server: LibraryServer,
    _core: TestCore,
}

/// Albums on `server` at `at`.
fn on(server: LibraryServer, at: Location) -> Albums {
    let (core, hiphile) = signed_in_core(server.url());
    on_core(server, core, hiphile, at)
}

/// Albums on `server` at `at`, signed in to it through `hiphile`.
fn on_core(server: LibraryServer, core: TestCore, hiphile: Hiphile, at: Location) -> Albums {
    let workspace = hiphile.workspace().expect("signed in");
    let location = signal(at);
    let here = signal(true);
    let left = Rc::new(RefCell::new(Vec::new()));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let passed = Rc::new(RefCell::new(Vec::new()));
    let commands = {
        let (left, sent, workspace) = (left.clone(), sent.clone(), workspace.clone());
        Commands::new(move |command: Command| {
            sent.borrow_mut().push(command.clone());
            match command {
                Command::Show(to) => {
                    let now = location.get_untracked();
                    if now.is_worth_returning_to_from(&to) {
                        left.borrow_mut().push(now);
                    }
                    location.set_if_changed(to);
                }
                Command::ShowInPlace(to) => location.set_if_changed(to),
                Command::Refresh => workspace.refresh(Destination::Albums),
                _ => {}
            }
        })
    };
    let ui = start({
        let passed = passed.clone();
        move || {
            let cx = screen_context(&hiphile, commands);
            // Kept between visits, as the shell keeps it
            let split = Split::new();
            let window = div().size_full().flex().flex_col().on_key_down(move |e| {
                if !e.is_default_prevented() {
                    passed
                        .borrow_mut()
                        .push(Keystroke::new(e.key.clone(), e.modifiers));
                }
            });
            window.child(show(
                move || here.get(),
                move || {
                    let screen = albums(location, split, &cx);
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(div().flex().flex_row().children(screen.actions))
                        .child(screen.body.flex_1().min_h(0))
                },
            ))
        }
    });
    let mut albums = Albums {
        ui,
        location,
        left,
        sent,
        here,
        passed,
        workspace,
        server,
        _core: core,
    };
    albums.settle();
    albums
}

impl Albums {
    /// Lets what has come lay out and what follows from it happen.
    fn settle(&mut self) {
        frames(&mut self.ui, 50);
    }

    fn wait_for(&mut self, mut done: impl FnMut() -> bool) {
        wait_until(&mut self.ui, &mut done);
        self.settle();
    }

    fn wait_for_the_albums(&mut self) {
        self.wait_for(|| !is_loading() && !nodes(Role::ListOption).is_empty());
    }

    /// Waits for the songs of `album` to show.
    /// Waits until whether the album on show and its songs are favorites is known.
    fn wait_for_the_favorites(&mut self) {
        self.wait_for(|| {
            let favorites: Vec<_> = snapshot()
                .nodes
                .into_iter()
                .filter(|node| node.label.as_deref() == Some("Favorite"))
                .collect();
            !favorites.is_empty() && favorites.iter().all(|favorite| !favorite.disabled)
        });
    }

    fn wait_for_songs(&mut self, title: &str) {
        let title = title.to_owned();
        self.wait_for(move || {
            find(Role::Heading, &title).is_some() && find(Role::ListBox, "Songs").is_some()
        });
    }

    /// Lets what the core tells cross to the UI thread.
    fn pause(&mut self) {
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(2));
            self.ui.update();
        }
        self.settle();
    }

    fn location(&self) -> Location {
        self.location.get_untracked()
    }

    fn on_show(&self) -> Option<AlbumId> {
        match self.location() {
            Location::Albums(album) => album,
            other => panic!("the window went to {other:?}"),
        }
    }

    fn left(&self) -> Vec<Location> {
        self.left.borrow().clone()
    }

    /// What was sent besides where to go.
    fn sent(&self) -> Vec<Command> {
        self.sent
            .borrow()
            .iter()
            .filter(|command| !matches!(command, Command::Show(_) | Command::ShowInPlace(_)))
            .cloned()
            .collect()
    }

    /// Shows `location` as going there from elsewhere does.
    fn go(&mut self, location: Location) {
        self.location.set(location);
        self.settle();
    }

    fn primary(&mut self, named: NamedKey) {
        key_with(&mut self.ui, Key::Named(named), Modifiers::PRIMARY);
        self.settle();
    }

    fn with(&mut self, named: NamedKey, modifiers: Modifiers) {
        key_with(&mut self.ui, Key::Named(named), modifiers);
        self.settle();
    }

    fn primary_char(&mut self, c: char, shift: bool) {
        let modifiers = if shift {
            Modifiers::PRIMARY.with_shift()
        } else {
            Modifiers::PRIMARY
        };
        key_with(&mut self.ui, Key::Character(c.to_string()), modifiers);
        self.settle();
    }

    fn key(&mut self, named: NamedKey) {
        key(&mut self.ui, named);
        self.settle();
    }

    fn filter(&mut self, query: &str) {
        focus(&mut self.ui, Role::TextInput, "Filter Albums");
        typed(&mut self.ui, query);
        self.settle();
    }
}

fn is_loading() -> bool {
    find(Role::ProgressBar, "Loading Albums").is_some()
}

/// The role and the name of what has the focus.
fn focused() -> Option<(Role, String)> {
    let tree = snapshot();
    let focus = tree.focus?;
    let node = tree.nodes.into_iter().find(|node| node.id == focus)?;
    Some((node.role, node.label.unwrap_or_default()))
}

fn list_width() -> f64 {
    let handle = find(Role::Separator, "List Width").expect("the handle");
    handle.value.expect("a width").now
}

fn has_focus(role: Role, label: &str) -> bool {
    focused() == Some((role, label.to_owned()))
}

/// The names of the rows of `list` on show, in order.
fn rows_of(list: &str) -> Vec<String> {
    let tree = snapshot();
    let Some(list) = tree
        .nodes
        .iter()
        .find(|node| node.role == Role::ListBox && node.label.as_deref() == Some(list))
    else {
        return Vec::new();
    };
    let children = list.children.clone();
    tree.nodes
        .iter()
        .filter(|node| node.role == Role::ListOption && children.contains(&node.id))
        .filter_map(|node| node.label.clone())
        .collect()
}

/// The names of the selected rows of `list`.
fn selected_of(list: &str) -> Vec<String> {
    let tree = snapshot();
    let Some(list) = tree
        .nodes
        .iter()
        .find(|node| node.role == Role::ListBox && node.label.as_deref() == Some(list))
    else {
        return Vec::new();
    };
    let children = list.children.clone();
    tree.nodes
        .iter()
        .filter(|node| children.contains(&node.id) && node.selected == Some(true))
        .filter_map(|node| node.label.clone())
        .collect()
}

fn selected_row() -> Option<String> {
    nodes(Role::ListOption)
        .into_iter()
        .find(|row| row.selected == Some(true))
        .and_then(|row| row.label)
}

fn filter_text() -> Option<String> {
    find(Role::TextInput, "Filter Albums").and_then(|field| field.value_text)
}

fn song_ids(songs: &[Song]) -> Vec<&str> {
    songs.iter().map(|song| song.id().as_str()).collect()
}

fn at(id: &str) -> Location {
    Location::Albums(Some(AlbumId::new(id)))
}

/// Albums on [`alpha_and_beta`], with Alpha and its songs on show.
fn showing_alpha() -> Albums {
    let mut albums = on(alpha_and_beta(), Location::Albums(None));
    albums.wait_for_the_albums();
    albums.wait_for_songs("Alpha");
    albums
}

#[test]
fn coming_with_no_album_named_shows_the_first_in_place() {
    let albums = showing_alpha();

    assert_eq!(albums.location(), at("a"));
    assert!(albums.left().is_empty());
    assert_eq!(rows_of("Albums"), ["Alpha", "Beta"]);
    assert_eq!(rows_of("Songs"), ["One", "Two"]);
    assert!(has_focus(Role::ListBox, "Albums"));
}

#[test]
fn choosing_another_album_leaves_the_one_on_show_to_go_back_to() {
    let mut albums = showing_alpha();

    click_text(&mut albums.ui, "Beta");
    albums.wait_for_songs("Beta");

    assert_eq!(albums.location(), at("b"));
    assert_eq!(albums.left(), [at("a")]);
    assert_eq!(selected_row().as_deref(), Some("Beta"));
}

#[test]
fn choosing_the_album_on_show_leaves_nothing() {
    let mut albums = showing_alpha();

    click_text(&mut albums.ui, "Alpha");
    albums.settle();

    assert_eq!(albums.location(), at("a"));
    assert!(albums.left().is_empty());
}

#[test]
fn moving_through_the_albums_shows_them_in_place() {
    let mut albums = showing_alpha();

    albums.key(NamedKey::ArrowDown);
    albums.wait_for_songs("Beta");

    assert_eq!(albums.location(), at("b"));
    assert!(albums.left().is_empty());
}

#[test]
fn the_secondary_button_shows_the_album_in_place_and_opens_its_menu() {
    let mut albums = showing_alpha();
    let (x, y) = middle_of("Beta");

    right_click(&mut albums.ui, x, y);
    albums.settle();

    assert_eq!(albums.location(), at("b"));
    assert!(albums.left().is_empty());
    assert!(find(Role::MenuItem, "Play Next").is_some());
}

#[test]
fn revealing_an_album_the_filter_hides_clears_the_filter() {
    let mut albums = showing_alpha();
    albums.filter("Alpha");
    albums.wait_for(|| rows_of("Albums") == ["Alpha"]);

    albums.go(at("b"));
    albums.wait_for(|| rows_of("Albums") == ["Alpha", "Beta"]);

    assert_eq!(filter_text().as_deref(), Some(""));
    assert_eq!(selected_row().as_deref(), Some("Beta"));
}

#[test]
fn revealing_an_album_the_filter_shows_keeps_the_filter() {
    let mut albums = showing_alpha();
    albums.filter("Beta");
    albums.wait_for(|| rows_of("Albums") == ["Beta"]);

    albums.go(at("b"));

    assert_eq!(filter_text().as_deref(), Some("Beta"));
    assert_eq!(rows_of("Albums"), ["Beta"]);
    assert_eq!(selected_row().as_deref(), Some("Beta"));
}

#[test]
fn revealing_an_album_moves_the_focus_to_the_album_list() {
    let mut albums = showing_alpha();
    albums.key(NamedKey::ArrowRight);
    assert!(has_focus(Role::ListBox, "Songs"));

    albums.go(at("b"));

    assert!(has_focus(Role::ListBox, "Albums"));
}

#[test]
fn an_album_asked_for_before_the_list_has_it_stays_on_show_until_it_comes() {
    let server = alpha_and_beta();
    server.hold("getAlbumList2");
    let mut albums = on(server, at("b"));
    albums.wait_for_songs("Beta");
    assert_eq!(albums.location(), at("b"));

    albums.server.release("getAlbumList2");
    albums.wait_for_the_albums();

    assert_eq!(albums.location(), at("b"));
    assert_eq!(selected_row().as_deref(), Some("Beta"));
    assert!(has_focus(Role::ListBox, "Albums"));
}

#[test]
fn the_end_of_a_sync_gives_up_on_an_album_it_did_not_bring() {
    let mut albums = on(alpha_and_beta(), at("c"));
    albums.wait_for_the_albums();
    let list = albums.workspace.albums().clone();
    albums.wait_for(|| !list.status().is_updating());
    albums.pause();

    albums
        .server
        .set_albums(vec![album("a", "Alpha", &["One", "Two"])]);
    albums.workspace.refresh(Destination::Albums);
    albums.wait_for(|| rows_of("Albums") == ["Alpha"]);

    let location = albums.location;
    albums.wait_for(|| location.get_untracked() == at("a"));
}

#[test]
fn the_list_says_while_a_sync_brings_it_up_to_date() {
    let mut albums = showing_alpha();
    assert!(find(Role::ProgressBar, "Updating the library").is_none());

    albums.server.hold("getAlbumList2");
    albums.workspace.refresh(Destination::Albums);
    albums.wait_for(|| find(Role::ProgressBar, "Updating the library").is_some());

    albums.server.release("getAlbumList2");
    albums.wait_for(|| find(Role::ProgressBar, "Updating the library").is_none());
}

/// Albums whose server refuses the list, with focus on Retry.
fn failing_with_retry_focused() -> Albums {
    let server = alpha_and_beta();
    server.refuse("getAlbumList2");
    let mut albums = on(server, Location::Albums(None));
    albums.wait_for(|| find(Role::Button, "Retry").is_some());
    assert!(find(Role::Heading, "Couldn't load albums").is_some());
    focus(&mut albums.ui, Role::Button, "Retry");
    albums
}

#[test]
fn enter_presses_a_focused_retry() {
    let mut albums = failing_with_retry_focused();

    albums.key(NamedKey::Enter);

    assert_eq!(albums.sent(), [Command::Refresh]);
}

#[test]
fn space_presses_a_focused_retry() {
    let mut albums = failing_with_retry_focused();
    albums.server.accept("getAlbumList2");

    albums.key(NamedKey::Space);
    albums.wait_for_the_albums();

    assert_eq!(albums.sent(), [Command::Refresh]);
    assert_eq!(rows_of("Albums"), ["Alpha", "Beta"]);
    assert!(has_focus(Role::ListBox, "Albums"));
}

#[test]
fn enter_on_the_play_button_plays_the_whole_album() {
    let mut albums = showing_alpha();
    albums.key(NamedKey::ArrowRight);
    albums.key(NamedKey::ArrowDown);
    focus(&mut albums.ui, Role::Button, "Play");

    albums.key(NamedKey::Enter);

    let sent = albums.sent();
    let [Command::Play { songs, start: None }] = sent.as_slice() else {
        panic!("sent {sent:?}");
    };
    assert_eq!(song_ids(songs), ["a1", "a2"]);
}

#[test]
fn the_play_key_plays_the_whole_album() {
    let mut albums = showing_alpha();

    albums.primary(NamedKey::Enter);

    let sent = albums.sent();
    let [Command::Play { songs, start: None }] = sent.as_slice() else {
        panic!("sent {sent:?}");
    };
    assert_eq!(song_ids(songs), ["a1", "a2"]);
}

#[test]
fn an_album_goes_to_the_queue_where_asked() {
    let mut albums = showing_alpha();

    albums.with(
        NamedKey::Enter,
        Modifiers {
            alt: true,
            ..Modifiers::default()
        },
    );
    albums.with(
        NamedKey::Enter,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );

    let placements: Vec<_> = albums
        .sent()
        .into_iter()
        .map(|command| match command {
            Command::Enqueue { songs, placement } => {
                assert_eq!(song_ids(&songs), ["a1", "a2"]);
                placement
            }
            other => panic!("sent {other:?}"),
        })
        .collect();
    assert_eq!(placements, [Placement::Next, Placement::Last]);
}

#[test]
fn the_playlist_key_asks_which_playlist_takes_the_album() {
    let mut albums = showing_alpha();

    albums.primary_char('p', false);

    let sent = albums.sent();
    let [Command::ChoosePlaylist(songs)] = sent.as_slice() else {
        panic!("sent {sent:?}");
    };
    assert_eq!(song_ids(songs), ["a1", "a2"]);
}

#[test]
fn the_keys_go_to_the_album_or_the_song_with_the_focus() {
    let mut albums = showing_alpha();
    albums.wait_for_the_favorites();
    albums.primary_char('d', false);
    albums.key(NamedKey::ArrowRight);
    albums.key(NamedKey::ArrowDown);
    albums.primary_char('d', false);
    albums.primary_char('a', true);

    let favorites: Vec<_> = albums
        .sent()
        .into_iter()
        .filter_map(|command| match command {
            Command::ToggleFavorite(Favorite::Album(album)) => Some(album.id().to_string()),
            Command::ToggleFavorite(Favorite::Song(song)) => Some(song.id().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(favorites, ["a", "a2"]);
    assert!(
        albums
            .sent
            .borrow()
            .contains(&Command::Show(Location::Artists(Some(ArtistId::new("ar")))))
    );
}

#[test]
fn the_songs_play_from_the_one_opened_and_give_the_keys_back_to_the_albums() {
    let mut albums = showing_alpha();
    albums.key(NamedKey::ArrowRight);
    assert_eq!(selected_row().as_deref(), Some("Alpha"));
    albums.key(NamedKey::ArrowDown);

    albums.key(NamedKey::Enter);
    let sent = albums.sent();
    let [Command::Play { songs, start }] = sent.as_slice() else {
        panic!("sent {sent:?}");
    };
    assert_eq!(song_ids(songs), ["a1", "a2"]);
    assert_eq!(*start, Some(1));

    albums.key(NamedKey::Escape);
    assert!(has_focus(Role::ListBox, "Albums"));
}

#[test]
fn an_action_on_an_album_still_loading_waits_for_its_songs() {
    let server = alpha_and_beta();
    server.hold("getAlbum");
    let mut albums = on(server, Location::Albums(None));
    albums.wait_for(|| find(Role::ProgressBar, "Loading Album").is_some());

    albums.primary(NamedKey::Enter);

    assert_eq!(
        albums.sent(),
        [Command::ActOnAlbum {
            album: AlbumId::new("a"),
            action: AlbumAction::Play,
        }]
    );
}

#[test]
fn an_album_without_songs_is_not_played() {
    let mut albums = on(
        LibraryServer::new(vec![album("a", "Alpha", &[])]),
        Location::Albums(None),
    );
    albums.wait_for(|| has_text("This album has no songs."));

    albums.primary(NamedKey::Enter);
    albums.key(NamedKey::ArrowRight);

    assert!(albums.sent().is_empty());
    assert!(find(Role::Button, "Play").is_some_and(|play| play.disabled));
    assert_eq!(
        *albums.passed.borrow(),
        [
            Keystroke::named(NamedKey::Enter).with_primary(),
            Keystroke::named(NamedKey::ArrowRight),
        ]
    );
    assert!(has_focus(Role::ListBox, "Albums"));
}

#[test]
fn the_list_comes_back_as_wide_as_it_was_left() {
    let mut albums = showing_alpha();
    let first = list_width();
    focus(&mut albums.ui, Role::Separator, "List Width");
    albums.key(NamedKey::ArrowRight);
    let widened = list_width();
    assert!(widened > first, "{widened} from {first}");
    albums.here.set(false);
    albums.go(Location::Home);

    albums.go(at("a"));
    albums.here.set(true);
    albums.settle();
    albums.wait_for_songs("Alpha");

    assert_eq!(list_width(), widened);
}

#[test]
fn a_failed_album_loads_again_on_retry() {
    let server = alpha_and_beta();
    server.refuse("getAlbum");
    let mut albums = on(server, Location::Albums(None));
    albums.wait_for(|| find(Role::Heading, "Couldn't load this album").is_some());

    albums.server.accept("getAlbum");
    press(&mut albums.ui, Role::Button, "Retry");
    albums.wait_for_songs("Alpha");

    assert_eq!(rows_of("Songs"), ["One", "Two"]);
}

/// Alpha of the year `year`, played last at `played`.
fn alpha_of(year: u32, played: &str) -> Value {
    let mut alpha = album("a", "Alpha", &["One", "Two", "Three"]);
    alpha["year"] = json!(year);
    alpha["played"] = json!(played);
    alpha
}

#[test]
fn the_servers_version_of_a_saved_album_changes_it_in_place() {
    let saved = alpha_of(2020, "2026-11-01T20:00:00Z");
    let server = LibraryServer::new(vec![alpha_of(2021, "2026-11-12T20:00:00Z")]);
    server.hold("getAlbum");
    let library = SavedLibrary {
        albums: vec![serde_json::from_value(saved.clone()).unwrap()],
        ..SavedLibrary::default()
    };
    let (core, hiphile, catalog) = signed_in_core_with_library(server.url(), &library);
    catalog
        .put_album(&serde_json::from_value(saved).unwrap())
        .unwrap();
    let mut albums = on_core(server, core, hiphile, at("a"));
    albums.wait_for(|| has_text("2020 · 3 songs · 9:00"));
    focus(&mut albums.ui, Role::ListBox, "Songs");
    albums.key(NamedKey::ArrowDown);
    albums.key(NamedKey::ArrowDown);
    assert_eq!(selected_of("Songs"), ["Two"]);
    let list = find(Role::ListBox, "Songs").expect("the songs").id;

    albums.server.release("getAlbum");
    albums.wait_for(|| has_text("2021 · 3 songs · 9:00"));

    assert_eq!(
        find(Role::ListBox, "Songs").map(|songs| songs.id),
        Some(list)
    );
    assert!(has_focus(Role::ListBox, "Songs"));
    assert_eq!(selected_of("Songs"), ["Two"]);
}

#[test]
fn the_page_keys_move_by_the_albums_in_view() {
    let many = (0..100)
        .map(|at| album(&format!("{at:03}"), &format!("{at:03}"), &["Song"]))
        .collect();
    let mut albums = on(LibraryServer::new(many), Location::Albums(None));
    albums.wait_for_the_albums();
    let list = find(Role::ListBox, "Albums").expect("the list");
    let page = (list.bounds.height / next_ui::row_list::ROW_HEIGHT).floor() as usize;
    assert!(page > 1, "the list shows {page} rows");

    albums.key(NamedKey::PageDown);
    assert_eq!(albums.on_show(), Some(AlbumId::new(format!("{page:03}"))));
    albums.key(NamedKey::PageUp);
    assert_eq!(albums.on_show(), Some(AlbumId::new("000")));
    assert!(albums.left().is_empty());
}

#[test]
fn enter_in_the_filter_goes_to_the_albums() {
    let mut albums = showing_alpha();
    albums.filter("Be");
    albums.wait_for(|| rows_of("Albums") == ["Beta"]);

    albums.key(NamedKey::Enter);

    assert!(has_focus(Role::ListBox, "Albums"));
}

#[test]
fn the_filter_stays_for_the_screen_to_come_again() {
    let mut albums = showing_alpha();
    albums.filter("Beta");
    albums.wait_for(|| rows_of("Albums") == ["Beta"]);
    albums.here.set(false);
    albums.go(Location::Home);

    albums.go(Location::Albums(None));
    albums.here.set(true);
    albums.settle();
    albums.wait_for_songs("Beta");

    assert_eq!(filter_text().as_deref(), Some("Beta"));
    assert_eq!(rows_of("Albums"), ["Beta"]);
    assert_eq!(albums.location(), at("b"));
}

#[test]
fn the_order_is_chosen_in_the_bar() {
    let mut albums = on(
        LibraryServer::new(vec![
            album("a", "Alpha", &["One"]),
            album("b", "Beta", &["Two"]),
        ]),
        Location::Albums(None),
    );
    albums.wait_for_the_albums();

    press(&mut albums.ui, Role::Button, "Sort by Name, Ascending");
    albums.settle();
    press(&mut albums.ui, Role::MenuItem, "Descending");
    albums.wait_for(|| rows_of("Albums") == ["Beta", "Alpha"]);

    assert!(find(Role::Button, "Sort by Name, Descending").is_some());
    // The album on show stays
    assert_eq!(albums.location(), at("a"));
}

/// Albums of a hundred albums, named by their place, at `at`.
fn hundred_albums_at(at: Location) -> Albums {
    let many = (0..100)
        .map(|at| album(&format!("{at:03}"), &format!("{at:03}"), &["Song"]))
        .collect();
    on(LibraryServer::new(many), at)
}

/// Whether the row of `album` shows whole in the album list.
fn is_in_view(album: &str) -> bool {
    let list = find(Role::ListBox, "Albums").expect("the list").bounds;
    find(Role::ListOption, album).is_some_and(|row| {
        let row = row.bounds;
        row.y >= list.y && row.y + row.height <= list.y + list.height
    })
}

#[test]
fn the_list_starts_with_the_first_album_in_view_on_a_screen_of_two_pixels_to_a_point() {
    let mut albums = hundred_albums_at(Location::Albums(None));
    albums.ui.handle_event(WindowEvent::Resized {
        size: SizeI::new(2400, 1520),
        scale: Scale::from_f64(2.0),
    });
    albums.wait_for_the_albums();
    albums.pause();

    assert_eq!(albums.on_show(), Some(AlbumId::new("000")));
    let list = find(Role::ListBox, "Albums").expect("the list").bounds;
    let first = find(Role::ListOption, "000").expect("the first row").bounds;
    assert_eq!(first.y, list.y);
}

#[test]
fn an_album_shown_from_elsewhere_comes_into_view() {
    let mut albums = hundred_albums_at(at("090"));
    albums.wait_for_the_albums();
    albums.pause();

    assert!(is_in_view("090"));
    assert!(has_focus(Role::ListBox, "Albums"));
}
