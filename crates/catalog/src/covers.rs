use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use subsonic::CoverArtRequest;

use crate::Error;
use crate::blobs::Blobs;
use crate::store::servers_in;

/// How much cover art may stay on disk, across every server, unless the
/// settings say otherwise. When a write goes over, the images used longest ago
/// go until a quarter is free.
pub const COVER_CAPACITY: u64 = 8 << 30;
/// How old a kept image may get before it is fetched again, for servers
/// that change an image without changing its ID.
pub const COVER_REFRESH_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const COVERS: &str = "covers";

/// An image is kept under its cover art ID and size, and is fetched again
/// once it is [`COVER_REFRESH_AFTER`] old: Navidrome gives a changed image
/// a new ID, but other servers may not. Cheap to clone; clones share the
/// record of how much is kept. Every method blocks on the disk: call them
/// where blocking is allowed, such as GPUI's background executor.
#[derive(Clone)]
pub struct CoverStore {
    inner: Arc<Inner>,
}

struct Inner {
    blobs: Blobs,
    refresh_after: Duration,
}

#[derive(Debug)]
pub struct KeptCover {
    bytes: Vec<u8>,
    fresh: bool,
}

impl KeptCover {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn is_fresh(&self) -> bool {
        self.fresh
    }
}

impl CoverStore {
    /// In the app's cache folder `cache`, beside the songs.
    pub fn in_cache(cache: &Path, capacity: u64) -> Self {
        Self::at(servers_in(cache), capacity, COVER_REFRESH_AFTER)
    }

    pub fn at(root: impl AsRef<Path>, capacity: u64, refresh_after: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                blobs: Blobs::new(root.as_ref(), COVERS, capacity),
                refresh_after,
            }),
        }
    }

    /// Keeps at most about `capacity` bytes from the next image on, for every
    /// clone.
    pub fn set_capacity(&self, capacity: u64) {
        self.inner.blobs.set_capacity(capacity);
    }

    pub fn get(&self, request: &CoverArtRequest) -> Option<KeptCover> {
        let path = self.path(request);
        let bytes = fs::read(&path).ok()?;
        // A write cut short, by a power loss for example, can leave an empty
        // file; without it, the image is fetched again.
        if bytes.is_empty() {
            fs::remove_file(&path).ok();
            return None;
        }
        let metadata = fs::metadata(&path).ok();
        // A rename keeps the time the temporary file was created, which is
        // when the image arrived. Without that time, the image stays fresh.
        let age = metadata
            .as_ref()
            .and_then(|metadata| metadata.created().ok())
            .and_then(|created| created.elapsed().ok());
        let fresh = age.is_none_or(|age| age < self.inner.refresh_after);
        if let Some(metadata) = &metadata {
            self.inner.blobs.touch(&path, metadata);
        }
        Some(KeptCover { bytes, fresh })
    }

    pub fn put(&self, request: &CoverArtRequest, bytes: &[u8]) -> Result<(), Error> {
        self.inner.blobs.put(&self.path(request), bytes)
    }

    fn path(&self, request: &CoverArtRequest) -> PathBuf {
        let size = request
            .size()
            .map_or_else(|| "original".to_owned(), |size| size.to_string());
        self.inner
            .blobs
            .path(request.server(), &size, request.id().as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::SystemTime;

    use subsonic::{CoverArtId, Credentials, Endpoint};
    use url::Url;

    use super::*;
    use crate::blobs::TOUCH_AFTER;
    use crate::store::directory_name;

    fn request(server: &str, id: &str, size: Option<u32>) -> CoverArtRequest {
        let base = Url::parse(server).unwrap();
        let url = Endpoint::cover_art(&CoverArtId::new(id), size)
            .url(&base, &Credentials::password("alice", "secret"));
        CoverArtRequest::parse(&url).unwrap()
    }

    fn store(dir: &tempfile::TempDir) -> CoverStore {
        CoverStore::at(dir.path(), COVER_CAPACITY, COVER_REFRESH_AFTER)
    }

    fn kept(store: &CoverStore, request: &CoverArtRequest) -> Option<Vec<u8>> {
        store.get(request).map(KeptCover::into_bytes)
    }

    fn age(store: &CoverStore, request: &CoverArtRequest, by: Duration) {
        File::options()
            .write(true)
            .open(store.path(request))
            .unwrap()
            .set_modified(SystemTime::now() - by)
            .unwrap();
    }

    #[test]
    fn keeps_each_image_by_server_id_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let small = request("https://music.example.com", "al-1", Some(80));
        let large = request("https://music.example.com", "al-1", Some(512));
        let original = request("https://music.example.com", "al-1", None);
        let other = request("https://other.example.com", "al-1", Some(80));
        store.put(&small, b"small").unwrap();
        store.put(&large, b"large").unwrap();
        assert_eq!(kept(&store, &small).as_deref(), Some(&b"small"[..]));
        assert_eq!(kept(&store, &large).as_deref(), Some(&b"large"[..]));
        assert_eq!(kept(&store, &original), None);
        assert_eq!(kept(&store, &other), None);
        assert!(store.get(&small).unwrap().is_fresh());

        // Another URL for the same image, with a fresh salt, finds it
        let again = request("https://music.example.com/", "al-1", Some(80));
        assert_eq!(kept(&store, &again).as_deref(), Some(&b"small"[..]));
    }

    #[test]
    fn an_image_older_than_the_refresh_age_is_not_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let store = CoverStore::at(dir.path(), COVER_CAPACITY, Duration::ZERO);
        let cover = request("https://music.example.com", "al-1", Some(80));
        store.put(&cover, b"image").unwrap();
        let found = store.get(&cover).unwrap();
        assert!(!found.is_fresh());
        assert_eq!(found.bytes(), b"image");
    }

    #[test]
    fn an_empty_file_counts_as_no_image() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let cover = request("https://music.example.com", "al-1", Some(80));
        store.put(&cover, b"image").unwrap();
        fs::write(store.path(&cover), b"").unwrap();
        assert!(store.get(&cover).is_none());
        assert!(!store.path(&cover).exists());
    }

    #[test]
    fn images_live_beside_their_servers_library() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let cover = request("https://example.com/my%20music/", "al-1", None);
        let library = directory_name(&Url::parse("https://example.com/my%20music").unwrap());
        assert!(
            store
                .path(&cover)
                .starts_with(dir.path().join(library).join(COVERS))
        );
    }

    #[test]
    fn going_over_capacity_removes_the_images_used_longest_ago() {
        let dir = tempfile::tempdir().unwrap();
        let store = CoverStore::at(dir.path(), 300, COVER_REFRESH_AFTER);
        let covers: Vec<_> = (0..3)
            .map(|ix| request("https://music.example.com", &format!("al-{ix}"), Some(80)))
            .collect();
        for (ix, cover) in covers.iter().enumerate() {
            store.put(cover, &[1; 100]).unwrap();
            age(&store, cover, Duration::from_secs(1000 - ix as u64 * 100));
        }
        // Reading the oldest makes it the most recently used
        age(&store, &covers[0], TOUCH_AFTER * 2);
        assert!(store.get(&covers[0]).is_some());

        let newest = request("https://music.example.com", "al-3", Some(80));
        store.put(&newest, &[1; 100]).unwrap();
        assert!(store.get(&covers[0]).is_some());
        assert!(store.get(&covers[1]).is_none());
        assert!(store.get(&covers[2]).is_none());
        assert!(store.get(&newest).is_some());
    }
}
