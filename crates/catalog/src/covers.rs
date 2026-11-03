use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use md5::{Digest, Md5};
use subsonic::CoverArtRequest;

use crate::Error;
use crate::store::{directory_name, servers_dir};

/// How much cover art may stay on disk, across every server. When a write
/// goes over, the images used longest ago go until a quarter is free.
pub const COVER_CAPACITY: u64 = 512 << 20;
/// How old a kept image may get before it is fetched again, for servers
/// that change an image without changing its ID.
pub const COVER_REFRESH_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const COVERS: &str = "covers";
const TEMPORARY_SUFFIX: &str = ".tmp";
/// How long an image may go without its use being recorded. Recording each
/// use would write to the disk on every read.
const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// A temporary file this old belongs to no write still running.
const ABANDONED_AFTER: Duration = Duration::from_secs(10 * 60);

/// Numbers temporary files, so no two writes in this process share one.
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Cover art images kept on disk, beside each server's library, so an
/// image is downloaded once rather than once per session.
///
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
    root: PathBuf,
    capacity: u64,
    refresh_after: Duration,
    usage: Mutex<Usage>,
}

#[derive(Default)]
struct Usage {
    /// Bytes kept, once counted.
    kept: Option<u64>,
    /// Whether a write is counting the images or removing some. Both touch
    /// every image, so they run without the lock and other writes go on.
    walking: bool,
    /// Bytes written while the images were being walked.
    added_while_walking: u64,
}

/// An image found on disk.
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

    /// Whether it is young enough to show without asking the server.
    pub fn is_fresh(&self) -> bool {
        self.fresh
    }
}

impl CoverStore {
    /// Images in the system cache directory, beside the libraries.
    pub fn user() -> Result<Self, Error> {
        Ok(Self::at(
            servers_dir()?,
            COVER_CAPACITY,
            COVER_REFRESH_AFTER,
        ))
    }

    /// Images under `root`, one directory per server, keeping at most about
    /// `capacity` bytes, each fresh for `refresh_after`.
    pub fn at(root: impl AsRef<Path>, capacity: u64, refresh_after: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                root: root.as_ref().to_owned(),
                capacity,
                refresh_after,
                usage: Mutex::default(),
            }),
        }
    }

    /// The kept image `request` asks for, if there is one.
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
        // The modification time records the last use, which decides what
        // goes first when the images take too much space. Windows changes it
        // only through a handle opened for writing.
        let unused_for = metadata
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.elapsed().ok());
        if unused_for.is_none_or(|unused| unused >= TOUCH_AFTER) {
            File::options()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(SystemTime::now()))
                .ok();
        }
        Some(KeptCover { bytes, fresh })
    }

    /// Keeps `bytes` as the image `request` asks for.
    pub fn put(&self, request: &CoverArtRequest, bytes: &[u8]) -> Result<(), Error> {
        let path = self.path(request);
        let dir = path.parent().unwrap_or(&self.inner.root);
        fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
        // Written whole beside the image, then moved over it, so a reader
        // never sees part of one.
        let mut temporary = path.clone().into_os_string();
        temporary.push(format!(
            ".{}-{}{TEMPORARY_SUFFIX}",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = PathBuf::from(temporary);
        let written = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, &path));
        if let Err(source) = written {
            fs::remove_file(&temporary).ok();
            return Err(Error::Io { path, source });
        }
        self.record(bytes.len() as u64);
        Ok(())
    }

    /// Counts `added` bytes, and counts every image or removes some when
    /// that is due.
    fn record(&self, added: u64) {
        {
            let mut usage = self.usage();
            if usage.walking {
                usage.added_while_walking += added;
                return;
            }
            if let Some(kept) = &mut usage.kept {
                // An image written over is counted twice until the next walk.
                *kept += added;
                if *kept <= self.inner.capacity {
                    return;
                }
            }
            // The first count includes what was just written.
            usage.walking = true;
        }
        let kept = self.walk();
        let mut usage = self.usage();
        usage.kept = Some(kept + std::mem::take(&mut usage.added_while_walking));
        usage.walking = false;
    }

    /// Counts the images and, when they take more than the capacity, removes
    /// those used longest ago until a quarter of it is free. Returns how
    /// many bytes are kept.
    fn walk(&self) -> u64 {
        let mut images = self.images();
        let mut kept: u64 = images.iter().map(|image| image.len).sum();
        if kept <= self.inner.capacity {
            return kept;
        }
        let target = self.inner.capacity / 4 * 3;
        images.sort_by_key(|image| image.used);
        for image in images {
            if kept <= target {
                break;
            }
            if fs::remove_file(&image.path).is_ok() {
                kept -= image.len;
            }
        }
        kept
    }

    fn usage(&self) -> MutexGuard<'_, Usage> {
        self.inner
            .usage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn path(&self, request: &CoverArtRequest) -> PathBuf {
        let size = request
            .size()
            .map_or_else(|| "original".to_owned(), |size| size.to_string());
        // IDs are opaque and may hold any character, so they are hashed
        // into names every file system accepts.
        let name = hex::encode(Md5::digest(request.id().as_str().as_bytes()));
        self.inner
            .root
            .join(directory_name(request.server()))
            .join(COVERS)
            .join(size)
            .join(name)
    }

    /// Every image kept for every server. Removes temporary files that
    /// were abandoned, by a crash for example.
    fn images(&self) -> Vec<Image> {
        let mut images = Vec::new();
        for server in entries(&self.inner.root) {
            for size in entries(&server.join(COVERS)) {
                for path in entries(&size) {
                    let Ok(metadata) = fs::metadata(&path) else {
                        continue;
                    };
                    let used = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    if path.to_string_lossy().ends_with(TEMPORARY_SUFFIX) {
                        if used.elapsed().is_ok_and(|age| age >= ABANDONED_AFTER) {
                            fs::remove_file(&path).ok();
                        }
                        continue;
                    }
                    if metadata.is_file() {
                        images.push(Image {
                            path,
                            len: metadata.len(),
                            used,
                        });
                    }
                }
            }
        }
        images
    }
}

struct Image {
    path: PathBuf,
    len: u64,
    used: SystemTime,
}

/// The entries of `dir`, or none when it cannot be read.
fn entries(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use subsonic::{CoverArtId, Credentials, Endpoint};
    use url::Url;

    use super::*;

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

        // Another URL for the same image, with a fresh salt, finds it.
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
        // Reading the oldest makes it the most recently used.
        age(&store, &covers[0], TOUCH_AFTER * 2);
        assert!(store.get(&covers[0]).is_some());

        let newest = request("https://music.example.com", "al-3", Some(80));
        store.put(&newest, &[1; 100]).unwrap();
        assert!(store.get(&covers[0]).is_some());
        assert!(store.get(&covers[1]).is_none());
        assert!(store.get(&covers[2]).is_none());
        assert!(store.get(&newest).is_some());
    }

    #[test]
    fn abandoned_temporary_files_are_removed_and_not_counted() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let cover = request("https://music.example.com", "al-1", Some(80));
        store.put(&cover, b"image").unwrap();
        let abandoned = PathBuf::from(format!(
            "{}.1-1{TEMPORARY_SUFFIX}",
            store.path(&cover).display()
        ));
        fs::write(&abandoned, [0; 50]).unwrap();
        File::options()
            .write(true)
            .open(&abandoned)
            .unwrap()
            .set_modified(SystemTime::now() - ABANDONED_AFTER * 2)
            .unwrap();
        let images = store.images();
        assert_eq!(images.len(), 1);
        assert!(!abandoned.exists());
    }
}
