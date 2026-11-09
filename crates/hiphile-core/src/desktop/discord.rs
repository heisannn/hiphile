use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use executor::{Executor, Task};
use futures::executor::block_on;
use playback::PlayerSnapshot;
use presence::{CoverHost, Presence};
use subsonic::{CoverArtId, Song};
use tokio::sync::broadcast;

use super::{COVER_PIXELS, REFRESH};
use crate::{covers, follow};

const NOTICES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscordNotice {
    /// A cover was looked for to put on the status, and why it could not be published, if it
    /// could not.
    CoverSought { error: Option<String> },
}

/// Shows what `player` plays as the user's Discord status, with its cover uploaded to ImgBB.
/// Dropping the last handle takes the status away.
#[derive(Clone)]
pub struct DiscordStatus(Arc<Inner>);

struct WeakStatus(Weak<Inner>);

struct Inner {
    executor: Executor,
    player: playback::Player,
    session: session::Session,
    covers: Option<catalog::CoverStore>,
    cover_host: Arc<CoverHost>,
    state: Mutex<State>,
    notices: broadcast::Sender<DiscordNotice>,
}

struct State {
    presence: Presence,
    /// Without an API key, no cover goes on the status.
    cover_key: Option<Arc<str>>,
    /// The cover being looked for, or last looked for without success, which
    /// is then not looked for again until another song has had its turn.
    sought_cover: Option<CoverArtId>,
    _changes: Task<()>,
    /// While a song plays, so that a seek reaches the status.
    _refresh: Option<Task<()>>,
}

impl DiscordStatus {
    /// Shows the status through `presence`, which tests give disconnected.
    pub fn new(
        executor: Executor,
        player: playback::Player,
        session: session::Session,
        covers: Option<catalog::CoverStore>,
        presence: Presence,
    ) -> Self {
        let cover_host = Arc::new(CoverHost::imgbb(session.client().net().clone()));
        let this = Self(Arc::new(Inner {
            executor,
            player,
            session,
            covers,
            cover_host,
            state: Mutex::new(State {
                presence,
                cover_key: None,
                sought_cover: None,
                _changes: Task::ready(()),
                _refresh: None,
            }),
            notices: broadcast::Sender::new(NOTICES),
        }));
        let weak = this.downgrade();
        let changes = follow::observe(
            &this.0.executor,
            this.0.player.subscribe(),
            move |shown: PlayerSnapshot| {
                if let Some(this) = weak.upgrade() {
                    this.show(&shown);
                }
            },
        );
        this.lock()._changes = changes;
        this
    }

    /// Hears each notice from now on.
    pub fn notices(&self) -> broadcast::Receiver<DiscordNotice> {
        self.0.notices.subscribe()
    }

    /// Publishes covers with this ImgBB API key from now on, or none without
    /// one, trying again a cover that the last key could not publish.
    pub fn set_cover_key(&self, key: Option<&str>) {
        let mut state = self.lock();
        state.cover_key = key.map(Arc::from);
        state.sought_cover = None;
    }

    /// Shows what is playing on the status from now on, or takes it away and publishes no more
    /// covers. Turning it on tries again a cover that could not be published before.
    pub fn set_on(&self, is_on: bool) {
        let mut state = self.lock();
        state.presence.set_on(is_on);
        state.sought_cover = None;
    }

    /// A song opening keeps the status of the one before, so that skipping does not make
    /// it flicker.
    fn show(&self, shown: &PlayerSnapshot) {
        let mut state = self.lock();
        let is_playing = shown.is_started() && shown.is_playing();
        if is_playing {
            if let Some(song) = shown.current_song() {
                let position = self.0.player.position();
                state.presence.listening(song, position, shown.duration());
            }
            self.seek_cover(&mut state, shown);
        } else if shown.is_started() || shown.is_stopped() {
            state.presence.clear();
        }
        self.keep_refreshing(&mut state, is_playing);
    }

    fn keep_refreshing(&self, state: &mut State, is_playing: bool) {
        if !is_playing {
            state._refresh = None;
            return;
        }
        if state._refresh.is_some() {
            return;
        }
        let executor = self.0.executor.clone();
        let weak = self.downgrade();
        state._refresh = Some(self.0.executor.spawn(async move {
            loop {
                executor.timer(REFRESH).await;
                let Some(this) = weak.upgrade() else {
                    return;
                };
                this.show(&this.0.player.snapshot());
            }
        }));
    }

    /// Finds the current song's cover a public URL for the status. Only the status depends on
    /// it, so a failure is not shown. It is not called off when the song changes: an upload cut
    /// short leaves a file up with no record of it, and the next play would upload another. It
    /// is also how a link that has run out gets replaced, for as long as the app runs.
    fn seek_cover(&self, state: &mut State, shown: &PlayerSnapshot) {
        if !state.presence.is_on() {
            return;
        }
        let Some(key) = state.cover_key.clone() else {
            return;
        };
        let Some(cover) = shown
            .current_song()
            .and_then(Song::cover_art)
            .filter(|cover| {
                state.sought_cover.as_ref() != Some(*cover) && !state.presence.has_cover(cover)
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
        let host = self.0.cover_host.clone();
        let executor = self.0.executor.clone();
        let weak = self.downgrade();
        self.0
            .executor
            .spawn(async move {
                // Without the image, it is the server that failed, which the rest
                // of the app shows.
                let Some(image) = image.await else {
                    return;
                };
                // The record of links is on the disk
                let link = executor
                    .spawn_blocking(move || block_on(host.link(&image, &key)))
                    .await;
                let Some(this) = weak.upgrade() else {
                    return;
                };
                let error = match link {
                    Ok(link) => {
                        let mut state = this.lock();
                        if state.sought_cover.as_ref() == Some(&cover) {
                            state.sought_cover = None;
                        }
                        state.presence.set_cover(cover, link);
                        None
                    }
                    Err(error) => Some(error.to_string()),
                };
                this.0
                    .notices
                    .send(DiscordNotice::CoverSought { error })
                    .ok();
            })
            .detach();
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn downgrade(&self) -> WeakStatus {
        WeakStatus(Arc::downgrade(&self.0))
    }
}

impl WeakStatus {
    fn upgrade(&self) -> Option<DiscordStatus> {
        self.0.upgrade().map(DiscordStatus)
    }
}

#[cfg(test)]
mod tests;
