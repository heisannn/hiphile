use std::time::Duration;

use futures::FutureExt as _;
use gpui_kit::TestAppContext;
use serde_json::json;

use super::*;
use crate::test_support::{self, PlaylistsServer};

fn song(id: &str) -> Song {
    serde_json::from_value(json!({ "id": id, "title": id })).unwrap()
}

/// On GPUI's executor, which the test's `run_until_parked` drives.
fn playlists_on(server: &PlaylistsServer, cx: &mut TestAppContext) -> PlaylistsState {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    PlaylistsState::new(executor, test_support::session(server.url()))
}

/// `server`'s playlist `a` open, with nothing waiting.
fn loaded_from(server: &PlaylistsServer, cx: &mut TestAppContext) -> PlaylistsState {
    let playlists = playlists_on(server, cx);
    wait_until(cx, || playlists.snapshot().loaded_list().is_some());
    playlists.open(PlaylistId::new("a"));
    wait_until(cx, || playlists.snapshot().open_playlist().is_some());
    playlists
}

/// A server with `alpha` (a) holding songs 1, 2 and 3, and `Beta` (b) song 4.
fn two_playlists() -> PlaylistsServer {
    PlaylistsServer::new(&[("a", "alpha", &["1", "2", "3"]), ("b", "Beta", &["4"])])
}

fn wait_until(cx: &mut TestAppContext, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("it never happened");
}

/// Gives what is under way the time to arrive, for what must not happen.
fn pause(cx: &mut TestAppContext) {
    for _ in 0..10 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_request(server: &PlaylistsServer, method: &str, count: usize, cx: &mut TestAppContext) {
    wait_until(cx, || server.requests(method).len() >= count);
}

fn is_sending(playlists: &PlaylistsState) -> bool {
    playlists.0.state.read(|state| state.sending.is_some())
}

/// Waits for what was saved to be read back.
fn wait_for_saving(playlists: &PlaylistsState, cx: &mut TestAppContext) {
    wait_until(cx, || {
        let snapshot = playlists.snapshot();
        !is_sending(playlists) && !snapshot.is_diverged() && snapshot.open_playlist().is_some()
    });
    pause(cx);
}

fn is_loading(playlists: &PlaylistsState) -> bool {
    matches!(playlists.snapshot().detail(), PlaylistDetail::Loading)
}

fn listed_names(playlists: &PlaylistsState) -> Vec<String> {
    let snapshot = playlists.snapshot();
    snapshot
        .loaded_list()
        .map(|list| list.iter().map(|listed| listed.name().to_owned()).collect())
        .unwrap_or_default()
}

fn song_ids(playlists: &PlaylistsState) -> Vec<String> {
    let snapshot = playlists.snapshot();
    let open = snapshot.open_playlist().unwrap();
    let ids = open.entries().song_ids();
    ids.iter().map(|id| id.as_str().to_owned()).collect()
}

fn entry_at(playlists: &PlaylistsState, ix: usize) -> EntryId {
    let snapshot = playlists.snapshot();
    snapshot
        .open_playlist()
        .unwrap()
        .entries()
        .get(ix)
        .unwrap()
        .id()
}

fn move_entry_at(playlists: &PlaylistsState, ix: usize, to_ix: usize) -> bool {
    playlists.move_entry(entry_at(playlists, ix), to_ix)
}

fn remove_entry_at(playlists: &PlaylistsState, ix: usize) -> Option<(usize, Song)> {
    playlists.remove_entry(entry_at(playlists, ix))
}

#[gpui_kit::test]
fn the_list_is_sorted_by_name_ignoring_case(cx: &mut TestAppContext) {
    let server = PlaylistsServer::new(&[("b", "Beta", &[]), ("a", "alpha", &[])]);
    let playlists = playlists_on(&server, cx);

    wait_until(cx, || playlists.snapshot().loaded_list().is_some());

    assert_eq!(listed_names(&playlists), ["alpha", "Beta"]);
}

#[gpui_kit::test]
fn the_choices_are_the_playlists_the_user_may_edit(cx: &mut TestAppContext) {
    let server = PlaylistsServer::new(&[("b", "Beta", &[]), ("a", "alpha", &[]), ("c", "C", &[])]);
    server.make_readonly("c");
    let playlists = playlists_on(&server, cx);

    wait_until(cx, || playlists.snapshot().loaded_list().is_some());

    let snapshot = playlists.snapshot();
    let choices: Vec<_> = snapshot
        .choices()
        .iter()
        .map(|choice| (choice.id().as_str(), &**choice.name()))
        .collect();
    assert_eq!(choices, [("a", "alpha"), ("b", "Beta")]);
}

#[gpui_kit::test]
fn songs_added_to_the_playlist_on_show_appear_at_the_end(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    playlists.add_songs(&PlaylistId::new("a"), &[song("9")]);

    assert_eq!(song_ids(&playlists), ["1", "2", "3", "9"]);
    wait_for_saving(&playlists, cx);
    assert_eq!(server.songs("a"), ["1", "2", "3", "9"]);
}

#[gpui_kit::test]
fn songs_added_to_another_playlist_leave_the_one_on_show(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    playlists.add_songs(&PlaylistId::new("b"), &[song("9")]);

    assert_eq!(song_ids(&playlists), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn renaming_shows_at_once_in_the_list_and_the_detail(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    playlists.rename(PlaylistId::new("a"), "Zeta".to_owned());

    let snapshot = playlists.snapshot();
    assert_eq!(snapshot.open_playlist().unwrap().playlist().name(), "Zeta");
    assert_eq!(listed_names(&playlists), ["Beta", "Zeta"]);
    assert_eq!(&**snapshot.choices()[1].name(), "Zeta");
}

#[gpui_kit::test]
fn a_deleted_playlist_leaves_the_list_and_closes(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    playlists.delete(PlaylistId::new("a"));

    assert_eq!(listed_names(&playlists), ["Beta"]);
    assert_eq!(*playlists.snapshot().detail(), PlaylistDetail::None);
    wait_for_request(&server, "deletePlaylist", 1, cx);
}

#[gpui_kit::test]
fn a_readonly_playlist_takes_no_edits(cx: &mut TestAppContext) {
    let server = two_playlists();
    server.make_readonly("a");
    let playlists = loaded_from(&server, cx);

    assert!(!move_entry_at(&playlists, 0, 1));
    assert_eq!(remove_entry_at(&playlists, 0), None);

    assert_eq!(song_ids(&playlists), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn a_failed_save_stops_what_was_waiting_on_it(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("createPlaylist");

    move_entry_at(&playlists, 0, 1);
    move_entry_at(&playlists, 1, 2);
    assert!(is_sending(&playlists));

    wait_until(cx, || !is_sending(&playlists));

    playlists
        .0
        .state
        .update(|state| assert_eq!(state.writes.pop(), None));
    assert_eq!(server.requests("createPlaylist").len(), 1);
}

#[gpui_kit::test]
fn a_failed_save_says_what_failed_and_why(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("createPlaylist");
    let mut notices = playlists.notices();

    move_entry_at(&playlists, 0, 1);
    wait_until(cx, || !is_sending(&playlists));

    assert_eq!(
        notices.try_recv().unwrap(),
        PlaylistsNotice::WriteFailed(
            "Couldn't save the playlist's songs. The server refused the request: Refused."
                .to_owned()
        )
    );
}

#[gpui_kit::test]
fn edits_go_on_being_saved_after_a_failure(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("createPlaylist");
    move_entry_at(&playlists, 0, 1);
    wait_for_saving(&playlists, cx);
    assert_eq!(song_ids(&playlists), ["1", "2", "3"]);
    server.accept("createPlaylist");

    move_entry_at(&playlists, 0, 1);
    wait_for_saving(&playlists, cx);

    assert_eq!(server.songs("a"), ["2", "1", "3"]);
}

#[gpui_kit::test]
fn after_a_failed_save_songs_wait_to_be_read_again_before_more_edits(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("createPlaylist");
    server.hold("getPlaylist");
    move_entry_at(&playlists, 0, 1);
    wait_until(cx, || !is_sending(&playlists));
    let attempts = server.requests("createPlaylist").len();

    assert!(!move_entry_at(&playlists, 1, 2));
    assert_eq!(remove_entry_at(&playlists, 1), None);
    pause(cx);
    assert_eq!(server.requests("createPlaylist").len(), attempts);

    server.release("getPlaylist");
    wait_for_saving(&playlists, cx);
    assert_eq!(song_ids(&playlists), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn when_a_failed_save_cant_be_read_back_the_songs_leave_the_screen(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("createPlaylist");
    server.refuse("getPlaylist");

    move_entry_at(&playlists, 0, 1);

    wait_until(cx, || {
        matches!(playlists.snapshot().detail(), PlaylistDetail::Failed(_))
    });
}

#[gpui_kit::test]
fn a_reload_that_fails_leaves_songs_the_server_has_on_show(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("getPlaylist");

    playlists.refresh();
    wait_for_request(&server, "getPlaylist", 2, cx);
    pause(cx);

    assert_eq!(song_ids(&playlists), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn undo_goes_away_when_the_removal_fails(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("createPlaylist");

    remove_entry_at(&playlists, 1);
    wait_for_saving(&playlists, cx);
    assert_eq!(song_ids(&playlists), ["1", "2", "3"]);
    assert!(!playlists.snapshot().can_undo_removal());
    server.accept("createPlaylist");
    let attempts = server.requests("createPlaylist").len();

    assert_eq!(playlists.undo_removal(), None);
    pause(cx);

    assert_eq!(song_ids(&playlists), ["1", "2", "3"]);
    assert_eq!(server.requests("createPlaylist").len(), attempts);
}

#[gpui_kit::test]
fn undo_reaches_a_playlist_no_longer_on_show(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    remove_entry_at(&playlists, 1);
    wait_for_saving(&playlists, cx);
    assert_eq!(server.songs("a"), ["1", "3"]);
    playlists.open(PlaylistId::new("b"));
    wait_for_saving(&playlists, cx);

    playlists.undo_removal();
    wait_for_saving(&playlists, cx);

    assert_eq!(server.songs("a"), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn a_playlist_being_saved_waits_for_the_save_when_opened_again(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.hold("createPlaylist");
    remove_entry_at(&playlists, 1);
    wait_for_request(&server, "createPlaylist", 1, cx);
    playlists.open(PlaylistId::new("b"));
    wait_until(cx, || !is_loading(&playlists));

    let reads = server.requests("getPlaylist").len();
    playlists.open(PlaylistId::new("a"));
    wait_for_request(&server, "getPlaylist", reads + 1, cx);
    pause(cx);
    assert!(is_loading(&playlists));

    server.release("createPlaylist");
    wait_for_saving(&playlists, cx);
    assert_eq!(song_ids(&playlists), ["1", "3"]);
    assert_eq!(server.songs("a"), ["1", "3"]);
}

#[gpui_kit::test]
fn songs_added_from_elsewhere_are_waited_for_when_opening_their_playlist(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.hold("updatePlaylist");
    playlists.add_songs(&PlaylistId::new("b"), &[song("9")]);
    wait_for_request(&server, "updatePlaylist", 1, cx);

    let reads = server.requests("getPlaylist").len();
    playlists.open(PlaylistId::new("b"));
    wait_for_request(&server, "getPlaylist", reads + 1, cx);
    pause(cx);
    assert!(is_loading(&playlists));

    server.release("updatePlaylist");
    wait_for_saving(&playlists, cx);
    assert_eq!(song_ids(&playlists), ["4", "9"]);
}

#[gpui_kit::test]
fn another_playlist_shows_while_one_is_being_saved(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.hold("createPlaylist");
    move_entry_at(&playlists, 0, 1);
    wait_for_request(&server, "createPlaylist", 1, cx);

    playlists.open(PlaylistId::new("b"));

    wait_until(cx, || !is_loading(&playlists));
    assert_eq!(song_ids(&playlists), ["4"]);
    server.release("createPlaylist");
}

#[gpui_kit::test]
fn removing_the_only_song_takes_it_out_by_position(cx: &mut TestAppContext) {
    let server = PlaylistsServer::new(&[("a", "alpha", &["1"])]);
    let playlists = loaded_from(&server, cx);

    remove_entry_at(&playlists, 0);
    wait_for_saving(&playlists, cx);

    assert!(server.requests("createPlaylist").is_empty());
    let removal = server.requests("updatePlaylist");
    assert_eq!(removal[0].values("songIndexToRemove"), ["0"]);
    assert!(server.songs("a").is_empty());
    assert!(song_ids(&playlists).is_empty());
}

#[gpui_kit::test]
fn a_song_list_too_long_for_a_url_is_saved_by_post(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    let many: Vec<Song> = (0..500)
        .map(|ix| song(&format!("0123456789abcdef0123456789abcd{ix:03}")))
        .collect();

    playlists.add_songs(&PlaylistId::new("a"), &many);
    wait_for_saving(&playlists, cx);

    let saved = &server.requests("updatePlaylist")[0];
    assert_eq!(saved.verb, "POST");
    assert_eq!(saved.values("songIdToAdd").len(), 500);
    assert_eq!(server.songs("a").len(), 503);
}

#[gpui_kit::test]
fn a_new_playlist_outlives_a_list_fetched_before_it(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.hold("getPlaylists");
    let reads = server.requests("getPlaylists").len();
    playlists.refresh();
    wait_for_request(&server, "getPlaylists", reads + 1, cx);

    let mut created = playlists.create("Road".to_owned(), &[]);
    wait_until(cx, || listed_names(&playlists).len() == 3);
    assert_eq!(listed_names(&playlists), ["alpha", "Beta", "Road"]);
    assert_eq!(
        (&mut created).now_or_never(),
        Some(Ok(PlaylistId::new("new1")))
    );

    server.release("getPlaylists");
    pause(cx);
    assert_eq!(listed_names(&playlists), ["alpha", "Beta", "Road"]);
}

#[gpui_kit::test]
fn a_new_playlist_doesnt_stand_in_for_a_list_that_failed(cx: &mut TestAppContext) {
    let server = two_playlists();
    server.refuse("getPlaylists");
    let playlists = playlists_on(&server, cx);
    wait_until(cx, || {
        matches!(playlists.snapshot().list(), PlaylistList::Failed(_))
    });

    let _created = playlists.create("Road".to_owned(), &[]);
    wait_for_request(&server, "createPlaylist", 1, cx);
    pause(cx);
    assert!(listed_names(&playlists).is_empty());

    server.accept("getPlaylists");
    playlists.refresh();
    wait_until(cx, || !listed_names(&playlists).is_empty());
    assert_eq!(listed_names(&playlists), ["alpha", "Beta", "Road"]);
}

#[gpui_kit::test]
fn a_playlist_the_server_wont_create_says_why(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("createPlaylist");

    let mut created = playlists.create("Road".to_owned(), &[song("1")]);
    wait_for_request(&server, "createPlaylist", 1, cx);
    pause(cx);

    assert_eq!(
        (&mut created).now_or_never(),
        Some(Err("The server refused the request: Refused.".to_owned()))
    );
    assert_eq!(listed_names(&playlists), ["alpha", "Beta"]);
}

#[gpui_kit::test]
fn songs_the_server_changed_are_new_entries(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    let first = entry_at(&playlists, 0);
    server.set_songs("a", &["2", "1"]);

    playlists.refresh();
    wait_until(cx, || song_ids(&playlists) == ["2", "1"]);

    let snapshot = playlists.snapshot();
    assert_eq!(
        snapshot.open_playlist().unwrap().entries().position(first),
        None
    );
}
