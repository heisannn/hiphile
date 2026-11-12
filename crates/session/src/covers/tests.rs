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

fn cover(id: &str) -> CoverArtRequest {
    CoverArtRequest::parse(&cover_url(id)).unwrap()
}

fn response(content_type: &str, body: &'static [u8]) -> Response<Bytes> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Bytes::from_static(body))
        .unwrap()
}

fn answer(
    covers: &CoverStore,
    cover: &CoverArtRequest,
    fetched: Result<Response<Bytes>, net::Error>,
    fetches: &Cell<u32>,
) -> Result<Bytes, net::Error> {
    let fetch = || {
        fetches.set(fetches.get() + 1);
        async { fetched }
    };
    block_on(answer_cover(covers, cover, fetch)).map(|response| response.into_body())
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
fn a_kept_image_is_given_without_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let covers = CoverStore::at(dir.path(), 1 << 20, Duration::from_secs(3600));
    let net = net::Client::new("hiphile-test").unwrap();
    // The server's address is no one's, so only the kept image can answer
    let url = cover_url("al-1")
        .as_str()
        .replace("example.com", "127.0.0.1:9");
    let unreachable = CoverArtRequest::parse(&url::Url::parse(&url).unwrap()).unwrap();
    covers.put(&unreachable, b"jpeg").unwrap();

    let image = block_on(cover_image(&net, Some(&covers), &url));
    assert_eq!(image.as_deref(), Some(&b"jpeg"[..]));
    assert_eq!(block_on(cover_image(&net, None, &url)), None);
}

#[test]
fn a_kept_image_is_given_as_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let covers = CoverStore::at(dir.path(), 1 << 20, Duration::from_secs(3600));
    let net = net::Client::new("hiphile-test").unwrap();
    let url = cover_url("al-1")
        .as_str()
        .replace("example.com", "127.0.0.1:9");
    let unreachable = CoverArtRequest::parse(&url::Url::parse(&url).unwrap()).unwrap();
    let path = covers.put(&unreachable, b"jpeg").unwrap().unwrap();

    let file = block_on(cover_file(&net, Some(&covers), &url));
    assert_eq!(file, Some(CoverSource::File(path)));
    assert_eq!(block_on(cover_file(&net, None, &url)), None);
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
