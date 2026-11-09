use gpui_kit::TestAppContext;
use serde_json::json;
use subsonic::Song;

use super::*;
use crate::test_support::session;
use crate::testing::NOWHERE;

fn workspace(settings: &Settings, cx: &mut TestAppContext) -> Workspace {
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let parts = Parts {
        executor,
        settings: settings.clone(),
        store: Err(config::Error::NoHomeDir),
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
