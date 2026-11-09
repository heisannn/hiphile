use gpui_kit::TestAppContext;
use serde_json::json;

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::{NOWHERE, server_answering, wait_for};

fn album_json(id: &str, songs: &[&str]) -> serde_json::Value {
    let songs: Vec<_> = songs
        .iter()
        .map(|song| json!({ "id": song, "title": song }))
        .collect();
    json!({ "id": id, "name": id, "song": songs })
}

fn album_with(id: &str, songs: &[&str]) -> AlbumWithSongs {
    serde_json::from_value(album_json(id, songs)).unwrap()
}

/// A server that lists `0`'s albums and nothing else.
struct Listing(Vec<Album>);

impl catalog::Source for Listing {
    async fn albums_page(&self, _: u32, _: u32) -> Result<Vec<Album>, client::Error> {
        Ok(self.0.clone())
    }

    async fn artists(&self) -> Result<Vec<subsonic::Artist>, client::Error> {
        Ok(Vec::new())
    }

    async fn songs_page(&self, _: u32, _: u32) -> Result<Vec<Song>, client::Error> {
        Ok(Vec::new())
    }

    async fn scan_status(&self) -> Result<subsonic::ScanStatus, client::Error> {
        Ok(serde_json::from_value(json!({ "scanning": false })).unwrap())
    }

    async fn library_modified(&self) -> Result<Option<u64>, client::Error> {
        Ok(None)
    }
}

/// A library saved on disk with `albums` and their songs in it.
fn saved(albums: &[AlbumWithSongs]) -> (tempfile::TempDir, catalog::Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let listing = Listing(albums.iter().map(|album| album.album().clone()).collect());
    futures::executor::block_on(catalog::sync_albums(
        &catalog,
        &listing,
        catalog::Mode::Always,
    ))
    .unwrap();
    for album in albums {
        catalog.put_album(album).unwrap();
    }
    (dir, catalog)
}

/// On GPUI's executor, which the test's `run_until_parked` drives.
fn detail_on(catalog: Option<catalog::Catalog>, url: &str, cx: &mut TestAppContext) -> AlbumDetail {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    AlbumDetail::new(executor, session_saving_into(catalog, url))
}

fn song_ids(detail: &AlbumDetail) -> Vec<String> {
    let snapshot = detail.snapshot();
    let loaded = snapshot.loaded().expect("an album on show");
    loaded
        .songs()
        .iter()
        .map(|song| song.id().to_string())
        .collect()
}

#[gpui_kit::test]
fn a_saved_album_shows_without_the_server(cx: &mut TestAppContext) {
    let (_dir, catalog) = saved(&[album_with("a", &["a1"])]);
    let detail = detail_on(Some(catalog), NOWHERE, cx);

    detail.show(AlbumId::new("a"));
    assert_eq!(
        detail.snapshot(),
        AlbumDetailSnapshot::Loading(AlbumId::new("a"))
    );

    wait_for(cx, || detail.snapshot().loaded().is_some());
    assert_eq!(song_ids(&detail), ["a1"]);
}

#[gpui_kit::test]
fn the_servers_album_replaces_the_saved_one_and_is_saved(cx: &mut TestAppContext) {
    let (_dir, catalog) = saved(&[album_with("a", &["a1"])]);
    let url = server_answering(json!({ "album": album_json("a", &["a1", "a2"]) }));
    let detail = detail_on(Some(catalog.clone()), &url, cx);

    detail.show(AlbumId::new("a"));

    wait_for(cx, || {
        detail
            .snapshot()
            .loaded()
            .is_some_and(|loaded| loaded.songs().len() == 2)
    });
    wait_for(cx, || {
        let kept = catalog.album(&AlbumId::new("a")).unwrap().unwrap();
        kept.songs().len() == 2
    });
}

#[gpui_kit::test]
fn an_album_neither_saved_nor_reached_fails_with_why(cx: &mut TestAppContext) {
    let detail = detail_on(None, NOWHERE, cx);

    detail.show(AlbumId::new("a"));

    wait_for(cx, || {
        matches!(detail.snapshot(), AlbumDetailSnapshot::Failed { .. })
    });
    assert_eq!(
        detail.snapshot(),
        AlbumDetailSnapshot::Failed {
            album: AlbumId::new("a"),
            error: "Couldn't reach the server. Check your connection and try again.".to_owned(),
        }
    );
}

#[gpui_kit::test]
fn a_failure_leaves_the_saved_songs_on_show(cx: &mut TestAppContext) {
    let detail = detail_on(None, NOWHERE, cx);
    let saved = AlbumDetailSnapshot::Loaded(album_with("a", &["a1"]).into());

    detail.0.state.update(|state| {
        state.detail = saved.clone();
        finish(state, AlbumId::new("a"), Err("offline".to_owned()));
    });

    assert_eq!(detail.snapshot(), saved);
}

#[gpui_kit::test]
fn what_an_album_replaced_brings_is_dropped(cx: &mut TestAppContext) {
    let (_dir, catalog) = saved(&[album_with("a", &["a1"]), album_with("b", &["b1"])]);
    let detail = detail_on(Some(catalog), NOWHERE, cx);
    detail.show(AlbumId::new("a"));
    let replaced = detail.0.state.read(|state| state.work_id);

    detail.show(AlbumId::new("b"));
    detail.downgrade().change(replaced, |state| {
        finish(state, AlbumId::new("a"), Ok(album_with("a", &["a1"])));
    });
    assert_eq!(
        detail.snapshot(),
        AlbumDetailSnapshot::Loading(AlbumId::new("b"))
    );

    wait_for(cx, || detail.snapshot().loaded().is_some());
    assert_eq!(song_ids(&detail), ["b1"]);
}

#[gpui_kit::test]
fn clearing_leaves_nothing_on_show(cx: &mut TestAppContext) {
    let (_dir, catalog) = saved(&[album_with("a", &["a1"])]);
    let detail = detail_on(Some(catalog), NOWHERE, cx);
    detail.show(AlbumId::new("a"));

    detail.clear();
    cx.run_until_parked();

    assert_eq!(detail.snapshot(), AlbumDetailSnapshot::None);
}

fn songs_found(
    catalog: Option<catalog::Catalog>,
    url: &str,
    cx: &mut TestAppContext,
) -> Result<Vec<String>, String> {
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let session = session_saving_into(catalog, url);
    let task = album_songs(&executor, &session, AlbumId::new("a"));
    let found = std::sync::Arc::new(std::sync::Mutex::new(None));
    executor
        .spawn({
            let found = found.clone();
            async move { *found.lock().unwrap() = Some(task.await) }
        })
        .detach();
    wait_for(cx, || found.lock().unwrap().is_some());
    let songs = found.lock().unwrap().take().unwrap()?;
    Ok(songs.iter().map(|song| song.id().to_string()).collect())
}

#[gpui_kit::test]
fn the_songs_to_act_on_are_the_saved_ones_when_there_are_any(cx: &mut TestAppContext) {
    let (_dir, catalog) = saved(&[album_with("a", &["a1"])]);
    let url = server_answering(json!({ "album": album_json("a", &["a1", "a2"]) }));

    assert_eq!(songs_found(Some(catalog), &url, cx).unwrap(), ["a1"]);
}

#[gpui_kit::test]
fn the_songs_to_act_on_come_from_the_server_when_none_are_saved(cx: &mut TestAppContext) {
    let (_dir, catalog) = saved(&[]);
    let url = server_answering(json!({ "album": album_json("a", &["a1", "a2"]) }));

    assert_eq!(songs_found(Some(catalog), &url, cx).unwrap(), ["a1", "a2"]);
}

#[gpui_kit::test]
fn songs_that_cannot_be_had_say_so_and_why(cx: &mut TestAppContext) {
    assert_eq!(
        songs_found(None, NOWHERE, cx),
        Err(
            "Couldn't load this album. Couldn't reach the server. Check your connection and try \
             again."
                .to_owned()
        )
    );
}
