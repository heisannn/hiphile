use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use subsonic::SongId;
use url::Url;

use crate::Error;
use crate::blobs::Blobs;
use crate::store::servers_in;

/// How much audio may stay on disk, across every server, unless the
/// settings say otherwise. When a write goes over, the songs played longest
/// ago go until a quarter is free.
pub const SONG_CAPACITY: u64 = 8 << 30;
const SONGS: &str = "songs";
/// Songs are kept as the server's original files; a transcoded format
/// would be kept beside them under its own name.
const ORIGINAL: &str = "original";

/// A song is kept under its ID only once it has arrived whole. Cheap to
/// clone; clones share the record of how much is kept. Every method blocks
/// on the disk: call them where blocking is allowed.
#[derive(Clone)]
pub struct SongStore {
    blobs: Arc<Blobs>,
}

#[derive(Debug)]
pub struct KeptSong {
    file: File,
    byte_len: u64,
}

impl KeptSong {
    /// The file's length, read when it was opened.
    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// It stays readable even if the song is removed from the store meanwhile.
    pub fn into_file(self) -> File {
        self.file
    }
}

impl SongStore {
    /// In the app's cache folder `cache`, beside the covers.
    pub fn in_cache(cache: &Path, capacity: u64) -> Self {
        Self::at(servers_in(cache), capacity)
    }

    pub fn at(root: impl AsRef<Path>, capacity: u64) -> Self {
        Self {
            blobs: Arc::new(Blobs::new(root.as_ref(), SONGS, capacity)),
        }
    }

    /// A file not `expected_len` bytes long is of an older version of the song, and is
    /// removed.
    pub fn open(&self, server: &Url, id: &SongId, expected_len: Option<u64>) -> Option<KeptSong> {
        let path = self.path(server, id);
        let file = File::open(&path).ok()?;
        // The open file's own length, which a rename over it cannot change
        let metadata = file.metadata().ok()?;
        let byte_len = metadata.len();
        if byte_len == 0 || expected_len.is_some_and(|expected| expected != byte_len) {
            drop(file);
            fs::remove_file(&path).ok();
            return None;
        }
        self.blobs.touch(&path, &metadata);
        Some(KeptSong { file, byte_len })
    }

    /// Keeps at most about `capacity` bytes from the next write on, for every clone.
    pub fn set_capacity(&self, capacity: u64) {
        self.blobs.set_capacity(capacity);
    }

    /// Bytes that take most of the capacity on their own are not kept.
    pub fn put(&self, server: &Url, id: &SongId, bytes: &[u8]) -> Result<(), Error> {
        self.blobs.put(&self.path(server, id), bytes).map(|_| ())
    }

    pub fn remove(&self, server: &Url, id: &SongId) -> Result<(), Error> {
        let path = self.path(server, id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io { path, source }),
        }
    }

    pub fn kept_bytes(&self) -> u64 {
        self.blobs.kept_bytes()
    }

    /// One that is being played goes on from its open file.
    pub fn clear(&self) -> Result<(), Error> {
        self.blobs.clear()
    }

    fn path(&self, server: &Url, id: &SongId) -> PathBuf {
        self.blobs.path(server, ORIGINAL, id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;
    use crate::store::directory_name;

    fn server(address: &str) -> Url {
        Url::parse(address).unwrap()
    }

    fn read(song: KeptSong) -> Vec<u8> {
        let mut bytes = Vec::new();
        song.into_file().read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn keeps_each_song_by_server_and_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = SongStore::at(dir.path(), SONG_CAPACITY);
        let music = server("https://music.example.com");
        let other = server("https://other.example.com");
        let id = SongId::new("so-1");
        store.put(&music, &id, b"flac").unwrap();

        let kept = store.open(&music, &id, None).unwrap();
        assert_eq!(kept.byte_len(), 4);
        assert_eq!(read(kept), b"flac");
        assert!(store.open(&music, &SongId::new("so-2"), None).is_none());
        assert!(store.open(&other, &id, None).is_none());
        assert!(
            store
                .path(&music, &id)
                .starts_with(dir.path().join(directory_name(&music)).join(SONGS))
        );
    }

    #[test]
    fn a_file_of_another_length_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let store = SongStore::at(dir.path(), SONG_CAPACITY);
        let music = server("https://music.example.com");
        let id = SongId::new("so-1");
        store.put(&music, &id, b"flac").unwrap();

        assert!(store.open(&music, &id, Some(4)).is_some());
        assert!(store.open(&music, &id, Some(5)).is_none());
        assert!(!store.path(&music, &id).exists());
    }

    #[test]
    fn a_removed_song_is_gone_and_removing_again_is_no_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = SongStore::at(dir.path(), SONG_CAPACITY);
        let music = server("https://music.example.com");
        let id = SongId::new("so-1");
        store.put(&music, &id, b"flac").unwrap();
        store.remove(&music, &id).unwrap();
        assert!(store.open(&music, &id, None).is_none());
        store.remove(&music, &id).unwrap();
    }

    #[test]
    fn an_empty_file_counts_as_no_song() {
        let dir = tempfile::tempdir().unwrap();
        let store = SongStore::at(dir.path(), SONG_CAPACITY);
        let music = server("https://music.example.com");
        let id = SongId::new("so-1");
        store.put(&music, &id, b"").unwrap();
        assert!(store.open(&music, &id, None).is_none());
        assert!(!store.path(&music, &id).exists());
    }
}
