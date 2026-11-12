//! Fake servers and a session on them, for the tests of the core and of the views that follow it.

mod artists_server;
mod favorites_server;
mod home_server;
mod library_server;
mod playlists_server;
mod songs_server;

pub use artists_server::ArtistsServer;
pub use favorites_server::{FavoritesServer, song_json};
pub use home_server::HomeServer;
pub use library_server::LibraryServer;
pub use playlists_server::{PlaylistsServer, Request};
pub use songs_server::SongsServer;

use std::sync::Arc;

use serde_json::json;
use subsonic::Credentials;

use crate::Shown;

/// Signed in to `url` as alice, with no saved library.
pub fn session(url: &str) -> session::Session {
    session_saving_into(None, url)
}

/// Signed in to `url` as alice, saving what the server gives into `catalog`.
pub fn session_saving_into(catalog: Option<catalog::Catalog>, url: &str) -> session::Session {
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new(url, Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        catalog,
        None,
    )
}

/// Every one of `items`, in their order, as a list would show them without a query.
pub fn shown_in_order<T>(items: impl Into<Arc<[T]>>) -> Shown<T> {
    let items = items.into();
    let visible = (0..items.len()).collect();
    Shown::new(items, visible)
}
