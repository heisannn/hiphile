use std::cell::RefCell;

use anti_webui::core::Ui;
use anti_webui::core::tree::{NodeKind, with_doc};
use anti_webui_md3e::components::snackbars;
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::{Destination, Location};
use hiphile_core::test_support::{FavoritesServer, PlaylistsServer};
use next_ui::testing::{TestCore, find, frames, nodes, signed_in_core, start, wait_until};
use playback::RepeatMode;
use serde_json::json;
use subsonic::{PlaylistId, StarTarget};

use super::*;
use crate::navigation::location_of;

fn songs(ids: &[&str]) -> Arc<[Song]> {
    ids.iter()
        .map(|id| serde_json::from_value(json!({ "id": id, "title": id })).unwrap())
        .collect()
}

struct Shown {
    ui: Ui,
    shell: Shell,
    _hiphile: Hiphile,
    _core: TestCore,
}

/// The shell of a workspace on `server`, with its dialogs and snackbars and no screen.
fn on(server: &str) -> Shown {
    let (core, hiphile) = signed_in_core(server);
    let made = Rc::new(RefCell::new(None));
    let ui = start({
        let (made, hiphile) = (made.clone(), hiphile.clone());
        move || {
            let snackbars = snackbars();
            let workspace = hiphile.workspace().unwrap();
            let shell = Shell::new(
                &hiphile,
                workspace,
                Navigation::new(),
                signal(false),
                signal(false),
                snackbars,
            );
            let cx = shell.context();
            made.replace(Some(shell.clone()));
            div().child(shell.dialogs.view(&cx)).child(snackbars.view())
        }
    });
    let shell = made.take().unwrap();
    Shown {
        ui,
        shell,
        _hiphile: hiphile,
        _core: core,
    }
}

impl Shown {
    fn run(&mut self, command: Command) {
        self.shell.run(command);
        frames(&mut self.ui, 400);
    }

    fn wait_until(&mut self, done: impl FnMut() -> bool) {
        wait_until(&mut self.ui, done);
    }
}

fn told() -> bool {
    !nodes(Role::Status).is_empty()
}

fn shows_text_with(part: &str) -> bool {
    with_doc(|doc| {
        doc.nodes.iter().any(
            |(_, node)| matches!(&node.kind, NodeKind::Text(shown) if shown.text.contains(part)),
        )
    })
}

fn wait_for_favorites(shown: &mut Shown) {
    let known = shown.shell.known;
    let target = StarTarget::Song(subsonic::SongId::new("s1"));
    shown.wait_until(|| known.is_favorite(&target).is_some());
}

#[test]
fn show_goes_there() {
    let mut shown = on(next_ui::testing::NOWHERE);

    shown.run(Command::Show(Location::Albums(None)));

    assert_eq!(
        untrack(|| shown.shell.navigation.destination()),
        Destination::Albums
    );
}

#[test]
fn showing_in_place_leaves_nowhere_to_go_back_to() {
    let mut shown = on(next_ui::testing::NOWHERE);
    shown.run(Command::Show(Location::Albums(Some(AlbumId::new("a")))));
    let navigation = shown.shell.navigation;
    assert!(untrack(|| navigation.can_go_back()));

    shown.run(Command::ShowInPlace(Location::Albums(Some(AlbumId::new(
        "b",
    )))));
    shown.run(Command::GoBack);

    assert_eq!(
        untrack(|| navigation.location().get()),
        Location::Home,
        "back past the album shown in place"
    );
}

#[test]
fn nothing_to_shuffle_leaves_shuffling_as_it_was() {
    let mut shown = on(next_ui::testing::NOWHERE);
    let player = shown.shell.workspace.player().clone();

    shown.run(Command::Shuffle(songs(&[])));

    assert!(!player.snapshot().is_shuffled());
}

#[test]
fn toggling_a_favorite_stars_it_on_the_server() {
    let server = FavoritesServer::new(&[]);
    let mut shown = on(server.url());
    wait_for_favorites(&mut shown);

    let song = songs(&["s1"])[0].clone();
    shown.run(Command::ToggleFavorite(Favorite::Song(song)));
    shown.wait_until(|| !server.writes().is_empty());

    assert_eq!(server.writes(), ["star id=s1"]);
    assert!(!told());
}

#[test]
fn a_favorite_the_server_refused_is_told() {
    let server = FavoritesServer::new(&[]);
    server.refuse("star");
    let mut shown = on(server.url());
    wait_for_favorites(&mut shown);

    let song = songs(&["s1"])[0].clone();
    shown.run(Command::ToggleFavorite(Favorite::Song(song)));
    shown.wait_until(told);
}

#[test]
fn songs_added_to_a_playlist_are_told() {
    let server = PlaylistsServer::new(&[("p1", "Road Trip", &[])]);
    let mut shown = on(server.url());

    shown.run(Command::AddToPlaylist {
        playlist: PlaylistId::new("p1"),
        songs: songs(&["s1"]),
    });
    shown.wait_until(told);

    // The notice comes as the write starts, not once the server has it
    shown.wait_until(|| server.songs("p1") == ["s1"]);
}

#[test]
fn a_playlist_is_chosen_from_those_there_are() {
    let server = PlaylistsServer::new(&[("p1", "Road Trip", &[])]);
    let mut shown = on(server.url());
    let known = shown.shell.known;
    shown.wait_until(|| !known.playlist_choices().is_empty());

    shown.run(Command::ChoosePlaylist(songs(&["s1"])));

    assert!(find(Role::Heading, "Add to Playlist").is_some());
}

#[test]
fn with_no_playlist_to_choose_a_new_one_is_named() {
    let server = PlaylistsServer::new(&[]);
    let mut shown = on(server.url());

    shown.run(Command::ChoosePlaylist(songs(&["s1"])));

    assert!(find(Role::Heading, "New Playlist").is_some());
}

#[test]
fn no_songs_have_no_playlist_to_go_to() {
    let server = PlaylistsServer::new(&[("p1", "Road Trip", &[])]);
    let mut shown = on(server.url());

    shown.run(Command::ChoosePlaylist(songs(&[])));

    assert!(nodes(Role::Dialog).is_empty());
}

#[test]
fn new_playlist_asks_for_the_name() {
    let mut shown = on(next_ui::testing::NOWHERE);

    shown.run(Command::NewPlaylist(songs(&["s1"])));

    assert!(find(Role::Heading, "New Playlist").is_some());
    assert!(shows_text_with("Starts with “s1”"));
}

#[test]
fn an_action_on_an_album_has_its_songs_first() {
    let server = FavoritesServer::new(&[]);
    let mut shown = on(server.url());

    shown.run(Command::ActOnAlbum {
        album: AlbumId::new("al"),
        action: AlbumAction::AddToPlaylist(None),
    });
    shown.wait_until(|| find(Role::Heading, "New Playlist").is_some());

    assert!(shows_text_with("Starts with 2 songs"));
}

#[test]
fn an_album_that_could_not_be_had_is_told() {
    let mut shown = on(next_ui::testing::NOWHERE);

    shown.run(Command::ActOnAlbum {
        album: AlbumId::new("al"),
        action: AlbumAction::AddToPlaylist(None),
    });
    shown.wait_until(told);

    assert!(nodes(Role::Dialog).is_empty());
}

#[test]
fn refresh_reads_again_what_is_on_show() {
    let server = FavoritesServer::new(&[]);
    let mut shown = on(server.url());
    wait_for_favorites(&mut shown);
    shown.run(Command::Show(location_of(Destination::Favorites)));
    let read = server.reads();

    shown.run(Command::Refresh);
    shown.wait_until(|| server.reads() > read);
}

#[test]
fn saving_the_queue_asks_for_the_name_of_a_playlist_of_its_songs() {
    let mut shown = on(next_ui::testing::NOWHERE);
    let player = shown.shell.workspace.player().clone();
    player.fill_queue(&songs(&["a", "b"]), 1);

    shown.run(Command::SaveQueue);

    assert!(find(Role::Heading, "New Playlist").is_some());
    assert!(shows_text_with("Starts with 2 songs"));
}

#[test]
fn an_empty_queue_is_not_saved() {
    let mut shown = on(next_ui::testing::NOWHERE);

    shown.run(Command::SaveQueue);

    assert!(nodes(Role::Dialog).is_empty());
}

#[test]
fn the_controls_that_play_nothing_change_the_player() {
    let mut shown = on(next_ui::testing::NOWHERE);
    let player = shown.shell.workspace.player().clone();
    player.fill_queue(&songs(&["a", "b"]), 0);
    let volume = player.snapshot().volume();

    shown.run(Command::ToggleShuffle);
    shown.run(Command::SetRepeat(RepeatMode::One));
    shown.run(Command::VolumeDown);
    assert!(player.snapshot().volume() < volume);
    shown.run(Command::VolumeUp);
    shown.run(Command::ToggleMute);

    let after = player.snapshot();
    assert!(after.is_shuffled());
    assert_eq!(after.repeat(), RepeatMode::One);
    assert_eq!(after.volume(), volume);
    assert!(after.is_muted());
    assert!(!after.is_playing());
}

#[test]
fn the_queue_opens_and_closes() {
    let mut shown = on(next_ui::testing::NOWHERE);
    let panels = shown.shell.panels;

    shown.run(Command::ToggleQueue);
    assert!(panels.queue.get_untracked());
    shown.run(Command::ToggleQueue);
    assert!(!panels.queue.get_untracked());
}
