//! The token requests are made with: given, kept on disk, or asked for.

use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::lock::Mutex;
use serde::{Deserialize, Serialize};

use super::Client;
use crate::Error;

/// How long a token asked for is used. The service does not say how long one
/// lasts.
const LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
/// How long after the service answers a request for a token another may be
/// asked for, whether or not it gave one. How often it allows that is not known,
/// and it has refused to when asked often.
const REQUEST_PAUSE: Duration = Duration::from_secs(5 * 60);
const FORMAT_VERSION: u32 = 1;

/// A token and whether it can be replaced by asking for another.
#[derive(Clone, PartialEq)]
pub(crate) struct Token {
    value: Arc<str>,
    replaceable: bool,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("value", &"<hidden>")
            .field("replaceable", &self.replaceable)
            .finish()
    }
}

impl Token {
    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn is_replaceable(&self) -> bool {
        self.replaceable
    }
}

/// Where tokens come from. Requests for a token wait for one another, so that
/// songs looked up together share one token.
pub(crate) struct Tokens {
    source: Mutex<Source>,
}

enum Source {
    Given(Token),
    Kept {
        path: Option<PathBuf>,
        current: Option<KeptToken>,
        last_request: Option<Instant>,
    },
}

#[derive(Clone)]
struct KeptToken {
    token: Token,
    created_at: SystemTime,
}

impl KeptToken {
    fn is_expired(&self) -> bool {
        self.created_at
            .elapsed()
            .map_or(true, |age| age >= LIFETIME)
    }
}

/// A token on disk.
#[derive(Deserialize, Serialize)]
struct TokenFile {
    version: u32,
    usertoken: String,
    /// Seconds since the Unix epoch.
    created_at: u64,
}

impl Tokens {
    /// Always `token`, which is never written anywhere.
    pub(crate) fn given(token: String) -> Self {
        Self {
            source: Mutex::new(Source::Given(Token {
                value: token.into(),
                replaceable: false,
            })),
        }
    }

    /// Tokens asked for and kept at `path`, or only in memory without one.
    pub(crate) fn kept_in(path: Option<PathBuf>) -> Self {
        Self {
            source: Mutex::new(Source::Kept {
                path,
                current: None,
                last_request: None,
            }),
        }
    }

    /// The token to use, asking for one if there is none still in date.
    pub(crate) async fn current(&self, client: &Client) -> Result<Token, Error> {
        let mut source = self.source.lock().await;
        let (path, current, last_request) = match &mut *source {
            Source::Given(token) => return Ok(token.clone()),
            Source::Kept {
                path,
                current,
                last_request,
            } => (path, current, last_request),
        };
        if current.as_ref().is_none_or(KeptToken::is_expired) {
            *current = path.as_deref().and_then(read_token);
        }
        if let Some(kept) = current.as_ref().filter(|kept| !kept.is_expired()) {
            return Ok(kept.token.clone());
        }
        let kept = request(client, path.as_deref(), last_request).await?;
        let token = kept.token.clone();
        *current = Some(kept);
        Ok(token)
    }

    /// A token to use instead of `refused`. Failing leaves the one there is in
    /// place, in memory and on disk.
    pub(crate) async fn replace(&self, client: &Client, refused: &Token) -> Result<Token, Error> {
        let mut source = self.source.lock().await;
        let (path, current, last_request) = match &mut *source {
            Source::Given(token) => return Ok(token.clone()),
            Source::Kept {
                path,
                current,
                last_request,
            } => (path, current, last_request),
        };
        // Another request may have replaced it while this one was refused.
        if let Some(kept) = current.as_ref().filter(|kept| kept.token != *refused) {
            return Ok(kept.token.clone());
        }
        let kept = request(client, path.as_deref(), last_request).await?;
        let token = kept.token.clone();
        *current = Some(kept);
        Ok(token)
    }

    #[cfg(test)]
    pub(crate) fn forget_last_request(&self) {
        if let Some(mut source) = self.source.try_lock()
            && let Source::Kept { last_request, .. } = &mut *source
        {
            *last_request = None;
        }
    }
}

/// Asks the service for a token, unless it answered moments ago. A request given
/// up on or that never reached the service says nothing of what it allows.
async fn request(
    client: &Client,
    path: Option<&Path>,
    last_request: &mut Option<Instant>,
) -> Result<KeptToken, Error> {
    if last_request.is_some_and(|last| last.elapsed() < REQUEST_PAUSE) {
        return Err(Error::TokenUnavailable);
    }
    let answer = client.request_token().await;
    if !matches!(answer, Err(Error::Network(_))) {
        *last_request = Some(Instant::now());
    }
    let value = answer?;
    let created_at = SystemTime::now();
    // Failing to keep it only means asking again on the next launch.
    if let Some(path) = path {
        write_token(path, &value, created_at).ok();
    }
    Ok(KeptToken {
        token: Token {
            value: value.into(),
            replaceable: true,
        },
        created_at,
    })
}

/// The token at `path`, whatever its age, unless the file can't be used.
fn read_token(path: &Path) -> Option<KeptToken> {
    let file: TokenFile = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if file.version != FORMAT_VERSION || file.usertoken.is_empty() {
        return None;
    }
    Some(KeptToken {
        token: Token {
            value: file.usertoken.into(),
            replaceable: true,
        },
        created_at: UNIX_EPOCH.checked_add(Duration::from_secs(file.created_at))?,
    })
}

/// Replaces the file in one step. The temporary file is readable by this user
/// alone, and the replaced file takes on that.
fn write_token(path: &Path, value: &str, created_at: SystemTime) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let file = TokenFile {
        version: FORMAT_VERSION,
        usertoken: value.to_owned(),
        created_at: created_at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |age| age.as_secs()),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(&serde_json::to_vec(&file)?)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(age: Duration) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        write_token(&path, "kept", SystemTime::now() - age).unwrap();
        (dir, path)
    }

    #[test]
    fn a_token_on_disk_reads_back_with_its_age() {
        let (_dir, path) = written(Duration::from_secs(60));
        let kept = read_token(&path).unwrap();
        assert_eq!(kept.token.value(), "kept");
        assert!(kept.token.is_replaceable());
        assert!(!kept.is_expired());
    }

    #[test]
    fn a_token_past_its_lifetime_is_expired() {
        let (_dir, path) = written(LIFETIME + Duration::from_secs(60));
        assert!(read_token(&path).unwrap().is_expired());
    }

    #[test]
    fn a_damaged_or_foreign_file_is_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        assert!(read_token(&path).is_none());
        fs::write(&path, "{ not json").unwrap();
        assert!(read_token(&path).is_none());
        fs::write(&path, r#"{"version":99,"usertoken":"t","created_at":1}"#).unwrap();
        assert!(read_token(&path).is_none());
        fs::write(&path, r#"{"version":1,"usertoken":"","created_at":1}"#).unwrap();
        assert!(read_token(&path).is_none());
        let far_off = format!(
            r#"{{"version":1,"usertoken":"t","created_at":{}}}"#,
            u64::MAX
        );
        fs::write(&path, far_off).unwrap();
        assert!(read_token(&path).is_none());
    }

    #[test]
    fn debug_output_does_not_show_a_token() {
        let (_dir, path) = written(Duration::ZERO);
        let token = read_token(&path).unwrap().token;
        assert!(!format!("{token:?}").contains("kept"));
    }
}
