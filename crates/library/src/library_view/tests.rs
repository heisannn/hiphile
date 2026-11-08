use std::cell::RefCell;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::rc::Rc;
use std::time::Duration;

use actions::NewPlaylist;
use gpui_kit::{KeyBinding, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext, actions};
use serde_json::json;
use subsonic::Credentials;

use super::*;

fn album(id: &str, name: &str) -> Album {
    serde_json::from_value(json!({ "id": id, "name": name })).unwrap()
}

/// A library with two albums, whose server cannot be reached.
fn library(cx: &mut TestAppContext) -> (Entity<Library>, &mut VisualTestContext) {
    library_on("http://127.0.0.1:9", cx)
}

/// A library with two albums once the first sync, which finds no albums at `url`, is over.
fn library_on<'a>(
    url: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<Library>, &'a mut VisualTestContext) {
    // The failing requests wake the test from the network's own thread
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
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
    let sync = LibrarySync::new(cx.update(|cx| gpui_host::executor(cx)), session.clone());
    let (library, cx) = cx.add_window_view(|window, cx| Library::new(session, sync, window, cx));
    wait_for_the_sync(&library, cx);
    set_albums(&library, vec![album("a", "Alpha"), album("b", "Beta")], cx);
    (library, cx)
}

fn album_a_json() -> serde_json::Value {
    json!({
        "id": "a",
        "name": "Alpha",
        "song": [
            { "id": "a1", "title": "One" },
            { "id": "a2", "title": "Two" },
        ],
    })
}

/// Shows album `a` and its songs, with focus on the album list.
fn show_album_a(library: &Entity<Library>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        library.update(cx, |library, cx| {
            let album = serde_json::from_value(album_a_json()).unwrap();
            library.show_album(&AlbumId::new("a"), album, cx);
            library.albums_focus.focus(window, cx);
        });
    });
    cx.run_until_parked();
}

fn dispatched<A: gpui_kit::Action + Clone>(
    command: impl gpui_kit::Action,
    cx: &mut VisualTestContext,
) -> Vec<A> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let seen = seen.clone();
        cx.on_action(move |action: &A, _| seen.borrow_mut().push(action.clone()));
    });
    cx.dispatch_action(command);
    cx.run_until_parked();
    seen.take()
}

fn set_filter(library: &Entity<Library>, query: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        library.update(cx, |library, cx| {
            library
                .filter
                .update(cx, |filter, cx| filter.set_value(query, window, cx));
            library.apply_filter(cx);
        });
    });
}

fn reveal(library: &Entity<Library>, id: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        library.update(cx, |library, cx| {
            library.reveal_album(AlbumId::new(id), window, cx);
        });
    });
}

/// Presses `key` on what focus reaches next after `from`. A button acts on
/// the release.
fn press_after(
    from: impl Fn(&Library) -> FocusHandle,
    key: &str,
    library: &Entity<Library>,
    cx: &mut VisualTestContext,
) {
    cx.update(|window, cx| {
        let focus = from(library.read(cx));
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.focus_next(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes(key);
    let os = cx.update(|_, cx| cx.platform_info().os);
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse_with_os(key, os).unwrap(),
    });
}

fn key_presses_retry(key: &str, cx: &mut TestAppContext) -> bool {
    actions!(library_test, [TakenByTheWorkspace]);
    let (library, cx) = library(cx);
    init_keys(cx);
    cx.update(|_, cx| {
        cx.bind_keys([KeyBinding::new(
            "space",
            TakenByTheWorkspace,
            Some(ALBUMS_CONTEXT),
        )]);
        cx.on_action(|_: &TakenByTheWorkspace, _| {});
    });
    cx.update(|_, cx| {
        library.update(cx, |library, cx| {
            library.albums = Albums::Failed("before".into());
            library.visible = Arc::default();
            cx.notify();
        });
    });
    press_after(|library| library.albums_focus.clone(), key, &library, cx);

    // Pressed, it loads again, and whatever that comes to is not this
    cx.update(
        |_, cx| !matches!(&library.read(cx).albums, Albums::Failed(error) if error == "before"),
    )
}

fn init_keys(cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        ui::list_navigation::bind_control_keys(cx);
        crate::init(cx);
    });
}

/// Lets a sync of the library end, as one the server cannot answer fails.
fn wait_for_the_sync(library: &Entity<Library>, cx: &mut VisualTestContext) {
    for _ in 0..500 {
        cx.run_until_parked();
        let running = cx.update(|_, cx| library.read(cx).sync.status().is_running());
        if !running {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the sync never ended");
}

#[gpui_kit::test]
fn enter_presses_a_focused_retry(cx: &mut TestAppContext) {
    assert!(key_presses_retry("enter", cx));
}

#[gpui_kit::test]
fn space_presses_a_focused_retry(cx: &mut TestAppContext) {
    assert!(key_presses_retry("space", cx));
}

#[gpui_kit::test]
fn enter_on_the_play_button_plays_the_whole_album(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    init_keys(cx);
    show_album_a(&library, cx);
    cx.update(|_, cx| {
        library.update(cx, |library, cx| library.select_song(1, cx));
    });
    let played = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let played = played.clone();
        cx.on_action(move |action: &PlaySongs, _| played.borrow_mut().push(action.start_ix()));
    });

    press_after(|library| library.songs_focus.clone(), "enter", &library, cx);
    cx.run_until_parked();

    assert_eq!(*played.borrow(), [None]);
}

#[gpui_kit::test]
fn revealing_an_album_the_filter_hides_clears_the_filter(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    set_filter(&library, "Alpha", cx);

    reveal(&library, "b", cx);

    cx.update(|_, cx| {
        let library = library.read(cx);
        assert_eq!(library.filter.read(cx).value(), "");
        assert_eq!(library.visible.len(), 2);
        assert_eq!(library.selected_album, Some(AlbumId::new("b")));
    });
}

#[gpui_kit::test]
fn revealing_an_album_the_filter_shows_keeps_the_filter(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    set_filter(&library, "Beta", cx);

    reveal(&library, "b", cx);

    cx.update(|_, cx| {
        let library = library.read(cx);
        assert_eq!(library.filter.read(cx).value(), "Beta");
        assert_eq!(library.visible.len(), 1);
        assert_eq!(library.selected_album, Some(AlbumId::new("b")));
    });
}

#[gpui_kit::test]
fn revealing_an_album_moves_focus_to_the_album_list(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    cx.update(|window, cx| {
        let songs_focus = library.read(cx).songs_focus.clone();
        songs_focus.focus(window, cx);
    });

    reveal(&library, "a", cx);

    cx.update(|window, cx| {
        assert!(library.read(cx).albums_focus.is_focused(window));
    });
}

fn set_albums(library: &Entity<Library>, albums: Vec<Album>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        library.update(cx, |library, cx| library.set_albums(albums.into(), cx));
    });
}

#[gpui_kit::test]
fn an_album_revealed_before_the_list_has_it_stays_selected_until_it_arrives(
    cx: &mut TestAppContext,
) {
    let (library, cx) = library(cx);
    set_filter(&library, "Alpha", cx);

    reveal(&library, "c", cx);
    set_albums(&library, vec![album("a", "Alpha"), album("b", "Beta")], cx);

    cx.update(|_, cx| {
        let library = library.read(cx);
        assert_eq!(library.filter.read(cx).value(), "Alpha");
        assert_eq!(library.selected_album, Some(AlbumId::new("c")));
        assert_eq!(library.revealing, Some(AlbumId::new("c")));
    });

    set_albums(
        &library,
        vec![album("a", "Alpha"), album("c", "Alpha Two")],
        cx,
    );

    cx.update(|_, cx| {
        let library = library.read(cx);
        assert_eq!(library.selected_album, Some(AlbumId::new("c")));
        assert_eq!(library.revealing, None);
    });
}

#[gpui_kit::test]
fn selecting_another_album_gives_up_on_one_being_revealed(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    reveal(&library, "c", cx);

    cx.update(|_, cx| {
        library.update(cx, |library, cx| {
            library.select_album(AlbumId::new("a"), cx);
        });
    });

    cx.update(|_, cx| assert_eq!(library.read(cx).revealing, None));
}

#[gpui_kit::test]
fn the_end_of_a_sync_gives_up_on_an_album_being_revealed(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    reveal(&library, "c", cx);

    cx.update(|_, cx| {
        library.read(cx).refresh_library();
    });
    wait_for_the_sync(&library, cx);
    cx.update(|_, cx| assert_eq!(library.read(cx).revealing, None));

    set_albums(&library, vec![album("a", "Alpha"), album("b", "Beta")], cx);
    cx.update(|_, cx| {
        assert_eq!(library.read(cx).selected_album, Some(AlbumId::new("a")));
    });
}

fn song_ids(songs: &[Song]) -> Vec<&str> {
    songs.iter().map(|song| song.id().as_str()).collect()
}

#[gpui_kit::test]
fn play_on_an_album_plays_the_whole_of_it(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    show_album_a(&library, cx);

    let plays = dispatched::<PlaySongs>(Play, cx);

    assert_eq!(plays.len(), 1);
    assert_eq!(song_ids(plays[0].songs()), ["a1", "a2"]);
    assert_eq!(plays[0].start_ix(), None);
}

#[gpui_kit::test]
fn an_album_goes_to_the_queue_where_asked(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    show_album_a(&library, cx);

    let next = dispatched::<EnqueueSongs>(PlayNext, cx);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].placement(), Placement::Next);
    assert_eq!(song_ids(next[0].songs()), ["a1", "a2"]);

    let last = dispatched::<EnqueueSongs>(AddToQueue, cx);
    assert_eq!(last.last().unwrap().placement(), Placement::Last);
}

#[gpui_kit::test]
fn an_album_goes_to_the_playlist_chosen_in_its_menu(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    show_album_a(&library, cx);
    let playlist = PlaylistId::new("p");

    let added = dispatched::<actions::AddToPlaylist>(
        AddAlbumToPlaylist {
            playlist: Some(playlist.clone()),
        },
        cx,
    );
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].playlist(), &playlist);
    assert_eq!(song_ids(added[0].songs()), ["a1", "a2"]);

    let created = dispatched::<NewPlaylist>(AddAlbumToPlaylist { playlist: None }, cx);
    assert_eq!(created.len(), 1);
    assert_eq!(song_ids(created[0].songs()), ["a1", "a2"]);
}

#[gpui_kit::test]
fn the_favorite_key_goes_to_the_album_or_the_song_with_focus(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    init_keys(cx);
    show_album_a(&library, cx);
    let asked = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        actions::set_known_favorites(Some(Default::default()), cx);
        library.update(cx, |library, cx| {
            library.selected_album = Some(AlbumId::new("a"));
            library.select_song(1, cx);
        });
        let asked = asked.clone();
        cx.on_action(move |action: &actions::SetFavorite, _| {
            asked.borrow_mut().push(action.favorite().target());
        });
    });
    let key = if cfg!(target_os = "macos") {
        "cmd-d"
    } else {
        "ctrl-d"
    };

    for (focus, target) in [
        (
            library.read_with(cx, |library, _| library.albums_focus.clone()),
            subsonic::StarTarget::Album(AlbumId::new("a")),
        ),
        (
            library.read_with(cx, |library, _| library.songs_focus.clone()),
            subsonic::StarTarget::Song(SongId::new("a2")),
        ),
    ] {
        cx.update(|window, cx| focus.focus(window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        assert_eq!(asked.borrow().last(), Some(&target));
    }
}

fn server_of_album_a() -> String {
    server_answering(json!({ "album": album_a_json() }))
}

fn server_answering(fields: serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while request.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let mut body = json!({ "status": "ok", "version": "1.16.1" });
            body.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
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

#[gpui_kit::test]
fn an_action_on_an_album_still_loading_waits_for_its_songs(cx: &mut TestAppContext) {
    let url = server_of_album_a();
    let (library, cx) = library_on(&url, cx);
    let plays = Rc::new(RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = plays.clone();
        cx.on_action(move |action: &PlaySongs, _| seen.borrow_mut().push(action.clone()));
        library.update(cx, |library, cx| {
            library._album_task = Task::ready(());
            library.detail = Detail::Loading;
            library.albums_focus.focus(window, cx);
        });
    });

    cx.dispatch_action(Play);
    // The answer arrives from the network's own thread
    for _ in 0..500 {
        cx.run_until_parked();
        if !plays.borrow().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let plays = plays.borrow();
    assert_eq!(plays.len(), 1);
    assert_eq!(song_ids(plays[0].songs()), ["a1", "a2"]);
}

#[gpui_kit::test]
fn an_album_without_songs_is_not_played(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    cx.update(|window, cx| {
        library.update(cx, |library, cx| {
            let empty = serde_json::from_value(json!({ "id": "a", "name": "a" })).unwrap();
            library.show_album(&AlbumId::new("a"), empty, cx);
            library.albums_focus.focus(window, cx);
        });
    });

    assert!(dispatched::<PlaySongs>(Play, cx).is_empty());
}

#[gpui_kit::test]
fn the_page_keys_move_by_the_albums_in_view(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    init_keys(cx);
    let albums: Vec<Album> = (0..100)
        .map(|ix| album(&ix.to_string(), &ix.to_string()))
        .collect();
    let len = albums.len();
    set_albums(&library, albums, cx);
    cx.update(|window, cx| {
        let focus = library.read(cx).albums_focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    let page = cx.update(|_, cx| {
        let state = library.read(cx).albums_scroll.0.borrow();
        let row = state.last_item_size.unwrap().contents.height / len as f32;
        (state.base_handle.bounds().size.height / row).floor() as usize
    });
    assert!(page > 1, "the window shows {page} rows");

    cx.simulate_keystrokes("home pagedown");
    cx.update(|_, cx| assert_eq!(library.read(cx).selected_position(), Some(page)));
    cx.simulate_keystrokes("pageup");
    cx.update(|_, cx| assert_eq!(library.read(cx).selected_position(), Some(0)));
}

fn left_albums(library: &Entity<Library>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<AlbumId>>> {
    let left = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let left = left.clone();
        cx.subscribe(library, move |_, event, _| {
            let LibraryEvent::LeftAlbum(album) = event;
            left.borrow_mut().push(album.clone());
        })
        .detach();
    });
    left
}

#[gpui_kit::test]
fn choosing_another_album_says_which_was_left(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    let left = left_albums(&library, cx);
    let selected = cx.update(|_, cx| library.read(cx).selected_album().cloned().unwrap());
    let other = AlbumId::new(if selected == AlbumId::new("a") {
        "b"
    } else {
        "a"
    });

    cx.update(|_, cx| {
        library.update(cx, |library, cx| library.choose_album(other.clone(), cx));
    });
    cx.run_until_parked();

    assert_eq!(*left.borrow(), vec![selected]);
    cx.update(|_, cx| assert_eq!(library.read(cx).selected_album(), Some(&other)));
}

#[gpui_kit::test]
fn choosing_the_selected_album_leaves_nothing(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    let left = left_albums(&library, cx);
    let selected = cx.update(|_, cx| library.read(cx).selected_album().cloned().unwrap());

    cx.update(|_, cx| {
        library.update(cx, |library, cx| library.choose_album(selected, cx));
    });
    cx.run_until_parked();

    assert!(left.borrow().is_empty());
}

#[gpui_kit::test]
fn revealing_and_moving_to_an_album_leave_nothing(cx: &mut TestAppContext) {
    let (library, cx) = library(cx);
    let left = left_albums(&library, cx);

    reveal(&library, "b", cx);
    cx.update(|_, cx| {
        library.update(cx, |library, cx| library.move_album(Motion::First, cx));
    });
    cx.run_until_parked();

    assert!(left.borrow().is_empty());
}
