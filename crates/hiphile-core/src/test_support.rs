//! Fake servers and a session on them, for the tests of the core and of the views that follow it.

mod favorites_server;
mod playlists_server;

pub use favorites_server::{FavoritesServer, song_json};
pub use playlists_server::{PlaylistsServer, Request};

use serde_json::json;
use subsonic::Credentials;

/// Signed in to `url` as alice, with no saved library.
pub fn session(url: &str) -> session::Session {
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new(url, Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        None,
        None,
    )
}
