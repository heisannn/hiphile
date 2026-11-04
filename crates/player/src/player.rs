use std::sync::Arc;
use std::time::{Duration, SystemTime};

use audio::{Engine, Event, EventKind, LoadId, OutputSettings, PlaybackFormat};
use futures::AsyncReadExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    slider::{Slider, SliderEvent, SliderState},
    v_flex,
};
use gpui_kit::http_client::AsyncBody;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use playback::{Arrival, EntryId, Placement, PlayTally, Queue};
use presence::{CoverHost, Presence};
use subsonic::{CoverArtId, Song, SongId};
use ui::{AlbumLink, ArtistLink, CoverArt, format_duration, song_artist_link};

use crate::format;
use crate::{GlobalSongCache, NextSong, PreviousSong, ToggleMute, TogglePlayback};

/// How often the position redraws while playing.
const POSITION_REFRESH: Duration = Duration::from_millis(250);
/// Within this much of a song's start, Previous goes to the previous song;
/// later, it restarts the current one.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
/// How far Seek Forward and Seek Backward jump.
const SEEK_STEP: Duration = Duration::from_secs(10);
/// How much Volume Up and Volume Down change the volume.
const VOLUME_STEP: f32 = 0.1;
/// Pixel size requested for the bar's cover: twice its on-screen size.
const COVER_PIXELS: u32 = 96;
/// Pixel size of the cover published for the Discord status, the same as the
/// library's, so that it is often on disk already.
const PRESENCE_COVER_PIXELS: u32 = 512;

pub enum PlayerEvent {
    /// What the player is on, for views that mark it.
    NowPlaying {
        song: Option<SongId>,
        /// Whether it is playing or about to, rather than paused or stopped.
        playing: bool,
    },
    /// Entries were added, removed, or moved. A change of the current entry
    /// alone comes as [`PlayerEvent::NowPlaying`].
    QueueChanged,
    /// A play of a song has been reported to the server, as far as it could
    /// be, so what the server says was played last may have changed.
    Played,
    /// A cover was looked for to put on the Discord status, and why it could
    /// not be published, if it could not.
    CoverSought { error: Option<SharedString> },
}

/// The play queue, the audio engine, and the now-playing bar that controls
/// them. Dropping it, or quitting the app, stops playback and returns the
/// output device to its sample rate from before.
pub struct Player {
    session: session::Session,
    store: Option<config::Store>,
    engine: Engine,
    /// The engine load of the current song, once it has one. Events about
    /// any other load are stale.
    load: Option<LoadId>,
    /// Whether that load plays the song's kept file, which a failure may
    /// show to be damaged.
    loaded_kept: bool,
    /// Whether that load was opened ahead of its turn and has yet to start,
    /// so a failure may be out of date by now.
    loaded_ahead: bool,
    /// The current song's download, which the next one waits for.
    arrival: Option<Arrival>,
    /// The song that follows the current one, being made ready to go on to.
    following: Option<Following>,
    queue: Queue<Song>,
    state: State,
    format: Option<PlaybackFormat>,
    /// Where the stopped song picks back up, after its output failed.
    resume: Option<Resume>,
    /// How much of this play of the current song has been listened to.
    tally: PlayTally,
    presence: Presence,
    cover_host: Arc<CoverHost>,
    /// Without an API key, no cover goes on the Discord status.
    cover_key: Option<Arc<str>>,
    /// The cover being looked for, or last looked for without success, which
    /// is then not looked for again until another song has had its turn.
    sought_cover: Option<CoverArtId>,
    cover_url: Option<SharedString>,
    error: Option<SharedString>,
    /// The position as a fraction of the song.
    position: Entity<SliderState>,
    /// While the position is being dragged, the fraction under the pointer.
    scrub: Option<f32>,
    /// The position at the last refresh, to redraw when it moves while
    /// paused, as after a seek.
    refreshed_position: Duration,
    /// From `0.0` to `1.0`, as the slider shows it.
    volume: Entity<SliderState>,
    muted: bool,
    _open: Task<()>,
    _now_playing: Task<()>,
    _events: Task<()>,
    _refresh: Task<()>,
    /// The volume to save once the save in progress is done.
    unsaved_volume: Option<f32>,
    saving_volume: bool,
    _save_volume: Task<()>,
    _subscriptions: Vec<Subscription>,
}

/// The queue's next entry, whose download and decoder are started ahead of
/// its turn. Dropping it stops getting it ready; a load the engine already has
/// is left for the owner to take back.
struct Following {
    entry: EntryId,
    /// Once it is open and set to follow in the engine. Stays `None` if that
    /// fails: its turn comes the usual way, and shows why then.
    load: Option<LoadId>,
    kept: bool,
    arrival: Option<Arrival>,
    _open: Task<()>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Resume {
    position: Duration,
    paused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Stopped,
    /// The current song's download is starting or its decoder is opening.
    Opening,
    Playing,
    Paused,
}

impl EventEmitter<PlayerEvent> for Player {}

impl Player {
    pub fn new(
        session: session::Session,
        output: OutputSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (engine, events) = Engine::spawn();
        engine.set_output(output);
        let events = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.on_engine_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let refresh = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POSITION_REFRESH).await;
                if this
                    .update_in(cx, |this, window, cx| this.refresh(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });

        // Plays from before that the server has yet to hear of.
        if let Some(scrobbles) = session.scrobbles().cloned() {
            let client = session.client().clone();
            let sent = cx.background_spawn(async move { scrobbles.submit(&client).await });
            cx.spawn(async move |this, cx| {
                if matches!(sent.await, Ok(1..)) {
                    this.update(cx, |_, cx| cx.emit(PlayerEvent::Played)).ok();
                }
            })
            .detach();
        }

        let store = config::Store::user().ok();
        let level = store
            .as_ref()
            .and_then(|store| store.volume().ok().flatten())
            .unwrap_or(1.0);
        engine.set_volume(gain(level, false));
        let position = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.01)
                .default_value(level)
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &position,
                window,
                |this, _, event, window, cx| match event {
                    SliderEvent::Change(value) => {
                        this.scrub = Some(value.end());
                        cx.notify();
                    }
                    SliderEvent::Release(value) => {
                        this.scrub = None;
                        if let Some(duration) = this.duration() {
                            this.seek_to(duration.mul_f32(value.end()), window, cx);
                        }
                    }
                },
            ),
            cx.subscribe(&volume, |this, _, event, cx| match event {
                SliderEvent::Change(value) => this.apply_volume(value.end(), cx),
                SliderEvent::Release(value) => this.save_volume(value.end(), cx),
            }),
            // Quitting may not drop entities, and dropping the engine does not
            // wait for it, so shut it down here, while the process still runs.
            cx.on_app_quit(|this, _| {
                this.engine.shutdown();
                async {}
            }),
        ];
        let cover_host = Arc::new(CoverHost::imgbb(session.client().net().clone()));
        Self {
            session,
            store,
            engine,
            load: None,
            loaded_kept: false,
            loaded_ahead: false,
            arrival: None,
            following: None,
            queue: Queue::new(),
            state: State::Stopped,
            format: None,
            resume: None,
            tally: PlayTally::default(),
            // Tests play made-up songs, which must not reach the user's status.
            presence: if cfg!(test) {
                Presence::disconnected()
            } else {
                Presence::spawn()
            },
            cover_host,
            cover_key: None,
            sought_cover: None,
            cover_url: None,
            error: None,
            position,
            scrub: None,
            refreshed_position: Duration::ZERO,
            volume,
            muted: false,
            _open: Task::ready(()),
            _now_playing: Task::ready(()),
            _events: events,
            _refresh: refresh,
            unsaved_volume: None,
            saving_volume: false,
            _save_volume: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    /// Plays on `output` from now on, moving a song already playing there and
    /// picking up one that the last output stopped.
    pub fn set_output(&mut self, output: OutputSettings, cx: &mut Context<Self>) {
        self.engine.set_output(output);
        if self.state == State::Stopped && self.resume.is_some() {
            self.open_current(false, cx);
        }
    }

    /// Publishes covers with this ImgBB API key from now on, or none without
    /// one, trying again a cover that the last key could not publish.
    pub fn set_cover_key(&mut self, key: Option<&str>) {
        self.cover_key = key.map(Arc::from);
        self.sought_cover = None;
    }

    /// Whether there is a song to play, pause, or skip.
    pub fn has_song(&self) -> bool {
        self.queue.current().is_some()
    }

    /// Whether the queue holds any song, even with playback past its end.
    pub fn has_queued_songs(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Every song in the queue, in order.
    pub fn queued_songs(&self) -> Arc<[Song]> {
        self.queue
            .entries()
            .iter()
            .map(|entry| entry.item().clone())
            .collect()
    }

    /// Replaces the queue with `songs` and plays from `start_ix`.
    pub fn play(&mut self, songs: &[Song], start_ix: usize, cx: &mut Context<Self>) {
        self.queue.replace(songs.to_vec(), start_ix);
        self.queue_changed(cx);
        self.play_current(cx);
    }

    /// Adds `songs` to the queue at `placement`. When nothing is playing or
    /// paused, playback starts: from the first of them if there was no
    /// current song, otherwise from the entry after the current one, so
    /// that songs already waiting are not skipped. The current one is
    /// stopped only after it failed.
    pub fn enqueue(&mut self, songs: &[Song], placement: Placement, cx: &mut Context<Self>) {
        let had_current = self.has_song();
        if self.queue.insert(songs.to_vec(), placement).is_none() {
            return;
        }
        self.queue_changed(cx);
        if self.state != State::Stopped {
            return;
        }
        // Without a current song, the first one added has become it.
        if !had_current || self.queue.advance().is_some() {
            self.play_current(cx);
        }
    }

    pub(crate) fn queue(&self) -> &Queue<Song> {
        &self.queue
    }

    /// Whether the current song is playing or about to, rather than paused
    /// or stopped.
    pub(crate) fn is_playing(&self) -> bool {
        matches!(self.state, State::Playing | State::Opening)
    }

    /// Plays the entry from its start.
    pub(crate) fn play_entry(&mut self, id: EntryId, cx: &mut Context<Self>) {
        if self.queue.jump_to(id).is_some() {
            self.play_current(cx);
        }
    }

    /// Removes the entry, unless it is the current one.
    pub(crate) fn remove_entry(&mut self, id: EntryId, cx: &mut Context<Self>) {
        if self.queue.remove(id) {
            self.queue_changed(cx);
        }
    }

    pub(crate) fn move_entry(&mut self, id: EntryId, to_ix: usize, cx: &mut Context<Self>) {
        if self.queue.move_entry(id, to_ix) {
            self.queue_changed(cx);
        }
    }

    /// Moves the entry to right after the current one.
    pub(crate) fn move_entry_to_next(&mut self, id: EntryId, cx: &mut Context<Self>) {
        if self.queue.move_to_next(id) {
            self.queue_changed(cx);
        }
    }

    /// Removes every entry but the current one.
    pub(crate) fn clear_queue(&mut self, cx: &mut Context<Self>) {
        self.queue.clear();
        self.queue_changed(cx);
    }

    fn queue_changed(&mut self, cx: &mut Context<Self>) {
        self.prepare_following(cx);
        cx.emit(PlayerEvent::QueueChanged);
        cx.notify();
    }

    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        match self.state {
            State::Playing => {
                self.engine.pause();
                self.set_state(State::Paused, cx);
            }
            State::Paused => {
                self.engine.resume();
                self.set_state(State::Playing, cx);
                self.announce_current(cx);
            }
            State::Stopped if self.resume.is_some() => self.open_current(false, cx),
            State::Stopped if self.has_song() => self.play_current(cx),
            State::Stopped | State::Opening => {}
        }
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        if !self.has_song() {
            return;
        }
        if self.queue.advance().is_some() {
            self.play_current(cx);
        } else {
            self.stop(cx);
        }
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        if !self.has_song() {
            return;
        }
        let restart = self.is_started() && self.engine.position() > RESTART_THRESHOLD;
        if !restart {
            self.queue.retreat();
        }
        self.play_current(cx);
    }

    pub fn seek_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.engine.position().saturating_add(SEEK_STEP);
        self.seek_to(target, window, cx);
    }

    pub fn seek_backward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.engine.position().saturating_sub(SEEK_STEP);
        self.seek_to(target, window, cx);
    }

    pub fn volume_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_volume(VOLUME_STEP, window, cx);
    }

    pub fn volume_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_volume(-VOLUME_STEP, window, cx);
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.muted = !self.muted;
        let level = self.level(cx);
        self.engine.set_volume(gain(level, self.muted));
        cx.notify();
    }

    fn is_started(&self) -> bool {
        matches!(self.state, State::Playing | State::Paused)
    }

    /// The position to show: where the song is, or where it would pick up.
    fn position_now(&self) -> Duration {
        if self.is_started() {
            self.engine.position()
        } else {
            self.resume.unwrap_or_default().position
        }
    }

    fn duration(&self) -> Option<Duration> {
        self.format
            .as_ref()
            .and_then(PlaybackFormat::duration)
            .or_else(|| self.queue.current().and_then(Song::duration))
            .filter(|duration| !duration.is_zero())
    }

    /// Jumps to `target` in the current song; at or past its end, moves on
    /// to the next one.
    fn seek_to(&mut self, target: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let Some(duration) = self.duration().filter(|_| self.is_started()) else {
            return;
        };
        if target >= duration {
            self.next(cx);
            return;
        }
        self.engine.seek(target);
        self.show_position(target, window, cx);
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let position = self.position_now();
        if self.state == State::Playing && self.tally.observe(position, self.duration()) {
            self.scrobble_current(cx);
        }
        self.show_presence(cx);
        if self.scrub.is_none() {
            self.show_position(position, window, cx);
        }
        if position != self.refreshed_position {
            self.refreshed_position = position;
            cx.notify();
        }
    }

    /// Keeps the Discord status on what is playing. A song opening keeps the
    /// status of the one before, so that skipping does not make it flicker.
    fn show_presence(&mut self, cx: &mut Context<Self>) {
        match self.state {
            State::Playing => {
                let duration = self.duration();
                if let Some(song) = self.queue.current() {
                    self.presence
                        .listening(song, self.engine.position(), duration);
                }
                self.seek_presence_cover(cx);
            }
            State::Paused | State::Stopped => self.presence.clear(),
            State::Opening => {}
        }
    }

    /// Finds the current song's cover a public URL for the Discord status.
    /// Only the status depends on it, so a failure is not shown. It is not
    /// called off when the song changes: an upload cut short leaves a file up
    /// with no record of it, and the next play would upload another. It is also
    /// how a link that has run out gets replaced, for as long as the app runs.
    fn seek_presence_cover(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.cover_key.clone() else {
            return;
        };
        let Some(cover) = self
            .queue
            .current()
            .and_then(Song::cover_art)
            .filter(|cover| {
                self.sought_cover.as_ref() != Some(*cover) && !self.presence.has_cover(cover)
            })
            .cloned()
        else {
            return;
        };
        self.sought_cover = Some(cover.clone());
        let url = self.session.cover_art_url(&cover, PRESENCE_COVER_PIXELS);
        let http = cx.http_client();
        let host = self.cover_host.clone();
        let link = cx.background_spawn(async move {
            let mut response = http
                .get(url.as_ref(), AsyncBody::empty(), true)
                .await
                .ok()?;
            let mut image = Vec::new();
            response.body_mut().read_to_end(&mut image).await.ok()?;
            Some(host.link(&image, &key).await)
        });
        cx.spawn(async move |this, cx| {
            // Without the image, it is the server that failed, which the rest
            // of the app shows.
            let Some(link) = link.await else {
                return;
            };
            this.update(cx, |this, cx| match link {
                Ok(link) => {
                    if this.sought_cover.as_ref() == Some(&cover) {
                        this.sought_cover = None;
                    }
                    this.presence.set_cover(cover, link);
                    cx.emit(PlayerEvent::CoverSought { error: None });
                }
                Err(error) => cx.emit(PlayerEvent::CoverSought {
                    error: Some(error.to_string().into()),
                }),
            })
            .ok();
        })
        .detach();
    }

    /// Tells the server the current song was played. The play is kept first,
    /// so that it still gets there if it cannot be sent now; failing to send
    /// it is not shown, as nothing about playback depends on it.
    fn scrobble_current(&self, cx: &mut Context<Self>) {
        let Some(song) = self.queue.current().map(|song| song.id().clone()) else {
            return;
        };
        let played_at = SystemTime::now();
        let client = self.session.client().clone();
        let scrobbles = self.session.scrobbles().cloned();
        let sent = cx.background_spawn(async move {
            match scrobbles {
                Some(scrobbles) if scrobbles.record(&song, played_at).is_ok() => {
                    scrobbles.submit(&client).await.ok();
                }
                _ => {
                    client.scrobble(&song, played_at).await.ok();
                }
            }
        });
        cx.spawn(async move |this, cx| {
            sent.await;
            this.update(cx, |_, cx| cx.emit(PlayerEvent::Played)).ok();
        })
        .detach();
    }

    /// Tells the server the current song is playing now, also after a pause,
    /// which the server may have taken for the end of it. Replaces a notice
    /// still on its way, which is out of date, and is not sent again if it
    /// fails: by then it would be out of date too.
    fn announce_current(&mut self, cx: &mut Context<Self>) {
        let Some(song) = self.queue.current().map(|song| song.id().clone()) else {
            return;
        };
        let client = self.session.client().clone();
        self._now_playing = cx.background_spawn(async move {
            client.now_playing(&song).await.ok();
        });
    }

    /// Moves the position slider, unless it is there already.
    fn show_position(&self, position: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let fraction = self.duration().map_or(0.0, |duration| {
            (position.as_secs_f32() / duration.as_secs_f32()).min(1.0)
        });
        if self.position.read(cx).value().end() != fraction {
            self.position
                .update(cx, |slider, cx| slider.set_value(fraction, window, cx));
        }
    }

    fn level(&self, cx: &Context<Self>) -> f32 {
        self.volume.read(cx).value().end()
    }

    fn apply_volume(&mut self, level: f32, cx: &mut Context<Self>) {
        // Moving the volume up is a way to unmute.
        if level > 0.0 {
            self.muted = false;
        }
        self.engine.set_volume(gain(level, self.muted));
        cx.notify();
    }

    fn step_volume(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let level = (self.level(cx) + delta).clamp(0.0, 1.0);
        self.volume
            .update(cx, |slider, cx| slider.set_value(level, window, cx));
        self.apply_volume(level, cx);
        self.save_volume(level, cx);
    }

    /// Remembers the volume for the next launch. A failure only means the
    /// next launch starts at the old volume, so it is not shown.
    ///
    /// One write at a time, so a held volume key cannot leave an older
    /// value on disk; the latest value is written when the current write
    /// finishes.
    fn save_volume(&mut self, level: f32, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        self.unsaved_volume = Some(level);
        if self.saving_volume {
            return;
        }
        self.saving_volume = true;
        self._save_volume = cx.spawn(async move |this, cx| {
            while let Ok(Some(level)) = this.update(cx, |this, _| {
                let level = this.unsaved_volume.take();
                this.saving_volume = level.is_some();
                level
            }) {
                let store = store.clone();
                cx.background_executor()
                    .spawn(async move {
                        store.save_volume(level).ok();
                    })
                    .await;
            }
        });
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self._open = Task::ready(());
        self.engine.stop();
        self.load = None;
        self.arrival = None;
        self.following = None;
        self.format = None;
        self.resume = None;
        if !self.has_song() {
            self.cover_url = None;
        }
        self.set_state(State::Stopped, cx);
    }

    fn play_current(&mut self, cx: &mut Context<Self>) {
        self.resume = None;
        self.tally = PlayTally::default();
        self.open_current(false, cx);
    }

    /// Plays the current song from `resume`, or else from its start; with
    /// `forget_kept`, from the server after removing its kept file.
    fn open_current(&mut self, forget_kept: bool, cx: &mut Context<Self>) {
        let Some(song) = self.queue.current().cloned() else {
            self.stop(cx);
            return;
        };
        // Silence the previous song first, and ignore what is still on its
        // way about it, so its end cannot advance the queue a second time.
        self.engine.stop();
        self.load = None;
        self.arrival = None;
        self.following = None;
        self.loaded_ahead = false;
        self.show_opening(&song, cx);

        let client = self.session.client().clone();
        let song_cache = song_cache(cx);
        // Replacing the task drops an earlier open, and with it its download.
        self._open = cx.spawn(async move |this, cx| {
            // Removing and opening a kept song read and write the disk.
            let opened = cx
                .background_spawn({
                    let song = song.clone();
                    async move {
                        // A file that cannot be removed would fail again, so
                        // the song then comes from the server uncached.
                        let song_cache = song_cache.filter(|song_cache| {
                            !forget_kept || song_cache.forget(&client, song.id()).is_ok()
                        });
                        playback::open_track(&client, song_cache.as_ref(), &song).await
                    }
                })
                .await;
            this.update(cx, |this, cx| match opened {
                Ok(opened) => {
                    this.loaded_kept = opened.is_kept();
                    this.arrival = opened.arrival();
                    let resume = this.resume.unwrap_or_default();
                    let track = opened
                        .into_track()
                        .start_at(resume.position)
                        .start_paused(resume.paused);
                    this.load = Some(this.engine.load(track));
                }
                Err(error) => this.fail(&song, &session::describe(&error), cx),
            })
            .ok();
        });
    }

    /// Shows `song`, now current, as about to play.
    fn show_opening(&mut self, song: &Song, cx: &mut Context<Self>) {
        self.format = None;
        self.error = None;
        self.scrub = None;
        self.cover_url = song
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, COVER_PIXELS));
        self.set_state(State::Opening, cx);
    }

    /// Makes the entry that follows the current one ready to go on to,
    /// replacing what was being made ready if that is no longer next. Waits
    /// for the current song's download, so that the two never compete for the
    /// connection and leave the song playing short of data.
    fn prepare_following(&mut self, cx: &mut Context<Self>) {
        if !self.is_started() {
            return;
        }
        let entry = self.queue.following_id();
        if self.following.as_ref().map(|following| following.entry) == entry {
            return;
        }
        let was_set = self
            .following
            .take()
            .is_some_and(|following| following.load.is_some());
        if was_set {
            self.engine.clear_next();
        }
        let Some((entry, song)) = entry.zip(self.queue.following().cloned()) else {
            return;
        };
        let client = self.session.client().clone();
        let song_cache = song_cache(cx);
        let arrival = self.arrival.clone();
        let open = cx.spawn(async move |this, cx| {
            if let Some(arrival) = arrival {
                arrival.wait().await;
            }
            let opened = cx
                .background_spawn(async move {
                    playback::open_track(&client, song_cache.as_ref(), &song).await
                })
                .await;
            this.update(cx, |this, _| {
                if let (Some(following), Ok(opened)) = (this.following.as_mut(), opened) {
                    following.kept = opened.is_kept();
                    following.arrival = opened.arrival();
                    following.load = Some(this.engine.set_next(opened.into_track()));
                }
            })
            .ok();
        });
        self.following = Some(Following {
            entry,
            load: None,
            kept: false,
            arrival: None,
            _open: open,
        });
    }

    /// Moves on after the current song has ended. When the engine goes on to
    /// the song made ready for this, it only has to be shown; in any other
    /// case, such as a change of the queue crossing the end of the song, the
    /// next song opens from scratch.
    ///
    /// A pause on its way through the events about that holds for the next
    /// song, as the engine stays paused.
    fn song_ended(&mut self, next: Option<LoadId>, cx: &mut Context<Self>) {
        let paused = self.state == State::Paused;
        let following_id = self.queue.following_id();
        let following = self.following.take().filter(|following| {
            following.load.is_some()
                && following.load == next
                && Some(following.entry) == following_id
        });
        let Some(song) = self.queue.follow().cloned() else {
            self.stop(cx);
            return;
        };
        self.tally = PlayTally::default();
        self.resume = paused.then_some(Resume {
            position: Duration::ZERO,
            paused,
        });
        let Some(following) = following else {
            self.open_current(false, cx);
            return;
        };
        self.load = following.load;
        self.loaded_kept = following.kept;
        self.loaded_ahead = true;
        self.arrival = following.arrival;
        self.show_opening(&song, cx);
    }

    fn on_engine_event(&mut self, event: Event, cx: &mut Context<Self>) {
        if self.load != Some(event.load) {
            return;
        }
        match event.kind {
            EventKind::Started(format) => {
                self.format = Some(format);
                self.error = None;
                self.loaded_ahead = false;
                let paused = self.resume.take().is_some_and(|resume| resume.paused);
                let state = if paused {
                    State::Paused
                } else {
                    State::Playing
                };
                self.set_state(state, cx);
                if !paused {
                    self.announce_current(cx);
                }
                self.prepare_following(cx);
            }
            EventKind::OutputChanged(format) => {
                self.format = Some(format);
                cx.notify();
            }
            EventKind::Ended { next } => self.song_ended(next, cx),
            // The song fetched again plays from a download, not a kept
            // file, so a song is fetched again at most once per play.
            EventKind::Failed(error) if self.loaded_kept && error.is_track_error() => {
                self.open_current(true, cx);
            }
            // It opened a while ago, and what stopped it then may have passed.
            EventKind::Failed(error) if self.loaded_ahead && error.is_track_error() => {
                self.open_current(false, cx);
            }
            EventKind::Failed(error) => {
                // The output is at fault, not the song, so it can pick up
                // once there is one that works.
                self.resume = (!error.is_track_error()).then(|| self.resume_point());
                if let Some(song) = self.queue.current().cloned() {
                    self.fail(&song, &describe(&error), cx);
                }
            }
        }
    }

    /// Where the current song is, or was to pick up if it had yet to start.
    fn resume_point(&self) -> Resume {
        match self.state {
            State::Playing | State::Paused => Resume {
                position: self.engine.position(),
                paused: self.state == State::Paused,
            },
            State::Opening | State::Stopped => self.resume.unwrap_or_default(),
        }
    }

    /// Stops on the failed song and says why, so Play retries it.
    fn fail(&mut self, song: &Song, reason: &str, cx: &mut Context<Self>) {
        self.engine.stop();
        self.load = None;
        self.arrival = None;
        self.following = None;
        self.format = None;
        self.error = Some(format!("Couldn't play “{}”. {reason}", song.title()).into());
        self.set_state(State::Stopped, cx);
    }

    fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        cx.emit(PlayerEvent::NowPlaying {
            song: self.queue.current().map(|song| song.id().clone()),
            playing: matches!(state, State::Playing | State::Opening),
        });
        cx.notify();
    }

    fn render_song(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let song = self.queue.current();
        let error = self.error.as_ref().map(|error| {
            div()
                .text_xs()
                .truncate()
                .text_color(cx.theme().danger)
                .child(error.clone())
        });
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .when_some(song, |this, song| {
                this.child(CoverArt::new(self.cover_url.clone()).small())
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .child(SharedString::from(song.title().to_owned())),
                            )
                            .map(|this| match error {
                                Some(error) => this.child(error),
                                None => this.child(song_details(song, cx)),
                            }),
                    )
            })
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_song = self.has_song();
        let playing = matches!(self.state, State::Playing | State::Opening);
        let (icon, label) = if playing {
            (IconName::Pause, "Pause")
        } else {
            (IconName::Play, "Play")
        };
        h_flex()
            .flex_none()
            .gap_1()
            .child(
                Button::new("previous-song")
                    .ghost()
                    .icon(IconName::SkipBack)
                    .accessibility_label("Previous")
                    .tooltip_with_action("Previous", &PreviousSong, None)
                    .disabled(!has_song)
                    .on_click(cx.listener(|this, _, _, cx| this.previous(cx))),
            )
            .child(
                Button::new("toggle-playback")
                    .ghost()
                    .icon(icon)
                    .accessibility_label(label)
                    .tooltip_with_action(label, &TogglePlayback, None)
                    .disabled(!has_song || self.state == State::Opening)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_playback(cx))),
            )
            .child(
                Button::new("next-song")
                    .ghost()
                    .icon(IconName::SkipForward)
                    .accessibility_label("Next")
                    .tooltip_with_action("Next", &NextSong, None)
                    .disabled(!has_song)
                    .on_click(cx.listener(|this, _, _, cx| this.next(cx))),
            )
    }

    fn render_position(&self) -> impl IntoElement {
        let duration = self.duration();
        let seekable = self.is_started() && duration.is_some();
        let position = match (self.scrub, duration) {
            (Some(fraction), Some(duration)) => duration.mul_f32(fraction),
            _ => self.position_now(),
        };
        h_flex()
            .flex_none()
            .gap_3()
            .child(div().flex_none().child(format_duration(position)))
            .child(
                div().w_48().child(
                    Slider::new(&self.position)
                        .disabled(!seekable)
                        .accessibility_label("Position"),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .child(duration.map(format_duration).unwrap_or_default()),
            )
    }

    fn render_volume(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let level = self.level(cx);
        let (icon, label) = if self.muted || level == 0.0 {
            (IconName::VolumeX, "Unmute")
        } else if level < 0.5 {
            (IconName::Volume1, "Mute")
        } else {
            (IconName::Volume2, "Mute")
        };
        h_flex()
            .flex_none()
            .gap_1()
            .child(
                Button::new("toggle-mute")
                    .ghost()
                    .small()
                    .icon(icon)
                    .accessibility_label(label)
                    .tooltip_with_action(label, &ToggleMute, None)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
            )
            .child(
                div()
                    .w_20()
                    .child(Slider::new(&self.volume).accessibility_label("Volume")),
            )
    }
}

/// The line under the song playing: its artist and its album, whose names can
/// be clicked.
fn song_details(song: &Song, cx: &gpui_kit::App) -> impl IntoElement + use<> {
    let artist: Option<ArtistLink> = song_artist_link(song);
    let album = AlbumLink::of_song(song);
    let separated = artist.is_some();
    h_flex()
        .min_w_0()
        .gap_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .children(artist)
        .when_some(album, |this, album| {
            this.when(separated, |this| this.child(div().flex_none().child("—")))
                .child(album)
        })
}

/// The songs kept on this computer, if there are any to be had.
fn song_cache(cx: &Context<Player>) -> Option<playback::SongCache> {
    cx.try_global::<GlobalSongCache>()
        .and_then(|song_cache| song_cache.0.clone())
}

/// The engine gain for a volume slider at `level`. Squared, so equal slider
/// steps sound like roughly equal changes in loudness.
fn gain(level: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { level * level }
}

/// Why the engine could not play, as a sentence that says what to try.
fn describe(error: &audio::Error) -> String {
    match error {
        audio::Error::NoOutputDevice => {
            "No audio output device is available. Connect one and press Play.".to_owned()
        }
        audio::Error::OutputDeviceUnavailable => {
            "The output device isn't connected. Connect it or choose another in Settings, then press Play."
                .to_owned()
        }
        audio::Error::UnsupportedOutputFormat | audio::Error::Resample(_) => {
            "The output device can't play this audio. Choose another in Settings.".to_owned()
        }
        audio::Error::Output(_) => "The audio output stopped. Press Play to try again.".to_owned(),
        audio::Error::UnsupportedFormat(_) => "hiphile can't play this file format.".to_owned(),
        audio::Error::Decode(_) | audio::Error::Read(_) => {
            "The file couldn't be read. Press Play to try again.".to_owned()
        }
    }
}

impl Render for Player {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let format_text = self
            .format
            .as_ref()
            .map(|format| format::describe(format, self.queue.current().and_then(Song::suffix)));
        let muted_foreground = cx.theme().muted_foreground;
        h_flex()
            .flex_none()
            .h_16()
            .px_4()
            .gap_6()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(self.render_song(cx))
            .child(self.render_controls(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .gap_4()
                    .text_xs()
                    .text_color(muted_foreground)
                    .when(self.has_song(), |this| this.child(self.render_position()))
                    .child(self.render_volume(cx))
                    .when_some(format_text, |this, text| {
                        this.child(div().flex_none().child(text))
                    }),
            )
    }
}

#[cfg(test)]
impl Player {
    /// Queues `songs` with the one at `current_ix` current, without playing.
    pub(crate) fn fill_queue(&mut self, songs: &[Song], current_ix: usize) {
        self.queue.replace(songs.to_vec(), current_ix);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use gpui_kit::{TestAppContext, VisualTestContext};
    use serde_json::json;
    use subsonic::Credentials;

    use super::*;

    const RATE: u32 = 44_100;
    const WAIT: Duration = Duration::from_secs(15);

    /// The line under a song playing, on its own.
    struct Details(Song);

    impl gpui_kit::Render for Details {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(song_details(&self.0, cx))
        }
    }

    #[gpui_kit::test]
    fn clicking_the_album_under_the_song_shows_it(cx: &mut TestAppContext) {
        let shown = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            let shown = shown.clone();
            cx.on_action(move |action: &actions::ShowAlbum, _| {
                shown.borrow_mut().push(action.album().clone());
            });
        });
        let song = serde_json::from_value(json!({
            "id": "s", "title": "So What", "album": "Kind of Blue", "albumId": "al1",
        }))
        .unwrap();
        let (_, cx) = cx.add_window_view(|_, _| Details(song));
        let cx: &mut VisualTestContext = cx;
        cx.simulate_resize(gpui_kit::size(gpui_kit::px(400.), gpui_kit::px(100.)));
        cx.run_until_parked();

        let position = gpui_kit::point(gpui_kit::px(10.), gpui_kit::px(6.));
        cx.simulate_click(position, Default::default());
        cx.run_until_parked();

        assert_eq!(shown.take(), [subsonic::AlbumId::new("al1")]);
    }

    #[test]
    fn gain_follows_the_square_of_the_level() {
        assert_eq!(gain(1.0, false), 1.0);
        assert_eq!(gain(0.5, false), 0.25);
        assert_eq!(gain(1.0, true), 0.0);
    }

    /// A silent 16-bit stereo WAV.
    fn wav(millis: u32) -> Vec<u8> {
        let frames = RATE * millis / 1_000;
        let data_len = frames * 4;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&RATE.to_le_bytes());
        bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(44 + data_len as usize, 0);
        bytes
    }

    /// Serves songs by ID and notes which were asked for, in order.
    #[derive(Clone, Default)]
    struct Server {
        songs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        streamed: Arc<Mutex<Vec<String>>>,
        /// While set, the body of the song named keeps its last half back.
        held: Arc<Mutex<Option<String>>>,
        released: Arc<AtomicBool>,
        /// The connection is lost after this many bytes of the song named.
        cut: Arc<Mutex<Option<(String, usize)>>>,
    }

    impl Server {
        fn start(self) -> String {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = format!("http://{}/", listener.local_addr().unwrap());
            std::thread::spawn(move || {
                for socket in listener.incoming() {
                    let Ok(socket) = socket else { return };
                    let server = self.clone();
                    std::thread::spawn(move || server.answer(socket));
                }
            });
            address
        }

        fn answer(&self, mut socket: TcpStream) {
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).ok();
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 2 {
                line.clear();
            }
            let streamed = request.contains("/stream");
            let id = request
                .split(['?', '&', ' '])
                .find_map(|part| part.strip_prefix("id="))
                .unwrap_or_default()
                .to_owned();
            let song = streamed
                .then(|| {
                    self.streamed.lock().unwrap().push(id.clone());
                    self.songs.lock().unwrap().get(&id).cloned()
                })
                .flatten();
            let Some(body) = song else {
                let status = if streamed { "404 Not Found" } else { "200 OK" };
                let reply =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                socket.write_all(reply.as_bytes()).ok();
                return;
            };
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).ok();
            if let Some((_, len)) = self.cut.lock().unwrap().as_ref().filter(|cut| cut.0 == id) {
                socket.write_all(&body[..*len]).ok();
                return;
            }
            let held = self.held.lock().unwrap().as_deref() == Some(id.as_str());
            let head = if held { body.len() / 2 } else { body.len() };
            socket.write_all(&body[..head]).ok();
            while !self.released.load(Ordering::SeqCst) && head < body.len() {
                std::thread::sleep(Duration::from_millis(5));
            }
            socket.write_all(&body[head..]).ok();
            // Until the client has gone, so that it decides when a body ends.
            let _ = reader.read(&mut [0; 1]);
        }

        fn add(&self, id: &str, body: Vec<u8>) -> Song {
            let song = json!({
                "id": id, "title": id, "suffix": "wav", "size": body.len(), "duration": 1
            });
            self.songs.lock().unwrap().insert(id.to_owned(), body);
            serde_json::from_value(song).unwrap()
        }

        fn streamed(&self) -> Vec<String> {
            self.streamed.lock().unwrap().clone()
        }
    }

    fn player<'a>(
        server: &Server,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Player>, &'a mut VisualTestContext) {
        // The engine and the network wake the test from their own threads.
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let name = std::env::var("HIPHILE_TEST_DEVICE")
            .expect("name a virtual device in HIPHILE_TEST_DEVICE");
        let device = audio::output_devices()
            .unwrap()
            .into_iter()
            .find(|device| device.name().trim() == name)
            .expect("no such device");
        let output = OutputSettings::new(device.id().map(str::to_owned), None);
        let address = server.clone().start();
        let net = net::Client::new("hiphile-test").unwrap();
        let server = client::Server::new(&address, Credentials::api_key("k")).unwrap();
        let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
        let session = session::Session::new(
            client::Client::new(net, server),
            "alice".to_owned(),
            info,
            None,
            None,
        );
        cx.add_window_view(|window, cx| Player::new(session, output, window, cx))
    }

    fn wait_until(
        player: &Entity<Player>,
        cx: &mut VisualTestContext,
        what: &str,
        mut condition: impl FnMut(&Player) -> bool,
    ) {
        let started = Instant::now();
        loop {
            cx.run_until_parked();
            if player.read_with(cx, |player, _| condition(player)) {
                return;
            }
            assert!(started.elapsed() < WAIT, "still waiting: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn current(player: &Player) -> Option<&str> {
        player.queue.current().map(|song| song.id().as_str())
    }

    fn is_playing_song(player: &Player, id: &str) -> bool {
        player.state == State::Playing && current(player) == Some(id)
    }

    fn is_following_set(player: &Player, id: &str) -> bool {
        player.queue.following().map(|song| song.id().as_str()) == Some(id)
            && player
                .following
                .as_ref()
                .is_some_and(|following| following.load.is_some())
    }

    /// Each song is fetched once and only after the one before it has
    /// arrived, and playback goes through the queue on its own.
    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn the_next_song_is_fetched_once_the_current_one_has_arrived(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [
            server.add("a", wav(2_000)),
            server.add("b", wav(500)),
            server.add("c", wav(300)),
        ];
        *server.held.lock().unwrap() = Some("a".to_owned());
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));

        wait_until(&player, cx, "a to play", |player| {
            is_playing_song(player, "a")
        });
        std::thread::sleep(Duration::from_millis(100));
        cx.run_until_parked();
        assert_eq!(server.streamed(), ["a"]);

        server.released.store(true, Ordering::SeqCst);
        wait_until(&player, cx, "b to be set", |player| {
            is_following_set(player, "b")
        });
        assert_eq!(server.streamed(), ["a", "b"]);

        wait_until(&player, cx, "b to play", |player| {
            is_playing_song(player, "b")
        });
        let duration = player.read_with(cx, |player, _| player.duration());
        assert_eq!(duration, Some(Duration::from_millis(500)));
        wait_until(&player, cx, "c to play", |player| {
            is_playing_song(player, "c")
        });
        wait_until(&player, cx, "the end", |player| {
            player.state == State::Stopped
        });
        assert_eq!(server.streamed(), ["a", "b", "c"]);
        player.read_with(cx, |player, _| {
            assert_eq!(current(player), None);
            assert!(player.following.is_none());
            assert!(player.arrival.is_none());
            assert!(player.error.is_none());
        });
    }

    /// A change of what comes next replaces what was made ready for it.
    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn moving_another_song_next_makes_it_follow(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [
            server.add("a", wav(1_500)),
            server.add("b", wav(5_000)),
            server.add("c", wav(5_000)),
        ];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "b to be set", |player| {
            is_following_set(player, "b")
        });

        player.update(cx, |player, cx| {
            let last = player.queue.entries()[2].id();
            player.move_entry_to_next(last, cx);
        });
        wait_until(&player, cx, "c to be set", |player| {
            is_following_set(player, "c")
        });
        wait_until(&player, cx, "c to play", |player| {
            is_playing_song(player, "c")
        });
        assert_eq!(server.streamed()[..3], ["a", "b", "c"]);
    }

    /// With nothing left to come next, playback ends after the song, even when
    /// the removed song was already on its way to the device. That its audio
    /// is taken back is for the engine's tests, which can hear it.
    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn removing_the_next_song_ends_playback_after_the_current_one(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(1_000)), server.add("b", wav(5_000))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "b to be set", |player| {
            is_following_set(player, "b")
        });
        // Within what the engine writes ahead of the end of the song.
        wait_until(&player, cx, "the end of a to be near", |player| {
            player.engine.position() > Duration::from_millis(700)
        });
        player.update(cx, |player, cx| {
            let next = player.queue.entries()[1].id();
            player.remove_entry(next, cx);
        });
        wait_until(&player, cx, "the end", |player| {
            player.state == State::Stopped
        });
        player.read_with(cx, |player, _| assert!(player.error.is_none()));
    }

    /// A next song the server has no file for fails when its turn comes, not
    /// before: the current song plays out undisturbed.
    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn a_next_song_that_cannot_be_fetched_fails_at_its_turn(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [
            server.add("a", wav(1_000)),
            serde_json::from_value(json!({ "id": "gone", "title": "Gone" })).unwrap(),
        ];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "the attempt", |_| {
            server.streamed() == ["a", "gone"]
        });
        cx.run_until_parked();
        player.read_with(cx, |player, _| {
            assert!(is_playing_song(player, "a"));
            assert!(player.error.is_none());
        });
        wait_until(&player, cx, "the failure", |player| player.error.is_some());
        player.read_with(cx, |player, _| {
            assert_eq!(current(player), Some("gone"));
            assert_eq!(player.state, State::Stopped);
        });
        assert_eq!(server.streamed(), ["a", "gone", "gone"]);
    }

    /// A next song that opened ahead of time gets one more try at its turn,
    /// as what stopped it may have passed.
    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn a_next_song_that_cannot_open_is_tried_once_more(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [
            server.add("a", wav(1_000)),
            server.add("broken", b"not audio".to_vec()),
        ];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "the failure", |player| player.error.is_some());
        player.read_with(cx, |player, _| {
            assert_eq!(current(player), Some("broken"));
            assert_eq!(player.state, State::Stopped);
            assert!(player.arrival.is_none());
        });
        assert_eq!(server.streamed(), ["a", "broken", "broken"]);
    }

    /// So does one that opened and then broke, once its audio has joined the
    /// current song's but before the device has got to it.
    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn a_next_song_that_breaks_as_it_joins_is_tried_once_more(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(1_000)), server.add("b", wav(1_000))];
        *server.cut.lock().unwrap() = Some(("b".to_owned(), 44 + 16));
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "the failure", |player| player.error.is_some());
        player.read_with(cx, |player, _| {
            assert_eq!(current(player), Some("b"));
            assert_eq!(player.state, State::Stopped);
        });
        let tries = server.streamed();
        assert_eq!(tries.iter().filter(|id| *id == "b").count(), 2, "{tries:?}");
    }

    /// A pause that crosses the events of the next song starting stays: the
    /// engine has paused the song, so the player must not show it as playing.
    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn a_pause_as_the_next_song_starts_holds(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(1_000)), server.add("b", wav(5_000))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "b to be set", |player| {
            is_following_set(player, "b")
        });

        // Without running the player, so that the events wait for it.
        let position = || player.read_with(cx, |player, _| player.engine.position());
        let started = Instant::now();
        let mut last = position();
        while position() >= last {
            last = position();
            assert!(started.elapsed() < WAIT, "still waiting for the next song");
            std::thread::sleep(Duration::from_millis(1));
        }
        player.update(cx, |player, cx| {
            assert!(is_playing_song(player, "a"));
            player.toggle_playback(cx);
        });

        wait_until(&player, cx, "b to start", |player| {
            current(player) == Some("b") && player.format.is_some()
        });
        player.read_with(cx, |player, _| assert_eq!(player.state, State::Paused));
        std::thread::sleep(Duration::from_millis(100));
        let paused_at = player.read_with(cx, |player, _| player.engine.position());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            player.read_with(cx, |player, _| player.engine.position()),
            paused_at
        );

        player.update(cx, |player, cx| player.toggle_playback(cx));
        player.read_with(cx, |player, _| assert_eq!(player.state, State::Playing));
        std::thread::sleep(Duration::from_millis(100));
        assert!(player.read_with(cx, |player, _| player.engine.position()) > paused_at);
    }
}
