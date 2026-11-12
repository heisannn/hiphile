use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::core::Ui;
use anti_webui::core::access::{AccessAction, AccessNode, perform};
use anti_webui::core::event::Modifiers;
use hiphile_core::Hiphile;
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::Location;
use hiphile_core::songs::SongList;
use hiphile_core::test_support::SongsServer;
use next_ui::testing::{
    TestCore, click, click_with, double_click, find, focus, frames, has_text, key, key_with, nodes,
    press, recording_commands, right_click, screen_context, signed_in_core, start, typed,
    wait_until,
};
use next_ui::{Command, app_bar};
use playback::Placement;
use serde_json::{Value, json};
use subsonic::{AlbumId, ArtistId, Song};

use super::*;

fn song_json(id: &str, title: &str, seconds: u64) -> Value {
    json!({
        "id": id, "title": title, "duration": seconds,
        "artist": format!("Artist of {title}"), "artistId": format!("artist-{id}"),
        "album": format!("Album of {title}"), "albumId": format!("album-{id}"),
    })
}

/// Arrive in another order than they sort in by title, time or id.
fn three() -> Vec<Value> {
    vec![
        song_json("c", "Cherokee", 100),
        song_json("a", "Airegin", 300),
        song_json("b", "Blues", 200),
    ]
}

fn song(fields: &Value) -> Song {
    serde_json::from_value(fields.clone()).unwrap()
}

struct Shown {
    ui: Ui,
    sent: Rc<RefCell<Vec<Command>>>,
    hiphile: Hiphile,
    _core: TestCore,
}

impl Shown {
    fn song_list(&self) -> SongList {
        self.hiphile.workspace().unwrap().songs().clone()
    }

    fn sent(&self) -> Vec<Command> {
        self.sent.take()
    }
}

/// The screen on the songs of `server`, which the workspace reads as the shell shows it.
fn songs_on(server: &SongsServer) -> Shown {
    shown_after(server, |_| {})
}

/// The screen once `before` has done what it does to the workspace on `server`.
fn shown_after(server: &SongsServer, before: impl FnOnce(&Hiphile)) -> Shown {
    let (core, hiphile) = signed_in_core(server.url());
    hiphile.workspace().unwrap().showing(Destination::Songs);
    before(&hiphile);
    let (commands, sent) = recording_commands();
    let ui = start({
        let hiphile = hiphile.clone();
        move || {
            let cx = screen_context(&hiphile, commands);
            let Screen {
                title,
                actions,
                body,
                ..
            } = songs(&cx);
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(app_bar(Vec::new(), title, actions))
                .child(body.flex_1().min_h(0))
        }
    });
    Shown {
        ui,
        sent,
        hiphile,
        _core: core,
    }
}

/// The three songs on show, by title.
fn table() -> (Shown, SongsServer) {
    let server = SongsServer::new(three());
    let mut shown = songs_on(&server);
    wait_for_rows(&mut shown, ["Airegin", "Blues", "Cherokee"]);
    (shown, server)
}

fn row_nodes() -> Vec<AccessNode> {
    let mut rows = nodes(Role::ListOption);
    rows.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));
    rows
}

/// What the rows are called, from the top.
fn rows() -> Vec<String> {
    row_nodes()
        .into_iter()
        .filter_map(|row| row.label)
        .collect()
}

fn selected() -> Vec<String> {
    row_nodes()
        .into_iter()
        .filter(|row| row.selected == Some(true))
        .filter_map(|row| row.label)
        .collect()
}

fn wait_for_rows<const N: usize>(shown: &mut Shown, titles: [&str; N]) {
    wait_until(&mut shown.ui, || rows() == titles);
}

fn middle(node: &AccessNode) -> (f32, f32) {
    let b = node.bounds;
    (b.x + b.width / 2.0, b.y + b.height / 2.0)
}

fn row(title: &str) -> AccessNode {
    row_nodes()
        .into_iter()
        .find(|row| row.label.as_deref() == Some(title))
        .unwrap_or_else(|| panic!("no row {title:?}"))
}

/// Clicks the row of `title` away from its links and buttons.
fn click_row(shown: &mut Shown, title: &str) {
    click_row_with(shown, title, Modifiers::default());
}

fn click_row_with(shown: &mut Shown, title: &str, modifiers: Modifiers) {
    let row = row(title);
    let (x, y) = (row.bounds.x + 40.0, row.bounds.y + row.bounds.height / 2.0);
    click_with(&mut shown.ui, x, y, modifiers);
}

fn heading(label: &str) -> AccessNode {
    find(Role::Button, label).unwrap_or_else(|| panic!("no heading {label:?}"))
}

fn click_heading(shown: &mut Shown, label: &str) {
    let (x, y) = middle(&heading(label));
    click(&mut shown.ui, x, y);
}

/// Types `query` in the filter in place of what it has.
fn filter_by(shown: &mut Shown, query: &str) {
    focus(&mut shown.ui, Role::TextInput, "Filter Songs");
    let (a, primary) = primary('a');
    key_with(&mut shown.ui, a, primary);
    key(&mut shown.ui, NamedKey::Backspace);
    typed(&mut shown.ui, query);
}

fn focus_rows(shown: &mut Shown) {
    focus(&mut shown.ui, Role::ListBox, "Songs");
}

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Modifiers::default()
    }
}

fn shift() -> Modifiers {
    Modifiers::default().with_shift()
}

fn primary(c: char) -> (Key, Modifiers) {
    (Key::Character(c.to_string()), Modifiers::PRIMARY)
}

/// The item of an open menu that says `name`. Its accessible name also has the shortcut after
/// it, which is one word.
fn menu_item(name: &str) -> Option<AccessNode> {
    nodes(Role::MenuItem).into_iter().find(|item| {
        item.label.as_deref().is_some_and(|label| {
            label == name
                || label
                    .strip_prefix(name)
                    .and_then(|rest| rest.strip_prefix(' '))
                    .is_some_and(|key| !key.contains(' '))
        })
    })
}

fn choose(shown: &mut Shown, name: &str) {
    let item = menu_item(name).unwrap_or_else(|| panic!("no menu item {name:?}"));
    assert!(!item.disabled, "{name:?} is disabled");
    perform(item.id, AccessAction::Click);
    shown.ui.update();
}

fn played(command: &Command) -> (Vec<String>, Option<usize>) {
    match command {
        Command::Play { songs, start } => (
            songs.iter().map(|song| song.id().to_string()).collect(),
            *start,
        ),
        other => panic!("{other:?} is no Play"),
    }
}

#[test]
fn while_the_songs_are_read_the_pane_waits() {
    let server = SongsServer::new(three());
    server.hold();
    let mut shown = songs_on(&server);

    wait_until(&mut shown.ui, || server.reads() > 0);
    shown.ui.update();
    assert!(find(Role::ProgressBar, "Loading Songs").is_some());
    assert!(rows().is_empty());

    server.release();
    wait_for_rows(&mut shown, ["Airegin", "Blues", "Cherokee"]);
}

#[test]
fn the_servers_songs_show_by_title_with_the_first_selected() {
    let (_shown, _server) = table();

    assert_eq!(selected(), ["Airegin"]);
    assert!(has_text("3 songs"));
    assert!(find(Role::Link, "Artist of Blues").is_some());
    assert!(find(Role::Link, "Album of Cherokee").is_some());
    assert!(has_text("5:00"));
    assert!(find(Role::Button, "Title, Ascending").is_some());
    assert!(find(Role::Button, "Sort by Title, Ascending").is_some());
}

#[test]
fn a_failed_reading_says_why_and_retry_reads_again() {
    let server = SongsServer::new(three());
    server.refuse();
    let mut shown = songs_on(&server);

    wait_until(&mut shown.ui, || {
        find(Role::Heading, "Songs didn't load").is_some()
    });
    server.accept();
    press(&mut shown.ui, Role::Button, "Retry");

    wait_for_rows(&mut shown, ["Airegin", "Blues", "Cherokee"]);
}

#[test]
fn a_failed_refresh_keeps_the_songs_and_retry_reads_again() {
    let (mut shown, server) = table();
    server.refuse();

    shown.song_list().reload();
    wait_until(&mut shown.ui, || has_text("Songs didn't update"));
    assert_eq!(rows(), ["Airegin", "Blues", "Cherokee"]);
    server.accept();
    server.set_songs([three(), vec![song_json("d", "Dolphin Dance", 400)]].concat());
    press(&mut shown.ui, Role::Button, "Retry");

    wait_for_rows(
        &mut shown,
        ["Airegin", "Blues", "Cherokee", "Dolphin Dance"],
    );
    assert!(!has_text("Songs didn't update"));
}

#[test]
fn reading_again_shows_that_the_songs_are_updating() {
    let (mut shown, server) = table();
    server.hold();

    shown.song_list().reload();
    wait_until(&mut shown.ui, || {
        find(Role::ProgressBar, "Updating Songs").is_some()
    });
    assert_eq!(rows(), ["Airegin", "Blues", "Cherokee"]);

    server.release();
    wait_until(&mut shown.ui, || {
        find(Role::ProgressBar, "Updating Songs").is_none()
    });
}

#[test]
fn a_server_without_songs_says_so() {
    let server = SongsServer::new(Vec::new());
    let mut shown = songs_on(&server);

    wait_until(&mut shown.ui, || has_text("This server has no songs"));
}

#[test]
fn clicking_headings_sorts_and_clicking_again_turns_it_around() {
    let (mut shown, _server) = table();

    click_heading(&mut shown, "Title, Ascending");
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);
    assert!(find(Role::Button, "Title, Descending").is_some());
    click_heading(&mut shown, "Title, Descending");
    wait_for_rows(&mut shown, ["Airegin", "Blues", "Cherokee"]);

    click_heading(&mut shown, "Artist");
    wait_until(&mut shown.ui, || {
        find(Role::Button, "Artist, Ascending").is_some()
    });
    assert!(find(Role::Button, "Title").is_some());
    click_heading(&mut shown, "Artist, Ascending");
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);

    click_heading(&mut shown, "Album");
    wait_until(&mut shown.ui, || {
        find(Role::Button, "Album, Ascending").is_some()
    });

    click_heading(&mut shown, "Time");
    // The rows are already in this order by album
    wait_until(&mut shown.ui, || {
        find(Role::Button, "Sort by Time, Ascending").is_some()
    });
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);
    click_heading(&mut shown, "Time, Ascending");
    wait_for_rows(&mut shown, ["Airegin", "Blues", "Cherokee"]);
}

#[test]
fn a_date_from_the_menu_sorts_through_a_filter_and_a_heading_starts_from_the_top() {
    let dated: Vec<Value> = three()
        .into_iter()
        .zip([
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "2023-01-01T00:00:00Z",
        ])
        .map(|(mut song, created)| {
            song["created"] = json!(created);
            song
        })
        .collect();
    let server = SongsServer::new(dated);
    let mut shown = songs_on(&server);
    wait_for_rows(&mut shown, ["Airegin", "Blues", "Cherokee"]);
    filter_by(&mut shown, "e");
    wait_until(&mut shown.ui, || find(Role::Button, "Sort").is_some());

    press(&mut shown.ui, Role::Button, "Sort");
    press(&mut shown.ui, Role::MenuItem, "Recently Added");
    // Cherokee went in in 2024, Airegin in 2025 and Blues in 2023
    wait_for_rows(&mut shown, ["Airegin", "Cherokee", "Blues"]);
    assert!(find(Role::Button, "Sort by Recently Added, Descending").is_some());
    frames(&mut shown.ui, 400);

    press(
        &mut shown.ui,
        Role::Button,
        "Sort by Recently Added, Descending",
    );
    press(&mut shown.ui, Role::MenuItem, "Ascending");
    wait_for_rows(&mut shown, ["Blues", "Cherokee", "Airegin"]);
    frames(&mut shown.ui, 400);
    // Choosing the order on show again keeps it the way it runs
    press(
        &mut shown.ui,
        Role::Button,
        "Sort by Recently Added, Ascending",
    );
    press(&mut shown.ui, Role::MenuItem, "Recently Added");
    frames(&mut shown.ui, 400);
    wait_until(&mut shown.ui, || {
        find(Role::Button, "Sort by Recently Added, Ascending").is_some()
    });
    assert_eq!(rows(), ["Blues", "Cherokee", "Airegin"]);

    click_heading(&mut shown, "Title");
    wait_for_rows(&mut shown, ["Airegin", "Blues", "Cherokee"]);
}

#[test]
fn a_heading_clicked_during_a_filter_holds_until_the_filter_is_emptied() {
    let (mut shown, _server) = table();
    filter_by(&mut shown, "e");
    wait_until(&mut shown.ui, || find(Role::Button, "Title").is_some());

    click_heading(&mut shown, "Time");
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);

    // Typing on keeps to the heading
    focus(&mut shown.ui, Role::TextInput, "Filter Songs");
    typed(&mut shown.ui, "r");
    wait_for_rows(&mut shown, ["Cherokee"]);
    assert!(find(Role::Button, "Time, Ascending").is_some());
    key(&mut shown.ui, NamedKey::Backspace);
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);
    assert!(find(Role::Button, "Time, Ascending").is_some());

    // A new filter ranks again, with the sort kept for when it is emptied
    filter_by(&mut shown, "");
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);
    filter_by(&mut shown, "e");
    wait_until(&mut shown.ui, || find(Role::Button, "Time").is_some());
    filter_by(&mut shown, "nobody");
    wait_until(&mut shown.ui, || has_text("No songs match “nobody”"));
    assert!(rows().is_empty());
}

#[test]
fn the_filter_starts_from_the_one_the_songs_go_by() {
    let server = SongsServer::new(three());
    let mut shown = shown_after(&server, |hiphile| {
        hiphile.workspace().unwrap().songs().set_query("blues");
    });

    wait_for_rows(&mut shown, ["Blues"]);
    let field = find(Role::TextInput, "Filter Songs").unwrap();
    assert_eq!(field.value_text.as_deref(), Some("blues"));
}

#[test]
fn enter_in_the_filter_goes_on_to_the_rows() {
    let (mut shown, _server) = table();
    filter_by(&mut shown, "blues");
    wait_for_rows(&mut shown, ["Blues"]);

    key(&mut shown.ui, NamedKey::Enter);
    key(&mut shown.ui, NamedKey::Enter);

    assert_eq!(played(&shown.sent()[0]), (vec!["b".to_owned()], Some(0)));
}

#[test]
fn the_first_song_stays_selected_until_one_is_chosen() {
    let (mut shown, _server) = table();

    click_heading(&mut shown, "Time");
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);
    wait_until(&mut shown.ui, || selected() == ["Cherokee"]);

    filter_by(&mut shown, "nothing matches this");
    wait_until(&mut shown.ui, || {
        has_text("No songs match “nothing matches this”")
    });
    filter_by(&mut shown, "");
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);
    assert_eq!(selected(), ["Cherokee"]);
}

#[test]
fn the_selection_follows_its_song_when_the_order_changes() {
    let (mut shown, _server) = table();
    focus_rows(&mut shown);
    key(&mut shown.ui, NamedKey::ArrowDown);
    assert_eq!(selected(), ["Blues"]);

    click_heading(&mut shown, "Time");
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);
    assert_eq!(selected(), ["Blues"]);

    focus_rows(&mut shown);
    key(&mut shown.ui, NamedKey::ArrowDown);
    assert_eq!(selected(), ["Airegin"]);
}

#[test]
fn a_song_clicked_stays_chosen_though_it_was_first() {
    let (mut shown, _server) = table();
    click_row(&mut shown, "Airegin");

    filter_by(&mut shown, "blues");
    wait_for_rows(&mut shown, ["Blues"]);
    assert!(selected().is_empty());

    focus_rows(&mut shown);
    key(&mut shown.ui, NamedKey::Enter);
    // A selection the filter hides is acted on by nothing
    assert!(shown.sent().is_empty());
}

#[test]
fn enter_plays_the_songs_on_show_in_their_order_from_the_selected_one() {
    let (mut shown, _server) = table();
    click_heading(&mut shown, "Time");
    wait_for_rows(&mut shown, ["Cherokee", "Blues", "Airegin"]);
    click_row(&mut shown, "Blues");

    key(&mut shown.ui, NamedKey::Enter);

    let sent = shown.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        played(&sent[0]),
        (
            vec!["c".to_owned(), "b".to_owned(), "a".to_owned()],
            Some(1)
        )
    );
}

#[test]
fn a_double_click_plays_from_its_row() {
    let (mut shown, _server) = table();
    let row = row("Cherokee");

    double_click(&mut shown.ui, row.bounds.x + 40.0, middle(&row).1);

    let sent = shown.sent();
    assert_eq!(
        played(sent.last().unwrap()),
        (
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            Some(2)
        )
    );
}

#[test]
fn a_filter_narrows_what_plays() {
    let (mut shown, _server) = table();
    filter_by(&mut shown, "blues");
    wait_for_rows(&mut shown, ["Blues"]);
    click_row(&mut shown, "Blues");

    key(&mut shown.ui, NamedKey::Enter);

    assert_eq!(played(&shown.sent()[0]), (vec!["b".to_owned()], Some(0)));
}

#[test]
fn songs_selected_together_play_and_queue_by_themselves() {
    let (mut shown, _server) = table();
    click_row(&mut shown, "Airegin");
    click_row_with(&mut shown, "Cherokee", Modifiers::PRIMARY);
    assert_eq!(selected(), ["Airegin", "Cherokee"]);

    key(&mut shown.ui, NamedKey::Enter);
    key_with(&mut shown.ui, Key::Named(NamedKey::Enter), alt());
    key_with(&mut shown.ui, Key::Named(NamedKey::Enter), shift());
    let (p, m) = primary('p');
    key_with(&mut shown.ui, p, m);

    let both: Arc<[Song]> = [song(&three()[1]), song(&three()[0])].into();
    assert_eq!(
        shown.sent(),
        [
            Command::Play {
                songs: both.clone(),
                start: None
            },
            Command::Enqueue {
                songs: both.clone(),
                placement: Placement::Next
            },
            Command::Enqueue {
                songs: both.clone(),
                placement: Placement::Last
            },
            Command::ChoosePlaylist(both),
        ]
    );
}

#[test]
fn the_keys_queue_the_selected_song_and_go_to_its_album_and_artist() {
    let (mut shown, _server) = table();
    focus_rows(&mut shown);

    key_with(&mut shown.ui, Key::Named(NamedKey::Enter), alt());
    key_with(&mut shown.ui, Key::Named(NamedKey::Enter), shift());
    let (l, m) = primary('l');
    key_with(&mut shown.ui, l, m);
    key_with(
        &mut shown.ui,
        Key::Character("A".to_owned()),
        Modifiers::PRIMARY.with_shift(),
    );

    let sent = shown.sent();
    let airegin: Arc<[Song]> = [song(&three()[1])].into();
    assert_eq!(
        sent,
        [
            Command::Enqueue {
                songs: airegin.clone(),
                placement: Placement::Next
            },
            Command::Enqueue {
                songs: airegin,
                placement: Placement::Last
            },
            Command::Show(Location::Albums(Some(AlbumId::new("album-a")))),
            Command::Show(Location::Artists(Some(ArtistId::new("artist-a")))),
        ]
    );
}

#[test]
fn the_favorite_key_goes_to_the_selected_songs() {
    let server = SongsServer::new(three());
    server.star(&["a"]);
    let mut shown = songs_on(&server);
    wait_for_rows(&mut shown, ["Airegin", "Blues", "Cherokee"]);
    wait_until(&mut shown.ui, || {
        nodes(Role::ToggleButton)
            .iter()
            .any(|button| !button.disabled)
    });
    focus_rows(&mut shown);
    let (d, m) = primary('d');

    key_with(&mut shown.ui, d.clone(), m);
    key_with(&mut shown.ui, Key::Named(NamedKey::ArrowDown), shift());
    key_with(&mut shown.ui, d, m);

    assert_eq!(
        shown.sent(),
        [
            Command::ToggleFavorite(Favorite::Song(song(&three()[1]))),
            Command::ToggleFavorite(Favorite::Song(song(&three()[2]))),
        ]
    );
}

#[test]
fn the_secondary_button_opens_the_menu_of_the_selection() {
    let (mut shown, _server) = table();
    click_row(&mut shown, "Airegin");
    click_row_with(&mut shown, "Blues", shift());

    let (x, y) = middle(&row("Blues"));
    right_click(&mut shown.ui, x, y);

    assert_eq!(selected(), ["Airegin", "Blues"]);
    let play = menu_item("Play").expect("the menu");
    assert!(!play.disabled);
    assert!(menu_item("Go to Album").is_none());
    choose(&mut shown, "Add to Queue");
    let two: Arc<[Song]> = [song(&three()[1]), song(&three()[2])].into();
    assert_eq!(
        shown.sent(),
        [Command::Enqueue {
            songs: two,
            placement: Placement::Last
        }]
    );
}

#[test]
fn the_menu_of_a_song_goes_to_its_album_and_plays_from_it() {
    let (mut shown, _server) = table();

    let (x, y) = middle(&row("Blues"));
    right_click(&mut shown.ui, x, y);
    assert_eq!(selected(), ["Blues"]);
    choose(&mut shown, "Go to Album");
    frames(&mut shown.ui, 400);
    right_click(&mut shown.ui, x, y);
    choose(&mut shown, "Play");

    let sent = shown.sent();
    assert_eq!(
        sent[0],
        Command::Show(Location::Albums(Some(AlbumId::new("album-b"))))
    );
    assert_eq!(
        played(&sent[1]),
        (
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            Some(1)
        )
    );
}

#[test]
fn more_on_a_row_outside_the_selection_acts_on_its_song_alone() {
    let (mut shown, _server) = table();
    let more = nodes(Role::Button)
        .into_iter()
        .filter(|button| button.label.as_deref() == Some("More"))
        .max_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y))
        .unwrap();

    let (x, y) = middle(&more);
    click(&mut shown.ui, x, y);
    choose(&mut shown, "Play Next");

    assert_eq!(selected(), ["Airegin"]);
    let cherokee: Arc<[Song]> = [song(&three()[0])].into();
    assert_eq!(
        shown.sent(),
        [Command::Enqueue {
            songs: cherokee,
            placement: Placement::Next
        }]
    );
}

#[test]
fn clicking_an_album_in_a_row_shows_it_and_leaves_the_selection() {
    let (mut shown, _server) = table();

    let (x, y) = middle(&find(Role::Link, "Album of Blues").unwrap());
    click(&mut shown.ui, x, y);

    assert_eq!(
        shown.sent(),
        [Command::Show(Location::Albums(Some(AlbumId::new(
            "album-b"
        ))))]
    );
    assert_eq!(selected(), ["Airegin"]);
}

#[test]
fn the_song_playing_is_marked() {
    let (mut shown, _server) = table();
    let player = shown.hiphile.workspace().unwrap().player().clone();

    player.fill_queue(&[song(&three()[2])], 0);
    player.pretend_playing(true);
    wait_for_rows(&mut shown, ["Airegin", "Blues, playing", "Cherokee"]);

    player.pretend_playing(false);
    wait_for_rows(&mut shown, ["Airegin", "Blues, paused", "Cherokee"]);
}

#[test]
fn instrumentals_say_they_are_skipped_while_they_are() {
    let server = SongsServer::new(vec![
        song_json("a", "Airegin", 300),
        song_json("i", "Airegin (Instrumental)", 300),
    ]);
    let mut shown = songs_on(&server);
    wait_for_rows(&mut shown, ["Airegin", "Airegin (Instrumental)"]);

    shown
        .hiphile
        .settings()
        .queue()
        .set_skip_instrumentals(true);

    wait_for_rows(&mut shown, ["Airegin", "Airegin (Instrumental), skipped"]);
}
