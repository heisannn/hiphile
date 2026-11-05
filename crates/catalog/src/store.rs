use std::collections::HashMap;
use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use rusqlite::{
    Connection, ErrorCode, OptionalExtension as _, Transaction, TransactionBehavior, params,
};
use subsonic::{Album, AlbumId, AlbumWithSongs, Artist, ArtistId, Song};
use url::Url;

use crate::Error;

/// Bump when the tables or the stored JSON change meaning. A database with
/// another version is emptied, and the next sync fills it again.
///
/// 2: songs keep their file size.
/// 3: how texts read is kept.
/// 4: artists are kept, and readings keep their romaji.
const SCHEMA_VERSION: i32 = 4;
const DATABASE: &str = "catalog.sqlite";
const SYNC_LOCK: &str = "sync.lock";
/// The meta key for what the `readings` table was worked out with.
const READINGS_SOURCE: &str = "readings_source";
/// How long a write by another connection, such as the CLI's, may hold up
/// this one.
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// `json` holds the item as the server sent it; the other columns repeat
/// what is sorted or looked up by. `generation` is the sync that last saw
/// the row: a sync that completes removes the rows it did not see.
const SCHEMA: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE albums (
    id           TEXT PRIMARY KEY,
    -- Position in the server's name order, which ignores articles and
    -- follows sort names, so it cannot be redone locally.
    ord          INTEGER NOT NULL,
    name         TEXT NOT NULL,
    artist       TEXT,
    json         TEXT NOT NULL,
    generation   INTEGER NOT NULL,
    -- Whether the songs table holds this album's songs.
    songs_cached INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX albums_by_ord ON albums (ord);

CREATE TABLE songs (
    id         TEXT PRIMARY KEY,
    album_id   TEXT,
    disc       INTEGER,
    track      INTEGER,
    title      TEXT NOT NULL,
    artist     TEXT,
    json       TEXT NOT NULL,
    generation INTEGER NOT NULL
);
CREATE INDEX songs_by_album ON songs (album_id, disc, track);

CREATE TABLE artists (
    id         TEXT PRIMARY KEY,
    -- Position in the list the server sent.
    ord        INTEGER NOT NULL,
    name       TEXT NOT NULL,
    json       TEXT NOT NULL,
    generation INTEGER NOT NULL
);
CREATE INDEX artists_by_ord ON artists (ord);

-- How texts read, by the text and not by what bears it, which any list may share.
-- `readings` is a JSON array of kana; an empty one means it was tried and came to
-- nothing. `romaji` is how those are spelled for matching, in the matcher's form.
CREATE TABLE readings (
    text     TEXT PRIMARY KEY,
    readings TEXT NOT NULL,
    romaji   TEXT NOT NULL
) WITHOUT ROWID;
";

/// How one text reads, to keep: see [`Catalog::put_readings`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SavedReading {
    pub text: String,
    /// Each way it may read, in kana.
    pub readings: Vec<String>,
    /// How those are spelled for matching, in a form of the matcher's own.
    pub romaji: String,
}

impl SavedReading {
    pub fn new(text: String, readings: Vec<String>, romaji: String) -> Self {
        Self {
            text,
            readings,
            romaji,
        }
    }
}

/// The albums recommended last and the day they were picked for.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Recommended {
    pub day: String,
    pub albums: Vec<Album>,
}

/// A collection that is synced as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Albums,
    Artists,
    Songs,
}

impl Kind {
    fn table(self) -> &'static str {
        match self {
            Self::Albums => "albums",
            Self::Artists => "artists",
            Self::Songs => "songs",
        }
    }

    fn meta_key(self, name: &str) -> String {
        format!("{}.{name}", self.table())
    }
}

/// One server's library on disk. Cheap to clone; clones share one
/// connection.
///
/// Every method blocks on the disk: call them from a thread that may block,
/// never from a UI executor.
#[derive(Clone)]
pub struct Catalog {
    inner: Arc<Inner>,
}

struct Inner {
    dir: PathBuf,
    connection: Mutex<Connection>,
}

/// Held while a sync runs, by whichever process runs it.
pub(crate) struct SyncLock {
    _file: File,
}

impl Catalog {
    /// The library of the server at `server`, in the system cache directory
    /// (`~/Library/Caches/hiphile` on macOS). Created if there is none.
    pub fn open_server(server: &Url) -> Result<Self, Error> {
        Self::open(servers_dir()?.join(directory_name(server)))
    }

    /// The library kept in `dir`. Created if there is none.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        Self::open_with(dir.as_ref(), BUSY_TIMEOUT)
    }

    fn open_with(dir: &Path, busy_timeout: Duration) -> Result<Self, Error> {
        let dir = dir.to_owned();
        fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        let path = dir.join(DATABASE);
        let connection = match connect(&path, busy_timeout) {
            Ok(connection) => connection,
            // A file SQLite cannot read holds nothing worth keeping. Any
            // other failure, such as another process holding a write, leaves
            // the files alone: removing a database in use corrupts it.
            Err(error) if is_damaged(&error) => {
                remove_database(&path)?;
                connect(&path, busy_timeout)?
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                dir,
                connection: Mutex::new(connection),
            }),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// Every artist, in the server's name order.
    pub fn artists(&self) -> Result<Vec<Artist>, Error> {
        let connection = self.connection();
        let mut statement =
            connection.prepare_cached("SELECT json FROM artists ORDER BY ord, id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|json| Ok(serde_json::from_str(&json?)?)).collect()
    }

    /// Every album, in the server's name order.
    pub fn albums(&self) -> Result<Vec<Album>, Error> {
        let connection = self.connection();
        let mut statement =
            connection.prepare_cached("SELECT json FROM albums ORDER BY ord, id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|json| Ok(serde_json::from_str(&json?)?)).collect()
    }

    /// The albums of `artist`, in the server's name order. Reads no others, for
    /// a screen that asks each time its selection moves.
    pub fn albums_by_artist(&self, artist: &ArtistId) -> Result<Vec<Album>, Error> {
        let connection = self.connection();
        let mut statement = connection.prepare_cached(
            "SELECT json FROM albums WHERE json_extract(json, '$.artistId') = ?1
             ORDER BY ord, id",
        )?;
        let rows = statement.query_map([artist.as_str()], |row| row.get::<_, String>(0))?;
        rows.map(|json| Ok(serde_json::from_str(&json?)?)).collect()
    }

    /// The songs that name `artist`, in no particular order. Reads no others.
    pub fn songs_by_artist(&self, artist: &ArtistId) -> Result<Vec<Song>, Error> {
        let connection = self.connection();
        let mut statement = connection
            .prepare_cached("SELECT json FROM songs WHERE json_extract(json, '$.artistId') = ?1")?;
        let rows = statement.query_map([artist.as_str()], |row| row.get::<_, String>(0))?;
        rows.map(|json| Ok(serde_json::from_str(&json?)?)).collect()
    }

    /// The album with its songs, or `None` when its songs have not been
    /// fetched yet.
    pub fn album(&self, id: &AlbumId) -> Result<Option<AlbumWithSongs>, Error> {
        let connection = self.connection();
        let album = connection
            .prepare_cached("SELECT json FROM albums WHERE id = ?1 AND songs_cached")?
            .query_row([id.as_str()], |row| row.get::<_, String>(0))
            .optional()?;
        let Some(album) = album else {
            return Ok(None);
        };
        let album: Album = serde_json::from_str(&album)?;
        let mut statement = connection.prepare_cached(
            "SELECT json FROM songs WHERE album_id = ?1
             ORDER BY disc IS NULL, disc, track IS NULL, track, title, id",
        )?;
        let songs = statement
            .query_map([id.as_str()], |row| row.get::<_, String>(0))?
            .map(|json| Ok(serde_json::from_str(&json?)?))
            .collect::<Result<Vec<Song>, Error>>()?;
        Ok(Some(AlbumWithSongs::new(album, songs)))
    }

    /// Every song, in no particular order.
    pub fn songs(&self) -> Result<Vec<Song>, Error> {
        let connection = self.connection();
        let mut statement = connection.prepare_cached("SELECT json FROM songs")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|json| Ok(serde_json::from_str(&json?)?)).collect()
    }

    pub fn album_count(&self) -> Result<usize, Error> {
        self.count(Kind::Albums)
    }

    pub fn song_count(&self) -> Result<usize, Error> {
        self.count(Kind::Songs)
    }

    /// When the collection was last synced completely.
    pub fn albums_synced_at(&self) -> Result<Option<SystemTime>, Error> {
        self.synced_at(Kind::Albums)
    }

    pub fn songs_synced_at(&self) -> Result<Option<SystemTime>, Error> {
        self.synced_at(Kind::Songs)
    }

    /// Keeps an album fetched on its own, replacing the songs kept for it.
    /// An album the catalog does not list is ignored: the next album sync
    /// decides whether it exists.
    pub fn put_album(&self, album: &AlbumWithSongs) -> Result<(), Error> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let id = album.album().id();
        let updated = transaction.execute(
            "UPDATE albums SET name = ?2, artist = ?3, json = ?4, songs_cached = 1 WHERE id = ?1",
            params![
                id.as_str(),
                album.album().name(),
                album.album().artist(),
                serde_json::to_string(album.album())?,
            ],
        )?;
        if updated == 0 {
            return Ok(());
        }
        // The generation a running song sync writes, so it keeps these.
        let generation = generation(&transaction, Kind::Songs)?;
        transaction.execute("DELETE FROM songs WHERE album_id = ?1", [id.as_str()])?;
        insert_songs(&transaction, generation, album.songs(), Some(id))?;
        transaction.commit()?;
        Ok(())
    }

    /// The albums recommended to `user` last, whichever day that was. They
    /// follow from what `user` listens to, which others on the server should
    /// not see.
    pub fn recommended(&self, user: &str) -> Result<Option<Recommended>, Error> {
        let connection = self.connection();
        let Some(day) = get_meta(&connection, &recommended_key(user, "day"))? else {
            return Ok(None);
        };
        let albums = get_meta(&connection, &recommended_key(user, "albums"))?.unwrap_or_default();
        Ok(Some(Recommended {
            day,
            albums: serde_json::from_str(&albums)?,
        }))
    }

    /// Keeps the albums picked for `user` on `day`, in place of those kept
    /// before.
    pub fn put_recommended(&self, user: &str, day: &str, albums: &[Album]) -> Result<(), Error> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        set_meta(&transaction, &recommended_key(user, "day"), day)?;
        set_meta(
            &transaction,
            &recommended_key(user, "albums"),
            &serde_json::to_string(albums)?,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Each distinct text that finds an album or a song: names, titles and
    /// artists, without reading the items whole.
    pub fn texts(&self) -> Result<Vec<String>, Error> {
        let connection = self.connection();
        let mut statement = connection.prepare_cached(
            "SELECT name FROM albums
             UNION SELECT artist FROM albums WHERE artist IS NOT NULL
             UNION SELECT name FROM artists
             UNION SELECT title FROM songs
             UNION SELECT artist FROM songs WHERE artist IS NOT NULL
             UNION SELECT json_extract(json, '$.album') FROM songs
                 WHERE json_extract(json, '$.album') IS NOT NULL",
        )?;
        let texts = statement.query_map([], |row| row.get(0))?;
        Ok(texts.collect::<Result<_, _>>()?)
    }

    /// The romaji of each text kept so far, as worked out with `source`. Nothing
    /// worked out with anything else counts.
    pub fn romaji(&self, source: &str) -> Result<HashMap<String, String>, Error> {
        let connection = self.connection();
        if get_meta(&connection, READINGS_SOURCE)?.as_deref() != Some(source) {
            return Ok(HashMap::new());
        }
        let mut statement = connection.prepare_cached("SELECT text, romaji FROM readings")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Keeps how these texts read as worked out with `source`, in place of
    /// anything worked out with something else.
    pub fn put_readings(&self, source: &str, readings: &[SavedReading]) -> Result<(), Error> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        if get_meta(&transaction, READINGS_SOURCE)?.as_deref() != Some(source) {
            transaction.execute("DELETE FROM readings", [])?;
            set_meta(&transaction, READINGS_SOURCE, source)?;
        }
        {
            let mut insert = transaction.prepare_cached(
                "INSERT INTO readings (text, readings, romaji) VALUES (?1, ?2, ?3)
                 ON CONFLICT (text) DO UPDATE SET
                     readings = excluded.readings, romaji = excluded.romaji",
            )?;
            for reading in readings {
                insert.execute(params![
                    reading.text,
                    serde_json::to_string(&reading.readings)?,
                    reading.romaji,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// `None` while another sync of this library holds the lock.
    pub(crate) fn try_lock_sync(&self) -> Result<Option<SyncLock>, Error> {
        let path = self.inner.dir.join(SYNC_LOCK);
        let io_error = |source| Error::Io {
            path: path.clone(),
            source,
        };
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io_error)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(SyncLock { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(source)) => Err(io_error(source)),
        }
    }

    /// Whether `kind` needs a sync: it never completed one, one started
    /// since did not complete, the server's library changed since
    /// (`modified`, when the server says), or the last one is older than
    /// `max_age`.
    pub(crate) fn is_stale(
        &self,
        kind: Kind,
        modified: Option<u64>,
        now: SystemTime,
        max_age: Duration,
    ) -> Result<bool, Error> {
        let Some(synced_at) = self.synced_at(kind)? else {
            return Ok(true);
        };
        if get_meta(&self.connection(), &kind.meta_key("unfinished"))?.is_some() {
            return Ok(true);
        }
        if let Some(modified) = modified {
            let seen = get_meta(&self.connection(), &kind.meta_key("modified"))?;
            if seen != Some(modified.to_string()) {
                return Ok(true);
            }
        }
        // A clock set back since counts as stale.
        Ok(match now.duration_since(synced_at) {
            Ok(age) => age >= max_age,
            Err(_) => true,
        })
    }

    /// Starts a sync of `kind` and returns the generation it writes. The
    /// copy counts as stale until [`Self::finish`]: a sync that fails or
    /// does not settle may have missed items.
    pub(crate) fn begin(&self, kind: Kind) -> Result<i64, Error> {
        let mut connection = self.connection();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let next = generation(&transaction, kind)? + 1;
        set_meta(
            &transaction,
            &kind.meta_key("generation"),
            &next.to_string(),
        )?;
        set_meta(&transaction, &kind.meta_key("unfinished"), "1")?;
        transaction.commit()?;
        Ok(next)
    }

    /// Keeps every artist in the server's name order.
    pub(crate) fn store_artists(&self, generation: i64, artists: &[Artist]) -> Result<(), Error> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO artists (id, ord, name, json, generation)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET
                     ord = excluded.ord, name = excluded.name, json = excluded.json,
                     generation = excluded.generation",
            )?;
            for (artist, ord) in artists.iter().zip(0_i64..) {
                statement.execute(params![
                    artist.id().as_str(),
                    ord,
                    artist.name(),
                    serde_json::to_string(artist)?,
                    generation,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn store_albums(
        &self,
        generation: i64,
        offset: u32,
        albums: &[Album],
    ) -> Result<(), Error> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO albums (id, ord, name, artist, json, generation)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET
                     ord = excluded.ord, name = excluded.name, artist = excluded.artist,
                     json = excluded.json, generation = excluded.generation",
            )?;
            for (album, ord) in albums.iter().zip(i64::from(offset)..) {
                statement.execute(params![
                    album.id().as_str(),
                    ord,
                    album.name(),
                    album.artist(),
                    serde_json::to_string(album)?,
                    generation,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn store_songs(&self, generation: i64, songs: &[Song]) -> Result<(), Error> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        insert_songs(&transaction, generation, songs, None)?;
        transaction.commit()?;
        Ok(())
    }

    /// Completes a sync of `kind`: removes what it did not see and records
    /// when it ran.
    pub(crate) fn finish(
        &self,
        kind: Kind,
        generation: i64,
        modified: Option<u64>,
        started: SystemTime,
    ) -> Result<(), Error> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute(
            &format!("DELETE FROM {} WHERE generation < ?1", kind.table()),
            [generation],
        )?;
        transaction.execute(
            "DELETE FROM meta WHERE key = ?1",
            [kind.meta_key("unfinished")],
        )?;
        if kind == Kind::Songs {
            transaction.execute(
                "UPDATE albums SET songs_cached =
                     EXISTS (SELECT 1 FROM songs WHERE songs.album_id = albums.id)",
                [],
            )?;
        }
        let seconds = started
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        set_meta(
            &transaction,
            &kind.meta_key("synced_at"),
            &seconds.to_string(),
        )?;
        match modified {
            Some(modified) => set_meta(
                &transaction,
                &kind.meta_key("modified"),
                &modified.to_string(),
            )?,
            None => {
                transaction.execute(
                    "DELETE FROM meta WHERE key = ?1",
                    [kind.meta_key("modified")],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn count(&self, kind: Kind) -> Result<usize, Error> {
        let count: i64 = self.connection().query_row(
            &format!("SELECT count(*) FROM {}", kind.table()),
            [],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or_default())
    }

    fn synced_at(&self, kind: Kind) -> Result<Option<SystemTime>, Error> {
        let value = get_meta(&self.connection(), &kind.meta_key("synced_at"))?;
        Ok(value
            .and_then(|value| value.parse().ok())
            .map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds)))
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.inner
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn execute(&self, sql: &str) {
        self.connection().execute_batch(sql).unwrap();
    }
}

/// Where each server's cached files live, one directory per server.
pub(crate) fn servers_dir() -> Result<PathBuf, Error> {
    let cache = dirs::cache_dir().ok_or(Error::NoCacheDir)?;
    Ok(cache.join("hiphile").join("servers"))
}

/// `host-hash`: readable, and different for servers on one host under
/// different paths or ports.
pub(crate) fn directory_name(server: &Url) -> String {
    let address = server.as_str().trim_end_matches('/');
    let digest = hex::encode(Md5::digest(address.as_bytes()));
    let host: String = server
        .host_str()
        .unwrap_or("server")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{host}-{}", &digest[..12])
}

/// Whether `error` says the file is not a database SQLite can read.
pub(crate) fn is_damaged(error: &Error) -> bool {
    matches!(
        error,
        Error::Database(rusqlite::Error::SqliteFailure(failure, _))
            if matches!(failure.code, ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
    )
}

/// Whether `error` is another connection holding the database too long.
pub(crate) fn is_busy(error: &Error) -> bool {
    matches!(
        error,
        Error::Database(rusqlite::Error::SqliteFailure(failure, _))
            if matches!(failure.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

fn connect(path: &Path, busy_timeout: Duration) -> Result<Connection, Error> {
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(busy_timeout)?;
    // Readers never wait for a sync's writes, and the GUI and the CLI can
    // share the file.
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
    if schema_version(&connection)? != SCHEMA_VERSION {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Another process may have just done this.
        if schema_version(&transaction)? != SCHEMA_VERSION {
            let tables = transaction
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for table in tables {
                transaction.execute_batch(&format!("DROP TABLE \"{table}\""))?;
            }
            transaction.execute_batch(SCHEMA)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        transaction.commit()?;
    }
    Ok(connection)
}

fn schema_version(connection: &Connection) -> Result<i32, Error> {
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

pub(crate) fn remove_database(path: &Path) -> Result<(), Error> {
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        let file = PathBuf::from(file);
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::Io { path: file, source }),
        }
    }
    Ok(())
}

fn generation(connection: &Connection, kind: Kind) -> Result<i64, Error> {
    Ok(get_meta(connection, &kind.meta_key("generation"))?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0))
}

fn recommended_key(user: &str, name: &str) -> String {
    format!("recommended.{user}.{name}")
}

fn get_meta(connection: &Connection, key: &str) -> Result<Option<String>, Error> {
    Ok(connection
        .prepare_cached("SELECT value FROM meta WHERE key = ?1")?
        .query_row([key], |row| row.get(0))
        .optional()?)
}

fn set_meta(connection: &Connection, key: &str, value: &str) -> Result<(), Error> {
    connection
        .prepare_cached(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )?
        .execute([key, value])?;
    Ok(())
}

/// Keeps `songs`; `album`, when given, is the album they were fetched as.
fn insert_songs(
    transaction: &Transaction<'_>,
    generation: i64,
    songs: &[Song],
    album: Option<&AlbumId>,
) -> Result<(), Error> {
    let mut statement = transaction.prepare_cached(
        "INSERT INTO songs (id, album_id, disc, track, title, artist, json, generation)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (id) DO UPDATE SET
             album_id = excluded.album_id, disc = excluded.disc, track = excluded.track,
             title = excluded.title, artist = excluded.artist, json = excluded.json,
             generation = excluded.generation",
    )?;
    for song in songs {
        statement.execute(params![
            song.id().as_str(),
            album.or(song.album_id()).map(AlbumId::as_str),
            song.disc_number(),
            song.track(),
            song.title(),
            song.artist(),
            serde_json::to_string(song)?,
            generation,
        ])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_names_tell_servers_apart_and_ignore_a_trailing_slash() {
        let name = |address| directory_name(&Url::parse(address).unwrap());
        assert_eq!(
            name("https://music.example.com"),
            name("https://music.example.com/")
        );
        assert_ne!(
            name("https://example.com/navidrome"),
            name("https://example.com/other")
        );
        assert_ne!(
            name("https://music.example.com"),
            name("https://music.example.com:4533")
        );
        assert!(name("https://music.example.com").starts_with("music.example.com-"));
        assert!(name("http://[::1]:4533").starts_with("___1_-"));
    }

    #[test]
    fn asking_by_artist_reads_only_what_names_them() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        let album = |id: &str, artist: &str| -> Album {
            let album = serde_json::json!({ "id": id, "name": id, "artistId": artist });
            serde_json::from_value(album).unwrap()
        };
        let song = |id: &str, artist: &str| -> Song {
            serde_json::from_value(serde_json::json!({
                "id": id, "title": id, "artistId": artist,
            }))
            .unwrap()
        };
        let generation = catalog.begin(Kind::Albums).unwrap();
        let albums = [album("a2", "m"), album("b", "x"), album("a1", "m")];
        catalog.store_albums(generation, 0, &albums).unwrap();
        let generation = catalog.begin(Kind::Songs).unwrap();
        let songs = [song("s1", "m"), song("s2", "x"), song("s3", "m")];
        catalog.store_songs(generation, &songs).unwrap();

        let miles = ArtistId::new("m");
        let found = catalog.albums_by_artist(&miles).unwrap();
        let ids: Vec<_> = found.iter().map(|album| album.id().as_str()).collect();
        assert_eq!(ids, ["a2", "a1"]);

        let found = catalog.songs_by_artist(&miles).unwrap();
        let mut ids: Vec<_> = found.iter().map(|song| song.id().as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["s1", "s3"]);

        assert!(
            catalog
                .albums_by_artist(&ArtistId::new("nobody"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn another_schema_version_is_emptied() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        catalog.execute(
            "INSERT INTO albums (id, ord, name, json, generation)
             VALUES ('a1', 0, 'A', '{\"id\":\"a1\",\"name\":\"A\"}', 1);
             PRAGMA user_version = 99;",
        );
        drop(catalog);
        let catalog = Catalog::open(dir.path()).unwrap();
        assert_eq!(catalog.album_count().unwrap(), 0);
    }

    #[test]
    fn an_unreadable_database_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(DATABASE),
            b"not a database, just some bytes",
        )
        .unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        assert_eq!(catalog.album_count().unwrap(), 0);
    }

    #[test]
    fn a_database_another_process_holds_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        // Another version holds a write, so opening must wait to reset it.
        catalog.execute(
            "INSERT INTO albums (id, ord, name, json, generation)
             VALUES ('a1', 0, 'A', '{\"id\":\"a1\",\"name\":\"A\"}', 1);
             PRAGMA user_version = 99;
             BEGIN IMMEDIATE;",
        );
        let error = Catalog::open_with(dir.path(), Duration::from_millis(50))
            .err()
            .unwrap();
        assert!(is_busy(&error), "{error}");
        catalog.execute("COMMIT;");
        assert_eq!(catalog.album_count().unwrap(), 1);
    }

    #[test]
    fn the_albums_recommended_last_are_kept_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        assert!(catalog.recommended("alice").unwrap().is_none());

        let album = |id: &str| -> Album {
            serde_json::from_value(serde_json::json!({ "id": id, "name": id })).unwrap()
        };
        catalog
            .put_recommended("alice", "2026-11-03", &[album("old")])
            .unwrap();
        catalog
            .put_recommended("alice", "2026-11-04", &[album("b"), album("a")])
            .unwrap();

        let kept = Catalog::open(dir.path())
            .unwrap()
            .recommended("alice")
            .unwrap()
            .unwrap();
        assert_eq!(kept.day, "2026-11-04");
        let ids: Vec<_> = kept
            .albums
            .iter()
            .map(|album| album.id().as_str())
            .collect();
        assert_eq!(ids, ["b", "a"]);
    }

    #[test]
    fn the_albums_recommended_to_one_user_stay_theirs() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        catalog.put_recommended("alice", "2026-11-04", &[]).unwrap();
        assert!(catalog.recommended("bob").unwrap().is_none());
    }

    #[test]
    fn texts_name_every_field_a_list_finds_by_once() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        let albums: Vec<Album> = serde_json::from_value(serde_json::json!([
            { "id": "a", "name": "教育", "artist": "東京事変" },
        ]))
        .unwrap();
        let songs: Vec<Song> = serde_json::from_value(serde_json::json!([
            { "id": "s", "title": "群青日和", "artist": "東京事変", "album": "教育" },
            { "id": "t", "title": "遭難", "album": "別の盤" },
        ]))
        .unwrap();
        let generation = catalog.begin(Kind::Albums).unwrap();
        catalog.store_albums(generation, 0, &albums).unwrap();
        let generation = catalog.begin(Kind::Songs).unwrap();
        catalog.store_songs(generation, &songs).unwrap();
        let artists: Vec<Artist> = serde_json::from_value(serde_json::json!([
            { "id": "t", "name": "東京事変" },
            { "id": "s", "name": "椎名林檎" },
        ]))
        .unwrap();
        let generation = catalog.begin(Kind::Artists).unwrap();
        catalog.store_artists(generation, &artists).unwrap();

        let mut texts = catalog.texts().unwrap();
        texts.sort();

        let mut expected = ["教育", "東京事変", "椎名林檎", "群青日和", "遭難", "別の盤"];
        expected.sort_unstable();
        assert_eq!(texts, expected);
    }

    #[test]
    fn artists_come_back_whole_in_the_order_the_server_sent() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        let artists: Vec<Artist> = serde_json::from_value(serde_json::json!([
            { "id": "z", "name": "Zebra", "coverArt": "c", "albumCount": 2 },
            { "id": "a", "name": "Apple" },
        ]))
        .unwrap();
        let generation = catalog.begin(Kind::Artists).unwrap();
        catalog.store_artists(generation, &artists).unwrap();

        assert_eq!(catalog.artists().unwrap(), artists);
    }

    fn reading(text: &str, romaji: &str) -> SavedReading {
        SavedReading {
            text: text.to_owned(),
            readings: vec!["ハクジツ".to_owned()],
            romaji: romaji.to_owned(),
        }
    }

    #[test]
    fn readings_last_with_the_kana_beside_the_romaji() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        assert!(catalog.romaji("a").unwrap().is_empty());

        catalog
            .put_readings("a", &[reading("白日", "spelled")])
            .unwrap();

        let reopened = Catalog::open(dir.path()).unwrap();
        assert_eq!(reopened.romaji("a").unwrap()["白日"], "spelled");
        let kana: String = reopened
            .connection()
            .query_row("SELECT readings FROM readings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(kana, r#"["ハクジツ"]"#);
    }

    #[test]
    fn readings_add_up_within_a_source_and_another_source_replaces_them() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        catalog.put_readings("a", &[reading("白日", "")]).unwrap();
        catalog.put_readings("a", &[reading("群青", "")]).unwrap();
        assert_eq!(catalog.romaji("a").unwrap().len(), 2);
        assert!(catalog.romaji("b").unwrap().is_empty());

        catalog.put_readings("b", &[reading("群青", "")]).unwrap();
        assert_eq!(catalog.romaji("b").unwrap().len(), 1);
        assert!(catalog.romaji("a").unwrap().is_empty());
    }

    #[test]
    fn only_one_sync_holds_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(dir.path()).unwrap();
        let other = Catalog::open(dir.path()).unwrap();
        let lock = catalog.try_lock_sync().unwrap();
        assert!(lock.is_some());
        assert!(other.try_lock_sync().unwrap().is_none());
        drop(lock);
        assert!(other.try_lock_sync().unwrap().is_some());
    }
}
