use gpui_kit::TestAppContext;
use serde_json::json;

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::NOWHERE;

fn album(id: &str) -> Album {
    serde_json::from_value(json!({ "id": id, "name": id })).unwrap()
}

fn recommended(count: usize) -> Vec<Album> {
    (0..count).map(|ix| album(&format!("r{ix}"))).collect()
}

fn catalog_in(dir: &tempfile::TempDir) -> catalog::Catalog {
    catalog::Catalog::open(dir.path()).unwrap()
}

/// On GPUI's executor, which the test's `run_until_parked` and clock drive, for `alice`, who
/// keeps her picks in `catalog`, with a server that cannot be reached.
fn recommendations(
    catalog: Option<catalog::Catalog>,
    cx: &mut TestAppContext,
) -> DailyRecommendations {
    // The failing requests wake the test from the network's own thread
    cx.executor().allow_parking();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    DailyRecommendations::new(executor, session_saving_into(catalog, NOWHERE))
}

fn update<R>(
    recommendations: &DailyRecommendations,
    change: impl FnOnce(&DailyRecommendations, &mut State) -> R,
) -> R {
    recommendations
        .0
        .state
        .update(|state| change(recommendations, state))
}

/// No recommendations, cancelling the reading made on opening.
fn clear(state: &mut State) {
    state._work = Task::ready(());
    state.status.is_recommending = false;
    state.is_waiting_to_retry = false;
    state._retry = Task::ready(());
    state.day = None;
    state.status.albums = RecommendedAlbums::Loaded(Arc::from([]));
}

fn on_show(albums: Vec<Album>) -> RecommendedAlbums {
    RecommendedAlbums::Loaded(albums.into())
}

fn found(ids: &[&str], complete: bool) -> Result<Recommendations, String> {
    Ok(Recommendations::new(
        ids.iter().map(|id| album(id)).collect(),
        complete,
    ))
}

fn shown(state: &State) -> Vec<String> {
    state
        .status
        .albums
        .albums()
        .iter()
        .map(|album| album.id().to_string())
        .collect()
}

#[gpui_kit::test]
fn a_complete_pick_settles_the_day_and_is_kept(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    let recommendations = recommendations(Some(catalog.clone()), cx);
    update(&recommendations, |this, state| {
        clear(state);
        this.finish(state, found(&["r0"], true), today());
        assert_eq!(shown(state), ["r0"]);
        assert_eq!(state.day, Some(today()));
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
    let recommendations = recommendations(Some(catalog.clone()), cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.status.albums = on_show(recommended(2));
        this.finish(state, found(&["other"], false), today());
        assert_eq!(shown(state), ["r0", "r1"]);
        assert_eq!(state.day, None);
        assert!(state.is_waiting_to_retry);
    });
    cx.run_until_parked();

    assert!(catalog.recommended("alice").unwrap().is_none());
}

#[gpui_kit::test]
fn an_incomplete_pick_shows_with_nothing_else_to_show(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.status.albums = RecommendedAlbums::Loading;
        this.finish(state, found(&["r0"], false), today());
        assert_eq!(shown(state), ["r0"]);
        assert_eq!(state.day, None);
    });
}

#[gpui_kit::test]
fn a_failed_pick_does_not_settle_the_day(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.status.albums = on_show(recommended(1));
        this.finish(state, Err("offline".into()), today());
        assert_eq!(shown(state), ["r0"]);
        assert_eq!(state.day, None);
        assert!(state.is_waiting_to_retry);
    });
}

#[gpui_kit::test]
fn an_empty_pick_is_not_kept_and_leaves_what_is_on_show(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("alice", "2000-01-01", &recommended(1))
        .unwrap();
    let recommendations = recommendations(Some(catalog.clone()), cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.status.albums = on_show(recommended(1));
        this.finish(state, found(&[], true), today());
        assert_eq!(shown(state), ["r0"]);
        // Plays need not ask again
        this.renew_in(state);
        assert!(!state.status.is_recommending);
    });
    cx.run_until_parked();

    assert_eq!(
        catalog.recommended("alice").unwrap().unwrap().day,
        "2000-01-01"
    );
}

#[gpui_kit::test]
fn a_library_sync_gives_an_empty_shelf_another_pick(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        this.finish(state, found(&[], true), today());
    });

    recommendations.library_synced();

    assert!(recommendations.status().is_recommending());
}

#[gpui_kit::test]
fn a_library_sync_leaves_the_days_pick_alone(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        this.finish(state, found(&["r0"], true), today());
    });

    recommendations.library_synced();

    assert!(!recommendations.status().is_recommending());
}

#[gpui_kit::test]
fn a_pick_that_went_wrong_is_not_made_again_right_away(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        this.finish(state, Err("offline".into()), today());
    });

    recommendations.renew();
    assert!(!recommendations.status().is_recommending());

    cx.executor().advance_clock(RETRY_AFTER);
    cx.run_until_parked();
    recommendations.renew();
    assert!(recommendations.status().is_recommending());
}

#[gpui_kit::test]
fn retry_does_not_wait(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        this.finish(state, Err("offline".into()), today());
    });

    recommendations.pick();

    update(&recommendations, |_, state| {
        assert!(state.status.is_recommending);
        assert!(!state.is_waiting_to_retry);
    });
}

#[gpui_kit::test]
fn a_pick_that_leaves_albums_on_show_says_why(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.status.albums = on_show(recommended(1));
        let kept = this.finish(state, Err("offline".into()), today());
        assert_eq!(
            kept.as_deref(),
            Some("Couldn't get new recommendations. offline")
        );
        let kept = this.finish(state, found(&["other"], false), today());
        assert!(kept.is_some());
        assert_eq!(shown(state), ["r0"]);

        assert_eq!(this.finish(state, found(&["r1"], true), today()), None);
        assert_eq!(shown(state), ["r1"]);
    });
}

#[gpui_kit::test]
fn a_failure_that_shows_on_the_shelf_needs_no_other_word(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.status.albums = RecommendedAlbums::Loading;
        assert_eq!(this.finish(state, Err("offline".into()), today()), None);
        assert!(matches!(state.status.albums, RecommendedAlbums::Failed(_)));
    });
}

#[gpui_kit::test]
fn only_a_reload_says_why_it_left_the_albums_on_show(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("alice", &today(), &recommended(1))
        .unwrap();
    let recommendations = recommendations(Some(catalog), cx);
    let mut notices = recommendations.notices();
    cx.run_until_parked();

    recommendations.pick();
    crate::testing::wait_for(cx, || !recommendations.status().is_recommending());
    assert!(notices.try_recv().is_err());

    recommendations.reload();
    crate::testing::wait_for(cx, || !recommendations.status().is_recommending());
    let failed = notices.try_recv().unwrap();
    assert!(
        failed
            .message()
            .starts_with("Couldn't get new recommendations. ")
    );
    update(&recommendations, |_, state| {
        assert_eq!(shown(state), ["r0"]);
        assert_eq!(state.day, Some(today()));
    });
}

#[gpui_kit::test]
fn todays_kept_pick_shows_again_with_no_new_pick(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("alice", &today(), &recommended(2))
        .unwrap();
    let recommendations = recommendations(Some(catalog), cx);

    cx.run_until_parked();

    update(&recommendations, |_, state| {
        assert_eq!(shown(state), ["r0", "r1"]);
        assert!(!state.status.is_recommending);
        assert!(!state.is_waiting_to_retry);
    });
}

#[gpui_kit::test]
fn yesterdays_kept_pick_shows_while_a_new_one_is_made(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("alice", "2000-01-01", &recommended(1))
        .unwrap();
    let recommendations = recommendations(Some(catalog), cx);

    cx.run_until_parked();

    update(&recommendations, |_, state| {
        assert_eq!(shown(state), ["r0"]);
        // On its way, or already failed for want of a server
        assert!(state.status.is_recommending || state.is_waiting_to_retry);
    });
}

#[gpui_kit::test]
fn another_users_kept_pick_does_not_show(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog_in(&dir);
    catalog
        .put_recommended("bob", &today(), &recommended(1))
        .unwrap();
    let recommendations = recommendations(Some(catalog), cx);

    cx.run_until_parked();

    update(&recommendations, |_, state| {
        assert!(shown(state).is_empty())
    });
}

#[gpui_kit::test]
fn the_recommendations_stay_for_the_day(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.day = Some(today());
        this.renew_in(state);
        assert!(!state.status.is_recommending);
    });
}

#[gpui_kit::test]
fn another_day_picks_new_recommendations_with_the_old_on_show(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.status.albums = on_show(recommended(1));
        state.day = Some("2000-01-01".to_owned());

        this.renew_in(state);

        assert!(state.status.is_recommending);
        assert_eq!(state.status.albums.albums().len(), 1);
    });
}

#[gpui_kit::test]
fn a_pick_on_its_way_is_not_started_again(cx: &mut TestAppContext) {
    let recommendations = recommendations(None, cx);
    update(&recommendations, |this, state| {
        clear(state);
        state.status.albums = RecommendedAlbums::Failed("before".into());
        state.status.is_recommending = true;

        this.renew_in(state);

        assert!(matches!(state.status.albums, RecommendedAlbums::Failed(_)));
    });
}
