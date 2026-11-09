use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use executor::{BoxFuture, Detach, Spawned, Spawner};
use gpui_kit::TestAppContext;
use serde_json::json;
use subsonic::Song;

use super::*;
use crate::test_support::{FavoritesServer, PlaylistsServer, session, session_saving_into};
use crate::testing::{NOWHERE, wait_for};

fn workspace(settings: &Settings, cx: &mut TestAppContext) -> Workspace {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    workspace_on(executor, settings)
}

fn workspace_on(executor: Executor, settings: &Settings) -> Workspace {
    workspace_with(executor, settings, session(NOWHERE))
}

fn workspace_with(executor: Executor, settings: &Settings, session: Session) -> Workspace {
    let parts = Parts {
        executor,
        settings: settings.clone(),
        store: Err(config::Error::NoHomeDir),
        cache: None,
        covers: None,
        song_cache: None,
    };
    Workspace::new(session, config::Profile::new(NOWHERE, "alice"), parts)
}

/// A workspace whose server is at `url`, once what it loads on opening has failed or arrived.
fn settled_workspace_at(url: &str, cx: &mut TestAppContext) -> Workspace {
    // The answers come from the network's own thread
    cx.executor().allow_parking();
    let settings = settings(cx);
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let workspace = workspace_with(executor, &settings, session(url));
    settle(&workspace, cx);
    workspace
}

/// Waits for what the workspace loads to fail or arrive. A test that ends with a request on its
/// way can drop the last network client on the network's own thread, which aborts.
fn settle(workspace: &Workspace, cx: &mut TestAppContext) {
    fn is_loading<T>(shelf: &crate::Shelf<T>) -> bool {
        matches!(shelf, crate::Shelf::Loading)
    }
    wait_for(cx, || {
        let shelves = workspace.shelves().status();
        !workspace.library().status().is_running()
            && !workspace.favorites().snapshot().is_reloading()
            && workspace.playlists().snapshot().list() != &crate::playlists::PlaylistList::Loading
            && !workspace.recommendations().status().is_recommending()
            && !is_loading(shelves.recently_played())
            && !is_loading(shelves.most_played())
            && !is_loading(shelves.new_albums())
    });
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
fn the_player_plays_with_the_dsp_settings_from_the_start_and_as_they_change(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let settings = settings(cx);
    settings.dsp().set_crossfeed(true);
    let workspace = workspace(&settings, cx);
    assert_eq!(
        workspace.player().dsp(),
        settings.dsp().snapshot().settings()
    );
    assert!(workspace.player().dsp().crossfeed().is_some());

    settings.dsp().set_balance(-3.0);
    cx.run_until_parked();
    assert_eq!(
        workspace.player().dsp().balance(),
        audio::Balance::from_db(-3.0)
    );
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

#[gpui_kit::test]
fn the_lyrics_follow_the_song_the_player_is_on(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let settings = settings(cx);
    let workspace = workspace(&settings, cx);
    let song: Song = serde_json::from_value(json!({ "id": "a", "title": "A" })).unwrap();

    workspace.player().fill_queue(&[song], 0);
    cx.run_until_parked();
    assert_eq!(workspace.lyrics().song(), Some(subsonic::SongId::new("a")));

    workspace.player().fill_queue(&[], 0);
    cx.run_until_parked();
    assert_eq!(workspace.lyrics().song(), None);
}

fn play_dsd(settings: &Settings) {
    let dac = audio::OutputDevice::listed("coreaudio:dac", "DAC", &[44_100], &[176_400]);
    settings.audio().pretend_listed(vec![dac]);
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

#[gpui_kit::test]
fn the_artists_and_the_songs_are_read_once_their_screens_show(cx: &mut TestAppContext) {
    let workspace = settled_workspace_at(NOWHERE, cx);
    assert!(!workspace.artists().status().is_refreshing());
    assert!(!workspace.songs().status().is_loading());

    workspace.showing(Destination::Artists);
    workspace.showing(Destination::Songs);

    assert!(workspace.artists().status().is_refreshing());
    assert!(workspace.songs().status().is_loading());
    wait_for(cx, || {
        !workspace.artists().status().is_refreshing() && !workspace.songs().status().is_loading()
    });
}

#[gpui_kit::test]
fn the_playlists_are_read_again_each_time_they_show(cx: &mut TestAppContext) {
    let server = PlaylistsServer::new(&[("p1", "Mix", &[])]);
    let workspace = settled_workspace_at(server.url(), cx);
    // So that the end of the read shows
    server.refuse("getPlaylists");

    workspace.showing(Destination::Albums);
    workspace.showing(Destination::Playlists);

    wait_for(cx, || {
        workspace.playlists().snapshot().refresh_error().is_some()
    });
    assert_eq!(server.requests("getPlaylists").len(), 2);
}

#[gpui_kit::test]
fn the_favorites_are_read_again_each_time_they_show(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a"]);
    let workspace = settled_workspace_at(server.url(), cx);

    workspace.showing(Destination::Home);
    workspace.showing(Destination::Favorites);

    assert!(workspace.favorites().snapshot().is_reloading());
    settle(&workspace, cx);
    assert_eq!(server.reads(), 2);
}

#[gpui_kit::test]
fn a_refresh_off_the_playlists_and_the_favorites_syncs_the_library(cx: &mut TestAppContext) {
    let workspace = settled_workspace_at(NOWHERE, cx);

    workspace.refresh(Destination::Albums);

    assert!(workspace.library().status().is_running());
    // They failed to load, and every screen's stars wait on them
    assert!(workspace.favorites().snapshot().is_reloading());
    settle(&workspace, cx);
}

#[gpui_kit::test]
fn the_favorites_screen_refreshes_only_the_favorites(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a"]);
    let workspace = settled_workspace_at(server.url(), cx);

    workspace.refresh(Destination::Favorites);

    assert!(!workspace.library().status().is_running());
    assert!(workspace.favorites().snapshot().is_reloading());
    settle(&workspace, cx);
}

#[gpui_kit::test]
fn the_playlists_screen_refreshes_the_playlists_and_failed_favorites(cx: &mut TestAppContext) {
    let server = PlaylistsServer::new(&[("p1", "Mix", &[])]);
    let workspace = settled_workspace_at(server.url(), cx);
    assert!(matches!(
        workspace.favorites().snapshot().load(),
        crate::favorites::FavoritesLoad::Failed(_)
    ));
    server.refuse("getPlaylists");

    workspace.refresh(Destination::Playlists);

    assert!(!workspace.library().status().is_running());
    assert!(workspace.favorites().snapshot().is_reloading());
    wait_for(cx, || {
        workspace.playlists().snapshot().refresh_error().is_some()
    });
    settle(&workspace, cx);
}

#[gpui_kit::test]
fn coming_back_to_the_app_on_another_day_picks_again(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let catalog = catalog::Catalog::open(dir.path()).unwrap();
    let album: subsonic::Album =
        serde_json::from_value(json!({ "id": "r0", "name": "r0" })).unwrap();
    catalog
        .put_recommended("alice", "2000-01-01", &[album])
        .unwrap();
    let settings = settings(cx);
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let session = session_saving_into(Some(catalog), NOWHERE);
    let workspace = workspace_with(executor, &settings, session);
    settle(&workspace, cx);
    // The pick on opening went wrong, which may be tried again in a while
    cx.executor().advance_clock(Duration::from_secs(60 * 60));
    cx.run_until_parked();
    let is_recommending = || workspace.recommendations().status().is_recommending();
    assert!(!is_recommending());

    workspace.app_activated();

    assert!(is_recommending());
    settle(&workspace, cx);
}

#[gpui_kit::test]
fn what_failed_as_the_workspace_opened_is_told_to_a_host_that_follows_later(
    cx: &mut TestAppContext,
) {
    let workspace = settled_workspace_at(NOWHERE, cx);
    cx.run_until_parked();

    let earlier = workspace.alerts().earlier;

    assert_eq!(earlier.len(), 2);
    for screen in [Destination::Albums, Destination::Favorites] {
        assert!(earlier.iter().any(|alert| alert.told_on == Some(screen)));
    }
    assert!(
        earlier
            .iter()
            .all(|alert| alert.kind == alerts::AlertKind::Error)
    );
}

#[gpui_kit::test]
fn songs_left_out_as_instrumentals_are_skipped_are_told(cx: &mut TestAppContext) {
    let workspace = settled_workspace_at(NOWHERE, cx);
    let mut alerts = workspace.alerts();
    workspace.player().set_skip_instrumentals(true);

    assert!(!workspace.player().play(&[instrumental()], 0));
    cx.run_until_parked();

    let told = alerts.later.try_recv().unwrap();
    assert_eq!(told.kind, alerts::AlertKind::Info);
    assert!(told.message.starts_with("Instrumentals are being skipped."));
    assert_eq!(told.told_on, None);
}
