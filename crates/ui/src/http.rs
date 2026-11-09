use bytes::Bytes;
use catalog::CoverStore;
use futures::AsyncReadExt as _;
use futures::future::BoxFuture;
use gpui_kit::http_client::http::{Method, Uri};
use gpui_kit::http_client::{self, AsyncBody, HttpClient, Request, Response};
use subsonic::CoverArtRequest;

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
        // `net` sets the user agent on every request itself
        None
    }

    fn proxy(&self) -> Option<&http_client::Url> {
        None
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, gpui_kit::Result<Response<AsyncBody>>> {
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
                    session::answer_cover(&covers, &cover, || net.send(request)).await?
                }
                None => net.send(request).await?,
            };
            Ok(response.map(AsyncBody::from_bytes))
        })
    }
}

fn cover_request(method: &Method, uri: &Uri) -> Option<CoverArtRequest> {
    if method != Method::GET {
        return None;
    }
    CoverArtRequest::parse(&url::Url::parse(&uri.to_string()).ok()?)
}

#[cfg(test)]
mod tests {
    use subsonic::{CoverArtId, Credentials, Endpoint};

    use super::*;

    const SERVER: &str = "https://example.com/my%20music/";

    fn cover_url(id: &str) -> url::Url {
        let base = url::Url::parse(SERVER).unwrap();
        Endpoint::cover_art(&CoverArtId::new(id), Some(512))
            .url(&base, &Credentials::password("alice", "secret"))
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
}
