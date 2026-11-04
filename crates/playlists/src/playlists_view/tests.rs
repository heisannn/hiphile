use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{KeyDownEvent, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext};
use serde_json::json;
use subsonic::Credentials;

use super::*;
use crate::fake_server::FakeServer;

fn playlist(id: &str, name: &str) -> Playlist {
    serde_json::from_value(json!({ "id": id, "name": name, "owner": "alice" })).unwrap()
}

fn song(id: &str) -> Song {
    serde_json::from_value(json!({ "id": id, "title": id })).unwrap()
}

/// The playlists feature, whose server cannot be reached, with one
/// playlist of three songs on show.
fn playlists(cx: &mut TestAppContext) -> (Entity<Playlists>, &mut VisualTestContext) {
    let (view, cx) = view_on("http://127.0.0.1:9", cx);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.set_list(
                vec![playlist("b", "Beta"), playlist("a", "alpha")],
                window,
                cx,
            );
            this.selected = Some(PlaylistId::new("a"));
            this.detail = Detail::Loaded(LoadedPlaylist {
                playlist: playlist("a", "alpha"),
                entries: Entries::new(vec![song("1"), song("2"), song("3")]),
                cover_url: None,
            });
        });
    });
    (view, cx)
}

/// The feature on `url`, signed in as the owner of its playlists.
fn view_on<'a>(
    url: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<Playlists>, &'a mut VisualTestContext) {
    // The requests wake the test from the network's own thread.
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    cx.update(crate::init);
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new(url, Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    let session = session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        None,
        None,
    );
    // Notifications and dialogs live in the window's root.
    let (handle, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Playlists::new(session, window, cx))
            })
        })
        .unwrap();
    (view, VisualTestContext::from_window(handle, cx).into_mut())
}

/// `server`'s first playlist on show, with nothing waiting.
fn loaded_from<'a>(
    server: &FakeServer,
    cx: &'a mut TestAppContext,
) -> (Entity<Playlists>, &'a mut VisualTestContext) {
    let (view, cx) = view_on(server.url(), cx);
    wait_until(cx, |cx| {
        cx.update(|_, cx| view.read(cx).loaded_detail().is_some())
    });
    (view, cx)
}

/// A server with `alpha` (a) holding songs 1, 2 and 3, and `Beta` (b) song 4.
fn two_playlists() -> FakeServer {
    FakeServer::new(&[("a", "alpha", &["1", "2", "3"]), ("b", "Beta", &["4"])])
}

fn wait_until(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("it never happened");
}

/// Gives what is under way the time to arrive, for what must not happen.
fn pause(cx: &mut VisualTestContext) {
    for _ in 0..10 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_request(server: &FakeServer, method: &str, count: usize, cx: &mut VisualTestContext) {
    wait_until(cx, |_| server.requests(method).len() >= count);
}

/// Waits for what was saved to be read back.
fn wait_for_saving(view: &Entity<Playlists>, cx: &mut VisualTestContext) {
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            let this = view.read(cx);
            this.sending.is_none() && !this.diverged && this.loaded_detail().is_some()
        })
    });
    pause(cx);
}

fn select_playlist(view: &Entity<Playlists>, id: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.select(PlaylistId::new(id), window, cx));
    });
}

fn is_loading(view: &Entity<Playlists>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| matches!(view.read(cx).detail, Detail::Loading))
}

fn has_dialog(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| window.has_active_dialog(cx))
}

fn listed_names(view: &Entity<Playlists>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        let this = view.read(cx);
        this.loaded_list()
            .map(|list| list.iter().map(|listed| listed.name().to_owned()).collect())
            .unwrap_or_default()
    })
}

fn focus_songs(view: &Entity<Playlists>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.songs_focus.focus(window, cx));
    });
    cx.run_until_parked();
}

fn songs_are_focused(view: &Entity<Playlists>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| view.read(cx).songs_focus.is_focused(window))
}

/// Opens the New Playlist dialog and commits `name` from the keyboard.
fn create_from_dialog(
    view: &Entity<Playlists>,
    name: &str,
    cx: &mut VisualTestContext,
) -> Entity<NameForm> {
    let form = cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            let form = this.new_playlist_form(Arc::from([]), window, cx);
            open_name_dialog("New Playlist", form.clone(), window, cx);
            form
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        form.update(cx, |form, cx| form.set_name(name, window, cx));
    });
    cx.simulate_keystrokes("enter");
    form
}

/// Counts how many times the app is asked to play songs.
fn count_plays(cx: &mut VisualTestContext) -> Rc<Cell<usize>> {
    let plays = Rc::new(Cell::new(0));
    let counted = plays.clone();
    cx.update(|_, cx| {
        cx.on_action(move |_: &PlaySongs, _| counted.set(counted.get() + 1));
    });
    plays
}

fn song_ids(view: &Entity<Playlists>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        view.read(cx)
            .loaded_detail()
            .unwrap()
            .entries
            .song_ids()
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect()
    })
}

fn select_ix(view: &Entity<Playlists>, ix: usize, cx: &mut VisualTestContext) {
    cx.update(|_, cx| view.update(cx, |this, cx| this.select_entry_at(ix, cx)));
}

#[gpui_kit::test]
fn the_list_is_sorted_by_name_ignoring_case(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    cx.update(|_, cx| {
        let this = view.read(cx);
        let names: Vec<_> = this
            .loaded_list()
            .unwrap()
            .iter()
            .map(Playlist::name)
            .collect();
        assert_eq!(names, ["alpha", "Beta"]);
    });
}

#[gpui_kit::test]
fn moving_a_song_shows_at_once_and_keeps_it_selected(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    select_ix(&view, 0, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(1, window, cx));
    });

    assert_eq!(song_ids(&view, cx), ["2", "1", "3"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(1)));
}

#[gpui_kit::test]
fn a_song_at_the_top_stays_there_when_moved_up(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    select_ix(&view, 0, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(-1, window, cx));
    });

    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn removing_a_song_selects_the_next_and_undo_puts_it_back(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    select_ix(&view, 1, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.remove_selected_entry(window, cx));
    });

    assert_eq!(song_ids(&view, cx), ["1", "3"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(1)));

    cx.update(|window, cx| view.update(cx, |this, cx| this.undo_removal(window, cx)));

    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(1)));
}

#[gpui_kit::test]
fn removing_the_last_song_selects_the_one_before(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    select_ix(&view, 2, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.remove_selected_entry(window, cx));
    });

    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(1)));
}

#[gpui_kit::test]
fn songs_added_to_the_playlist_on_show_appear_at_the_end(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.add_songs(&PlaylistId::new("a"), &[song("9")], window, cx);
        });
    });

    assert_eq!(song_ids(&view, cx), ["1", "2", "3", "9"]);
}

#[gpui_kit::test]
fn songs_added_to_another_playlist_leave_the_one_on_show(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.add_songs(&PlaylistId::new("b"), &[song("9")], window, cx);
        });
    });

    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn renaming_shows_at_once_in_the_list_and_the_detail(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.rename_playlist(PlaylistId::new("a"), "Zeta".to_owned(), window, cx);
        });
    });

    cx.update(|_, cx| {
        let this = view.read(cx);
        assert_eq!(this.loaded_detail().unwrap().playlist.name(), "Zeta");
        let names: Vec<_> = this
            .loaded_list()
            .unwrap()
            .iter()
            .map(Playlist::name)
            .collect();
        assert_eq!(names, ["Beta", "Zeta"]);
    });
}

#[gpui_kit::test]
fn deleting_the_selected_playlist_selects_the_one_in_its_place(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.delete_playlist(PlaylistId::new("a"), window, cx);
        });
    });

    cx.update(|_, cx| {
        let this = view.read(cx);
        assert_eq!(this.selected, Some(PlaylistId::new("b")));
        assert_eq!(this.loaded_list().unwrap().len(), 1);
    });
}

#[gpui_kit::test]
fn a_readonly_playlist_takes_no_edits(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    cx.update(|_, cx| {
        view.update(cx, |this, _| {
            let shared = serde_json::from_value(json!({
                "id": "a", "name": "alpha", "owner": "bob", "readonly": true,
            }))
            .unwrap();
            this.loaded_detail_mut().unwrap().playlist = shared;
        });
    });
    select_ix(&view, 0, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.move_selected_entry(1, window, cx);
            this.remove_selected_entry(window, cx);
        });
    });

    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
}

/// Waits for the unreachable server to fail the write under way.
fn finish_writing(view: &Entity<Playlists>, cx: &mut VisualTestContext) {
    for _ in 0..500 {
        cx.run_until_parked();
        if !cx.update(|_, cx| view.read(cx).sending.is_some()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the write never finished");
}

#[gpui_kit::test]
fn a_failed_save_stops_what_was_waiting_on_it(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    select_ix(&view, 0, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.move_selected_entry(1, window, cx);
            this.move_selected_entry(1, window, cx);
        });
    });
    cx.update(|_, cx| assert!(view.read(cx).sending.is_some()));

    finish_writing(&view, cx);

    cx.update(|_, cx| {
        view.update(cx, |this, _| assert_eq!(this.writes.pop(), None));
    });
}

#[gpui_kit::test]
fn edits_go_on_being_saved_after_a_failure(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);
    server.refuse("createPlaylist");
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(1, window, cx));
    });
    wait_for_saving(&view, cx);
    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
    server.accept("createPlaylist");

    select_ix(&view, 0, cx);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(1, window, cx));
    });
    wait_for_saving(&view, cx);

    assert_eq!(server.songs("a"), ["2", "1", "3"]);
}

#[gpui_kit::test]
fn after_a_failed_save_songs_wait_to_be_read_again_before_more_edits(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);
    server.refuse("createPlaylist");
    server.hold("getPlaylist");
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(1, window, cx));
    });
    wait_until(cx, |cx| cx.update(|_, cx| view.read(cx).sending.is_none()));
    let attempts = server.requests("createPlaylist").len();

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.move_selected_entry(1, window, cx);
            this.remove_selected_entry(window, cx);
        });
    });
    pause(cx);
    assert_eq!(server.requests("createPlaylist").len(), attempts);

    server.release("getPlaylist");
    wait_for_saving(&view, cx);
    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn when_a_failed_save_cant_be_read_back_the_songs_leave_the_screen(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);
    server.refuse("createPlaylist");
    server.refuse("getPlaylist");

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(1, window, cx));
    });

    wait_until(cx, |cx| {
        cx.update(|_, cx| matches!(view.read(cx).detail, Detail::Failed(_)))
    });
}

#[gpui_kit::test]
fn a_reload_that_fails_leaves_songs_the_server_has_on_show(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    server.refuse("getPlaylist");

    cx.update(|window, cx| view.update(cx, |this, cx| this.refresh(window, cx)));
    wait_for_request(&server, "getPlaylist", 2, cx);
    pause(cx);

    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn undo_goes_away_when_the_removal_fails(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 1, cx);
    server.refuse("createPlaylist");

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.remove_selected_entry(window, cx));
    });
    wait_for_saving(&view, cx);
    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
    server.accept("createPlaylist");
    let attempts = server.requests("createPlaylist").len();

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.undo_removal(window, cx));
    });
    pause(cx);

    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
    assert_eq!(server.requests("createPlaylist").len(), attempts);
}

#[gpui_kit::test]
fn undo_reaches_a_playlist_no_longer_on_show(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 1, cx);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.remove_selected_entry(window, cx));
    });
    wait_for_saving(&view, cx);
    assert_eq!(server.songs("a"), ["1", "3"]);
    select_playlist(&view, "b", cx);
    wait_for_saving(&view, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.undo_removal(window, cx));
    });
    wait_for_saving(&view, cx);

    assert_eq!(server.songs("a"), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn a_playlist_being_saved_waits_for_the_save_when_selected_again(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 1, cx);
    server.hold("createPlaylist");
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.remove_selected_entry(window, cx));
    });
    wait_for_request(&server, "createPlaylist", 1, cx);
    select_playlist(&view, "b", cx);
    wait_until(cx, |cx| !is_loading(&view, cx));

    let reads = server.requests("getPlaylist").len();
    select_playlist(&view, "a", cx);
    wait_for_request(&server, "getPlaylist", reads + 1, cx);
    pause(cx);
    assert!(is_loading(&view, cx));

    server.release("createPlaylist");
    wait_for_saving(&view, cx);
    assert_eq!(song_ids(&view, cx), ["1", "3"]);
    assert_eq!(server.songs("a"), ["1", "3"]);
}

#[gpui_kit::test]
fn songs_added_from_elsewhere_are_waited_for_when_opening_their_playlist(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    server.hold("updatePlaylist");
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.add_songs(&PlaylistId::new("b"), &[song("9")], window, cx);
        });
    });
    wait_for_request(&server, "updatePlaylist", 1, cx);

    let reads = server.requests("getPlaylist").len();
    select_playlist(&view, "b", cx);
    wait_for_request(&server, "getPlaylist", reads + 1, cx);
    pause(cx);
    assert!(is_loading(&view, cx));

    server.release("updatePlaylist");
    wait_for_saving(&view, cx);
    assert_eq!(song_ids(&view, cx), ["4", "9"]);
}

#[gpui_kit::test]
fn another_playlist_shows_while_one_is_being_saved(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);
    server.hold("createPlaylist");
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(1, window, cx));
    });
    wait_for_request(&server, "createPlaylist", 1, cx);

    select_playlist(&view, "b", cx);

    wait_until(cx, |cx| !is_loading(&view, cx));
    assert_eq!(song_ids(&view, cx), ["4"]);
    server.release("createPlaylist");
}

#[gpui_kit::test]
fn removing_the_only_song_takes_it_out_by_position(cx: &mut TestAppContext) {
    let server = FakeServer::new(&[("a", "alpha", &["1"])]);
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.remove_selected_entry(window, cx));
    });
    wait_for_saving(&view, cx);

    assert!(server.requests("createPlaylist").is_empty());
    let removal = server.requests("updatePlaylist");
    assert_eq!(removal[0].values("songIndexToRemove"), ["0"]);
    assert!(server.songs("a").is_empty());
    assert!(song_ids(&view, cx).is_empty());
}

#[gpui_kit::test]
fn a_song_list_too_long_for_a_url_is_saved_by_post(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    let many: Vec<Song> = (0..500)
        .map(|ix| song(&format!("0123456789abcdef0123456789abcd{ix:03}")))
        .collect();

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.add_songs(&PlaylistId::new("a"), &many, window, cx);
        });
    });
    wait_for_saving(&view, cx);

    let saved = &server.requests("updatePlaylist")[0];
    assert_eq!(saved.verb, "POST");
    assert_eq!(saved.values("songIdToAdd").len(), 500);
    assert_eq!(server.songs("a").len(), 503);
}

#[gpui_kit::test]
fn a_new_playlist_outlives_a_list_fetched_before_it(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    server.hold("getPlaylists");
    let reads = server.requests("getPlaylists").len();
    cx.update(|window, cx| view.update(cx, |this, cx| this.refresh(window, cx)));
    wait_for_request(&server, "getPlaylists", reads + 1, cx);

    create_from_dialog(&view, "Road", cx);
    wait_until(cx, |cx| !has_dialog(cx));
    assert_eq!(listed_names(&view, cx), ["alpha", "Beta", "Road"]);

    server.release("getPlaylists");
    pause(cx);
    assert_eq!(listed_names(&view, cx), ["alpha", "Beta", "Road"]);
}

#[gpui_kit::test]
fn a_new_playlist_doesnt_stand_in_for_a_list_that_failed(cx: &mut TestAppContext) {
    let server = two_playlists();
    server.refuse("getPlaylists");
    let (view, cx) = view_on(server.url(), cx);
    wait_until(cx, |cx| {
        cx.update(|_, cx| matches!(view.read(cx).list, List::Failed(_)))
    });

    create_from_dialog(&view, "Road", cx);
    wait_until(cx, |cx| !has_dialog(cx));
    pause(cx);
    assert!(listed_names(&view, cx).is_empty());

    server.accept("getPlaylists");
    cx.update(|window, cx| view.update(cx, |this, cx| this.refresh(window, cx)));
    wait_until(cx, |cx| !listed_names(&view, cx).is_empty());
    assert_eq!(listed_names(&view, cx), ["alpha", "Beta", "Road"]);
}

#[gpui_kit::test]
fn the_new_playlist_dialog_stays_until_its_save_is_done(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    server.hold("createPlaylist");
    let form = create_from_dialog(&view, "Road", cx);
    wait_for_request(&server, "createPlaylist", 1, cx);
    cx.update(|_, cx| assert!(form.read(cx).is_submitting()));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(has_dialog(cx));

    server.release("createPlaylist");
    wait_until(cx, |cx| !has_dialog(cx));
}

#[gpui_kit::test]
fn escape_cancels_the_new_playlist_dialog_before_committing(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_new_playlist(Arc::from([]), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(has_dialog(cx));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    assert!(!has_dialog(cx));
}

#[gpui_kit::test]
fn a_second_new_playlist_dialog_doesnt_open_over_the_first(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    let open = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.open_new_playlist(Arc::from([]), window, cx);
            });
        });
        cx.run_until_parked();
    };
    open(cx);
    open(cx);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    assert!(!has_dialog(cx));
}

#[gpui_kit::test]
fn a_playlist_still_loading_can_be_renamed_and_deleted(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    server.hold("getPlaylist");
    select_playlist(&view, "b", cx);
    assert!(is_loading(&view, cx));

    cx.update(|window, cx| view.update(cx, |this, cx| this.open_rename(window, cx)));
    cx.run_until_parked();
    assert!(has_dialog(cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!has_dialog(cx));

    cx.update(|window, cx| view.update(cx, |this, cx| this.open_delete(window, cx)));
    cx.run_until_parked();
    assert!(has_dialog(cx));
    server.release("getPlaylist");
}

#[gpui_kit::test]
fn a_playlist_that_wont_load_can_be_deleted(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    server.refuse("getPlaylist");
    select_playlist(&view, "b", cx);
    wait_until(cx, |cx| {
        cx.update(|_, cx| matches!(view.read(cx).detail, Detail::Failed(_)))
    });

    cx.update(|window, cx| view.update(cx, |this, cx| this.open_delete(window, cx)));
    cx.run_until_parked();

    assert!(has_dialog(cx));
}

#[gpui_kit::test]
fn choosing_new_playlist_from_the_chooser_leaves_keys_with_the_dialog(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    select_ix(&view, 0, cx);
    focus_songs(&view, cx);
    let songs: Arc<[Song]> = Arc::from([song("9")]);
    let menu = cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_chooser(songs.clone(), window, cx);
            this.chooser.as_ref().unwrap().menu.clone()
        })
    });

    // The menu's New Playlist… asks for the dialog, then dismisses itself.
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.open_new_playlist(songs, window, cx));
        menu.update(cx, |_, cx| cx.emit(DismissEvent));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("backspace");
    cx.run_until_parked();

    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
    assert!(has_dialog(cx));
    cx.update(|window, cx| window.close_dialog(cx));
    cx.run_until_parked();
    assert!(songs_are_focused(&view, cx));
}

#[gpui_kit::test]
fn dismissing_the_chooser_returns_focus(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    focus_songs(&view, cx);
    let menu = cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_chooser(Arc::from([song("9")]), window, cx);
            this.chooser.as_ref().unwrap().menu.clone()
        })
    });
    assert!(!songs_are_focused(&view, cx));

    cx.update(|_, cx| menu.update(cx, |_, cx| cx.emit(DismissEvent)));
    cx.run_until_parked();

    assert!(songs_are_focused(&view, cx));
    cx.update(|_, cx| assert!(view.read(cx).chooser.is_none()));
}

#[gpui_kit::test]
fn dismissing_the_chooser_leaves_focus_that_moved_on(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    focus_songs(&view, cx);
    let menu = cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_chooser(Arc::from([song("9")]), window, cx);
            this.chooser.as_ref().unwrap().menu.clone()
        })
    });
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.list_focus.focus(window, cx));
    });

    cx.update(|_, cx| menu.update(cx, |_, cx| cx.emit(DismissEvent)));
    cx.run_until_parked();

    assert!(!songs_are_focused(&view, cx));
}

#[gpui_kit::test]
fn enter_on_the_play_button_plays(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    let plays = count_plays(cx);
    focus_songs(&view, cx);

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    // A button goes off when the key comes back up.
    let enter = Keystroke::parse("enter").unwrap();
    cx.simulate_event(KeyDownEvent {
        keystroke: enter.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke: enter });
    cx.run_until_parked();

    assert_eq!(plays.get(), 1);
}

#[gpui_kit::test]
fn songs_answer_to_their_keys(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);
    focus_songs(&view, cx);

    cx.simulate_keystrokes("down");
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(1)));
    cx.simulate_keystrokes("alt-up");
    assert_eq!(song_ids(&view, cx), ["2", "1", "3"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(0)));
    cx.simulate_keystrokes("backspace");
    assert_eq!(song_ids(&view, cx), ["1", "3"]);

    wait_for_saving(&view, cx);
    assert_eq!(server.songs("a"), ["1", "3"]);
}

#[gpui_kit::test]
fn a_command_given_while_songs_load_runs_when_they_arrive(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    let plays = count_plays(cx);
    server.hold("getPlaylist");
    select_playlist(&view, "b", cx);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.on_playlist(SongsCommand::Play, window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(plays.get(), 0);

    server.release("getPlaylist");
    wait_until(cx, |_| plays.get() == 1);
}

#[gpui_kit::test]
fn the_selected_song_is_followed_when_the_server_moves_it(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 1, cx);
    let moved: PlaylistWithSongs = serde_json::from_value(json!({
        "id": "a", "name": "alpha", "owner": "alice",
        "entry": [{ "id": "2", "title": "2" }, { "id": "1", "title": "1" }],
    }))
    .unwrap();

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.show_detail(moved, window, cx));
    });

    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(0)));
}

#[gpui_kit::test]
fn a_failure_to_create_a_playlist_shows_in_its_dialog(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    let form = cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            let form = this.new_playlist_form(Arc::from([]), window, cx);
            open_name_dialog("New Playlist", form.clone(), window, cx);
            form
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        form.update(cx, |form, cx| form.set_name("Road Trip", window, cx));
    });

    cx.simulate_keystrokes("enter");

    for _ in 0..500 {
        cx.run_until_parked();
        if !cx.update(|_, cx| form.read(cx).is_submitting()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.update(|window, cx| {
        assert!(form.read(cx).error().is_some());
        assert!(window.has_active_dialog(cx));
    });
}

#[gpui_kit::test]
fn enter_in_the_rename_dialog_closes_it_and_saves(cx: &mut TestAppContext) {
    let (view, cx) = playlists(cx);
    cx.update(|window, cx| view.update(cx, |this, cx| this.open_rename(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));

    cx.simulate_keystrokes("enter");

    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    finish_writing(&view, cx);
}
