//! A Subsonic server on a local port that serves albums, their songs and their covers, for the
//! tests and the shots of the views that show the library.

use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use serde_json::{Value, json};

type Covers = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

#[derive(Default)]
struct State {
    albums: Vec<Value>,
    asked: HashMap<String, usize>,
    held: HashSet<String>,
    refused: HashSet<String>,
    covers: Option<Covers>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub struct LibraryServer {
    url: String,
    state: Shared,
}

impl LibraryServer {
    /// Serves `albums`, each as `getAlbum` gives it, with its songs under `song`. The list of them
    /// leaves the songs out. The favorites and the playlists are there and empty.
    pub fn new(albums: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Shared::default();
        state.0.lock().unwrap().albums = albums;
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

    /// Puts `albums` in place of those it has, as when another device changed the library.
    pub fn set_albums(&self, albums: Vec<Value>) {
        self.state.0.lock().unwrap().albums = albums;
    }

    /// Answers `getCoverArt` with what `cover` gives for the cover's id, an image, or with 404
    /// for `None`. Until then every cover is missing.
    pub fn serve_covers(&self, cover: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static) {
        self.state.0.lock().unwrap().covers = Some(Arc::new(cover));
    }

    /// Keeps back the answers to `method` until it is let go. An answer has what the server has
    /// once it is let go.
    pub fn hold(&self, method: &str) {
        self.state.0.lock().unwrap().held.insert(method.to_owned());
    }

    pub fn release(&self, method: &str) {
        self.state.0.lock().unwrap().held.remove(method);
        self.state.1.notify_all();
    }

    pub fn refuse(&self, method: &str) {
        self.state
            .0
            .lock()
            .unwrap()
            .refused
            .insert(method.to_owned());
    }

    pub fn accept(&self, method: &str) {
        self.state.0.lock().unwrap().refused.remove(method);
    }

    /// How many times `method` was asked for.
    pub fn asked(&self, method: &str) -> usize {
        let state = self.state.0.lock().unwrap();
        state.asked.get(method).copied().unwrap_or(0)
    }
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let Some((method, params)) = read_request(&mut stream) else {
        return;
    };
    let (lock, released) = &**shared;
    let mut state = lock.lock().unwrap();
    *state.asked.entry(method.clone()).or_default() += 1;
    let mut state = released
        .wait_while(state, |state| state.held.contains(&method))
        .unwrap();
    let param = |name: &str| {
        params
            .iter()
            .find(|(param, _)| param == name)
            .map_or("", |(_, value)| value.as_str())
    };
    if method == "getCoverArt" {
        let cover = state.covers.clone();
        drop(state);
        let image = cover.and_then(|cover| cover(param("id")));
        write_image(&mut stream, image.as_deref());
        return;
    }
    let answer = if state.refused.contains(&method) {
        Err("The server refused.")
    } else {
        answer(&method, param("id"), param("offset"), &mut state)
    };
    drop(state);
    let mut body = json!({ "status": "ok", "version": "1.16.1" });
    match answer {
        Ok(Value::Object(payload)) => body.as_object_mut().unwrap().extend(payload),
        Ok(_) => {}
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
    // A request given up on has nobody to answer
    stream.write_all(response.as_bytes()).ok();
}

fn answer(method: &str, id: &str, offset: &str, state: &mut State) -> Result<Value, &'static str> {
    match method {
        "getAlbumList2" => {
            // One page has them all
            let listed: Vec<Value> = if offset.parse::<usize>().unwrap_or(0) > 0 {
                Vec::new()
            } else {
                state.albums.iter().map(without_songs).collect()
            };
            Ok(json!({ "albumList2": { "album": listed } }))
        }
        "getAlbum" => {
            let found = state.albums.iter().find(|album| album["id"] == id);
            found
                .map(|album| json!({ "album": album }))
                .ok_or("Album not found.")
        }
        "getStarred2" => Ok(json!({ "starred2": {} })),
        "getPlaylists" => Ok(json!({ "playlists": {} })),
        _ => Ok(json!({})),
    }
}

fn without_songs(album: &Value) -> Value {
    let mut album = album.clone();
    if let Some(album) = album.as_object_mut() {
        album.remove("song");
    }
    album
}

fn write_image(stream: &mut TcpStream, image: Option<&[u8]>) {
    match image {
        Some(image) => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                image.len()
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(image).ok();
        }
        None => {
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .ok();
        }
    }
}

/// The method asked for and its parameters, from the query and from a form.
fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<(String, String)>)> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    let body_start = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = end + 4;
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .map_or(0, |length| length.trim().parse().unwrap_or(0));
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
    let target = head.lines().next()?.split(' ').nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let method = path.rsplit('/').next()?;
    let method = method.split('.').next().unwrap_or(method).to_owned();
    let form = String::from_utf8_lossy(&bytes[body_start..]);
    let params = query
        .split('&')
        .chain(form.split('&'))
        .filter_map(|param| param.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    Some((method, params))
}
