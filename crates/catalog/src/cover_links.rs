use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::Error;
use crate::store::{BUSY_TIMEOUT, is_damaged, remove_database};

const DATABASE: &str = "cover_links.sqlite";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS links (
    -- MD5 of the image, in hex: the same image from any album or server.
    image TEXT PRIMARY KEY,
    url   TEXT NOT NULL,
    -- Milliseconds since the Unix epoch.
    expires INTEGER NOT NULL
) WITHOUT ROWID;
";

/// Where an image can be seen, until when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverLink {
    pub url: String,
    pub expires: SystemTime,
}

impl CoverLink {
    pub fn is_live(&self, now: SystemTime) -> bool {
        self.expires > now
    }
}

/// Where images were published for a limited time, so that an image is not
/// published again while its link still works.
///
/// Kept in the system cache directory: without it, images only go up again.
/// Cheap to clone; clones share one connection.
///
/// Every method blocks on the disk: call them from a thread that may block,
/// never from a UI executor.
#[derive(Clone)]
pub struct CoverLinks {
    connection: Arc<Mutex<Connection>>,
}

impl CoverLinks {
    /// The links in the system cache directory.
    pub fn user() -> Result<Self, Error> {
        let cache = dirs::cache_dir().ok_or(Error::NoCacheDir)?;
        Self::open(cache.join("hiphile"))
    }

    /// The links kept in `dir`. Created if there are none.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
        let path = dir.join(DATABASE);
        let connection = match connect(&path) {
            Ok(connection) => connection,
            Err(error) if is_damaged(&error) => {
                remove_database(&path)?;
                connect(&path)?
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Where `image` can be seen at `now`, if it can.
    pub fn get(&self, image: &[u8], now: SystemTime) -> Result<Option<CoverLink>, Error> {
        let link = self
            .lock()
            .query_row(
                "SELECT url, expires FROM links WHERE image = ?1",
                params![digest(image)],
                |row| {
                    Ok(CoverLink {
                        url: row.get(0)?,
                        expires: from_millis(row.get(1)?),
                    })
                },
            )
            .optional()?;
        Ok(link.filter(|link| link.is_live(now)))
    }

    /// Records where `image` can be seen, and forgets links past their time.
    pub fn put(&self, image: &[u8], link: &CoverLink) -> Result<(), Error> {
        let connection = self.lock();
        connection.execute(
            "DELETE FROM links WHERE expires <= ?1",
            params![to_millis(SystemTime::now())],
        )?;
        connection.execute(
            "INSERT OR REPLACE INTO links (image, url, expires) VALUES (?1, ?2, ?3)",
            params![digest(image), link.url, to_millis(link.expires)],
        )?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn connect(path: &Path) -> Result<Connection, Error> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.execute_batch(SCHEMA)?;
    Ok(connection)
}

fn to_millis(time: SystemTime) -> i64 {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

fn from_millis(millis: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(u64::try_from(millis).unwrap_or(0))
}

fn digest(image: &[u8]) -> String {
    hex::encode(Md5::digest(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(60 * 60);

    fn link(url: &str, expires: SystemTime) -> CoverLink {
        CoverLink {
            url: url.to_owned(),
            expires: from_millis(to_millis(expires)),
        }
    }

    #[test]
    fn a_link_belongs_to_the_image_and_outlives_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let links = CoverLinks::open(dir.path()).unwrap();
        assert_eq!(links.get(b"one", now).unwrap(), None);

        let one = link("https://files.example.com/1.png", now + HOUR);
        links.put(b"one", &one).unwrap();
        drop(links);

        let links = CoverLinks::open(dir.path()).unwrap();
        assert_eq!(links.get(b"one", now).unwrap(), Some(one));
        assert_eq!(links.get(b"two", now).unwrap(), None);
    }

    #[test]
    fn a_link_past_its_time_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let links = CoverLinks::open(dir.path()).unwrap();
        links.put(b"one", &link("https://a", now + HOUR)).unwrap();

        assert_eq!(links.get(b"one", now + HOUR * 2).unwrap(), None);
    }

    #[test]
    fn publishing_forgets_links_past_their_time() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let links = CoverLinks::open(dir.path()).unwrap();
        links.put(b"old", &link("https://a", now - HOUR)).unwrap();
        links.put(b"new", &link("https://b", now + HOUR)).unwrap();

        let rows: i64 = links
            .lock()
            .query_row("SELECT count(*) FROM links", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_damaged_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(DATABASE), b"not a database, not at all").unwrap();

        let links = CoverLinks::open(dir.path()).unwrap();

        assert_eq!(links.get(b"one", SystemTime::now()).unwrap(), None);
    }
}
