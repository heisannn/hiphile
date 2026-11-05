use std::io::Write as _;
use std::time::Duration;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
use futures::executor::block_on;
use subsonic::{Credentials, Song, SongId};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check that the server is reachable and the credentials are accepted.
    Ping(ServerArgs),
    /// List audio output devices.
    Devices,
    /// Fetch the whole library into the copy kept on this computer.
    Sync(ServerArgs),
    /// List the albums kept on this computer, updating them first when the
    /// server reports a change or the copy is a day old.
    Albums {
        #[command(flatten)]
        server: ServerArgs,
        /// Fetch every album from the server first.
        #[arg(long, conflicts_with = "offline")]
        refresh: bool,
        /// List the saved albums without contacting the server.
        #[arg(long)]
        offline: bool,
    },
    /// Search songs and print their IDs.
    Search {
        #[command(flatten)]
        server: ServerArgs,
        query: String,
        #[arg(long, default_value_t = 20)]
        count: u32,
    },
    /// Play songs on the default output device.
    Play {
        #[command(flatten)]
        server: ServerArgs,
        /// Song IDs, played in order.
        #[arg(required_unless_present = "search")]
        ids: Vec<String>,
        /// Play the songs matching this query instead of IDs.
        #[arg(long, conflicts_with = "ids")]
        search: Option<String>,
    },
}

/// Where to connect. Anything left out comes from the last sign-in in the
/// app: its server, its user, and the password in `config.toml` or else the
/// keychain.
#[derive(Args)]
struct ServerArgs {
    /// Server address, e.g. https://music.example.com
    #[arg(long, env = "HIPHILE_SERVER")]
    server: Option<String>,
    #[arg(long, env = "HIPHILE_USERNAME", conflicts_with = "api_key")]
    username: Option<String>,
    #[arg(
        long,
        env = "HIPHILE_PASSWORD",
        hide_env_values = true,
        conflicts_with = "api_key"
    )]
    password: Option<String>,
    /// OpenSubsonic API key, used instead of username and password.
    #[arg(long, env = "HIPHILE_API_KEY", hide_env_values = true)]
    api_key: Option<String>,
}

fn last_profile() -> Option<config::Profile> {
    config::Store::user()
        .ok()
        .and_then(|store| store.last_profile().ok().flatten())
}

impl ServerArgs {
    fn address(&self) -> anyhow::Result<String> {
        self.server
            .clone()
            .or_else(|| last_profile().map(|profile| profile.server().to_owned()))
            .context("pass --server (or set HIPHILE_SERVER), or sign in once in the app")
    }

    fn catalog(&self) -> anyhow::Result<catalog::Catalog> {
        let url = client::server_url(&self.address()?)?;
        Ok(catalog::Catalog::open_server(&url)?)
    }

    fn client(self) -> anyhow::Result<client::Client> {
        let profile = last_profile();
        let address = self.address()?;
        let credentials = match self.api_key {
            Some(key) => Credentials::api_key(key),
            None => {
                let username = self
                    .username
                    .or_else(|| {
                        profile
                            .filter(|profile| profile.server() == address)
                            .map(|profile| profile.username().to_owned())
                    })
                    .context("pass --username (or set HIPHILE_USERNAME)")?;
                let password = match self.password {
                    Some(password) => password,
                    None => saved_password(&address, &username)?,
                };
                Credentials::password(username, password)
            }
        };
        let server = client::Server::new(&address, credentials)?;
        let user_agent = concat!("hiphile-cli/", env!("CARGO_PKG_VERSION"));
        Ok(client::Client::new(net::Client::new(user_agent)?, server))
    }
}

fn saved_password(server: &str, username: &str) -> anyhow::Result<String> {
    let profile = config::Profile::new(server, username);
    let in_file = config::Store::user()
        .ok()
        .and_then(|store| store.password(&profile).ok().flatten());
    if let Some(password) = in_file {
        return Ok(password);
    }
    config::Keychain::system()?
        .password(&profile)?
        .with_context(|| {
            format!(
                "no password for {username} on {server} in the keychain; \
                 pass --password (or set HIPHILE_PASSWORD), or sign in in the app"
            )
        })
}

fn main() -> anyhow::Result<()> {
    // The CLI drives runtime-agnostic futures itself; `net` owns the tokio
    // runtime, so this must not run inside another tokio runtime.
    match Cli::parse().command {
        Command::Ping(args) => ping(args),
        Command::Devices => devices(),
        Command::Sync(args) => sync(args),
        Command::Albums {
            server,
            refresh,
            offline,
        } => albums(server, refresh, offline),
        Command::Search {
            server,
            query,
            count,
        } => search(server, &query, count),
        Command::Play {
            server,
            ids,
            search,
        } => play(server, ids, search),
    }
}

fn ping(args: ServerArgs) -> anyhow::Result<()> {
    let client = args.client()?;
    let server = block_on(client.ping())
        .with_context(|| format!("ping {} failed", client.server().base_url()))?;
    let software = match (server.server_type(), server.server_version()) {
        (Some(name), Some(version)) => format!("{name} {version}"),
        (Some(name), None) => name.to_owned(),
        _ => "Subsonic server".to_owned(),
    };
    println!(
        "{software}, protocol {}{}",
        server.version(),
        if server.is_open_subsonic() {
            ", OpenSubsonic"
        } else {
            ""
        }
    );
    Ok(())
}

fn devices() -> anyhow::Result<()> {
    for device in audio::output_devices()? {
        let marker = if device.is_default() { "*" } else { " " };
        let rate = device
            .sample_rate()
            .map(|rate| format!("{rate} Hz"))
            .unwrap_or_else(|| "unknown rate".to_owned());
        println!("{marker} {} ({rate})", device.name());
    }
    Ok(())
}

fn sync(args: ServerArgs) -> anyhow::Result<()> {
    let catalog = args.catalog()?;
    let client = args.client()?;
    let albums = block_on(catalog::sync_albums(
        &catalog,
        &client,
        catalog::Mode::Always,
    ))?;
    report("albums", albums)?;
    let artists = block_on(catalog::sync_artists(
        &catalog,
        &client,
        catalog::Mode::Always,
    ))?;
    report("artists", artists)?;
    let songs = block_on(catalog::sync_songs(
        &catalog,
        &client,
        catalog::Mode::Always,
    ))?;
    report("songs", songs)?;
    println!("saved in {}", catalog.dir().display());
    Ok(())
}

fn report(items: &str, outcome: catalog::Outcome) -> anyhow::Result<()> {
    match outcome {
        catalog::Outcome::Synced { count } => println!("{count} {items}"),
        catalog::Outcome::Unsettled { count } => println!(
            "{count} {items}; the server was scanning, so {items} it removed are kept until the next sync"
        ),
        catalog::Outcome::UpToDate => println!("{items} are up to date"),
        catalog::Outcome::Busy => {
            anyhow::bail!("another hiphile is updating this library; try again when it is done")
        }
    }
    Ok(())
}

fn albums(args: ServerArgs, refresh: bool, offline: bool) -> anyhow::Result<()> {
    let catalog = args.catalog()?;
    if !offline {
        let mode = if refresh {
            catalog::Mode::Always
        } else {
            catalog::Mode::IfStale
        };
        let client = args.client()?;
        // Another process updating the copy leaves it as it is for now
        block_on(catalog::sync_albums(&catalog, &client, mode))?;
    }
    for album in catalog.albums()? {
        let details = match (album.artist(), album.year()) {
            (Some(artist), Some(year)) => format!("{artist} - {} ({year})", album.name()),
            (Some(artist), None) => format!("{artist} - {}", album.name()),
            (None, Some(year)) => format!("{} ({year})", album.name()),
            (None, None) => album.name().to_owned(),
        };
        println!("{}\t{details}", album.id());
    }
    Ok(())
}

fn search(args: ServerArgs, query: &str, count: u32) -> anyhow::Result<()> {
    let client = args.client()?;
    for song in block_on(client.search_songs(query, count))? {
        println!("{}\t{}", song.id(), describe(&song));
    }
    Ok(())
}

fn play(args: ServerArgs, ids: Vec<String>, query: Option<String>) -> anyhow::Result<()> {
    let client = args.client()?;
    let songs: Vec<SongId> = match query {
        Some(query) => block_on(client.search_songs(&query, 50))?
            .into_iter()
            .map(|song| song.id().clone())
            .collect(),
        None => ids.into_iter().map(SongId::new).collect(),
    };
    anyhow::ensure!(!songs.is_empty(), "nothing to play");

    let mut queue = playback::Queue::new();
    queue.replace(songs, 0);
    let (engine, events) = audio::Engine::spawn();
    engine.set_output(output_settings());
    let song_cache = song_cache();
    let mut failures = 0;
    while let Some(song) = queue.current().cloned() {
        if let Err(error) = play_song(&client, song_cache.as_ref(), &engine, &events, &song) {
            eprintln!("{song}: {error:#}");
            failures += 1;
        }
        queue.advance();
    }
    anyhow::ensure!(failures == 0, "{failures} song(s) could not be played");
    Ok(())
}

/// Settings that cannot be read mean the system's default device and each song's own rate.
fn output_settings() -> audio::OutputSettings {
    let saved = || -> anyhow::Result<audio::OutputSettings> {
        let store = config::Store::user()?;
        Ok(audio::OutputSettings::new(
            store.output_device()?,
            store.sample_rate()?,
        ))
    };
    saved().unwrap_or_else(|error| {
        eprintln!("warning: using the default audio output: {error:#}");
        audio::OutputSettings::default()
    })
}

/// Shared with the app. Settings that cannot be read keep none.
fn song_cache() -> Option<playback::SongCache> {
    let capacity = match config::Store::user() {
        Ok(store) => store.song_cache_capacity(),
        // Without a home directory there are no settings to follow
        Err(_) => Ok(None),
    };
    let song_cache = capacity
        .map_err(anyhow::Error::from)
        .and_then(|capacity| Ok(playback::SongCache::user(capacity)?));
    match song_cache {
        Ok(song_cache) => Some(song_cache),
        Err(error) => {
            eprintln!("warning: songs will not be kept on this computer: {error:#}");
            None
        }
    }
}

fn play_song(
    client: &client::Client,
    song_cache: Option<&playback::SongCache>,
    engine: &audio::Engine,
    events: &async_channel::Receiver<audio::Event>,
    id: &SongId,
) -> anyhow::Result<()> {
    let song = block_on(client.song(id))?;
    println!("{}", describe(&song));
    let opened = block_on(playback::open_track(client, song_cache, &song))?;
    let kept = opened.is_kept();
    match play_track(engine, events, &song, opened.into_track())? {
        // The kept file may be damaged: remove it and fetch the song again.
        // One that cannot be removed would fail again, so the song then
        // comes from the server uncached.
        Err(error) if kept && error.is_track_error() => {
            let song_cache = song_cache.filter(|song_cache| song_cache.forget(client, id).is_ok());
            let opened = block_on(playback::open_track(client, song_cache, &song))?;
            Ok(play_track(engine, events, &song, opened.into_track())??)
        }
        played => Ok(played?),
    }
}

/// The inner result is the track's own failure; the outer one, the engine's.
fn play_track(
    engine: &audio::Engine,
    events: &async_channel::Receiver<audio::Event>,
    song: &Song,
    track: audio::Track,
) -> anyhow::Result<Result<(), audio::Error>> {
    let load = engine.load(track);
    let mut duration = song.duration();
    loop {
        match events.try_recv() {
            Ok(event) if event.load != load => {}
            Ok(audio::Event {
                kind: audio::EventKind::Started(format) | audio::EventKind::OutputChanged(format),
                ..
            }) => {
                duration = format.duration().or(duration);
                println!("  {}", describe_format(&format));
            }
            Ok(audio::Event {
                kind: audio::EventKind::Buffering(_),
                ..
            }) => {}
            Ok(audio::Event {
                kind: audio::EventKind::Ended { .. },
                ..
            }) => {
                eprintln!();
                return Ok(Ok(()));
            }
            Ok(audio::Event {
                kind: audio::EventKind::Failed(error),
                ..
            }) => {
                eprintln!();
                return Ok(Err(error));
            }
            Err(async_channel::TryRecvError::Empty) => {
                let total = duration.map(clock).unwrap_or_else(|| "?".to_owned());
                eprint!("\r  {} / {total}", clock(engine.position()));
                std::io::stderr().flush().ok();
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(async_channel::TryRecvError::Closed) => {
                anyhow::bail!("the audio engine stopped")
            }
        }
    }
}

fn describe(song: &Song) -> String {
    match (song.artist(), song.album()) {
        (Some(artist), Some(album)) => format!("{artist} - {} ({album})", song.title()),
        (Some(artist), None) => format!("{artist} - {}", song.title()),
        _ => song.title().to_owned(),
    }
}

fn describe_format(format: &audio::PlaybackFormat) -> String {
    let source = format!("{} Hz {}ch", format.source_rate(), format.source_channels());
    let output = format!(
        "{} @ {} Hz {}ch",
        format.device(),
        format.output_rate(),
        format.output_channels()
    );
    let resampled = if format.is_resampled() {
        " (resampled)"
    } else {
        ""
    };
    format!("{source} -> {output}{resampled}")
}

fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
