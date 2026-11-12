use executor::testing::TestExecutor;
use serde_json::{Value, json};

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::{NOWHERE, Saved, keep, server_answering, wait_for};

fn album_json(id: &str, name: &str, artist: &str, created: &str) -> Value {
    json!({ "id": id, "name": name, "artist": artist, "created": created })
}

/// In the server's name order.
fn albums_json() -> Value {
    json!([
        album_json("b", "Blue Train", "John Coltrane", "2024-01-01T00:00:00Z"),
        album_json("g", "Giant Steps", "John Coltrane", "2025-01-01T00:00:00Z"),
        album_json("k", "Kind of Blue", "Miles Davis", "2023-01-01T00:00:00Z"),
    ])
}

fn album_list(
    catalog: Option<catalog::Catalog>,
    url: &str,
    cx: &TestExecutor,
) -> (AlbumList, LibrarySync) {
    // The requests wake the test from the network's own thread
    cx.allow_parking();
    let executor = cx.executor();
    let session = session_saving_into(catalog, url);
    let sync = LibrarySync::new(executor.clone(), session.clone());
    let list = AlbumList::new(executor, session, &sync);
    (list, sync)
}

/// The albums of [`albums_json`] on show, once the first sync is over.
fn shown_list(cx: &TestExecutor) -> (AlbumList, LibrarySync) {
    let url = server_answering(json!({ "albumList2": { "album": albums_json() } }));
    let (list, sync) = album_list(None, &url, cx);
    wait_until_shown(&list, cx);
    (list, sync)
}

fn wait_until_shown(list: &AlbumList, cx: &TestExecutor) {
    wait_for(cx, || {
        let status = list.status();
        !status.is_updating() && status.shown().is_some()
    });
}

fn shown(list: &AlbumList) -> Vec<String> {
    let status = list.status();
    let Some(shown) = status.shown() else {
        return Vec::new();
    };
    let ids = shown.visible().iter().map(|ix| shown.items()[*ix].id());
    ids.map(ToString::to_string).collect()
}

#[test]
fn the_albums_a_sync_brings_show_in_the_servers_order() {
    let cx = &TestExecutor::new();
    let url = server_answering(json!({ "albumList2": { "album": albums_json() } }));
    let (list, _sync) = album_list(None, &url, cx);
    assert!(list.status().is_updating());

    wait_until_shown(&list, cx);

    assert_eq!(shown(&list), ["b", "g", "k"]);
    assert_eq!(list.status().failure(), None);
}

#[test]
fn a_query_finds_albums_by_name_or_artist_the_best_first() {
    let cx = &TestExecutor::new();
    let (list, _sync) = shown_list(cx);

    list.set_query("blue");
    cx.run_until_parked();
    assert_eq!(shown(&list), ["b", "k"]);

    list.set_query("davis");
    cx.run_until_parked();
    assert_eq!(shown(&list), ["k"]);
}

#[test]
fn the_order_chosen_arranges_what_the_query_finds() {
    let cx = &TestExecutor::new();
    let (list, _sync) = shown_list(cx);

    list.choose_order(AlbumOrder::Added);
    cx.run_until_parked();
    assert_eq!(shown(&list), ["g", "b", "k"]);

    list.set_query("coltrane");
    list.choose_direction(false);
    cx.run_until_parked();
    assert_eq!(shown(&list), ["b", "g"]);
}

#[test]
fn choosing_the_order_already_chosen_keeps_its_direction() {
    let cx = &TestExecutor::new();
    let (list, _sync) = shown_list(cx);

    list.choose_order(AlbumOrder::Added);
    assert!(list.status().sort().is_descending);
    list.choose_direction(false);
    list.choose_order(AlbumOrder::Added);
    assert!(!list.status().sort().is_descending);

    list.choose_order(AlbumOrder::Released);
    assert_eq!(
        list.status().sort(),
        AlbumSort::chosen(AlbumOrder::Released)
    );
}

#[test]
fn the_same_albums_synced_again_keep_their_arc() {
    let cx = &TestExecutor::new();
    let (list, sync) = shown_list(cx);
    let before = list.status().shown().unwrap().items().clone();

    sync.refresh();
    wait_for(cx, || !sync.status().is_running());
    cx.run_until_parked();

    let status = list.status();
    assert!(Arc::ptr_eq(status.shown().unwrap().items(), &before));
    assert!(!status.is_updating());
}

#[test]
fn the_saved_albums_stay_with_word_of_a_failed_sync() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let saved = Saved {
        albums: serde_json::from_value(albums_json()).unwrap(),
        ..Saved::default()
    };
    keep(&catalog, &saved);
    let (list, sync) = album_list(Some(catalog), NOWHERE, cx);
    wait_until_shown(&list, cx);

    sync.refresh();
    wait_for(cx, || !sync.status().is_running());

    assert_eq!(shown(&list), ["b", "g", "k"]);
    assert!(list.status().failure().is_some());
}

#[test]
fn albums_that_never_came_fail_as_the_sync_did() {
    let cx = &TestExecutor::new();
    let (list, sync) = album_list(None, NOWHERE, cx);

    wait_for(cx, || !sync.status().is_running());
    cx.run_until_parked();

    let status = list.status();
    assert!(matches!(status.albums(), Shelf::Failed(_)));
    assert!(!status.is_updating());
}
