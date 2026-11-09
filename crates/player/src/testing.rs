//! What the tests of several views share.

use gpui_kit::{App, Context, Task, Window};
use lyrics::CurrentLyrics;
use serde_json::json;
use subsonic::Credentials;

use crate::Player;

/// Signed in to a server that cannot be reached.
pub(crate) fn session() -> session::Session {
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new("http://127.0.0.1:9", Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        None,
        None,
    )
}

/// A player that plays nothing, so nothing reaches a device or the server.
pub(crate) fn idle_core(cx: &App) -> playback::Player {
    playback::Player::new(
        gpui_host::executor(cx),
        session(),
        Err(config::Error::NoHomeDir),
        audio::OutputSettings::default(),
        None,
    )
}

pub(crate) fn idle_player(window: &mut Window, cx: &mut Context<Player>) -> Player {
    Player::new(idle_core(cx), session(), window, cx)
}

/// Tells `lyrics` the song `core` is on, as the workspace does, for as long as the task is kept.
pub(crate) fn follow_songs(core: &playback::Player, lyrics: &CurrentLyrics, cx: &App) -> Task<()> {
    let mut changes = core.subscribe();
    let song = changes.borrow().current_song().cloned();
    lyrics.set_song(song.as_ref());
    let lyrics = lyrics.clone();
    cx.background_executor().spawn(async move {
        while changes.changed().await.is_ok() {
            let song = changes.borrow_and_update().current_song().cloned();
            lyrics.set_song(song.as_ref());
        }
    })
}
