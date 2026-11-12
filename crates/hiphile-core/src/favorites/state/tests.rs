use std::time::{Duration, Instant};

use executor::testing::TestExecutor;
use serde_json::json;
use subsonic::{AlbumId, ArtistId, SongId};

use super::*;
use crate::test_support::{self, FavoritesServer, song_json};

fn favorites_on(server: &FavoritesServer, cx: &TestExecutor) -> FavoritesState {
    // The requests wake the test from the network's own thread
    cx.allow_parking();
    let executor = cx.executor();
    let session = test_support::session(server.url());
    FavoritesState::new(executor, session.client().clone())
}

fn wait_until(cx: &TestExecutor, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if done() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the favorites"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn is_settled(favorites: &FavoritesState) -> bool {
    favorites.0.state.read(|state| {
        state.favorites.is_loaded() && !state.favorites.is_reloading && !state.is_writing()
    })
}

fn settle(favorites: &FavoritesState, cx: &TestExecutor) {
    wait_until(cx, || is_settled(favorites));
}

fn is_writing(favorites: &FavoritesState) -> bool {
    favorites.0.state.read(State::is_writing)
}

fn song_ids(favorites: &FavoritesState) -> Vec<String> {
    let lists = favorites.snapshot().lists;
    lists
        .songs
        .iter()
        .map(|song| song.id().to_string())
        .collect()
}

fn target(id: &str) -> StarTarget {
    StarTarget::Song(SongId::new(id))
}

fn song(id: &str) -> Favorite {
    Favorite::Song(serde_json::from_value(song_json(id)).unwrap())
}

fn notices(notices: &mut broadcast::Receiver<FavoritesNotice>) -> Vec<FavoritesNotice> {
    std::iter::from_fn(|| notices.try_recv().ok()).collect()
}

#[test]
fn the_favorites_are_known_once_they_load() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a", "b"]);
    let favorites = favorites_on(&server, cx);
    assert_eq!(favorites.snapshot().is_favorite(&target("a")), None);

    settle(&favorites, cx);

    let snapshot = favorites.snapshot();
    assert_eq!(snapshot.is_favorite(&target("a")), Some(true));
    assert_eq!(snapshot.is_favorite(&target("z")), Some(false));
    let album = StarTarget::Album(AlbumId::new("al"));
    assert_eq!(snapshot.is_favorite(&album), Some(true));
    let artist = StarTarget::Artist(ArtistId::new("ar"));
    assert_eq!(snapshot.is_favorite(&artist), Some(true));
}

#[test]
fn a_new_favorite_shows_at_once_and_reaches_the_server() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.hold("star");

    favorites.set_favorite(song("n"), true);

    assert_eq!(song_ids(&favorites), ["n", "a"]);
    assert_eq!(favorites.snapshot().is_favorite(&target("n")), Some(true));

    server.release("star");
    settle(&favorites, cx);
    assert_eq!(server.writes(), ["star id=n"]);
}

#[test]
fn each_kind_of_favorite_is_sent_by_its_own_parameter() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&[]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    let lists = favorites.snapshot().lists;

    favorites.set_favorite(Favorite::Album(lists.albums[0].clone()), false);
    favorites.set_favorite(Favorite::Artist(lists.artists[0].clone()), false);
    settle(&favorites, cx);

    assert_eq!(server.writes(), ["unstar albumId=al", "unstar artistId=ar"]);
}

#[test]
fn asking_for_what_already_is_sends_nothing() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);

    favorites.set_favorite(song("a"), true);
    favorites.set_favorite(song("z"), false);
    settle(&favorites, cx);

    assert!(server.writes().is_empty());
}

#[test]
fn toggling_adds_what_is_no_favorite_and_takes_out_what_is() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);

    favorites.toggle(song("n"));
    favorites.toggle(song("a"));
    assert_eq!(song_ids(&favorites), ["n"]);
    settle(&favorites, cx);

    assert_eq!(server.writes(), ["star id=n", "unstar id=a"]);
}

#[test]
fn toggling_goes_by_the_changes_still_on_their_way() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.hold("star");

    favorites.toggle(song("n"));
    favorites.toggle(song("n"));
    server.release("star");
    settle(&favorites, cx);

    assert_eq!(server.writes(), ["star id=n", "unstar id=n"]);
    assert_eq!(favorites.snapshot().is_favorite(&target("n")), Some(false));
}

#[test]
fn toggling_does_nothing_until_the_favorites_load() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    server.hold("getStarred2");
    let favorites = favorites_on(&server, cx);

    favorites.toggle(song("n"));
    favorites.toggle(song("a"));
    server.release("getStarred2");
    settle(&favorites, cx);

    assert!(server.writes().is_empty());
    assert_eq!(song_ids(&favorites), ["a"]);
}

#[test]
fn a_removal_the_server_refuses_goes_back_where_it_was() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a", "b", "c"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.refuse("unstar");
    let mut told = favorites.notices();

    favorites.set_favorite(song("b"), false);
    assert_eq!(song_ids(&favorites), ["a", "c"]);
    settle(&favorites, cx);

    assert_eq!(song_ids(&favorites), ["a", "b", "c"]);
    assert_eq!(favorites.snapshot().is_favorite(&target("b")), Some(true));
    assert_eq!(
        notices(&mut told),
        [FavoritesNotice::WriteFailed(
            "Couldn't remove from favorites. The server refused the request: Refused.".to_owned()
        )]
    );
}

#[test]
fn an_addition_the_server_refuses_goes_away() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.refuse("star");

    favorites.set_favorite(song("n"), true);
    settle(&favorites, cx);

    assert_eq!(song_ids(&favorites), ["a"]);
    assert_eq!(favorites.snapshot().is_favorite(&target("n")), Some(false));
}

#[test]
fn writes_go_one_at_a_time_in_the_order_they_were_made() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.hold("star");

    favorites.set_favorite(song("n"), true);
    favorites.set_favorite(song("n"), false);
    favorites.set_favorite(song("n"), true);
    wait_until(cx, || server.writes().len() == 1);
    assert_eq!(server.writes(), ["star id=n"]);

    server.release("star");
    settle(&favorites, cx);

    assert_eq!(server.writes(), ["star id=n", "unstar id=n", "star id=n"]);
    assert_eq!(song_ids(&favorites), ["n", "a"]);
}

#[test]
fn a_refused_change_leaves_a_later_one_to_decide() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.refuse_once("star");
    server.hold("star");

    favorites.set_favorite(song("n"), true);
    favorites.set_favorite(song("n"), false);
    favorites.set_favorite(song("n"), true);
    server.release("star");
    settle(&favorites, cx);

    // Only the first was refused, and undoing it would hide what the last did
    assert_eq!(server.writes(), ["star id=n", "unstar id=n", "star id=n"]);
    assert_eq!(song_ids(&favorites), ["n", "a"]);
    assert_eq!(favorites.snapshot().is_favorite(&target("n")), Some(true));
}

// These read nothing back after the writes, to leave the undoing on its own
#[test]
fn an_addition_then_a_removal_both_refused_leave_it_out() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.refuse("star");
    server.refuse("unstar");
    server.refuse("getStarred2");
    server.hold("star");

    favorites.set_favorite(song("n"), true);
    favorites.set_favorite(song("n"), false);
    server.release("star");
    settle(&favorites, cx);

    assert_eq!(server.writes(), ["star id=n", "unstar id=n"]);
    assert_eq!(song_ids(&favorites), ["a"]);
    assert_eq!(favorites.snapshot().is_favorite(&target("n")), Some(false));
}

#[test]
fn a_removal_then_an_addition_both_refused_leave_it_where_it_was() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a", "b", "c"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.refuse("star");
    server.refuse("unstar");
    server.refuse("getStarred2");
    server.hold("unstar");

    favorites.set_favorite(song("b"), false);
    favorites.set_favorite(song("b"), true);
    assert_eq!(song_ids(&favorites), ["b", "a", "c"]);
    server.release("unstar");
    settle(&favorites, cx);

    assert_eq!(song_ids(&favorites), ["a", "b", "c"]);
    assert_eq!(favorites.snapshot().is_favorite(&target("b")), Some(true));
}

#[test]
fn a_refused_removal_brings_back_the_favorite_listed_not_the_one_asked_about() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.refuse("unstar");
    server.refuse("getStarred2");
    let bare = Favorite::Song(serde_json::from_value(json!({ "id": "a", "title": "" })).unwrap());

    favorites.set_favorite(bare, false);
    settle(&favorites, cx);

    assert_eq!(favorites.snapshot().lists.songs[0].title(), "a");
}

#[test]
fn a_refused_change_is_followed_by_a_read() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.refuse_once("star");

    favorites.set_favorite(song("n"), true);
    settle(&favorites, cx);

    assert_eq!(server.reads(), 2);
}

#[test]
fn a_read_from_before_a_change_does_not_undo_it() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.hold("getStarred2");
    favorites.refresh();
    wait_until(cx, || server.reads() == 2);

    favorites.set_favorite(song("n"), true);
    wait_until(cx, || !is_writing(&favorites));
    server.release("getStarred2");
    settle(&favorites, cx);

    assert_eq!(song_ids(&favorites), ["n", "a"]);
    assert_eq!(server.reads(), 3);
}

#[test]
fn a_refresh_during_a_write_waits_for_it() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.hold("star");

    favorites.set_favorite(song("n"), true);
    favorites.refresh();
    cx.run_until_parked();
    assert_eq!(server.reads(), 1);

    server.release("star");
    wait_until(cx, || server.reads() == 2);
    settle(&favorites, cx);

    assert_eq!(song_ids(&favorites), ["n", "a"]);
}

#[test]
fn a_failed_first_load_says_so_and_leaves_the_favorites_unknown() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    server.refuse("getStarred2");
    let favorites = favorites_on(&server, cx);

    wait_until(cx, || {
        matches!(favorites.snapshot().load(), FavoritesLoad::Failed(_))
    });

    assert_eq!(favorites.snapshot().is_favorite(&target("a")), None);
    favorites.set_favorite(song("n"), true);
    cx.run_until_parked();
    assert!(server.writes().is_empty());
}

#[test]
fn only_loads_that_leave_no_favorites_are_told_as_failures() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    server.refuse("getStarred2");
    let favorites = favorites_on(&server, cx);
    let mut told = favorites.notices();
    let failures = |told: &mut broadcast::Receiver<FavoritesNotice>| {
        notices(told)
            .iter()
            .filter(|notice| matches!(notice, FavoritesNotice::LoadFailed(_)))
            .count()
    };
    wait_until(cx, || {
        matches!(favorites.snapshot().load(), FavoritesLoad::Failed(_))
    });
    assert_eq!(failures(&mut told), 1);

    favorites.reload_if_failed();
    wait_until(cx, || {
        server.reads() == 2 && !favorites.snapshot().is_reloading()
    });
    assert_eq!(failures(&mut told), 1);

    server.accept("getStarred2");
    favorites.reload_if_failed();
    settle(&favorites, cx);
    server.refuse("getStarred2");
    favorites.refresh();
    settle(&favorites, cx);

    assert_eq!(failures(&mut told), 0);
}

#[test]
fn favorites_that_loaded_are_not_read_again_for_having_failed() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);

    favorites.reload_if_failed();
    settle(&favorites, cx);

    assert_eq!(server.reads(), 1);
}

#[test]
fn a_failed_refresh_keeps_the_favorites_on_show() {
    let cx = &TestExecutor::new();
    let server = FavoritesServer::new(&["a"]);
    let favorites = favorites_on(&server, cx);
    settle(&favorites, cx);
    server.refuse("getStarred2");

    favorites.refresh();
    settle(&favorites, cx);

    assert_eq!(song_ids(&favorites), ["a"]);
    assert!(favorites.snapshot().refresh_error().is_some());
}
