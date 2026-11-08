use bytes::Bytes;
use catalog::CoverStore;
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
    match look_up(covers, cover, fetch).await {
        Answer::Kept(image) => Ok(image_response(image)),
        Answer::Fetched(result) => result,
    }
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
        Some((covers, cover)) => match look_up(covers, &cover, fetch).await {
            Answer::Kept(image) => return Some(image),
            Answer::Fetched(result) => result,
        },
        None => fetch().await,
    };
    let response = fetched.ok()?;
    (response.status().is_success() && is_image(response.headers()))
        .then(|| response.into_body().to_vec())
}

enum Answer {
    Kept(Vec<u8>),
    Fetched(Result<Response<Bytes>, net::Error>),
}

async fn look_up<F, Fetch>(covers: &CoverStore, cover: &CoverArtRequest, fetch: F) -> Answer
where
    F: FnOnce() -> Fetch,
    Fetch: Future<Output = Result<Response<Bytes>, net::Error>>,
{
    let kept = match covers.get(cover) {
        Some(kept) if kept.is_fresh() => return Answer::Kept(kept.into_bytes()),
        kept => kept,
    };
    match fetch().await {
        Ok(response) if response.status().is_success() && is_image(response.headers()) => {
            // A failure to keep it costs a download later
            covers.put(cover, response.body()).ok();
            Answer::Fetched(Ok(response))
        }
        result => match kept {
            Some(kept) => Answer::Kept(kept.into_bytes()),
            None => Answer::Fetched(result),
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
