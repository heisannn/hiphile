//! What the user should hear of wherever they are, worded in full: a sync or a load that failed,
//! dictionaries that could not be had, songs that were skipped. The core collects them from when
//! it makes what they come from, so a host that follows later still hears what came before.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use executor::{Executor, Task};
use playback::PlayerNotice;
use session::{Readings, ReadingsFailed, ReadingsState};
use tokio::sync::broadcast;

use crate::favorites::{FavoritesLoad, FavoritesNotice, FavoritesState};
use crate::follow;
use crate::library::{LibraryNotice, LibrarySync, SyncStatus};
use crate::navigation::Destination;
use crate::shelf::Shelf;

/// How many alerts are kept for a host that follows later, and how many one that falls behind
/// can miss before it loses the oldest.
const KEPT: usize = 16;

/// What heads a failed sync on the albums' screen, and its alert.
pub const SYNC_FAILED: &str = "Couldn't refresh the library";
/// What heads favorites that failed to load on their screen, and their alert.
pub const FAVORITES_FAILED: &str = "Couldn't load favorites";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Error,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Alert {
    pub kind: AlertKind,
    pub message: String,
    /// The screen that tells of this itself, where a host need not put it up.
    pub told_on: Option<Destination>,
}

/// The alerts told before a host followed them, and those told after.
#[non_exhaustive]
pub struct Alerts {
    /// Oldest first, up to the last 16.
    pub earlier: Vec<Alert>,
    pub later: broadcast::Receiver<Alert>,
}

/// Keeps the last alerts told and sends each one to whoever follows.
#[derive(Clone)]
pub(crate) struct Collector(Arc<Inner>);

struct Inner {
    told: Mutex<VecDeque<Alert>>,
    sender: broadcast::Sender<Alert>,
}

impl Collector {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Inner {
            told: Mutex::new(VecDeque::with_capacity(KEPT)),
            sender: broadcast::Sender::new(KEPT),
        }))
    }

    /// Under the same lock as [`Self::tell`], so that each alert is either earlier or later.
    pub(crate) fn follow(&self) -> Alerts {
        let told = lock(&self.0.told);
        Alerts {
            earlier: told.iter().cloned().collect(),
            later: self.0.sender.subscribe(),
        }
    }

    fn tell(&self, alert: Alert) {
        let mut told = lock(&self.0.told);
        if told.len() == KEPT {
            told.pop_front();
        }
        told.push_back(alert.clone());
        // Fails only when nobody follows
        self.0.sender.send(alert).ok();
    }
}

/// A sync's failure, also one that came before this followed it.
pub(crate) fn follow_library(
    executor: &Executor,
    sync: &LibrarySync,
    alerts: &Collector,
) -> Task<()> {
    let notices = sync.notices();
    if let Some(error) = failed_sync(&sync.status())
        && notices.is_empty()
    {
        alerts.tell(sync_failed(error));
    }
    let alerts = alerts.clone();
    follow::subscribe(executor, notices, move |notice| match notice {
        LibraryNotice::SyncFailed(error) => alerts.tell(sync_failed(&error)),
        LibraryNotice::AlbumsSynced
        | LibraryNotice::ArtistsSynced
        | LibraryNotice::SongsSynced
        | LibraryNotice::SyncFinished
        | LibraryNotice::StageSynced(..) => {}
    })
}

fn failed_sync(status: &SyncStatus) -> Option<&str> {
    match status.albums() {
        Shelf::Failed(error) => Some(error),
        _ => status.failure(),
    }
}

fn sync_failed(error: &str) -> Alert {
    Alert {
        kind: AlertKind::Error,
        message: format!("{SYNC_FAILED}. {error}"),
        told_on: Some(Destination::Albums),
    }
}

/// Favorites that failed to load, also before this followed them.
pub(crate) fn follow_favorites(
    executor: &Executor,
    favorites: &FavoritesState,
    alerts: &Collector,
) -> Task<()> {
    let notices = favorites.notices();
    if let FavoritesLoad::Failed(error) = favorites.snapshot().load()
        && notices.is_empty()
    {
        alerts.tell(favorites_failed(error));
    }
    let alerts = alerts.clone();
    follow::subscribe(executor, notices, move |notice| match notice {
        FavoritesNotice::LoadFailed(error) => alerts.tell(favorites_failed(&error)),
        FavoritesNotice::WriteFailed(_) => {}
    })
}

fn favorites_failed(error: &str) -> Alert {
    Alert {
        kind: AlertKind::Error,
        message: format!("{FAVORITES_FAILED}. {error} Choose Refresh Library to try again."),
        told_on: Some(Destination::Favorites),
    }
}

pub(crate) fn follow_player(
    executor: &Executor,
    player: &playback::Player,
    alerts: &Collector,
) -> Task<()> {
    let alerts = alerts.clone();
    follow::subscribe(executor, player.notices(), move |notice| match notice {
        PlayerNotice::Skipped => alerts.tell(Alert {
            kind: AlertKind::Info,
            message: "Instrumentals are being skipped. Turn off Skip instrumentals in Settings \
                      to play them."
                .to_owned(),
            told_on: None,
        }),
        PlayerNotice::Played => {}
    })
}

/// Dictionaries that could not be had, which only Settings would show, also before this followed
/// them.
pub(crate) fn follow_readings(
    executor: &Executor,
    readings: &Readings,
    alerts: &Collector,
) -> Task<()> {
    let notices = readings.notices();
    if let ReadingsState::Failed(error) = readings.status().state()
        && notices.is_empty()
    {
        alerts.tell(readings_failed(error));
    }
    let alerts = alerts.clone();
    follow::subscribe(executor, notices, move |failed: ReadingsFailed| {
        alerts.tell(readings_failed(failed.error()));
    })
}

fn readings_failed(error: &str) -> Alert {
    Alert {
        kind: AlertKind::Error,
        message: format!("{error} Try again in Settings."),
        told_on: None,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
