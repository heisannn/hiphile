use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension as _, params};
use subsonic::{ServerErrorCode, SongId};
use url::Url;

use crate::Error;
use crate::store::{BUSY_TIMEOUT, is_damaged, remove_database};

const DATABASE: &str = "scrobbles.sqlite";

/// Changing a table needs a migration: unlike the library, what is in here
/// cannot be fetched again.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS pending (
    id        INTEGER PRIMARY KEY,
    account   TEXT NOT NULL,
    song_id   TEXT NOT NULL,
    -- Milliseconds since the Unix epoch.
    played_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS pending_by_account ON pending (account, id);
";

pub trait ScrobbleTarget: Sync {
    fn scrobble(
        &self,
        song: &SongId,
        played_at: SystemTime,
    ) -> impl Future<Output = Result<(), client::Error>> + Send;
}

impl ScrobbleTarget for client::Client {
    fn scrobble(
        &self,
        song: &SongId,
        played_at: SystemTime,
    ) -> impl Future<Output = Result<(), client::Error>> + Send {
        client::Client::scrobble(self, song, played_at)
    }
}

/// One user's plays that the server has yet to hear of, so that plays made
/// without the network, or cut off by quitting, reach it later and in order.
///
/// Kept in the system data directory, not with the cached library: nothing
/// else knows of these plays. Cheap to clone; clones share one connection.
///
/// Every method blocks on the disk: call them from a thread that may block,
/// never from a UI executor.
#[derive(Clone)]
pub struct Scrobbles {
    inner: Arc<Inner>,
}

struct Inner {
    /// Which user on which server the plays are by. One file holds them all.
    account: String,
    connection: Mutex<Connection>,
    submitting: AtomicBool,
}

struct Submitting<'a>(&'a AtomicBool);

impl Drop for Submitting<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Scrobbles {
    pub fn user(server: &Url, username: &str) -> Result<Self, Error> {
        let data = dirs::data_dir().ok_or(Error::NoDataDir)?;
        Self::open(data.join("hiphile"), server, username)
    }

    pub fn open(dir: impl AsRef<Path>, server: &Url, username: &str) -> Result<Self, Error> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
        let path = dir.join(DATABASE);
        let connection = match connect(&path) {
            Ok(connection) => connection,
            // A file SQLite cannot read has no plays left to save
            Err(error) if is_damaged(&error) => {
                remove_database(&path)?;
                connect(&path)?
            }
            Err(error) => return Err(error),
        };
        let server = server.as_str().trim_end_matches('/');
        Ok(Self {
            inner: Arc::new(Inner {
                account: format!("{username}@{server}"),
                connection: Mutex::new(connection),
                submitting: AtomicBool::new(false),
            }),
        })
    }

    pub fn record(&self, song: &SongId, played_at: SystemTime) -> Result<(), Error> {
        let played_at = played_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.connection()
            .prepare_cached(
                "INSERT INTO pending (account, song_id, played_at) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![
                self.inner.account,
                song.as_str(),
                i64::try_from(played_at).unwrap_or(i64::MAX),
            ])?;
        Ok(())
    }

    /// Sends the kept plays to `target`, oldest first, forgetting each one
    /// the server has dealt with, and says how many it was. On a failure the
    /// rest wait for the next submission, which is why the error can be
    /// ignored.
    ///
    /// Does nothing while another submission of these plays is running. That
    /// one sends a play kept meanwhile, unless it was just finishing; such a
    /// play waits for the next submission too.
    pub async fn submit(&self, target: &impl ScrobbleTarget) -> Result<usize, Error> {
        if self.inner.submitting.swap(true, Ordering::Acquire) {
            return Ok(0);
        }
        let _submitting = Submitting(&self.inner.submitting);
        let mut submitted = 0;
        while let Some((id, song, played_at)) = self.oldest()? {
            match target.scrobble(&song, played_at).await {
                Ok(()) => {}
                Err(error) if is_refusal(&error) => {}
                Err(error) => return Err(error.into()),
            }
            self.forget(id)?;
            submitted += 1;
        }
        Ok(submitted)
    }

    fn oldest(&self) -> Result<Option<(i64, SongId, SystemTime)>, Error> {
        Ok(self
            .connection()
            .prepare_cached(
                "SELECT id, song_id, played_at FROM pending WHERE account = ?1 ORDER BY id LIMIT 1",
            )?
            .query_row([&self.inner.account], |row| {
                let played_at = u64::try_from(row.get::<_, i64>(2)?).unwrap_or_default();
                Ok((
                    row.get(0)?,
                    SongId::new(row.get::<_, String>(1)?),
                    UNIX_EPOCH + Duration::from_millis(played_at),
                ))
            })
            .optional()?)
    }

    fn forget(&self, id: i64) -> Result<(), Error> {
        self.connection()
            .prepare_cached("DELETE FROM pending WHERE id = ?1")?
            .execute([id])?;
        Ok(())
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.inner
            .connection
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

/// Whether the server turned down this play for good: it no longer has the
/// song. Sending it again would only hold up the plays behind it. Any other
/// error may pass, and says nothing against the play, so it must not cost one.
fn is_refusal(error: &client::Error) -> bool {
    matches!(
        error,
        client::Error::Protocol(subsonic::Error::Server {
            code: ServerErrorCode::NOT_FOUND,
            ..
        })
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use futures::executor::block_on;

    use super::*;

    #[derive(Default)]
    struct Fake {
        answers: Mutex<VecDeque<Result<(), client::Error>>>,
        received: Mutex<Vec<(String, SystemTime)>>,
    }

    impl Fake {
        fn answering(answers: impl IntoIterator<Item = Result<(), client::Error>>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().collect()),
                ..Self::default()
            }
        }

        fn received_songs(&self) -> Vec<String> {
            let received = self.received.lock().unwrap();
            received.iter().map(|(song, _)| song.clone()).collect()
        }
    }

    impl ScrobbleTarget for Fake {
        async fn scrobble(
            &self,
            song: &SongId,
            played_at: SystemTime,
        ) -> Result<(), client::Error> {
            self.received
                .lock()
                .unwrap()
                .push((song.as_str().to_owned(), played_at));
            self.answers.lock().unwrap().pop_front().unwrap_or(Ok(()))
        }
    }

    fn server() -> Url {
        Url::parse("https://music.example.com/").unwrap()
    }

    fn scrobbles(dir: &tempfile::TempDir) -> Scrobbles {
        Scrobbles::open(dir.path(), &server(), "alice").unwrap()
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn refused(code: ServerErrorCode) -> client::Error {
        client::Error::Protocol(subsonic::Error::Server {
            code,
            message: String::new(),
        })
    }

    fn record(scrobbles: &Scrobbles, songs: &[&str]) {
        for song in songs {
            scrobbles.record(&SongId::new(*song), at(100)).unwrap();
        }
    }

    #[test]
    fn sends_plays_oldest_first_with_their_times_and_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let scrobbles = scrobbles(&dir);
        scrobbles.record(&SongId::new("a"), at(100)).unwrap();
        scrobbles.record(&SongId::new("b"), at(200)).unwrap();

        let target = Fake::default();
        assert_eq!(block_on(scrobbles.submit(&target)).unwrap(), 2);
        assert_eq!(block_on(scrobbles.submit(&target)).unwrap(), 0);

        assert_eq!(
            *target.received.lock().unwrap(),
            [("a".to_owned(), at(100)), ("b".to_owned(), at(200))]
        );
    }

    #[test]
    fn a_failure_keeps_that_play_and_those_behind_it_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let scrobbles = scrobbles(&dir);
        record(&scrobbles, &["a", "b", "c"]);

        let failing = Fake::answering([Ok(()), Err(client::Error::HttpStatus(502))]);
        assert!(block_on(scrobbles.submit(&failing)).is_err());
        assert_eq!(failing.received_songs(), ["a", "b"]);

        let target = Fake::default();
        block_on(scrobbles.submit(&target)).unwrap();
        assert_eq!(target.received_songs(), ["b", "c"]);
    }

    #[test]
    fn plays_outlast_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        record(&scrobbles(&dir), &["a"]);

        let target = Fake::default();
        block_on(scrobbles(&dir).submit(&target)).unwrap();
        assert_eq!(target.received_songs(), ["a"]);
    }

    #[test]
    fn a_refused_play_is_dropped_and_does_not_hold_up_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let scrobbles = scrobbles(&dir);
        record(&scrobbles, &["gone", "b"]);

        let target = Fake::answering([Err(refused(ServerErrorCode::NOT_FOUND))]);
        block_on(scrobbles.submit(&target)).unwrap();
        assert_eq!(target.received_songs(), ["gone", "b"]);

        let again = Fake::default();
        block_on(scrobbles.submit(&again)).unwrap();
        assert!(again.received_songs().is_empty());
    }

    #[test]
    fn plays_wait_through_any_other_server_error() {
        for code in [
            ServerErrorCode::GENERIC,
            ServerErrorCode::WRONG_CREDENTIALS,
            ServerErrorCode::NOT_AUTHORIZED,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let scrobbles = scrobbles(&dir);
            record(&scrobbles, &["a"]);

            let target = Fake::answering([Err(refused(code))]);
            assert!(block_on(scrobbles.submit(&target)).is_err());

            let again = Fake::default();
            block_on(scrobbles.submit(&again)).unwrap();
            assert_eq!(again.received_songs(), ["a"]);
        }
    }

    #[test]
    fn each_user_on_each_server_sends_only_their_own_plays() {
        let dir = tempfile::tempdir().unwrap();
        record(&scrobbles(&dir), &["alice"]);
        let bob = Scrobbles::open(dir.path(), &server(), "bob").unwrap();
        record(&bob, &["bob"]);
        let other = Url::parse("https://other.example.com").unwrap();
        let elsewhere = Scrobbles::open(dir.path(), &other, "alice").unwrap();
        record(&elsewhere, &["elsewhere"]);

        let target = Fake::default();
        block_on(bob.submit(&target)).unwrap();
        assert_eq!(target.received_songs(), ["bob"]);

        let target = Fake::default();
        block_on(scrobbles(&dir).submit(&target)).unwrap();
        assert_eq!(target.received_songs(), ["alice"]);
    }

    #[test]
    fn a_server_address_with_a_trailing_slash_is_the_same_account() {
        let dir = tempfile::tempdir().unwrap();
        let with = Url::parse("https://example.com/navidrome/").unwrap();
        let without = Url::parse("https://example.com/navidrome").unwrap();
        record(
            &Scrobbles::open(dir.path(), &with, "alice").unwrap(),
            &["a"],
        );

        let target = Fake::default();
        let scrobbles = Scrobbles::open(dir.path(), &without, "alice").unwrap();
        block_on(scrobbles.submit(&target)).unwrap();
        assert_eq!(target.received_songs(), ["a"]);
    }

    #[test]
    fn a_submission_running_keeps_a_second_from_sending_the_same_plays() {
        let dir = tempfile::tempdir().unwrap();
        let scrobbles = scrobbles(&dir);
        record(&scrobbles, &["a"]);

        assert!(!scrobbles.inner.submitting.swap(true, Ordering::Acquire));
        let target = Fake::default();
        block_on(scrobbles.submit(&target)).unwrap();
        assert!(target.received_songs().is_empty());

        scrobbles.inner.submitting.store(false, Ordering::Release);
        block_on(scrobbles.submit(&target)).unwrap();
        assert_eq!(target.received_songs(), ["a"]);
    }

    #[test]
    fn a_failed_submission_lets_the_next_one_run() {
        let dir = tempfile::tempdir().unwrap();
        let scrobbles = scrobbles(&dir);
        record(&scrobbles, &["a"]);

        let failing = Fake::answering([Err(client::Error::HttpStatus(502))]);
        assert!(block_on(scrobbles.submit(&failing)).is_err());
        assert!(!scrobbles.inner.submitting.load(Ordering::Acquire));
    }

    #[test]
    fn a_damaged_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(DATABASE), "not a database, at some length").unwrap();

        let scrobbles = scrobbles(&dir);
        record(&scrobbles, &["a"]);
        let target = Fake::default();
        block_on(scrobbles.submit(&target)).unwrap();
        assert_eq!(target.received_songs(), ["a"]);
    }
}
