use std::fs::{self, File, Metadata};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use md5::{Digest, Md5};
use url::Url;

use crate::Error;
use crate::store::directory_name;

const TEMPORARY_SUFFIX: &str = ".tmp";
/// How long a file may go without its use being recorded. Recording each
/// use would write to the disk on every read.
pub(crate) const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// A temporary file this old belongs to no write still running.
pub(crate) const ABANDONED_AFTER: Duration = Duration::from_secs(10 * 60);

/// Numbers temporary files, so no two writes in this process share one.
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Files of one kind kept under each server's directory, at
/// `<root>/<server>/<kind>/<variant>/<name>`, within a capacity shared by
/// every server.
///
/// When a write takes the files over the capacity, those used longest ago
/// go until a quarter of it is free. The modification time records the
/// last use. Every method blocks on the disk.
pub(crate) struct Blobs {
    root: PathBuf,
    kind: &'static str,
    capacity: AtomicU64,
    usage: Mutex<Usage>,
}

#[derive(Default)]
struct Usage {
    /// Bytes kept, once counted.
    kept: Option<u64>,
    /// Whether a write is counting the files or removing some. Both touch
    /// every file, so they run without the lock and other writes go on.
    walking: bool,
    added_while_walking: u64,
}

impl Blobs {
    pub(crate) fn new(root: &Path, kind: &'static str, capacity: u64) -> Self {
        Self {
            root: root.to_owned(),
            kind,
            capacity: AtomicU64::new(capacity),
            usage: Mutex::default(),
        }
    }

    /// Keeps at most about `capacity` bytes from the next write on, which is
    /// also when anything over it goes.
    pub(crate) fn set_capacity(&self, capacity: u64) {
        self.capacity.store(capacity, Ordering::Relaxed);
    }

    fn capacity(&self) -> u64 {
        self.capacity.load(Ordering::Relaxed)
    }

    /// IDs are opaque and may hold any character, so they are hashed into names every
    /// file system accepts.
    pub(crate) fn path(&self, server: &Url, variant: &str, id: &str) -> PathBuf {
        let name = hex::encode(Md5::digest(id.as_bytes()));
        self.root
            .join(directory_name(server))
            .join(self.kind)
            .join(variant)
            .join(name)
    }

    /// Records a use of the file at `path`, when the last one recorded is
    /// old enough. Windows changes the modification time only through a
    /// handle opened for writing.
    pub(crate) fn touch(&self, path: &Path, metadata: &Metadata) {
        let unused_for = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok());
        if unused_for.is_none_or(|unused| unused >= TOUCH_AFTER) {
            File::options()
                .write(true)
                .open(path)
                .and_then(|file| file.set_modified(SystemTime::now()))
                .ok();
        }
    }

    /// Writes `bytes` at `path` whole beside it, then moves them over it,
    /// so a reader never sees part of a file.
    ///
    /// Bytes that would not fit in what a removal leaves are not written:
    /// the removal would take them, and every other file, straight away.
    pub(crate) fn put(&self, path: &Path, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() as u64 > self.after_removal() {
            return Ok(());
        }
        let dir = path.parent().unwrap_or(&self.root);
        fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(format!(
            ".{}-{}{TEMPORARY_SUFFIX}",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = PathBuf::from(temporary);
        let written = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, path));
        if let Err(source) = written {
            fs::remove_file(&temporary).ok();
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
        self.record(bytes.len() as u64);
        Ok(())
    }

    fn record(&self, added: u64) {
        {
            let mut usage = self.usage();
            if usage.walking {
                usage.added_while_walking += added;
                return;
            }
            if let Some(kept) = &mut usage.kept {
                // A file written over is counted twice until the next walk
                *kept += added;
                if *kept <= self.capacity() {
                    return;
                }
            }
            // The first count includes what was just written
            usage.walking = true;
        }
        let kept = self.walk();
        let mut usage = self.usage();
        usage.kept = Some(kept + std::mem::take(&mut usage.added_while_walking));
        usage.walking = false;
    }

    fn walk(&self) -> u64 {
        let mut blobs = self.blobs();
        let mut kept: u64 = blobs.iter().map(|blob| blob.len).sum();
        if kept <= self.capacity() {
            return kept;
        }
        let target = self.after_removal();
        blobs.sort_by_key(|blob| blob.used);
        for blob in blobs {
            if kept <= target {
                break;
            }
            // A song that is playing may go too: its open file stays
            // readable until it is closed.
            if fs::remove_file(&blob.path).is_ok() {
                kept -= blob.len;
            }
        }
        kept
    }

    fn after_removal(&self) -> u64 {
        self.capacity() / 4 * 3
    }

    fn usage(&self) -> MutexGuard<'_, Usage> {
        self.usage.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn kept_bytes(&self) -> u64 {
        self.blobs().iter().map(|blob| blob.len).sum()
    }

    /// One that cannot be removed does not stop the rest; the first such failure is
    /// returned.
    pub(crate) fn clear(&self) -> Result<(), Error> {
        let mut cleared = Ok(());
        for blob in self.blobs() {
            match fs::remove_file(&blob.path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    if cleared.is_ok() {
                        cleared = Err(Error::Io {
                            path: blob.path,
                            source,
                        });
                    }
                }
            }
        }
        // The next write counts them again
        self.usage().kept = None;
        cleared
    }

    /// Also removes temporary files that were abandoned, by a crash for example.
    pub(crate) fn blobs(&self) -> Vec<Blob> {
        let mut blobs = Vec::new();
        for server in entries(&self.root) {
            for variant in entries(&server.join(self.kind)) {
                for path in entries(&variant) {
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
                        blobs.push(Blob {
                            path,
                            len: metadata.len(),
                            used,
                        });
                    }
                }
            }
        }
        blobs
    }
}

pub(crate) struct Blob {
    path: PathBuf,
    len: u64,
    used: SystemTime,
}

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
    use super::*;

    fn server() -> Url {
        Url::parse("https://music.example.com").unwrap()
    }

    fn age(path: &Path, by: Duration) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - by)
            .unwrap();
    }

    #[test]
    fn going_over_capacity_removes_the_files_used_longest_ago() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = Blobs::new(dir.path(), "songs", 300);
        let paths: Vec<_> = (0..3)
            .map(|ix| blobs.path(&server(), "original", &format!("so-{ix}")))
            .collect();
        for (ix, path) in paths.iter().enumerate() {
            blobs.put(path, &[1; 100]).unwrap();
            age(path, Duration::from_secs(1000 - ix as u64 * 100));
        }
        // A recorded use makes the oldest the most recently used
        age(&paths[0], TOUCH_AFTER * 2);
        blobs.touch(&paths[0], &fs::metadata(&paths[0]).unwrap());

        blobs
            .put(&blobs.path(&server(), "original", "so-3"), &[1; 100])
            .unwrap();
        assert!(paths[0].exists());
        assert!(!paths[1].exists());
        assert!(!paths[2].exists());
    }

    #[test]
    fn a_file_too_large_to_keep_is_not_written_and_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = Blobs::new(dir.path(), "songs", 400);
        let kept = blobs.path(&server(), "original", "so-1");
        blobs.put(&kept, &[1; 100]).unwrap();
        let large = blobs.path(&server(), "original", "so-2");
        blobs.put(&large, &[1; 301]).unwrap();
        assert!(!large.exists());
        assert!(kept.exists());
        // As large as a removal leaves still fits
        blobs.put(&large, &[1; 300]).unwrap();
        assert!(large.exists());
    }

    #[test]
    fn kinds_have_separate_capacities() {
        let dir = tempfile::tempdir().unwrap();
        let covers = Blobs::new(dir.path(), "covers", 150);
        let songs = Blobs::new(dir.path(), "songs", 150);
        let cover = covers.path(&server(), "80", "al-1");
        covers.put(&cover, &[1; 100]).unwrap();
        songs
            .put(&songs.path(&server(), "original", "so-1"), &[1; 100])
            .unwrap();
        songs
            .put(&songs.path(&server(), "original", "so-2"), &[1; 100])
            .unwrap();
        assert!(cover.exists(), "a song write removed a cover");
        assert_eq!(songs.blobs().len(), 1);
    }

    #[test]
    fn a_lower_capacity_takes_effect_at_the_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = Blobs::new(dir.path(), "test", 1000);
        let path = |id: &str| blobs.path(&server(), "v", id);
        blobs.put(&path("old"), &[0; 400]).unwrap();
        age(&path("old"), Duration::from_secs(60));
        blobs.put(&path("older"), &[0; 400]).unwrap();
        age(&path("older"), Duration::from_secs(120));

        blobs.set_capacity(500);
        assert!(path("old").exists(), "nothing goes until a write");

        blobs.put(&path("new"), &[0; 300]).unwrap();
        assert!(!path("older").exists());
        assert!(path("new").exists());
        assert!(blobs.kept_bytes() <= 500);
    }

    #[test]
    fn no_capacity_keeps_nothing_new_and_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = Blobs::new(dir.path(), "test", 1000);
        let kept = blobs.path(&server(), "v", "kept");
        blobs.put(&kept, &[0; 100]).unwrap();

        blobs.set_capacity(0);
        let new = blobs.path(&server(), "v", "new");
        blobs.put(&new, &[0; 100]).unwrap();

        assert!(kept.exists());
        assert!(!new.exists());
    }

    #[test]
    fn no_limit_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = Blobs::new(dir.path(), "songs", u64::MAX);
        let paths: Vec<_> = (0..3)
            .map(|ix| blobs.path(&server(), "original", &format!("so-{ix}")))
            .collect();
        for path in &paths {
            blobs.put(path, &[1; 100]).unwrap();
        }
        assert!(paths.iter().all(|path| path.exists()));
        assert_eq!(blobs.kept_bytes(), 300);
    }

    #[test]
    fn clearing_removes_only_this_kind_for_every_server() {
        let dir = tempfile::tempdir().unwrap();
        let covers = Blobs::new(dir.path(), "covers", 1000);
        let songs = Blobs::new(dir.path(), "songs", 1000);
        let other = Url::parse("https://other.example.com").unwrap();
        let cover = covers.path(&server(), "80", "al-1");
        covers.put(&cover, &[1; 100]).unwrap();
        songs
            .put(&songs.path(&server(), "original", "so-1"), &[1; 100])
            .unwrap();
        songs
            .put(&songs.path(&other, "original", "so-1"), &[1; 100])
            .unwrap();
        assert_eq!(songs.kept_bytes(), 200);

        songs.clear().unwrap();
        assert_eq!(songs.kept_bytes(), 0);
        assert!(cover.exists(), "clearing songs removed a cover");

        // What is written afterwards is counted from nothing
        let path = songs.path(&server(), "original", "so-2");
        songs.put(&path, &[1; 100]).unwrap();
        assert!(path.exists());
        assert_eq!(songs.kept_bytes(), 100);
    }

    #[test]
    fn abandoned_temporary_files_are_removed_and_not_counted() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = Blobs::new(dir.path(), "songs", 1000);
        let path = blobs.path(&server(), "original", "so-1");
        blobs.put(&path, b"song").unwrap();
        let abandoned = PathBuf::from(format!("{}.1-1{TEMPORARY_SUFFIX}", path.display()));
        fs::write(&abandoned, [0; 50]).unwrap();
        age(&abandoned, ABANDONED_AFTER * 2);
        assert_eq!(blobs.blobs().len(), 1);
        assert!(!abandoned.exists());
    }
}
