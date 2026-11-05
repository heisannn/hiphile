use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use gpui_kit::{Entity, TestAppContext};

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

fn updater(
    releases: &Arc<Scripted>,
    is_automatic: bool,
    cx: &mut TestAppContext,
) -> Entity<Updater> {
    let releases = releases.clone();
    cx.new(|cx| Updater::with(releases, None, is_automatic, None, cx))
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

#[gpui_kit::test]
fn automatic_installs_what_it_finds_and_then_looks_no_more(cx: &mut TestAppContext) {
    let releases = scripted(vec![nine()], vec![installed_nine()]);
    let updater = updater(&releases, true, cx);
    cx.run_until_parked();

    updater.read_with(cx, |it, _| {
        assert_eq!(*it.state(), UpdateState::Ready("9.0.0".into()));
    });
    updater.update(cx, |it, cx| it.check(cx));
    cx.run_until_parked();
    assert_eq!(looks(&releases), 1);
}

#[gpui_kit::test]
fn when_not_automatic_it_looks_only_when_asked_and_installs_only_when_asked(
    cx: &mut TestAppContext,
) {
    let releases = scripted(vec![nine()], vec![installed_nine()]);
    let updater = updater(&releases, false, cx);
    cx.run_until_parked();
    assert_eq!(looks(&releases), 0);

    updater.update(cx, |it, cx| it.check(cx));
    cx.run_until_parked();
    updater.read_with(cx, |it, _| {
        assert_eq!(*it.state(), UpdateState::Available("9.0.0".into()));
    });
    assert_eq!(installs(&releases), 0);

    updater.update(cx, |it, cx| it.install(cx));
    cx.run_until_parked();
    updater.read_with(cx, |it, _| {
        assert_eq!(*it.state(), UpdateState::Ready("9.0.0".into()));
    });
}

#[gpui_kit::test]
fn turning_it_off_while_looking_keeps_what_is_found_from_installing(cx: &mut TestAppContext) {
    let releases = scripted(vec![nine()], vec![]);
    let updater = updater(&releases, true, cx);

    updater.update(cx, |it, cx| {
        it.check(cx);
        it.set_automatic(false, cx);
    });
    cx.run_until_parked();

    updater.read_with(cx, |it, _| {
        assert_eq!(*it.state(), UpdateState::Available("9.0.0".into()));
    });
    assert_eq!(installs(&releases), 0);
}

#[gpui_kit::test]
fn turning_it_on_installs_what_was_already_found(cx: &mut TestAppContext) {
    let releases = scripted(vec![nine()], vec![installed_nine()]);
    let updater = updater(&releases, false, cx);
    updater.update(cx, |it, cx| it.check(cx));
    cx.run_until_parked();

    updater.update(cx, |it, cx| it.set_automatic(true, cx));
    cx.run_until_parked();

    assert_eq!(looks(&releases), 1);
    assert_eq!(installs(&releases), 1);
}

#[gpui_kit::test]
fn a_failure_shows_until_the_next_look_succeeds(cx: &mut TestAppContext) {
    let releases = scripted(vec![Err("offline".to_owned()), Ok(None)], vec![]);
    let updater = updater(&releases, true, cx);
    cx.run_until_parked();
    updater.read_with(cx, |it, _| {
        assert_eq!(
            it.errors().collect::<Vec<_>>(),
            [&SharedString::from("offline")]
        );
    });

    updater.update(cx, |it, cx| it.check(cx));
    cx.run_until_parked();
    updater.read_with(cx, |it, _| assert_eq!(it.errors().count(), 0));
}

#[gpui_kit::test]
fn a_failed_install_can_be_looked_for_again(cx: &mut TestAppContext) {
    let releases = scripted(vec![nine(), nine()], vec![Err("damaged".to_owned())]);
    let updater = updater(&releases, true, cx);
    cx.run_until_parked();
    updater.read_with(cx, |it, _| {
        assert_eq!(*it.state(), UpdateState::Failed("damaged".into()));
    });

    updater.update(cx, |it, cx| {
        it.set_automatic(false, cx);
        it.check(cx);
    });
    cx.run_until_parked();
    updater.read_with(cx, |it, _| {
        assert_eq!(*it.state(), UpdateState::Available("9.0.0".into()));
    });
}

#[gpui_kit::test]
fn it_looks_again_after_the_interval_while_automatic(cx: &mut TestAppContext) {
    let releases = scripted(vec![Ok(None), Ok(None)], vec![]);
    let _updater = updater(&releases, true, cx);
    cx.run_until_parked();
    assert_eq!(looks(&releases), 1);

    cx.executor().advance_clock(CHECK_INTERVAL);
    cx.run_until_parked();
    assert_eq!(looks(&releases), 2);
}

#[gpui_kit::test]
fn the_choice_is_saved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = config::Store::at(dir.path());
    let releases = scripted(vec![Ok(None)], vec![]);
    let updater = {
        let releases = releases.clone();
        let store = store.clone();
        cx.new(|cx| Updater::with(releases, Some(store), false, None, cx))
    };

    updater.update(cx, |it, cx| it.set_automatic(true, cx));
    cx.run_until_parked();
    assert_eq!(looks(&releases), 1);
    assert!(store.auto_update().unwrap());

    updater.update(cx, |it, cx| it.set_automatic(false, cx));
    cx.run_until_parked();
    assert_eq!(looks(&releases), 1);
    assert!(!store.auto_update().unwrap());
}
