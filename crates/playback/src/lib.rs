//! Play queue and playback helpers shared by the GUI and the CLI.

mod play_tally;
mod queue;
mod song_cache;

pub use play_tally::PlayTally;
pub use queue::{Entry, EntryId, Placement, Queue, RepeatMode};
pub use song_cache::SongCache;

use subsonic::Song;

/// A track from [`open_track`], and where its bytes come from.
pub struct OpenedTrack {
    track: audio::Track,
    kept: bool,
}

impl OpenedTrack {
    /// Whether it plays the file kept on this computer rather than a
    /// download. A kept file that fails with an
    /// [`is_track_error`](audio::Error::is_track_error) may be damaged:
    /// [`SongCache::forget`] it and open the song again.
    pub fn is_kept(&self) -> bool {
        self.kept
    }

    pub fn into_track(self) -> audio::Track {
        self.track
    }
}

/// Opens `song` for the audio engine: from `cache` when it holds the whole
/// file, otherwise by starting its download, which `cache` then keeps once
/// it has all arrived. The download's cancellation is wired into the track,
/// so replacing or stopping the track never waits on the network.
///
/// Looking in `cache` blocks on the disk: run it where blocking is allowed,
/// such as GPUI's background executor.
pub async fn open_track(
    client: &client::Client,
    cache: Option<&SongCache>,
    song: &Song,
) -> Result<OpenedTrack, client::Error> {
    let server = client.server().base_url();
    if let Some(kept) = cache.and_then(|cache| cache.open(server, song)) {
        let len = kept.byte_len();
        let track = audio::Track::with_known_len(kept.into_file(), len);
        return Ok(OpenedTrack {
            track: with_hints(track, song.content_type(), song),
            kept: true,
        });
    }

    let download = client.open_stream(song.id()).await?;
    let content_type = download.content_type().map(str::to_owned);
    let handle = download.handle();
    if let Some(cache) = cache.filter(|_| is_keepable(song, download.content_length())) {
        // Kept only if it then arrives whole and as long as the response
        // says; see `DownloadHandle::completed`.
        cache.keep_when_complete(server, song.id(), handle.clone());
    }
    // When the end of the file is a range request away, letting the decoder
    // know the length costs little and lets it seek MP3 and Ogg by position
    // instead of reading every page before the target.
    let len = download
        .content_length()
        .filter(|_| download.is_random_access());
    let track = match len {
        Some(len) => audio::Track::with_known_len(download, len),
        None => audio::Track::new(download),
    };
    let track = track.cancel_with(move || handle.cancel());
    Ok(OpenedTrack {
        track: with_hints(track, content_type.as_deref(), song),
        kept: false,
    })
}

/// Whether a response of `content_length` bytes for `song` is worth
/// keeping. One of another length than the song's would never be opened
/// from the cache: the song's size is out of date, or the file is not the
/// original.
fn is_keepable(song: &Song, content_length: Option<u64>) -> bool {
    song.size().is_none_or(|size| content_length == Some(size))
}

/// Helps the format probe with the MIME type and the song's extension.
fn with_hints(mut track: audio::Track, content_type: Option<&str>, song: &Song) -> audio::Track {
    if let Some(content_type) = content_type {
        track = track.mime_type(content_type);
    }
    if let Some(suffix) = song.suffix() {
        track = track.extension(suffix);
    }
    track
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use catalog::SongStore;
    use futures::executor::block_on;
    use subsonic::Credentials;

    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    fn song(id: &str, size: usize) -> Song {
        serde_json::from_str(&format!(
            r#"{{"id":"{id}","title":"Song","suffix":"flac","size":{size}}}"#
        ))
        .unwrap()
    }

    fn client(address: &str) -> client::Client {
        let server = client::Server::new(address, Credentials::api_key("key")).unwrap();
        client::Client::new(net::Client::new("test").unwrap(), server)
    }

    /// Accepts connections on another thread and hands each to `answer`.
    fn listen(answer: impl Fn(TcpStream) + Send + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(socket) = socket else { return };
                answer(socket);
            }
        });
        address
    }

    fn read_request(socket: &TcpStream) -> BufReader<TcpStream> {
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 2 {
            line.clear();
        }
        reader
    }

    /// Answers every request with `body`, saying it sends ranges of it when
    /// `ranges` is set, the way Navidrome serves an original file.
    fn serve(body: &'static [u8], ranges: bool) -> String {
        listen(move |mut socket| {
            read_request(&socket);
            let accept_ranges = if ranges {
                "Accept-Ranges: bytes\r\n"
            } else {
                ""
            };
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\n{accept_ranges}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).ok();
            socket.write_all(body).ok();
        })
    }

    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < WAIT {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_kept_song_opens_without_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let store = SongStore::at(dir.path(), catalog::SONG_CAPACITY);
        let connections = Arc::new(AtomicUsize::new(0));
        // Closes every connection at once, so any request fails.
        let client = client(&listen({
            let connections = connections.clone();
            move |_| {
                connections.fetch_add(1, Ordering::SeqCst);
            }
        }));
        let song = song("so-1", 4);
        store
            .put(client.server().base_url(), song.id(), b"flac")
            .unwrap();
        let cache = SongCache::new(store);

        let opened = block_on(open_track(&client, Some(&cache), &song)).unwrap();
        assert!(opened.is_kept());
        assert_eq!(connections.load(Ordering::SeqCst), 0);
        assert!(block_on(open_track(&client, None, &song)).is_err());
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_streamed_song_is_kept_once_it_has_arrived() {
        const BODY: &[u8] = b"not really flac, but long enough to arrive";
        for ranges in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = SongStore::at(dir.path(), catalog::SONG_CAPACITY);
            let client = client(&serve(BODY, ranges));
            let server = client.server().base_url().clone();
            let song = song("so-1", BODY.len());
            let cache = SongCache::new(store.clone());
            // The body arrives while the track is held, as the engine holds it.
            let opened = block_on(open_track(&client, Some(&cache), &song)).unwrap();
            assert!(!opened.is_kept());
            assert!(
                wait_until(|| store.open(&server, song.id(), None).is_some()),
                "not kept when the server sends ranges: {ranges}"
            );
            let kept = store.open(&server, song.id(), None).unwrap();
            assert_eq!(kept.byte_len(), BODY.len() as u64);
        }
    }

    #[test]
    fn dropping_the_track_ends_its_download() {
        let (closed, closed_receiver) = mpsc::channel();
        // Sends part of the body, then waits for the client to go.
        let client = client(&listen(move |mut socket| {
            let mut reader = read_request(&socket);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nContent-Length: 100\r\n\r\nfla")
                .ok();
            let _ = reader.read(&mut [0; 1]);
            closed.send(()).ok();
        }));
        let dir = tempfile::tempdir().unwrap();
        let cache = SongCache::new(SongStore::at(dir.path(), catalog::SONG_CAPACITY));
        let opened = block_on(open_track(&client, Some(&cache), &song("so-1", 100))).unwrap();
        drop(opened);
        closed_receiver
            .recv_timeout(WAIT)
            .expect("the download goes on without its track");
    }

    #[test]
    fn only_a_response_as_long_as_the_song_is_keepable() {
        assert!(is_keepable(&song("so-1", 4), Some(4)));
        assert!(!is_keepable(&song("so-1", 4), Some(5)));
        assert!(!is_keepable(&song("so-1", 4), None));
        let no_size: Song = serde_json::from_str(r#"{"id":"so-1","title":"Song"}"#).unwrap();
        assert!(is_keepable(&no_size, None));
    }
}
