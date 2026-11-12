use std::cell::RefCell;

use anti_webui::prelude::{Role, div};
use anti_webui_md3e::components::snackbars;
use hiphile_core::test_support::FavoritesServer;
use serde_json::json;
use subsonic::PlaylistId;

use super::*;
use anti_webui::core::Ui;

use crate::testing::{NOWHERE, TestCore, nodes, signed_in_core, start, wait_until};

fn songs(ids: &[&str]) -> Arc<[Song]> {
    ids.iter()
        .map(|id| serde_json::from_value(json!({ "id": id, "title": id })).unwrap())
        .collect()
}

fn ids(command: &Command) -> Vec<String> {
    let songs = match command {
        Command::Play { songs, .. }
        | Command::Enqueue { songs, .. }
        | Command::AddToPlaylist { songs, .. }
        | Command::NewPlaylist(songs)
        | Command::ChoosePlaylist(songs) => songs,
        other => panic!("{other:?} has no songs"),
    };
    songs.iter().map(|song| song.id().to_string()).collect()
}

#[test]
fn each_action_on_an_album_is_the_command_for_its_songs() {
    let songs = songs(&["t1", "t2"]);
    let playlist = PlaylistId::new("p1");
    let command = |action: AlbumAction| action.command(songs.clone()).unwrap();

    assert_eq!(
        command(AlbumAction::Play),
        Command::Play {
            songs: songs.clone(),
            start: None
        }
    );
    assert_eq!(
        command(AlbumAction::Enqueue(Placement::Next)),
        Command::Enqueue {
            songs: songs.clone(),
            placement: Placement::Next
        }
    );
    assert_eq!(
        command(AlbumAction::AddToPlaylist(Some(playlist.clone()))),
        Command::AddToPlaylist {
            playlist,
            songs: songs.clone()
        }
    );
    assert_eq!(
        command(AlbumAction::AddToPlaylist(None)),
        Command::NewPlaylist(songs.clone())
    );
    assert_eq!(
        command(AlbumAction::ChoosePlaylist),
        Command::ChoosePlaylist(songs.clone())
    );
}

#[test]
fn an_album_without_songs_is_left_alone() {
    assert_eq!(AlbumAction::Play.command(songs(&[])), None);
}

struct Started {
    _ui: Ui,
    given: Vec<Command>,
    told: bool,
    _core: TestCore,
}

/// What `start` gives for the album "al" on `server`, once it has given something or told why
/// not.
fn album_on(server: &str) -> Started {
    let (core, hiphile) = signed_in_core(server);
    let workspace = hiphile.workspace().unwrap();
    let given = Rc::new(RefCell::new(Vec::new()));
    let mut ui = start({
        let given = given.clone();
        move || {
            let snackbars = snackbars();
            let _ = AlbumAction::Enqueue(Placement::Last).start(
                AlbumId::new("al"),
                &workspace,
                Scope::here(),
                snackbars,
                move |command| given.borrow_mut().push(command),
            );
            div().child(snackbars.view())
        }
    });
    let told = || !nodes(Role::Status).is_empty();
    wait_until(&mut ui, || !given.borrow().is_empty() || told());
    Started {
        given: given.take(),
        told: told(),
        _ui: ui,
        _core: core,
    }
}

#[test]
fn the_songs_of_an_album_come_to_the_action() {
    let server = FavoritesServer::new(&[]);

    let Started { given, told, .. } = album_on(server.url());

    assert_eq!(given.len(), 1);
    assert!(matches!(
        given[0],
        Command::Enqueue {
            placement: Placement::Last,
            ..
        }
    ));
    assert_eq!(ids(&given[0]), ["t1", "t2"]);
    assert!(!told);
}

#[test]
fn why_an_album_could_not_be_had_is_told() {
    let Started { given, told, .. } = album_on(NOWHERE);

    assert!(given.is_empty());
    assert!(told);
}
