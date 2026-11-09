use std::io::{self, Write};
use std::pin::pin;
use std::time::Duration;

use audio::PlaybackFormat;
use executor::Executor;
use futures::future;
use playback::{EntryId, Player, PlayerSnapshot};
use subsonic::Song;

/// How often the position is shown while nothing else changes.
const TICK: Duration = Duration::from_millis(250);

/// Plays `songs` in order on `player`, going on past those that fail. Each song and what it plays
/// as go to `out`; how far it is and why a song failed go to `progress`. Returns how many failed.
pub(crate) async fn play_through(
    player: &Player,
    executor: &Executor,
    songs: &[Song],
    out: &mut impl Write,
    progress: &mut impl Write,
) -> io::Result<usize> {
    let mut changes = player.subscribe();
    // A player that skips nothing always starts
    player.play(songs, 0);
    let mut report = Report::default();
    loop {
        let snapshot = changes.borrow_and_update().clone();
        match report.look(&snapshot, player.position(), out, progress)? {
            Step::Wait => {
                future::select(pin!(changes.changed()), executor.timer(TICK)).await;
            }
            Step::Next => player.next(),
            Step::Done => return Ok(report.failures),
        }
    }
}

/// What has been said of the playing so far.
#[derive(Default)]
struct Report {
    song: Option<EntryId>,
    format: Option<PlaybackFormat>,
    /// Whether the position is shown on a line not ended yet.
    is_line_open: bool,
    failures: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Wait,
    /// The current song failed.
    Next,
    /// The queue has ended.
    Done,
}

impl Report {
    fn look(
        &mut self,
        snapshot: &PlayerSnapshot,
        position: Duration,
        out: &mut impl Write,
        progress: &mut impl Write,
    ) -> io::Result<Step> {
        let current = snapshot.queue().current_id().zip(snapshot.current_song());
        let Some((entry, song)) = current else {
            self.end_line(progress)?;
            return Ok(Step::Done);
        };
        if self.song != Some(entry) {
            self.end_line(progress)?;
            self.song = Some(entry);
            self.format = None;
            writeln!(out, "{}", crate::describe(song))?;
        }
        if let Some(error) = snapshot.error().filter(|_| snapshot.is_stopped()) {
            self.end_line(progress)?;
            writeln!(progress, "{}: {error}", song.id())?;
            self.failures += 1;
            return Ok(Step::Next);
        }
        if let Some(format) = snapshot.format()
            && self.format.as_ref() != Some(format)
        {
            self.format = Some(format.clone());
            writeln!(out, "  {}", describe_format(format))?;
        }
        let total = snapshot.duration().map_or_else(|| "?".to_owned(), clock);
        write!(progress, "\r  {} / {total}", clock(position))?;
        progress.flush()?;
        self.is_line_open = true;
        Ok(Step::Wait)
    }

    fn end_line(&mut self, progress: &mut impl Write) -> io::Result<()> {
        if self.is_line_open {
            self.is_line_open = false;
            writeln!(progress)?;
        }
        Ok(())
    }
}

fn describe_format(format: &PlaybackFormat) -> String {
    let source = format!("{} Hz {}ch", format.source_rate(), format.source_channels());
    let output = format!(
        "{} @ {} Hz {}ch",
        format.device(),
        format.output_rate(),
        format.output_channels()
    );
    let conversion = match format.dop_rate() {
        Some(rate) => format!(" ({rate} DoP)"),
        None if format.is_resampled() => " (resampled)".to_owned(),
        None => String::new(),
    };
    format!("{source} -> {output}{conversion}")
}

fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests;
