use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Error;

/// Environment variables with this prefix override the file, so
/// `HIPHILE_SERVER` replaces `server`.
const ENV_PREFIX: &str = "HIPHILE";

/// The server and user last signed in to.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// The keys of `config.toml` this crate reads. Others are left alone.
#[derive(Deserialize)]
struct Settings {
    server: Option<String>,
    username: Option<String>,
}

/// The settings directory and its `config.toml`:
///
/// ```toml
/// server = "https://music.example.com"
/// username = "alice"
/// ```
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// `~/.config/hiphile`, on every platform, so the files are in the same
    /// place wherever the app runs.
    pub fn user() -> Result<Self, Error> {
        let home = dirs::home_dir().ok_or(Error::NoHomeDir)?;
        Ok(Self::at(home.join(".config").join("hiphile")))
    }

    pub fn at(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_owned(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    /// The last profile signed in to, with `HIPHILE_SERVER` and
    /// `HIPHILE_USERNAME` taking precedence over the file. `None` until both
    /// are known.
    pub fn last_profile(&self) -> Result<Option<Profile>, Error> {
        self.load(config_rs::Environment::with_prefix(ENV_PREFIX))
    }

    fn load(&self, environment: config_rs::Environment) -> Result<Option<Profile>, Error> {
        let path = self.config_path();
        let read_error = |source| Error::Read {
            path: path.clone(),
            source,
        };
        let settings: Settings = config_rs::Config::builder()
            .add_source(
                config_rs::File::from(path.as_path())
                    .format(config_rs::FileFormat::Toml)
                    .required(false),
            )
            .add_source(environment)
            .build()
            .and_then(config_rs::Config::try_deserialize)
            .map_err(read_error)?;
        Ok(settings
            .server
            .zip(settings.username)
            .map(|(server, username)| Profile::new(server, username)))
    }

    /// Records `profile` in `config.toml`, keeping the file's other keys and
    /// comments.
    pub fn save_profile(&self, profile: &Profile) -> Result<(), Error> {
        let io_error = |path: &Path| {
            let path = path.to_owned();
            move |source| Error::Io { path, source }
        };
        let path = self.config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => return Err(Error::Io { path, source }),
        };
        let mut document: toml_edit::DocumentMut =
            text.parse().map_err(|source| Error::Unparsable {
                path: path.clone(),
                source,
            })?;
        document["server"] = toml_edit::value(profile.server());
        document["username"] = toml_edit::value(profile.username());

        std::fs::create_dir_all(&self.dir).map_err(io_error(&self.dir))?;
        // Write then rename, so a crash never leaves a half-written file.
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, document.to_string()).map_err(io_error(&temporary))?;
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

    /// Reads without the process environment, so tests do not depend on it.
    fn load(store: &Store, env: &[(&str, &str)]) -> Option<Profile> {
        let env = env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        store
            .load(config_rs::Environment::with_prefix(ENV_PREFIX).source(Some(env)))
            .unwrap()
    }

    #[test]
    fn user_store_is_under_the_home_config_dir() {
        let store = Store::user().unwrap();
        assert!(
            store.dir().ends_with(".config/hiphile"),
            "{:?}",
            store.dir()
        );
        assert!(store.dir().starts_with(dirs::home_dir().unwrap()));
    }

    #[test]
    fn missing_file_means_no_profile() {
        assert_eq!(load(&temporary_store("missing"), &[]), None);
    }

    #[test]
    fn saved_profile_round_trips() {
        let store = temporary_store("round-trip");
        let profile = Profile::new("https://music.example.com", "alice");
        store.save_profile(&profile).unwrap();
        assert_eq!(load(&store, &[]), Some(profile));
    }

    #[test]
    fn environment_overrides_the_file() {
        let store = temporary_store("environment");
        store
            .save_profile(&Profile::new("https://music.example.com", "alice"))
            .unwrap();
        assert_eq!(
            load(&store, &[("HIPHILE_SERVER", "https://other.example.com")]),
            Some(Profile::new("https://other.example.com", "alice"))
        );
    }

    #[test]
    fn environment_alone_is_a_profile() {
        let store = temporary_store("environment-alone");
        assert_eq!(
            load(
                &store,
                &[
                    ("HIPHILE_SERVER", "https://music.example.com"),
                    ("HIPHILE_USERNAME", "bob"),
                ]
            ),
            Some(Profile::new("https://music.example.com", "bob"))
        );
    }

    #[test]
    fn saving_keeps_other_settings_and_comments() {
        let store = temporary_store("keep");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "# My settings\nserver = \"https://old.example.com\"\nvolume = 0.5\n",
        )
        .unwrap();
        store
            .save_profile(&Profile::new("https://music.example.com", "alice"))
            .unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("# My settings"), "{text}");
        assert!(text.contains("volume = 0.5"), "{text}");
        assert!(
            text.contains("server = \"https://music.example.com\""),
            "{text}"
        );
        assert!(text.contains("username = \"alice\""), "{text}");
    }

    #[test]
    fn invalid_toml_is_reported_and_left_alone() {
        let store = temporary_store("invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "server = ").unwrap();
        assert!(matches!(
            store.load(
                config_rs::Environment::with_prefix(ENV_PREFIX).source(Some(Default::default()))
            ),
            Err(Error::Read { .. })
        ));
        assert!(matches!(
            store.save_profile(&Profile::new("https://music.example.com", "alice")),
            Err(Error::Unparsable { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(store.config_path()).unwrap(),
            "server = "
        );
    }
}
