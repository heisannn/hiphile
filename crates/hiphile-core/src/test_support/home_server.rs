//! A Subsonic server on a local port with what the home screen shows, for the tests and the shots
//! of the views: albums, the lists the server keeps of them, their songs and their covers.

use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

/// The lists of `getAlbumList2` it keeps.
const LISTS: [&str; 4] = ["newest", "recent", "frequent", "random"];

struct FakeAlbum {
    id: String,
    name: String,
    artist: String,
    songs: Vec<Value>,
}

#[derive(Default)]
struct State {
    albums: Vec<FakeAlbum>,
    lists: HashMap<String, Vec<String>>,
    covers: HashMap<String, Arc<[u8]>>,
    cover: Option<Arc<[u8]>>,
    asked: HashMap<String, usize>,
    held: HashSet<String>,
    refused: HashSet<String>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

/// What a request is named by in [`HomeServer::hold`], [`HomeServer::refuse`] and
/// [`HomeServer::asked`]: a list of `getAlbumList2` by its type, such as `"newest"`, and anything
/// else by its method, such as `"getAlbum"`.
pub struct HomeServer {
    url: String,
    state: Shared,
}

impl HomeServer {
    /// Serves no albums, no favorites and no playlists.
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Shared::default();
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

    /// An album by `artist`, whose artist's id and cover's are named after it.
    pub fn add_album(&self, id: &str, name: &str, artist: &str) {
        self.lock().albums.push(FakeAlbum {
            id: id.to_owned(),
            name: name.to_owned(),
            artist: artist.to_owned(),
            songs: Vec::new(),
        });
    }

    /// A song on `album`, by its artist, last played at `played`, in RFC 3339, if ever.
    pub fn add_song(&self, album: &str, id: &str, title: &str, played: Option<&str>) {
        let mut state = self.lock();
        let album = state
            .albums
            .iter_mut()
            .find(|listed| listed.id == album)
            .expect("the album added first");
        let mut song = json!({
            "id": id,
            "title": title,
            "album": album.name,
            "albumId": album.id,
            "artist": album.artist,
            "artistId": artist_id(&album.artist),
            "coverArt": album.id,
            "duration": 200 + album.songs.len() * 17,
        });
        if let Some(played) = played {
            song["played"] = json!(played);
        }
        album.songs.push(song);
    }

    /// What `list` holds, in its order: one of `"newest"`, `"recent"`, `"frequent"` and
    /// `"random"`. Each is empty until then.
    pub fn set_list(&self, list: &str, albums: &[&str]) {
        assert!(LISTS.contains(&list), "no list {list:?}");
        let albums = albums.iter().map(|album| (*album).to_owned()).collect();
        self.lock().lists.insert(list.to_owned(), albums);
    }

    /// The image of the cover of `album`, which is otherwise the one of [`Self::set_cover`].
    pub fn set_cover_of(&self, album: &str, image: Vec<u8>) {
        self.lock().covers.insert(album.to_owned(), image.into());
    }

    /// The image of every cover with none of its own. There is none until then.
    pub fn set_cover(&self, image: Vec<u8>) {
        self.lock().cover = Some(image.into());
    }

    /// Keeps back the answers to `what` until let go.
    pub fn hold(&self, what: &str) {
        self.lock().held.insert(what.to_owned());
    }

    pub fn release(&self, what: &str) {
        self.lock().held.remove(what);
        self.state.1.notify_all();
    }

    pub fn refuse(&self, what: &str) {
        self.lock().refused.insert(what.to_owned());
    }

    pub fn accept(&self, what: &str) {
        self.lock().refused.remove(what);
    }

    /// How many times `what` has been asked for.
    pub fn asked(&self, what: &str) -> usize {
        self.lock().asked.get(what).copied().unwrap_or(0)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.0.lock().unwrap()
    }
}

impl Default for HomeServer {
    fn default() -> Self {
        Self::new()
    }
}

fn artist_id(artist: &str) -> String {
    format!("artist-{artist}")
}

fn album_json(album: &FakeAlbum) -> Value {
    json!({
        "id": album.id,
        "name": album.name,
        "artist": album.artist,
        "artistId": artist_id(&album.artist),
        "coverArt": album.id,
        "songCount": album.songs.len(),
    })
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    let mut bytes = [0; 4096];
    let Ok(read) = stream.read(&mut bytes) else {
        return;
    };
    let head = String::from_utf8_lossy(&bytes[..read]).into_owned();
    let target = head.split(' ').nth(1).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let method = path.rsplit('/').next().unwrap_or_default().to_owned();
    let param = |name: &str| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
    };
    let what = match method.as_str() {
        "getAlbumList2" => param("type").unwrap_or_default(),
        _ => method.clone(),
    };

    let (lock, released) = &**shared;
    let mut state = lock.lock().unwrap();
    *state.asked.entry(what.clone()).or_default() += 1;
    let state = released
        .wait_while(state, |state| state.held.contains(&what))
        .unwrap();
    if method == "getCoverArt" {
        let id = param("id").unwrap_or_default();
        let image = state.covers.get(&id).or(state.cover.as_ref()).cloned();
        drop(state);
        write_cover(&mut stream, image.as_deref());
        return;
    }
    let answer = if state.refused.contains(&what) {
        Err("Refused")
    } else {
        answer(&state, &method, &what, &param).unwrap_or(Err("Not implemented"))
    };
    drop(state);
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
    // A request given up on has nobody to answer
    stream.write_all(response.as_bytes()).ok();
}

fn answer(
    state: &State,
    method: &str,
    what: &str,
    param: &dyn Fn(&str) -> Option<String>,
) -> Option<Result<Value, &'static str>> {
    let found = |id: &str| state.albums.iter().find(|album| album.id == id);
    Some(match method {
        "getAlbumList2" if LISTS.contains(&what) => {
            let size = param("size").and_then(|size| size.parse().ok());
            let listed: Vec<Value> = state
                .lists
                .get(what)
                .into_iter()
                .flatten()
                .filter_map(|id| found(id))
                .take(size.unwrap_or(usize::MAX))
                .map(album_json)
                .collect();
            Ok(json!({ "albumList2": { "album": listed } }))
        }
        "getAlbum" => {
            let id = param("id").unwrap_or_default();
            found(&id).ok_or("Album not found").map(|album| {
                let mut json = album_json(album);
                json["song"] = json!(album.songs);
                json!({ "album": json })
            })
        }
        "getSimilarSongs2" => Ok(json!({ "similarSongs2": {} })),
        "getStarred2" => Ok(json!({ "starred2": {} })),
        "getPlaylists" => Ok(json!({ "playlists": {} })),
        _ => return None,
    })
}

/// The image, or Not Found without one.
fn write_cover(stream: &mut TcpStream, image: Option<&[u8]>) {
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
