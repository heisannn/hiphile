use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::core::geometry::PointF;
use anti_webui::prelude::{Role, Styled as _, div};
use hiphile_core::test_support::{FavoritesServer, PlaylistsServer};
use serde_json::json;
use subsonic::PlaylistId;

use super::*;
use crate::ScreenContext;
use crate::context_menu::ContextMenu;
use crate::testing::{
    NOWHERE, TestCore, find, frames, nodes, press, recording_commands, screen_context,
    signed_in_core, start, wait_until,
};

fn song(id: &str, title: &str) -> Song {
    serde_json::from_value(json!({
        "id": id,
        "title": title,
        "artist": "Artist",
        "artistId": "ar",
        "album": "Album",
        "albumId": "al",
    }))
    .unwrap()
}

fn album() -> Album {
    serde_json::from_value(
        json!({ "id": "al", "name": "Album", "artist": "Artist", "artistId": "ar" }),
    )
    .unwrap()
}

struct Shown {
    ui: Ui,
    cx: ScreenContext,
    menu: ContextMenu,
    sent: Rc<RefCell<Vec<Command>>>,
    _core: TestCore,
}

/// A screen of the workspace on `server` with nothing on it but a context menu.
fn on(server: &str, hiphile_ready: impl FnOnce(&hiphile_core::Hiphile)) -> Shown {
    let (core, hiphile) = signed_in_core(server);
    hiphile_ready(&hiphile);
    let (commands, sent) = recording_commands();
    let menu = ContextMenu::new();
    let made = Rc::new(RefCell::new(None));
    let ui = start({
        let made = made.clone();
        move || {
            let cx = screen_context(&hiphile, commands);
            made.replace(Some(cx.clone()));
            div().size_full().child(menu.view())
        }
    });
    let cx = made.take().unwrap();
    Shown {
        ui,
        cx,
        menu,
        sent,
        _core: core,
    }
}

impl Shown {
    fn open(&mut self, groups: Vec<Vec<MenuItem>>) {
        self.menu.open_at(PointF::new(40.0, 40.0), groups);
        self.ui.update();
    }

    fn wait_for_favorites(&mut self) {
        let known = self.cx.known;
        let target = subsonic::StarTarget::Song(subsonic::SongId::new("s1"));
        wait_until(&mut self.ui, || known.is_favorite(&target).is_some());
    }
}

/// What the menu offers, in order.
fn offered() -> Vec<String> {
    nodes(Role::MenuItem)
        .into_iter()
        .filter_map(|item| item.label)
        .collect()
}

#[test]
fn a_song_offers_what_plays_it_files_it_and_goes_to_it() {
    let server = FavoritesServer::new(&[]);
    let mut shown = on(server.url(), |_| {});
    shown.wait_for_favorites();

    let cx = shown.cx.clone();
    shown.open(song_menu(&cx.commands, cx.known, &song("s1", "Song"), None));

    let offered = offered();
    assert_eq!(offered[..3], ["Play", "Play Next", "Add to Queue"]);
    assert!(offered[3].starts_with("Add to Playlist"));
    assert_eq!(
        offered[4..],
        ["Add to Favorites", "Go to Album", "Go to Artist"]
    );
}

#[test]
fn a_favorite_is_offered_to_be_taken_out() {
    let server = FavoritesServer::new(&["s1"]);
    let mut shown = on(server.url(), |_| {});
    shown.wait_for_favorites();

    let cx = shown.cx.clone();
    shown.open(song_menu(&cx.commands, cx.known, &song("s1", "Song"), None));
    press(&mut shown.ui, Role::MenuItem, "Remove from Favorites");

    assert_eq!(
        *shown.sent.borrow(),
        [Command::ToggleFavorite(Favorite::Song(song("s1", "Song")))]
    );
}

#[test]
fn whether_it_is_a_favorite_is_not_offered_until_known() {
    let mut shown = on(NOWHERE, |_| {});

    let cx = shown.cx.clone();
    shown.open(song_menu(&cx.commands, cx.known, &song("s1", "Song"), None));

    assert!(find(Role::MenuItem, "Add to Favorites").unwrap().disabled);
}

#[test]
fn choosing_an_item_sends_its_command_and_closes_the_menu() {
    let mut shown = on(NOWHERE, |_| {});

    let cx = shown.cx.clone();
    shown.open(song_menu(&cx.commands, cx.known, &song("s1", "Song"), None));
    press(&mut shown.ui, Role::MenuItem, "Play Next");
    frames(&mut shown.ui, 400);

    assert_eq!(
        *shown.sent.borrow(),
        [Command::Enqueue {
            songs: Arc::from([song("s1", "Song")]),
            placement: Placement::Next,
        }]
    );
    assert!(nodes(Role::MenuItem).is_empty());
    assert!(!shown.menu.is_open());
}

#[test]
fn play_is_what_the_screen_says() {
    let mut shown = on(NOWHERE, |_| {});
    let list: Arc<[Song]> = Arc::from([song("s0", "First"), song("s1", "Song")]);
    let play = Command::Play {
        songs: list.clone(),
        start: Some(1),
    };

    let cx = shown.cx.clone();
    shown.open(song_menu(
        &cx.commands,
        cx.known,
        &list[1],
        Some(play.clone()),
    ));
    press(&mut shown.ui, Role::MenuItem, "Play");

    assert_eq!(*shown.sent.borrow(), [play]);
}

#[test]
fn an_instrumental_skipped_is_not_offered_to_play() {
    let mut shown = on(NOWHERE, |hiphile| {
        hiphile.settings().queue().set_skip_instrumentals(true);
    });
    let known = shown.cx.known;
    wait_until(&mut shown.ui, || known.skips_instrumentals());

    let cx = shown.cx.clone();
    shown.open(song_menu(
        &cx.commands,
        cx.known,
        &song("s1", "Song (Instrumental)"),
        None,
    ));

    for label in ["Play", "Play Next", "Add to Queue"] {
        assert!(find(Role::MenuItem, label).unwrap().disabled, "{label}");
    }
}

#[test]
fn the_playlists_are_offered_to_add_to() {
    let server = PlaylistsServer::new(&[("p1", "Road Trip", &[]), ("p2", "Focus", &[])]);
    let mut shown = on(server.url(), |_| {});
    let known = shown.cx.known;
    wait_until(&mut shown.ui, || known.playlist_choices().len() == 2);

    let cx = shown.cx.clone();
    shown.open(song_menu(&cx.commands, cx.known, &song("s1", "Song"), None));
    let add = nodes(Role::MenuItem)
        .into_iter()
        .find(|item| {
            item.label
                .as_deref()
                .is_some_and(|label| label.starts_with("Add to Playlist"))
        })
        .unwrap();
    anti_webui::core::access::perform(add.id, anti_webui::core::access::AccessAction::Click);
    shown.ui.update();
    let offered = offered();
    for label in ["New Playlist…", "Road Trip", "Focus"] {
        assert!(offered.iter().any(|item| item == label), "{label}");
    }
    press(&mut shown.ui, Role::MenuItem, "Focus");

    assert_eq!(
        *shown.sent.borrow(),
        [Command::AddToPlaylist {
            playlist: PlaylistId::new("p2"),
            songs: Arc::from([song("s1", "Song")]),
        }]
    );
}

#[test]
fn an_album_is_acted_on_once_its_songs_are_had() {
    let mut shown = on(NOWHERE, |_| {});

    let cx = shown.cx.clone();
    shown.open(album_menu(&cx.commands, cx.known, &album()));
    assert!(find(Role::MenuItem, "Go to Album").is_none());
    assert!(find(Role::MenuItem, "Go to Artist").is_some());
    press(&mut shown.ui, Role::MenuItem, "Add to Queue");

    assert_eq!(
        *shown.sent.borrow(),
        [Command::ActOnAlbum {
            album: AlbumId::new("al"),
            action: AlbumAction::Enqueue(Placement::Last),
        }]
    );
}

#[test]
fn an_artist_outside_artists_goes_there() {
    let mut shown = on(NOWHERE, |_| {});
    let artist: Artist = serde_json::from_value(json!({ "id": "ar", "name": "Artist" })).unwrap();

    let cx = shown.cx.clone();
    shown.open(artist_menu(&cx.commands, cx.known, &artist));
    press(&mut shown.ui, Role::MenuItem, "Go to Artist");

    assert_eq!(
        *shown.sent.borrow(),
        [Command::Show(Location::Artists(Some(ArtistId::new("ar"))))]
    );
}
