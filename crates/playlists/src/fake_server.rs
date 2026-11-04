//! A server for tests to read and save playlists on, which can hold requests
//! back or refuse them to set up what a real one does only sometimes.

use std::collections::HashSet;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use serde_json::{Value, json};

const WRITES: [&str; 3] = ["createPlaylist", "updatePlaylist", "deletePlaylist"];

/// A request as the server took it, from its URL or its form.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) verb: String,
    pub(crate) method: String,
    params: Vec<(String, String)>,
}

impl Request {
    pub(crate) fn values(&self, name: &str) -> Vec<&str> {
        self.params
            .iter()
            .filter(|(param, _)| param == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values(name).first().copied()
    }
}

struct FakePlaylist {
    id: String,
    name: String,
    songs: Vec<String>,
}

#[derive(Default)]
struct State {
    playlists: Vec<FakePlaylist>,
    next_id: u32,
    requests: Vec<Request>,
    held: HashSet<&'static str>,
    refused: HashSet<&'static str>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub(crate) struct FakeServer {
    url: String,
    state: Shared,
}

impl FakeServer {
    /// Serves `playlists`, given as id, name and song ids.
    pub(crate) fn new(playlists: &[(&str, &str, &[&str])]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Shared::default();
        state.0.lock().unwrap().playlists = playlists
            .iter()
            .map(|(id, name, songs)| FakePlaylist {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                songs: songs.iter().map(|song| (*song).to_owned()).collect(),
            })
            .collect();
        let shared = state.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                thread::spawn(move || serve(stream, &state));
            }
        });
        Self { url, state }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// Keeps back the answers to `method`. A read has what the server has when
    /// it arrives, and a write takes effect only once let go.
    pub(crate) fn hold(&self, method: &'static str) {
        self.state.0.lock().unwrap().held.insert(method);
    }

    pub(crate) fn release(&self, method: &'static str) {
        self.state.0.lock().unwrap().held.remove(method);
        self.state.1.notify_all();
    }

    pub(crate) fn refuse(&self, method: &'static str) {
        self.state.0.lock().unwrap().refused.insert(method);
    }

    pub(crate) fn accept(&self, method: &'static str) {
        self.state.0.lock().unwrap().refused.remove(method);
    }

    /// The requests for `method` so far.
    pub(crate) fn requests(&self, method: &str) -> Vec<Request> {
        let state = self.state.0.lock().unwrap();
        state
            .requests
            .iter()
            .filter(|request| request.method == method)
            .cloned()
            .collect()
    }

    /// The songs the server has for the playlist.
    pub(crate) fn songs(&self, playlist: &str) -> Vec<String> {
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
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let is_write = WRITES.contains(&request.method.as_str());
    let (lock, released) = &**shared;
    let mut state = lock.lock().unwrap();
    state.requests.push(request.clone());
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
    let mut body = json!({ "status": "ok", "version": "1.16.1" });
    match answer {
        Ok(payload) => {
            if let (Some(body), Value::Object(payload)) = (body.as_object_mut(), payload) {
                body.extend(payload);
            }
        }
        Err(message) => {
            body["status"] = json!("failed");
            body["error"] = json!({ "code": 0, "message": message });
        }
    }
    let body = json!({ "subsonic-response": body }).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // A request given up on has nobody to answer.
    stream.write_all(response.as_bytes()).ok();
}

fn answer(request: &Request, state: &mut State) -> Result<Value, &'static str> {
    if state.refused.contains(request.method.as_str()) {
        return Err("Refused");
    }
    match request.method.as_str() {
        "getPlaylists" => {
            let listed: Vec<Value> = state.playlists.iter().map(playlist_json).collect();
            Ok(json!({ "playlists": { "playlist": listed } }))
        }
        "getPlaylist" => {
            let id = request.value("id").unwrap_or_default();
            let found = state.playlists.iter().find(|listed| listed.id == id);
            found
                .map(|playlist| json!({ "playlist": with_songs(playlist) }))
                .ok_or("Not found")
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
                });
                id
            };
            let playlist = state.playlists.iter().find(|listed| listed.id == id);
            Ok(json!({ "playlist": with_songs(playlist.ok_or("Not found")?) }))
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

fn playlist_json(playlist: &FakePlaylist) -> Value {
    json!({
        "id": playlist.id,
        "name": playlist.name,
        "owner": "alice",
        "songCount": playlist.songs.len(),
    })
}

fn with_songs(playlist: &FakePlaylist) -> Value {
    let mut json = playlist_json(playlist);
    json["entry"] = playlist
        .songs
        .iter()
        .map(|song| json!({ "id": song, "title": song }))
        .collect();
    json
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    let body_start = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = end + 4;
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .map_or(0, |length| length.trim().parse().unwrap());
            if bytes.len() >= end + length {
                break end;
            }
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&bytes[..body_start]);
    let mut request_line = head.lines().next()?.split(' ');
    let verb = request_line.next()?.to_owned();
    let (path, query) = {
        let target = request_line.next()?;
        target.split_once('?').unwrap_or((target, ""))
    };
    let method = path.rsplit('/').next()?.to_owned();
    let form = String::from_utf8_lossy(&bytes[body_start..]);
    let params = query
        .split('&')
        .chain(form.split('&'))
        .filter_map(|param| param.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    Some(Request {
        verb,
        method,
        params,
    })
}
