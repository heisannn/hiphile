//! A Subsonic server on a local port that serves playlists and takes changes to them, for the
//! tests of the core and of the views.

use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

use super::http::{self, Request};

const WRITES: [&str; 3] = ["createPlaylist", "updatePlaylist", "deletePlaylist"];

struct FakePlaylist {
    id: String,
    name: String,
    songs: Vec<String>,
    /// What the server says of it besides, such as its comment.
    details: Value,
}

#[derive(Default)]
struct State {
    playlists: Vec<FakePlaylist>,
    next_id: u32,
    requests: Vec<Request>,
    readonly: HashSet<String>,
    held: HashSet<&'static str>,
    refused: HashSet<&'static str>,
    /// What the server says of a song besides its title.
    songs: HashMap<String, Value>,
    /// The songs starred, once the server serves favorites.
    starred: Option<Vec<String>>,
    covers: HashMap<String, Arc<[u8]>>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub struct PlaylistsServer {
    url: String,
    state: Shared,
}

impl PlaylistsServer {
    /// Serves `playlists`, given as id, name and song ids.
    pub fn new(playlists: &[(&str, &str, &[&str])]) -> Self {
        let state = Shared::default();
        state.0.lock().unwrap().playlists = playlists
            .iter()
            .map(|(id, name, songs)| FakePlaylist {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                songs: songs.iter().map(|song| (*song).to_owned()).collect(),
                details: json!({}),
            })
            .collect();
        let url = http::listen(state.clone(), serve);
        Self { url, state }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Keeps back the answers to `method`. A read has what the server has when
    /// it arrives, and a write takes effect only once let go.
    pub fn hold(&self, method: &'static str) {
        self.state.0.lock().unwrap().held.insert(method);
    }

    pub fn release(&self, method: &'static str) {
        self.state.0.lock().unwrap().held.remove(method);
        self.state.1.notify_all();
    }

    pub fn refuse(&self, method: &'static str) {
        self.state.0.lock().unwrap().refused.insert(method);
    }

    pub fn accept(&self, method: &'static str) {
        self.state.0.lock().unwrap().refused.remove(method);
    }

    pub fn requests(&self, method: &str) -> Vec<Request> {
        let state = self.state.0.lock().unwrap();
        state
            .requests
            .iter()
            .filter(|request| request.method == method)
            .cloned()
            .collect()
    }

    /// Puts `songs` in `playlist` in place of those it has, as another device would.
    pub fn set_songs(&self, playlist: &str, songs: &[&str]) {
        let mut state = self.state.0.lock().unwrap();
        let found = state
            .playlists
            .iter_mut()
            .find(|listed| listed.id == playlist);
        found.unwrap().songs = songs.iter().map(|song| (*song).to_owned()).collect();
    }

    /// Makes `playlist` another user's, which alice may only read.
    pub fn make_readonly(&self, playlist: &str) {
        let mut state = self.state.0.lock().unwrap();
        state.readonly.insert(playlist.to_owned());
    }

    /// Says `details` of the song `id` besides its title, such as its artist, its duration and
    /// its cover.
    pub fn describe_song(&self, id: &str, details: Value) {
        let mut state = self.state.0.lock().unwrap();
        state.songs.insert(id.to_owned(), details);
    }

    /// Says `details` of `playlist` besides its name and songs, such as its comment.
    pub fn describe_playlist(&self, playlist: &str, details: Value) {
        let mut state = self.state.0.lock().unwrap();
        let found = state
            .playlists
            .iter_mut()
            .find(|listed| listed.id == playlist);
        found.unwrap().details = details;
    }

    /// Serves the favorites, of which `songs` are the songs. Until then they are not served.
    pub fn serve_favorites(&self, songs: &[&str]) {
        let mut state = self.state.0.lock().unwrap();
        state.starred = Some(songs.iter().map(|song| (*song).to_owned()).collect());
    }

    /// Serves `png` for the cover `id`.
    pub fn serve_cover(&self, id: &str, png: Vec<u8>) {
        let mut state = self.state.0.lock().unwrap();
        state.covers.insert(id.to_owned(), png.into());
    }

    pub fn songs(&self, playlist: &str) -> Vec<String> {
        let state = self.state.0.lock().unwrap();
        state
            .playlists
            .iter()
            .find(|listed| listed.id == playlist)
            .unwrap()
            .songs
            .clone()
    }
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let Some(request) = http::read_request(&mut stream) else {
        return;
    };
    let is_write = WRITES.contains(&request.method.as_str());
    let (lock, released) = &**shared;
    let mut state = lock.lock().unwrap();
    state.requests.push(request.clone());
    if request.method == "getCoverArt" {
        let cover = request
            .value("id")
            .and_then(|id| state.covers.get(id).cloned());
        drop(state);
        http::write_image(&mut stream, cover.as_deref());
        return;
    }
    let method = request.method.clone();
    let wait = |state| {
        released
            .wait_while(state, |state: &mut State| {
                state.held.contains(method.as_str())
            })
            .unwrap()
    };
    let answer = if is_write {
        let mut state = wait(state);
        answer(&request, &mut state)
    } else {
        let answer = answer(&request, &mut state);
        drop(wait(state));
        answer
    };
    http::write_json(&mut stream, http::answered(answer));
}

fn answer(request: &Request, state: &mut State) -> Result<Value, &'static str> {
    if state.refused.contains(request.method.as_str()) {
        return Err("Refused");
    }
    match request.method.as_str() {
        "getPlaylists" => {
            let listed: Vec<Value> = state
                .playlists
                .iter()
                .map(|playlist| playlist_json(playlist, state))
                .collect();
            Ok(json!({ "playlists": { "playlist": listed } }))
        }
        "getPlaylist" => {
            let id = request.value("id").unwrap_or_default();
            let found = state.playlists.iter().find(|listed| listed.id == id);
            found
                .map(|playlist| json!({ "playlist": with_songs(playlist, state) }))
                .ok_or("Not found")
        }
        "getStarred2" => {
            let starred = state.starred.as_ref().ok_or("Not implemented")?;
            let songs: Vec<Value> = starred.iter().map(|id| song_json(id, state)).collect();
            Ok(json!({ "starred2": { "song": songs } }))
        }
        "createPlaylist" => {
            let songs = request
                .values("songId")
                .into_iter()
                .map(str::to_owned)
                .collect();
            let id = if let Some(id) = request.value("playlistId") {
                let found = state.playlists.iter_mut().find(|listed| listed.id == id);
                found.ok_or("Not found")?.songs = songs;
                id.to_owned()
            } else {
                state.next_id += 1;
                let id = format!("new{}", state.next_id);
                state.playlists.push(FakePlaylist {
                    id: id.clone(),
                    name: request.value("name").unwrap_or_default().to_owned(),
                    songs,
                    details: json!({}),
                });
                id
            };
            let playlist = state.playlists.iter().find(|listed| listed.id == id);
            let playlist = playlist.ok_or("Not found")?;
            Ok(json!({ "playlist": with_songs(playlist, state) }))
        }
        "updatePlaylist" => {
            let id = request.value("playlistId").unwrap_or_default();
            let found = state.playlists.iter_mut().find(|listed| listed.id == id);
            let playlist = found.ok_or("Not found")?;
            if let Some(name) = request.value("name") {
                name.clone_into(&mut playlist.name);
            }
            let removed = request.values("songIndexToRemove");
            let mut position = 0;
            playlist.songs.retain(|_| {
                let keep = !removed.contains(&position.to_string().as_str());
                position += 1;
                keep
            });
            playlist
                .songs
                .extend(request.values("songIdToAdd").into_iter().map(str::to_owned));
            Ok(json!({}))
        }
        "deletePlaylist" => {
            let id = request.value("id").unwrap_or_default();
            state.playlists.retain(|listed| listed.id != id);
            Ok(json!({}))
        }
        _ => Err("Not implemented"),
    }
}

fn playlist_json(playlist: &FakePlaylist, state: &State) -> Value {
    let mut json = json!({
        "id": playlist.id,
        "name": playlist.name,
        "owner": "alice",
        "songCount": playlist.songs.len(),
    });
    if state.readonly.contains(&playlist.id) {
        json["owner"] = json!("bob");
        json["readonly"] = json!(true);
    }
    with_details(json, &playlist.details)
}

fn with_songs(playlist: &FakePlaylist, state: &State) -> Value {
    let mut json = playlist_json(playlist, state);
    json["entry"] = playlist
        .songs
        .iter()
        .map(|song| song_json(song, state))
        .collect();
    json
}

fn song_json(id: &str, state: &State) -> Value {
    let json = json!({ "id": id, "title": id });
    match state.songs.get(id) {
        Some(details) => with_details(json, details),
        None => json,
    }
}

fn with_details(mut json: Value, details: &Value) -> Value {
    if let (Value::Object(json), Value::Object(details)) = (&mut json, details) {
        json.extend(details.clone());
    }
    json
}
