use gpui_kit::TestAppContext;
use serde_json::json;
use subsonic::Album;

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::{NOWHERE, Saved, keep, server_answering, wait_for};

fn artist_list(
    catalog: Option<catalog::Catalog>,
    url: &str,
    cx: &mut TestAppContext,
) -> ArtistList {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
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

fn wait_until_read(list: &ArtistList, cx: &mut TestAppContext) {
    wait_for(cx, || !list.status().is_refreshing());
}

#[gpui_kit::test]
fn nothing_is_read_until_the_list_opens(cx: &mut TestAppContext) {
    let list = artist_list(None, NOWHERE, cx);

    list.reload();
    cx.run_until_parked();

    let status = list.status();
    assert!(!status.is_refreshing());
    assert_eq!(*status.artists(), Shelf::Loading);
}

#[gpui_kit::test]
fn with_nothing_saved_the_servers_artists_show(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn the_saved_artists_show_without_asking_the_server(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn a_reload_reads_the_saved_artists_again(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn the_saved_albums_name_the_artists_and_stay_when_the_server_cannot_be_reached(
    cx: &mut TestAppContext,
) {
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

#[gpui_kit::test]
fn failing_to_read_a_first_list_says_why(cx: &mut TestAppContext) {
    let list = artist_list(None, NOWHERE, cx);

    list.open();
    wait_until_read(&list, cx);

    let status = list.status();
    assert!(matches!(status.artists(), Shelf::Failed(_)));
    assert_eq!(status.failure(), None);
}
