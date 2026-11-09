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

fn finished<R>(mut task: Task<R>, cx: &mut TestAppContext) -> R {
    let mut output = None;
    wait_until(cx, || {
        output = (&mut task).now_or_never();
        output.is_some()
    });
    output.unwrap()
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

fn remove_entry_at(playlists: &PlaylistsState, ix: usize) -> Option<Removal> {
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
fn adding_songs_says_to_which_playlist(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    let mut notices = playlists.notices();

    playlists.add_songs(&PlaylistId::new("b"), &[song("9")]);
    playlists.add_songs(&PlaylistId::new("gone"), &[song("9")]);

    assert_eq!(notices.try_recv().unwrap().message(), "Added to Beta");
    assert_eq!(
        notices.try_recv().unwrap().message(),
        "Added to the playlist"
    );
    wait_for_saving(&playlists, cx);
}

#[gpui_kit::test]
fn renaming_shows_at_once_in_the_list_and_the_detail(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    playlists.rename(PlaylistId::new("a"), " Zeta ").unwrap();

    let snapshot = playlists.snapshot();
    assert_eq!(snapshot.open_playlist().unwrap().playlist().name(), "Zeta");
    assert_eq!(listed_names(&playlists), ["Beta", "Zeta"]);
    assert_eq!(&**snapshot.choices()[1].name(), "Zeta");
}

#[gpui_kit::test]
fn a_deleted_playlist_leaves_the_list_and_closes(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    playlists.delete(PlaylistId::new("a")).unwrap();

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

    let mut created = playlists.create("Road", &[]);
    wait_until(cx, || listed_names(&playlists).len() == 3);
    assert_eq!(listed_names(&playlists), ["alpha", "Beta", "Road"]);
    let created = (&mut created).now_or_never().unwrap().unwrap();
    assert_eq!(created.id(), &PlaylistId::new("new1"));
    assert_eq!(created.message(), "Created “Road”");

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

    let _created = playlists.create("Road", &[]);
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

    let mut created = playlists.create("Road", &[song("1")]);
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

#[gpui_kit::test]
fn a_new_playlist_says_what_it_starts_with(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    let mut created = playlists.create("Road", &[song("1"), song("2")]);
    wait_until(cx, || listed_names(&playlists).len() == 3);

    let created = (&mut created).now_or_never().unwrap().unwrap();
    assert_eq!(created.message(), "Created “Road” with 2 songs");
    let mut one = playlists.create("Trip", &[song("1")]);
    wait_until(cx, || listed_names(&playlists).len() == 4);
    let one = (&mut one).now_or_never().unwrap().unwrap();
    assert_eq!(one.message(), "Created “Trip” with 1 song");
}

#[test]
fn the_new_playlist_dialog_says_what_it_starts_with() {
    assert_eq!(new_playlist_scope(&[]), None);
    assert_eq!(
        new_playlist_scope(&[song("Intro")]).as_deref(),
        Some("Starts with “Intro”")
    );
    assert_eq!(
        new_playlist_scope(&[song("1"), song("2")]).as_deref(),
        Some("Starts with 2 songs")
    );
}

#[gpui_kit::test]
fn a_blank_name_is_refused(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    let mut created = playlists.create("  ", &[]);
    assert_eq!(
        (&mut created).now_or_never(),
        Some(Err("A playlist needs a name.".to_owned()))
    );
    assert_eq!(
        playlists.rename(PlaylistId::new("a"), " "),
        Err("A playlist needs a name.".to_owned())
    );

    pause(cx);
    assert!(server.requests("createPlaylist").is_empty());
    assert!(server.requests("updatePlaylist").is_empty());
    assert_eq!(listed_names(&playlists), ["alpha", "Beta"]);
}

#[gpui_kit::test]
fn a_new_name_is_saved_trimmed(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    playlists.rename(PlaylistId::new("a"), "  Zeta ").unwrap();
    let _created = playlists.create(" Road  ", &[]);

    wait_for_request(&server, "updatePlaylist", 1, cx);
    wait_for_request(&server, "createPlaylist", 1, cx);
    assert_eq!(
        server.requests("updatePlaylist")[0].values("name"),
        ["Zeta"]
    );
    assert_eq!(
        server.requests("createPlaylist")[0].values("name"),
        ["Road"]
    );
    wait_for_saving(&playlists, cx);
}

#[gpui_kit::test]
fn another_users_playlist_cant_be_renamed_or_deleted(cx: &mut TestAppContext) {
    let server = two_playlists();
    server.make_readonly("a");
    let playlists = loaded_from(&server, cx);
    let snapshot = playlists.snapshot();
    assert!(!snapshot.is_editable(&PlaylistId::new("a")));
    assert!(snapshot.is_editable(&PlaylistId::new("b")));
    assert_eq!(snapshot.delete_confirmation(&PlaylistId::new("a")), None);

    let refused = Err("This playlist is readonly.".to_owned());
    assert_eq!(playlists.rename(PlaylistId::new("a"), "Zeta"), refused);
    assert_eq!(playlists.delete(PlaylistId::new("a")), refused);

    pause(cx);
    assert_eq!(listed_names(&playlists), ["alpha", "Beta"]);
    assert!(server.requests("updatePlaylist").is_empty());
    assert!(server.requests("deletePlaylist").is_empty());
}

#[gpui_kit::test]
fn deleting_asks_with_the_playlists_name(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    let confirmation = playlists
        .snapshot()
        .delete_confirmation(&PlaylistId::new("b"))
        .unwrap();

    assert_eq!(confirmation.title(), "Delete “Beta”?");
    assert_eq!(
        confirmation.message(),
        "The songs stay in your library. This can't be undone."
    );
}

#[gpui_kit::test]
fn the_songs_of_a_playlist_that_diverged_cant_be_edited(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    assert!(playlists.snapshot().are_songs_editable());
    server.refuse("createPlaylist");
    server.hold("getPlaylist");

    move_entry_at(&playlists, 0, 1);
    wait_until(cx, || playlists.snapshot().is_diverged());

    let snapshot = playlists.snapshot();
    assert!(!snapshot.are_songs_editable());
    assert!(snapshot.is_editable(&PlaylistId::new("a")));
    server.release("getPlaylist");
    wait_for_saving(&playlists, cx);
    assert!(playlists.snapshot().are_songs_editable());
}

#[gpui_kit::test]
fn removing_a_song_says_which(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    let removal = remove_entry_at(&playlists, 1).unwrap();

    assert_eq!(removal.ix(), 1);
    assert_eq!(removal.song().id().as_str(), "2");
    assert_eq!(removal.message(), "Removed “2”");
    wait_for_saving(&playlists, cx);
}

#[gpui_kit::test]
fn the_query_shows_the_playlists_it_matches(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    assert_eq!(**playlists.snapshot().visible(), [0, 1]);

    playlists.set_query("bet");

    let snapshot = playlists.snapshot();
    assert_eq!(**snapshot.visible(), [1]);
    assert_eq!(snapshot.visible_playlist(0).unwrap().name(), "Beta");
}

#[gpui_kit::test]
fn the_query_applies_to_a_list_read_again(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    playlists.set_query("alp");

    let _created = playlists.create("Alpine", &[]);
    wait_until(cx, || listed_names(&playlists).len() == 3);

    let snapshot = playlists.snapshot();
    let shown: Vec<&str> = (0..snapshot.visible().len())
        .map(|position| snapshot.visible_playlist(position).unwrap().name())
        .collect();
    assert_eq!(shown, ["alpha", "Alpine"]);
}

#[gpui_kit::test]
fn the_selection_stays_on_a_playlist_still_listed_or_goes_to_the_first_shown(
    cx: &mut TestAppContext,
) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    let (a, b) = (PlaylistId::new("a"), PlaylistId::new("b"));
    playlists.set_query("beta");
    let snapshot = playlists.snapshot();

    assert_eq!(snapshot.kept_selection(Some(&a)), Some(&a));
    assert_eq!(
        snapshot.kept_selection(Some(&PlaylistId::new("gone"))),
        Some(&b)
    );
    assert_eq!(snapshot.kept_selection(None), Some(&b));
}

#[gpui_kit::test]
fn the_songs_of_the_open_playlist_are_those_on_show(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    move_entry_at(&playlists, 0, 2);
    let reads = server.requests("getPlaylist").len();

    let mut songs = playlists.songs_of(&PlaylistId::new("a"));

    let songs = (&mut songs).now_or_never().unwrap().unwrap();
    let ids: Vec<&str> = songs.iter().map(|song| song.id().as_str()).collect();
    assert_eq!(ids, ["2", "3", "1"]);
    assert_eq!(server.requests("getPlaylist").len(), reads);
    wait_for_saving(&playlists, cx);
}

#[gpui_kit::test]
fn the_songs_of_another_playlist_come_from_the_server(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);

    let songs = finished(playlists.songs_of(&PlaylistId::new("b")), cx).unwrap();

    let ids: Vec<&str> = songs.iter().map(|song| song.id().as_str()).collect();
    assert_eq!(ids, ["4"]);
}

#[gpui_kit::test]
fn the_songs_of_a_playlist_that_wont_load_say_why(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("getPlaylist");

    let songs = finished(playlists.songs_of(&PlaylistId::new("b")), cx);

    assert_eq!(
        songs,
        Err("Couldn't load this playlist. The server refused the request: Refused.".to_owned())
    );
}

#[gpui_kit::test]
fn a_new_playlist_already_in_a_list_read_since_shows_once(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    let listed = playlists.snapshot().loaded_list().unwrap()[1].clone();

    playlists
        .0
        .state
        .update(|state| playlists.playlist_created(state, Ok(&listed)));

    assert_eq!(listed_names(&playlists), ["alpha", "Beta"]);
}

#[gpui_kit::test]
fn a_rename_waiting_behind_a_failed_save_of_songs_still_goes(cx: &mut TestAppContext) {
    let server = two_playlists();
    let playlists = loaded_from(&server, cx);
    server.refuse("createPlaylist");
    server.hold("createPlaylist");
    move_entry_at(&playlists, 0, 1);
    wait_for_request(&server, "createPlaylist", 1, cx);

    playlists.rename(PlaylistId::new("a"), "Zeta").unwrap();
    server.release("createPlaylist");

    wait_for_request(&server, "updatePlaylist", 1, cx);
    assert_eq!(
        server.requests("updatePlaylist")[0].values("name"),
        ["Zeta"]
    );
    wait_for_saving(&playlists, cx);
}

fn open_playlist(id: &str, songs: &[&str]) -> OpenPlaylist {
    OpenPlaylist {
        playlist: serde_json::from_value(json!({ "id": id, "name": id })).unwrap(),
        entries: Entries::new(songs.iter().map(|id| song(id)).collect()),
    }
}

#[test]
fn a_selected_song_is_followed_to_its_place_or_its_first_copy() {
    let was = open_playlist("a", &["1", "2", "1"]);
    let mut now = was.clone();
    now.entries.replace(vec![song("2"), song("1"), song("1")]);
    let entry = |open: &OpenPlaylist, ix| open.entries.get(ix).unwrap().id();

    assert_eq!(now.follow(&was, entry(&was, 2)), Some(entry(&now, 2)));
    assert_eq!(now.follow(&was, entry(&was, 0)), Some(entry(&now, 1)));
    assert_eq!(now.follow(&now, entry(&now, 0)), Some(entry(&now, 0)));
}

#[test]
fn nothing_is_followed_into_another_playlist() {
    let was = open_playlist("a", &["1"]);
    let now = open_playlist("b", &["1"]);

    assert_eq!(now.follow(&was, was.entries.get(0).unwrap().id()), None);
}
