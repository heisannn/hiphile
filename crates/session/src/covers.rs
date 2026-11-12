use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use catalog::{CoverStore, KeptCover, KeptCoverFile};
use net::http::{HeaderMap, Response, StatusCode, header};
use subsonic::CoverArtRequest;

#[cfg(test)]
mod tests;

/// Answers with the kept image while it is fresh. Otherwise answers with
/// what `fetch` brings, keeping it when it is an image; Subsonic reports a
/// failure as a document with HTTP 200. When `fetch` fails, an old image
/// beats the failure.
pub async fn answer_cover<F, Fetch>(
    covers: &CoverStore,
    cover: &CoverArtRequest,
    fetch: F,
) -> Result<Response<Bytes>, net::Error>
where
    F: FnOnce() -> Fetch,
    Fetch: Future<Output = Result<Response<Bytes>, net::Error>>,
{
    match look_up(covers, cover, CoverStore::get, fetch).await {
        Answer::Kept(image) => Ok(image_response(image.into_bytes())),
        Answer::Fetched { result, .. } => result,
    }
}

/// Where a cover's image is to be read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverSource {
    /// A kept image. Once it is no longer fresh, fetching it again writes over the same path,
    /// and keeping other covers over capacity may remove it.
    File(PathBuf),
    /// Where there is nowhere to keep it, or it is too large to keep.
    Bytes(Arc<[u8]>),
}

/// The image at `url`, from `covers` as [`answer_cover`] would answer, or nothing when neither
/// they nor the server have it. Blocks on the disk.
pub async fn cover_image(
    net: &net::Client,
    covers: Option<&CoverStore>,
    url: &str,
) -> Option<Vec<u8>> {
    let url = url::Url::parse(url).ok()?;
    let fetch = || net.get(url.clone());
    let fetched = match covers.zip(CoverArtRequest::parse(&url)) {
        Some((covers, cover)) => match look_up(covers, &cover, CoverStore::get, fetch).await {
            Answer::Kept(image) => return Some(image.into_bytes()),
            Answer::Fetched { result, .. } => result,
        },
        None => fetch().await,
    };
    image(fetched).map(|image| image.to_vec())
}

/// As [`cover_image`], giving the file a kept image is in rather than reading it.
pub async fn cover_file(
    net: &net::Client,
    covers: Option<&CoverStore>,
    url: &str,
) -> Option<CoverSource> {
    let url = url::Url::parse(url).ok()?;
    let fetch = || net.get(url.clone());
    let fetched = match covers.zip(CoverArtRequest::parse(&url)) {
        Some((covers, cover)) => match look_up(covers, &cover, CoverStore::get_file, fetch).await {
            Answer::Kept(file) => return Some(CoverSource::File(file.into_path())),
            Answer::Fetched {
                kept_at: Some(path),
                ..
            } => return Some(CoverSource::File(path)),
            Answer::Fetched { result, .. } => result,
        },
        None => fetch().await,
    };
    image(fetched).map(|image| CoverSource::Bytes(Arc::from(&image[..])))
}

fn image(fetched: Result<Response<Bytes>, net::Error>) -> Option<Bytes> {
    let response = fetched.ok()?;
    (response.status().is_success() && is_image(response.headers())).then(|| response.into_body())
}

trait Kept {
    fn is_fresh(&self) -> bool;
}

impl Kept for KeptCover {
    fn is_fresh(&self) -> bool {
        self.is_fresh()
    }
}

impl Kept for KeptCoverFile {
    fn is_fresh(&self) -> bool {
        self.is_fresh()
    }
}

enum Answer<K> {
    Kept(K),
    Fetched {
        result: Result<Response<Bytes>, net::Error>,
        /// Where the image fetched was kept.
        kept_at: Option<PathBuf>,
    },
}

async fn look_up<K, F, Fetch>(
    covers: &CoverStore,
    cover: &CoverArtRequest,
    find: impl FnOnce(&CoverStore, &CoverArtRequest) -> Option<K>,
    fetch: F,
) -> Answer<K>
where
    K: Kept,
    F: FnOnce() -> Fetch,
    Fetch: Future<Output = Result<Response<Bytes>, net::Error>>,
{
    let kept = match find(covers, cover) {
        Some(kept) if kept.is_fresh() => return Answer::Kept(kept),
        kept => kept,
    };
    match fetch().await {
        Ok(response) if response.status().is_success() && is_image(response.headers()) => {
            // A failure to keep it costs a download later
            let kept_at = covers.put(cover, response.body()).ok().flatten();
            Answer::Fetched {
                result: Ok(response),
                kept_at,
            }
        }
        result => match kept {
            Some(kept) => Answer::Kept(kept),
            None => Answer::Fetched {
                result,
                kept_at: None,
            },
        },
    }
}

fn image_response(image: Vec<u8>) -> Response<Bytes> {
    Response::builder()
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
