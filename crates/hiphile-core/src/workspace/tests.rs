use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use executor::{BoxFuture, Detach, Spawned, Spawner};
use gpui_kit::TestAppContext;
use serde_json::json;
use subsonic::Song;

use super::*;
use crate::test_support::session;
use crate::testing::NOWHERE;

fn workspace(settings: &Settings, cx: &mut TestAppContext) -> Workspace {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    workspace_on(executor, settings)
}

fn workspace_on(executor: Executor, settings: &Settings) -> Workspace {
    let parts = Parts {
        executor,
        settings: settings.clone(),
        store: Err(config::Error::NoHomeDir),
        cache: None,
        covers: None,
        song_cache: None,
    };
    Workspace::new(
        session(NOWHERE),
        config::Profile::new(NOWHERE, "alice"),
        parts,
    )
}

fn settings(cx: &mut TestAppContext) -> Settings {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    Settings::new(executor, Err(config::Error::NoHomeDir))
}

fn instrumental() -> Song {
    serde_json::from_value(json!({ "id": "i", "title": "Theme (Instrumental)", "duration": 100 }))
        .unwrap()
}

#[gpui_kit::test]
fn the_player_skips_instrumentals_once_the_setting_says_so(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let settings = settings(cx);
    let workspace = workspace(&settings, cx);

    settings.queue().set_skip_instrumentals(true);
    cx.run_until_parked();

    assert!(!workspace.player().play(&[instrumental()], 0));
    assert!(!workspace.player().snapshot().has_song());
}

#[gpui_kit::test]
fn the_skipping_on_at_signing_in_holds_from_the_start(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let settings = settings(cx);
    settings.queue().set_skip_instrumentals(true);

    let workspace = workspace(&settings, cx);

    assert!(!workspace.player().play(&[instrumental()], 0));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_lyrics_keep_to_the_chosen_highlight(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let settings = settings(cx);
    settings
        .lyrics()
        .set_highlight(config::LyricsHighlight::ByLine);
    let workspace = workspace(&settings, cx);
    assert_eq!(
        workspace.lyrics().snapshot().highlight(),
        config::LyricsHighlight::ByLine
    );

    settings
        .lyrics()
        .set_highlight(config::LyricsHighlight::ByWord);
    cx.run_until_parked();

    assert_eq!(
        workspace.lyrics().snapshot().highlight(),
        config::LyricsHighlight::ByWord
    );
}

fn play_dsd(settings: &Settings) {
    settings.audio().set_device("coreaudio:dac");
    settings.audio().turn_dsd_on();
}

#[gpui_kit::test]
fn the_player_plays_on_the_output_the_settings_choose(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let settings = settings(cx);
    let workspace = workspace(&settings, cx);
    assert!(!workspace.player().snapshot().is_volume_locked());

    play_dsd(&settings);
    cx.run_until_parked();

    assert!(workspace.player().snapshot().is_volume_locked());
}

type OnCall = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;

/// Calls `on_call` once, as the `at`th task is spawned.
struct Interrupting {
    inner: Executor,
    calls: AtomicUsize,
    at: usize,
    on_call: OnCall,
}

struct Kept(executor::Task<()>);

impl Detach for Kept {
    fn detach(self: Box<Self>) {
        self.0.detach();
    }
}

impl Interrupting {
    fn count(&self) {
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at
            && let Some(on_call) = self.on_call.lock().unwrap().take()
        {
            on_call();
        }
    }
}

impl Spawner for Interrupting {
    fn spawn(&self, future: BoxFuture<'static, ()>) -> Spawned {
        self.count();
        Spawned::new(Kept(self.inner.spawn(future)))
    }

    fn spawn_blocking(&self, work: Box<dyn FnOnce() + Send>) -> Spawned {
        self.count();
        Spawned::new(Kept(self.inner.spawn_blocking(work)))
    }

    fn timer(&self, duration: Duration) -> BoxFuture<'static, ()> {
        Box::pin(self.inner.timer(duration))
    }
}

#[gpui_kit::test]
fn an_output_chosen_while_the_workspace_is_made_reaches_the_player(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    // Each task the workspace starts is a moment at which the user may choose
    for at in 1..=40 {
        let settings = settings(cx);
        let on_call: OnCall = Arc::new(Mutex::new(Some(Box::new({
            let settings = settings.clone();
            move || play_dsd(&settings)
        }))));
        let interrupting = Interrupting {
            inner: cx.update(|cx| gpui_host::executor(cx)),
            calls: AtomicUsize::new(0),
            at,
            on_call: on_call.clone(),
        };
        let workspace = workspace_on(Executor::new(interrupting), &settings);
        cx.run_until_parked();

        if on_call.lock().unwrap().is_none() {
            assert!(
                workspace.player().snapshot().is_volume_locked(),
                "chosen as task {at} started"
            );
        }
        drop(workspace);
        cx.run_until_parked();
    }
}
