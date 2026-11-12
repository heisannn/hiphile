use executor::testing::TestExecutor;
use serde_json::json;
use subsonic::Album;

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::{NOWHERE, Saved, keep, server_answering, wait_for};

fn artist_list(catalog: Option<catalog::Catalog>, url: &str, cx: &TestExecutor) -> ArtistList {
    // The requests wake the test from the network's own thread
    cx.allow_parking();
    let executor = cx.executor();
    ArtistList::new(executor, session_saving_into(catalog, url))
}

fn saved_artist(id: &str, name: &str) -> Artist {
    serde_json::from_value(json!({ "id": id, "name": name })).unwrap()
}

fn names(list: &ArtistList) -> Vec<String> {
    let status = list.status();
    let artists = status.artists().items();
    artists.iter().map(|it| it.name().to_owned()).collect()
}

fn wait_until_read(list: &ArtistList, cx: &TestExecutor) {
    wait_for(cx, || !list.status().is_refreshing());
}

#[test]
fn nothing_is_read_until_the_list_opens() {
    let cx = &TestExecutor::new();
    let list = artist_list(None, NOWHERE, cx);

    list.reload();
    cx.run_until_parked();

    let status = list.status();
    assert!(!status.is_refreshing());
    assert_eq!(*status.artists(), Shelf::Loading);
}

#[test]
fn with_nothing_saved_the_servers_artists_show() {
    let cx = &TestExecutor::new();
    let url = server_answering(json!({ "artists": { "index": [
        { "name": "B", "artist": [{ "id": "b", "name": "Bill Evans", "albumCount": 1 }] },
        { "name": "M", "artist": [{ "id": "m", "name": "Miles Davis", "albumCount": 2 }] },
    ]}}));
    let list = artist_list(None, &url, cx);

    list.open();
    assert!(list.status().is_refreshing());
    wait_until_read(&list, cx);

    assert_eq!(names(&list), ["Bill Evans", "Miles Davis"]);
}

#[test]
fn the_saved_artists_show_without_asking_the_server() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let saved = Saved {
        artists: vec![saved_artist("m", "Miles Davis")],
        ..Saved::default()
    };
    keep(&catalog, &saved);
    let list = artist_list(Some(catalog), NOWHERE, cx);

    list.open();
    wait_until_read(&list, cx);

    assert_eq!(names(&list), ["Miles Davis"]);
    assert_eq!(list.status().failure(), None);
}

#[test]
fn a_reload_reads_the_saved_artists_again() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let saved = |artists| Saved {
        artists,
        ..Saved::default()
    };
    keep(&catalog, &saved(vec![saved_artist("m", "Miles Davis")]));
    let list = artist_list(Some(catalog.clone()), NOWHERE, cx);
    list.open();
    wait_until_read(&list, cx);

    keep(&catalog, &saved(vec![saved_artist("j", "John Coltrane")]));
    list.reload();
    wait_until_read(&list, cx);

    assert_eq!(names(&list), ["John Coltrane"]);
}

#[test]
fn the_saved_albums_name_the_artists_and_stay_when_the_server_cannot_be_reached() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let album: Album = serde_json::from_value(json!({
        "id": "a1", "name": "Kind of Blue", "year": 1959,
        "artistId": "m", "artist": "Miles Davis",
    }))
    .unwrap();
    let saved = Saved {
        albums: vec![album],
        ..Saved::default()
    };
    keep(&catalog, &saved);
    let list = artist_list(Some(catalog), NOWHERE, cx);

    list.open();
    wait_until_read(&list, cx);

    let status = list.status();
    assert_eq!(names(&list), ["Miles Davis"]);
    assert!(status.failure().is_some());
    assert_eq!(
        *status.dates(),
        Arc::new(ArtistDates::of_albums(&saved.albums))
    );
}

#[test]
fn failing_to_read_a_first_list_says_why() {
    let cx = &TestExecutor::new();
    let list = artist_list(None, NOWHERE, cx);

    list.open();
    wait_until_read(&list, cx);

    let status = list.status();
    assert!(matches!(status.artists(), Shelf::Failed(_)));
    assert_eq!(status.failure(), None);
}

fn shown(list: &ArtistList) -> Vec<String> {
    let status = list.status();
    let Some(shown) = status.shown() else {
        return Vec::new();
    };
    let ids = shown.visible().iter().map(|ix| shown.items()[*ix].id());
    ids.map(ToString::to_string).collect()
}

fn wait_until_shown(list: &ArtistList, cx: &TestExecutor) {
    wait_for(cx, || {
        let status = list.status();
        !status.is_refreshing() && status.shown().is_some()
    });
}

fn dated_album(id: &str, artist: &str, created: &str) -> Album {
    serde_json::from_value(json!({
        "id": id, "name": id, "artistId": artist, "artist": artist, "created": created,
    }))
    .unwrap()
}

/// Bill Evans and Miles Davis saved, with an album each added as `added` says.
fn saved_artists(added: [&str; 2]) -> Saved {
    Saved {
        artists: vec![
            saved_artist("b", "Bill Evans"),
            saved_artist("m", "Miles Davis"),
        ],
        albums: vec![
            dated_album("b1", "b", added[0]),
            dated_album("m1", "m", added[1]),
        ],
        ..Saved::default()
    }
}

/// The artists of [`saved_artists`] on show.
fn shown_list(
    added: [&str; 2],
    cx: &TestExecutor,
) -> (ArtistList, tempfile::TempDir, catalog::Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    keep(&catalog, &saved_artists(added));
    let list = artist_list(Some(catalog.clone()), NOWHERE, cx);
    list.open();
    wait_until_shown(&list, cx);
    (list, dir, catalog)
}

const OLDER: &str = "2024-01-01T00:00:00Z";
const NEWER: &str = "2025-01-01T00:00:00Z";

#[test]
fn a_query_finds_artists_by_name() {
    let cx = &TestExecutor::new();
    let (list, _dir, _catalog) = shown_list([OLDER, NEWER], cx);
    assert_eq!(shown(&list), ["b", "m"]);

    list.set_query("EVANS");
    cx.run_until_parked();

    assert_eq!(shown(&list), ["b"]);
}

#[test]
fn the_order_chosen_goes_by_each_artists_newest_album() {
    let cx = &TestExecutor::new();
    let (list, _dir, _catalog) = shown_list([OLDER, NEWER], cx);

    list.choose_order(ArtistOrder::Added);
    cx.run_until_parked();
    assert_eq!(shown(&list), ["m", "b"]);

    list.choose_direction(false);
    cx.run_until_parked();
    assert_eq!(shown(&list), ["b", "m"]);
}

#[test]
fn choosing_the_order_already_chosen_keeps_its_direction() {
    let cx = &TestExecutor::new();
    let list = artist_list(None, NOWHERE, cx);

    list.choose_order(ArtistOrder::Released);
    assert!(list.status().sort().is_descending);
    list.choose_direction(false);
    list.choose_order(ArtistOrder::Released);

    assert!(!list.status().sort().is_descending);
}

#[test]
fn new_dates_for_the_same_artists_put_them_in_order_again() {
    let cx = &TestExecutor::new();
    let (list, _dir, catalog) = shown_list([OLDER, NEWER], cx);
    list.choose_order(ArtistOrder::Added);
    cx.run_until_parked();
    let before = list.status().shown().unwrap().items().clone();

    keep(&catalog, &saved_artists([NEWER, OLDER]));
    list.reload();
    wait_until_shown(&list, cx);

    assert_eq!(shown(&list), ["b", "m"]);
    assert!(Arc::ptr_eq(list.status().shown().unwrap().items(), &before));
}
