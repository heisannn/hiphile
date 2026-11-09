//! Finds a song's lyrics on Musixmatch, through the API its mobile apps use.
//!
//! The API is not a published one, so everything it answers is checked and
//! anything missing is treated as an ordinary case.

mod response;
mod token;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use url::Url;

use crate::{Error, Lyrics, Provider, TrackQuery};
use token::{Token, Tokens};

const BASE_URL: &str = "https://apic-appmobile.musixmatch.com/ws/1.1/";
const APP_ID: &str = "mac-ios-v2.0";
const USER_AGENT: &str = "Musixmatch/2025120901 CFNetwork/3860.300.31 Darwin/25.2.0";
const APP_VERSION: &str = "10.1.1";
const TOKEN_VARIABLE: &str = "MUSIXMATCH_USER_TOKEN";
const TOKEN_FILE: &str = "musixmatch.json";

#[derive(Clone)]
pub struct Client {
    net: net::Client,
    base: Url,
    tokens: Arc<Tokens>,
}

impl Client {
    /// A client that uses the token in the environment, if there is one, and
    /// otherwise asks for one and keeps it in the app's cache folder `cache`, or
    /// only in memory without one.
    pub fn with_cache(net: net::Client, cache: Option<&Path>) -> Self {
        match std::env::var(TOKEN_VARIABLE) {
            Ok(token) if !token.is_empty() => Self::new(net, Tokens::given(token)),
            _ => Self::kept_in_cache(net, cache),
        }
    }

    fn kept_in_cache(net: net::Client, cache: Option<&Path>) -> Self {
        let path = cache.map(|cache| cache.join(TOKEN_FILE));
        Self::new(net, Tokens::kept_in(path))
    }

    pub fn with_token(net: net::Client, base: Url, token: impl Into<String>) -> Self {
        Self {
            base,
            ..Self::new(net, Tokens::given(token.into()))
        }
    }

    pub fn with_token_file(net: net::Client, base: Url, path: PathBuf) -> Self {
        Self {
            base,
            ..Self::new(net, Tokens::kept_in(Some(path)))
        }
    }

    fn new(net: net::Client, tokens: Tokens) -> Self {
        Self {
            net,
            base: Url::parse(BASE_URL).expect("the service's address is valid"),
            tokens: Arc::new(tokens),
        }
    }

    /// A token the service turned down is replaced once at most, here and for
    /// a while after, so that a service refusing every token is not asked for
    /// more of them.
    pub async fn find_lyrics(&self, query: &TrackQuery) -> Result<Lyrics, Error> {
        let token = self.tokens.current(self).await?;
        match self.request_lyrics(query, &token).await {
            Err(error) if error.is_unauthorized() && token.is_replaceable() => {
                let token = self.tokens.replace(self, &token).await.map_err(|_| error)?;
                self.request_lyrics(query, &token).await
            }
            result => result,
        }
    }

    async fn request_lyrics(&self, query: &TrackQuery, token: &Token) -> Result<Lyrics, Error> {
        let mut url = self.url("macro.subtitles.get");
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("usertoken", token.value())
                .append_pair("namespace", "lyrics_richsynched")
                .append_pair("subtitle_format", "mxm")
                .append_pair("q_track", &query.title)
                .append_pair("optional_calls", "track.richsync")
                .append_pair("richsync_compact_type", "words")
                .append_pair(
                    "part",
                    "track_lyrics_translation_status,track_structure,track_performer_tagging",
                );
            if let Some(artist) = &query.artist {
                pairs
                    .append_pair("q_artist", artist)
                    .append_pair("q_artists", artist);
            }
            if let Some(album) = &query.album {
                pairs.append_pair("q_album", album);
            }
            if let Some(duration) = query.duration {
                pairs
                    .append_pair("q_duration", &duration.as_secs_f64().to_string())
                    .append_pair("f_subtitle_length", &duration.as_secs().to_string());
            }
        }
        let body = self.get(url).await?;
        response::lyrics(&body, query)
    }

    async fn request_token(&self) -> Result<String, Error> {
        let body = self.get(self.url("token.get")).await?;
        response::token(&body)
    }

    fn url(&self, endpoint: &str) -> Url {
        let mut url = self
            .base
            .join(endpoint)
            .expect("an endpoint's name is a valid path");
        url.query_pairs_mut()
            .append_pair("app_id", APP_ID)
            .append_pair("format", "json");
        url
    }

    async fn get(&self, url: Url) -> Result<Bytes, Error> {
        let request = net::http::Request::get(url.as_str())
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT)
            .header("X-User-Agent", USER_AGENT)
            .header("x-mxm-app-version", APP_VERSION)
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Cookie", "x-mxm-token-guid=")
            .body(Bytes::new())
            .expect("a parsed Url and these headers make a valid request");
        let response = self.net.send(request).await?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        Ok(response.into_body())
    }
}

impl Provider for Client {
    fn find_lyrics<'a>(&'a self, query: &'a TrackQuery) -> BoxFuture<'a, Result<Lyrics, Error>> {
        Client::find_lyrics(self, query).boxed()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn lyrics_from_richsync_body(body: &str) -> Lyrics {
    let answer = serde_json::json!({ "message": {
        "header": { "status_code": 200 },
        "body": { "macro_calls": { "track.richsync.get": { "message": {
            "header": { "status_code": 200 },
            "body": { "richsync": { "richsync_body": body } },
        } } } },
    } });
    response::lyrics(answer.to_string().as_bytes(), &TrackQuery::default())
        .expect("an answer of the right shape")
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::TcpListener;
    use std::sync::Mutex;

    use futures::executor::block_on;
    use serde_json::json;

    use super::*;

    fn service(answers: Vec<String>) -> (Url, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for answer in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                recorded.lock().unwrap().push(line);
                let mut header = String::new();
                while reader.read_line(&mut header).unwrap() > 2 {
                    header.clear();
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len(),
                )
                .unwrap();
            }
        });
        (base, requests)
    }

    fn token_answer(token: &str) -> String {
        json!({
            "message": { "header": { "status_code": 200 }, "body": { "user_token": token } }
        })
        .to_string()
    }

    fn refusal() -> String {
        json!({
            "message": { "header": { "status_code": 401, "hint": "renew" }, "body": [] }
        })
        .to_string()
    }

    fn plain_answer(text: &str) -> String {
        json!({
            "message": {
                "header": { "status_code": 200 },
                "body": { "macro_calls": { "track.lyrics.get": { "message": {
                    "header": { "status_code": 200 },
                    "body": { "lyrics": { "lyrics_body": text } },
                } } } },
            }
        })
        .to_string()
    }

    fn net() -> net::Client {
        net::Client::new("hiphile-test").unwrap()
    }

    fn query() -> TrackQuery {
        TrackQuery::new("夜 & 朝").artist(Some("A+B"))
    }

    fn found_text(lyrics: Lyrics) -> String {
        match lyrics {
            Lyrics::Plain(plain) => plain.text().to_string(),
            other => panic!("expected plain lyrics, got {other:?}"),
        }
    }

    #[test]
    fn a_new_token_is_kept_for_the_next_client() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep").join("token.json");
        let (base, requests) = service(vec![
            token_answer("first"),
            plain_answer("a"),
            plain_answer("b"),
        ]);

        let client = Client::with_token_file(net(), base.clone(), path.clone());
        assert_eq!(
            found_text(block_on(client.find_lyrics(&query())).unwrap()),
            "a"
        );
        let client = Client::with_token_file(net(), base, path.clone());
        assert_eq!(
            found_text(block_on(client.find_lyrics(&query())).unwrap()),
            "b"
        );

        let requests = requests.lock().unwrap();
        assert!(requests[0].contains("/token.get?"));
        assert!(!requests[0].contains("usertoken"));
        assert!(requests[1].contains("usertoken=first"));
        assert!(requests[2].contains("usertoken=first"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn the_token_is_kept_in_the_cache_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (base, _requests) = service(vec![token_answer("first"), plain_answer("a")]);

        let client = Client {
            base,
            ..Client::kept_in_cache(net(), Some(dir.path()))
        };
        block_on(client.find_lyrics(&query())).unwrap();

        let kept = std::fs::read_to_string(dir.path().join("musixmatch.json")).unwrap();
        assert!(kept.contains("first"), "{kept}");
    }

    #[test]
    fn details_are_encoded_and_those_left_out_are_not_sent() {
        let (base, requests) = service(vec![plain_answer("a")]);
        let client = Client::with_token(net(), base, "t");
        block_on(client.find_lyrics(&query())).unwrap();

        let requests = requests.lock().unwrap();
        assert!(requests[0].contains("q_artists=A%2BB"), "{}", requests[0]);
        assert!(
            requests[0].contains("q_track=%E5%A4%9C+%26"),
            "{}",
            requests[0]
        );
        assert!(!requests[0].contains("q_album"));
        assert!(!requests[0].contains("q_duration"));
    }

    #[test]
    fn a_refused_token_is_replaced_once_and_the_new_one_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        let (base, requests) = service(vec![
            token_answer("old"),
            refusal(),
            token_answer("new"),
            plain_answer("a"),
        ]);

        let client = Client::with_token_file(net(), base, path.clone());
        // The first token was asked for moments ago, so wait out the pause
        client.tokens.forget_last_request();
        block_on(client.tokens.current(&client)).unwrap();
        client.tokens.forget_last_request();
        assert_eq!(
            found_text(block_on(client.find_lyrics(&query())).unwrap()),
            "a"
        );

        assert!(requests.lock().unwrap()[3].contains("usertoken=new"));
        let kept = std::fs::read_to_string(path).unwrap();
        assert!(kept.contains("new") && !kept.contains("old"));
    }

    #[test]
    fn a_service_refusing_every_token_is_not_asked_for_more() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        let (base, requests) = service(vec![token_answer("a"), refusal(), refusal()]);

        let client = Client::with_token_file(net(), base, path);
        for _ in 0..2 {
            let error = block_on(client.find_lyrics(&query())).unwrap_err();
            assert!(error.is_unauthorized(), "{error:?}");
        }

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests.iter().filter(|r| r.contains("token.get")).count(),
            1
        );
    }

    #[test]
    fn a_refused_request_for_a_token_keeps_the_token_there_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        let (base, _) = service(vec![token_answer("old"), refusal(), refusal()]);

        let client = Client::with_token_file(net(), base, path.clone());
        block_on(client.tokens.current(&client)).unwrap();
        client.tokens.forget_last_request();
        block_on(client.find_lyrics(&query())).unwrap_err();

        assert!(std::fs::read_to_string(path).unwrap().contains("old"));
    }

    fn client_of(base: Url) -> Client {
        Client::with_token(net(), base, "unused")
    }

    #[test]
    fn a_request_for_a_token_that_never_got_there_holds_nothing_up() {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let nobody = Url::parse(&format!("http://{}/", closed.local_addr().unwrap())).unwrap();
        drop(closed);
        let (base, _) = service(vec![token_answer("t")]);

        let tokens = Tokens::kept_in(None);
        let error = block_on(tokens.current(&client_of(nobody))).unwrap_err();
        assert!(matches!(error, Error::Network(_)), "{error:?}");
        block_on(tokens.current(&client_of(base))).unwrap();
    }

    #[test]
    fn a_request_for_a_token_given_up_on_holds_nothing_up() {
        let silent = TcpListener::bind("127.0.0.1:0").unwrap();
        let silent_base = Url::parse(&format!("http://{}/", silent.local_addr().unwrap())).unwrap();
        let (base, _) = service(vec![token_answer("t")]);

        let tokens = Tokens::kept_in(None);
        let client = client_of(silent_base);
        assert!(tokens.current(&client).now_or_never().is_none());
        block_on(tokens.current(&client_of(base))).unwrap();
    }

    #[test]
    fn a_refused_request_for_a_token_is_not_repeated_at_once() {
        let (base, requests) = service(vec![refusal()]);

        let client = client_of(base);
        let tokens = Tokens::kept_in(None);
        let error = block_on(tokens.current(&client)).unwrap_err();
        assert!(error.is_unauthorized(), "{error:?}");
        let error = block_on(tokens.current(&client)).unwrap_err();
        assert!(matches!(error, Error::TokenUnavailable), "{error:?}");
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_given_token_is_not_replaced() {
        let (base, requests) = service(vec![refusal()]);
        let client = Client::with_token(net(), base, "given");
        let error = block_on(client.find_lyrics(&query())).unwrap_err();
        assert!(error.is_unauthorized());
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn errors_do_not_show_the_token() {
        // Nothing listens here, so the request fails after it is built
        let base = Url::parse("http://127.0.0.1:9/").unwrap();
        let client = Client::with_token(net(), base, "secret-token");
        let error = block_on(client.find_lyrics(&query())).unwrap_err();
        assert!(matches!(error, Error::Network(_)));
        assert!(!format!("{error} {error:?}").contains("secret-token"));
    }
}
