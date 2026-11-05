//! The dictionaries that read Japanese, which are too large to ship in the app
//! and so arrive on their own: `script/build-readings` writes a bundle, and the
//! app fetches and installs it. It is a cache that can be fetched again.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use ruzstd::decoding::StreamingDecoder;
use sha2::{Digest as _, Sha256};

/// A new bundle gets a new name, so one never replaces another in place.
pub const BUNDLE: Bundle = Bundle {
    name: "readings-1",
    url: "https://github.com/heisannn/hiphile/releases/download/readings-1/readings-1.tar.zst",
    sha256: "a71c9d7f6a16157c4a3e0d30532768e4b1b619e84a0c684cf79d609c82386eaa",
    download_bytes: 111_974_029,
    disk_bytes: 654_942_208,
};

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Bundle {
    pub name: &'static str,
    pub url: &'static str,
    /// Of the compressed file, in lowercase hex.
    pub sha256: &'static str,
    pub download_bytes: u64,
    pub disk_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the system has no cache directory, so there is nowhere to keep the dictionaries")]
    NoCacheDir,
    #[error("could not use {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the download failed: {0}")]
    Read(#[source] io::Error),
    #[error("the download is damaged: {0}")]
    Damaged(String),
    #[error("the download is not the file expected")]
    WrongFile,
}

pub fn root() -> Result<PathBuf, Error> {
    let cache = dirs::cache_dir().ok_or(Error::NoCacheDir)?;
    Ok(cache.join("hiphile").join("readings"))
}

/// Only a complete bundle ever has this name, as [`install`] moves it there whole.
pub fn installed(root: &Path, bundle: &Bundle) -> Option<PathBuf> {
    let dir = root.join(bundle.name);
    dir.is_dir().then_some(dir)
}

pub fn remove(root: &Path) -> Result<(), Error> {
    match fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: root.to_owned(),
            source,
        }),
    }
}

struct Checked<R> {
    inner: R,
    hasher: Sha256,
    on_progress: Box<dyn FnMut(u64) + Send>,
    read: u64,
    /// What stopped the reading, which whatever reads from here may then report
    /// as something else.
    failure: Option<io::Error>,
}

impl<R: Read> Read for Checked<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let len = self.inner.read(buffer).inspect_err(|error| {
            self.failure = Some(io::Error::new(error.kind(), error.to_string()));
        })?;
        self.hasher.update(&buffer[..len]);
        self.read += len as u64;
        (self.on_progress)(self.read);
        Ok(len)
    }
}

/// Unpacks `body`, a `.tar.zst` as it arrives, and keeps it under `root` as
/// `bundle` once all of it proved to be the file expected. Older bundles go.
/// `on_progress` hears how many compressed bytes have been read.
///
/// Blocks on `body` and the disk. Nothing uses a bundle until it has moved into
/// place, so a failure or a damaged download leaves things as they were.
pub fn install(
    body: impl Read,
    root: &Path,
    bundle: &Bundle,
    on_progress: impl FnMut(u64) + Send + 'static,
) -> Result<PathBuf, Error> {
    let io_error = |path: &Path| {
        let path = path.to_owned();
        move |source| Error::Io { path, source }
    };
    fs::create_dir_all(root).map_err(io_error(root))?;
    let unpacking = root.join(format!(".{}-{}", bundle.name, std::process::id()));
    remove(&unpacking)?;
    fs::create_dir(&unpacking).map_err(io_error(&unpacking))?;

    let unpacked = unpack(body, &unpacking, bundle, Box::new(on_progress));
    if let Err(error) = unpacked {
        remove(&unpacking).ok();
        return Err(error);
    }
    let dir = root.join(bundle.name);
    remove(&dir)?;
    fs::rename(&unpacking, &dir).map_err(io_error(&dir))?;
    remove_others(root, bundle);
    Ok(dir)
}

fn unpack(
    body: impl Read,
    into: &Path,
    bundle: &Bundle,
    on_progress: Box<dyn FnMut(u64) + Send>,
) -> Result<(), Error> {
    let mut checked = Checked {
        inner: body,
        hasher: Sha256::new(),
        on_progress,
        read: 0,
        failure: None,
    };
    let unpacked = match StreamingDecoder::new(&mut checked) {
        Ok(decoder) => tar::Archive::new(decoder)
            .unpack(into)
            .map_err(|error| unpack_failure(error, into)),
        Err(error) => Err(Error::Damaged(error.to_string())),
    };
    if let Some(failure) = checked.failure.take() {
        return Err(Error::Read(failure));
    }
    unpacked?;
    // A decoder may stop at the end of the frame, short of the end of the file
    io::copy(&mut checked, &mut io::sink()).map_err(Error::Read)?;
    if hex::encode(checked.hasher.finalize()) == bundle.sha256 {
        Ok(())
    } else {
        Err(Error::WrongFile)
    }
}

/// What failing to unpack into `into` means, reading having gone well: the disk
/// is to blame for what it refuses, and the download for anything else.
fn unpack_failure(error: io::Error, into: &Path) -> Error {
    match error.kind() {
        io::ErrorKind::StorageFull
        | io::ErrorKind::QuotaExceeded
        | io::ErrorKind::FileTooLarge
        | io::ErrorKind::PermissionDenied
        | io::ErrorKind::ReadOnlyFilesystem => Error::Io {
            path: into.to_owned(),
            source: error,
        },
        _ => Error::Damaged(error.to_string()),
    }
}

/// Removes what an older version or an interrupted install left. One that is in
/// use where the system refuses that goes next time.
fn remove_others(root: &Path, bundle: &Bundle) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name() != bundle.name {
            fs::remove_dir_all(entry.path()).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use ruzstd::encoding::{CompressionLevel, compress_to_vec};

    use super::*;

    fn archive() -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        let contents = b"reading";
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "ipadic/dict.vals", &contents[..])
            .unwrap();
        compress_to_vec(&tar.into_inner().unwrap()[..], CompressionLevel::Fastest)
    }

    fn bundle_of(bytes: &[u8]) -> (Bundle, &'static str) {
        let sha256: &'static str = hex::encode(Sha256::digest(bytes)).leak();
        (
            Bundle {
                name: "readings-test",
                url: "",
                sha256,
                download_bytes: 0,
                disk_bytes: 0,
            },
            sha256,
        )
    }

    struct FailsHalfway<'a> {
        bytes: &'a [u8],
        kind: io::ErrorKind,
    }

    impl Read for FailsHalfway<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.bytes.is_empty() {
                return Err(self.kind.into());
            }
            let len = buffer.len().min(self.bytes.len());
            buffer[..len].copy_from_slice(&self.bytes[..len]);
            self.bytes = &self.bytes[len..];
            Ok(len)
        }
    }

    #[test]
    fn a_connection_that_gives_out_is_not_called_damage() {
        let root = tempfile::tempdir().unwrap();
        let bytes = archive();
        let (bundle, _) = bundle_of(&bytes);
        for kind in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::TimedOut,
            io::ErrorKind::Other,
        ] {
            let body = FailsHalfway {
                bytes: &bytes[..bytes.len() / 2],
                kind,
            };
            let error = install(body, root.path(), &bundle, |_| {}).unwrap_err();
            assert!(matches!(error, Error::Read(_)), "{kind:?}: {error}");
        }
    }

    #[test]
    fn a_disk_that_refuses_is_not_called_damage() {
        for kind in [
            io::ErrorKind::StorageFull,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::ReadOnlyFilesystem,
        ] {
            let error = unpack_failure(kind.into(), Path::new("/where"));
            assert!(matches!(error, Error::Io { .. }), "{kind:?}: {error}");
        }
        let error = unpack_failure(io::ErrorKind::InvalidData.into(), Path::new("/where"));
        assert!(matches!(error, Error::Damaged(_)), "{error}");
    }

    /// As tar reports it, which must not hide what the disk said.
    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_written_to_is_named() {
        use std::os::unix::fs::PermissionsExt as _;

        let into = tempfile::tempdir().unwrap();
        let bytes = archive();
        let (bundle, _) = bundle_of(&bytes);
        fs::set_permissions(into.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let error = unpack(&bytes[..], into.path(), &bundle, Box::new(|_| {})).unwrap_err();
        fs::set_permissions(into.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(error, Error::Io { .. }), "{error}");
    }

    #[test]
    fn a_good_download_lands_whole_and_reports_its_progress() {
        let root = tempfile::tempdir().unwrap();
        let bytes = archive();
        let (bundle, _) = bundle_of(&bytes);
        let progress = Arc::new(AtomicU64::new(0));
        let dir = install(&bytes[..], root.path(), &bundle, {
            let progress = progress.clone();
            move |read| progress.store(read, Ordering::Relaxed)
        })
        .unwrap();
        assert_eq!(fs::read(dir.join("ipadic/dict.vals")).unwrap(), b"reading");
        assert_eq!(installed(root.path(), &bundle), Some(dir));
        assert_eq!(progress.load(Ordering::Relaxed), bytes.len() as u64);
    }

    #[test]
    fn a_download_that_is_not_the_file_expected_leaves_nothing() {
        let root = tempfile::tempdir().unwrap();
        let bytes = archive();
        let (mut bundle, _) = bundle_of(&bytes);
        bundle.sha256 = "not it";
        let result = install(&bytes[..], root.path(), &bundle, |_| {});
        assert!(matches!(result, Err(Error::WrongFile)));
        assert_eq!(installed(root.path(), &bundle), None);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn garbage_is_damage_and_leaves_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (bundle, _) = bundle_of(b"garbage");
        let result = install(&b"garbage"[..], root.path(), &bundle, |_| {});
        assert!(matches!(result, Err(Error::Damaged(_))), "{result:?}");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_download_cut_short_leaves_nothing() {
        let root = tempfile::tempdir().unwrap();
        let bytes = archive();
        let (bundle, _) = bundle_of(&bytes);
        let cut = &bytes[..bytes.len() / 2];
        assert!(install(cut, root.path(), &bundle, |_| {}).is_err());
        assert_eq!(installed(root.path(), &bundle), None);
    }

    #[test]
    fn a_new_bundle_replaces_older_ones_and_leftovers() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("readings-0/ipadic")).unwrap();
        fs::create_dir_all(root.path().join(".readings-test-1")).unwrap();
        let bytes = archive();
        let (bundle, _) = bundle_of(&bytes);
        install(&bytes[..], root.path(), &bundle, |_| {}).unwrap();
        let names: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["readings-test"]);
    }

    /// The file published, which is far too large to keep with the code.
    /// `HIPHILE_READINGS_ARCHIVE=… cargo test -p fuzzy -- --ignored`
    #[test]
    #[ignore = "needs the bundle's file, at HIPHILE_READINGS_ARCHIVE"]
    fn the_published_file_is_the_one_named_and_installs() {
        let archive = std::env::var("HIPHILE_READINGS_ARCHIVE").unwrap();
        let root = tempfile::tempdir().unwrap();
        let file = fs::File::open(archive).unwrap();
        let dir = install(file, root.path(), &BUNDLE, |_| {}).unwrap();
        crate::Reader::open(&dir).unwrap();
        for terms in [
            "NOTICE.txt",
            "LICENSE-Apache-2.0.txt",
            "COPYING-mecab-ipadic-neologd.txt",
            "COPYING-ipadic.txt",
            "NOTICE-ipadic.txt",
            "NOTICE-sudachidict.txt",
        ] {
            assert!(dir.join(terms).is_file(), "{terms}");
        }
    }

    #[test]
    fn removing_what_is_not_there_is_no_error() {
        let root = tempfile::tempdir().unwrap();
        remove(&root.path().join("nothing")).unwrap();
    }
}
