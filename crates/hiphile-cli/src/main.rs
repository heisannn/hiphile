mod play;

use std::io;
use std::time::Duration;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
use executor::Executor;
use hiphile_core::library::{LibraryNotice, LibrarySync, Outcome, SyncStage, SyncStatus};
use hiphile_core::{Caches, Hiphile, Shelf};
use subsonic::{Credentials, Song, SongId};
use tokio::sync::broadcast::error::RecvError;

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

/// The app's core on a runtime of this process, as a binding crate would run it. Only what
/// `block_on` waits for runs on it, so nothing the core sends is missed before it is listened to.
struct Core {
    hiphile: Hiphile,
    runtime: tokio::runtime::Runtime,
    /// Dropped last, outside the runtime: the last client must not be dropped inside one.
    _net: net::Client,
}

impl Core {
    /// Only `play` keeps anything, to share the songs the app keeps. That also fetches the
    /// dictionaries while it plays, when the app's search wants them and they are not there yet.
    fn new(caches: Caches) -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let user_agent = concat!("hiphile-cli/", env!("CARGO_PKG_VERSION"));
        let net = net::Client::new(user_agent)?;
        let hiphile = Hiphile::new(
            Executor::tokio(runtime.handle().clone()),
            net.clone(),
            config::Store::user(),
            config::Keychain::system().ok(),
            caches,
        );
        Ok(Self {
            hiphile,
            runtime,
            _net: net,
        })
    }

    fn executor(&self) -> Executor {
        Executor::tokio(self.runtime.handle().clone())
    }

    fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

impl ServerArgs {
    fn address(&self, core: &Core) -> anyhow::Result<String> {
        self.server
            .clone()
            .or_else(|| {
                let profile = core.hiphile.last_profile()?;
                Some(profile.server().to_owned())
            })
            .context("pass --server (or set HIPHILE_SERVER), or sign in once in the app")
    }

    fn catalog(&self, core: &Core) -> anyhow::Result<catalog::Catalog> {
        let url = client::server_url(&self.address(core)?)?;
        Ok(catalog::Catalog::open_server(&url)?)
    }

    /// A session through the core, which remembers nothing of it.
    fn connect(self, core: &Core) -> anyhow::Result<session::Session> {
        let address = self.address(core)?;
        let credentials = match self.api_key {
            Some(key) => Credentials::api_key(key),
            None => {
                let username = self
                    .username
                    .or_else(|| {
                        core.hiphile
                            .last_profile()
                            .filter(|profile| is_on(profile, &address))
                            .map(|profile| profile.username().to_owned())
                    })
                    .context("pass --username (or set HIPHILE_USERNAME)")?;
                let password = match self.password {
                    Some(password) => password,
                    None => saved_password(core, &address, &username)?,
                };
                Credentials::password(username, password)
            }
        };
        let connecting = core
            .hiphile
            .connect(&address, credentials)
            .map_err(|invalid| anyhow::anyhow!("{}", invalid.message()))?;
        core.block_on(connecting).map_err(anyhow::Error::msg)
    }
}

fn is_on(profile: &config::Profile, address: &str) -> bool {
    config::Profile::new(address, profile.username()) == *profile
}

fn saved_password(core: &Core, server: &str, username: &str) -> anyhow::Result<String> {
    let profile = config::Profile::new(server, username);
    core.block_on(core.hiphile.saved_password(profile))
        .with_context(|| {
            format!(
                "no password for {username} on {server} in the keychain; \
                 pass --password (or set HIPHILE_PASSWORD), or sign in in the app"
            )
        })
}

fn main() -> anyhow::Result<()> {
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
    let core = Core::new(Caches::None)?;
    let session = args.connect(&core)?;
    let server = session.server();
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
    let core = Core::new(Caches::None)?;
    let session = args.connect(&core)?;
    let dir = session
        .catalog()
        .context("the library can't be kept on this computer")?
        .dir()
        .to_owned();
    let library = LibrarySync::refreshing(core.executor(), session);
    let mut notices = library.notices();
    core.block_on(async {
        loop {
            match notices.recv().await {
                Ok(LibraryNotice::StageSynced(stage, outcome)) => report(stage, outcome)?,
                Ok(LibraryNotice::SyncFinished) => return Ok(()),
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => anyhow::bail!("the sync stopped"),
            }
        }
    })?;
    failure(&library.status())?;
    println!("saved in {}", dir.display());
    Ok(())
}

fn report(stage: SyncStage, outcome: Outcome) -> anyhow::Result<()> {
    let items = match stage {
        SyncStage::Albums => "albums",
        SyncStage::Artists => "artists",
        SyncStage::Songs => "songs",
    };
    match outcome {
        Outcome::Synced { count } => println!("{count} {items}"),
        Outcome::Unsettled { count } => println!(
            "{count} {items}; the server was scanning, so {items} it removed are kept until the next sync"
        ),
        Outcome::UpToDate => println!("{items} are up to date"),
        Outcome::Busy => {
            anyhow::bail!("another hiphile is updating this library; try again when it is done")
        }
    }
    Ok(())
}

/// Why the sync that ended failed, if it did.
fn failure(status: &SyncStatus) -> anyhow::Result<()> {
    match (status.albums(), status.failure()) {
        (Shelf::Failed(error), _) => anyhow::bail!("{error}"),
        (_, Some(error)) => anyhow::bail!("{error}"),
        _ => Ok(()),
    }
}

fn albums(args: ServerArgs, refresh: bool, offline: bool) -> anyhow::Result<()> {
    let core = Core::new(Caches::None)?;
    let albums = if offline {
        args.catalog(&core)?.albums()?.into()
    } else {
        let session = args.connect(&core)?;
        let library = if refresh {
            LibrarySync::refreshing(core.executor(), session)
        } else {
            LibrarySync::new(core.executor(), session)
        };
        let mut notices = library.notices();
        // The artists and songs that follow are no concern here
        core.block_on(async {
            loop {
                match notices.recv().await {
                    Ok(
                        LibraryNotice::StageSynced(SyncStage::Albums, _)
                        | LibraryNotice::SyncFinished,
                    )
                    | Err(RecvError::Closed) => return,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                }
            }
        });
        let status = library.status();
        failure(&status)?;
        match status.albums() {
            Shelf::Loaded(albums) => albums.clone(),
            Shelf::Loading | Shelf::Failed(_) => anyhow::bail!("the albums never came"),
        }
    };
    for album in albums.iter() {
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

/// Asks the server, which knows songs the saved library may not have yet.
fn search(args: ServerArgs, query: &str, count: u32) -> anyhow::Result<()> {
    let core = Core::new(Caches::None)?;
    let session = args.connect(&core)?;
    for song in core.block_on(session.client().search_songs(query, count))? {
        println!("{}\t{}", song.id(), describe(&song));
    }
    Ok(())
}

fn play(args: ServerArgs, ids: Vec<String>, query: Option<String>) -> anyhow::Result<()> {
    let core = Core::new(Caches::User)?;
    let session = args.connect(&core)?;
    let client = session.client();
    let mut failures = 0;
    let songs: Vec<Song> = match query {
        Some(query) => core.block_on(client.search_songs(&query, 50))?,
        None => ids
            .into_iter()
            .map(SongId::new)
            .filter_map(|id| match core.block_on(client.song(&id)) {
                Ok(song) => Some(song),
                Err(error) => {
                    eprintln!("{id}: {}", session::describe(&error));
                    failures += 1;
                    None
                }
            })
            .collect(),
    };
    anyhow::ensure!(!songs.is_empty() || failures > 0, "nothing to play");

    let settings = core.hiphile.settings();
    let audio = settings.audio().snapshot();
    let dsp = settings.dsp().snapshot();
    let song_limit = settings.song_cache().snapshot();
    for error in audio.errors().chain(dsp.errors()).chain(song_limit.error()) {
        eprintln!("warning: {error}");
    }
    if !songs.is_empty() {
        let player = core.hiphile.player(session);
        // Nothing pauses here, and songs follow one another without a crossfade
        player.set_transitions(audio::Transitions {
            crossfade: Duration::ZERO,
            fades_play_and_pause: false,
            ..audio.transitions()
        });
        let played = core.block_on(play::play_through(
            &player,
            &core.executor(),
            &songs,
            &mut io::stdout(),
            &mut io::stderr(),
        ));
        player.shutdown();
        failures += played?;
    }
    anyhow::ensure!(failures == 0, "{failures} song(s) could not be played");
    Ok(())
}

fn describe(song: &Song) -> String {
    match (song.artist(), song.album()) {
        (Some(artist), Some(album)) => format!("{artist} - {} ({album})", song.title()),
        (Some(artist), None) => format!("{artist} - {}", song.title()),
        _ => song.title().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_profile_is_on_its_server_however_the_address_is_written() {
        let alice = config::Profile::new("https://music.example.com", "alice");
        assert!(is_on(&alice, "https://music.example.com"));
        assert!(is_on(&alice, "HTTPS://Music.Example.com/"));
        assert!(!is_on(&alice, "https://other.example.com"));
    }
}
