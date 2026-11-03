use std::thread;

use catalog::{KeptSong, SONG_CAPACITY, SongStore};
use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream::FuturesUnordered;
use subsonic::SongId;
use url::Url;

/// Songs kept on disk once they have arrived whole, for
/// [`open_track`](crate::open_track) to play without the network.
///
/// Owns a thread that waits for each download to complete and writes the
/// file, one at a time, so a write never holds up the network runtime or a
/// UI thread. Cheap to clone; dropping the last clone stops the thread, and
/// downloads still arriving are then not kept.
#[derive(Clone)]
pub struct SongCache {
    store: SongStore,
    keeps: mpsc::UnboundedSender<Keep>,
}

/// A download to keep once it has arrived whole.
struct Keep {
    server: Url,
    song: SongId,
    download: net::DownloadHandle,
}

impl SongCache {
    /// Songs in the system cache directory, keeping at most about
    /// `capacity` bytes, the default when the settings leave it unset.
    pub fn user(capacity: Option<u64>) -> Result<Self, catalog::Error> {
        Ok(Self::new(SongStore::user(
            capacity.unwrap_or(SONG_CAPACITY),
        )?))
    }

    /// Keeps songs in `store`, starting the thread that writes them.
    pub fn new(store: SongStore) -> Self {
        let (keeps, receiver) = mpsc::unbounded();
        thread::Builder::new()
            .name("hiphile-song-cache".to_owned())
            .spawn({
                let store = store.clone();
                move || run(&store, receiver)
            })
            .expect("failed to spawn the song cache thread");
        Self { store, keeps }
    }

    /// The kept file of `song` on `server`, when it is as long as the
    /// server says the song is. Blocks on the disk.
    pub(crate) fn open(&self, server: &Url, song: &subsonic::Song) -> Option<KeptSong> {
        self.store.open(server, song.id(), song.size())
    }

    /// Removes the kept file of `song` from `client`'s server, such as one
    /// that turned out to be damaged, so the next open downloads it again.
    /// Blocks on the disk.
    pub fn forget(&self, client: &client::Client, song: &SongId) -> Result<(), catalog::Error> {
        self.store.remove(client.server().base_url(), song)
    }

    /// Keeps the body of `download` as `song` on `server` once it has all
    /// arrived.
    pub(crate) fn keep_when_complete(
        &self,
        server: &Url,
        song: &SongId,
        download: net::DownloadHandle,
    ) {
        // Fails only once the thread has stopped, and then nothing is kept.
        self.keeps
            .unbounded_send(Keep {
                server: server.clone(),
                song: song.clone(),
                download,
            })
            .ok();
    }
}

/// Waits for every download handed over at once and writes each that
/// completes, until the last [`SongCache`] is dropped.
fn run(store: &SongStore, mut keeps: mpsc::UnboundedReceiver<Keep>) {
    futures::executor::block_on(async {
        let mut arriving = FuturesUnordered::new();
        loop {
            futures::select! {
                keep = keeps.next() => match keep {
                    Some(Keep { server, song, download }) => arriving.push(async move {
                        let body = download.completed().await;
                        (server, song, body)
                    }),
                    None => return,
                },
                (server, song, body) = arriving.select_next_some() => {
                    // A failure to keep it costs a download later.
                    if let Some(body) = body {
                        store.put(&server, &song, &body).ok();
                    }
                }
            }
        }
    });
}
