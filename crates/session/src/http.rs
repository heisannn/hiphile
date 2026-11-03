use bytes::Bytes;
use futures::AsyncReadExt as _;
use futures::future::BoxFuture;
use gpui_kit::http_client::{self, AsyncBody, HttpClient, Request, Response, Url};

/// GPUI's [`HttpClient`] backed by the shared [`net::Client`].
///
/// Bodies are buffered in both directions; this serves API calls and images,
/// not audio streams, which `playback` reads through `net` directly.
pub(crate) struct NetHttpClient {
    net: net::Client,
}

impl NetHttpClient {
    pub(crate) fn new(net: net::Client) -> Self {
        Self { net }
    }
}

impl HttpClient for NetHttpClient {
    fn user_agent(&self) -> Option<&http_client::http::HeaderValue> {
        // `net` sets the user agent on every request itself.
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, http_client::Result<Response<AsyncBody>>> {
        let net = self.net.clone();
        Box::pin(async move {
            let (parts, mut body) = request.into_parts();
            let mut buffer = Vec::new();
            body.read_to_end(&mut buffer).await?;
            let request = Request::from_parts(parts, Bytes::from(buffer));
            let response = net.send(request).await?;
            Ok(response.map(AsyncBody::from_bytes))
        })
    }
}
