//! A Subsonic server on a local port that serves a library's songs page by page, for the tests and
//! the shots of the views that list them.

use std::collections::HashSet;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use serde_json::{Value, json};

#[derive(Default)]
struct State {
    songs: Arc<[Value]>,
    starred: HashSet<String>,
    /// Pages of songs asked for.
    reads: usize,
    held: bool,
    refused: bool,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub struct SongsServer {
    url: String,
    state: Shared,
}

impl SongsServer {
    /// Serves `songs`, as the server lists them, with no playlists and none of them favorites.
    pub fn new(songs: impl Into<Arc<[Value]>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Shared::default();
        state.0.lock().unwrap().songs = songs.into();
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                thread::spawn(move || serve(stream, &state));
            }
        });
        Self { url, state }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Lists `songs` from now on, as after a scan.
    pub fn set_songs(&self, songs: impl Into<Arc<[Value]>>) {
        self.state.0.lock().unwrap().songs = songs.into();
    }

    /// Makes the songs of `ids` favorites, for what reads the favorites from now on.
    pub fn star(&self, ids: &[&str]) {
        let mut state = self.state.0.lock().unwrap();
        state.starred.extend(ids.iter().map(|id| (*id).to_owned()));
    }

    /// Keeps back the pages of songs until [`release`](Self::release). A page has the songs the
    /// server has when it is let go.
    pub fn hold(&self) {
        self.state.0.lock().unwrap().held = true;
    }

    pub fn release(&self) {
        self.state.0.lock().unwrap().held = false;
        self.state.1.notify_all();
    }

    /// Fails the pages of songs until [`accept`](Self::accept).
    pub fn refuse(&self) {
        self.state.0.lock().unwrap().refused = true;
    }

    pub fn accept(&self) {
        self.state.0.lock().unwrap().refused = false;
    }

    /// How many pages of songs were asked for.
    pub fn reads(&self) -> usize {
        self.state.0.lock().unwrap().reads
    }
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let Some((method, params)) = read_request(&mut stream) else {
        return;
    };
    let param = |name: &str| {
        params
            .iter()
            .find(|(param, _)| param == name)
            .and_then(|(_, value)| value.parse::<usize>().ok())
    };
    let (lock, released) = &**shared;
    let mut body = json!({ "status": "ok", "version": "1.16.1" });
    match method.as_str() {
        "search3" => {
            let mut state = lock.lock().unwrap();
            state.reads += 1;
            let state = released.wait_while(state, |state| state.held).unwrap();
            if state.refused {
                body["status"] = json!("failed");
                body["error"] = json!({ "code": 0, "message": "Refused" });
            } else {
                let offset = param("songOffset").unwrap_or(0).min(state.songs.len());
                let count = param("songCount").unwrap_or(state.songs.len());
                let end = offset.saturating_add(count).min(state.songs.len());
                body["searchResult3"] = json!({ "song": state.songs[offset..end] });
            }
        }
        "getStarred2" => {
            let state = lock.lock().unwrap();
            let starred: Vec<&Value> = state
                .songs
                .iter()
                .filter(|song| {
                    song["id"]
                        .as_str()
                        .is_some_and(|id| state.starred.contains(id))
                })
                .collect();
            body["starred2"] = json!({ "song": starred });
        }
        "getPlaylists" => body["playlists"] = json!({}),
        _ => {}
    }
    let body = json!({ "subsonic-response": body }).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // A request given up on has nobody to answer
    stream.write_all(response.as_bytes()).ok();
}

/// The method a request calls, and the parameters of its query.
fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<(String, String)>)> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&bytes);
    let target = head.lines().next()?.split(' ').nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let method = path.rsplit('/').next()?.to_owned();
    let params = query
        .split('&')
        .filter_map(|param| param.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    Some((method, params))
}
