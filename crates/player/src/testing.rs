//! What the tests of several views share.

use gpui_kit::{App, Context, Window};
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
