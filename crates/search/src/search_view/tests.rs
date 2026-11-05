use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{FocusHandle, InteractiveElement as _, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use subsonic::{Album, Credentials, ScanStatus};

use super::*;

struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct Saved {
    albums: Value,
    songs: Value,
}

impl catalog::Source for Saved {
    async fn albums_page(&self, _: u32, _: u32) -> Result<Vec<Album>, client::Error> {
        Ok(serde_json::from_value(self.albums.clone()).unwrap())
    }

    async fn artists(&self) -> Result<Vec<subsonic::Artist>, client::Error> {
        Ok(Vec::new())
    }

    async fn songs_page(&self, _: u32, _: u32) -> Result<Vec<Song>, client::Error> {
        Ok(serde_json::from_value(self.songs.clone()).unwrap())
    }

    async fn scan_status(&self) -> Result<ScanStatus, client::Error> {
        Ok(serde_json::from_value(json!({ "scanning": false })).unwrap())
    }

    async fn library_modified(&self) -> Result<Option<u64>, client::Error> {
        Ok(None)
    }
}

fn saved_library(dir: &tempfile::TempDir) -> catalog::Catalog {
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let source = Saved {
        albums: json!([
            { "id": "blue", "name": "Kind of Blue", "artist": "Miles Davis", "artistId": "miles" },
        ]),
        songs: json!([
            { "id": "so-what", "title": "So What", "artist": "Miles Davis", "artistId": "miles",
              "album": "Kind of Blue", "albumId": "blue" },
            { "id": "naima", "title": "Naima", "artist": "John Coltrane", "artistId": "john" },
        ]),
    };
    let mode = catalog::Mode::Always;
    futures::executor::block_on(catalog::sync_albums(&catalog, &source, mode)).unwrap();
    futures::executor::block_on(catalog::sync_songs(&catalog, &source, mode)).unwrap();
    catalog
}

fn palette(
    catalog: Option<catalog::Catalog>,
    cx: &mut TestAppContext,
) -> (
    Entity<Search>,
    Rc<RefCell<Vec<SearchEvent>>>,
    &mut VisualTestContext,
) {
    // Reading a library wakes the test from a background thread
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new("http://127.0.0.1:9", Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    let session = session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        catalog,
        None,
    );
    // Dialogs live in the window's root
    let (handle, _host) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Host)
            })
        })
        .unwrap();
    let cx = VisualTestContext::from_window(handle, cx).into_mut();
    let search = cx.update(|window, cx| cx.new(|cx| Search::new(session, window, cx)));
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&search, move |_, event: &SearchEvent, _| {
            events.borrow_mut().push(event.clone());
        })
        .detach();
    });
    (search, events, cx)
}

fn open(search: &Entity<Search>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| search.update(cx, |search, cx| search.open(window, cx)));
    settle(cx);
}

/// Lets what is under way finish, with what it reads arriving off the executor.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.run_until_parked();
}

fn ask(search: &Entity<Search>, query: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let state = search.read(cx).state.clone();
        state.update(cx, |state, cx| state.set_query(query, window, cx));
    });
    settle(cx);
}

fn is_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| window.has_active_dialog(cx))
}

fn sections(search: &Entity<Search>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    cx.update(|_, cx| {
        let results = search.read(cx).results.clone();
        results.sections.iter().map(|(label, _)| *label).collect()
    })
}

struct Shell {
    focus: FocusHandle,
    asked: usize,
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::OpenSearch, _, _| this.asked += 1))
    }
}

#[gpui_kit::test]
fn each_shortcut_asks_for_the_palette(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::init("Workspace", cx);
    });
    let (shell, cx) = cx.add_window_view(|_, cx| Shell {
        focus: cx.focus_handle(),
        asked: 0,
    });
    cx.update(|window, cx| shell.read(cx).focus.clone().focus(window, cx));
    cx.run_until_parked();
    let keys = cx.update(|_, cx| {
        if cx.platform_info().is_mac() {
            ["cmd-k", "cmd-shift-p"]
        } else {
            ["ctrl-k", "ctrl-shift-p"]
        }
    });

    for (ix, key) in keys.into_iter().enumerate() {
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        cx.update(|_, cx| assert_eq!(shell.read(cx).asked, ix + 1));
    }
}

#[gpui_kit::test]
fn it_opens_empty_and_finds_each_kind_as_typed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (search, _, cx) = palette(Some(saved_library(&dir)), cx);

    open(&search, cx);
    assert!(is_open(cx));
    assert!(sections(&search, cx).is_empty());

    ask(&search, "miles", cx);
    assert_eq!(sections(&search, cx), ["Artists", "Albums", "Songs"]);

    ask(&search, "naima", cx);
    assert_eq!(sections(&search, cx), ["Songs"]);

    ask(&search, "nothing like it", cx);
    assert!(sections(&search, cx).is_empty());
}

#[gpui_kit::test]
fn enter_asks_for_what_is_highlighted_and_closes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (search, events, cx) = palette(Some(saved_library(&dir)), cx);
    open(&search, cx);
    ask(&search, "naima", cx);

    cx.simulate_keystrokes("enter");
    settle(cx);

    assert!(!is_open(cx));
    let events = events.borrow();
    assert!(matches!(
        events.as_slice(),
        [SearchEvent::PlaySong(song)] if song.id().as_str() == "naima"
    ));
}

#[gpui_kit::test]
fn the_highlight_moves_to_the_next_kind(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (search, events, cx) = palette(Some(saved_library(&dir)), cx);
    open(&search, cx);
    ask(&search, "miles", cx);

    cx.simulate_keystrokes("down enter");
    settle(cx);

    let events = events.borrow();
    assert!(matches!(
        events.as_slice(),
        [SearchEvent::ShowAlbum(album)] if album.as_str() == "blue"
    ));
}

#[gpui_kit::test]
fn escape_closes_without_asking_for_anything(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (search, events, cx) = palette(Some(saved_library(&dir)), cx);
    open(&search, cx);

    cx.simulate_keystrokes("escape");
    settle(cx);

    assert!(!is_open(cx));
    assert!(events.borrow().is_empty());
}

fn resync(catalog: &catalog::Catalog, title: &str) {
    let source = Saved {
        albums: json!([]),
        songs: json!([{ "id": title, "title": title, "artist": "Someone", "artistId": "s" }]),
    };
    let mode = catalog::Mode::Always;
    futures::executor::block_on(catalog::sync_albums(catalog, &source, mode)).unwrap();
    futures::executor::block_on(catalog::sync_songs(catalog, &source, mode)).unwrap();
}

fn found_songs(search: &Entity<Search>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        let results = search.read(cx).results.clone();
        results
            .sections
            .iter()
            .flat_map(|(_, hits)| hits)
            .filter_map(|hit| match hit {
                Hit::Song(song) => Some(song.title().to_owned()),
                _ => None,
            })
            .collect()
    })
}

#[gpui_kit::test]
fn a_library_that_changed_while_open_shows_without_opening_it_again(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = saved_library(&dir);
    let (search, _, cx) = palette(Some(catalog.clone()), cx);
    open(&search, cx);
    ask(&search, "a", cx);
    assert!(found_songs(&search, cx).contains(&"Naima".to_owned()));

    resync(&catalog, "A Love Supreme");
    cx.update(|_, cx| search.update(cx, |search, cx| search.library_changed(cx)));
    settle(cx);

    assert_eq!(found_songs(&search, cx), ["A Love Supreme"]);
}

#[gpui_kit::test]
fn what_was_read_stays_searchable_during_a_new_reading(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (search, _, cx) = palette(Some(saved_library(&dir)), cx);
    open(&search, cx);

    cx.update(|_, cx| search.update(cx, |search, cx| search.library_changed(cx)));

    cx.update(|_, cx| {
        let search = search.read(cx);
        assert!(search.is_reading);
        assert!(matches!(search.library, SavedLibrary::Read(_)));
        assert!(!search.state.read(cx).is_loading());
    });
    settle(cx);
}

#[gpui_kit::test]
fn a_change_during_a_reading_is_read_too_and_the_spinner_stops(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = saved_library(&dir);
    let (search, _, cx) = palette(Some(catalog.clone()), cx);

    cx.update(|window, cx| search.update(cx, |search, cx| search.open(window, cx)));
    resync(&catalog, "A Love Supreme");
    cx.update(|_, cx| search.update(cx, |search, cx| search.library_changed(cx)));
    settle(cx);
    settle(cx);
    ask(&search, "a", cx);

    cx.update(|_, cx| {
        let search = search.read(cx);
        assert!(!search.is_reading);
        assert!(!search.state.read(cx).is_loading());
    });
    assert_eq!(found_songs(&search, cx), ["A Love Supreme"]);
}

#[gpui_kit::test]
fn a_library_that_changed_while_never_opened_waits_to_be(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (search, _, cx) = palette(Some(saved_library(&dir)), cx);

    cx.update(|_, cx| search.update(cx, |search, cx| search.library_changed(cx)));

    cx.update(|_, cx| {
        let search = search.read(cx);
        assert!(!search.is_reading);
        assert!(matches!(search.library, SavedLibrary::Unread));
    });
}

#[gpui_kit::test]
fn without_a_saved_library_it_says_so(cx: &mut TestAppContext) {
    let (search, _, cx) = palette(None, cx);
    open(&search, cx);
    cx.update(|_, cx| assert!(matches!(search.read(cx).library, SavedLibrary::Failed(_))));
}
