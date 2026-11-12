use executor::testing::TestExecutor;
use serde_json::{Value, json};

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::{NOWHERE, Saved, keep, server_answering, wait_for};

fn song_json(id: &str, title: &str, seconds: u64) -> Value {
    json!({
        "id": id, "title": title, "duration": seconds,
        "artist": format!("Artist of {title}"), "album": format!("Album of {title}"),
    })
}

fn songs() -> Vec<Song> {
    [
        ("c", "Cherokee", 100),
        ("a", "Airegin", 300),
        ("b", "Blues", 200),
    ]
    .iter()
    .map(|(id, title, seconds)| serde_json::from_value(song_json(id, title, *seconds)).unwrap())
    .collect()
}

fn saved_songs(songs: Vec<Song>) -> (tempfile::TempDir, catalog::Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    keep(
        &catalog,
        &Saved {
            songs,
            ..Saved::default()
        },
    );
    (dir, catalog)
}

fn song_list(catalog: Option<catalog::Catalog>, url: &str, cx: &TestExecutor) -> SongList {
    // The requests wake the test from the network's own thread
    cx.allow_parking();
    let executor = cx.executor();
    SongList::new(executor, session_saving_into(catalog, url))
}

/// Waits for the songs read last to show.
fn wait_until_shown(list: &SongList, cx: &TestExecutor) {
    wait_for(cx, || {
        let status = list.status();
        !status.is_loading()
            && matches!(
                (status.songs(), status.shown()),
                (Shelf::Loaded(read), Some(shown)) if Arc::ptr_eq(read, shown.items())
            )
    });
}

fn shown(list: &SongList) -> Vec<String> {
    let status = list.status();
    let Some(shown) = status.shown() else {
        return Vec::new();
    };
    let songs = shown.items();
    shown
        .visible()
        .iter()
        .map(|ix| songs[*ix].id().to_string())
        .collect()
}

/// Three songs read from `catalog` and on show.
fn shown_list(cx: &TestExecutor) -> (SongList, tempfile::TempDir, catalog::Catalog) {
    let (dir, catalog) = saved_songs(songs());
    let list = song_list(Some(catalog.clone()), NOWHERE, cx);
    list.open();
    wait_until_shown(&list, cx);
    (list, dir, catalog)
}

#[test]
fn nothing_is_read_until_the_list_opens() {
    let cx = &TestExecutor::new();
    let list = song_list(None, NOWHERE, cx);

    list.reload();
    cx.run_until_parked();

    let status = list.status();
    assert!(!status.is_loading());
    assert_eq!(*status.songs(), Shelf::Loading);
}

#[test]
fn with_nothing_saved_the_servers_songs_show_by_title() {
    let cx = &TestExecutor::new();
    let url = server_answering(json!({ "searchResult3": { "song": [
        song_json("c", "Cherokee", 100),
        song_json("a", "Airegin", 300),
        song_json("b", "Blues", 200),
    ]}}));
    let list = song_list(None, &url, cx);

    list.open();
    assert!(list.status().is_loading());
    wait_until_shown(&list, cx);

    assert_eq!(shown(&list), ["a", "b", "c"]);
}

#[test]
fn the_saved_songs_show_without_asking_the_server() {
    let cx = &TestExecutor::new();
    let (list, _dir, _catalog) = shown_list(cx);

    assert_eq!(shown(&list), ["a", "b", "c"]);
    assert_eq!(list.status().failure(), None);
}

#[test]
fn other_songs_read_again_replace_those_on_show() {
    let cx = &TestExecutor::new();
    let (list, _dir, catalog) = shown_list(cx);
    let one_song = Saved {
        songs: songs().into_iter().take(1).collect(),
        ..Saved::default()
    };
    keep(&catalog, &one_song);

    list.reload();
    wait_until_shown(&list, cx);

    assert_eq!(shown(&list), ["c"]);
}

#[test]
fn the_same_songs_read_again_keep_their_index() {
    let cx = &TestExecutor::new();
    let (list, _dir, _catalog) = shown_list(cx);
    let before = list.status().shown().unwrap().items().clone();

    list.reload();
    wait_for(cx, || !list.status().is_loading());
    list.sort_by(Column::Time);
    cx.run_until_parked();

    let status = list.status();
    assert!(Arc::ptr_eq(status.shown().unwrap().items(), &before));
    assert_eq!(shown(&list), ["c", "b", "a"]);
}

#[test]
fn a_failed_first_reading_says_why() {
    let cx = &TestExecutor::new();
    let list = song_list(None, NOWHERE, cx);

    list.open();
    wait_for(cx, || !list.status().is_loading());

    let status = list.status();
    assert!(matches!(status.songs(), Shelf::Failed(_)));
    assert_eq!(status.failure(), None);
}

#[test]
fn a_failed_reload_keeps_the_songs_until_a_new_try_starts() {
    let cx = &TestExecutor::new();
    let (list, _dir, catalog) = shown_list(cx);
    // With nothing saved, the server is asked, and cannot be reached
    keep(&catalog, &Saved::default());

    list.reload();
    wait_for(cx, || !list.status().is_loading());

    assert!(list.status().failure().is_some());
    assert_eq!(shown(&list), ["a", "b", "c"]);
    list.reload();
    assert_eq!(list.status().failure(), None);
}

#[test]
fn a_sort_chosen_during_the_first_reading_holds_when_the_songs_arrive() {
    let cx = &TestExecutor::new();
    let (_dir, catalog) = saved_songs(songs());
    let list = song_list(Some(catalog), NOWHERE, cx);

    list.open();
    list.sort_by(Column::Time);
    wait_until_shown(&list, cx);

    assert_eq!(shown(&list), ["c", "b", "a"]);
}

#[test]
fn a_query_given_during_a_reload_holds_when_the_songs_arrive() {
    let cx = &TestExecutor::new();
    let (list, _dir, _catalog) = shown_list(cx);

    list.reload();
    list.set_query("blues");
    cx.run_until_parked();
    assert_eq!(shown(&list), ["b"]);
    wait_until_shown(&list, cx);

    assert_eq!(shown(&list), ["b"]);
}

#[test]
fn the_last_order_asked_for_is_the_one_that_shows() {
    let cx = &TestExecutor::new();
    let (list, _dir, _catalog) = shown_list(cx);

    list.sort_by(Column::Time);
    list.set_query("Cherokee");
    cx.run_until_parked();

    assert_eq!(shown(&list), ["c"]);
}

#[test]
fn a_sort_chosen_during_a_query_holds_until_the_query_is_emptied() {
    let cx = &TestExecutor::new();
    let (list, _dir, _catalog) = shown_list(cx);

    list.set_query("e");
    assert!(list.status().is_ranked());
    list.sort_by(Column::Time);
    assert!(!list.status().is_ranked());
    // No heading showed as sorted, so the first click starts from the top
    assert!(!list.status().sort().is_descending);

    list.set_query("ee");
    assert!(!list.status().is_ranked());
    list.set_query("");
    list.set_query("e");
    assert!(list.status().is_ranked());
    assert_eq!(list.status().sort().column, Column::Time);
}

#[test]
fn choosing_the_sort_already_chosen_keeps_its_direction() {
    let cx = &TestExecutor::new();
    let (list, _dir, _catalog) = shown_list(cx);

    list.choose_sort(Column::Added);
    assert!(list.status().sort().is_descending);
    list.choose_direction(false);
    list.choose_sort(Column::Added);

    assert!(!list.status().sort().is_descending);
}

#[test]
fn a_library_without_songs_shows_as_loaded() {
    let cx = &TestExecutor::new();
    let url = server_answering(json!({ "searchResult3": { "song": [] }}));
    let list = song_list(None, &url, cx);

    list.open();
    wait_for(cx, || !list.status().is_loading());
    cx.run_until_parked();

    let status = list.status();
    assert!(status.songs().is_loaded());
    assert!(status.shown().is_some());
}
