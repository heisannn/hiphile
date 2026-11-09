use gpui_kit::TestAppContext;

use super::*;
use crate::test_support::session;
use crate::testing::{NOWHERE, wait_for};

fn executor(cx: &mut TestAppContext) -> Executor {
    // The failures come from the network's own thread
    cx.executor().allow_parking();
    cx.update(|cx| gpui_host::executor(cx))
}

fn alert(message: &str) -> Alert {
    Alert {
        kind: AlertKind::Info,
        message: message.to_owned(),
        told_on: None,
    }
}

fn messages(alerts: &[Alert]) -> Vec<&str> {
    alerts.iter().map(|alert| alert.message.as_str()).collect()
}

fn later(alerts: &mut Alerts) -> Vec<Alert> {
    std::iter::from_fn(|| alerts.later.try_recv().ok()).collect()
}

#[test]
fn a_host_that_follows_hears_each_alert_once_whenever_it_came() {
    let collector = Collector::new();
    collector.tell(alert("before"));

    let mut alerts = collector.follow();
    collector.tell(alert("after"));

    assert_eq!(messages(&alerts.earlier), ["before"]);
    assert_eq!(messages(&later(&mut alerts)), ["after"]);
}

#[test]
fn only_the_last_alerts_are_kept_for_a_host_that_follows_late() {
    let collector = Collector::new();
    for ix in 0..=KEPT {
        collector.tell(alert(&ix.to_string()));
    }

    let alerts = collector.follow();

    assert_eq!(alerts.earlier.len(), KEPT);
    assert_eq!(alerts.earlier[0].message, "1");
}

#[gpui_kit::test]
fn a_sync_that_failed_before_it_was_followed_is_still_told(cx: &mut TestAppContext) {
    let executor = executor(cx);
    let sync = LibrarySync::new(executor.clone(), session(NOWHERE));
    wait_for(cx, || !sync.status().is_running());
    let collector = Collector::new();

    let _route = follow_library(&executor, &sync, &collector);
    cx.run_until_parked();

    let alerts = collector.follow();
    assert_eq!(alerts.earlier.len(), 1);
    let told = &alerts.earlier[0];
    assert!(
        told.message
            .starts_with("Couldn't refresh the library. Couldn't reach")
    );
    assert_eq!(told.told_on, Some(Destination::Albums));
}

#[gpui_kit::test]
fn the_first_sync_tells_of_its_failure_when_it_comes(cx: &mut TestAppContext) {
    let executor = executor(cx);
    let sync = LibrarySync::new(executor.clone(), session(NOWHERE));
    let collector = Collector::new();
    let _route = follow_library(&executor, &sync, &collector);

    wait_for(cx, || !sync.status().is_running());
    cx.run_until_parked();

    assert_eq!(collector.follow().earlier.len(), 1);
}

#[gpui_kit::test]
fn favorites_that_failed_before_they_were_followed_are_still_told(cx: &mut TestAppContext) {
    let executor = executor(cx);
    let favorites = FavoritesState::new(executor.clone(), session(NOWHERE).client().clone());
    wait_for(cx, || {
        matches!(favorites.snapshot().load(), FavoritesLoad::Failed(_))
    });
    let collector = Collector::new();

    let _route = follow_favorites(&executor, &favorites, &collector);
    favorites.refresh();
    wait_for(cx, || !favorites.snapshot().is_reloading());
    cx.run_until_parked();

    let earlier = collector.follow().earlier;
    assert_eq!(earlier.len(), 2);
    assert!(earlier[0].message.starts_with("Couldn't load favorites. "));
    assert!(
        earlier[0]
            .message
            .ends_with(" Choose Refresh Library to try again.")
    );
    assert_eq!(earlier[0].told_on, Some(Destination::Favorites));
}
