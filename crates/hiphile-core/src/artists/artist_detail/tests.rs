use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::TestAppContext;
use serde_json::{Value, json};

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::{NOWHERE, Saved, keep, server_answering_each};

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

fn answer(method: &str) -> Option<Value> {
    Some(match method {
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

/// A server that knows of one artist's whole page, which it gives for any artist.
fn server() -> String {
    server_answering_each(answer)
}

fn server_of_no_albums() -> String {
    server_answering_each(|method| match method {
        "getArtist" => Some(json!({ "artist": { "id": "x", "name": "Guest" } })),
        method => answer(method),
    })
}

fn artist_detail(
    catalog: Option<catalog::Catalog>,
    url: &str,
    cx: &mut TestAppContext,
) -> ArtistDetail {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    ArtistDetail::new(executor, session_saving_into(catalog, url))
}

fn page(detail: &ArtistDetail) -> ArtistPage {
    detail.page().expect("an artist on show")
}

/// Runs what is under way until `done`, past the delay before asking the server.
fn wait_until(detail: &ArtistDetail, cx: &mut TestAppContext, done: impl Fn(&ArtistPage) -> bool) {
    for _ in 0..500 {
        cx.executor().advance_clock(SERVER_DELAY);
        cx.run_until_parked();
        if detail.page().as_ref().is_some_and(&done) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("what was waited for never happened");
}

/// Runs what is under way until `task` ends.
fn result_of<R>(mut task: Task<R>, cx: &mut TestAppContext) -> R {
    for _ in 0..500 {
        cx.run_until_parked();
        if let Some(result) = futures::FutureExt::now_or_never(&mut task) {
            return result;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the task never ended");
}

fn listed(id: &str, name: &str) -> Artist {
    Artist::new(ArtistId::new(id), name.to_owned(), None, Some(1))
}

fn songs(count: usize) -> Vec<Song> {
    (0..count)
        .map(|ix| serde_json::from_value(song_json(ix)).unwrap())
        .collect()
}

fn song_by(id: &str, artist: &str, album: &str, track: u32) -> Song {
    serde_json::from_value(json!({
        "id": id, "title": id, "artistId": artist, "album": album, "track": track,
    }))
    .unwrap()
}

fn guest_songs() -> Saved {
    Saved {
        songs: vec![
            song_by("b1", "x", "Blue", 1),
            song_by("other", "m", "Blue", 2),
            song_by("a1", "x", "Agharta", 1),
        ],
        ..Saved::default()
    }
}

fn song_ids(songs: &[Song]) -> Vec<String> {
    songs.iter().map(|song| song.id().to_string()).collect()
}

/// Ends the albums' work as if the server had answered `result` and the saved songs naming the
/// artist were `songs`.
fn finish_albums(detail: &ArtistDetail, result: Result<Vec<Album>, String>, songs: Vec<Song>) {
    let found = result.map(|albums| {
        let artist = Artist::new(ArtistId::new("m"), "Miles Davis".to_owned(), None, None);
        (artist, albums)
    });
    detail
        .0
        .state
        .update(|state| detail.finish_albums(state, found, songs));
}

#[gpui_kit::test]
fn showing_an_artist_fills_every_section(cx: &mut TestAppContext) {
    let url = server();
    let detail = artist_detail(None, &url, cx);

    detail.show(&ArtistId::new("m"), Some(&listed("m", "Miles Davis")));
    assert_eq!(page(&detail).name.as_deref(), Some("Miles Davis"));
    wait_until(&detail, cx, |page| {
        page.albums.is_loaded() && page.top_songs.is_loaded() && page.similar.is_loaded()
    });

    let page = page(&detail);
    let years: Vec<_> = page.albums.items().iter().map(Album::year).collect();
    assert_eq!(years, [Some(1970), Some(1959)]);
    assert_eq!(page.top_songs.items().len(), 10);
    assert_eq!(page.similar.items().len(), 1);
}

#[gpui_kit::test]
fn a_failed_section_leaves_the_rest(cx: &mut TestAppContext) {
    let url = server_answering_each(|method| match method {
        "getTopSongs" => Some(json!({
            "status": "failed", "error": { "code": 0, "message": "No outside service" },
        })),
        method => answer(method),
    });
    let detail = artist_detail(None, &url, cx);

    detail.show(&ArtistId::new("m"), Some(&listed("m", "Miles Davis")));
    wait_until(&detail, cx, |page| {
        page.albums.is_loaded() && page.similar.is_loaded() && page.top_songs != Shelf::Loading
    });

    let Shelf::Failed(error) = page(&detail).top_songs else {
        panic!("the top songs did not fail");
    };
    assert!(error.starts_with("Couldn't load top songs."), "{error}");
}

#[gpui_kit::test]
fn moving_through_artists_asks_the_server_only_about_where_it_stops(cx: &mut TestAppContext) {
    static ASKED: AtomicUsize = AtomicUsize::new(0);
    let url = server_answering_each(|method| {
        if method == "getArtist" {
            ASKED.fetch_add(1, Ordering::SeqCst);
        }
        answer(method)
    });
    let detail = artist_detail(None, &url, cx);

    detail.show(&ArtistId::new("b"), Some(&listed("b", "Bill Evans")));
    cx.executor().advance_clock(SERVER_DELAY / 2);
    cx.run_until_parked();
    detail.show(&ArtistId::new("m"), Some(&listed("m", "Miles Davis")));
    wait_until(&detail, cx, |page| page.albums.is_loaded());

    assert_eq!(ASKED.load(Ordering::SeqCst), 1);
    assert_eq!(page(&detail).id, ArtistId::new("m"));
}

#[gpui_kit::test]
fn an_artist_off_the_list_shows_once_the_server_names_them(cx: &mut TestAppContext) {
    let url = server();
    let detail = artist_detail(None, &url, cx);

    detail.show(&ArtistId::new("only-on-songs"), None);
    assert_eq!(page(&detail).name, None);
    // Top songs are asked for by name
    wait_until(&detail, cx, |page| page.top_songs.is_loaded());

    assert_eq!(page(&detail).name.as_deref(), Some("Miles Davis"));
}

#[gpui_kit::test]
fn the_saved_albums_show_when_the_server_cannot_be_reached(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let saved = Saved {
        albums: vec![serde_json::from_value(album_json("a1", 1959)).unwrap()],
        ..Saved::default()
    };
    keep(&catalog, &saved);
    let detail = artist_detail(Some(catalog), NOWHERE, cx);

    detail.show(&ArtistId::new("m"), None);
    wait_until(&detail, cx, |page| {
        page.albums.is_loaded() && page.similar != Shelf::Loading
    });

    let page = page(&detail);
    assert_eq!(page.name.as_deref(), Some("Miles Davis"));
    assert_eq!(page.albums.items().len(), 1);
}

#[gpui_kit::test]
fn the_saved_songs_of_an_artist_with_no_albums_make_their_page_and_play(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    keep(&catalog, &guest_songs());
    let url = server_of_no_albums();
    let detail = artist_detail(Some(catalog), &url, cx);

    detail.show(&ArtistId::new("x"), None);
    wait_until(&detail, cx, |page| page.is_only_on_songs);
    assert_eq!(song_ids(page(&detail).top_songs.items()), ["a1", "b1"]);

    // Their songs are all there is: asking for top songs leaves them
    detail.reload_top_songs();
    cx.executor().advance_clock(SERVER_DELAY);
    cx.run_until_parked();
    assert_eq!(song_ids(page(&detail).top_songs.items()), ["a1", "b1"]);

    // Their albums being none, what plays comes from the saved songs too
    let played = result_of(detail.songs(&ArtistId::new("x")), cx);
    assert_eq!(song_ids(&played.unwrap()), ["a1", "b1"]);
}

#[gpui_kit::test]
fn songs_arriving_in_a_sync_fill_the_page_of_an_artist_with_no_albums(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let url = server_of_no_albums();
    let detail = artist_detail(Some(catalog.clone()), &url, cx);
    detail.show(&ArtistId::new("x"), None);
    wait_until(&detail, cx, |page| page.albums.is_loaded());
    assert!(!page(&detail).is_only_on_songs);

    keep(&catalog, &guest_songs());
    detail.songs_synced();
    wait_until(&detail, cx, |page| page.is_only_on_songs);

    assert_eq!(song_ids(page(&detail).top_songs.items()), ["a1", "b1"]);
}

#[gpui_kit::test]
fn an_artist_with_no_songs_at_all_has_no_albums_on_show(cx: &mut TestAppContext) {
    let detail = artist_detail(None, NOWHERE, cx);
    detail.show(&ArtistId::new("m"), Some(&listed("m", "Miles Davis")));

    finish_albums(&detail, Ok(Vec::new()), Vec::new());

    let page = page(&detail);
    assert_eq!(page.albums, Shelf::Loaded(Arc::from([])));
    assert!(!page.is_only_on_songs);
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_songs_of_an_artist_only_on_songs(cx: &mut TestAppContext) {
    let detail = artist_detail(None, NOWHERE, cx);
    detail.show(&ArtistId::new("m"), Some(&listed("m", "Miles Davis")));
    finish_albums(&detail, Ok(Vec::new()), songs(3));

    finish_albums(&detail, Err("offline".to_owned()), Vec::new());

    let page = page(&detail);
    assert!(page.is_only_on_songs);
    assert_eq!(page.top_songs.items().len(), 3);
}

#[gpui_kit::test]
fn albums_arriving_later_bring_back_the_albums_and_the_top_songs(cx: &mut TestAppContext) {
    let url = server();
    let detail = artist_detail(None, &url, cx);
    detail.show(&ArtistId::new("m"), Some(&listed("m", "Miles Davis")));
    finish_albums(&detail, Ok(Vec::new()), songs(3));

    let album: Album = serde_json::from_value(album_json("a1", 1959)).unwrap();
    finish_albums(&detail, Ok(vec![album]), Vec::new());

    wait_until(&detail, cx, |page| {
        page.top_songs.items().len() == 10 && !page.is_only_on_songs
    });
}

#[gpui_kit::test]
fn closing_drops_the_page_and_what_was_on_its_way(cx: &mut TestAppContext) {
    let url = server();
    let detail = artist_detail(None, &url, cx);
    detail.show(&ArtistId::new("m"), Some(&listed("m", "Miles Davis")));

    detail.close();
    for _ in 0..10 {
        cx.executor().advance_clock(SERVER_DELAY);
        cx.run_until_parked();
    }

    assert_eq!(detail.page(), None);
}

#[gpui_kit::test]
fn an_artists_songs_come_album_by_album(cx: &mut TestAppContext) {
    let url = server();
    let detail = artist_detail(None, &url, cx);

    let songs = result_of(detail.songs(&ArtistId::new("m")), cx);

    // Two albums of two songs each
    assert_eq!(songs.unwrap().len(), 4);
}
