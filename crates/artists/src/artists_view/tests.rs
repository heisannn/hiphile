use std::cell::RefCell;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::rc::Rc;
use std::time::Duration;

use actions::{EnqueueSongs, PlaySongs, ShowAlbum, ShowArtist};
use gpui_kit::{KeyUpEvent, Keystroke, TestAppContext, VisualTestContext, px};
use hiphile_core::artists::ArtistSection;
use hiphile_core::favorites::Favorite;
use hiphile_core::test_support;
use serde_json::{Value, json};
use subsonic::{Album, AlbumId, Song};

use super::detail::{Part, same_place};
use super::*;
use ui::card_grid::card_row_height;

/// Past the delay before the detail asks the server.
const SERVER_DELAY: Duration = Duration::from_millis(250);

fn artist(id: &str, name: &str) -> Artist {
    Artist::new(ArtistId::new(id), name.to_owned(), None, Some(1))
}

fn album_json(id: &str, year: u32) -> Value {
    json!({
        "id": id, "name": format!("Album {id}"), "year": year, "songCount": 2,
        "artistId": "m", "artist": "Miles Davis",
    })
}

fn song_json(ix: usize) -> Value {
    json!({
        "id": format!("s{ix}"), "title": format!("Song {ix}"), "albumId": "a1",
        "artistId": "m", "artist": "Miles Davis",
    })
}

fn songs(count: usize) -> Vec<Song> {
    (0..count)
        .map(|ix| serde_json::from_value(song_json(ix)).unwrap())
        .collect()
}

fn answer(method: &str) -> Option<Value> {
    Some(match method {
        "getArtists" => json!({ "artists": { "index": [
            { "name": "B", "artist": [{ "id": "b", "name": "Bill Evans", "albumCount": 1 }] },
            { "name": "M", "artist": [{ "id": "m", "name": "Miles Davis", "albumCount": 2 }] },
        ]}}),
        "getArtist" => json!({ "artist": {
            "id": "m", "name": "Miles Davis",
            "album": [album_json("a1", 1959), album_json("a2", 1970)],
        }}),
        "getTopSongs" => {
            let songs: Vec<_> = (0..10).map(song_json).collect();
            json!({ "topSongs": { "song": songs } })
        }
        "getArtistInfo2" => json!({ "artistInfo2": { "similarArtist": [
            { "id": "b", "name": "Bill Evans", "albumCount": 1 },
        ]}}),
        "getAlbum" => json!({ "album": {
            "id": "a1", "name": "An album", "song": [song_json(0), song_json(1)],
        }}),
        _ => return None,
    })
}

/// A server that knows of one artist's whole detail, returning its URL.
fn server() -> String {
    server_answering(answer)
}

fn server_answering(answer: fn(&str) -> Option<Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            request.read_line(&mut first).ok();
            let mut line = String::new();
            while request.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let method = first
                .split_whitespace()
                .nth(1)
                .and_then(|path| path.split("/rest/").nth(1))
                .and_then(|path| path.split(['?', '.']).next())
                .unwrap_or_default();
            let mut body = json!({ "status": "ok", "version": "1.16.1" });
            if let Some(Value::Object(fields)) = answer(method) {
                body.as_object_mut().unwrap().extend(fields);
            }
            let body = json!({ "subsonic-response": body }).to_string();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len(),
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(body.as_bytes()).ok();
        }
    });
    url
}

fn view_on<'a>(
    url: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<Artists>, &'a mut VisualTestContext) {
    view_with(url, None, cx)
}

fn view_with<'a>(
    url: &str,
    catalog: Option<catalog::Catalog>,
    cx: &'a mut TestAppContext,
) -> (Entity<Artists>, &'a mut VisualTestContext) {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        ui::list_navigation::bind_control_keys(cx);
        crate::init(cx);
    });
    let session = test_support::session_saving_into(catalog, url);
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let list = ArtistList::new(executor.clone(), session.clone());
    let detail = ArtistDetail::new(executor, session.clone());
    cx.add_window_view(|window, cx| Artists::new(session, list, detail, window, cx))
}

/// A screen that has yet to show, whose server cannot be reached.
fn view(cx: &mut TestAppContext) -> (Entity<Artists>, &mut VisualTestContext) {
    view_on("http://127.0.0.1:9", cx)
}

fn update(
    view: &Entity<Artists>,
    cx: &mut VisualTestContext,
    update: impl FnOnce(&mut Artists, &mut Context<Artists>),
) {
    cx.update(|_, cx| view.update(cx, update));
    cx.run_until_parked();
}

/// Waits for `done` to hold, past the delay before asking an outside service.
fn wait_until(view: &Entity<Artists>, cx: &mut VisualTestContext, done: impl Fn(&Artists) -> bool) {
    for _ in 0..500 {
        cx.executor().advance_clock(SERVER_DELAY);
        cx.run_until_parked();
        if cx.update(|_, cx| done(view.read(cx))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("what was waited for never happened");
}

/// Puts `artists` on show as the list would without a filter, while its own stay as they are.
fn set_artists(view: &mut Artists, artists: Vec<Artist>, cx: &mut Context<Artists>) {
    let shown = test_support::shown_in_order(artists);
    view.artists = Shelf::Loaded(shown.items().clone());
    view.show_list(Some(shown), cx);
}

/// Shows artist `m`, in place of what the detail asks the server, which answers only once the
/// clock moves.
fn show(view: &mut Artists, top_songs: usize, similar: Vec<Artist>, cx: &mut Context<Artists>) {
    view.is_activated = true;
    view.is_refreshing = false;
    let artists = vec![artist("b", "Bill Evans"), artist("m", "Miles Davis")];
    set_artists(view, artists, cx);
    view.select_artist(ArtistId::new("m"), cx);
    let detail = view.detail.as_mut().unwrap();
    detail.albums = Shelf::Loaded(Arc::from([
        serde_json::from_value(album_json("a2", 1970)).unwrap(),
        serde_json::from_value(album_json("a1", 1959)).unwrap(),
    ]));
    detail.top_songs = Shelf::Loaded(songs(top_songs).into());
    detail.similar = Shelf::Loaded(similar.into());
    cx.notify();
}

fn dispatched_by<A: gpui_kit::Action + Clone>(
    keys: &str,
    focus: impl Fn(&Artists) -> FocusHandle,
    view: &Entity<Artists>,
    cx: &mut VisualTestContext,
) -> Vec<A> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = seen.clone();
        cx.on_action(move |action: &A, _| seen.borrow_mut().push(action.clone()));
        let focus = focus(view.read(cx));
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    seen.take()
}

#[gpui_kit::test]
fn nothing_loads_until_the_screen_shows(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let view = view.read(cx);
        assert!(!view.artist_list.status().is_refreshing());
        assert!(!view.is_refreshing);
        assert!(view.detail.is_none());
    });
}

#[gpui_kit::test]
fn showing_the_screen_fills_the_list_and_the_first_artists_detail(cx: &mut TestAppContext) {
    let url = server();
    let (view, cx) = view_on(&url, cx);
    update(&view, cx, |view, cx| view.activate(cx));

    wait_until(&view, cx, |view| {
        let Some(detail) = &view.detail else {
            return false;
        };
        !detail.top_songs.items().is_empty()
            && !detail.similar.items().is_empty()
            && !detail.albums.items().is_empty()
    });

    cx.update(|_, cx| {
        let view = view.read(cx);
        assert_eq!(view.visible_len(), 2);
        assert_eq!(view.selected_artist, Some(ArtistId::new("b")));
        let detail = view.detail.as_ref().unwrap();
        // The server here answers with the same artist for any ID
        let albums: Vec<_> = detail.albums.items().iter().map(Album::year).collect();
        assert_eq!(albums, [Some(1970), Some(1959)]);
        assert_eq!(detail.top_songs.items().len(), 10);
    });
}

#[gpui_kit::test]
fn a_failed_section_leaves_the_rest_of_the_detail(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 3, Vec::new(), cx);
        view.detail.as_mut().unwrap().top_songs = Shelf::Failed("offline".to_owned());
        let parts = view.parts();
        assert!(parts.contains(&Part::TopSongsStatus));
        assert!(parts.contains(&Part::AlbumRow(0..2)));
    });
}

#[gpui_kit::test]
fn nothing_to_say_takes_top_songs_and_similar_artists_away(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 0, Vec::new(), cx);
        view.columns = 2;
        assert_eq!(
            view.parts(),
            [Part::Header, Part::AlbumsHeading, Part::AlbumRow(0..2)]
        );
    });
}

#[gpui_kit::test]
fn five_top_songs_show_until_see_more_is_pressed(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    let top_songs = |view: &Artists| {
        let parts = view.parts();
        let songs = parts.iter().filter(|part| matches!(part, Part::Song(_)));
        (songs.count(), parts.contains(&Part::TopSongsToggle))
    };
    update(&view, cx, |view, cx| {
        show(view, 10, Vec::new(), cx);
        assert_eq!(top_songs(view), (5, true));
        view.toggle_top_songs(cx);
        assert_eq!(top_songs(view), (10, true));
        view.toggle_top_songs(cx);
        assert_eq!(top_songs(view), (5, true));
    });
}

#[gpui_kit::test]
fn see_more_goes_away_with_no_more_to_see(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 5, Vec::new(), cx);
        assert!(!view.parts().contains(&Part::TopSongsToggle));
    });
}

/// Presses `key` on the button that Tab reaches `tabs` stops after the detail.
fn press_button_in_detail(
    tabs: usize,
    key: &str,
    view: &Entity<Artists>,
    cx: &mut VisualTestContext,
) {
    cx.update(|window, cx| {
        let focus = view.read(cx).detail_focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    for _ in 0..tabs {
        cx.update(|window, cx| window.focus_next(cx));
        cx.run_until_parked();
    }
    cx.simulate_keystrokes(key);
    let os = cx.update(|_, cx| cx.platform_info().os);
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse_with_os(key, os).unwrap(),
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_see_more_button_works_from_the_keyboard(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| show(view, 10, Vec::new(), cx));

    // Past Play and the menu
    press_button_in_detail(3, "enter", &view, cx);
    cx.update(|_, cx| assert!(view.read(cx).top_songs_expanded));

    press_button_in_detail(3, "space", &view, cx);
    cx.update(|_, cx| assert!(!view.read(cx).top_songs_expanded));
}

#[gpui_kit::test]
fn another_artist_starts_with_five_top_songs(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 10, Vec::new(), cx);
        view.toggle_top_songs(cx);
        view.select_artist(ArtistId::new("b"), cx);
        assert!(!view.top_songs_expanded);
    });
}

#[gpui_kit::test]
fn commands_leave_a_song_that_show_less_took_off_the_screen(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 10, Vec::new(), cx);
        view.toggle_top_songs(cx);
        view.selection = Some(Item::Song(SongId::new("s9")));
        assert!(view.selection_on_screen().is_some());
        view.toggle_top_songs(cx);
        assert_eq!(view.selection_on_screen(), None);
    });
}

fn select(view: &Entity<Artists>, item: Item, cx: &mut VisualTestContext) {
    update(view, cx, |view, cx| {
        view.selection = Some(item);
        cx.notify();
    });
}

#[gpui_kit::test]
fn enter_plays_only_the_top_songs_on_show(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| show(view, 10, Vec::new(), cx));
    select(&view, Item::Song(SongId::new("s2")), cx);

    let played: Vec<PlaySongs> = dispatched_by("enter", |v| v.detail_focus.clone(), &view, cx);

    assert_eq!(played.len(), 1);
    assert_eq!(played[0].songs().len(), 5);
    assert_eq!(played[0].start_ix(), Some(2));
}

#[gpui_kit::test]
fn enter_shows_the_album_or_the_artist_selected(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    let similar = vec![artist("b", "Bill Evans")];
    update(&view, cx, |view, cx| show(view, 1, similar, cx));

    select(&view, Item::Album(AlbumId::new("a1")), cx);
    let albums: Vec<ShowAlbum> = dispatched_by("enter", |v| v.detail_focus.clone(), &view, cx);
    assert_eq!(albums[0].album(), &AlbumId::new("a1"));

    select(&view, Item::Artist(ArtistId::new("b")), cx);
    let artists: Vec<ShowArtist> = dispatched_by("enter", |v| v.detail_focus.clone(), &view, cx);
    assert_eq!(artists[0].artist(), &ArtistId::new("b"));
}

#[gpui_kit::test]
fn the_arrows_move_through_songs_then_cards(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 1, vec![artist("b", "Bill Evans")], cx);
        view.columns = 2;
    });
    cx.update(|window, cx| {
        let focus = view.read(cx).list_focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();

    // Into the detail, where the song comes first
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let view = view.read(cx);
        assert!(view.detail_focus.is_focused(window));
        assert_eq!(view.selection, Some(Item::Song(SongId::new("s0"))));
    });

    cx.simulate_keystrokes("down right");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).selection,
            Some(Item::Album(AlbumId::new("a1")))
        );
    });

    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).selection,
            Some(Item::Artist(ArtistId::new("b")))
        );
    });

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| assert!(view.read(cx).list_focus.is_focused(window)));
}

#[gpui_kit::test]
fn the_go_to_artist_key_follows_the_song_selected(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| show(view, 1, Vec::new(), cx));
    select(&view, Item::Song(SongId::new("s0")), cx);

    let key = if cfg!(target_os = "macos") {
        "cmd-shift-a"
    } else {
        "ctrl-shift-a"
    };
    let artists: Vec<ShowArtist> = dispatched_by(key, |v| v.detail_focus.clone(), &view, cx);

    assert_eq!(artists[0].artist(), &ArtistId::new("m"));
}

#[gpui_kit::test]
fn the_favorite_key_follows_what_is_selected_in_each_pane(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 1, vec![artist("x", "Similar")], cx);
    });
    cx.update(|_, cx| actions::set_known_favorites(Some(Default::default()), cx));
    let key = if cfg!(target_os = "macos") {
        "cmd-d"
    } else {
        "ctrl-d"
    };

    let asked: Vec<actions::ToggleFavoriteOf> =
        dispatched_by(key, |view| view.list_focus.clone(), &view, cx);
    assert_eq!(
        asked[0].favorite().target(),
        subsonic::StarTarget::Artist(ArtistId::new("m"))
    );

    for (item, target) in [
        (
            Item::Song(SongId::new("s0")),
            subsonic::StarTarget::Song(SongId::new("s0")),
        ),
        (
            Item::Album(AlbumId::new("a1")),
            subsonic::StarTarget::Album(AlbumId::new("a1")),
        ),
        (
            Item::Artist(ArtistId::new("x")),
            subsonic::StarTarget::Artist(ArtistId::new("x")),
        ),
    ] {
        select(&view, item, cx);
        let asked: Vec<actions::ToggleFavoriteOf> =
            dispatched_by(key, |view| view.detail_focus.clone(), &view, cx);
        assert_eq!(asked.last().unwrap().favorite().target(), target);
    }
}

#[gpui_kit::test]
fn an_artist_cannot_become_a_favorite_before_their_name_is_known(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 0, Vec::new(), cx);
        view.select_artist(ArtistId::new("unlisted"), cx);
    });
    cx.update(|_, cx| actions::set_known_favorites(Some(Default::default()), cx));
    let key = if cfg!(target_os = "macos") {
        "cmd-d"
    } else {
        "ctrl-d"
    };

    let asked: Vec<actions::ToggleFavoriteOf> =
        dispatched_by(key, |view| view.list_focus.clone(), &view, cx);
    assert!(asked.is_empty());

    update(&view, cx, |view, cx| {
        view.detail.as_mut().unwrap().name = Some("Unlisted".to_owned());
        cx.notify();
    });
    let asked: Vec<actions::ToggleFavoriteOf> =
        dispatched_by(key, |view| view.list_focus.clone(), &view, cx);
    let Favorite::Artist(artist) = asked[0].favorite() else {
        panic!("not an artist");
    };
    assert_eq!(artist.name(), "Unlisted");
}

#[gpui_kit::test]
fn revealing_an_artist_the_filter_hides_clears_the_filter(cx: &mut TestAppContext) {
    let url = server();
    let (view, cx) = view_on(&url, cx);
    update(&view, cx, |view, cx| view.activate(cx));
    wait_until(&view, cx, |view| view.visible_len() == 2);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.filter
                .update(cx, |filter, cx| filter.set_value("Miles", window, cx));
            view.filter_changed(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(view.read(cx).visible_len(), 1));

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.reveal_artist(ArtistId::new("b"), window, cx)
        });
    });
    cx.run_until_parked();

    cx.update(|window, cx| {
        let view = view.read(cx);
        assert_eq!(view.filter.read(cx).value(), "");
        assert_eq!(view.visible_len(), 2);
        assert_eq!(view.selected_artist, Some(ArtistId::new("b")));
        assert!(view.list_focus.is_focused(window));
    });
}

fn revealed_before_the_list(id: &str, view: &Entity<Artists>, cx: &mut VisualTestContext) {
    update(view, cx, |view, _| {
        view.is_activated = true;
        view.is_refreshing = true;
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.reveal_artist(ArtistId::new(id), window, cx);
        });
    });
}

fn many_artists() -> Vec<Artist> {
    (0..500)
        .map(|ix| artist(&format!("a{ix}"), &format!("Artist {ix:03}")))
        .collect()
}

fn top_row(view: &Entity<Artists>, cx: &mut VisualTestContext) -> usize {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let view = view.read(cx);
        let scroll = view.list_scroll.0.borrow();
        let contents = scroll
            .last_item_size
            .map_or(px(0.), |size| size.contents.height);
        let row = contents / view.visible_len().max(1) as f32;
        if row > px(0.) {
            (-scroll.base_handle.offset().y / row) as usize
        } else {
            0
        }
    })
}

#[gpui_kit::test]
fn the_list_comes_to_an_artist_revealed_before_it_arrived(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    revealed_before_the_list("a400", &view, cx);
    assert_eq!(top_row(&view, cx), 0);

    update(&view, cx, |view, cx| set_artists(view, many_artists(), cx));
    assert!(top_row(&view, cx) > 300);
}

#[gpui_kit::test]
fn the_list_follows_a_revealed_artist_the_servers_answer_moves(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    revealed_before_the_list("a400", &view, cx);
    update(&view, cx, |view, cx| set_artists(view, many_artists(), cx));
    assert!(top_row(&view, cx) > 300);

    let moved = many_artists().into_iter().rev().collect();
    update(&view, cx, |view, cx| {
        set_artists(view, moved, cx);
        view.set_refreshing(false, cx);
    });
    // Now the 100th row, which `Nearest` brings to the bottom of the list
    let answered = top_row(&view, cx);
    assert!(
        (1..100).contains(&answered),
        "the list is at row {answered}"
    );
}

#[gpui_kit::test]
fn a_later_list_leaves_the_scroll_alone(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    revealed_before_the_list("a400", &view, cx);
    update(&view, cx, |view, cx| {
        set_artists(view, many_artists(), cx);
        view.set_refreshing(false, cx);
    });
    assert!(top_row(&view, cx) > 300);

    update(&view, cx, |view, _| {
        view.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
    });
    assert_eq!(top_row(&view, cx), 0);

    update(&view, cx, |view, cx| {
        set_artists(view, many_artists(), cx);
        view.set_refreshing(false, cx);
    });
    assert_eq!(top_row(&view, cx), 0);
}

#[gpui_kit::test]
fn choosing_another_artist_stops_waiting_to_reveal(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    revealed_before_the_list("a400", &view, cx);
    update(&view, cx, |view, cx| {
        view.select_artist(ArtistId::new("a1"), cx);
        set_artists(view, many_artists(), cx);
    });
    assert_eq!(top_row(&view, cx), 0);
}

/// Shows artist `m` with no albums, and `songs` as the saved songs naming them.
fn show_no_albums(view: &Entity<Artists>, songs: Vec<Song>, cx: &mut VisualTestContext) {
    update(view, cx, |view, cx| {
        show(view, 0, Vec::new(), cx);
        let detail = view.detail.as_mut().unwrap();
        detail.albums = Shelf::Loaded(Arc::from([]));
        detail.is_only_on_songs = !songs.is_empty();
        detail.top_songs = Shelf::Loaded(songs.into());
    });
}

#[gpui_kit::test]
fn an_artist_only_on_songs_shows_those_songs_in_place_of_albums(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_no_albums(&view, songs(7), cx);

    cx.update(|_, cx| {
        let parts = view.read(cx).parts();
        assert!(parts.contains(&Part::SongsHeading(ArtistSection::Songs)));
        assert!(parts.contains(&Part::TopSongsToggle));
        assert!(!parts.contains(&Part::AlbumsHeading));
        assert!(!parts.contains(&Part::AlbumsStatus));
    });
}

#[gpui_kit::test]
fn left_goes_back_to_the_list_from_the_start_of_a_row(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| show(view, 2, Vec::new(), cx));
    let focus_detail = |view: &Entity<Artists>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            let focus = view.read(cx).detail_focus.clone();
            focus.focus(window, cx);
        });
        cx.run_until_parked();
    };
    let list_is_focused = |view: &Entity<Artists>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| view.read(cx).list_focus.is_focused(window))
    };

    // From a song, which has nothing to its left
    select(&view, Item::Song(SongId::new("s0")), cx);
    focus_detail(&view, cx);
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert!(list_is_focused(&view, cx));

    // From the second card of a row, along the row first
    select(&view, Item::Album(AlbumId::new("a1")), cx);
    focus_detail(&view, cx);
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert!(!list_is_focused(&view, cx));
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).selection,
            Some(Item::Album(AlbumId::new("a2")))
        );
    });

    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert!(list_is_focused(&view, cx));
}

#[gpui_kit::test]
fn an_artist_with_no_songs_at_all_says_so_under_albums(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_no_albums(&view, Vec::new(), cx);

    cx.update(|_, cx| assert!(view.read(cx).parts().contains(&Part::AlbumsStatus)));
}

#[gpui_kit::test]
fn a_new_list_keeps_the_selection_unless_the_artist_has_gone(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 0, Vec::new(), cx);

        set_artists(view, vec![artist("m", "Miles Davis")], cx);
        assert_eq!(view.selected_artist, Some(ArtistId::new("m")));

        set_artists(view, vec![artist("z", "Zoot Sims")], cx);
        assert_eq!(view.selected_artist, Some(ArtistId::new("z")));
    });
}

#[gpui_kit::test]
fn a_new_list_keeps_an_artist_who_was_never_on_it(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 0, Vec::new(), cx);
        view.select_artist(ArtistId::new("only-on-songs"), cx);
        set_artists(view, vec![artist("m", "Miles Davis")], cx);
        assert_eq!(view.selected_artist, Some(ArtistId::new("only-on-songs")));
    });
}

#[gpui_kit::test]
fn play_next_queues_every_song_of_the_artist_album_by_album(cx: &mut TestAppContext) {
    let url = server();
    let (view, cx) = view_on(&url, cx);
    update(&view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    let seen = Rc::new(RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = seen.clone();
        cx.on_action(move |action: &EnqueueSongs, _| {
            seen.borrow_mut().push(action.songs().len());
        });
        let focus = view.read(cx).list_focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("alt-enter");

    for _ in 0..500 {
        cx.run_until_parked();
        if !seen.borrow().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Two albums of two songs each
    assert_eq!(*seen.borrow(), [4]);
}

fn albums(count: usize) -> Shelf<Album> {
    let albums: Vec<Album> = (0..count)
        .map(|ix| serde_json::from_value(album_json(&format!("a{ix}"), 1970)).unwrap())
        .collect();
    Shelf::Loaded(albums.into())
}

/// Shows artist `m` with `count` albums, laid out at the width they found.
fn show_albums(view: &Entity<Artists>, count: usize, cx: &mut VisualTestContext) {
    update(view, cx, |view, cx| {
        show(view, 10, Vec::new(), cx);
        view.detail.as_mut().unwrap().albums = albums(count);
        cx.notify();
    });
    lay_out(cx);
    lay_out(cx);
}

fn lay_out(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn last_part(view: &Artists) -> usize {
    view.listed.parts.len() - 1
}

#[gpui_kit::test]
fn a_long_artist_lays_out_only_what_is_near_the_viewport(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_albums(&view, 500, cx);
    cx.update(|_, cx| {
        let view = view.read(cx);
        assert!(view.detail_list.bounds_for_item(0).is_some());
        assert_eq!(view.detail_list.bounds_for_item(last_part(view)), None);
    });
}

#[gpui_kit::test]
fn every_part_is_as_wide_as_the_detail(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_albums(&view, 500, cx);
    let width = cx.update(|_, cx| view.read(cx).detail_width);
    assert!(width > px(0.));
    for part in 0..3 {
        let part = cx.debug_bounds(format!("artist-part-{part}").leak());
        assert_eq!(part.unwrap().size.width, width);
    }
}

#[gpui_kit::test]
fn a_row_of_cards_is_as_tall_as_reckoned_at_the_width_of_the_detail(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_albums(&view, 500, cx);
    let row = cx.update(|_, cx| {
        let view = view.read(cx);
        let parts = &view.listed.parts;
        let row = parts
            .iter()
            .position(|part| matches!(part, Part::AlbumRow(_)));
        let row = row.unwrap();
        view.detail_list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(0.),
        });
        row
    });
    lay_out(cx);
    cx.update(|window, cx| {
        let view = view.read(cx);
        let rem = window.rem_size();
        let height = view.detail_list.bounds_for_item(row).unwrap().size.height;
        let reckoned = rem * card_row_height(view.detail_width / rem, view.columns);
        assert!(
            (height - reckoned).abs() < rem * 1.5,
            "{height:?} tall, reckoned {reckoned:?}"
        );
    });
}

#[gpui_kit::test]
fn the_scrollbar_knows_the_length_of_a_long_artist_from_the_start(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_albums(&view, 500, cx);
    cx.update(|window, cx| {
        let view = view.read(cx);
        let rows = view
            .listed
            .parts
            .iter()
            .filter(|part| matches!(part, Part::AlbumRow(_)))
            .count();
        // A row holds a cover at its narrowest, ten rem, and more
        let shortest = window.rem_size() * 10.;
        let length = view.detail_list.max_offset_for_scrollbar().y;
        assert!(
            length > shortest * rows as f32,
            "{length:?} for {rows} rows"
        );
    });
}

#[gpui_kit::test]
fn end_and_home_reach_albums_not_yet_laid_out(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_albums(&view, 500, cx);

    update(&view, cx, |view, cx| {
        view.move_selection(GridMotion::Last, cx);
    });
    lay_out(cx);
    cx.update(|_, cx| {
        let view = view.read(cx);
        let last = view
            .detail_list
            .bounds_for_item(last_part(view))
            .expect("the last row is laid out");
        let viewport = view.detail_list.viewport_bounds();
        assert!(last.top() >= viewport.top() && last.bottom() <= viewport.bottom());
    });

    update(&view, cx, |view, cx| {
        view.move_selection(GridMotion::First, cx);
    });
    lay_out(cx);
    cx.update(|_, cx| {
        assert_eq!(view.read(cx).detail_list.logical_scroll_top().item_ix, 1);
    });
}

#[gpui_kit::test]
fn a_row_not_yet_laid_out_comes_into_view_however_wrong_its_reckoned_height(
    cx: &mut TestAppContext,
) {
    let (view, cx) = view(cx);
    show_albums(&view, 500, cx);
    cx.update(|_, cx| {
        let view = view.read(cx);
        let len = view.listed.parts.len();
        view.detail_list.reset_with_uniform_height(len, px(1.));
    });
    lay_out(cx);

    update(&view, cx, |view, cx| {
        view.move_selection(GridMotion::Last, cx);
    });
    lay_out(cx);

    cx.update(|_, cx| {
        let view = view.read(cx);
        assert!(view.detail_list.bounds_for_item(last_part(view)).is_some());
    });
}

#[gpui_kit::test]
fn see_more_leaves_what_is_on_show_in_place(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_albums(&view, 500, cx);
    let place = cx.update(|_, cx| {
        let view = view.read(cx);
        let parts = &view.listed.parts;
        let row = parts
            .iter()
            .rposition(|part| matches!(part, Part::AlbumRow(_)));
        let row = row.unwrap() - 20;
        view.detail_list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(5.),
        });
        parts[row].clone()
    });
    lay_out(cx);

    update(&view, cx, |view, cx| view.toggle_top_songs(cx));
    lay_out(cx);

    cx.update(|_, cx| {
        let view = view.read(cx);
        let top = view.detail_list.logical_scroll_top();
        assert_eq!(view.listed.parts[top.item_ix], place);
        assert_eq!(top.offset_in_item, px(5.));
    });
}

#[gpui_kit::test]
fn another_artist_starts_at_the_top(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    show_albums(&view, 500, cx);
    cx.update(|_, cx| {
        let view = view.read(cx);
        view.detail_list.scroll_to(ListOffset {
            item_ix: last_part(view),
            offset_in_item: px(0.),
        });
    });
    lay_out(cx);

    update(&view, cx, |view, cx| {
        view.select_artist(ArtistId::new("b"), cx);
        view.detail.as_mut().unwrap().albums = albums(500);
    });
    lay_out(cx);

    cx.update(|_, cx| {
        let top = view.read(cx).detail_list.logical_scroll_top();
        assert_eq!((top.item_ix, top.offset_in_item), (0, px(0.)));
    });
}

#[test]
fn the_part_at_the_top_stays_there_when_the_rest_changes() {
    let rows = |width: usize| {
        (0..9)
            .step_by(width)
            .map(move |start| Part::AlbumRow(start..(start + width).min(9)))
    };
    let parts = |width: usize| {
        let mut parts = vec![Part::Header, Part::AlbumsHeading];
        parts.extend(rows(width));
        parts
    };
    // The row holding the album that started the old one
    assert_eq!(same_place(&parts(3), 3, &parts(2)), 3);
    assert_eq!(same_place(&parts(2), 3, &parts(3)), 2);

    // A song that Show less took away gives way to the toggle after it
    let long = [Part::Song(4), Part::Song(5), Part::TopSongsToggle];
    let short = [Part::Header, Part::Song(4), Part::TopSongsToggle];
    assert_eq!(same_place(&long, 1, &short), 2);

    assert_eq!(same_place(&[], 0, &parts(3)), 0);
    assert_eq!(same_place(&parts(3), 2, &[]), 0);
}

#[gpui_kit::test]
fn play_shows_it_is_fetching_until_the_songs_arrive(cx: &mut TestAppContext) {
    let url = server();
    let (view, cx) = view_on(&url, cx);
    update(&view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.act_on_artist(
                ArtistId::new("m"),
                AlbumAction::Enqueue(Placement::Next),
                window,
                cx,
            );
            assert_eq!(view.artist_being_fetched, Some(ArtistId::new("m")));
        });
    });

    wait_until(&view, cx, |view| view.artist_being_fetched.is_none());
}

#[gpui_kit::test]
fn choosing_another_artist_says_which_was_left(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    let left = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let left = left.clone();
        cx.subscribe(&view, move |_, event: &ArtistsEvent, _| {
            let ArtistsEvent::LeftArtist(artist) = event;
            left.borrow_mut().push(artist.clone());
        })
        .detach();
    });

    // Nothing selected yet: nothing to leave
    update(&view, cx, |this, cx| {
        this.choose_artist(ArtistId::new("a"), cx);
    });
    assert!(left.borrow().is_empty());

    update(&view, cx, |this, cx| {
        this.choose_artist(ArtistId::new("a"), cx);
        this.choose_artist(ArtistId::new("b"), cx);
    });
    assert_eq!(*left.borrow(), vec![ArtistId::new("a")]);

    update(&view, cx, |this, cx| {
        this.select_artist(ArtistId::new("c"), cx);
    });
    assert_eq!(*left.borrow(), vec![ArtistId::new("a")]);
    cx.update(|_, cx| {
        assert_eq!(view.read(cx).selected_artist(), Some(&ArtistId::new("c")));
    });
}
