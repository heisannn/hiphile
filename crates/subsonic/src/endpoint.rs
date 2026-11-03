use url::Url;

use crate::Credentials;

/// Protocol version sent as `v`.
pub const PROTOCOL_VERSION: &str = "1.16.1";
/// Client name sent as `c`.
pub const CLIENT_NAME: &str = "hiphile";

/// One OpenSubsonic method call, before authentication is attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    method: &'static str,
    params: Vec<(&'static str, String)>,
}

impl Endpoint {
    fn new(method: &'static str) -> Self {
        Self {
            method,
            params: Vec::new(),
        }
    }

    fn param(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.params.push((name, value.into()));
        self
    }

    pub fn ping() -> Self {
        Self::new("ping")
    }

    /// One page of albums sorted by name. `size` is capped at 500 by the
    /// protocol.
    pub fn albums_by_name(size: u32, offset: u32) -> Self {
        Self::new("getAlbumList2")
            .param("type", "alphabeticalByName")
            .param("size", size.to_string())
            .param("offset", offset.to_string())
    }

    /// The artist index, sent only if it changed after `since` (milliseconds
    /// since the Unix epoch). Either way the response says when the library
    /// last changed.
    pub fn indexes_modified_since(since: u64) -> Self {
        Self::new("getIndexes").param("ifModifiedSince", since.to_string())
    }

    pub fn scan_status() -> Self {
        Self::new("getScanStatus")
    }

    pub fn album(album: &crate::AlbumId) -> Self {
        Self::new("getAlbum").param("id", album.as_str())
    }

    /// Cover art image, scaled by the server to at most `size` pixels when given.
    pub fn cover_art(cover_art: &crate::CoverArtId, size: Option<u32>) -> Self {
        let endpoint = Self::new("getCoverArt").param("id", cover_art.as_str());
        match size {
            Some(size) => endpoint.param("size", size.to_string()),
            None => endpoint,
        }
    }

    pub fn song(song: &crate::SongId) -> Self {
        Self::new("getSong").param("id", song.as_str())
    }

    /// Songs matching `query`; artists and albums are not requested.
    pub fn search_songs(query: &str, count: u32) -> Self {
        Self::new("search3")
            .param("query", query)
            .param("songCount", count.to_string())
            .param("albumCount", "0")
            .param("artistCount", "0")
    }

    /// One page of every song: OpenSubsonic servers return the whole library
    /// for an empty query, for clients that keep a copy of it.
    pub fn all_songs(count: u32, offset: u32) -> Self {
        Self::new("search3")
            .param("query", "")
            .param("songCount", count.to_string())
            .param("songOffset", offset.to_string())
            .param("albumCount", "0")
            .param("artistCount", "0")
    }

    /// Original file bytes (`format=raw`), so the client decodes the source itself.
    pub fn stream_original(song: &crate::SongId) -> Self {
        Self::new("stream")
            .param("id", song.as_str())
            .param("format", "raw")
    }

    pub fn method(&self) -> &'static str {
        self.method
    }

    /// Absolute URL for `base`, e.g. `https://music.example.com` or a server
    /// mounted under a path prefix such as `https://example.com/navidrome`.
    pub fn url(&self, base: &Url, credentials: &Credentials) -> Url {
        let mut url = base.clone();
        url.path_segments_mut()
            // A base that cannot be a base (e.g. `mailto:`) is rejected when
            // the server address is validated; keep the URL unchanged here.
            .map(|mut segments| {
                segments.pop_if_empty().extend(["rest", self.method]);
            })
            .ok();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("v", PROTOCOL_VERSION)
                .append_pair("c", CLIENT_NAME)
                .append_pair("f", "json");
            for (name, value) in credentials.query_pairs() {
                query.append_pair(name, &value);
            }
            for (name, value) in &self.params {
                query.append_pair(name, value);
            }
        }
        url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(url: &Url, name: &str) -> Option<String> {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }

    #[test]
    fn url_keeps_base_path_prefix() {
        let base = Url::parse("https://example.com/navidrome/").unwrap();
        let url = Endpoint::ping().url(&base, &Credentials::api_key("k"));
        assert_eq!(url.path(), "/navidrome/rest/ping");
    }

    #[test]
    fn url_carries_protocol_and_auth_params() {
        let base = Url::parse("https://music.example.com").unwrap();
        let song = crate::SongId::new("s1");
        let url = Endpoint::stream_original(&song).url(&base, &Credentials::api_key("k"));
        assert_eq!(url.path(), "/rest/stream");
        assert_eq!(query(&url, "v").as_deref(), Some(PROTOCOL_VERSION));
        assert_eq!(query(&url, "f").as_deref(), Some("json"));
        assert_eq!(query(&url, "apiKey").as_deref(), Some("k"));
        assert_eq!(query(&url, "id").as_deref(), Some("s1"));
        assert_eq!(query(&url, "format").as_deref(), Some("raw"));
    }
}
