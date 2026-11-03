use std::time::{Duration, SystemTime};

use audio::{Engine, Event, EventKind, LoadId, OutputSettings, PlaybackFormat};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    slider::{Slider, SliderEvent, SliderState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use playback::{EntryId, Placement, PlayTally, Queue};
use subsonic::{Song, SongId};
use ui::{CoverArt, format_duration};

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
    queue: Queue<Song>,
    state: State,
    format: Option<PlaybackFormat>,
    /// Where the stopped song picks back up, after its output failed.
    resume: Option<Resume>,
    /// How much of this play of the current song has been listened to.
    tally: PlayTally,
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
            cx.background_spawn(async move { scrobbles.submit(&client).await.ok() })
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
        Self {
            session,
            store,
            engine,
            load: None,
            loaded_kept: false,
            queue: Queue::new(),
            state: State::Stopped,
            format: None,
            resume: None,
            tally: PlayTally::default(),
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

    /// Whether there is a song to play, pause, or skip.
    pub fn has_song(&self) -> bool {
        self.queue.current().is_some()
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
        if self.scrub.is_none() {
            self.show_position(position, window, cx);
        }
        if position != self.refreshed_position {
            self.refreshed_position = position;
            cx.notify();
        }
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
        cx.background_spawn(async move {
            match scrobbles {
                Some(scrobbles) if scrobbles.record(&song, played_at).is_ok() => {
                    scrobbles.submit(&client).await.ok();
                }
                _ => {
                    client.scrobble(&song, played_at).await.ok();
                }
            }
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
        self.format = None;
        self.error = None;
        self.scrub = None;
        self.cover_url = song
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, COVER_PIXELS));
        self.set_state(State::Opening, cx);

        let client = self.session.client().clone();
        let song_cache = cx
            .try_global::<GlobalSongCache>()
            .and_then(|song_cache| song_cache.0.clone());
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

    fn on_engine_event(&mut self, event: Event, cx: &mut Context<Self>) {
        if self.load != Some(event.load) {
            return;
        }
        match event.kind {
            EventKind::Started(format) => {
                self.format = Some(format);
                self.error = None;
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
            }
            EventKind::OutputChanged(format) => {
                self.format = Some(format);
                cx.notify();
            }
            EventKind::Ended => {
                if self.queue.advance().is_some() {
                    self.play_current(cx);
                } else {
                    self.stop(cx);
                }
            }
            // The song fetched again plays from a download, not a kept
            // file, so a song is fetched again at most once per play.
            EventKind::Failed(error) if self.loaded_kept && error.is_track_error() => {
                self.open_current(true, cx);
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
        let secondary = match (&self.error, song) {
            (Some(error), _) => Some((error.clone(), cx.theme().danger)),
            (None, Some(song)) => {
                let line = match (song.artist(), song.album()) {
                    (Some(artist), Some(album)) => format!("{artist} — {album}"),
                    (Some(artist), None) => artist.to_owned(),
                    (None, Some(album)) => album.to_owned(),
                    (None, None) => String::new(),
                };
                Some((line.into(), cx.theme().muted_foreground))
            }
            (None, None) => None,
        };
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
                            .when_some(secondary, |this, (text, color)| {
                                this.child(div().text_xs().truncate().text_color(color).child(text))
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
mod tests {
    use super::gain;

    #[test]
    fn gain_follows_the_square_of_the_level() {
        assert_eq!(gain(1.0, false), 1.0);
        assert_eq!(gain(0.5, false), 0.25);
        assert_eq!(gain(1.0, true), 0.0);
    }
}
