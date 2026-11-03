//! Server profiles and preferences shared by the GUI and the CLI.
//!
//! Profiles live in the platform config directory. They never hold a
//! password or API key; those belong in the system keychain.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no configuration directory is available on this system")]
    NoConfigDir,
    #[error("could not read or write {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is not a valid profile: {source}", path.display())]
    Invalid {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// The server and user last signed in to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    server: String,
    username: String,
}

impl Profile {
    pub fn new(server: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            username: username.into(),
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn username(&self) -> &str {
        &self.username
    }
}

/// Where profiles are stored.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The platform config directory, e.g. `~/Library/Application Support/dev.heisannn.hiphile`.
    pub fn platform() -> Result<Self, Error> {
        let dirs = directories::ProjectDirs::from("dev", "heisannn", "hiphile")
            .ok_or(Error::NoConfigDir)?;
        Ok(Self::at(dirs.config_dir()))
    }

    pub fn at(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_owned(),
        }
    }

    fn profile_path(&self) -> PathBuf {
        self.dir.join("profile.json")
    }

    /// The last saved profile, or `None` before the first sign-in.
    pub fn last_profile(&self) -> Result<Option<Profile>, Error> {
        let path = self.profile_path();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(Error::Io { path, source }),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| Error::Invalid { path, source })
    }

    pub fn save_profile(&self, profile: &Profile) -> Result<(), Error> {
        let io_error = |path: &Path| {
            let path = path.to_owned();
            move |source| Error::Io { path, source }
        };
        std::fs::create_dir_all(&self.dir).map_err(io_error(&self.dir))?;
        let path = self.profile_path();
        let json = serde_json::to_vec_pretty(profile).expect("a profile always serializes");
        // Write then rename, so a crash never leaves a half-written profile.
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, json).map_err(io_error(&temporary))?;
        std::fs::rename(&temporary, &path).map_err(io_error(&path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_store(name: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("hiphile-config-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Store::at(dir)
    }

    #[test]
    fn missing_profile_is_none() {
        assert_eq!(temporary_store("missing").last_profile().unwrap(), None);
    }

    #[test]
    fn saved_profile_round_trips() {
        let store = temporary_store("round-trip");
        let profile = Profile::new("https://music.example.com", "alice");
        store.save_profile(&profile).unwrap();
        assert_eq!(store.last_profile().unwrap(), Some(profile));
    }

    #[test]
    fn corrupt_profile_is_reported() {
        let store = temporary_store("corrupt");
        std::fs::create_dir_all(&store.dir).unwrap();
        std::fs::write(store.profile_path(), b"{").unwrap();
        assert!(matches!(store.last_profile(), Err(Error::Invalid { .. })));
    }
}
