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

/// Light or dark, chosen rather than following the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// The keys of `config.toml` this crate reads. Others are left alone.
#[derive(Deserialize)]
struct Settings {
    server: Option<String>,
    username: Option<String>,
    volume: Option<f32>,
    song_cache_gib: Option<f64>,
    output_device: Option<String>,
    sample_rate: Option<u32>,
    appearance: Option<String>,
}

/// The settings directory and its `config.toml`:
///
/// ```toml
/// server = "https://music.example.com"
/// username = "alice"
/// volume = 0.8
/// song_cache_gib = 8
/// output_device = "coreaudio:BuiltInSpeakerDevice"
/// sample_rate = 48000
/// appearance = "dark"
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

    /// How many bytes of songs may be kept on this computer, if the file
    /// says: `song_cache_gib`, in GiB, where `0` means no limit.
    /// `HIPHILE_SONG_CACHE_GIB` takes precedence over the file. A negative
    /// number is an error, not a reason to keep the default.
    pub fn song_cache_capacity(&self) -> Result<Option<u64>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.song_cache_bytes(settings.song_cache_gib)
    }

    /// `song_cache_gib`, as [`Self::song_cache_capacity`] reads it.
    pub fn song_cache_gib(&self) -> Result<Option<f64>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.song_cache_bytes(settings.song_cache_gib)?;
        Ok(settings.song_cache_gib)
    }

    /// The id of the output device to play on, if one was chosen rather than
    /// the system's default. `HIPHILE_OUTPUT_DEVICE` takes precedence over the
    /// file.
    pub fn output_device(&self) -> Result<Option<String>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.output_device)
    }

    /// The rate in Hz to run the output device at, if one was chosen rather
    /// than each song's own. `HIPHILE_SAMPLE_RATE` takes precedence over the
    /// file.
    pub fn sample_rate(&self) -> Result<Option<u32>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        Ok(settings.sample_rate.filter(|rate| *rate > 0))
    }

    /// Light or dark, if one was chosen rather than following the system.
    /// `HIPHILE_APPEARANCE` takes precedence over the file, and `system` there
    /// sets a choice in the file aside.
    pub fn appearance(&self) -> Result<Option<Appearance>, Error> {
        let settings = self.read(config_rs::Environment::with_prefix(ENV_PREFIX))?;
        self.parse_appearance(settings.appearance.as_deref())
    }

    fn parse_appearance(&self, value: Option<&str>) -> Result<Option<Appearance>, Error> {
        match value {
            None | Some("system") => Ok(None),
            Some("light") => Ok(Some(Appearance::Light)),
            Some("dark") => Ok(Some(Appearance::Dark)),
            Some(_) => Err(Error::InvalidSetting {
                path: self.config_path(),
                key: "appearance",
                expected: "\"system\", \"light\" or \"dark\"",
            }),
        }
    }

    fn song_cache_bytes(&self, gib: Option<f64>) -> Result<Option<u64>, Error> {
        let Some(gib) = gib else {
            return Ok(None);
        };
        song_cache_capacity_of(gib)
            .map(Some)
            .ok_or_else(|| Error::InvalidSetting {
                path: self.config_path(),
                key: "song_cache_gib",
                expected: "a number of GiB, 0 or more",
            })
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

    /// Records the output device, like [`Self::save_profile`]. `None` removes
    /// the key, which means the system's default device.
    pub fn save_output_device(&self, device: Option<&str>) -> Result<(), Error> {
        self.edit(|document| match device {
            Some(device) => document["output_device"] = toml_edit::value(device),
            None => {
                document.remove("output_device");
            }
        })
    }

    /// Records the output rate in Hz, like [`Self::save_profile`]. `None`
    /// removes the key, which means each song's own rate.
    pub fn save_sample_rate(&self, sample_rate: Option<u32>) -> Result<(), Error> {
        self.edit(|document| match sample_rate {
            Some(rate) => document["sample_rate"] = toml_edit::value(i64::from(rate)),
            None => {
                document.remove("sample_rate");
            }
        })
    }

    /// Records how many GiB of songs to keep, like [`Self::save_profile`].
    /// `None` removes the key, which means the default amount. A negative
    /// number would make the setting unreadable, so callers keep to `0` or
    /// more.
    pub fn save_song_cache_gib(&self, gib: Option<f64>) -> Result<(), Error> {
        self.edit(|document| match gib {
            Some(gib) => document["song_cache_gib"] = toml_edit::value(gib),
            None => {
                document.remove("song_cache_gib");
            }
        })
    }

    /// Records the appearance, like [`Self::save_profile`]. `None` removes the
    /// key, which means following the system.
    pub fn save_appearance(&self, appearance: Option<Appearance>) -> Result<(), Error> {
        self.edit(|document| match appearance {
            Some(Appearance::Light) => document["appearance"] = toml_edit::value("light"),
            Some(Appearance::Dark) => document["appearance"] = toml_edit::value("dark"),
            None => {
                document.remove("appearance");
            }
        })
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

/// How many bytes of songs a `song_cache_gib` of `gib` keeps, where `0` means no
/// limit; `None` for a negative number or one that is not.
pub fn song_cache_capacity_of(gib: f64) -> Option<u64> {
    (gib.is_finite() && gib >= 0.0).then(|| {
        if gib == 0.0 {
            u64::MAX
        } else {
            // Saturates for sizes no disk has.
            (gib * f64::from(1u32 << 30)) as u64
        }
    })
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
    fn output_round_trips_and_clears() {
        let store = temporary_store("output");
        let profile = Profile::new("https://music.example.com", "alice");
        store.save_profile(&profile).unwrap();

        store.save_output_device(Some("coreaudio:device")).unwrap();
        store.save_sample_rate(Some(96_000)).unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.output_device.as_deref(), Some("coreaudio:device"));
        assert_eq!(settings.sample_rate, Some(96_000));

        store.save_output_device(None).unwrap();
        store.save_sample_rate(None).unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.output_device, None);
        assert_eq!(settings.sample_rate, None);
        assert_eq!(load(&store, &[]), Some(profile));
    }

    #[test]
    fn saving_one_output_key_leaves_the_other_alone() {
        let store = temporary_store("output-one-key");
        store.save_output_device(Some("a:b")).unwrap();
        store.save_sample_rate(Some(44_100)).unwrap();

        store.save_sample_rate(None).unwrap();
        assert_eq!(read(&store, &[]).output_device.as_deref(), Some("a:b"));

        store.save_sample_rate(Some(96_000)).unwrap();
        store.save_output_device(None).unwrap();
        assert_eq!(read(&store, &[]).sample_rate, Some(96_000));
    }

    #[test]
    fn saving_an_output_key_leaves_a_key_it_cannot_read_alone() {
        let store = temporary_store("output-unreadable");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.config_path(),
            "volume = \"loud\"\noutput_device = \"a:b\"\n",
        )
        .unwrap();
        store.save_sample_rate(Some(96_000)).unwrap();
        let text = std::fs::read_to_string(store.config_path()).unwrap();
        assert!(text.contains("volume = \"loud\""), "{text}");
        assert!(text.contains("output_device = \"a:b\""), "{text}");
        assert!(text.contains("sample_rate = 96000"), "{text}");
    }

    #[test]
    fn environment_overrides_the_saved_output() {
        let store = temporary_store("output-environment");
        store.save_output_device(Some("a:b")).unwrap();
        store.save_sample_rate(Some(44_100)).unwrap();
        let settings = read(
            &store,
            &[
                ("HIPHILE_OUTPUT_DEVICE", "c:d"),
                ("HIPHILE_SAMPLE_RATE", "48000"),
            ],
        );
        assert_eq!(settings.output_device.as_deref(), Some("c:d"));
        assert_eq!(settings.sample_rate, Some(48_000));
    }

    #[test]
    fn song_cache_capacity_is_read_in_gib() {
        let store = temporary_store("song-cache");
        let capacity = |env: &[(&str, &str)]| {
            store
                .song_cache_bytes(read(&store, env).song_cache_gib)
                .map_err(|error| error.to_string())
        };
        assert_eq!(capacity(&[]), Ok(None));
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "song_cache_gib = 2\n").unwrap();
        assert_eq!(capacity(&[]), Ok(Some(2 << 30)));
        assert_eq!(
            capacity(&[("HIPHILE_SONG_CACHE_GIB", "0.5")]),
            Ok(Some(512 << 20))
        );
        assert_eq!(
            capacity(&[("HIPHILE_SONG_CACHE_GIB", "0")]),
            Ok(Some(u64::MAX))
        );
        let negative = capacity(&[("HIPHILE_SONG_CACHE_GIB", "-1")]).unwrap_err();
        assert!(negative.contains("HIPHILE_SONG_CACHE_GIB"), "{negative}");
    }

    #[test]
    fn song_cache_gib_round_trips_and_clears() {
        let store = temporary_store("song-cache-save");
        store.save_volume(0.5).unwrap();

        store.save_song_cache_gib(Some(0.5)).unwrap();
        assert_eq!(read(&store, &[]).song_cache_gib, Some(0.5));
        assert_eq!(
            store
                .song_cache_bytes(read(&store, &[]).song_cache_gib)
                .unwrap(),
            Some(512 << 20)
        );

        store.save_song_cache_gib(Some(0.0)).unwrap();
        assert_eq!(read(&store, &[]).song_cache_gib, Some(0.0));

        store.save_song_cache_gib(None).unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.song_cache_gib, None);
        assert_eq!(settings.volume, Some(0.5));
    }

    #[test]
    fn appearance_round_trips_and_clears() {
        let store = temporary_store("appearance");
        store.save_volume(0.5).unwrap();
        let appearance = |env: &[(&str, &str)]| {
            store
                .parse_appearance(read(&store, env).appearance.as_deref())
                .unwrap()
        };
        assert_eq!(appearance(&[]), None);

        store.save_appearance(Some(Appearance::Dark)).unwrap();
        assert_eq!(appearance(&[]), Some(Appearance::Dark));
        store.save_appearance(Some(Appearance::Light)).unwrap();
        assert_eq!(appearance(&[]), Some(Appearance::Light));

        store.save_appearance(None).unwrap();
        assert_eq!(appearance(&[]), None);
        assert_eq!(read(&store, &[]).volume, Some(0.5));
    }

    #[test]
    fn environment_overrides_the_saved_appearance() {
        let store = temporary_store("appearance-environment");
        store.save_appearance(Some(Appearance::Light)).unwrap();
        let appearance = |value| {
            store
                .parse_appearance(
                    read(&store, &[("HIPHILE_APPEARANCE", value)])
                        .appearance
                        .as_deref(),
                )
                .unwrap()
        };
        assert_eq!(appearance("dark"), Some(Appearance::Dark));
        assert_eq!(appearance("system"), None);
    }

    #[test]
    fn an_unknown_appearance_is_an_error_that_leaves_other_settings_readable() {
        let store = temporary_store("appearance-unknown");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.config_path(), "appearance = \"blue\"\nvolume = 0.5\n").unwrap();
        let settings = read(&store, &[]);
        assert_eq!(settings.volume, Some(0.5));
        assert!(matches!(
            store.parse_appearance(settings.appearance.as_deref()),
            Err(Error::InvalidSetting {
                key: "appearance",
                ..
            })
        ));
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
