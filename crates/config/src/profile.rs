use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

use crate::Error;

/// Environment variables with this prefix override the file, so
/// `HIPHILE_SERVER` replaces `server`.
const ENV_PREFIX: &str = "HIPHILE";

/// Held while `config.toml` is read, changed and written back, so two saves
/// in this process never interleave and lose one's change.
static EDITING: Mutex<()> = Mutex::new(());
/// Numbers temporary files, so no two writes ever share one.
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

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
    volume: Option<f32>,
}

/// The settings directory and its `config.toml`:
///
/// ```toml
/// server = "https://music.example.com"
/// username = "alice"
/// volume = 0.8
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
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings
            .server
            .zip(settings.username)
            .map(|(server, username)| Profile::new(server, username)))
    }

    /// The playback volume from `0.0` to `1.0`, if one was saved.
    /// `HIPHILE_VOLUME` takes precedence over the file.
    pub fn volume(&self) -> Result<Option<f32>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings
            .volume
            .filter(|volume| volume.is_finite())
            .map(|volume| volume.clamp(0.0, 1.0)))
    }

    fn read(&self, environment: config_rs::Environment) -> Result<Settings, Error> {
        let path = self.config_path();
        let read_error = |source| Error::Read {
            path: path.clone(),
            source,
        };
        config_rs::Config::builder()
            .add_source(
                config_rs::File::from(path.as_path())
                    .format(config_rs::FileFormat::Toml)
                    .required(false),
            )
            .add_source(environment)
            .build()
            .and_then(config_rs::Config::try_deserialize)
            .map_err(read_error)
    }

    /// Records `profile` in `config.toml`, keeping the file's other keys and
    /// comments.
    pub fn save_profile(&self, profile: &Profile) -> Result<(), Error> {
        self.edit(|document| {
            document["server"] = toml_edit::value(profile.server());
            document["username"] = toml_edit::value(profile.username());
        })
    }

    /// Records the playback volume, like [`Self::save_profile`].
    pub fn save_volume(&self, volume: f32) -> Result<(), Error> {
        // Two decimals are plenty, and keep the file readable.
        let volume = (f64::from(volume.clamp(0.0, 1.0)) * 100.0).round() / 100.0;
        self.edit(|document| document["volume"] = toml_edit::value(volume))
    }

    fn edit(&self, change: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<(), Error> {
        let io_error = |path: &Path| {
            let path = path.to_owned();
            move |source| Error::Io { path, source }
        };
        let _editing = EDITING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        change(&mut document);

        std::fs::create_dir_all(&self.dir).map_err(io_error(&self.dir))?;
        // Write then rename, so a crash never leaves a half-written file.
        let temporary = path.with_extension(format!(
            "toml.{}.{}.tmp",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
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
    fn read(store: &Store, env: &[(&str, &str)]) -> Settings {
        let env = env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        store
            .read(config_rs::Environment::with_prefix(ENV_PREFIX).source(Some(env)))
            .unwrap()
    }

    fn load(store: &Store, env: &[(&str, &str)]) -> Option<Profile> {
        let settings = read(store, env);
        settings
            .server
            .zip(settings.username)
            .map(|(server, username)| Profile::new(server, username))
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
    fn volume_round_trips_next_to_the_profile() {
        let store = temporary_store("volume");
        let profile = Profile::new("https://music.example.com", "alice");
        store.save_profile(&profile).unwrap();
        store.save_volume(0.123).unwrap();
        assert_eq!(read(&store, &[]).volume, Some(0.12));
        assert_eq!(load(&store, &[]), Some(profile));
        assert_eq!(read(&store, &[("HIPHILE_VOLUME", "0.5")]).volume, Some(0.5));
    }

    #[test]
    fn concurrent_saves_keep_every_change() {
        let store = temporary_store("concurrent");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        if i == 0 {
                            store
                                .save_profile(&Profile::new("https://music.example.com", "alice"))
                                .unwrap();
                        } else {
                            store.save_volume(0.5).unwrap();
                        }
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let settings = read(&store, &[]);
        assert_eq!(settings.volume, Some(0.5));
        assert_eq!(
            settings.server.as_deref(),
            Some("https://music.example.com")
        );
        let leftovers = std::fs::read_dir(store.dir())
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path() != store.config_path())
            .count();
        assert_eq!(leftovers, 0, "temporary files were left behind");
    }

    #[test]
    fn invalid_toml_is_reported_and_left_alone() {
        let store = temporary_store("invalid");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "server = ").unwrap();
        assert!(matches!(
            store.read(
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
