use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::core::Ui;
use anti_webui::core::access::{AccessAction, AccessNode, Checked, perform, snapshot};
use hiphile_core::Hiphile;
use hiphile_core::test_support::{FavoritesServer, song_json};
use next_ui::testing::{
    TestCore, click, double_click, find, focus, frames, has_text, key, key_with, middle_of, nodes,
    press, right_click, screen_context, signed_in_core, start, wait_until,
};
use next_ui::{Commands, Screen};
use subsonic::{AlbumId, ArtistId};

use super::*;
use playback::Placement;

struct Shown {
    ui: Ui,
    sent: Rc<RefCell<Vec<Command>>>,
    hiphile: Hiphile,
    _core: TestCore,
}

impl Shown {
    /// The screen on `server`, with the commands it sends kept. A toggle of a favorite goes on to
    /// the favorites, as the shell sends it there.
    fn on(server: &FavoritesServer) -> Self {
        let (core, hiphile) = signed_in_core(server.url());
        let sent = Rc::new(RefCell::new(Vec::new()));
        let toggled = hiphile.workspace().expect("signed in").favorites().clone();
        let commands = Commands::new({
            let sent = sent.clone();
            move |command| {
                if let Command::ToggleFavorite(favorite) = &command {
                    toggled.toggle(favorite.clone());
                }
                sent.borrow_mut().push(command);
            }
        });
        let kept = hiphile.clone();
        let ui = start(move || {
            let cx = screen_context(&kept, commands);
            let Screen { body, .. } = favorites(&cx);
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(body.flex_1().min_h(0))
                .child(cx.snackbars.view())
        });
        Self {
            ui,
            sent,
            hiphile,
            _core: core,
        }
    }

    fn favorites(&self) -> FavoritesState {
        self.hiphile.workspace().unwrap().favorites().clone()
    }

    fn settled(&mut self) {
        let favorites = self.favorites();
        wait_until(&mut self.ui, || {
            let snapshot = favorites.snapshot();
            snapshot.is_loaded() && !snapshot.is_reloading()
        });
        // What the favorites published comes round to the view
        wait_until(&mut self.ui, || {
            find(Role::ListBox, "Favorite songs").is_some()
        });
    }

    fn last_sent(&self) -> Command {
        self.sent.borrow().last().cloned().expect("a command")
    }

    fn keyed(&mut self, key: Key, modifiers: Modifiers) -> Command {
        let before = self.sent.borrow().len();
        key_with(&mut self.ui, key, modifiers);
        assert_eq!(
            self.sent.borrow().len(),
            before + 1,
            "one command for the key"
        );
        self.last_sent()
    }
}

fn options() -> Vec<String> {
    nodes(Role::ListOption)
        .into_iter()
        .filter_map(|node| node.label)
        .collect()
}

fn selected() -> Option<String> {
    nodes(Role::ListOption)
        .into_iter()
        .find(|node| node.selected == Some(true))
        .and_then(|node| node.label)
}

fn selected_tab() -> Option<String> {
    nodes(Role::Tab)
        .into_iter()
        .find(|node| node.selected == Some(true))
        .and_then(|node| node.label)
        .map(|label| label.trim().to_owned())
}

fn focused() -> Option<AccessNode> {
    let tree = snapshot();
    let focus = tree.focus?;
    tree.nodes.into_iter().find(|node| node.id == focus)
}

fn song(id: &str) -> Favorite {
    Favorite::Song(serde_json::from_value(song_json(id)).unwrap())
}

fn ids(songs: &[Song]) -> Vec<String> {
    songs.iter().map(|song| song.id().to_string()).collect()
}

fn primary() -> Modifiers {
    Modifiers::PRIMARY
}

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Modifiers::default()
    }
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::default()
    }
}

fn enter() -> Key {
    Key::Named(NamedKey::Enter)
}

fn char_key(c: &str) -> Key {
    Key::Character(c.to_owned())
}

fn on_the_list(shown: &mut Shown) {
    focus(&mut shown.ui, Role::ListBox, "Favorite songs");
}

#[test]
fn the_first_of_each_kind_is_selected_once_they_load() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();

    assert_eq!(options(), ["a", "b"]);
    assert_eq!(selected().as_deref(), Some("a"));
}

#[test]
fn removing_the_selected_favorite_selects_the_one_that_follows() {
    let server = FavoritesServer::new(&["a", "b", "c"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    let (x, y) = middle_of("b");
    click(&mut shown.ui, x, y);
    assert_eq!(selected().as_deref(), Some("b"));

    let favorites = shown.favorites();
    for (gone, then) in [("b", Some("c")), ("c", Some("a")), ("a", None)] {
        favorites.set_favorite(song(gone), false);
        wait_until(&mut shown.ui, || !options().contains(&gone.to_owned()));
        assert_eq!(selected().as_deref(), then);
    }
    wait_until(&mut shown.ui, || has_text("No favorite songs"));
}

#[test]
fn retry_loads_the_favorites_that_failed() {
    let server = FavoritesServer::new(&["a"]);
    server.refuse("getStarred2");
    let mut shown = Shown::on(&server);
    wait_until(&mut shown.ui, || find(Role::Button, "Retry").is_some());
    assert!(has_text(alerts::FAVORITES_FAILED));

    server.accept("getStarred2");
    press(&mut shown.ui, Role::Button, "Retry");
    shown.settled();

    assert_eq!(options(), ["a"]);
}

#[test]
fn a_kind_with_none_says_how_to_add_one() {
    let server = FavoritesServer::new(&[]);
    let mut shown = Shown::on(&server);
    wait_until(&mut shown.ui, || has_text("No favorite songs"));
    assert!(has_text("Right-click a song to add it."));

    press(&mut shown.ui, Role::Tab, "Albums");
    wait_until(&mut shown.ui, || options() == ["Album"]);
}

#[test]
fn the_key_takes_the_selected_favorite_out() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    on_the_list(&mut shown);

    let sent = shown.keyed(char_key("d"), primary());

    assert!(
        matches!(sent, Command::ToggleFavorite(Favorite::Song(ref song)) if song.id().as_str() == "a")
    );
    wait_until(&mut shown.ui, || options() == ["b"]);
    assert_eq!(selected().as_deref(), Some("b"));
}

#[test]
fn the_key_twice_takes_out_two_favorites() {
    let server = FavoritesServer::new(&["a", "b", "c"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    on_the_list(&mut shown);

    // Both before the first change has come round to the list
    for _ in 0..2 {
        shown
            .ui
            .handle_event(anti_webui::core::event::WindowEvent::Key(
                anti_webui::core::event::KeyEvent {
                    key: char_key("d"),
                    text: None,
                    modifiers: primary(),
                    pressed: true,
                    repeat: false,
                },
            ));
    }
    shown.ui.update();

    wait_until(&mut shown.ui, || options() == ["c"]);
    assert_eq!(selected().as_deref(), Some("c"));
}

#[test]
fn the_favorite_button_takes_its_favorite_out() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    let first = || {
        nodes(Role::ToggleButton)
            .into_iter()
            .next()
            .expect("a button")
    };
    wait_until(&mut shown.ui, || !first().disabled);
    assert_eq!(first().checked, Some(Checked::True));

    press(&mut shown.ui, Role::ToggleButton, "Favorite");

    wait_until(&mut shown.ui, || options() == ["b"]);
}

#[test]
fn enter_plays_the_songs_from_the_one_selected() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    on_the_list(&mut shown);
    key(&mut shown.ui, NamedKey::ArrowDown);

    let Command::Play { songs, start } = shown.keyed(enter(), Modifiers::default()) else {
        panic!("not played");
    };
    assert_eq!(ids(&songs), ["a", "b"]);
    assert_eq!(start, Some(1));
}

#[test]
fn enter_shows_the_album_or_the_artist_selected() {
    let server = FavoritesServer::new(&[]);
    let mut shown = Shown::on(&server);
    wait_until(&mut shown.ui, || has_text("No favorite songs"));
    press(&mut shown.ui, Role::Tab, "Albums");
    wait_until(&mut shown.ui, || options() == ["Album"]);
    focus(&mut shown.ui, Role::ListBox, "Favorite albums");

    let shown_album = shown.keyed(enter(), Modifiers::default());
    assert_eq!(
        shown_album,
        Command::Show(Location::Albums(Some(AlbumId::new("al"))))
    );

    key(&mut shown.ui, NamedKey::ArrowRight);
    assert_eq!(selected_tab().as_deref(), Some("Artists"));
    let shown_artist = shown.keyed(enter(), Modifiers::default());
    assert_eq!(
        shown_artist,
        Command::Show(Location::Artists(Some(ArtistId::new("ar"))))
    );
}

#[test]
fn the_keys_act_on_the_song_selected() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    on_the_list(&mut shown);
    key(&mut shown.ui, NamedKey::ArrowDown);
    let b = |songs: &Arc<[Song]>| ids(songs) == ["b"];

    let played = shown.keyed(enter(), primary());
    assert!(matches!(played, Command::Play { start: Some(1), .. }));

    let next = shown.keyed(enter(), alt());
    assert!(matches!(next, Command::Enqueue { ref songs, placement: Placement::Next } if b(songs)));

    let last = shown.keyed(enter(), shift());
    assert!(matches!(last, Command::Enqueue { ref songs, placement: Placement::Last } if b(songs)));

    let chosen = shown.keyed(char_key("p"), primary());
    assert!(matches!(chosen, Command::ChoosePlaylist(ref songs) if b(songs)));

    let album = shown.keyed(char_key("l"), primary());
    assert_eq!(
        album,
        Command::Show(Location::Albums(Some(AlbumId::new("album-b"))))
    );

    let artist = shown.keyed(
        char_key("a"),
        Modifiers {
            shift: true,
            ..primary()
        },
    );
    assert_eq!(
        artist,
        Command::Show(Location::Artists(Some(ArtistId::new("artist-b"))))
    );
}

#[test]
fn the_keys_act_on_the_songs_of_the_album_selected() {
    let server = FavoritesServer::new(&["a"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    on_the_list(&mut shown);
    key(&mut shown.ui, NamedKey::ArrowRight);
    wait_until(&mut shown.ui, || options() == ["Album"]);
    let on_album = |action: AlbumAction| Command::ActOnAlbum {
        album: AlbumId::new("al"),
        action,
    };

    assert_eq!(shown.keyed(enter(), primary()), on_album(AlbumAction::Play));
    assert_eq!(
        shown.keyed(enter(), alt()),
        on_album(AlbumAction::Enqueue(Placement::Next))
    );
    assert_eq!(
        shown.keyed(enter(), shift()),
        on_album(AlbumAction::Enqueue(Placement::Last))
    );
    assert_eq!(
        shown.keyed(char_key("p"), primary()),
        on_album(AlbumAction::ChoosePlaylist)
    );
    assert_eq!(
        shown.keyed(
            char_key("a"),
            Modifiers {
                shift: true,
                ..primary()
            }
        ),
        Command::Show(Location::Artists(Some(ArtistId::new("artist-al"))))
    );
}

#[test]
fn the_menu_adds_the_song_it_is_on_to_a_new_playlist() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    let (x, y) = middle_of("b");

    right_click(&mut shown.ui, x, y);
    frames(&mut shown.ui, 300);
    assert_eq!(selected().as_deref(), Some("b"));
    let add = nodes(Role::MenuItem)
        .into_iter()
        .find(|item| {
            item.label
                .as_deref()
                .is_some_and(|label| label.starts_with("Add to Playlist"))
        })
        .expect("Add to Playlist");
    perform(add.id, AccessAction::Click);
    shown.ui.update();
    press(&mut shown.ui, Role::MenuItem, "New Playlist…");

    assert!(matches!(shown.last_sent(), Command::NewPlaylist(ref songs) if ids(songs) == ["b"]));
}

#[test]
fn the_more_button_opens_the_menu_of_its_row() {
    let server = FavoritesServer::new(&["a"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    press(&mut shown.ui, Role::Tab, "Albums");
    wait_until(&mut shown.ui, || options() == ["Album"]);

    press(&mut shown.ui, Role::Button, "More");
    frames(&mut shown.ui, 300);
    press(&mut shown.ui, Role::MenuItem, "Show in Albums");

    assert_eq!(
        shown.last_sent(),
        Command::Show(Location::Albums(Some(AlbumId::new("al"))))
    );
}

#[test]
fn the_song_playing_is_the_one_marked() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    let player = shown.hiphile.workspace().unwrap().player().clone();
    let Favorite::Song(b) = song("b") else {
        unreachable!()
    };

    player.fill_queue(&[b], 0);
    player.pretend_playing(true);
    wait_until(&mut shown.ui, || options() == ["a", "b, playing"]);

    player.pretend_playing(false);
    wait_until(&mut shown.ui, || options() == ["a", "b, paused"]);
}

#[test]
fn a_click_selects_a_row_and_a_second_plays_it() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    let (x, y) = middle_of("b");

    click(&mut shown.ui, x, y);
    assert_eq!(selected().as_deref(), Some("b"));
    assert!(shown.sent.borrow().is_empty());

    double_click(&mut shown.ui, x, y);
    assert!(matches!(
        shown.last_sent(),
        Command::Play { start: Some(1), .. }
    ));
}

#[test]
fn the_arrows_move_between_the_kinds_and_stop_at_the_ends() {
    let server = FavoritesServer::new(&["a"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    on_the_list(&mut shown);

    for (arrow, then) in [
        (NamedKey::ArrowLeft, "Songs"),
        (NamedKey::ArrowRight, "Albums"),
        (NamedKey::ArrowRight, "Artists"),
        (NamedKey::ArrowRight, "Artists"),
        (NamedKey::ArrowLeft, "Albums"),
    ] {
        key(&mut shown.ui, arrow);
        assert_eq!(selected_tab().as_deref(), Some(then));
        assert_eq!(
            focused().map(|node| node.role),
            Some(Role::ListBox),
            "the list keeps the focus"
        );
    }
}

#[test]
fn each_kind_keeps_its_own_selection() {
    let server = FavoritesServer::new(&["a", "b"]);
    let mut shown = Shown::on(&server);
    shown.settled();
    on_the_list(&mut shown);

    key(&mut shown.ui, NamedKey::ArrowDown);
    key(&mut shown.ui, NamedKey::ArrowRight);
    assert_eq!(selected().as_deref(), Some("Album"));
    key(&mut shown.ui, NamedKey::ArrowLeft);

    assert_eq!(selected().as_deref(), Some("b"));
}
