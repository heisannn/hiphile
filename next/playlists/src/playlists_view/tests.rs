use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::core::access::{AccessNode, snapshot};
use anti_webui::core::event::Modifiers;
use hiphile_core::Hiphile;
use hiphile_core::favorites::Favorite;
use hiphile_core::test_support::PlaylistsServer;
use next_ui::cover::install_covers;
use next_ui::row_list::ROW_HEIGHT;
use next_ui::testing::{
    TestCore, click_text, drag, find, focus, frames, has_text, key, key_with, middle_of, nodes,
    press, right_click, screen_context, signed_in_core, start, typed, wait_until,
};
use next_ui::{Commands, Covers};
use playback::Placement;
use serde_json::json;
use subsonic::SongId;

use super::*;

/// The screen at a location of its own, which takes the [`Command::Show`]s it sends.
struct Opened {
    ui: Ui,
    hiphile: Hiphile,
    location: Signal<Location>,
    sent: Rc<RefCell<Vec<Command>>>,
    covers: Covers,
    _core: TestCore,
}

impl Opened {
    fn playlists(&self) -> PlaylistsState {
        self.workspace().playlists().clone()
    }

    fn workspace(&self) -> hiphile_core::Workspace {
        self.hiphile.workspace().expect("signed in")
    }

    fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    fn chosen(&self) -> Location {
        self.location.get_untracked()
    }
}

fn open_at(server: &PlaylistsServer, at: Location) -> Opened {
    let (core, hiphile) = signed_in_core(server.url());
    let location = Rc::new(Cell::new(None));
    let covers = Rc::new(Cell::new(None));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let ui = start({
        let (hiphile, made, sent) = (hiphile.clone(), location.clone(), sent.clone());
        let covers = covers.clone();
        move || {
            install_covers(hiphile.clone());
            let location = signal(at);
            made.set(Some(location));
            let commands = Commands::new(move |command| {
                if let Command::Show(to) = &command {
                    location.set(to.clone());
                }
                sent.borrow_mut().push(command);
            });
            let cx = screen_context(&hiphile, commands);
            covers.set(Some(cx.covers));
            let screen = playlists(location, &cx);
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(screen.body.flex_1().min_h(0))
                .child(cx.snackbars.view())
        }
    });
    Opened {
        ui,
        hiphile,
        location: location.get().expect("made"),
        sent,
        covers: covers.get().expect("made"),
        _core: core,
    }
}

/// `server`'s first playlist on show, with its songs.
fn loaded(server: &PlaylistsServer) -> Opened {
    let mut opened = open_at(server, Location::Playlists(None));
    wait_until(&mut opened.ui, || !songs().is_empty());
    opened
}

/// `alpha` (a) holding songs 1, 2 and 3, and `Beta` (b) song 4, each song titled by its number.
fn two_playlists() -> PlaylistsServer {
    let server = PlaylistsServer::new(&[("a", "alpha", &["1", "2", "3"]), ("b", "Beta", &["4"])]);
    for id in ["1", "2", "3", "4"] {
        server.describe_song(id, json!({ "title": format!("Song {id}") }));
    }
    server
}

/// The options of the list named `list`, from the top.
fn options_in(list: &str) -> Vec<AccessNode> {
    let tree = snapshot();
    let Some(listbox) = tree
        .nodes
        .iter()
        .find(|node| node.role == Role::ListBox && node.label.as_deref() == Some(list))
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut inside = listbox.children.clone();
    while let Some(id) = inside.pop() {
        let Some(node) = tree.nodes.iter().find(|node| node.id == id) else {
            continue;
        };
        if node.role == Role::ListOption {
            found.push(node.clone());
        } else {
            inside.extend(node.children.iter().copied());
        }
    }
    found.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));
    found
}

fn labels_in(list: &str) -> Vec<String> {
    options_in(list)
        .into_iter()
        .filter_map(|option| option.label)
        .collect()
}

fn songs() -> Vec<String> {
    labels_in("Songs")
}

fn selected_in(list: &str) -> Option<String> {
    options_in(list)
        .into_iter()
        .find(|option| option.selected == Some(true))
        .and_then(|option| option.label)
}

fn is_focused(list: &str) -> bool {
    let tree = snapshot();
    tree.focus.is_some_and(|focused| {
        tree.nodes.iter().any(|node| {
            node.id == focused && node.role == Role::ListBox && node.label.as_deref() == Some(list)
        })
    })
}

fn chosen(id: &str) -> Location {
    Location::Playlists(Some(PlaylistId::new(id)))
}

/// Selects the song and gives the songs the focus.
fn select_song(ui: &mut Ui, title: &str) {
    press(ui, Role::ListOption, title);
    focus(ui, Role::ListBox, "Songs");
    assert_eq!(selected_in("Songs").as_deref(), Some(title));
}

fn with(modifiers: impl FnOnce(&mut Modifiers)) -> Modifiers {
    let mut held = Modifiers::default();
    modifiers(&mut held);
    held
}

fn alt_key(ui: &mut Ui, named: NamedKey) {
    key_with(ui, Key::Named(named), with(|m| m.alt = true));
}

fn primary_key(ui: &mut Ui, key: Key) {
    key_with(ui, key, Modifiers::PRIMARY);
}

fn has_dialog(heading: &str) -> bool {
    find(Role::Heading, heading).is_some() || has_text(heading)
}

#[test]
fn the_first_playlist_is_chosen_and_the_list_takes_the_focus() {
    let server = two_playlists();
    let mut opened = loaded(&server);

    assert_eq!(opened.chosen(), chosen("a"));
    assert_eq!(songs(), ["Song 1", "Song 2", "Song 3"]);
    wait_until(&mut opened.ui, || is_focused("Playlists"));
    assert_eq!(selected_in("Playlists").as_deref(), Some("alpha"));
}

#[test]
fn the_playlist_the_location_names_is_shown() {
    let server = two_playlists();
    let mut opened = open_at(&server, chosen("b"));

    wait_until(&mut opened.ui, || songs() == ["Song 4"]);
    assert_eq!(selected_in("Playlists").as_deref(), Some("Beta"));
}

#[test]
fn moving_through_the_list_chooses_each_playlist() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));

    key(&mut opened.ui, NamedKey::ArrowDown);

    assert_eq!(opened.chosen(), chosen("b"));
    wait_until(&mut opened.ui, || songs() == ["Song 4"]);
}

#[test]
fn only_one_playlist_is_chosen_at_a_time() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));

    primary_key(&mut opened.ui, Key::Character("a".into()));

    assert_eq!(opened.chosen(), chosen("a"));
    let selected: Vec<_> = options_in("Playlists")
        .into_iter()
        .filter(|option| option.selected == Some(true))
        .collect();
    assert_eq!(selected.len(), 1);
}

#[test]
fn right_and_enter_go_to_the_songs_and_left_comes_back() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));

    key(&mut opened.ui, NamedKey::ArrowRight);
    assert!(is_focused("Songs"));
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 1"));

    key(&mut opened.ui, NamedKey::ArrowLeft);
    assert!(is_focused("Playlists"));

    key(&mut opened.ui, NamedKey::Enter);
    assert!(is_focused("Songs"));
    key(&mut opened.ui, NamedKey::Escape);
    assert!(is_focused("Playlists"));
}

#[test]
fn the_favorite_key_follows_the_song_selected() {
    let server = two_playlists();
    server.serve_favorites(&[]);
    let mut opened = loaded(&server);
    let favorites = opened.workspace().favorites().clone();
    wait_until(&mut opened.ui, || favorites.snapshot().known().is_some());
    select_song(&mut opened.ui, "Song 2");

    primary_key(&mut opened.ui, Key::Character("d".into()));

    let toggled: Vec<_> = opened
        .sent()
        .into_iter()
        .filter_map(|command| match command {
            Command::ToggleFavorite(Favorite::Song(song)) => Some(song.id().clone()),
            _ => None,
        })
        .collect();
    assert_eq!(toggled, [SongId::new("2")]);
}

#[test]
fn moving_a_song_shows_at_once_and_keeps_it_selected() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    select_song(&mut opened.ui, "Song 1");

    alt_key(&mut opened.ui, NamedKey::ArrowDown);

    wait_until(&mut opened.ui, || songs() == ["Song 2", "Song 1", "Song 3"]);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 1"));
    wait_until(&mut opened.ui, || server.songs("a") == ["2", "1", "3"]);
}

#[test]
fn a_song_at_the_top_stays_there_when_moved_up() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    select_song(&mut opened.ui, "Song 1");

    alt_key(&mut opened.ui, NamedKey::ArrowUp);
    frames(&mut opened.ui, 50);

    assert_eq!(songs(), ["Song 1", "Song 2", "Song 3"]);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 1"));
}

#[test]
fn removing_a_song_selects_the_next_and_undo_puts_it_back() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    select_song(&mut opened.ui, "Song 2");

    key(&mut opened.ui, NamedKey::Backspace);

    wait_until(&mut opened.ui, || songs() == ["Song 1", "Song 3"]);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 3"));
    wait_until(&mut opened.ui, || has_text("Removed “Song 2”"));

    click_text(&mut opened.ui, "Undo");

    wait_until(&mut opened.ui, || songs() == ["Song 1", "Song 2", "Song 3"]);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 2"));
    wait_until(&mut opened.ui, || server.songs("a") == ["1", "2", "3"]);
}

#[test]
fn removing_the_last_song_selects_the_one_before() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    select_song(&mut opened.ui, "Song 3");

    key(&mut opened.ui, NamedKey::Delete);

    wait_until(&mut opened.ui, || songs() == ["Song 1", "Song 2"]);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 2"));
}

#[test]
fn an_older_removal_cant_be_undone() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    select_song(&mut opened.ui, "Song 1");
    key(&mut opened.ui, NamedKey::Backspace);
    wait_until(&mut opened.ui, || songs() == ["Song 2", "Song 3"]);
    key(&mut opened.ui, NamedKey::Backspace);
    wait_until(&mut opened.ui, || songs() == ["Song 3"]);
    wait_until(&mut opened.ui, || has_text("Removed “Song 1”"));

    // Only the first shows, and the second waits behind it
    click_text(&mut opened.ui, "Undo");
    frames(&mut opened.ui, 50);

    assert_eq!(songs(), ["Song 3"]);
}

#[test]
fn songs_answer_to_their_keys() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    select_song(&mut opened.ui, "Song 1");

    key(&mut opened.ui, NamedKey::ArrowDown);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 2"));
    alt_key(&mut opened.ui, NamedKey::ArrowUp);
    wait_until(&mut opened.ui, || songs() == ["Song 2", "Song 1", "Song 3"]);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 2"));
    key(&mut opened.ui, NamedKey::Backspace);
    wait_until(&mut opened.ui, || songs() == ["Song 1", "Song 3"]);

    wait_until(&mut opened.ui, || server.songs("a") == ["1", "3"]);
}

#[test]
fn enter_plays_from_the_song_and_the_queue_keys_add_it() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    select_song(&mut opened.ui, "Song 2");

    key(&mut opened.ui, NamedKey::Enter);
    alt_key(&mut opened.ui, NamedKey::Enter);
    key_with(
        &mut opened.ui,
        Key::Named(NamedKey::Enter),
        with(|m| m.shift = true),
    );

    let sent = opened.sent();
    let [
        Command::Play { songs, start },
        Command::Enqueue {
            songs: next,
            placement: Placement::Next,
        },
        Command::Enqueue {
            songs: last,
            placement: Placement::Last,
        },
    ] = &sent[sent.len() - 3..]
    else {
        panic!("{sent:?}");
    };
    assert_eq!(songs.len(), 3);
    assert_eq!(*start, Some(1));
    assert_eq!(next[0].id(), &SongId::new("2"));
    assert_eq!(last[0].id(), &SongId::new("2"));
}

#[test]
fn play_and_shuffle_play_the_whole_playlist() {
    let server = two_playlists();
    let mut opened = loaded(&server);

    press(&mut opened.ui, Role::Button, "Play");
    press(&mut opened.ui, Role::Button, "Shuffle");

    let sent = opened.sent();
    let [
        Command::Play { songs, start: None },
        Command::Shuffle(shuffled),
    ] = &sent[sent.len() - 2..]
    else {
        panic!("{sent:?}");
    };
    assert_eq!(songs.len(), 3);
    assert_eq!(shuffled, songs);
}

#[test]
fn enter_on_the_play_button_plays() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    focus(&mut opened.ui, Role::Button, "Play");

    key(&mut opened.ui, NamedKey::Enter);
    frames(&mut opened.ui, 50);

    assert!(opened.sent().iter().any(
        |command| matches!(command, Command::Play { songs, start: None } if songs.len() == 3)
    ));
}

#[test]
fn a_command_given_while_songs_load_runs_when_they_arrive() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    server.hold("getPlaylist");
    press(&mut opened.ui, Role::ListOption, "Beta");
    focus(&mut opened.ui, Role::ListBox, "Playlists");

    key_with(
        &mut opened.ui,
        Key::Named(NamedKey::Enter),
        Modifiers::PRIMARY,
    );
    frames(&mut opened.ui, 50);
    let sent = opened.sent.clone();
    let plays = || {
        sent.borrow()
            .iter()
            .filter(|command| matches!(command, Command::Play { .. }))
            .count()
    };
    assert_eq!(plays(), 0);

    server.release("getPlaylist");
    wait_until(&mut opened.ui, || plays() == 1);
}

#[test]
fn the_selected_song_is_followed_when_the_server_moves_it() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    select_song(&mut opened.ui, "Song 2");
    server.set_songs("a", &["2", "1"]);

    opened.playlists().refresh();

    wait_until(&mut opened.ui, || songs() == ["Song 2", "Song 1"]);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 2"));
}

#[test]
fn a_song_dragged_down_lands_after_the_row_dropped_on_its_lower_half() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    let (x, y) = middle_of("Song 3");

    drag(
        &mut opened.ui,
        middle_of("Song 1"),
        (x, y + ROW_HEIGHT / 4.0),
    );

    wait_until(&mut opened.ui, || songs() == ["Song 2", "Song 3", "Song 1"]);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 1"));
    wait_until(&mut opened.ui, || server.songs("a") == ["2", "3", "1"]);
}

#[test]
fn a_song_dragged_up_lands_before_the_row_dropped_on_its_upper_half() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    let (x, y) = middle_of("Song 1");

    drag(
        &mut opened.ui,
        middle_of("Song 3"),
        (x, y - ROW_HEIGHT / 4.0),
    );

    wait_until(&mut opened.ui, || songs() == ["Song 3", "Song 1", "Song 2"]);
    wait_until(&mut opened.ui, || server.songs("a") == ["3", "1", "2"]);
}

#[test]
fn the_songs_of_a_readonly_playlist_cant_be_dragged_moved_or_removed() {
    let server = two_playlists();
    server.make_readonly("a");
    let mut opened = loaded(&server);
    let (x, y) = middle_of("Song 3");

    drag(
        &mut opened.ui,
        middle_of("Song 1"),
        (x, y + ROW_HEIGHT / 4.0),
    );
    select_song(&mut opened.ui, "Song 1");
    alt_key(&mut opened.ui, NamedKey::ArrowDown);
    key(&mut opened.ui, NamedKey::Backspace);
    frames(&mut opened.ui, 50);

    assert_eq!(songs(), ["Song 1", "Song 2", "Song 3"]);
    assert!(has_text("3 songs · Readonly"));
    assert!(server.requests("createPlaylist").is_empty());
}

#[test]
fn the_songs_of_a_playlist_that_diverged_cant_be_dragged_or_moved() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    server.refuse("createPlaylist");
    server.hold("getPlaylist");
    select_song(&mut opened.ui, "Song 1");
    alt_key(&mut opened.ui, NamedKey::ArrowDown);
    let playlists = opened.playlists();
    wait_until(&mut opened.ui, || playlists.snapshot().is_diverged());
    wait_until(&mut opened.ui, || songs() == ["Song 2", "Song 1", "Song 3"]);
    let writes = server.requests("createPlaylist").len();

    assert!(is_focused("Songs"));
    alt_key(&mut opened.ui, NamedKey::ArrowDown);
    let (x, y) = middle_of("Song 3");
    drag(
        &mut opened.ui,
        middle_of("Song 2"),
        (x, y + ROW_HEIGHT / 4.0),
    );
    frames(&mut opened.ui, 50);

    assert_eq!(songs(), ["Song 2", "Song 1", "Song 3"]);
    assert_eq!(server.requests("createPlaylist").len(), writes);
    server.release("getPlaylist");
}

#[test]
fn the_page_keys_move_by_the_songs_in_view() {
    let songs: Vec<String> = (0..100).map(|ix| ix.to_string()).collect();
    let songs: Vec<&str> = songs.iter().map(String::as_str).collect();
    let server = PlaylistsServer::new(&[("a", "alpha", &songs)]);
    for id in &songs {
        server.describe_song(id, json!({ "title": format!("Song {id}") }));
    }
    let mut opened = loaded(&server);
    focus(&mut opened.ui, Role::ListBox, "Songs");
    let height = find(Role::ListBox, "Songs")
        .expect("the songs")
        .bounds
        .height;
    let page = (height / ROW_HEIGHT).floor() as usize;
    assert!(page > 1, "the window shows {page} rows");

    key(&mut opened.ui, NamedKey::Home);
    key(&mut opened.ui, NamedKey::PageDown);
    assert_eq!(selected_in("Songs"), Some(format!("Song {page}")));
    key(&mut opened.ui, NamedKey::PageUp);
    assert_eq!(selected_in("Songs").as_deref(), Some("Song 0"));
}

#[test]
fn choosing_a_playlist_the_filter_hides_empties_the_filter() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    focus(&mut opened.ui, Role::TextInput, "Filter Playlists");
    typed(&mut opened.ui, "alpha");
    wait_until(&mut opened.ui, || labels_in("Playlists") == ["alpha"]);

    opened.location.set(chosen("b"));

    wait_until(&mut opened.ui, || {
        labels_in("Playlists") == ["alpha", "Beta"] && is_focused("Playlists")
    });
    assert_eq!(selected_in("Playlists").as_deref(), Some("Beta"));
    let filter = find(Role::TextInput, "Filter Playlists").expect("the filter");
    assert_eq!(filter.value_text.as_deref().unwrap_or_default(), "");
}

#[test]
fn a_playlist_that_has_gone_leaves_the_one_open() {
    let server = two_playlists();
    let mut opened = open_at(&server, chosen("b"));
    wait_until(&mut opened.ui, || songs() == ["Song 4"]);

    let location = opened.location;
    location.set(chosen("gone"));

    wait_until(&mut opened.ui, || location.get_untracked() == chosen("b"));
    assert_eq!(songs(), ["Song 4"]);
}

#[test]
fn typing_in_the_filter_keeps_the_playlist_chosen() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    focus(&mut opened.ui, Role::TextInput, "Filter Playlists");

    typed(&mut opened.ui, "beta");

    wait_until(&mut opened.ui, || labels_in("Playlists") == ["Beta"]);
    assert_eq!(opened.chosen(), chosen("a"));
    primary_key(&mut opened.ui, Key::Character("a".into()));
    typed(&mut opened.ui, "zzzz");
    wait_until(&mut opened.ui, || has_text("No playlists match “zzzz”"));
    assert_eq!(opened.chosen(), chosen("a"));
}

#[test]
fn deleting_the_chosen_playlist_chooses_the_one_in_its_place() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));

    key_with(
        &mut opened.ui,
        Key::Named(NamedKey::Backspace),
        Modifiers::PRIMARY,
    );
    assert!(has_dialog("Delete “alpha”?"));
    press(&mut opened.ui, Role::Button, "Delete");

    assert_eq!(opened.chosen(), chosen("b"));
    wait_until(&mut opened.ui, || labels_in("Playlists") == ["Beta"]);
    wait_until(&mut opened.ui, || {
        !server.requests("deletePlaylist").is_empty()
    });
}

#[test]
fn cancelling_the_question_deletes_nothing() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));
    key_with(
        &mut opened.ui,
        Key::Named(NamedKey::Backspace),
        Modifiers::PRIMARY,
    );

    press(&mut opened.ui, Role::Button, "Cancel");
    frames(&mut opened.ui, 500);

    assert!(!has_dialog("Delete “alpha”?"));
    assert_eq!(labels_in("Playlists"), ["alpha", "Beta"]);
    assert!(server.requests("deletePlaylist").is_empty());
}

#[test]
fn deleting_the_open_playlist_of_a_list_that_failed_leaves_nothing_chosen() {
    let server = two_playlists();
    server.refuse("getPlaylists");
    let mut opened = open_at(&server, chosen("a"));
    wait_until(&mut opened.ui, || {
        !songs().is_empty() && has_text("Couldn't load playlists")
    });

    press(&mut opened.ui, Role::Button, "More");
    press(&mut opened.ui, Role::MenuItem, "Delete…");
    press(&mut opened.ui, Role::Button, "Delete");

    assert_eq!(opened.chosen(), Location::Playlists(None));
    wait_until(&mut opened.ui, || songs().is_empty());
    wait_until(&mut opened.ui, || {
        !server.requests("deletePlaylist").is_empty()
    });
}

#[test]
fn a_playlist_still_loading_can_be_renamed_and_deleted() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    server.hold("getPlaylist");
    press(&mut opened.ui, Role::ListOption, "Beta");
    wait_until(&mut opened.ui, || {
        find(Role::ProgressBar, "Loading Playlist").is_some()
    });
    focus(&mut opened.ui, Role::ListBox, "Playlists");

    key(&mut opened.ui, NamedKey::F(2));
    assert!(has_dialog("Rename Playlist"));
    frames(&mut opened.ui, 50);
    key(&mut opened.ui, NamedKey::Enter);
    frames(&mut opened.ui, 500);
    assert!(!has_dialog("Rename Playlist"));
    wait_until(&mut opened.ui, || {
        !server.requests("updatePlaylist").is_empty()
    });

    focus(&mut opened.ui, Role::ListBox, "Playlists");
    key_with(
        &mut opened.ui,
        Key::Named(NamedKey::Backspace),
        Modifiers::PRIMARY,
    );
    assert!(has_dialog("Delete “Beta”?"));
    server.release("getPlaylist");
}

#[test]
fn a_playlist_that_wont_load_can_be_deleted() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    server.refuse("getPlaylist");
    press(&mut opened.ui, Role::ListOption, "Beta");
    wait_until(&mut opened.ui, || has_text("Couldn't load this playlist"));
    focus(&mut opened.ui, Role::ListBox, "Playlists");

    key_with(
        &mut opened.ui,
        Key::Named(NamedKey::Backspace),
        Modifiers::PRIMARY,
    );

    assert!(has_dialog("Delete “Beta”?"));
}

#[test]
fn a_playlist_that_wont_load_loads_again_on_retry() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    server.refuse("getPlaylist");
    press(&mut opened.ui, Role::ListOption, "Beta");
    wait_until(&mut opened.ui, || has_text("Couldn't load this playlist"));
    server.accept("getPlaylist");

    press(&mut opened.ui, Role::Button, "Retry");

    wait_until(&mut opened.ui, || songs() == ["Song 4"]);
}

#[test]
fn the_screens_dialogs_cover_the_screens_while_they_are_open() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));
    assert!(!opened.covers.any());

    key(&mut opened.ui, NamedKey::F(2));
    assert!(has_dialog("Rename Playlist"));
    assert!(opened.covers.any());

    key(&mut opened.ui, NamedKey::Escape);
    frames(&mut opened.ui, 500);
    assert!(!opened.covers.any());
}

#[test]
fn enter_in_the_rename_dialog_saves_the_name_and_closes_it() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));
    key(&mut opened.ui, NamedKey::F(2));
    frames(&mut opened.ui, 50);

    typed(&mut opened.ui, "-mix");
    key(&mut opened.ui, NamedKey::Enter);
    frames(&mut opened.ui, 500);

    assert!(!has_dialog("Rename Playlist"));
    wait_until(&mut opened.ui, || labels_in("Playlists")[0] == "alpha-mix");
    wait_until(&mut opened.ui, || {
        !server.requests("updatePlaylist").is_empty()
    });
    assert_eq!(
        server.requests("updatePlaylist")[0].values("name"),
        ["alpha-mix"]
    );
}

#[test]
fn a_blank_name_isnt_saved() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));
    key(&mut opened.ui, NamedKey::F(2));
    frames(&mut opened.ui, 50);

    primary_key(&mut opened.ui, Key::Character("a".into()));
    key(&mut opened.ui, NamedKey::Backspace);
    typed(&mut opened.ui, "  ");
    key(&mut opened.ui, NamedKey::Enter);
    frames(&mut opened.ui, 500);

    assert!(has_dialog("Rename Playlist"));
    assert!(find(Role::Button, "Rename").expect("Rename").disabled);
    assert!(server.requests("updatePlaylist").is_empty());
}

#[test]
fn escape_closes_the_rename_dialog_without_saving() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));
    key(&mut opened.ui, NamedKey::F(2));
    frames(&mut opened.ui, 50);

    typed(&mut opened.ui, " mix");
    key(&mut opened.ui, NamedKey::Escape);
    frames(&mut opened.ui, 500);

    assert!(!has_dialog("Rename Playlist"));
    assert!(server.requests("updatePlaylist").is_empty());
    assert_eq!(labels_in("Playlists")[0], "alpha");
}

#[test]
fn a_readonly_playlist_cant_be_renamed_or_deleted() {
    let server = two_playlists();
    server.make_readonly("a");
    let mut opened = loaded(&server);
    wait_until(&mut opened.ui, || is_focused("Playlists"));

    key(&mut opened.ui, NamedKey::F(2));
    key_with(
        &mut opened.ui,
        Key::Named(NamedKey::Backspace),
        Modifiers::PRIMARY,
    );

    assert!(!has_dialog("Rename Playlist"));
    assert!(!has_dialog("Delete “alpha”?"));
    let (x, y) = {
        let row = &options_in("Playlists")[0];
        (row.bounds.x + 20.0, row.bounds.y + row.bounds.height / 2.0)
    };
    right_click(&mut opened.ui, x, y);
    assert!(find(Role::MenuItem, "Play").is_some());
    assert!(find(Role::MenuItem, "Rename…").is_none());
}

#[test]
fn an_editable_songs_menu_moves_and_removes_it() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    let (x, y) = middle_of("Song 2");

    right_click(&mut opened.ui, x, y);
    press(&mut opened.ui, Role::MenuItem, "Remove from Playlist");

    wait_until(&mut opened.ui, || songs() == ["Song 1", "Song 3"]);
    // A menu that closes takes the presses until it has gone
    frames(&mut opened.ui, 400);
    let (x, y) = middle_of("Song 1");
    right_click(&mut opened.ui, x, y);
    let up = find(Role::MenuItem, "Move Up").expect("Move Up");
    assert!(up.disabled);
    press(&mut opened.ui, Role::MenuItem, "Move Down");
    wait_until(&mut opened.ui, || songs() == ["Song 3", "Song 1"]);
}

#[test]
fn a_playlists_menu_queues_it_once_its_songs_come() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    let row = options_in("Playlists")
        .into_iter()
        .find(|option| option.label.as_deref() == Some("Beta"))
        .expect("Beta");

    right_click(
        &mut opened.ui,
        row.bounds.x + 20.0,
        row.bounds.y + row.bounds.height / 2.0,
    );
    press(&mut opened.ui, Role::MenuItem, "Play Next");

    let sent = opened.sent.clone();
    wait_until(&mut opened.ui, || {
        sent.borrow().iter().any(|command| {
            matches!(
                command,
                Command::Enqueue { songs, placement: Placement::Next }
                    if songs.len() == 1 && songs[0].id() == &SongId::new("4")
            )
        })
    });
}

#[test]
fn the_list_failing_to_load_offers_to_try_again() {
    let server = two_playlists();
    server.refuse("getPlaylists");
    let mut opened = open_at(&server, Location::Playlists(None));
    wait_until(&mut opened.ui, || has_text("Couldn't load playlists"));
    server.accept("getPlaylists");

    press(&mut opened.ui, Role::Button, "Retry");

    let location = opened.location;
    wait_until(&mut opened.ui, || {
        labels_in("Playlists") == ["alpha", "Beta"] && location.get_untracked() == chosen("a")
    });
}

#[test]
fn new_playlist_asks_the_shell() {
    let server = PlaylistsServer::new(&[]);
    let mut opened = open_at(&server, Location::Playlists(None));
    wait_until(&mut opened.ui, || has_text("No playlists yet"));
    assert!(!has_text("0 playlists"));

    press(&mut opened.ui, Role::Button, "New Playlist…");

    assert!(opened.sent().contains(&Command::NewPlaylist(Arc::from([]))));
    assert_eq!(nodes(Role::ListBox).len(), 0);
}

#[test]
fn a_playlist_made_elsewhere_is_chosen_when_shown() {
    let server = two_playlists();
    let mut opened = loaded(&server);
    let playlists = opened.playlists();
    let _created = playlists.create("Road", &[]);
    let road = || {
        playlists
            .snapshot()
            .loaded_list()
            .and_then(|list| list.iter().find(|listed| listed.name() == "Road").cloned())
    };
    wait_until(&mut opened.ui, || road().is_some());
    let road = Location::Playlists(Some(road().unwrap().id().clone()));

    opened.location.set(road.clone());

    wait_until(&mut opened.ui, || {
        selected_in("Playlists").as_deref() == Some("Road")
    });
    assert_eq!(opened.chosen(), road);
}
