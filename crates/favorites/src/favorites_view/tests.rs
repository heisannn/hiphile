use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use actions::{ChoosePlaylist, EnqueueSongs, ShowAlbum, ShowArtist};
use gpui_kit::{
    Entity, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, TestAppContext, VisualTestContext, point,
};
use hiphile_core::test_support::{self, FavoritesServer, song_json};
use subsonic::{AlbumId, ArtistId};

use super::*;

fn view_on<'a>(
    server: &FavoritesServer,
    cx: &'a mut TestAppContext,
) -> (Entity<Favorites>, &'a mut VisualTestContext) {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        ui::list_navigation::bind_control_keys(cx);
        crate::init(cx);
    });
    let session = test_support::session(server.url());
    let favorites = FavoritesState::new(
        cx.update(|cx| gpui_host::executor(cx)),
        session.client().clone(),
    );
    // Notifications live in the window's root
    let (handle, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Favorites::new(session, favorites, window, cx))
            })
        })
        .unwrap();
    (view, VisualTestContext::from_window(handle, cx).into_mut())
}

fn wait_until(
    view: &Entity<Favorites>,
    cx: &mut VisualTestContext,
    done: impl Fn(&Favorites) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if cx.update(|_, cx| done(view.read(cx))) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for the view");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn is_settled(view: &Favorites) -> bool {
    view.shown.is_loaded() && !view.shown.is_reloading()
}

fn song_ids(view: &Entity<Favorites>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        view.read(cx)
            .lists()
            .songs
            .iter()
            .map(|song| song.id().to_string())
            .collect()
    })
}

fn selected_song(view: &Entity<Favorites>, cx: &mut VisualTestContext) -> Option<StarTarget> {
    cx.update(|_, cx| view.read(cx).selected[Section::Songs.ix()].clone())
}

fn target(id: &str) -> StarTarget {
    StarTarget::Song(SongId::new(id))
}

fn song(id: &str) -> Favorite {
    Favorite::Song(serde_json::from_value(song_json(id)).unwrap())
}

fn set_favorite(
    view: &Entity<Favorites>,
    favorite: Favorite,
    is_favorite: bool,
    cx: &mut VisualTestContext,
) {
    let favorites = cx.update(|_, cx| view.read(cx).favorites.clone());
    favorites.set_favorite(favorite, is_favorite);
    cx.run_until_parked();
}

fn focus(view: &Entity<Favorites>, cx: &mut VisualTestContext) {
    let handle = cx.update(|_, cx| view.read(cx).focus.clone());
    cx.update(|window, cx| handle.focus(window, cx));
    cx.run_until_parked();
}

fn platform_key(
    mac: &'static str,
    other: &'static str,
    cx: &mut VisualTestContext,
) -> &'static str {
    cx.update(|_, cx| {
        if cx.platform_info().is_mac() {
            mac
        } else {
            other
        }
    })
}

fn dispatched_by<A: gpui_kit::Action + Clone>(
    keys: &str,
    view: &Entity<Favorites>,
    cx: &mut VisualTestContext,
) -> A {
    let seen = Rc::new(RefCell::new(None));
    cx.update(|_, cx| {
        let seen = seen.clone();
        cx.on_action(move |action: &A, _| *seen.borrow_mut() = Some(action.clone()));
    });
    cx.simulate_keystrokes(keys);
    wait_until(view, cx, |_| seen.borrow().is_some());
    seen.take().unwrap()
}

fn ids(songs: &[Song]) -> Vec<String> {
    songs.iter().map(|song| song.id().to_string()).collect()
}

/// The middle of the song at `ix`, as last drawn.
fn song_row(view: &Entity<Favorites>, ix: usize, cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.update(|_, cx| {
        let this = view.read(cx);
        let state = this.scrolls[Section::Songs.ix()].0.borrow();
        let list = state.base_handle.bounds();
        let height =
            state.last_item_size.unwrap().contents.height / this.lists().songs.len() as f32;
        point(
            list.origin.x + list.size.width / 2.,
            list.origin.y + height * (ix as f32 + 0.5),
        )
    })
}

fn click(position: Point<Pixels>, button: MouseButton, count: usize, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position,
        button,
        modifiers: Modifiers::default(),
        click_count: count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        button,
        modifiers: Modifiers::default(),
        click_count: count,
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn removing_the_selected_favorite_selects_the_one_that_follows(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a", "b", "c"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    cx.update(|_, cx| view.update(cx, |view, cx| view.select(target("b"), cx)));

    set_favorite(&view, song("b"), false, cx);
    assert_eq!(selected_song(&view, cx), Some(target("c")));

    set_favorite(&view, song("c"), false, cx);
    assert_eq!(selected_song(&view, cx), Some(target("a")));

    set_favorite(&view, song("a"), false, cx);
    assert_eq!(selected_song(&view, cx), None);
    wait_until(&view, cx, is_settled);
}

#[gpui_kit::test]
fn retry_loads_the_favorites_that_failed(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a"]);
    server.refuse("getStarred2");
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, |view| {
        matches!(view.shown.load(), FavoritesLoad::Failed(_))
    });
    server.accept("getStarred2");
    focus(&view, cx);
    cx.update(|window, cx| window.focus_next(cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("enter");
    let os = cx.update(|_, cx| cx.platform_info().os);
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse_with_os("enter", os).unwrap(),
    });
    wait_until(&view, cx, is_settled);

    assert_eq!(song_ids(&view, cx), ["a"]);
}

#[gpui_kit::test]
fn the_key_asks_to_take_the_selected_favorite_out(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    let asked = Rc::new(RefCell::new(Vec::new()));
    // What the app shell keeps, and does with what the key leads to
    cx.update(|_, cx| {
        let known = view.read(cx).shown.known().map(|known| (**known).clone());
        actions::set_known_favorites(known, cx);
        let asked = asked.clone();
        cx.on_action(move |action: &actions::SetFavorite, _| {
            asked.borrow_mut().push(action.clone());
        });
    });
    focus(&view, cx);
    let key = cx.update(|_, cx| {
        if cx.platform_info().is_mac() {
            "cmd-d"
        } else {
            "ctrl-d"
        }
    });

    cx.simulate_keystrokes(key);

    assert_eq!(
        *asked.borrow(),
        [actions::SetFavorite::new(song("a"), false)]
    );
}

#[gpui_kit::test]
fn enter_plays_the_songs_from_the_one_selected(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    focus(&view, cx);
    cx.simulate_keystrokes("down");

    let played: PlaySongs = dispatched_by("enter", &view, cx);

    assert_eq!(ids(played.songs()), ["a", "b"]);
    assert_eq!(played.start_ix(), Some(1));
}

#[gpui_kit::test]
fn enter_shows_the_album_or_the_artist_selected(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&[]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    focus(&view, cx);

    cx.simulate_keystrokes("right");
    let shown: ShowAlbum = dispatched_by("enter", &view, cx);
    assert_eq!(shown.album(), &AlbumId::new("al"));

    cx.simulate_keystrokes("right");
    let shown: ShowArtist = dispatched_by("enter", &view, cx);
    assert_eq!(shown.artist(), &ArtistId::new("ar"));
}

#[gpui_kit::test]
fn the_keys_act_on_the_song_selected(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    focus(&view, cx);
    cx.simulate_keystrokes("down");

    let key = platform_key("cmd-enter", "ctrl-enter", cx);
    let played: PlaySongs = dispatched_by(key, &view, cx);
    assert_eq!(played.start_ix(), Some(1));

    let queued: EnqueueSongs = dispatched_by("alt-enter", &view, cx);
    assert_eq!(ids(queued.songs()), ["b"]);
    assert_eq!(queued.placement(), Placement::Next);

    let queued: EnqueueSongs = dispatched_by("shift-enter", &view, cx);
    assert_eq!(ids(queued.songs()), ["b"]);
    assert_eq!(queued.placement(), Placement::Last);

    let key = platform_key("cmd-p", "ctrl-p", cx);
    let chosen: ChoosePlaylist = dispatched_by(key, &view, cx);
    assert_eq!(ids(chosen.songs()), ["b"]);

    let key = platform_key("cmd-l", "ctrl-l", cx);
    let shown: ShowAlbum = dispatched_by(key, &view, cx);
    assert_eq!(shown.album(), &AlbumId::new("album-b"));

    let key = platform_key("cmd-shift-a", "ctrl-shift-a", cx);
    let shown: ShowArtist = dispatched_by(key, &view, cx);
    assert_eq!(shown.artist(), &ArtistId::new("artist-b"));
}

#[gpui_kit::test]
fn the_keys_act_on_the_songs_of_the_album_selected(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&[]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    focus(&view, cx);
    cx.simulate_keystrokes("right");

    let key = platform_key("cmd-enter", "ctrl-enter", cx);
    let played: PlaySongs = dispatched_by(key, &view, cx);
    assert_eq!(ids(played.songs()), ["t1", "t2"]);
    assert_eq!(played.start_ix(), None);

    let queued: EnqueueSongs = dispatched_by("alt-enter", &view, cx);
    assert_eq!(ids(queued.songs()), ["t1", "t2"]);
    assert_eq!(queued.placement(), Placement::Next);

    let queued: EnqueueSongs = dispatched_by("shift-enter", &view, cx);
    assert_eq!(queued.placement(), Placement::Last);

    let key = platform_key("cmd-p", "ctrl-p", cx);
    let chosen: ChoosePlaylist = dispatched_by(key, &view, cx);
    assert_eq!(ids(chosen.songs()), ["t1", "t2"]);

    let key = platform_key("cmd-shift-a", "ctrl-shift-a", cx);
    let shown: ShowArtist = dispatched_by(key, &view, cx);
    assert_eq!(shown.artist(), &ArtistId::new("artist-al"));
}

#[gpui_kit::test]
fn choosing_a_playlist_in_the_menu_adds_the_selection_to_it(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    focus(&view, cx);
    let playlist = PlaylistId::new("p");
    let added = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let added = added.clone();
        cx.on_action(move |action: &actions::AddToPlaylist, _| {
            added.borrow_mut().push(action.clone());
        });
    });

    cx.dispatch_action(AddSelectionToPlaylist {
        playlist: Some(playlist.clone()),
    });
    cx.run_until_parked();

    let added = added.borrow();
    assert_eq!(added[0].playlist(), &playlist);
    assert_eq!(ids(added[0].songs()), ["a"]);
}

#[gpui_kit::test]
fn the_song_playing_is_the_one_marked(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    let state_of = |id: &str, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let now_playing = view.read(cx).now_playing.clone();
            now_playing.and_then(|now| now.state_of(&SongId::new(id)))
        })
    };

    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_now_playing(Some(SongId::new("b")), true, cx);
        });
    });
    assert_eq!(state_of("a", cx), None);
    assert!(state_of("b", cx).is_some());

    cx.update(|_, cx| view.update(cx, |view, cx| view.set_now_playing(None, false, cx)));
    assert_eq!(state_of("b", cx), None);
}

#[gpui_kit::test]
fn a_click_selects_a_row_and_a_second_plays_it(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    let played = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let played = played.clone();
        cx.on_action(move |action: &PlaySongs, _| played.borrow_mut().push(action.start_ix()));
    });
    let row = song_row(&view, 1, cx);

    click(row, MouseButton::Left, 1, cx);
    assert_eq!(selected_song(&view, cx), Some(target("b")));
    assert!(played.borrow().is_empty());

    click(row, MouseButton::Left, 2, cx);
    assert_eq!(*played.borrow(), [Some(1)]);
}

#[gpui_kit::test]
fn a_right_click_selects_the_row_its_menu_acts_on(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    let row = song_row(&view, 1, cx);

    click(row, MouseButton::Right, 1, cx);

    assert_eq!(selected_song(&view, cx), Some(target("b")));
}

#[gpui_kit::test]
fn the_arrows_move_between_the_kinds_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    focus(&view, cx);
    let section = |cx: &mut VisualTestContext| cx.update(|_, cx| view.read(cx).section);

    cx.simulate_keystrokes("left");
    assert_eq!(section(cx), Section::Songs);
    cx.simulate_keystrokes("right");
    assert_eq!(section(cx), Section::Albums);
    cx.simulate_keystrokes("right");
    assert_eq!(section(cx), Section::Artists);
    cx.simulate_keystrokes("right");
    assert_eq!(section(cx), Section::Artists);
    cx.simulate_keystrokes("left");
    assert_eq!(section(cx), Section::Albums);
}

#[gpui_kit::test]
fn each_kind_keeps_its_own_selection(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server, cx);
    wait_until(&view, cx, is_settled);
    focus(&view, cx);

    cx.simulate_keystrokes("down right left");

    assert_eq!(selected_song(&view, cx), Some(target("b")));
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).selected[Section::Albums.ix()],
            Some(StarTarget::Album(AlbumId::new("al")))
        );
    });
}
