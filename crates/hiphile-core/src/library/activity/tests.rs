use executor::testing::TestExecutor;
use serde_json::json;

use super::*;
use crate::test_support::session_saving_into;
use crate::testing::{NOWHERE, server_answering, wait_for};

#[test]
fn a_count_with_a_total_says_how_far_along_it_is() {
    let status = ActivityStatus::of_sync(SyncStage::Songs, 12_000, Some(48_000));
    assert_eq!(status.count().as_deref(), Some("12000 of 48000"));
    assert_eq!(status.percent(), Some(25.));
}

#[test]
fn a_count_without_a_total_just_counts() {
    let status = ActivityStatus::of_sync(SyncStage::Albums, 500, None);
    assert_eq!(status.count().as_deref(), Some("500"));
    assert_eq!(status.percent(), None);
}

#[test]
fn nothing_saved_and_no_total_says_no_number() {
    assert_eq!(
        ActivityStatus::of_sync(SyncStage::Artists, 0, None).count(),
        None
    );
}

#[test]
fn a_total_that_proved_too_low_is_dropped() {
    let status = ActivityStatus::of_sync(SyncStage::Songs, 600, Some(500));
    assert_eq!(status.count().as_deref(), Some("600"));
    assert_eq!(status.percent(), None);
}

#[test]
fn a_download_says_how_far_along_it_is_in_percent() {
    let status = ActivityStatus::of_download(&ReadingsState::Downloading(0.424)).unwrap();
    assert_eq!(status.count().as_deref(), Some("42%"));
    assert_eq!(status.percent(), Some(42.));
}

#[test]
fn a_download_that_just_started_still_shows() {
    let status = ActivityStatus::of_download(&ReadingsState::Downloading(0.)).unwrap();
    assert_eq!(status.count().as_deref(), Some("0%"));
    assert_eq!(status.percent(), Some(0.));
}

#[test]
fn only_a_download_under_way_is_a_download() {
    for state in [
        ReadingsState::Off,
        ReadingsState::Checking,
        ReadingsState::Ready("dir".into()),
        ReadingsState::Failed("no".into()),
    ] {
        assert_eq!(ActivityStatus::of_download(&state), None);
    }
}

#[test]
fn the_notice_goes_up_once_and_comes_down_when_quiet() {
    let mut notice = Notice::default();
    assert_eq!(notice.follow(false), None);
    assert_eq!(notice.follow(true), Some(NoticeChange::Show));
    assert_eq!(notice.follow(true), None);
    assert_eq!(notice.follow(false), Some(NoticeChange::Hide));
    assert_eq!(notice.follow(false), None);
}

#[test]
fn a_notice_taken_down_can_go_up_again() {
    let mut notice = Notice::default();
    notice.follow(true);
    notice.follow(false);
    // Taking it down closes it too, which is not the user closing it
    notice.closed();
    assert_eq!(notice.follow(true), Some(NoticeChange::Show));
}

#[test]
fn a_notice_the_user_closed_stays_closed_until_the_library_goes_quiet() {
    let mut notice = Notice::default();
    notice.follow(true);
    notice.closed();

    assert_eq!(notice.follow(true), None);

    assert_eq!(notice.follow(false), None);
    assert_eq!(notice.follow(true), Some(NoticeChange::Show));
}

/// An activity following a library whose server is at `url`.
fn activity(
    catalog: Option<catalog::Catalog>,
    url: &str,
    cx: &TestExecutor,
) -> (LibraryActivity, LibrarySync) {
    // The sync's requests wake the test from the network's own thread
    cx.allow_parking();
    let executor = cx.executor();
    let session = session_saving_into(catalog, url);
    let sync = LibrarySync::new(executor.clone(), session.clone());
    (LibraryActivity::new(executor, session, &sync), sync)
}

fn start_reading(activity: &LibraryActivity) {
    activity.0.state.update(|state| {
        state.is_reading = true;
        state.reading_id += 1;
        let reading_id = state.reading_id;
        activity.reading_progress(state, reading_id, 0, 10);
    });
}

#[test]
fn a_reading_shows_once_it_proves_slow() {
    let cx = &TestExecutor::new();
    let (activity, _sync) = activity(None, NOWHERE, cx);

    start_reading(&activity);
    cx.run_until_parked();
    assert_eq!(activity.status(), ActivitySnapshot::default());

    cx.advance_clock(SLOW_READING);
    cx.run_until_parked();
    let status = activity.status();
    assert_eq!(
        status.shown().map(ActivityStatus::label),
        Some("Preparing romaji search")
    );
    assert!(status.is_notice_shown());

    activity
        .0
        .state
        .update(|state| activity.finish_reading(state));
    let status = activity.status();
    assert!(!status.is_notice_shown());
    // What it said stays on the notice as it leaves
    assert!(status.shown().is_some());
}

#[test]
fn a_reading_that_ends_quickly_never_shows() {
    let cx = &TestExecutor::new();
    let (activity, _sync) = activity(None, NOWHERE, cx);

    start_reading(&activity);
    activity
        .0
        .state
        .update(|state| activity.finish_reading(state));
    cx.advance_clock(SLOW_READING);
    cx.run_until_parked();

    assert_eq!(activity.status(), ActivitySnapshot::default());
}

#[test]
fn a_notice_the_user_closed_stays_down_while_busy() {
    let cx = &TestExecutor::new();
    let (activity, _sync) = activity(None, NOWHERE, cx);
    start_reading(&activity);
    cx.advance_clock(SLOW_READING);
    cx.run_until_parked();

    activity.notice_closed();
    assert!(!activity.status().is_notice_shown());
    start_reading(&activity);
    assert!(!activity.status().is_notice_shown());
}

#[test]
fn a_sync_shows_until_it_ends() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let url = server_answering(json!({
        "albumList2": { "album": [{ "id": "a", "name": "A" }] },
        "artists": { "index": [] },
        "searchResult3": { "song": [] },
        "scanStatus": { "scanning": false },
    }));
    let (activity, sync) = activity(Some(catalog), &url, cx);

    wait_for(cx, || !sync.status().is_running());

    let status = activity.status();
    assert!(
        status
            .shown()
            .is_some_and(|shown| shown.label().starts_with("Syncing"))
    );
    assert!(!status.is_notice_shown());
}

#[test]
fn a_sync_under_way_before_the_activity_follows_it_shows() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let url = server_answering(json!({
        "albumList2": { "album": [{ "id": "a", "name": "A" }] },
        "artists": { "index": [] },
        "searchResult3": { "song": [] },
        "scanStatus": { "scanning": false },
    }));
    cx.allow_parking();
    let executor = cx.executor();
    let sync = LibrarySync::new(executor.clone(), session_saving_into(Some(catalog), &url));
    // One step at a time, to stop between two stages
    while sync.status().progress().is_none() {
        assert!(sync.status().is_running(), "the sync ended unseen");
        if !cx.tick() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    let activity = LibraryActivity::new(executor, session_saving_into(None, &url), &sync);

    let status = activity.status();
    assert!(status.is_notice_shown());
    assert!(
        status
            .shown()
            .is_some_and(|shown| shown.label().starts_with("Syncing"))
    );
    wait_for(cx, || !sync.status().is_running());
}
