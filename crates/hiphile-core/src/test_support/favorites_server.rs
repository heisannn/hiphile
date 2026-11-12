//! A Subsonic server on a local port that serves the favorites and takes changes to them, for
//! the tests of the core and of the views.

use std::collections::HashSet;
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

use super::http;

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
        let state = Shared::default();
        state.0.lock().unwrap().songs = songs.iter().map(|song| (*song).to_owned()).collect();
        let url = http::listen(state.clone(), serve);
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

fn serve(mut stream: TcpStream, shared: &Shared) {
    let Some(request) = http::read_request(&mut stream) else {
        return;
    };
    let method = &request.method;
    let named = request
        .params
        .iter()
        .find(|(name, _)| ["id", "albumId", "artistId"].contains(&name.as_str()));
    let id = named.map_or("", |(_, id)| id.as_str()).to_owned();
    let named = named.map_or(String::new(), |(name, id)| format!("{name}={id}"));

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
        .wait_while(state, |state| state.held.contains(method))
        .unwrap();
    let mut body = http::ok();
    let refused_once = state.refused_once.remove(method);
    if refused_once || state.refused.contains(method) {
        http::fail(&mut body, 0, "Refused");
    } else if is_write {
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
    http::write_json(&mut stream, body);
}
