//! Play queue and playback helpers shared by the GUI and the CLI.

mod queue;

pub use queue::{Queue, RepeatMode};

use subsonic::Song;

/// Opens `song` for the audio engine: starts its download and wires the
/// download's cancellation into the track, so replacing or stopping the
/// track never waits on the network.
pub async fn open_track(
    client: &client::Client,
    song: &Song,
) -> Result<audio::Track, client::Error> {
    let download = client.open_stream(song.id()).await?;
    let content_type = download.content_type().map(str::to_owned);
    let handle = download.handle();
    // When the end of the file is a range request away, letting the decoder
    // know the length costs little and lets it seek MP3 and Ogg by position
    // instead of reading every page before the target.
    let len = download
        .content_length()
        .filter(|_| download.is_random_access());
    let track = match len {
        Some(len) => audio::Track::with_known_len(download, len),
        None => audio::Track::new(download),
    };
    let mut track = track.cancel_with(move || handle.cancel());
    if let Some(content_type) = &content_type {
        track = track.mime_type(content_type);
    }
    if let Some(suffix) = song.suffix() {
        track = track.extension(suffix);
    }
    Ok(track)
}
