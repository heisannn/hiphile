//! A Subsonic server on a local port that serves the favorites and takes changes to them, for
//! the tests of the core and of the views.

use std::collections::HashSet;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

#[derive(Default)]
struct ServerState {
    songs: Vec<String>,
    /// `star` and `unstar` as they came, with the parameter they named.
    writes: Vec<String>,
    reads: usize,
    held: HashSet<String>,
    refused: HashSet<String>,
    /// Refused the next time they come, then not again.
    refused_once: HashSet<String>,
}

type Shared = Arc<(Mutex<ServerState>, Condvar)>;

pub struct FavoritesServer {
    url: String,
    state: Shared,
}

impl FavoritesServer {
    /// Serves the songs `songs` names, an album and an artist.
    pub fn new(songs: &[&str]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Shared::default();
        state.0.lock().unwrap().songs = songs.iter().map(|song| (*song).to_owned()).collect();
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                std::thread::spawn(move || serve(stream, &state));
            }
        });
        Self { url, state }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Keeps back the answers to `method`. A read has what the server has when it arrives, and
    /// a write takes effect only once let go.
    pub fn hold(&self, method: &str) {
        let mut state = self.state.0.lock().unwrap();
        state.held.insert(method.to_owned());
    }

    pub fn release(&self, method: &str) {
        self.state.0.lock().unwrap().held.remove(method);
        self.state.1.notify_all();
    }

    pub fn refuse(&self, method: &str) {
        let mut state = self.state.0.lock().unwrap();
        state.refused.insert(method.to_owned());
    }

    pub fn accept(&self, method: &str) {
        self.state.0.lock().unwrap().refused.remove(method);
    }

    pub fn refuse_once(&self, method: &str) {
        let mut state = self.state.0.lock().unwrap();
        state.refused_once.insert(method.to_owned());
    }

    pub fn writes(&self) -> Vec<String> {
        self.state.0.lock().unwrap().writes.clone()
    }

    pub fn reads(&self) -> usize {
        self.state.0.lock().unwrap().reads
    }
}

/// A song as the server lists it, on an album and by an artist named after it.
pub fn song_json(id: &str) -> Value {
    json!({ "id": id, "title": id, "artistId": format!("artist-{id}"), "albumId": format!("album-{id}") })
}

fn serve(mut stream: std::net::TcpStream, shared: &Shared) {
    let mut bytes = [0; 4096];
    let Ok(read) = stream.read(&mut bytes) else {
        return;
    };
    let head = String::from_utf8_lossy(&bytes[..read]).into_owned();
    let target = head.split(' ').nth(1).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let method = path.rsplit('/').next().unwrap_or_default().to_owned();
    let named = query
        .split('&')
        .find(|param| {
            ["id=", "albumId=", "artistId="]
                .iter()
                .any(|name| param.starts_with(name))
        })
        .unwrap_or_default()
        .to_owned();

    let (lock, released) = &**shared;
    let mut state = lock.lock().unwrap();
    let is_write = method == "star" || method == "unstar";
    if is_write {
        state.writes.push(format!("{method} {named}"));
    } else if method == "getStarred2" {
        state.reads += 1;
    }
    // A read has what the server has when it arrives, and a write takes effect
    // once let go.
    let songs_when_read = state.songs.clone();
    let mut state = released
        .wait_while(state, |state| state.held.contains(&method))
        .unwrap();
    let mut body = json!({ "status": "ok", "version": "1.16.1" });
    let refused_once = state.refused_once.remove(&method);
    if refused_once || state.refused.contains(&method) {
        body["status"] = json!("failed");
        body["error"] = json!({ "code": 0, "message": "Refused" });
    } else if is_write {
        let id = named.split_once('=').map_or("", |(_, id)| id).to_owned();
        if method == "star" {
            state.songs.insert(0, id);
        } else {
            state.songs.retain(|song| *song != id);
        }
    } else if method == "getAlbum" {
        body["album"] = json!({
            "id": "al",
            "name": "Album",
            "song": [song_json("t1"), song_json("t2")],
        });
    } else {
        let songs: Vec<Value> = songs_when_read.iter().map(|id| song_json(id)).collect();
        body["starred2"] = json!({
            "song": songs,
            "album": [{ "id": "al", "name": "Album", "artistId": "artist-al" }],
            "artist": [{ "id": "ar", "name": "Artist", "albumCount": 2 }],
        });
    }
    drop(state);
    let body = json!({ "subsonic-response": body }).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).ok();
}
