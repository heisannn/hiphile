//! What the tests of several handles share. Apart from [`crate::test_support`], which the views'
//! tests use too, as `wait_for` needs the test executor, a dev-dependency only.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::time::Duration;

use executor::testing::TestExecutor;
use serde_json::{Value, json};
use subsonic::{Album, Artist, ScanStatus, Song};

/// A server that cannot be reached.
pub(crate) const NOWHERE: &str = "http://127.0.0.1:9";

/// A server that takes each request and never answers it.
pub(crate) fn server_never_answering() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        // Held open, unanswered
        let _requests: Vec<_> = listener.incoming().collect();
    });
    url
}

/// A server that gives `fields` in answer to anything.
pub(crate) fn server_answering(fields: Value) -> String {
    server_answering_each(move |_| Some(fields.clone()))
}

/// A server that answers each method, such as `getArtist`, with the fields `answer` gives, which
/// can turn the answer into a failure with `"status": "failed"`.
pub(crate) fn server_answering_each(
    answer: impl Fn(&str) -> Option<Value> + Send + 'static,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            request.read_line(&mut first).ok();
            let mut line = String::new();
            while request.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let method = first
                .split_whitespace()
                .nth(1)
                .and_then(|path| path.split("/rest/").nth(1))
                .and_then(|path| path.split(['?', '.']).next())
                .unwrap_or_default();
            let mut body = json!({ "status": "ok", "version": "1.16.1" });
            if let Some(Value::Object(fields)) = answer(method) {
                body.as_object_mut().unwrap().extend(fields);
            }
            let body = json!({ "subsonic-response": body }).to_string();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len(),
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(body.as_bytes()).ok();
        }
    });
    url
}

/// A server that gives `image` as a JPEG in answer to anything.
pub(crate) fn server_giving_image(image: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while request.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                image.len(),
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(image).ok();
        }
    });
    url
}

/// Runs what is under way until `done`, as answers from the network arrive off the executor.
pub(crate) fn wait_for(cx: &TestExecutor, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("what was waited for never happened");
}

/// What a sync would have saved.
#[derive(Default)]
pub(crate) struct Saved {
    pub(crate) albums: Vec<Album>,
    pub(crate) artists: Vec<Artist>,
    pub(crate) songs: Vec<Song>,
}

impl catalog::Source for Saved {
    async fn albums_page(&self, _: u32, _: u32) -> Result<Vec<Album>, client::Error> {
        Ok(self.albums.clone())
    }

    async fn artists(&self) -> Result<Vec<Artist>, client::Error> {
        Ok(self.artists.clone())
    }

    async fn songs_page(&self, _: u32, _: u32) -> Result<Vec<Song>, client::Error> {
        Ok(self.songs.clone())
    }

    async fn scan_status(&self) -> Result<ScanStatus, client::Error> {
        Ok(serde_json::from_value(json!({ "scanning": false })).unwrap())
    }

    async fn library_modified(&self) -> Result<Option<u64>, client::Error> {
        Ok(None)
    }
}

/// Saves `saved` in `catalog`, in place of what it held.
pub(crate) fn keep(catalog: &catalog::Catalog, saved: &Saved) {
    let mode = catalog::Mode::Always;
    futures::executor::block_on(catalog::sync_albums(catalog, saved, mode)).unwrap();
    futures::executor::block_on(catalog::sync_artists(catalog, saved, mode)).unwrap();
    futures::executor::block_on(catalog::sync_songs(catalog, saved, mode)).unwrap();
}
