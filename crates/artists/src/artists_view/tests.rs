use std::cell::RefCell;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use actions::{EnqueueSongs, PlaySongs, ShowAlbum, ShowArtist};
use gpui_kit::{KeyUpEvent, Keystroke, TestAppContext, VisualTestContext, px};
use serde_json::{Value, json};
use subsonic::{AlbumId, Credentials};

use super::detail::{Part, same_place};
use super::*;
use ui::card_grid::card_row_height;

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

/// What the fake server answers `method` with, and whether it knows of it.
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

/// A server that answers as [`answer`] does but knows of no albums of anybody.
fn server_of_no_albums() -> String {
    server_answering(|method| match method {
        "getArtist" => Some(json!({ "artist": { "id": "x", "name": "Guest" } })),
        method => answer(method),
    })
}

/// A server that answers with `answer`, returning its URL.
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

/// What a sync keeps in a catalog.
struct Saved {
    albums: Vec<Album>,
    songs: Vec<Song>,
}

impl catalog::Source for Saved {
    async fn albums_page(&self, _: u32, _: u32) -> Result<Vec<Album>, client::Error> {
        Ok(self.albums.clone())
    }

    async fn songs_page(&self, _: u32, _: u32) -> Result<Vec<Song>, client::Error> {
        Ok(self.songs.clone())
    }

    async fn scan_status(&self) -> Result<subsonic::ScanStatus, client::Error> {
        Ok(serde_json::from_value(json!({ "scanning": false })).unwrap())
    }

    async fn library_modified(&self) -> Result<Option<u64>, client::Error> {
        Ok(None)
    }
}

fn keep(catalog: &catalog::Catalog, saved: Saved) {
    let mode = catalog::Mode::Always;
    futures::executor::block_on(catalog::sync_albums(catalog, &saved, mode)).unwrap();
    futures::executor::block_on(catalog::sync_songs(catalog, &saved, mode)).unwrap();
}

fn song_by(id: &str, artist: &str, album: &str, track: u32) -> Song {
    serde_json::from_value(json!({
        "id": id, "title": id, "artistId": artist, "album": album, "track": track,
    }))
    .unwrap()
}

/// Waits for a keystroke's songs to be fetched and queued.
fn wait_for_queued(
    seen: &Rc<RefCell<Vec<Vec<String>>>>,
    cx: &mut VisualTestContext,
) -> Vec<Vec<String>> {
    for _ in 0..500 {
        cx.run_until_parked();
        if !seen.borrow().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    seen.take()
}

fn view_with<'a>(
    url: &str,
    catalog: Option<catalog::Catalog>,
    cx: &'a mut TestAppContext,
) -> (Entity<Artists>, &'a mut VisualTestContext) {
    // The requests wake the test from the network's own thread.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        ui::list_navigation::bind_control_keys(cx);
        crate::init(cx);
    });
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new(url, Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    let session = session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        catalog,
        None,
    );
    cx.add_window_view(|window, cx| Artists::new(session, window, cx))
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

/// Shows artist `m`, with the requests that makes cancelled.
fn show(view: &mut Artists, top_songs: usize, similar: Vec<Artist>, cx: &mut Context<Artists>) {
    view.is_activated = true;
    view._list_task = Task::ready(());
    view.is_refreshing = false;
    view.artists = ArtistList::Loaded(Arc::new(vec![
        artist("b", "Bill Evans"),
        artist("m", "Miles Davis"),
    ]));
    view.apply_filter(cx);
    view.select_artist(ArtistId::new("m"), cx);
    view._albums_task = Task::ready(());
    view._top_songs_task = Task::ready(());
    view._similar_task = Task::ready(());
    let detail = view.detail.as_mut().unwrap();
    detail.albums = Shelf::Loaded(Arc::from([
        serde_json::from_value(album_json("a2", 1970)).unwrap(),
        serde_json::from_value(album_json("a1", 1959)).unwrap(),
    ]));
    detail.top_songs = Shelf::Loaded(songs(top_songs).into());
    detail.similar = Shelf::Loaded(similar.into());
    cx.notify();
}

/// The `A`s that pressing `keys` in `focus` results in.
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
    update(&view, cx, |view, cx| view.refresh(cx));
    cx.update(|_, cx| {
        let view = view.read(cx);
        assert!(!view.is_activated);
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
        assert_eq!(view.visible.len(), 2);
        assert_eq!(view.selected_artist, Some(ArtistId::new("b")));
        let detail = view.detail.as_ref().unwrap();
        // The server here answers with the same artist for any ID.
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
        let id = ArtistId::new("m");
        view.detail.as_mut().unwrap().top_songs = Shelf::Loading;
        view.finish_loading_top_songs(&id, Err("offline".into()), cx);
        let parts = view.parts();
        assert!(parts.contains(&Part::TopSongsStatus));
        assert!(parts.contains(&Part::AlbumRow(0..2)));
    });
}

#[gpui_kit::test]
fn an_answer_about_another_artist_is_dropped(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 3, Vec::new(), cx);
        view.finish_loading_top_songs(&ArtistId::new("b"), Ok(songs(9)), cx);
        let detail = view.detail.as_ref().unwrap();
        assert_eq!(detail.top_songs.items().len(), 3);
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

    // Past Play and the menu.
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
    assert_eq!(played[0].start_ix(), 2);
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

    // Into the detail, where the song comes first.
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
fn revealing_an_artist_the_filter_hides_clears_the_filter(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.filter
                .update(cx, |filter, cx| filter.set_value("Miles", window, cx));
            view.apply_filter(cx);
            assert_eq!(view.visible.len(), 1);
            view.reveal_artist(ArtistId::new("b"), window, cx);
            assert_eq!(view.filter.read(cx).value(), "");
            assert_eq!(view.visible.len(), 2);
            assert_eq!(view.selected_artist, Some(ArtistId::new("b")));
        });
        assert!(view.read(cx).list_focus.is_focused(window));
    });
}

/// The screen waiting for its first list, with `id` just revealed.
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

/// The first row on show in the list, once the screen has drawn.
fn top_row(view: &Entity<Artists>, cx: &mut VisualTestContext) -> usize {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let view = view.read(cx);
        let scroll = view.list_scroll.0.borrow();
        let contents = scroll
            .last_item_size
            .map_or(px(0.), |size| size.contents.height);
        let row = contents / view.visible.len().max(1) as f32;
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

    update(&view, cx, |view, cx| view.set_artists(many_artists(), cx));
    assert!(top_row(&view, cx) > 300);
}

#[gpui_kit::test]
fn the_list_follows_a_revealed_artist_the_servers_answer_moves(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    revealed_before_the_list("a400", &view, cx);
    update(&view, cx, |view, cx| view.set_artists(many_artists(), cx));
    assert!(top_row(&view, cx) > 300);

    let moved = many_artists().into_iter().rev().collect();
    update(&view, cx, |view, cx| {
        view.finish_loading_artists(Ok(moved), cx);
    });
    // Now the 100th row, which `Nearest` brings to the bottom of the list.
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
        view.finish_loading_artists(Ok(many_artists()), cx);
    });
    assert!(top_row(&view, cx) > 300);

    update(&view, cx, |view, _| {
        view.list_scroll.scroll_to_item(0, ScrollStrategy::Top);
    });
    assert_eq!(top_row(&view, cx), 0);

    update(&view, cx, |view, cx| {
        view.finish_loading_artists(Ok(many_artists()), cx);
    });
    assert_eq!(top_row(&view, cx), 0);
}

#[gpui_kit::test]
fn choosing_another_artist_stops_waiting_to_reveal(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    revealed_before_the_list("a400", &view, cx);
    update(&view, cx, |view, cx| {
        view.select_artist(ArtistId::new("a1"), cx);
        view.set_artists(many_artists(), cx);
    });
    assert_eq!(top_row(&view, cx), 0);
}

#[gpui_kit::test]
fn an_artist_off_the_list_shows_once_the_server_names_them(cx: &mut TestAppContext) {
    let url = server();
    let (view, cx) = view_on(&url, cx);
    update(&view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.reveal_artist(ArtistId::new("only-on-songs"), window, cx);
            assert_eq!(view.detail.as_ref().unwrap().name, None);
        });
    });

    wait_until(&view, cx, |view| {
        let top_songs = view.detail.as_ref().map(|detail| detail.top_songs.items());
        top_songs.is_some_and(|songs| !songs.is_empty())
    });

    cx.update(|_, cx| {
        let view = view.read(cx);
        let detail = view.detail.as_ref().unwrap();
        assert_eq!(detail.name.as_ref().unwrap(), "Miles Davis");
        assert_eq!(view.selected_artist, Some(ArtistId::new("only-on-songs")));
        assert_eq!(view.selected_position(), None);
    });
}

fn finish_albums(
    view: &Entity<Artists>,
    albums: Vec<Album>,
    songs: Vec<Song>,
    cx: &mut VisualTestContext,
) {
    update(view, cx, |view, cx| {
        let id = ArtistId::new("m");
        let found = Artist::new(id.clone(), "Miles Davis".to_owned(), None, None);
        view.finish_loading_albums(&id, Ok((found, albums)), songs, cx);
    });
}

#[gpui_kit::test]
fn an_artist_only_on_songs_shows_those_songs_in_place_of_albums(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    finish_albums(&view, Vec::new(), songs(7), cx);

    cx.update(|_, cx| {
        let parts = view.read(cx).parts();
        assert!(parts.contains(&Part::TopSongsHeading));
        assert!(parts.contains(&Part::TopSongsToggle));
        assert!(!parts.contains(&Part::AlbumsHeading));
        assert!(!parts.contains(&Part::AlbumsStatus));
    });

    // Their songs are all there is: asking for top songs leaves them.
    update(&view, cx, |view, cx| {
        view.load_top_songs(ArtistId::new("m"), cx);
        assert_eq!(view.detail.as_ref().unwrap().top_songs.items().len(), 7);
    });
}

/// Goes to artist `x`, whom the server knows of no albums by.
fn reveal_guest(view: &Entity<Artists>, cx: &mut VisualTestContext) {
    update(view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.reveal_artist(ArtistId::new("x"), window, cx);
        });
    });
}

fn guest_songs() -> Saved {
    Saved {
        albums: Vec::new(),
        songs: vec![
            song_by("b1", "x", "Blue", 1),
            song_by("other", "m", "Blue", 2),
            song_by("a1", "x", "Agharta", 1),
        ],
    }
}

fn song_ids(songs: &[Song]) -> Vec<String> {
    songs.iter().map(|song| song.id().to_string()).collect()
}

#[gpui_kit::test]
fn the_saved_songs_of_an_artist_with_no_albums_show_and_play(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    keep(&catalog, guest_songs());
    let url = server_of_no_albums();
    let (view, cx) = view_with(&url, Some(catalog), cx);

    reveal_guest(&view, cx);
    wait_until(&view, cx, |view| {
        view.detail.as_ref().unwrap().is_only_on_songs
    });

    cx.update(|_, cx| {
        let view = view.read(cx);
        let songs = view.detail.as_ref().unwrap().top_songs.items();
        assert_eq!(song_ids(songs), ["a1", "b1"]);
        assert!(!view.parts().contains(&Part::AlbumsHeading));
    });

    // Their albums being none, what plays comes from the saved songs too.
    let seen = Rc::new(RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = seen.clone();
        cx.on_action(move |action: &EnqueueSongs, _| {
            seen.borrow_mut().push(song_ids(action.songs()));
        });
        let focus = view.read(cx).list_focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("alt-enter");
    assert_eq!(wait_for_queued(&seen, cx), [["a1", "b1"]]);
}

#[gpui_kit::test]
fn songs_arriving_in_a_sync_fill_the_page_of_an_artist_with_no_albums(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let url = server_of_no_albums();
    let (view, cx) = view_with(&url, Some(catalog.clone()), cx);

    reveal_guest(&view, cx);
    wait_until(&view, cx, |view| {
        matches!(view.detail.as_ref().unwrap().albums, Shelf::Loaded(_))
    });
    cx.update(|_, cx| assert!(view.read(cx).parts().contains(&Part::AlbumsStatus)));

    keep(&catalog, guest_songs());
    update(&view, cx, |view, cx| view.songs_synced(cx));
    wait_until(&view, cx, |view| {
        view.detail.as_ref().unwrap().is_only_on_songs
    });

    cx.update(|_, cx| {
        let songs = view.read(cx).detail.as_ref().unwrap().top_songs.items();
        assert_eq!(song_ids(songs), ["a1", "b1"]);
    });
}

#[gpui_kit::test]
fn the_saved_albums_show_when_the_server_cannot_be_reached(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let album: Album = serde_json::from_value(album_json("a1", 1959)).unwrap();
    let saved = Saved {
        albums: vec![album],
        songs: Vec::new(),
    };
    keep(&catalog, saved);
    let (view, cx) = view_with("http://127.0.0.1:9", Some(catalog), cx);

    update(&view, cx, |view, cx| view.activate(cx));
    wait_until(&view, cx, |view| {
        view.refresh_error.is_some()
            && view
                .detail
                .as_ref()
                .is_some_and(|detail| !matches!(detail.albums, Shelf::Loading))
    });

    cx.update(|_, cx| {
        let view = view.read(cx);
        assert_eq!(view.visible.len(), 1);
        assert_eq!(view.selected_artist, Some(ArtistId::new("m")));
        let detail = view.detail.as_ref().unwrap();
        assert_eq!(detail.name.as_ref().unwrap(), "Miles Davis");
        assert_eq!(detail.albums.items().len(), 1);
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

    // From a song, which has nothing to its left.
    select(&view, Item::Song(SongId::new("s0")), cx);
    focus_detail(&view, cx);
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert!(list_is_focused(&view, cx));

    // From the second card of a row, along the row first.
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
    update(&view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    finish_albums(&view, Vec::new(), Vec::new(), cx);

    cx.update(|_, cx| assert!(view.read(cx).parts().contains(&Part::AlbumsStatus)));
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_songs_of_an_artist_only_on_songs(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    finish_albums(&view, Vec::new(), songs(3), cx);

    update(&view, cx, |view, cx| {
        let id = ArtistId::new("m");
        view.finish_loading_albums(&id, Err("offline".into()), Vec::new(), cx);
        let detail = view.detail.as_ref().unwrap();
        assert!(detail.is_only_on_songs);
        assert_eq!(detail.top_songs.items().len(), 3);
    });
}

#[gpui_kit::test]
fn albums_arriving_later_bring_back_the_albums_and_the_top_songs(cx: &mut TestAppContext) {
    let url = server();
    let (view, cx) = view_on(&url, cx);
    update(&view, cx, |view, cx| show(view, 0, Vec::new(), cx));
    finish_albums(&view, Vec::new(), songs(3), cx);

    let album: Album = serde_json::from_value(album_json("a1", 1959)).unwrap();
    finish_albums(&view, vec![album], Vec::new(), cx);

    wait_until(&view, cx, |view| {
        let detail = view.detail.as_ref().unwrap();
        detail.top_songs.items().len() == 10 && !detail.is_only_on_songs
    });
    cx.update(|_, cx| assert!(view.read(cx).parts().contains(&Part::AlbumsHeading)));
}

#[gpui_kit::test]
fn a_new_list_keeps_the_selection_unless_the_artist_has_gone(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 0, Vec::new(), cx);

        view.set_artists(vec![artist("m", "Miles Davis")], cx);
        assert_eq!(view.selected_artist, Some(ArtistId::new("m")));

        view.set_artists(vec![artist("z", "Zoot Sims")], cx);
        assert_eq!(view.selected_artist, Some(ArtistId::new("z")));
    });
}

#[gpui_kit::test]
fn a_new_list_keeps_an_artist_who_was_never_on_it(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 0, Vec::new(), cx);
        view.select_artist(ArtistId::new("only-on-songs"), cx);
        view.set_artists(vec![artist("m", "Miles Davis")], cx);
        assert_eq!(view.selected_artist, Some(ArtistId::new("only-on-songs")));
    });
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_list_on_show(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        show(view, 0, Vec::new(), cx);
        view.finish_loading_artists(Err("offline".into()), cx);
        assert_eq!(view.visible.len(), 2);
        assert_eq!(view.refresh_error.as_ref().unwrap(), "offline");
    });
}

#[gpui_kit::test]
fn failing_to_load_a_first_list_says_so(cx: &mut TestAppContext) {
    let (view, cx) = view(cx);
    update(&view, cx, |view, cx| {
        view.finish_loading_artists(Err("offline".into()), cx);
        assert!(matches!(view.artists, ArtistList::Failed(_)));
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
    // Two albums of two songs each.
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
        // A row holds a cover at its narrowest, ten rem, and more.
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
        view._albums_task = Task::ready(());
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
    // The row holding the album that started the old one.
    assert_eq!(same_place(&parts(3), 3, &parts(2)), 3);
    assert_eq!(same_place(&parts(2), 3, &parts(3)), 2);

    // A song that Show less took away gives way to the toggle after it.
    let long = [Part::Song(4), Part::Song(5), Part::TopSongsToggle];
    let short = [Part::Header, Part::Song(4), Part::TopSongsToggle];
    assert_eq!(same_place(&long, 1, &short), 2);

    assert_eq!(same_place(&[], 0, &parts(3)), 0);
    assert_eq!(same_place(&parts(3), 2, &[]), 0);
}

#[gpui_kit::test]
fn moving_through_artists_asks_the_server_only_about_where_it_stops(cx: &mut TestAppContext) {
    static ASKED: AtomicUsize = AtomicUsize::new(0);
    fn counting(method: &str) -> Option<Value> {
        if method == "getArtist" {
            ASKED.fetch_add(1, Ordering::SeqCst);
        }
        answer(method)
    }
    let url = server_answering(counting);
    let (view, cx) = view_on(&url, cx);
    update(&view, cx, |view, cx| {
        view.is_activated = true;
        view._list_task = Task::ready(());
        view.artists = ArtistList::Loaded(Arc::new(vec![
            artist("b", "Bill Evans"),
            artist("m", "Miles Davis"),
        ]));
        view.apply_filter(cx);
        view.select_artist(ArtistId::new("b"), cx);
    });
    cx.executor().advance_clock(SERVER_DELAY / 2);
    cx.run_until_parked();
    update(&view, cx, |view, cx| {
        view.select_artist(ArtistId::new("m"), cx);
    });

    wait_until(&view, cx, |view| {
        view.detail
            .as_ref()
            .is_some_and(|detail| matches!(detail.albums, Shelf::Loaded(_)))
    });
    assert_eq!(ASKED.load(Ordering::SeqCst), 1);
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

    // Nothing selected yet: nothing to leave.
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
