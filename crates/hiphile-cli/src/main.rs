//! Command-line client built on the GPUI-independent crates.

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

#[derive(Args)]
struct ServerArgs {
    /// Server address, e.g. https://music.example.com
    #[arg(long, env = "HIPHILE_SERVER")]
    server: String,
    #[arg(long, env = "HIPHILE_USERNAME", required_unless_present = "api_key")]
    username: Option<String>,
    #[arg(
        long,
        env = "HIPHILE_PASSWORD",
        hide_env_values = true,
        requires = "username"
    )]
    password: Option<String>,
    /// OpenSubsonic API key, used instead of username and password.
    #[arg(
        long,
        env = "HIPHILE_API_KEY",
        hide_env_values = true,
        conflicts_with = "username"
    )]
    api_key: Option<String>,
}

impl ServerArgs {
    fn client(self) -> anyhow::Result<client::Client> {
        let credentials = match (self.api_key, self.username, self.password) {
            (Some(key), _, _) => Credentials::api_key(key),
            (None, Some(username), Some(password)) => Credentials::password(username, password),
            _ => anyhow::bail!("pass --password (or HIPHILE_PASSWORD) together with --username"),
        };
        let server = client::Server::new(&self.server, credentials)?;
        let user_agent = concat!("hiphile-cli/", env!("CARGO_PKG_VERSION"));
        Ok(client::Client::new(net::Client::new(user_agent)?, server))
    }
}

fn main() -> anyhow::Result<()> {
    // The CLI drives runtime-agnostic futures itself; `net` owns the tokio
    // runtime, so this must not run inside another tokio runtime.
    match Cli::parse().command {
        Command::Ping(args) => ping(args),
        Command::Devices => devices(),
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
    let mut failures = 0;
    while let Some(song) = queue.current().cloned() {
        if let Err(error) = play_song(&client, &engine, &events, &song) {
            eprintln!("{song}: {error:#}");
            failures += 1;
        }
        queue.advance();
    }
    anyhow::ensure!(failures == 0, "{failures} song(s) could not be played");
    Ok(())
}

fn play_song(
    client: &client::Client,
    engine: &audio::Engine,
    events: &async_channel::Receiver<audio::Event>,
    id: &SongId,
) -> anyhow::Result<()> {
    let song = block_on(client.song(id))?;
    let track = block_on(playback::open_track(client, &song))?;
    let load = engine.load(track);

    println!("{}", describe(&song));
    let mut duration = song.duration();
    loop {
        match events.try_recv() {
            Ok(event) if event.load != load => {}
            Ok(audio::Event {
                kind: audio::EventKind::Started(format),
                ..
            }) => {
                duration = format.duration().or(duration);
                println!("  {}", describe_format(&format));
            }
            Ok(audio::Event {
                kind: audio::EventKind::Ended,
                ..
            }) => {
                eprintln!();
                return Ok(());
            }
            Ok(audio::Event {
                kind: audio::EventKind::Failed(error),
                ..
            }) => {
                eprintln!();
                return Err(error.into());
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
