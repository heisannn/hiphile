use std::cell::RefCell;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{Modifiers, TestAppContext, VisualTestContext, point, px, size};
use hiphile_core::test_support;
use serde_json::{Value, json};
use subsonic::{Album, AlbumId, ArtistId, ScanStatus};

use super::*;

fn song_json(id: &str, title: &str, seconds: u64) -> Value {
    json!({
        "id": id, "title": title, "duration": seconds,
        "artist": format!("Artist of {title}"), "artistId": format!("artist-{id}"),
        "album": format!("Album of {title}"), "albumId": format!("album-{id}"),
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

fn server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while request.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let body = json!({ "subsonic-response": {
                "status": "ok",
                "version": "1.16.1",
                "searchResult3": { "song": [
                    song_json("c", "Cherokee", 100),
                    song_json("a", "Airegin", 300),
                    song_json("b", "Blues", 200),
                ]},
            }})
            .to_string();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len(),
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(body.as_bytes()).ok();
        }
    });
    url
}

fn view_on<'a>(
    url: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<Songs>, &'a mut VisualTestContext) {
    view_with(url, None, cx)
}

fn catalog_of(dir: &tempfile::TempDir, songs: Vec<Song>) -> catalog::Catalog {
    struct Saved(Vec<Song>);

    impl catalog::Source for Saved {
        async fn albums_page(&self, _: u32, _: u32) -> Result<Vec<Album>, client::Error> {
            Ok(Vec::new())
        }

        async fn artists(&self) -> Result<Vec<subsonic::Artist>, client::Error> {
            Ok(Vec::new())
        }

        async fn songs_page(&self, _: u32, _: u32) -> Result<Vec<Song>, client::Error> {
            Ok(self.0.clone())
        }

        async fn scan_status(&self) -> Result<ScanStatus, client::Error> {
            Ok(serde_json::from_value(json!({ "scanning": false })).unwrap())
        }

        async fn library_modified(&self) -> Result<Option<u64>, client::Error> {
            Ok(None)
        }
    }

    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let source = Saved(songs);
    futures::executor::block_on(catalog::sync_songs(
        &catalog,
        &source,
        catalog::Mode::Always,
    ))
    .unwrap();
    catalog
}

fn view_with<'a>(
    url: &str,
    catalog: Option<catalog::Catalog>,
    cx: &'a mut TestAppContext,
) -> (Entity<Songs>, &'a mut VisualTestContext) {
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        ui::list_navigation::bind_control_keys(cx);
        crate::init(cx);
    });
    let session = test_support::session_saving_into(catalog, url);
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let songs = SongList::new(executor, session.clone());
    cx.add_window_view(|window, cx| Songs::new(session, songs, window, cx))
}

/// The saved `songs` on show, with the server out of reach.
fn table_of(
    songs: Vec<Song>,
    cx: &mut TestAppContext,
) -> (Entity<Songs>, &mut VisualTestContext, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_of(&dir, songs);
    let (view, cx) = view_with("http://127.0.0.1:9", Some(catalog), cx);
    cx.simulate_resize(size(px(1000.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.activate(cx);
            view.focus.focus(window, cx);
        });
    });
    wait_for_songs(&view, cx);
    (view, cx, dir)
}

fn table(cx: &mut TestAppContext) -> (Entity<Songs>, &mut VisualTestContext, tempfile::TempDir) {
    table_of(songs(), cx)
}

fn shown(view: &Entity<Songs>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        let view = view.read(cx);
        (0..view.visible_len())
            .map(|position| view.visible_song(position).unwrap().id().to_string())
            .collect()
    })
}

fn click_heading(x: f32, cx: &mut VisualTestContext) {
    // Below the header, in the middle of the row of headings
    let rem = cx.update(|window, _| f32::from(window.rem_size()));
    let position = point(px(x), px(rem * 2.5 + rem));
    cx.simulate_click(position, Modifiers::default());
    cx.run_until_parked();
}

fn heading_x(column: Column, cx: &mut VisualTestContext) -> f32 {
    let (rem, width) = cx.update(|window, _| {
        (
            f32::from(window.rem_size()),
            f32::from(window.viewport_size().width),
        )
    });
    let inset = rem;
    let gap = rem * 0.75;
    let marker = rem;
    let time = rem * 4.;
    let shared = width - inset * 2. - marker - time - gap * 4.;
    let title = inset + marker + gap;
    let artist = title + shared * 3. / 7. + gap;
    let album = artist + shared * 2. / 7. + gap;
    let edge = rem * 0.5;
    match column {
        Column::Title => title + edge,
        Column::Artist => artist + edge,
        Column::Album => album + edge,
        // The time's heading sits at the end of its column
        Column::Time => width - inset - edge,
        Column::Added | Column::Released => unreachable!("only the menu sorts by a date"),
    }
}

fn dispatched_by<A: gpui_kit::Action + Clone>(keys: &str, cx: &mut VisualTestContext) -> Vec<A> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let seen = seen.clone();
        cx.on_action(move |action: &A, _| seen.borrow_mut().push(action.clone()));
    });
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    seen.take()
}

fn set_filter(view: &Entity<Songs>, query: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.filter
                .update(cx, |filter, cx| filter.set_value(query, window, cx));
            view.filter_changed(cx);
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn nothing_loads_until_the_screen_shows(cx: &mut TestAppContext) {
    let (view, cx) = view_on("http://127.0.0.1:9", cx);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let view = view.read(cx);
        assert!(!view.song_list.status().is_loading());
        assert!(matches!(view.status.songs(), Shelf::Loading));
    });
}

#[gpui_kit::test]
fn with_nothing_saved_the_servers_songs_show_by_title(cx: &mut TestAppContext) {
    let url = server();
    let (view, cx) = view_on(&url, cx);
    cx.update(|_, cx| view.update(cx, |view, cx| view.activate(cx)));

    wait_for_songs(&view, cx);

    assert_eq!(shown(&view, cx), ["a", "b", "c"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected, Some(SongId::new("a"))));
}

fn wait_for_songs(view: &Entity<Songs>, cx: &mut VisualTestContext) {
    wait_until(view, cx, |view| view.shown().is_some());
}

fn wait_until(view: &Entity<Songs>, cx: &mut VisualTestContext, done: impl Fn(&Songs) -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if cx.update(|_, cx| done(view.read(cx))) {
            cx.run_until_parked();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("what was waited for never happened");
}

#[gpui_kit::test]
fn a_sort_chosen_during_the_first_load_holds_when_the_songs_arrive(cx: &mut TestAppContext) {
    let url = server();
    let (view, cx) = view_on(&url, cx);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.activate(cx);
            view.sort_by(Column::Time, cx);
        });
    });

    wait_for_songs(&view, cx);

    assert_eq!(shown(&view, cx), ["c", "b", "a"]);
}

#[gpui_kit::test]
fn a_filter_typed_during_a_reload_holds_when_the_songs_arrive(cx: &mut TestAppContext) {
    let (view, cx, dir) = table(cx);
    let mut more = songs();
    more.push(serde_json::from_value(song_json("d", "Donna Lee", 150)).unwrap());
    catalog_of(&dir, more);
    cx.update(|_, cx| view.update(cx, |view, _| view.song_list.reload()));
    set_filter(&view, "blues", cx);
    assert_eq!(shown(&view, cx), ["b"]);

    wait_until(&view, cx, |view| {
        view.shown().is_some_and(|shown| shown.songs().len() == 4)
    });

    assert_eq!(shown(&view, cx), ["b"]);
}

#[gpui_kit::test]
fn the_saved_songs_show_without_asking_the_server(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);

    assert_eq!(shown(&view, cx), ["a", "b", "c"]);
    cx.update(|_, cx| assert_eq!(view.read(cx).status.failure(), None));
}

#[gpui_kit::test]
fn a_failed_reload_keeps_the_songs_until_a_new_try_starts(cx: &mut TestAppContext) {
    let (view, cx, dir) = table(cx);
    // With nothing saved, the server is asked, and cannot be reached
    catalog_of(&dir, Vec::new());
    cx.update(|_, cx| view.update(cx, |view, _| view.song_list.reload()));

    wait_until(&view, cx, |view| view.status.failure().is_some());

    assert_eq!(shown(&view, cx), ["a", "b", "c"]);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.song_list.reload();
            view.sync(cx);
            assert_eq!(view.status.failure(), None);
        });
    });
}

#[gpui_kit::test]
fn clicking_headings_sorts_and_clicking_again_turns_it_around(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    assert_eq!(shown(&view, cx), ["a", "b", "c"]);

    let title = heading_x(Column::Title, cx);
    click_heading(title, cx);
    assert_eq!(shown(&view, cx), ["c", "b", "a"]);
    click_heading(title, cx);
    assert_eq!(shown(&view, cx), ["a", "b", "c"]);

    let artist = heading_x(Column::Artist, cx);
    click_heading(artist, cx);
    cx.update(|_, cx| assert_eq!(view.read(cx).status.sort().column, Column::Artist));
    assert_eq!(shown(&view, cx), ["a", "b", "c"]);
    click_heading(artist, cx);
    assert_eq!(shown(&view, cx), ["c", "b", "a"]);

    let album = heading_x(Column::Album, cx);
    click_heading(album, cx);
    cx.update(|_, cx| assert_eq!(view.read(cx).status.sort().column, Column::Album));

    let time = heading_x(Column::Time, cx);
    click_heading(time, cx);
    cx.update(|_, cx| assert_eq!(view.read(cx).status.sort().column, Column::Time));
    assert_eq!(shown(&view, cx), ["c", "b", "a"]);
    click_heading(time, cx);
    assert_eq!(shown(&view, cx), ["a", "b", "c"]);
}

#[gpui_kit::test]
fn a_date_from_the_menu_sorts_through_a_filter_and_a_heading_starts_from_the_top(
    cx: &mut TestAppContext,
) {
    let dated = songs()
        .into_iter()
        .zip([
            "2024-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
            "2023-01-01T00:00:00Z",
        ])
        .map(|(song, created)| {
            let mut fields = serde_json::to_value(&song).unwrap();
            fields["created"] = json!(created);
            serde_json::from_value(fields).unwrap()
        })
        .collect();
    let (view, cx, _dir) = table_of(dated, cx);
    set_filter(&view, "e", cx);
    cx.update(|_, cx| view.update(cx, |view, cx| view.choose_sort(Column::Added, cx)));
    cx.run_until_parked();
    // Cherokee went in in 2024, Airegin in 2025 and Blues in 2023
    assert_eq!(shown(&view, cx), ["a", "c", "b"]);
    cx.update(|_, cx| assert!(!view.read(cx).is_ranked()));

    cx.update(|_, cx| view.update(cx, |view, cx| view.choose_direction(false, cx)));
    cx.run_until_parked();
    assert_eq!(shown(&view, cx), ["b", "c", "a"]);
    // Choosing the order on show again keeps it the way it runs
    cx.update(|_, cx| view.update(cx, |view, cx| view.choose_sort(Column::Added, cx)));
    cx.run_until_parked();
    assert_eq!(shown(&view, cx), ["b", "c", "a"]);

    click_heading(heading_x(Column::Title, cx), cx);
    assert_eq!(shown(&view, cx), ["a", "b", "c"]);
}

#[gpui_kit::test]
fn clicking_an_album_in_a_row_shows_it_and_leaves_the_selection(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    let shown = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let shown = shown.clone();
        cx.on_action(move |action: &ShowAlbum, _| {
            shown.borrow_mut().push(action.album().clone());
        });
    });

    // Past the search header and the headings, in the middle of the first row
    let rem = cx.update(|window, _| f32::from(window.rem_size()));
    let x = heading_x(Column::Album, cx);
    cx.simulate_click(point(px(x), px(rem * 5.5)), Modifiers::default());
    cx.run_until_parked();

    assert_eq!(shown.take(), [AlbumId::new("album-a")]);
    cx.update(|_, cx| assert!(!view.read(cx).has_chosen_selection));
}

#[gpui_kit::test]
fn the_first_song_stays_selected_until_one_is_chosen(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    cx.update(|_, cx| view.update(cx, |view, cx| view.sort_by(Column::Time, cx)));
    cx.run_until_parked();

    cx.update(|_, cx| {
        let view = view.read(cx);
        assert_eq!(view.selected_position(), Some(0));
        assert_eq!(
            view.selected,
            view.visible_song(0).map(|song| song.id().clone())
        );
    });

    set_filter(&view, "nothing matches this", cx);
    cx.update(|_, cx| assert_eq!(view.read(cx).selected, None));
}

#[gpui_kit::test]
fn the_selection_follows_its_song_when_the_order_changes(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(view.read(cx).selected, Some(SongId::new("b"))));

    cx.update(|_, cx| view.update(cx, |view, cx| view.sort_by(Column::Time, cx)));
    cx.run_until_parked();

    cx.update(|_, cx| {
        let view = view.read(cx);
        assert_eq!(view.selected, Some(SongId::new("b")));
        assert_eq!(view.selected_position(), Some(1));
    });
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(view.read(cx).selected, Some(SongId::new("a"))));
}

#[gpui_kit::test]
fn a_heading_clicked_during_a_filter_holds_until_the_filter_is_emptied(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);

    set_filter(&view, "e", cx);
    cx.update(|_, cx| view.update(cx, |view, cx| view.sort_by(Column::Time, cx)));
    cx.run_until_parked();
    assert_eq!(shown(&view, cx), ["c", "b", "a"]);

    // Typing on keeps to the heading
    set_filter(&view, "ee", cx);
    set_filter(&view, "e", cx);
    assert_eq!(shown(&view, cx), ["c", "b", "a"]);
    cx.update(|_, cx| assert!(!view.read(cx).is_ranked()));

    // A new filter ranks again, with the sort kept for when it is emptied
    set_filter(&view, "", cx);
    assert_eq!(shown(&view, cx), ["c", "b", "a"]);
    set_filter(&view, "e", cx);
    cx.update(|_, cx| assert!(view.read(cx).is_ranked()));
    set_filter(&view, "nobody", cx);
    assert!(shown(&view, cx).is_empty());
}

#[gpui_kit::test]
fn a_filter_shows_no_heading_as_sorted_until_one_is_clicked(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    set_filter(&view, "e", cx);
    cx.update(|_, cx| assert!(view.read(cx).is_ranked()));

    cx.update(|_, cx| view.update(cx, |view, cx| view.sort_by(Column::Title, cx)));
    cx.run_until_parked();

    cx.update(|_, cx| {
        let view = view.read(cx);
        assert!(!view.is_ranked());
        // The heading was not showing as sorted, so its first click starts it
        assert!(!view.status.sort().is_descending);
    });
}

#[gpui_kit::test]
fn enter_plays_the_songs_on_show_in_their_order(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    cx.update(|_, cx| view.update(cx, |view, cx| view.sort_by(Column::Time, cx)));
    cx.run_until_parked();
    cx.update(|_, cx| {
        view.update(cx, |view, cx| view.select_song(SongId::new("b"), cx));
    });

    let played: Vec<PlaySongs> = dispatched_by("enter", cx);

    assert_eq!(played.len(), 1);
    let ids: Vec<_> = played[0].songs().iter().map(|s| s.id().as_str()).collect();
    assert_eq!(ids, ["c", "b", "a"]);
    assert_eq!(played[0].start_ix(), Some(1));
}

#[gpui_kit::test]
fn a_filter_narrows_what_plays(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    set_filter(&view, "blues", cx);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| view.select_song(SongId::new("b"), cx));
    });

    let played: Vec<PlaySongs> = dispatched_by("enter", cx);

    assert_eq!(played[0].songs().len(), 1);
    assert_eq!(played[0].start_ix(), Some(0));
}

#[gpui_kit::test]
fn a_selection_the_filter_hides_is_acted_on_by_nothing(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    cx.update(|_, cx| view.update(cx, |view, cx| view.select_song(SongId::new("a"), cx)));
    set_filter(&view, "blues", cx);

    let played: Vec<PlaySongs> = dispatched_by("enter", cx);

    cx.update(|_, cx| assert_eq!(view.read(cx).selected, Some(SongId::new("a"))));
    assert!(played.is_empty());
}

#[gpui_kit::test]
fn the_keys_queue_the_selected_song_and_go_to_its_album_and_artist(cx: &mut TestAppContext) {
    let (_view, cx, _dir) = table(cx);

    let next: Vec<EnqueueSongs> = dispatched_by("alt-enter", cx);
    assert_eq!(next[0].placement(), Placement::Next);
    assert_eq!(next[0].songs()[0].id(), &SongId::new("a"));

    let last: Vec<EnqueueSongs> = dispatched_by("shift-enter", cx);
    assert_eq!(last[0].placement(), Placement::Last);

    let command = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let albums: Vec<ShowAlbum> = dispatched_by(&format!("{command}-l"), cx);
    assert_eq!(albums[0].album(), &AlbumId::new("album-a"));

    let artists: Vec<ShowArtist> = dispatched_by(&format!("{command}-shift-a"), cx);
    assert_eq!(artists[0].artist(), &ArtistId::new("artist-a"));
}

#[gpui_kit::test]
fn the_favorite_key_goes_to_the_selected_song(cx: &mut TestAppContext) {
    let (_view, cx, _dir) = table(cx);
    let key = if cfg!(target_os = "macos") {
        "cmd-d"
    } else {
        "ctrl-d"
    };

    let asked: Vec<actions::ToggleFavoriteOf> = dispatched_by(key, cx);

    let target = subsonic::StarTarget::Song(SongId::new("a"));
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].favorite().target(), target);
}

#[gpui_kit::test]
fn the_song_playing_is_marked_only_when_it_changes(cx: &mut TestAppContext) {
    let (view, cx, _dir) = table(cx);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_now_playing(Some(SongId::new("a")), true, cx);
            assert!(view.now_playing.is_some());
            view.set_now_playing(None, false, cx);
            assert!(view.now_playing.is_none());
        });
    });
}
