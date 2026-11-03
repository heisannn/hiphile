//! Connection state for the GUI.
//!
//! Installs [`net`] as GPUI's HTTP client, so `img(url)` cover art shares the
//! API's transport, and names the signed-in connection that the app shell
//! hands to feature views.

mod http;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use gpui_kit::{App, SharedString};
use subsonic::{CoverArtId, ServerInfo};

/// Installs the shared transport. Call once at startup, after `gpui_kit::init`.
pub fn init(net: net::Client, cx: &mut App) {
    cx.set_http_client(Arc::new(http::NetHttpClient::new(net)));
}

/// A server the user has signed in to.
#[derive(Clone)]
pub struct Session {
    client: client::Client,
    server: ServerInfo,
    cover_urls: Arc<Mutex<HashMap<(CoverArtId, u32), SharedString>>>,
}

impl Session {
    pub fn new(client: client::Client, server: ServerInfo) -> Self {
        Self {
            client,
            server,
            cover_urls: Arc::default(),
        }
    }

    /// The URL of a cover image `size` pixels across.
    ///
    /// The same image gets the same URL for the whole session. Each new URL
    /// would be a new download and a new decoded copy to an image cache, as
    /// every URL the client builds carries a fresh salt. Never log it: it
    /// carries the user's credentials.
    pub fn cover_art_url(&self, id: &CoverArtId, size: u32) -> SharedString {
        let mut urls = self
            .cover_urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        urls.entry((id.clone(), size))
            .or_insert_with(|| self.client.cover_art_url(id, Some(size)).to_string().into())
            .clone()
    }

    pub fn client(&self) -> &client::Client {
        &self.client
    }

    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    /// The server's host, for showing which server is connected.
    pub fn host(&self) -> &str {
        self.client
            .server()
            .base_url()
            .host_str()
            .unwrap_or_default()
    }
}
