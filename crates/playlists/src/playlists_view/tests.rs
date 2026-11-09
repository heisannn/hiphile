use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    DismissEvent, KeyDownEvent, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext,
};
use gpui_kit::{Modifiers, MouseButton, Pixels, Point, point};
use hiphile_core::test_support::{self, PlaylistsServer};
use serde_json::json;

use super::dialogs::open_name_dialog;
use super::*;
use crate::name_form::NameForm;

fn song(id: &str) -> Song {
    serde_json::from_value(json!({ "id": id, "title": id })).unwrap()
}

/// The feature on `server`, signed in as the owner of its playlists.
fn view_on<'a>(
    server: &PlaylistsServer,
    cx: &'a mut TestAppContext,
) -> (Entity<Playlists>, &'a mut VisualTestContext) {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    cx.update(ui::list_navigation::bind_control_keys);
    cx.update(crate::init);
    let session = test_support::session(server.url());
    let playlists = PlaylistsState::new(cx.update(|cx| gpui_host::executor(cx)), session.clone());
    // Notifications and dialogs live in the window's root
    let (handle, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Playlists::new(session, playlists, window, cx))
            })
        })
        .unwrap();
    (view, VisualTestContext::from_window(handle, cx).into_mut())
}

/// `server`'s first playlist on show.
fn loaded_from<'a>(
    server: &PlaylistsServer,
    cx: &'a mut TestAppContext,
) -> (Entity<Playlists>, &'a mut VisualTestContext) {
    let (view, cx) = view_on(server, cx);
    wait_until(cx, |cx| {
        cx.update(|_, cx| view.read(cx).loaded_detail().is_some())
    });
    (view, cx)
}

/// A server with `alpha` (a) holding songs 1, 2 and 3, and `Beta` (b) song 4.
fn two_playlists() -> PlaylistsServer {
    PlaylistsServer::new(&[("a", "alpha", &["1", "2", "3"]), ("b", "Beta", &["4"])])
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

fn wait_for_request(
    server: &PlaylistsServer,
    method: &str,
    count: usize,
    cx: &mut VisualTestContext,
) {
    wait_until(cx, |_| server.requests(method).len() >= count);
}

fn select_playlist(view: &Entity<Playlists>, id: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.select(PlaylistId::new(id), window, cx));
    });
}

fn is_loading(view: &Entity<Playlists>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| matches!(view.read(cx).shown.detail(), PlaylistDetail::Loading))
}

fn has_dialog(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| window.has_active_dialog(cx))
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
            .entries()
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
fn the_favorite_key_follows_the_song_selected(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 1, cx);
    focus_songs(&view, cx);
    let asked = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        actions::set_known_favorites(Some(Default::default()), cx);
        let asked = asked.clone();
        cx.on_action(move |action: &actions::ToggleFavoriteOf, _| {
            asked.borrow_mut().push(action.favorite().target());
        });
    });

    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-d"
    } else {
        "ctrl-d"
    });

    assert_eq!(
        *asked.borrow(),
        [subsonic::StarTarget::Song(SongId::new("2"))]
    );
}

#[gpui_kit::test]
fn moving_a_song_shows_at_once_and_keeps_it_selected(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(1, window, cx));
    });

    assert_eq!(song_ids(&view, cx), ["2", "1", "3"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(1)));
}

#[gpui_kit::test]
fn a_song_at_the_top_stays_there_when_moved_up(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.move_selected_entry(-1, window, cx));
    });

    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn removing_a_song_selects_the_next_and_undo_puts_it_back(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
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
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 2, cx);

    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.remove_selected_entry(window, cx));
    });

    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(1)));
}

#[gpui_kit::test]
fn deleting_the_selected_playlist_selects_the_one_in_its_place(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);

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
fn deleting_the_open_playlist_of_a_list_that_failed_leaves_nothing_selected(
    cx: &mut TestAppContext,
) {
    let server = two_playlists();
    server.refuse("getPlaylists");
    let (view, cx) = view_on(&server, cx);
    wait_until(cx, |cx| {
        cx.update(|_, cx| matches!(view.read(cx).shown.list(), PlaylistList::Failed(_)))
    });
    select_playlist(&view, "a", cx);
    wait_until(cx, |cx| {
        cx.update(|_, cx| view.read(cx).loaded_detail().is_some())
    });

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.delete_playlist(PlaylistId::new("a"), window, cx);
        });
    });

    cx.update(|_, cx| {
        let this = view.read(cx);
        assert_eq!(this.selected, None);
        assert_eq!(*this.shown.detail(), PlaylistDetail::None);
    });
    wait_for_request(&server, "deletePlaylist", 1, cx);
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
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
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
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
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
        cx.update(|_, cx| matches!(view.read(cx).shown.detail(), PlaylistDetail::Failed(_)))
    });

    cx.update(|window, cx| view.update(cx, |this, cx| this.open_delete(window, cx)));
    cx.run_until_parked();

    assert!(has_dialog(cx));
}

#[gpui_kit::test]
fn choosing_new_playlist_from_the_chooser_leaves_keys_with_the_dialog(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    select_ix(&view, 0, cx);
    focus_songs(&view, cx);
    let songs: Arc<[Song]> = Arc::from([song("9")]);
    let menu = cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_chooser(songs.clone(), window, cx);
            this.chooser.as_ref().unwrap().menu.clone()
        })
    });

    // The menu's New Playlist… asks for the dialog, then dismisses itself
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
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
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
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
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
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    let plays = count_plays(cx);
    focus_songs(&view, cx);

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    // A button goes off when the key comes back up
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

    wait_until(cx, |_| server.songs("a") == ["1", "3"]);
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
            this.on_playlist(AlbumAction::Play, window, cx);
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
    server.set_songs("a", &["2", "1"]);

    cx.update(|_, cx| view.read(cx).playlists.refresh());
    wait_until(cx, |cx| song_ids(&view, cx) == ["2", "1"]);

    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(0)));
}

#[gpui_kit::test]
fn a_failure_to_create_a_playlist_shows_in_its_dialog(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    server.refuse("createPlaylist");
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
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    cx.update(|window, cx| view.update(cx, |this, cx| this.open_rename(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));

    cx.simulate_keystrokes("enter");

    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    wait_for_request(&server, "updatePlaylist", 1, cx);
    assert_eq!(
        server.requests("updatePlaylist")[0].values("name"),
        ["alpha"]
    );
}

/// The middle of the row at `ix` in the song list, as last drawn.
fn song_row(view: &Entity<Playlists>, ix: usize, cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.update(|_, cx| {
        let this = view.read(cx);
        let len = this.loaded_detail().unwrap().entries().len();
        let state = this.songs_scroll.0.borrow();
        let list = state.base_handle.bounds();
        let row = state.last_item_size.unwrap().contents.height / len as f32;
        point(
            list.origin.x + list.size.width / 2.,
            list.origin.y + row * (ix as f32 + 0.5),
        )
    })
}

/// Presses on the row at `from` and takes the pointer to the row at `to`.
fn drag_toward(from: usize, to: usize, view: &Entity<Playlists>, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    let (from, to) = (song_row(view, from, cx), song_row(view, to, cx));
    let none = Modifiers::default();
    cx.simulate_mouse_move(from, None, none);
    cx.simulate_mouse_down(from, MouseButton::Left, none);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
}

fn drop_on(ix: usize, view: &Entity<Playlists>, cx: &mut VisualTestContext) {
    let to = song_row(view, ix, cx);
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_song_dragged_down_lands_after_the_row_dropped_on(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);

    drag_toward(0, 2, &view, cx);
    drop_on(2, &view, cx);

    assert_eq!(song_ids(&view, cx), ["2", "3", "1"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(2)));
    wait_until(cx, |_| server.songs("a") == ["2", "3", "1"]);
}

#[gpui_kit::test]
fn a_song_dragged_up_lands_before_the_row_dropped_on(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);

    drag_toward(2, 0, &view, cx);
    drop_on(0, &view, cx);

    assert_eq!(song_ids(&view, cx), ["3", "1", "2"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(0)));
    wait_until(cx, |_| server.songs("a") == ["3", "1", "2"]);
}

#[gpui_kit::test]
fn the_songs_of_a_readonly_playlist_cant_be_dragged(cx: &mut TestAppContext) {
    let server = two_playlists();
    server.make_readonly("a");
    let (view, cx) = loaded_from(&server, cx);

    drag_toward(0, 2, &view, cx);

    cx.update(|_, cx| assert!(!cx.has_active_drag()));
    drop_on(2, &view, cx);
    assert_eq!(song_ids(&view, cx), ["1", "2", "3"]);
}

#[gpui_kit::test]
fn the_songs_of_a_playlist_that_diverged_cant_be_dragged_or_moved(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    server.refuse("createPlaylist");
    server.hold("getPlaylist");
    select_ix(&view, 0, cx);
    focus_songs(&view, cx);
    cx.simulate_keystrokes("alt-down");
    wait_until(cx, |cx| {
        cx.update(|_, cx| view.read(cx).shown.is_diverged())
    });
    let moves = Rc::new(Cell::new(0));
    cx.update(|_, cx| {
        let moves = moves.clone();
        cx.on_action(move |_: &MoveDown, _| moves.set(moves.get() + 1));
    });

    cx.simulate_keystrokes("alt-down");
    assert_eq!(moves.get(), 1);
    drag_toward(0, 2, &view, cx);

    cx.update(|_, cx| assert!(!cx.has_active_drag()));
    drop_on(2, &view, cx);
    server.release("getPlaylist");
}

#[gpui_kit::test]
fn the_songs_of_an_editable_playlist_can_be_dragged(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);

    drag_toward(0, 2, &view, cx);

    cx.update(|_, cx| assert!(cx.has_active_drag()));
}

#[gpui_kit::test]
fn the_page_keys_move_by_the_songs_in_view(cx: &mut TestAppContext) {
    let songs: Vec<String> = (0..100).map(|ix| ix.to_string()).collect();
    let songs: Vec<&str> = songs.iter().map(String::as_str).collect();
    let len = songs.len();
    let server = PlaylistsServer::new(&[("a", "alpha", &songs)]);
    let (view, cx) = loaded_from(&server, cx);
    focus_songs(&view, cx);
    cx.run_until_parked();
    let page = cx.update(|_, cx| {
        let state = view.read(cx).songs_scroll.0.borrow();
        let row = state.last_item_size.unwrap().contents.height / len as f32;
        (state.base_handle.bounds().size.height / row).floor() as usize
    });
    assert!(page > 1, "the window shows {page} rows");

    cx.simulate_keystrokes("home pagedown");
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(page)));
    cx.simulate_keystrokes("pageup");
    cx.update(|_, cx| assert_eq!(view.read(cx).selected_entry_ix(), Some(0)));
}

fn reveal_playlist(view: &Entity<Playlists>, id: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.reveal_playlist(PlaylistId::new(id), window, cx);
        });
    });
}

#[gpui_kit::test]
fn choosing_another_playlist_says_which_was_left(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    let left = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let left = left.clone();
        cx.subscribe(&view, move |_, event, _| {
            if let PlaylistsEvent::LeftPlaylist(playlist) = event {
                left.borrow_mut().push(playlist.clone());
            }
        })
        .detach();
    });

    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.choose(PlaylistId::new("a"), window, cx);
            this.choose(PlaylistId::new("b"), window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(*left.borrow(), vec![PlaylistId::new("a")]);

    select_playlist(&view, "a", cx);
    reveal_playlist(&view, "b", cx);
    cx.run_until_parked();
    assert_eq!(*left.borrow(), vec![PlaylistId::new("a")]);
}

#[gpui_kit::test]
fn revealing_a_playlist_the_filter_hides_clears_the_filter(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.filter
                .update(cx, |filter, cx| filter.set_value("alpha", window, cx));
            this.apply_filter(window, cx);
        });
    });

    reveal_playlist(&view, "b", cx);

    cx.update(|window, cx| {
        let this = view.read(cx);
        assert_eq!(this.filter.read(cx).value(), "");
        assert_eq!(this.selected_playlist_id(), Some(&PlaylistId::new("b")));
        assert!(this.list_focus.is_focused(window));
    });
}

#[gpui_kit::test]
fn revealing_a_playlist_that_has_gone_keeps_the_selection(cx: &mut TestAppContext) {
    let server = two_playlists();
    let (view, cx) = loaded_from(&server, cx);

    reveal_playlist(&view, "gone", cx);

    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).selected_playlist_id(),
            Some(&PlaylistId::new("a"))
        );
    });
}
