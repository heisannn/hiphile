//! A Subsonic server on a local port that serves artists, their albums, top songs and similar
//! artists, and the songs of their albums, for the tests and the shots of the views.

use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

use super::http;

type Covers = Arc<dyn Fn(&str) -> Vec<u8> + Send + Sync>;

#[derive(Default)]
struct ServerState {
    artists: Vec<Value>,
    albums: HashMap<String, Vec<Value>>,
    /// By the artist's name, which the server is asked by.
    top_songs: HashMap<String, Vec<Value>>,
    similar: HashMap<String, Vec<Value>>,
    album_songs: HashMap<String, Vec<Value>>,
    covers: Option<Covers>,
    held: HashSet<String>,
    refused: HashSet<String>,
    asked: HashMap<String, usize>,
}

type Shared = Arc<(Mutex<ServerState>, Condvar)>;

pub struct ArtistsServer {
    url: String,
    state: Shared,
}

impl Default for ArtistsServer {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtistsServer {
    /// Knows of no artists, and of no favorites or playlists.
    pub fn new() -> Self {
        let state = Shared::default();
        let url = http::listen(state.clone(), serve);
        Self { url, state }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// The artists `getArtists` lists, each as the server has one, in their order.
    pub fn set_artists(&self, artists: Vec<Value>) {
        self.change(|state| state.artists = artists);
    }

    /// The albums `getArtist` gives for `artist`, who is otherwise as listed or else named
    /// `Artist {id}`. It fails for the artist until this is called.
    pub fn set_albums(&self, artist: &str, albums: Vec<Value>) {
        self.change(|state| {
            state.albums.insert(artist.to_owned(), albums);
        });
    }

    /// What `getTopSongs` gives for the artist named `name`. It gives none for others.
    pub fn set_top_songs(&self, name: &str, songs: Vec<Value>) {
        self.change(|state| {
            state.top_songs.insert(name.to_owned(), songs);
        });
    }

    /// The similar artists `getArtistInfo2` gives for `artist`. It gives none for others.
    pub fn set_similar(&self, artist: &str, similar: Vec<Value>) {
        self.change(|state| {
            state.similar.insert(artist.to_owned(), similar);
        });
    }

    /// What `getAlbum` gives for `album`, which it fails for until this is called.
    pub fn set_album_songs(&self, album: &str, songs: Vec<Value>) {
        self.change(|state| {
            state.album_songs.insert(album.to_owned(), songs);
        });
    }

    /// Answers `getCoverArt` with what `cover` makes of the cover's ID, an image file. Without
    /// it, there is no cover.
    pub fn set_covers(&self, cover: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static) {
        self.change(|state| state.covers = Some(Arc::new(cover)));
    }

    /// Keeps back the answers to `method` until it is let go. What they say is what the server
    /// has then.
    pub fn hold(&self, method: &str) {
        self.change(|state| {
            state.held.insert(method.to_owned());
        });
    }

    pub fn release(&self, method: &str) {
        self.change(|state| {
            state.held.remove(method);
        });
        self.state.1.notify_all();
    }

    pub fn refuse(&self, method: &str) {
        self.change(|state| {
            state.refused.insert(method.to_owned());
        });
    }

    pub fn accept(&self, method: &str) {
        self.change(|state| {
            state.refused.remove(method);
        });
    }

    /// How many times `method` has been asked for.
    pub fn asked(&self, method: &str) -> usize {
        let state = self.state.0.lock().unwrap();
        state.asked.get(method).copied().unwrap_or_default()
    }

    fn change(&self, change: impl FnOnce(&mut ServerState)) {
        change(&mut self.state.0.lock().unwrap());
    }
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let Some(request) = http::read_request(&mut stream) else {
        return;
    };
    let method = &request.method;
    let param = |name: &str| request.value(name).unwrap_or_default().to_owned();

    let (lock, released) = &**shared;
    let mut state = lock.lock().unwrap();
    *state.asked.entry(method.clone()).or_default() += 1;
    let state = released
        .wait_while(state, |state| state.held.contains(method))
        .unwrap();
    if method == "getCoverArt" {
        let cover = state.covers.clone();
        drop(state);
        let image = cover.map(|cover| cover(&param("id")));
        http::write_image(&mut stream, image.as_deref());
        return;
    }
    let mut body = http::ok();
    let failed = |body: &mut Value, message: &str| http::fail(body, 70, message);
    if state.refused.contains(method) {
        failed(&mut body, "Refused");
    } else {
        match method.as_str() {
            "getArtists" => {
                body["artists"] = json!({ "index": [{ "name": "#", "artist": state.artists }] });
            }
            "getArtist" => {
                let id = param("id");
                match state.albums.get(&id) {
                    Some(albums) => {
                        let listed = state
                            .artists
                            .iter()
                            .find(|artist| artist["id"] == id.as_str());
                        let mut artist = listed
                            .cloned()
                            .unwrap_or_else(|| json!({ "id": id, "name": format!("Artist {id}") }));
                        artist["album"] = json!(albums);
                        body["artist"] = artist;
                    }
                    None => failed(&mut body, "Artist not found"),
                }
            }
            "getTopSongs" => {
                let songs = state.top_songs.get(&param("artist")).cloned();
                body["topSongs"] = json!({ "song": songs.unwrap_or_default() });
            }
            "getArtistInfo2" => {
                let similar = state.similar.get(&param("id")).cloned();
                body["artistInfo2"] = json!({ "similarArtist": similar.unwrap_or_default() });
            }
            "getAlbum" => {
                let id = param("id");
                match state.album_songs.get(&id) {
                    Some(songs) => {
                        body["album"] =
                            json!({ "id": id, "name": format!("Album {id}"), "song": songs });
                    }
                    None => failed(&mut body, "Album not found"),
                }
            }
            "getStarred2" => {
                body["starred2"] = json!({ "song": [], "album": [], "artist": [] });
            }
            "getPlaylists" => body["playlists"] = json!({ "playlist": [] }),
            _ => {}
        }
    }
    drop(state);
    http::write_json(&mut stream, body);
}
