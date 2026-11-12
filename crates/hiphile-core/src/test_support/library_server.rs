//! A Subsonic server on a local port that serves albums, their songs and their covers, for the
//! tests and the shots of the views that show the library.

use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

use super::http;

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
        let state = Shared::default();
        state.0.lock().unwrap().albums = albums;
        let url = http::listen(state.clone(), serve);
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
    let Some(request) = http::read_request(&mut stream) else {
        return;
    };
    let method = &request.method;
    let (lock, released) = &**shared;
    let mut state = lock.lock().unwrap();
    *state.asked.entry(method.clone()).or_default() += 1;
    let mut state = released
        .wait_while(state, |state| state.held.contains(method))
        .unwrap();
    let param = |name: &str| request.value(name).unwrap_or_default();
    if method == "getCoverArt" {
        let cover = state.covers.clone();
        drop(state);
        let image = cover.and_then(|cover| cover(param("id")));
        http::write_image(&mut stream, image.as_deref());
        return;
    }
    let answer = if state.refused.contains(method) {
        Err("The server refused.")
    } else {
        answer(method, param("id"), param("offset"), &mut state)
    };
    drop(state);
    http::write_json(&mut stream, http::answered(answer));
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
