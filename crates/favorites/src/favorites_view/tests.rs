use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use actions::{ChoosePlaylist, EnqueueSongs, ShowAlbum, ShowArtist};
use gpui_kit::{
    Entity, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, TestAppContext, VisualTestContext, point,
};
use serde_json::{Value, json};
use subsonic::{AlbumId, ArtistId, Credentials};

use super::*;

#[derive(Default)]
struct ServerState {
    songs: Vec<String>,
    /// `star` and `unstar` as they came, with the parameter they named.
    writes: Vec<String>,
    reads: usize,
    held: HashSet<String>,
    refused: HashSet<String>,
    /// Refused the next time they come, then not again.
    refused_once: HashSet<String>,
}

type Shared = Arc<(Mutex<ServerState>, Condvar)>;

struct FakeServer {
    url: String,
    state: Shared,
}

impl FakeServer {
    fn new(songs: &[&str]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Shared::default();
        state.0.lock().unwrap().songs = songs.iter().map(|song| (*song).to_owned()).collect();
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                std::thread::spawn(move || serve(stream, &state));
            }
        });
        Self { url, state }
    }

    fn hold(&self, method: &str) {
        let mut state = self.state.0.lock().unwrap();
        state.held.insert(method.to_owned());
    }

    fn release(&self, method: &str) {
        self.state.0.lock().unwrap().held.remove(method);
        self.state.1.notify_all();
    }

    fn refuse(&self, method: &str) {
        let mut state = self.state.0.lock().unwrap();
        state.refused.insert(method.to_owned());
    }

    fn accept(&self, method: &str) {
        self.state.0.lock().unwrap().refused.remove(method);
    }

    fn refuse_once(&self, method: &str) {
        let mut state = self.state.0.lock().unwrap();
        state.refused_once.insert(method.to_owned());
    }

    fn writes(&self) -> Vec<String> {
        self.state.0.lock().unwrap().writes.clone()
    }

    fn reads(&self) -> usize {
        self.state.0.lock().unwrap().reads
    }
}

fn song_json(id: &str) -> Value {
    json!({ "id": id, "title": id, "artistId": format!("artist-{id}"), "albumId": format!("album-{id}") })
}

fn serve(mut stream: std::net::TcpStream, shared: &Shared) {
    let mut bytes = [0; 4096];
    let Ok(read) = stream.read(&mut bytes) else {
        return;
    };
    let head = String::from_utf8_lossy(&bytes[..read]).into_owned();
    let target = head.split(' ').nth(1).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let method = path.rsplit('/').next().unwrap_or_default().to_owned();
    let named = query
        .split('&')
        .find(|param| {
            ["id=", "albumId=", "artistId="]
                .iter()
                .any(|name| param.starts_with(name))
        })
        .unwrap_or_default()
        .to_owned();

    let (lock, released) = &**shared;
    let mut state = lock.lock().unwrap();
    let is_write = method == "star" || method == "unstar";
    if is_write {
        state.writes.push(format!("{method} {named}"));
    } else if method == "getStarred2" {
        state.reads += 1;
    }
    // A read has what the server has when it arrives, and a write takes effect
    // once let go.
    let songs_when_read = state.songs.clone();
    let mut state = released
        .wait_while(state, |state| state.held.contains(&method))
        .unwrap();
    let mut body = json!({ "status": "ok", "version": "1.16.1" });
    let refused_once = state.refused_once.remove(&method);
    if refused_once || state.refused.contains(&method) {
        body["status"] = json!("failed");
        body["error"] = json!({ "code": 0, "message": "Refused" });
    } else if is_write {
        let id = named.split_once('=').map_or("", |(_, id)| id).to_owned();
        if method == "star" {
            state.songs.insert(0, id);
        } else {
            state.songs.retain(|song| *song != id);
        }
    } else if method == "getAlbum" {
        body["album"] = json!({
            "id": "al",
            "name": "Album",
            "song": [song_json("t1"), song_json("t2")],
        });
    } else {
        let songs: Vec<Value> = songs_when_read.iter().map(|id| song_json(id)).collect();
        body["starred2"] = json!({
            "song": songs,
            "album": [{ "id": "al", "name": "Album", "artistId": "artist-al" }],
            "artist": [{ "id": "ar", "name": "Artist", "albumCount": 2 }],
        });
    }
    drop(state);
    let body = json!({ "subsonic-response": body }).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).ok();
}

fn view_on<'a>(
    url: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<Favorites>, &'a mut VisualTestContext) {
    // The requests wake the test from the network's own thread
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
        None,
        None,
    );
    // Notifications live in the window's root
    let (handle, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Favorites::new(session, window, cx))
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
    matches!(view.state, LoadState::Loaded) && !view.is_reloading && !view.is_writing()
}

fn song_ids(view: &Entity<Favorites>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        view.read(cx)
            .lists
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
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.set_favorite(favorite, is_favorite, window, cx);
        });
    });
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
        let height = state.last_item_size.unwrap().contents.height / this.lists.songs.len() as f32;
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
fn every_screen_learns_the_favorites_when_they_load(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server.url, cx);
    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("a"), cx), None));

    wait_until(&view, cx, is_settled);

    cx.update(|_, cx| {
        assert_eq!(actions::is_favorite(&target("a"), cx), Some(true));
        assert_eq!(actions::is_favorite(&target("z"), cx), Some(false));
        let album = StarTarget::Album(AlbumId::new("al"));
        assert_eq!(actions::is_favorite(&album, cx), Some(true));
        let artist = StarTarget::Artist(ArtistId::new("ar"));
        assert_eq!(actions::is_favorite(&artist, cx), Some(true));
    });
    assert_eq!(selected_song(&view, cx), Some(target("a")));
}

#[gpui_kit::test]
fn a_new_favorite_shows_at_once_and_reaches_the_server(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.hold("star");

    set_favorite(&view, song("n"), true, cx);

    assert_eq!(song_ids(&view, cx), ["n", "a"]);
    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("n"), cx), Some(true)));

    server.release("star");
    wait_until(&view, cx, is_settled);
    assert_eq!(server.writes(), ["star id=n"]);
}

#[gpui_kit::test]
fn each_kind_of_favorite_is_sent_by_its_own_parameter(cx: &mut TestAppContext) {
    let server = FakeServer::new(&[]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    let (album, artist) = cx.update(|_, cx| {
        let lists = &view.read(cx).lists;
        (lists.albums[0].clone(), lists.artists[0].clone())
    });

    set_favorite(&view, Favorite::Album(album), false, cx);
    set_favorite(&view, Favorite::Artist(artist), false, cx);
    wait_until(&view, cx, is_settled);

    assert_eq!(server.writes(), ["unstar albumId=al", "unstar artistId=ar"]);
}

#[gpui_kit::test]
fn asking_for_what_already_is_sends_nothing(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);

    set_favorite(&view, song("a"), true, cx);
    set_favorite(&view, song("z"), false, cx);
    wait_until(&view, cx, is_settled);

    assert!(server.writes().is_empty());
}

#[gpui_kit::test]
fn removing_the_selected_favorite_selects_the_one_that_follows(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a", "b", "c"]);
    let (view, cx) = view_on(&server.url, cx);
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
fn a_removal_the_server_refuses_goes_back_where_it_was(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a", "b", "c"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.refuse("unstar");

    set_favorite(&view, song("b"), false, cx);
    assert_eq!(song_ids(&view, cx), ["a", "c"]);
    wait_until(&view, cx, is_settled);

    assert_eq!(song_ids(&view, cx), ["a", "b", "c"]);
    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("b"), cx), Some(true)));
}

#[gpui_kit::test]
fn an_addition_the_server_refuses_goes_away(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.refuse("star");

    set_favorite(&view, song("n"), true, cx);
    wait_until(&view, cx, is_settled);

    assert_eq!(song_ids(&view, cx), ["a"]);
    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("n"), cx), Some(false)));
}

#[gpui_kit::test]
fn writes_go_one_at_a_time_in_the_order_they_were_made(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.hold("star");

    set_favorite(&view, song("n"), true, cx);
    set_favorite(&view, song("n"), false, cx);
    set_favorite(&view, song("n"), true, cx);
    wait_until(&view, cx, |_| server.writes().len() == 1);
    assert_eq!(server.writes(), ["star id=n"]);

    server.release("star");
    wait_until(&view, cx, is_settled);

    assert_eq!(server.writes(), ["star id=n", "unstar id=n", "star id=n"]);
    assert_eq!(song_ids(&view, cx), ["n", "a"]);
}

#[gpui_kit::test]
fn a_refused_change_leaves_a_later_one_to_decide(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.refuse_once("star");
    server.hold("star");

    set_favorite(&view, song("n"), true, cx);
    set_favorite(&view, song("n"), false, cx);
    set_favorite(&view, song("n"), true, cx);
    server.release("star");
    wait_until(&view, cx, is_settled);

    // Only the first was refused, and undoing it would hide what the last did
    assert_eq!(server.writes(), ["star id=n", "unstar id=n", "star id=n"]);
    assert_eq!(song_ids(&view, cx), ["n", "a"]);
    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("n"), cx), Some(true)));
}

// These read nothing back after the writes, to leave the undoing on its own
#[gpui_kit::test]
fn an_addition_then_a_removal_both_refused_leave_it_out(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.refuse("star");
    server.refuse("unstar");
    server.refuse("getStarred2");
    server.hold("star");

    set_favorite(&view, song("n"), true, cx);
    set_favorite(&view, song("n"), false, cx);
    server.release("star");
    wait_until(&view, cx, is_settled);

    assert_eq!(server.writes(), ["star id=n", "unstar id=n"]);
    assert_eq!(song_ids(&view, cx), ["a"]);
    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("n"), cx), Some(false)));
}

#[gpui_kit::test]
fn a_removal_then_an_addition_both_refused_leave_it_where_it_was(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a", "b", "c"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.refuse("star");
    server.refuse("unstar");
    server.refuse("getStarred2");
    server.hold("unstar");

    set_favorite(&view, song("b"), false, cx);
    set_favorite(&view, song("b"), true, cx);
    assert_eq!(song_ids(&view, cx), ["b", "a", "c"]);
    server.release("unstar");
    wait_until(&view, cx, is_settled);

    assert_eq!(song_ids(&view, cx), ["a", "b", "c"]);
    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("b"), cx), Some(true)));
}

#[gpui_kit::test]
fn a_refused_removal_brings_back_the_favorite_listed_not_the_one_asked_about(
    cx: &mut TestAppContext,
) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.refuse("unstar");
    server.refuse("getStarred2");
    let bare = Favorite::Song(serde_json::from_value(json!({ "id": "a", "title": "" })).unwrap());

    set_favorite(&view, bare, false, cx);
    wait_until(&view, cx, is_settled);

    cx.update(|_, cx| assert_eq!(view.read(cx).lists.songs[0].title(), "a"));
}

#[gpui_kit::test]
fn a_refused_change_is_followed_by_a_read(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.refuse_once("star");

    set_favorite(&view, song("n"), true, cx);
    wait_until(&view, cx, is_settled);

    assert_eq!(server.reads(), 2);
}

#[gpui_kit::test]
fn a_read_from_before_a_change_does_not_undo_it(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.hold("getStarred2");
    cx.update(|_, cx| view.update(cx, |view, cx| view.refresh(cx)));
    wait_until(&view, cx, |_| server.reads() == 2);

    set_favorite(&view, song("n"), true, cx);
    wait_until(&view, cx, |view| !view.is_writing());
    server.release("getStarred2");
    wait_until(&view, cx, is_settled);

    assert_eq!(song_ids(&view, cx), ["n", "a"]);
    assert_eq!(server.reads(), 3);
}

#[gpui_kit::test]
fn a_refresh_during_a_write_waits_for_it(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.hold("star");

    set_favorite(&view, song("n"), true, cx);
    cx.update(|_, cx| view.update(cx, |view, cx| view.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(server.reads(), 1);

    server.release("star");
    wait_until(&view, cx, |_| server.reads() == 2);
    wait_until(&view, cx, is_settled);

    assert_eq!(song_ids(&view, cx), ["n", "a"]);
}

#[gpui_kit::test]
fn a_failed_first_load_says_so_and_leaves_the_favorites_unknown(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    server.refuse("getStarred2");
    let (view, cx) = view_on(&server.url, cx);

    wait_until(&view, cx, |view| matches!(view.state, LoadState::Failed(_)));

    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("a"), cx), None));
    set_favorite(&view, song("n"), true, cx);
    cx.run_until_parked();
    assert!(server.writes().is_empty());
}

#[gpui_kit::test]
fn the_app_shell_hears_only_of_loads_that_leave_no_favorites(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    server.refuse("getStarred2");
    let (view, cx) = view_on(&server.url, cx);
    let failures = Rc::new(RefCell::new(0));
    cx.update(|_, cx| {
        let failures = failures.clone();
        cx.subscribe(&view, move |_, _: &FavoritesEvent, _| {
            *failures.borrow_mut() += 1;
        })
        .detach();
    });
    wait_until(&view, cx, |_| *failures.borrow() == 1);

    cx.update(|_, cx| view.update(cx, |view, cx| view.reload_if_failed(cx)));
    wait_until(&view, cx, |_| *failures.borrow() == 2);

    server.accept("getStarred2");
    cx.update(|_, cx| view.update(cx, |view, cx| view.reload_if_failed(cx)));
    wait_until(&view, cx, is_settled);
    server.refuse("getStarred2");
    cx.update(|_, cx| view.update(cx, |view, cx| view.refresh(cx)));
    wait_until(&view, cx, is_settled);

    assert_eq!(*failures.borrow(), 2);
}

#[gpui_kit::test]
fn favorites_that_loaded_are_not_read_again_for_having_failed(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);

    cx.update(|_, cx| view.update(cx, |view, cx| view.reload_if_failed(cx)));
    wait_until(&view, cx, is_settled);

    assert_eq!(server.reads(), 1);
}

#[gpui_kit::test]
fn retry_loads_the_favorites_that_failed(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    server.refuse("getStarred2");
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, |view| matches!(view.state, LoadState::Failed(_)));
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
fn starting_over_forgets_the_favorites_of_whoever_came_before(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    server.hold("getStarred2");
    cx.update(|cx| {
        let before = HashSet::from([target("from-before")]);
        actions::set_known_favorites(Some(before), cx);
    });

    let (view, cx) = view_on(&server.url, cx);

    cx.update(|_, cx| assert_eq!(actions::is_favorite(&target("from-before"), cx), None));
    server.release("getStarred2");
    wait_until(&view, cx, is_settled);
    cx.update(|_, cx| {
        assert_eq!(
            actions::is_favorite(&target("from-before"), cx),
            Some(false)
        );
    });
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_favorites_on_show(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    server.refuse("getStarred2");

    cx.update(|_, cx| view.update(cx, |view, cx| view.refresh(cx)));
    wait_until(&view, cx, is_settled);

    assert_eq!(song_ids(&view, cx), ["a"]);
    cx.update(|_, cx| assert!(view.read(cx).refresh_error.is_some()));
}

#[gpui_kit::test]
fn the_key_asks_to_take_the_selected_favorite_out(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    let asked = Rc::new(RefCell::new(Vec::new()));
    // What the app shell does with what the key leads to
    cx.update(|_, cx| {
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
    let server = FakeServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    focus(&view, cx);
    cx.simulate_keystrokes("down");

    let played: PlaySongs = dispatched_by("enter", &view, cx);

    assert_eq!(ids(played.songs()), ["a", "b"]);
    assert_eq!(played.start_ix(), Some(1));
}

#[gpui_kit::test]
fn enter_shows_the_album_or_the_artist_selected(cx: &mut TestAppContext) {
    let server = FakeServer::new(&[]);
    let (view, cx) = view_on(&server.url, cx);
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
    let server = FakeServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server.url, cx);
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
    let server = FakeServer::new(&[]);
    let (view, cx) = view_on(&server.url, cx);
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
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
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
    let server = FakeServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server.url, cx);
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
    let server = FakeServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server.url, cx);
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
    let server = FakeServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server.url, cx);
    wait_until(&view, cx, is_settled);
    let row = song_row(&view, 1, cx);

    click(row, MouseButton::Right, 1, cx);

    assert_eq!(selected_song(&view, cx), Some(target("b")));
}

#[gpui_kit::test]
fn the_arrows_move_between_the_kinds_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let server = FakeServer::new(&["a"]);
    let (view, cx) = view_on(&server.url, cx);
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
    let server = FakeServer::new(&["a", "b"]);
    let (view, cx) = view_on(&server.url, cx);
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
