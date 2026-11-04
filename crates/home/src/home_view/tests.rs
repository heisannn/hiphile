use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    Entity, KeyBinding, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext, actions, px, size,
};
use serde_json::json;
use subsonic::Credentials;

use super::*;

fn song(id: &str) -> Song {
    serde_json::from_value(json!({
        "id": id, "title": id, "played": "2026-11-03T22:36:15Z", "albumId": "a1",
    }))
    .unwrap()
}

fn album(id: &str) -> Album {
    serde_json::from_value(json!({ "id": id, "name": id })).unwrap()
}

fn albums(count: usize) -> Vec<Album> {
    (0..count).map(|ix| album(&format!("a{ix}"))).collect()
}

fn recommended(count: usize) -> Vec<Album> {
    (0..count).map(|ix| album(&format!("r{ix}"))).collect()
}

/// A home screen whose server cannot be reached.
fn home(cx: &mut TestAppContext) -> (Entity<Home>, &mut VisualTestContext) {
    home_with(None, cx)
}

fn catalog_in(dir: &tempfile::TempDir) -> catalog::Catalog {
    catalog::Catalog::open(dir.path()).unwrap()
}

/// A home screen for `alice` that keeps its picks in `catalog`.
fn home_with(
    catalog: Option<catalog::Catalog>,
    cx: &mut TestAppContext,
) -> (Entity<Home>, &mut VisualTestContext) {
    // The failing requests wake the test from the network's own thread.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::init(cx);
    });
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new("http://127.0.0.1:9", Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    let session = session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        catalog,
        None,
    );
    cx.add_window_view(|window, cx| Home::new(session, window, cx))
}

fn update(
    home: &Entity<Home>,
    cx: &mut VisualTestContext,
    update: impl FnOnce(&mut Home, &mut Context<Home>),
) {
    cx.update(|_, cx| home.update(cx, update));
}

/// Fills the shelves, with no recommendations, cancelling the requests
/// made on opening.
fn fill(home: &mut Home, songs: &[&str], album_count: usize) {
    home._recommended_task = Task::ready(());
    home.recommending = false;
    home.recommended_retry = None;
    home.recommended_day = None;
    home.recommended = Shelf::Loaded(Arc::from([]));
    home._recent_task = Task::ready(());
    home._most_played_task = Task::ready(());
    home.most_played = Shelf::Loaded(Arc::from([]));
    home._new_albums_task = Task::ready(());
    home.recent = Shelf::Loaded(songs.iter().map(|id| song(id)).collect());
    home.new_albums = Shelf::Loaded(albums(album_count).into());
}

#[gpui_kit::test]
fn no_plays_takes_the_song_shelf_away(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &[], 1);
        assert_eq!(
            home.parts(),
            [
                Part::NewAlbumsHeading,
                Part::AlbumRow(AlbumShelf::New, 0..1)
            ]
        );
    });
}

#[gpui_kit::test]
fn loading_and_failing_shelves_keep_their_place(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        home._recommended_task = Task::ready(());
        home._recent_task = Task::ready(());
        home._most_played_task = Task::ready(());
        home._new_albums_task = Task::ready(());
        home.recommended = Shelf::Failed("offline".into());
        home.recent = Shelf::Loading;
        home.most_played = Shelf::Failed("offline".into());
        home.new_albums = Shelf::Failed("offline".into());
        assert_eq!(
            home.parts(),
            [
                Part::RecommendedHeading,
                Part::RecommendedStatus,
                Part::RecentHeading,
                Part::RecentStatus,
                Part::MostPlayedHeading,
                Part::MostPlayedStatus,
                Part::NewAlbumsHeading,
                Part::NewAlbumsStatus,
            ]
        );
    });
}

#[gpui_kit::test]
fn albums_show_in_two_rows_of_as_many_as_fit(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &[], 24);
        home.columns = 3;
        assert_eq!(
            home.parts(),
            [
                Part::NewAlbumsHeading,
                Part::AlbumRow(AlbumShelf::New, 0..3),
                Part::AlbumRow(AlbumShelf::New, 3..6),
            ]
        );
        fill(home, &[], 4);
        assert_eq!(
            home.parts(),
            [
                Part::NewAlbumsHeading,
                Part::AlbumRow(AlbumShelf::New, 0..3),
                Part::AlbumRow(AlbumShelf::New, 3..4),
            ]
        );
    });
}

#[gpui_kit::test]
fn recommendations_come_first_in_one_row_of_as_many_as_fit(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &["s1"], 1);
        home.recommended = Shelf::Loaded(recommended(12).into());
        home.columns = 3;
        assert_eq!(
            home.parts(),
            [
                Part::RecommendedHeading,
                Part::AlbumRow(AlbumShelf::Recommended, 0..3),
                Part::RecentHeading,
                Part::Song(0),
                Part::NewAlbumsHeading,
                Part::AlbumRow(AlbumShelf::New, 0..1),
            ]
        );
        home.recommended = Shelf::Loaded(recommended(2).into());
        assert_eq!(
            home.parts()[1],
            Part::AlbumRow(AlbumShelf::Recommended, 0..2)
        );
    });
}

#[gpui_kit::test]
fn most_played_comes_above_new_albums_in_one_row_of_as_many_as_fit(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &["s1"], 1);
        home.most_played = Shelf::Loaded(albums(12).into());
        home.columns = 3;
        assert_eq!(
            home.parts(),
            [
                Part::RecentHeading,
                Part::Song(0),
                Part::MostPlayedHeading,
                Part::AlbumRow(AlbumShelf::MostPlayed, 0..3),
                Part::NewAlbumsHeading,
                Part::AlbumRow(AlbumShelf::New, 0..1),
            ]
        );
        home.most_played = Shelf::Loaded(albums(2).into());
        assert_eq!(
            home.parts()[3],
            Part::AlbumRow(AlbumShelf::MostPlayed, 0..2)
        );
    });
}

#[gpui_kit::test]
fn no_plays_takes_the_most_played_shelf_away(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &[], 1);
        assert!(!home.parts().contains(&Part::MostPlayedHeading));
    });
}

#[gpui_kit::test]
fn the_keys_pass_through_an_album_on_two_shelves(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 1);
        home.most_played = Shelf::Loaded(albums(1).into());
        cx.notify();
    });
    cx.update(|window, cx| {
        let focus = home.read(cx).focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();

    let selection =
        |cx: &mut VisualTestContext| cx.update(|_, cx| home.read(cx).selection.clone()).unwrap();
    let on = |shelf| Item::Album(shelf, AlbumId::new("a0"));
    cx.simulate_keystrokes("down");
    assert_eq!(selection(cx), on(AlbumShelf::MostPlayed));
    cx.simulate_keystrokes("down");
    assert_eq!(selection(cx), on(AlbumShelf::New));
    cx.simulate_keystrokes("up");
    assert_eq!(selection(cx), on(AlbumShelf::MostPlayed));
}

#[gpui_kit::test]
fn an_album_on_the_most_played_shelf_leaves_the_recommendations(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(2).into());
        home.most_played = Shelf::Loaded(Arc::from([album("r1")]));

        home.keep_shelves_off_recommended();

        assert_eq!(shown(home), ["r0"]);
    });
}

#[gpui_kit::test]
fn a_play_reloads_the_shelves_it_changes_keeping_what_is_on_show(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &["s1"], 0);
        home.most_played = Shelf::Loaded(albums(1).into());
        home.recommended_day = Some(today());

        home.song_played(cx);

        assert_eq!(home.recent.items().len(), 1);
        assert_eq!(home.most_played.items().len(), 1);
        assert!(!home.recommending);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_arrow_keys_move_from_the_recommendations_to_the_songs(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &["s1"], 1);
        home.recommended = Shelf::Loaded(recommended(2).into());
    });
    cx.update(|window, cx| {
        let focus = home.read(cx).focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();

    let selection =
        |cx: &mut VisualTestContext| cx.update(|_, cx| home.read(cx).selection.clone()).unwrap();
    cx.simulate_keystrokes("down");
    assert_eq!(
        selection(cx),
        Item::Album(AlbumShelf::Recommended, AlbumId::new("r0"))
    );
    cx.simulate_keystrokes("right");
    assert_eq!(
        selection(cx),
        Item::Album(AlbumShelf::Recommended, AlbumId::new("r1"))
    );
    cx.simulate_keystrokes("down");
    assert_eq!(selection(cx), Item::Song(SongId::new("s1")));
}

#[gpui_kit::test]
fn enter_shows_a_recommended_album_in_the_library(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(1).into());
        home.selection = Some(Item::Album(AlbumShelf::Recommended, AlbumId::new("r0")));
        cx.notify();
    });
    cx.update(|window, cx| {
        let focus = home.read(cx).focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    let shown = Rc::new(RefCell::new(None));
    cx.update(|_, cx| {
        let shown = shown.clone();
        cx.on_action(move |action: &ShowAlbum, _| {
            *shown.borrow_mut() = Some(action.album().clone());
        });
    });

    cx.simulate_keystrokes("enter");

    assert_eq!(*shown.borrow(), Some(AlbumId::new("r0")));
}

#[gpui_kit::test]
fn a_recommended_album_past_the_row_is_off_the_screen(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(6).into());
        home.columns = 6;
        home.selection = Some(Item::Album(AlbumShelf::Recommended, AlbumId::new("r5")));
        assert_eq!(home.selection_on_screen(), home.selection);

        home.columns = 4;
        assert_eq!(home.selection_on_screen(), None);
    });
}

fn found(ids: &[&str], complete: bool) -> Result<Recommendations, SharedString> {
    Ok(Recommendations {
        albums: ids.iter().map(|id| album(id)).collect(),
        complete,
    })
}

fn shown(home: &Home) -> Vec<String> {
    home.recommended
        .items()
        .iter()
        .map(|album| album.id().to_string())
        .collect()
}

#[gpui_kit::test]
fn a_complete_pick_settles_the_day_and_is_kept(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    let (home, cx) = home_with(Some(catalog.clone()), cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.finish_recommended(found(&["r0"], true), today(), cx);
        assert_eq!(shown(home), ["r0"]);
        assert_eq!(home.recommended_day, Some(today()));
    });
    cx.run_until_parked();

    let kept = catalog.recommended("alice").unwrap().unwrap();
    assert_eq!(kept.day, today());
    assert_eq!(kept.albums[0].id().as_str(), "r0");
}

#[gpui_kit::test]
fn an_incomplete_pick_is_not_kept_and_leaves_what_is_on_show(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    let (home, cx) = home_with(Some(catalog.clone()), cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(2).into());
        home.finish_recommended(found(&["other"], false), today(), cx);
        assert_eq!(shown(home), ["r0", "r1"]);
        assert_eq!(home.recommended_day, None);
        assert!(home.recommended_retry.is_some());
    });
    cx.run_until_parked();

    assert!(catalog.recommended("alice").unwrap().is_none());
}

#[gpui_kit::test]
fn an_incomplete_pick_shows_with_nothing_else_to_show(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loading;
        home.finish_recommended(found(&["r0"], false), today(), cx);
        assert_eq!(shown(home), ["r0"]);
        assert_eq!(home.recommended_day, None);
    });
}

#[gpui_kit::test]
fn a_failed_pick_does_not_settle_the_day(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(1).into());
        home.finish_recommended(Err("offline".into()), today(), cx);
        assert_eq!(shown(home), ["r0"]);
        assert_eq!(home.recommended_day, None);
        assert!(home.recommended_retry.is_some());
    });
}

#[gpui_kit::test]
fn an_empty_pick_is_not_kept_and_leaves_what_is_on_show(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("alice", "2000-01-01", &recommended(1))
        .unwrap();
    let (home, cx) = home_with(Some(catalog.clone()), cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(1).into());
        home.finish_recommended(found(&[], true), today(), cx);
        assert_eq!(shown(home), ["r0"]);
        // Plays need not ask again.
        home.renew_recommended(cx);
        assert!(!home.recommending);
    });
    cx.run_until_parked();

    assert_eq!(
        catalog.recommended("alice").unwrap().unwrap().day,
        "2000-01-01"
    );
}

#[gpui_kit::test]
fn a_library_sync_gives_an_empty_shelf_another_pick(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.finish_recommended(found(&[], true), today(), cx);

        home.library_synced(cx);

        assert!(home.recommending);
    });
}

#[gpui_kit::test]
fn a_library_sync_leaves_the_days_pick_alone(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.finish_recommended(found(&["r0"], true), today(), cx);

        home.library_synced(cx);

        assert!(!home.recommending);
    });
}

#[gpui_kit::test]
fn a_pick_that_went_wrong_is_not_made_again_right_away(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.finish_recommended(Err("offline".into()), today(), cx);

        home.renew_recommended(cx);
        assert!(!home.recommending);

        home.recommended_retry = Some(Instant::now());
        home.renew_recommended(cx);
        assert!(home.recommending);
    });
}

#[gpui_kit::test]
fn retry_does_not_wait(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.finish_recommended(Err("offline".into()), today(), cx);
        home.pick_recommended(cx);
        assert!(home.recommending);
        assert_eq!(home.recommended_retry, None);
    });
}

#[gpui_kit::test]
fn todays_kept_pick_shows_again_with_no_new_pick(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("alice", &today(), &recommended(2))
        .unwrap();
    let (home, cx) = home_with(Some(catalog), cx);

    cx.run_until_parked();

    cx.update(|_, cx| {
        let home = home.read(cx);
        assert_eq!(shown(home), ["r0", "r1"]);
        assert!(!home.recommending);
        assert_eq!(home.recommended_retry, None);
    });
}

#[gpui_kit::test]
fn yesterdays_kept_pick_shows_while_a_new_one_is_made(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("alice", "2000-01-01", &recommended(1))
        .unwrap();
    let (home, cx) = home_with(Some(catalog), cx);

    cx.run_until_parked();

    cx.update(|_, cx| {
        let home = home.read(cx);
        assert_eq!(shown(home), ["r0"]);
        // On its way, or already failed for want of a server.
        assert!(home.recommending || home.recommended_retry.is_some());
    });
}

#[gpui_kit::test]
fn another_users_kept_pick_does_not_show(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("bob", &today(), &recommended(1))
        .unwrap();
    let (home, cx) = home_with(Some(catalog), cx);

    cx.run_until_parked();

    cx.update(|_, cx| assert!(shown(home.read(cx)).is_empty()));
}

#[gpui_kit::test]
fn coming_back_to_the_window_on_another_day_picks_again(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(1).into());
        home.recommended_day = Some("2000-01-01".to_owned());
    });
    cx.deactivate_window();
    cx.run_until_parked();
    cx.update(|_, cx| assert!(!home.read(cx).recommending));

    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();

    cx.update(|_, cx| {
        let home = home.read(cx);
        assert!(home.recommending || home.recommended_retry.is_some());
    });
}

#[gpui_kit::test]
fn an_album_that_becomes_new_leaves_the_recommendations(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(2).into());
        home.new_albums = Shelf::Loaded(Arc::from([album("r0")]));

        home.keep_shelves_off_recommended();

        let ids: Vec<_> = home
            .recommended
            .items()
            .iter()
            .map(|album| album.id().as_str())
            .collect();
        assert_eq!(ids, ["r1"]);
    });
}

#[gpui_kit::test]
fn the_recommendations_stay_for_the_day(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.recommended_day = Some(today());
        home.renew_recommended(cx);
        assert!(!home.recommending);
    });
}

#[gpui_kit::test]
fn another_day_picks_new_recommendations_with_the_old_on_show(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.recommended = Shelf::Loaded(recommended(1).into());
        home.recommended_day = Some("2000-01-01".to_owned());

        home.renew_recommended(cx);

        assert!(home.recommending);
        assert_eq!(home.recommended.items().len(), 1);
    });
}

#[gpui_kit::test]
fn a_pick_on_its_way_is_not_started_again(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &[], 0);
        home.recommended = Shelf::Failed("before".into());
        home.recommending = true;

        home.renew_recommended(cx);

        assert!(matches!(home.recommended, Shelf::Failed(_)));
    });
}

#[gpui_kit::test]
fn the_arrow_keys_move_through_the_songs_into_the_albums(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &["s1", "s2"], 4);
    });
    // Room for two cards across.
    cx.simulate_resize(size(px(24. * 16.), px(800.)));
    cx.update(|window, cx| {
        let focus = home.read(cx).focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();

    let selection =
        |cx: &mut VisualTestContext| cx.update(|_, cx| home.read(cx).selection.clone()).unwrap();
    cx.simulate_keystrokes("down");
    assert_eq!(selection(cx), Item::Song(SongId::new("s1")));
    cx.simulate_keystrokes("down down");
    assert_eq!(
        selection(cx),
        Item::Album(AlbumShelf::New, AlbumId::new("a0"))
    );
    cx.simulate_keystrokes("right");
    assert_eq!(
        selection(cx),
        Item::Album(AlbumShelf::New, AlbumId::new("a1"))
    );
    cx.simulate_keystrokes("down");
    assert_eq!(
        selection(cx),
        Item::Album(AlbumShelf::New, AlbumId::new("a3"))
    );
    cx.simulate_keystrokes("pageup");
    assert_eq!(selection(cx), Item::Song(SongId::new("s1")));
    cx.simulate_keystrokes("pagedown");
    assert_eq!(
        selection(cx),
        Item::Album(AlbumShelf::New, AlbumId::new("a2"))
    );
    cx.simulate_keystrokes("home");
    assert_eq!(selection(cx), Item::Song(SongId::new("s1")));
    cx.simulate_keystrokes("end");
    assert_eq!(
        selection(cx),
        Item::Album(AlbumShelf::New, AlbumId::new("a3"))
    );
}

/// Whether `key` presses a focused Retry, which the screen's own use of
/// the key must not take.
fn key_presses_retry(key: &str, cx: &mut TestAppContext) -> bool {
    actions!(home_test, [TakenByTheWorkspace]);
    let (home, cx) = home(cx);
    cx.update(|_, cx| {
        cx.bind_keys([KeyBinding::new(
            "space",
            TakenByTheWorkspace,
            Some(HOME_CONTEXT),
        )]);
        cx.on_action(|_: &TakenByTheWorkspace, _| {});
    });
    update(&home, cx, |home, cx| {
        fill(home, &[], 1);
        home.recent = Shelf::Failed("before".into());
        home.selection = Some(Item::Album(AlbumShelf::New, AlbumId::new("a0")));
        cx.notify();
    });
    cx.update(|window, cx| {
        let focus = home.read(cx).focus.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.focus_next(cx));
    cx.run_until_parked();

    // A button acts on the release.
    cx.simulate_keystrokes(key);
    let os = cx.update(|_, cx| cx.platform_info().os);
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse_with_os(key, os).unwrap(),
    });

    // Pressed, it loads again, and whatever that comes to is not this.
    cx.update(|_, cx| !matches!(&home.read(cx).recent, Shelf::Failed(error) if error == "before"))
}

#[gpui_kit::test]
fn enter_presses_a_focused_retry(cx: &mut TestAppContext) {
    assert!(key_presses_retry("enter", cx));
}

#[gpui_kit::test]
fn space_presses_a_focused_retry(cx: &mut TestAppContext) {
    assert!(key_presses_retry("space", cx));
}

#[gpui_kit::test]
fn commands_ignore_a_selection_a_narrower_window_took_off_the_screen(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &[], 24);
        home.columns = 7;
        home.selection = Some(Item::Album(AlbumShelf::New, AlbumId::new("a13")));
        assert_eq!(home.selection_on_screen(), home.selection);

        home.columns = 2;
        assert_eq!(home.selection_on_screen(), None);
        assert_eq!(
            home.selection,
            Some(Item::Album(AlbumShelf::New, AlbumId::new("a13")))
        );

        home.columns = 7;
        assert_eq!(home.selection_on_screen(), home.selection);
    });
}

#[gpui_kit::test]
fn a_reload_in_progress_keeps_what_is_on_show(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, cx| {
        fill(home, &["s1"], 1);
        home.reload_recently_played(cx);
        home.reload_new_albums(cx);
        assert_eq!(home.recent.items().len(), 1);
        assert_eq!(home.new_albums.items().len(), 1);
    });
}

#[gpui_kit::test]
fn an_incomplete_reload_keeps_what_is_on_show(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &["s1", "s2"], 0);
        home.finish_recently_played(Ok(RecentlyPlayed {
            songs: vec![song("s2")],
            complete: false,
        }));
        assert_eq!(home.recent.items().len(), 2);
    });
}

#[gpui_kit::test]
fn an_incomplete_first_load_shows_what_it_found(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        home.recent = Shelf::Loading;
        home.finish_recently_played(Ok(RecentlyPlayed {
            songs: vec![song("s2")],
            complete: false,
        }));
        assert_eq!(home.recent.items().len(), 1);
    });
}

#[gpui_kit::test]
fn the_cards_across_follow_the_width_of_the_screen(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| fill(home, &["s1"], 24));
    let columns = |cx: &mut VisualTestContext, width: f32| {
        cx.simulate_resize(size(px(width), px(800.)));
        cx.run_until_parked();
        cx.update(|_, cx| home.read(cx).columns)
    };

    assert_eq!(columns(cx, 80. * 16.), 7);
    assert_eq!(columns(cx, 46. * 16.), 4);
}

#[gpui_kit::test]
fn a_failed_reload_keeps_what_is_on_show(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &["s1"], 1);
        home.recent.finish(Err("offline".into()));
        assert_eq!(home.recent.items().len(), 1);
    });
}

#[gpui_kit::test]
fn a_failure_with_nothing_on_show_says_so(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        home.recent = Shelf::Loading;
        home.recent.finish(Err("offline".into()));
        assert!(matches!(home.recent, Shelf::Failed(_)));
    });
}

#[gpui_kit::test]
fn a_reload_keeps_the_selection_of_a_song_still_there(cx: &mut TestAppContext) {
    let (home, cx) = home(cx);
    update(&home, cx, |home, _| {
        fill(home, &["s1", "s2"], 0);
        let selection = Some(Item::Song(SongId::new("s2")));
        home.selection = selection.clone();

        home.finish_recently_played(Ok(RecentlyPlayed {
            songs: vec![song("s2"), song("s1")],
            complete: true,
        }));

        assert_eq!(home.selection_on_screen(), selection);
        assert_eq!(home.song_ix(&SongId::new("s2")), Some(0));
    });
}
