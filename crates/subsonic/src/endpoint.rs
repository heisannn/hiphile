use url::Url;

use crate::{CoverArtId, Credentials};

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

    /// The albums added to the library last, newest first.
    pub fn newest_albums(size: u32) -> Self {
        Self::new("getAlbumList2")
            .param("type", "newest")
            .param("size", size.to_string())
    }

    /// The albums played last by the user, most recent first.
    pub fn recently_played_albums(size: u32) -> Self {
        Self::new("getAlbumList2")
            .param("type", "recent")
            .param("size", size.to_string())
    }

    /// The albums the user played most, most played first.
    pub fn frequent_albums(size: u32) -> Self {
        Self::new("getAlbumList2")
            .param("type", "frequent")
            .param("size", size.to_string())
    }

    /// Albums in no order, different each time.
    pub fn random_albums(size: u32) -> Self {
        Self::new("getAlbumList2")
            .param("type", "random")
            .param("size", size.to_string())
    }

    /// Songs similar to those of `album`, as far as the server knows: it asks
    /// an outside service such as Last.fm, if it is set up to.
    pub fn similar_songs(album: &crate::AlbumId, count: u32) -> Self {
        Self::new("getSimilarSongs2")
            .param("id", album.as_str())
            .param("count", count.to_string())
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

    /// Tells the server `song` was listened to at `played_at` (milliseconds
    /// since the Unix epoch), which updates its play count and last played
    /// time.
    pub fn scrobble(song: &crate::SongId, played_at: u64) -> Self {
        Self::new("scrobble")
            .param("id", song.as_str())
            .param("time", played_at.to_string())
            .param("submission", "true")
    }

    /// Tells the server `song` is playing now, without counting it as played.
    pub fn now_playing(song: &crate::SongId) -> Self {
        Self::new("scrobble")
            .param("id", song.as_str())
            .param("submission", "false")
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

/// What a URL built from [`Endpoint::cover_art`] asks for, read back from
/// it without the salt and credentials it carries, so the same image can be
/// recognized whichever URL asked for it.
#[derive(Clone, PartialEq, Eq)]
pub struct CoverArtRequest {
    server: Url,
    id: CoverArtId,
    size: Option<u32>,
}

impl CoverArtRequest {
    /// `None` when `url` is not a `getCoverArt` call, or says what it asks
    /// for more than once, so which image the server sends is unclear.
    pub fn parse(url: &Url) -> Option<Self> {
        let prefix = url.path().strip_suffix("/rest/getCoverArt")?;
        let mut id = None;
        let mut size = None;
        for (name, value) in url.query_pairs() {
            match name.as_ref() {
                "id" if id.is_none() => id = Some(CoverArtId::new(value.as_ref())),
                "size" if size.is_none() => size = Some(value.parse().ok()?),
                "id" | "size" => return None,
                _ => {}
            }
        }
        let mut server = url.clone();
        server.set_query(None);
        server.set_fragment(None);
        server.set_path(prefix);
        Some(Self {
            server,
            id: id?,
            size,
        })
    }

    /// The server's base URL, as given to [`Endpoint::url`].
    pub fn server(&self) -> &Url {
        &self.server
    }

    pub fn id(&self) -> &CoverArtId {
        &self.id
    }

    /// Pixels across, when a scaled image was asked for.
    pub fn size(&self) -> Option<u32> {
        self.size
    }
}

/// Leaves out any user name and password in the server's address.
impl std::fmt::Debug for CoverArtRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut server = self.server.clone();
        server.set_username("").ok();
        server.set_password(None).ok();
        f.debug_struct("CoverArtRequest")
            .field("server", &server.as_str())
            .field("id", &self.id)
            .field("size", &self.size)
            .finish()
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
    fn cover_art_requests_read_back_without_credentials() {
        let credentials = Credentials::password("alice", "secret");
        for base in [
            "https://music.example.com",
            "https://example.com/navidrome/",
        ] {
            let base = Url::parse(base).unwrap();
            let id = CoverArtId::new("al-1_6512");
            let url = Endpoint::cover_art(&id, Some(512)).url(&base, &credentials);
            let request = CoverArtRequest::parse(&url).unwrap();
            assert_eq!(request.id(), &id);
            assert_eq!(request.size(), Some(512));
            assert_eq!(
                request.server().as_str().trim_end_matches('/'),
                base.as_str().trim_end_matches('/')
            );
            assert_eq!(request.server().query(), None);
        }
        let base = Url::parse("https://music.example.com").unwrap();
        let ping = Endpoint::ping().url(&base, &credentials);
        assert_eq!(CoverArtRequest::parse(&ping), None);
    }

    #[test]
    fn unclear_cover_art_requests_are_not_read() {
        let parse = |url: &str| CoverArtRequest::parse(&Url::parse(url).unwrap());
        let base = "https://music.example.com/rest/getCoverArt";
        assert!(parse(&format!("{base}?id=a&size=80")).is_some());
        assert_eq!(parse(&format!("{base}?id=a")).unwrap().size(), None);
        assert_eq!(parse(&format!("{base}?size=80")), None);
        assert_eq!(parse(&format!("{base}?id=a&size=large")), None);
        assert_eq!(parse(&format!("{base}?id=a&id=b")), None);
        assert_eq!(parse(&format!("{base}?id=a&size=80&size=512")), None);
    }

    #[test]
    fn cover_art_requests_print_without_a_password() {
        let url =
            Url::parse("https://alice:secret@music.example.com/rest/getCoverArt?id=a").unwrap();
        let printed = format!("{:?}", CoverArtRequest::parse(&url).unwrap());
        assert!(!printed.contains("secret"), "{printed}");
        assert!(!printed.contains("alice"), "{printed}");
    }

    #[test]
    fn scrobble_submits_the_play_with_its_time() {
        let base = Url::parse("https://music.example.com").unwrap();
        let song = crate::SongId::new("s1");
        let url =
            Endpoint::scrobble(&song, 1_700_000_000_123).url(&base, &Credentials::api_key("k"));
        assert_eq!(url.path(), "/rest/scrobble");
        assert_eq!(query(&url, "id").as_deref(), Some("s1"));
        assert_eq!(query(&url, "time").as_deref(), Some("1700000000123"));
        assert_eq!(query(&url, "submission").as_deref(), Some("true"));
    }

    #[test]
    fn now_playing_does_not_submit_a_play() {
        let base = Url::parse("https://music.example.com").unwrap();
        let song = crate::SongId::new("s1");
        let url = Endpoint::now_playing(&song).url(&base, &Credentials::api_key("k"));
        assert_eq!(url.path(), "/rest/scrobble");
        assert_eq!(query(&url, "id").as_deref(), Some("s1"));
        assert_eq!(query(&url, "submission").as_deref(), Some("false"));
        assert_eq!(query(&url, "time"), None);
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
