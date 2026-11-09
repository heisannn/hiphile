use gpui_kit::TestAppContext;
use serde_json::json;

use super::*;
use crate::home::today;
use crate::test_support::{session, session_saving_into};
use crate::testing::{NOWHERE, server_answering, wait_for};

fn song(id: &str) -> Song {
    serde_json::from_value(json!({ "id": id, "title": id, "played": "2026-11-03T22:36:15Z" }))
        .unwrap()
}

fn album(id: &str) -> Album {
    serde_json::from_value(json!({ "id": id, "name": id })).unwrap()
}

fn albums(count: usize) -> Vec<Album> {
    (0..count).map(|ix| album(&format!("a{ix}"))).collect()
}

/// On GPUI's executor, which the test's `run_until_parked` drives, for a server at `url`.
fn shelves_on(url: &str, cx: &mut TestAppContext) -> HomeShelves {
    shelves_with(session(url), cx).0
}

fn shelves_with(session: Session, cx: &mut TestAppContext) -> (HomeShelves, DailyRecommendations) {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let recommendations = DailyRecommendations::new(executor.clone(), session.clone());
    let shelves = HomeShelves::new(executor, session, recommendations.clone());
    (shelves, recommendations)
}

fn update<R>(shelves: &HomeShelves, change: impl FnOnce(&mut State) -> R) -> R {
    shelves.0.state.update(change)
}

/// Shelves on show, cancelling the loads made on opening.
fn fill(state: &mut State, songs: &[&str], album_count: usize) {
    state.recently_played = Load::none();
    state.most_played = Load::none();
    state.new_albums = Load::none();
    state.status.recently_played = Shelf::Loaded(songs.iter().map(|id| song(id)).collect());
    state.status.most_played = Shelf::Loaded(albums(album_count).into());
    state.status.new_albums = Shelf::Loaded(albums(album_count).into());
}

fn is_loading(status: &ShelvesStatus) -> bool {
    matches!(status.recently_played, Shelf::Loading)
        || matches!(status.most_played, Shelf::Loading)
        || matches!(status.new_albums, Shelf::Loading)
}

#[gpui_kit::test]
fn the_shelves_load_from_the_server(cx: &mut TestAppContext) {
    let url = server_answering(json!({
        "albumList2": { "album": [{ "id": "a1", "name": "a1" }] },
        "album": {
            "id": "a1",
            "name": "a1",
            "song": [
                { "id": "old", "title": "old", "played": "2026-07-01T10:00:00Z" },
                { "id": "never", "title": "never" },
                { "id": "new", "title": "new", "played": "2026-11-03T22:36:15Z" },
            ],
        },
    }));
    let shelves = shelves_on(&url, cx);
    assert!(is_loading(&shelves.status()));

    wait_for(cx, || !is_loading(&shelves.status()));

    let status = shelves.status();
    let songs: Vec<_> = status
        .recently_played()
        .items()
        .iter()
        .map(|song| song.id().as_str())
        .collect();
    assert_eq!(songs, ["new", "old"]);
    assert_eq!(status.most_played().items(), [album("a1")]);
    assert_eq!(status.new_albums().items(), [album("a1")]);
}

#[gpui_kit::test]
fn the_album_shelves_keep_their_albums_off_the_recommendations(cx: &mut TestAppContext) {
    let url = server_answering(json!({
        "albumList2": { "album": [{ "id": "r1", "name": "r1" }] },
    }));
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    catalog
        .put_recommended("alice", &today(), &[album("r0"), album("r1")])
        .unwrap();
    let (shelves, recommendations) = shelves_with(session_saving_into(Some(catalog), &url), cx);

    wait_for(cx, || {
        !is_loading(&shelves.status()) && !recommendations.status().is_recommending()
    });

    assert_eq!(recommendations.status().albums().items(), [album("r0")]);
}

#[gpui_kit::test]
fn a_shelf_that_never_showed_says_why_it_failed(cx: &mut TestAppContext) {
    let shelves = shelves_on(NOWHERE, cx);

    wait_for(cx, || !is_loading(&shelves.status()));

    let status = shelves.status();
    let failed = |error: Option<&str>| error.is_some_and(|error| !error.is_empty());
    assert!(failed(failure(status.recently_played())));
    assert!(failed(failure(status.most_played())));
    assert!(failed(failure(status.new_albums())));
}

fn failure<T>(shelf: &Shelf<T>) -> Option<&str> {
    match shelf {
        Shelf::Failed(error) => Some(error),
        Shelf::Loading | Shelf::Loaded(_) => None,
    }
}

#[gpui_kit::test]
fn a_play_reloads_the_shelves_it_changes_keeping_what_is_on_show(cx: &mut TestAppContext) {
    let shelves = shelves_on(NOWHERE, cx);
    update(&shelves, |state| fill(state, &["s1"], 1));

    shelves.song_played();

    update(&shelves, |state| {
        assert_eq!(state.status.recently_played.items().len(), 1);
        assert_eq!(state.status.most_played.items().len(), 1);
        assert_eq!(state.recently_played.id, 1);
        assert_eq!(state.most_played.id, 1);
        assert_eq!(state.new_albums.id, 0);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_library_sync_reloads_the_album_shelves_keeping_what_is_on_show(cx: &mut TestAppContext) {
    let shelves = shelves_on(NOWHERE, cx);
    update(&shelves, |state| fill(state, &["s1"], 1));

    shelves.library_synced();

    update(&shelves, |state| {
        assert_eq!(state.status.most_played.items().len(), 1);
        assert_eq!(state.status.new_albums.items().len(), 1);
        assert_eq!(state.recently_played.id, 0);
        assert_eq!(state.most_played.id, 1);
        assert_eq!(state.new_albums.id, 1);
    });
    cx.run_until_parked();
}

#[test]
fn an_incomplete_reload_keeps_what_is_on_show() {
    let mut shelf = Shelf::Loaded(Arc::from([song("s1"), song("s2")]));
    let found = RecentlyPlayed {
        songs: vec![song("s2")],
        complete: false,
    };
    finish_recently_played(&mut shelf, Ok(found));
    assert_eq!(shelf.items().len(), 2);
}

#[test]
fn an_incomplete_first_load_shows_what_it_found() {
    let mut shelf = Shelf::Loading;
    let found = RecentlyPlayed {
        songs: vec![song("s2")],
        complete: false,
    };
    finish_recently_played(&mut shelf, Ok(found));
    assert_eq!(shelf.items().len(), 1);
}

#[gpui_kit::test]
fn a_retry_loads_again_the_shelf_that_failed(cx: &mut TestAppContext) {
    let (shelves, recommendations) = shelves_with(session(NOWHERE), cx);
    wait_for(cx, || {
        !is_loading(&shelves.status()) && !recommendations.status().is_recommending()
    });
    assert!(matches!(shelves.status().most_played(), Shelf::Failed(_)));

    shelves.retry(HomeSection::MostPlayed);
    assert_eq!(*shelves.status().most_played(), Shelf::Loading);
    shelves.retry(HomeSection::Recommended);
    assert!(recommendations.status().is_recommending());

    wait_for(cx, || {
        !is_loading(&shelves.status()) && !recommendations.status().is_recommending()
    });
}
