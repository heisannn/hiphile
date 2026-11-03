use bytes::Bytes;
use catalog::CoverStore;
use futures::AsyncReadExt as _;
use futures::future::BoxFuture;
use gpui_kit::http_client::http::{HeaderMap, Method, StatusCode, Uri, header};
use gpui_kit::http_client::{self, AsyncBody, HttpClient, Request, Response};
use subsonic::CoverArtRequest;

/// GPUI's [`HttpClient`] backed by the shared [`net::Client`].
///
/// Bodies are buffered in both directions; this serves API calls and images,
/// not audio streams, which `playback` reads through `net` directly. Cover
/// art comes from `covers` once it has been downloaded.
pub(crate) struct NetHttpClient {
    net: net::Client,
    covers: Option<CoverStore>,
}

impl NetHttpClient {
    pub(crate) fn new(net: net::Client, covers: Option<CoverStore>) -> Self {
        Self { net, covers }
    }
}

impl HttpClient for NetHttpClient {
    fn user_agent(&self) -> Option<&http_client::http::HeaderValue> {
        // `net` sets the user agent on every request itself.
        None
    }

    fn proxy(&self) -> Option<&http_client::Url> {
        None
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, http_client::Result<Response<AsyncBody>>> {
        let net = self.net.clone();
        let covers = self.covers.clone();
        // Image loads run on GPUI's background executor, so the disk may
        // block here.
        Box::pin(async move {
            let (parts, mut body) = request.into_parts();
            let cover = covers.zip(cover_request(&parts.method, &parts.uri));
            let mut buffer = Vec::new();
            body.read_to_end(&mut buffer).await?;
            let request = Request::from_parts(parts, Bytes::from(buffer));
            let response = match cover {
                Some((covers, cover)) => {
                    answer_cover(&covers, &cover, || net.send(request)).await?
                }
                None => net.send(request).await?,
            };
            Ok(response.map(AsyncBody::from_bytes))
        })
    }
}

/// The cover art a request asks for, when it is a `getCoverArt` call.
fn cover_request(method: &Method, uri: &Uri) -> Option<CoverArtRequest> {
    if method != Method::GET {
        return None;
    }
    CoverArtRequest::parse(&url::Url::parse(&uri.to_string()).ok()?)
}

/// Answers with the kept image while it is fresh. Otherwise answers with
/// what `fetch` brings, keeping it when it is an image; Subsonic reports a
/// failure as a document with HTTP 200. When `fetch` fails, an old image
/// beats the failure.
async fn answer_cover<F, Fetch>(
    covers: &CoverStore,
    cover: &CoverArtRequest,
    fetch: F,
) -> Result<net::http::Response<Bytes>, net::Error>
where
    F: FnOnce() -> Fetch,
    Fetch: Future<Output = Result<net::http::Response<Bytes>, net::Error>>,
{
    let kept = match covers.get(cover) {
        Some(kept) if kept.is_fresh() => return Ok(image_response(kept.into_bytes())),
        kept => kept,
    };
    match fetch().await {
        Ok(response) if response.status().is_success() && is_image(response.headers()) => {
            // A failure to keep it costs a download later.
            covers.put(cover, response.body()).ok();
            Ok(response)
        }
        result => match kept {
            Some(kept) => Ok(image_response(kept.into_bytes())),
            None => result,
        },
    }
}

fn image_response(image: Vec<u8>) -> net::http::Response<Bytes> {
    net::http::Response::builder()
        .status(StatusCode::OK)
        .body(Bytes::from(image))
        .expect("a status and a body make a valid response")
}

fn is_image(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("image/")
        })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use futures::executor::block_on;
    use subsonic::{CoverArtId, Credentials, Endpoint};

    use super::*;

    const SERVER: &str = "https://example.com/my%20music/";

    fn cover_url(id: &str) -> url::Url {
        let base = url::Url::parse(SERVER).unwrap();
        Endpoint::cover_art(&CoverArtId::new(id), Some(512))
            .url(&base, &Credentials::password("alice", "secret"))
    }

    /// The request as GPUI hands it over.
    fn cover(id: &str) -> CoverArtRequest {
        let request = Request::get(cover_url(id).as_str()).body(()).unwrap();
        cover_request(request.method(), request.uri()).unwrap()
    }

    fn response(content_type: &str, body: &'static [u8]) -> net::http::Response<Bytes> {
        net::http::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .body(Bytes::from_static(body))
            .unwrap()
    }

    /// Answers `cover`, counting the fetches in `fetches`.
    fn answer(
        covers: &CoverStore,
        cover: &CoverArtRequest,
        fetched: Result<net::http::Response<Bytes>, net::Error>,
        fetches: &Cell<u32>,
    ) -> Result<Bytes, net::Error> {
        let fetch = || {
            fetches.set(fetches.get() + 1);
            async { fetched }
        };
        block_on(answer_cover(covers, cover, fetch)).map(|response| response.into_body())
    }

    #[test]
    fn a_request_as_gpui_sends_it_reads_back_like_the_url() {
        let url = cover_url("al-1");
        let request = Request::get(url.as_str()).body(()).unwrap();
        assert_eq!(
            cover_request(request.method(), request.uri()),
            CoverArtRequest::parse(&url)
        );
        assert!(cover_request(request.method(), request.uri()).is_some());

        let post = Request::post(url.as_str()).body(()).unwrap();
        assert_eq!(cover_request(post.method(), post.uri()), None);
        let ping = Request::get("https://example.com/rest/ping?f=json")
            .body(())
            .unwrap();
        assert_eq!(cover_request(ping.method(), ping.uri()), None);
    }

    #[test]
    fn a_kept_image_is_answered_without_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let covers = CoverStore::at(dir.path(), 1 << 20, Duration::from_secs(3600));
        let fetches = Cell::new(0);
        let first = answer(
            &covers,
            &cover("al-1"),
            Ok(response("image/jpeg", b"jpeg")),
            &fetches,
        );
        assert_eq!(first.unwrap(), &b"jpeg"[..]);
        let again = answer(&covers, &cover("al-1"), Err(net::Error::Canceled), &fetches);
        assert_eq!(again.unwrap(), &b"jpeg"[..]);
        assert_eq!(fetches.get(), 1);
    }

    #[test]
    fn a_document_in_place_of_an_image_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let covers = CoverStore::at(dir.path(), 1 << 20, Duration::from_secs(3600));
        let fetches = Cell::new(0);
        let failure = br#"{"subsonic-response":{"status":"failed"}}"#;
        let answered = answer(
            &covers,
            &cover("al-1"),
            Ok(response("application/json", failure)),
            &fetches,
        );
        assert_eq!(answered.unwrap(), &failure[..]);
        assert!(covers.get(&cover("al-1")).is_none());
    }

    #[test]
    fn an_old_image_is_fetched_again_and_beats_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let covers = CoverStore::at(dir.path(), 1 << 20, Duration::ZERO);
        let fetches = Cell::new(0);
        answer(
            &covers,
            &cover("al-1"),
            Ok(response("image/png", b"old")),
            &fetches,
        )
        .unwrap();
        let offline = answer(&covers, &cover("al-1"), Err(net::Error::Canceled), &fetches);
        assert_eq!(offline.unwrap(), &b"old"[..]);
        let changed = answer(
            &covers,
            &cover("al-1"),
            Ok(response("image/png", b"new")),
            &fetches,
        );
        assert_eq!(changed.unwrap(), &b"new"[..]);
        assert_eq!(covers.get(&cover("al-1")).unwrap().bytes(), b"new");
        assert_eq!(fetches.get(), 3);
    }

    #[test]
    fn only_images_are_kept() {
        let headers = |content_type: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, content_type.parse().unwrap());
            headers
        };
        assert!(is_image(&headers("image/jpeg")));
        assert!(is_image(&headers("Image/PNG")));
        assert!(!is_image(&headers("application/json; charset=utf-8")));
        assert!(!is_image(&HeaderMap::new()));
    }
}
