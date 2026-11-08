//! The signed-in connection that the app shell hands to feature views, signing in, and what
//! reads kanji. No GPUI.

mod accounts;
mod covers;
mod reading_book;
mod readings;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use subsonic::{AlbumId, AlbumWithSongs, CoverArtId, ServerInfo};

pub use accounts::{Accounts, Invalid, SignInField, SignedIn};
pub use covers::{answer_cover, cover_image};
pub use reading_book::ReadingBook;
pub use readings::{Readings, ReadingsFailed, ReadingsState, ReadingsStatus};

/// Where cover art is kept on disk, when the system has a cache directory, holding as much as
/// the saved setting says.
pub fn open_covers(store: Result<config::Store, config::Error>) -> Option<catalog::CoverStore> {
    let capacity = match store {
        Ok(store) => store.cover_cache_capacity(),
        // Without a home directory there are no settings to follow
        Err(_) => Ok(None),
    };
    // A setting that cannot be read keeps no more covers rather than the
    // default amount.
    let capacity = capacity.map_or(0, cover_capacity);
    catalog::CoverStore::user(capacity).ok()
}

/// Keeps at most about `capacity` bytes of covers from the next one on, or the
/// default amount for `None`.
pub fn set_cover_capacity(covers: &catalog::CoverStore, capacity: Option<u64>) {
    covers.set_capacity(cover_capacity(capacity));
}

fn cover_capacity(capacity: Option<u64>) -> u64 {
    capacity.unwrap_or(catalog::COVER_CAPACITY)
}

type CoverUrls = HashMap<(CoverArtId, u32), Arc<str>>;

#[derive(Clone)]
pub struct Session {
    client: client::Client,
    username: String,
    server: ServerInfo,
    catalog: Option<catalog::Catalog>,
    scrobbles: Option<catalog::Scrobbles>,
    readings: Option<Readings>,
    cover_urls: Arc<Mutex<CoverUrls>>,
    reading_gate: Arc<Mutex<()>>,
}

impl Session {
    /// `catalog` is the server's cached library; without one, views load
    /// everything from the server. `scrobbles` keeps the user's plays until
    /// the server has them; without it, a play that fails to send is lost.
    pub fn new(
        client: client::Client,
        username: String,
        server: ServerInfo,
        catalog: Option<catalog::Catalog>,
        scrobbles: Option<catalog::Scrobbles>,
    ) -> Self {
        Self {
            client,
            username,
            server,
            catalog,
            scrobbles,
            readings: None,
            cover_urls: Arc::default(),
            reading_gate: Arc::default(),
        }
    }

    /// Without them, lists find text only as written and by its kana.
    pub fn with_readings(mut self, readings: Readings) -> Self {
        self.readings = Some(readings);
        self
    }

    pub fn readings(&self) -> Option<&Readings> {
        self.readings.as_ref()
    }

    /// [`ReadingsStatus::missing_note`] of the session's dictionaries.
    pub fn missing_readings_note(&self) -> Option<&'static str> {
        self.readings()?.status().missing_note()
    }

    /// What reads kanji for building a list's index, when the dictionaries are
    /// there. Each list asks again: they come and go.
    pub fn reading_book(&self) -> Option<ReadingBook> {
        Some(ReadingBook {
            dir: self.readings()?.status().dictionaries()?.to_owned(),
            catalog: self.catalog.clone(),
            gate: self.reading_gate.clone(),
        })
    }

    /// The same image gets the same URL for the whole session. Each new URL
    /// would be a new download and a new decoded copy to an image cache, as
    /// every URL the client builds carries a fresh salt. Never log it: it
    /// carries the user's credentials.
    pub fn cover_art_url(&self, id: &CoverArtId, size: u32) -> Arc<str> {
        let mut urls = self
            .cover_urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        urls.entry((id.clone(), size))
            .or_insert_with(|| self.client.cover_art_url(id, Some(size)).as_str().into())
            .clone()
    }

    pub fn client(&self) -> &client::Client {
        &self.client
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    /// An album and its songs: the saved ones when there are any, otherwise
    /// the server's. Blocks on the saved library, so await it off the UI thread.
    pub async fn album(&self, id: &AlbumId) -> Result<AlbumWithSongs, client::Error> {
        let saved = self
            .catalog()
            .and_then(|catalog| catalog.album(id).ok().flatten());
        match saved {
            Some(saved) => Ok(saved),
            None => self.client().album(id).await,
        }
    }

    pub fn catalog(&self) -> Option<&catalog::Catalog> {
        self.catalog.as_ref()
    }

    pub fn scrobbles(&self) -> Option<&catalog::Scrobbles> {
        self.scrobbles.as_ref()
    }

    pub fn server(&self) -> &ServerInfo {
        &self.server
    }
}

pub fn describe_catalog(error: &catalog::Error) -> String {
    if let Some(error) = error.server_error() {
        describe(error)
    } else if error.is_busy() {
        "Another hiphile is updating the saved library. Try again in a moment.".to_owned()
    } else if error.is_unreadable() {
        "The library saved on this computer can't be read. Choose Refresh Library to fetch it again."
            .to_owned()
    } else {
        "The library couldn't be saved on this computer. Check that the disk has free space, then try again."
            .to_owned()
    }
}

/// What went wrong talking to the server, as a sentence for the user that
/// says what to try next.
pub fn describe(error: &client::Error) -> String {
    match error {
        client::Error::Network(_) => {
            "Couldn't reach the server. Check your connection and try again.".to_owned()
        }
        client::Error::HttpStatus(status) => {
            format!("The server answered with HTTP {status}. Try again later.")
        }
        client::Error::Protocol(subsonic::Error::Server { code, .. })
            if code.is_authentication() =>
        {
            "The server no longer accepts your sign-in. Sign out and sign in again.".to_owned()
        }
        client::Error::Protocol(subsonic::Error::Server { message, .. }) => {
            let message = message.trim().trim_end_matches('.');
            format!("The server refused the request: {message}.")
        }
        client::Error::Protocol(subsonic::Error::InvalidResponse(_)) => {
            "The server sent a response hiphile doesn't understand.".to_owned()
        }
        client::Error::NotAudio(_) => "The server sent something other than audio.".to_owned(),
        client::Error::InvalidServerUrl(_) | client::Error::ServerUrlHasExtras(_) => {
            "The server address is not usable. Sign out and check it.".to_owned()
        }
    }
}
