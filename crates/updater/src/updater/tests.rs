use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use executor::testing::TestExecutor;

use super::*;

#[derive(Default)]
struct Scripted {
    found: Mutex<Vec<Result<Option<String>, String>>>,
    installed: Mutex<Vec<Result<Outcome, String>>>,
    looks: AtomicU32,
    installs: AtomicU32,
}

impl Releases for Scripted {
    fn newer_version(&self) -> Result<Option<String>, String> {
        self.looks.fetch_add(1, Ordering::SeqCst);
        self.found.lock().unwrap().remove(0)
    }

    fn install(&self) -> Result<Outcome, String> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        self.installed.lock().unwrap().remove(0)
    }
}

fn scripted(
    found: Vec<Result<Option<String>, String>>,
    installed: Vec<Result<Outcome, String>>,
) -> Arc<Scripted> {
    Arc::new(Scripted {
        found: Mutex::new(found),
        installed: Mutex::new(installed),
        ..Scripted::default()
    })
}

fn updater(releases: &Arc<Scripted>, is_automatic: bool, cx: &TestExecutor) -> Updater {
    let executor = cx.executor();
    Updater::with(executor, releases.clone(), None, is_automatic, None)
}

fn looks(releases: &Scripted) -> u32 {
    releases.looks.load(Ordering::SeqCst)
}

fn installs(releases: &Scripted) -> u32 {
    releases.installs.load(Ordering::SeqCst)
}

fn nine() -> Result<Option<String>, String> {
    Ok(Some("9.0.0".to_owned()))
}

fn installed_nine() -> Result<Outcome, String> {
    Ok(Outcome::Installed("9.0.0".to_owned()))
}

fn ready_nine() -> UpdateState {
    UpdateState::Ready("9.0.0".to_owned())
}

fn available_nine() -> UpdateState {
    UpdateState::Available("9.0.0".to_owned())
}

#[test]
fn automatic_installs_what_it_finds_and_then_looks_no_more() {
    let cx = &TestExecutor::new();
    let releases = scripted(vec![nine()], vec![installed_nine()]);
    let updater = updater(&releases, true, cx);
    cx.run_until_parked();

    assert_eq!(*updater.status().state(), ready_nine());
    updater.check();
    cx.run_until_parked();
    assert_eq!(looks(&releases), 1);
}

#[test]
fn when_not_automatic_it_looks_only_when_asked_and_installs_only_when_asked() {
    let cx = &TestExecutor::new();
    let releases = scripted(vec![nine()], vec![installed_nine()]);
    let updater = updater(&releases, false, cx);
    cx.run_until_parked();
    assert_eq!(looks(&releases), 0);

    updater.check();
    cx.run_until_parked();
    assert_eq!(*updater.status().state(), available_nine());
    assert_eq!(installs(&releases), 0);

    updater.install();
    cx.run_until_parked();
    assert_eq!(*updater.status().state(), ready_nine());
}

#[test]
fn turning_it_off_while_looking_keeps_what_is_found_from_installing() {
    let cx = &TestExecutor::new();
    let releases = scripted(vec![nine()], vec![]);
    let updater = updater(&releases, true, cx);

    updater.check();
    updater.set_automatic(false);
    cx.run_until_parked();

    assert_eq!(*updater.status().state(), available_nine());
    assert_eq!(installs(&releases), 0);
}

#[test]
fn turning_it_on_installs_what_was_already_found() {
    let cx = &TestExecutor::new();
    let releases = scripted(vec![nine()], vec![installed_nine()]);
    let updater = updater(&releases, false, cx);
    updater.check();
    cx.run_until_parked();

    updater.set_automatic(true);
    cx.run_until_parked();

    assert_eq!(looks(&releases), 1);
    assert_eq!(installs(&releases), 1);
}

#[test]
fn a_failure_shows_until_the_next_look_succeeds() {
    let cx = &TestExecutor::new();
    let releases = scripted(vec![Err("offline".to_owned()), Ok(None)], vec![]);
    let updater = updater(&releases, true, cx);
    cx.run_until_parked();
    assert_eq!(updater.status().errors().collect::<Vec<_>>(), ["offline"]);

    updater.check();
    cx.run_until_parked();
    assert_eq!(updater.status().errors().count(), 0);
}

#[test]
fn a_failed_install_can_be_looked_for_again() {
    let cx = &TestExecutor::new();
    let releases = scripted(vec![nine(), nine()], vec![Err("damaged".to_owned())]);
    let updater = updater(&releases, true, cx);
    cx.run_until_parked();
    assert_eq!(
        *updater.status().state(),
        UpdateState::Failed("damaged".to_owned())
    );

    updater.set_automatic(false);
    updater.check();
    cx.run_until_parked();
    assert_eq!(*updater.status().state(), available_nine());
}

#[test]
fn it_looks_again_after_the_interval_while_automatic() {
    let cx = &TestExecutor::new();
    let releases = scripted(vec![Ok(None), Ok(None)], vec![]);
    let _updater = updater(&releases, true, cx);
    cx.run_until_parked();
    assert_eq!(looks(&releases), 1);

    cx.advance_clock(CHECK_INTERVAL);
    cx.run_until_parked();
    assert_eq!(looks(&releases), 2);
}

#[test]
fn the_choice_is_saved() {
    let cx = &TestExecutor::new();
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let releases = scripted(vec![Ok(None)], vec![]);
    let executor = cx.executor();
    let updater = Updater::with(executor, releases.clone(), Some(store.clone()), false, None);

    updater.set_automatic(true);
    cx.run_until_parked();
    assert_eq!(looks(&releases), 1);
    assert!(store.auto_update().unwrap());

    updater.set_automatic(false);
    cx.run_until_parked();
    assert_eq!(looks(&releases), 1);
    assert!(!store.auto_update().unwrap());
}

#[test]
fn each_change_is_published() {
    let cx = &TestExecutor::new();
    let releases = scripted(vec![nine()], vec![]);
    let updater = updater(&releases, false, cx);
    let mut changes = updater.subscribe();

    updater.check();
    assert!(changes.has_changed().unwrap());
    assert_eq!(*changes.borrow_and_update().state(), UpdateState::Checking);
    cx.run_until_parked();
    assert_eq!(*changes.borrow_and_update().state(), available_nine());
}

#[tokio::test(start_paused = true)]
async fn on_tokio_it_looks_again_after_the_interval_while_automatic() {
    let releases = scripted(vec![Ok(None), Ok(None)], vec![]);
    let executor = Executor::tokio(tokio::runtime::Handle::current());
    let updater = Updater::with(executor, releases.clone(), None, true, None);
    let mut changes = updater.subscribe();
    changes
        .wait_for(|status| *status.state() == UpdateState::UpToDate)
        .await
        .unwrap();
    assert_eq!(looks(&releases), 1);

    tokio::time::sleep(CHECK_INTERVAL - Duration::from_secs(1)).await;
    assert_eq!(looks(&releases), 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    changes
        .wait_for(|status| *status.state() == UpdateState::UpToDate && looks(&releases) == 2)
        .await
        .unwrap();
}
