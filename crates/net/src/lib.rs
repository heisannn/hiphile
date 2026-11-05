//! reqwest needs a tokio reactor, but GPUI's executor is not tokio. [`Client`]
//! therefore owns its own runtime and hands back futures that complete through
//! a channel, so any executor can await them.
//!
//! Do not drop the last [`Client`] from inside a tokio runtime: dropping a
//! runtime there panics. Callers should drive these futures with their own
//! executor (GPUI's, or `futures::executor::block_on` in the CLI).

mod download;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::channel::oneshot;
use url::Url;

pub use download::{Download, DownloadHandle};
pub use http;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to start the network runtime: {0}")]
    Runtime(#[source] std::io::Error),
    /// The error never carries the request URL, which holds credentials.
    #[error(transparent)]
    Request(reqwest::Error),
    #[error("the server answered with HTTP {0}")]
    HttpStatus(u16),
    #[error("the download was interrupted: {0}")]
    Interrupted(String),
    #[error("the network runtime stopped before the request finished")]
    Canceled,
}

/// Cheap to clone; all clones share one connection pool and runtime.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    runtime: tokio::runtime::Runtime,
    http: reqwest::Client,
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(error.without_url())
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a response may go without sending any bytes.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

impl Client {
    pub fn new(user_agent: &str) -> Result<Self, Error> {
        Self::with_timeouts(user_agent, CONNECT_TIMEOUT, READ_TIMEOUT)
    }

    fn with_timeouts(
        user_agent: &str,
        connect_timeout: Duration,
        read_timeout: Duration,
    ) -> Result<Self, Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("hiphile-net")
            .enable_all()
            .build()
            .map_err(Error::Runtime)?;
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(connect_timeout)
            .read_timeout(read_timeout)
            .build()?;
        Ok(Self {
            inner: Arc::new(Inner { runtime, http }),
        })
    }

    /// Sends a request and buffers the whole response body.
    pub fn send(
        &self,
        request: http::Request<Bytes>,
    ) -> impl Future<Output = Result<http::Response<Bytes>, Error>> + Send + 'static {
        let http = self.inner.http.clone();
        self.run(async move {
            let request = reqwest::Request::try_from(request)?;
            let response = http.execute(request).await?;
            let mut builder = http::Response::builder()
                .status(response.status())
                .version(response.version());
            if let Some(headers) = builder.headers_mut() {
                headers.extend(response.headers().clone());
            }
            let body = response.bytes().await?;
            Ok(builder
                .body(body)
                .expect("status and headers come from a valid response"))
        })
    }

    pub fn get(
        &self,
        url: Url,
    ) -> impl Future<Output = Result<http::Response<Bytes>, Error>> + Send + 'static {
        let request = http::Request::get(url.as_str())
            .body(Bytes::new())
            .expect("a parsed Url is a valid URI");
        self.send(request)
    }

    /// Sends `form`, already URL-encoded, as a form body.
    pub fn post_form(
        &self,
        url: Url,
        form: String,
    ) -> impl Future<Output = Result<http::Response<Bytes>, Error>> + Send + 'static {
        let request = http::Request::post(url.as_str())
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(Bytes::from(form))
            .expect("a parsed Url is a valid URI");
        self.send(request)
    }

    /// Starts a download and resolves once the response headers arrive.
    ///
    /// The body keeps arriving in the background; see [`Download`].
    pub fn download(
        &self,
        url: Url,
    ) -> impl Future<Output = Result<Download, Error>> + Send + 'static {
        let http = self.inner.http.clone();
        let (sender, receiver) = oneshot::channel();
        // The request is built inside the task: with timeouts configured,
        // reqwest creates its timers when the request is built, which needs
        // the runtime's context.
        self.inner
            .runtime
            .spawn(async move { download::run(http, url, sender).await });
        async move { receiver.await.unwrap_or(Err(Error::Canceled)) }
    }

    fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, Error>> + Send + 'static,
    ) -> impl Future<Output = Result<T, Error>> + Send + 'static {
        let (sender, receiver) = oneshot::channel();
        self.inner.runtime.spawn(async move {
            sender.send(work.await).ok();
        });
        async move { receiver.await.unwrap_or(Err(Error::Canceled)) }
    }
}
