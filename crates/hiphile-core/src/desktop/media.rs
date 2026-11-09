use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use executor::{Executor, Task};
use media_controls::{CoverFile, MediaCommand, MediaControls, MediaStatus};
use playback::{EntryId, PlayerSnapshot};
use subsonic::{CoverArtId, Song};
use tokio::sync::broadcast;

use super::{COVER_PIXELS, REFRESH};
use crate::{covers, follow};

const NOTICES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaNotice {
    /// The system asks for the app's window to come to the front.
    Raise,
}

/// Shows what `player` plays in the system's media controls, and passes their commands on to it.
/// Dropping the last handle clears them.
#[derive(Clone)]
pub struct SystemMediaControls(Arc<Inner>);

pub(crate) struct WeakControls(Weak<Inner>);

struct Inner {
    executor: Executor,
    player: playback::Player,
    session: session::Session,
    covers: Option<catalog::CoverStore>,
    state: Mutex<State>,
    notices: broadcast::Sender<MediaNotice>,
}

struct State {
    controls: MediaControls,
    /// The cover being looked for, or last looked for without success, which is then not looked
    /// for again until the next song.
    sought_cover: Option<CoverArtId>,
    /// The current entry, as the last snapshot showed it.
    followed: Option<EntryId>,
    _cover: Task<()>,
    _changes: Task<()>,
    _commands: Task<()>,
    /// The system learns of a seek, and of a position it could not have worked out, only from
    /// this.
    _refresh: Task<()>,
}

impl SystemMediaControls {
    /// Takes the controls and their commands as [`MediaControls::connect`] gives them for the
    /// app's window. Covers come from `covers` while they keep them.
    pub fn new(
        executor: Executor,
        player: playback::Player,
        session: session::Session,
        covers: Option<catalog::CoverStore>,
        (controls, commands): (MediaControls, async_channel::Receiver<MediaCommand>),
    ) -> Self {
        let this = Self(Arc::new(Inner {
            executor,
            player,
            session,
            covers,
            state: Mutex::new(State {
                controls,
                sought_cover: None,
                followed: None,
                _cover: Task::ready(()),
                _changes: Task::ready(()),
                _commands: Task::ready(()),
                _refresh: Task::ready(()),
            }),
            notices: broadcast::Sender::new(NOTICES),
        }));
        let changes = follow::observe(&this.0.executor, this.0.player.subscribe(), {
            let weak = this.downgrade();
            move |shown: PlayerSnapshot| {
                if let Some(this) = weak.upgrade() {
                    this.show(&shown);
                }
            }
        });
        let weak = this.downgrade();
        let commands = this.0.executor.spawn(async move {
            while let Ok(command) = commands.recv().await {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                this.on_command(command);
            }
        });
        let weak = this.downgrade();
        let executor = this.0.executor.clone();
        let refresh = this.0.executor.spawn(async move {
            loop {
                executor.timer(REFRESH).await;
                let Some(this) = weak.upgrade() else {
                    return;
                };
                this.show(&this.0.player.snapshot());
            }
        });
        let mut state = this.lock();
        state._changes = changes;
        state._commands = commands;
        state._refresh = refresh;
        drop(state);
        this
    }

    /// Hears each notice from now on.
    pub fn notices(&self) -> broadcast::Receiver<MediaNotice> {
        self.0.notices.subscribe()
    }

    /// Shows nothing and removes the cover's file. For when the app quits, which may not drop
    /// this.
    pub fn clear(&self) {
        self.lock().controls.clear();
    }

    fn show(&self, shown: &PlayerSnapshot) {
        let mut state = self.lock();
        let entry = shown.queue().current_id();
        if std::mem::replace(&mut state.followed, entry) != entry {
            state.sought_cover = None;
        }
        let position = self.0.player.position();
        state.controls.show(
            shown.current_song(),
            shown.duration(),
            media_status(shown),
            position,
        );
        self.seek_cover(&mut state, shown);
    }

    /// Finds the current song's cover, calling off the search for the song before. The controls
    /// show the song without it if it cannot be had, so a failure is not shown.
    fn seek_cover(&self, state: &mut State, shown: &PlayerSnapshot) {
        if !state.controls.is_connected() {
            return;
        }
        let Some(cover) = shown
            .current_song()
            .and_then(Song::cover_art)
            .filter(|cover| {
                state.sought_cover.as_ref() != Some(*cover) && !state.controls.has_cover(cover)
            })
            .cloned()
        else {
            return;
        };
        state.sought_cover = Some(cover.clone());
        let image = covers::cover_image(
            &self.0.executor,
            self.0.session.client().net().clone(),
            self.0.covers.clone(),
            self.0.session.cover_art_url(&cover, COVER_PIXELS),
        );
        let executor = self.0.executor.clone();
        let weak = self.downgrade();
        state._cover = self.0.executor.spawn(async move {
            let Some(image) = image.await else {
                return;
            };
            // The file is on the disk
            let file = executor.spawn_blocking(move || CoverFile::write(&image).ok());
            if let (Some(file), Some(this)) = (file.await, weak.upgrade()) {
                this.lock().controls.set_cover(cover, file);
            }
        });
    }

    fn on_command(&self, command: MediaCommand) {
        let player = &self.0.player;
        match command {
            MediaCommand::Play => player.set_playing(true),
            MediaCommand::Pause => player.set_playing(false),
            MediaCommand::Toggle => player.toggle_playback(),
            MediaCommand::Stop => player.stop(),
            MediaCommand::Next => player.next(),
            MediaCommand::Previous => player.previous(),
            MediaCommand::SeekForward(by) => {
                by.map_or_else(|| player.seek_forward(), |by| player.seek_forward_by(by));
            }
            MediaCommand::SeekBackward(by) => {
                by.map_or_else(|| player.seek_backward(), |by| player.seek_backward_by(by));
            }
            MediaCommand::SeekTo(target) => {
                player.seek_to(target);
            }
            MediaCommand::Raise => {
                self.0.notices.send(MediaNotice::Raise).ok();
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn downgrade(&self) -> WeakControls {
        WeakControls(Arc::downgrade(&self.0))
    }
}

impl WeakControls {
    pub(crate) fn upgrade(&self) -> Option<SystemMediaControls> {
        self.0.upgrade().map(SystemMediaControls)
    }
}

fn media_status(shown: &PlayerSnapshot) -> MediaStatus {
    if shown.is_stopped() {
        MediaStatus::Stopped
    } else if shown.is_playing() {
        MediaStatus::Playing
    } else {
        MediaStatus::Paused
    }
}

#[cfg(test)]
mod tests;
