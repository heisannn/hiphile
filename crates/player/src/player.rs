use std::sync::Arc;
use std::time::{Duration, SystemTime};

use audio::{Engine, Event, EventKind, LoadId, OutputSettings, PlaybackFormat, Transitions};
use futures::AsyncReadExt as _;
use gpui_kit::base::{Transition, transition};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, h_flex,
    slider::{SliderEvent, SliderState},
    v_flex,
};
use gpui_kit::http_client::{AsyncBody, HttpClient};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, relative, rems,
};
use hiphile_core::favorites::Favorite;
use media_controls::{CoverFile, MediaCommand, MediaControls, MediaStatus};
use playback::{Arrival, EntryId, Placement, PlayTally, Queue, RepeatMode, describe_format};
use presence::{CoverHost, Presence};
use subsonic::{CoverArtId, Song, SongId};
use ui::{AlbumLink, ArtistLink, CoverArt, song_artist_link};

use crate::transport::{PanelToggles, PositionLine, TransportControls, VolumeControl};
use crate::{GlobalSongCache, ToggleLyrics};

const POSITION_REFRESH: Duration = Duration::from_millis(250);
/// Within this much of a song's start, Previous goes to the previous song;
/// later, it restarts the current one.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
const SEEK_STEP: Duration = Duration::from_secs(10);
const VOLUME_STEP: f32 = 0.1;
/// Pixel size requested for the bar's cover. The renderer aliases an image drawn
/// at less than half its size, so this stays close to the on-screen size.
const COVER_PIXELS: u32 = 48;
const LARGE_COVER_PIXELS: u32 = 512;
/// Pixel size of the cover published for the Discord status and given to the
/// system's media controls, the same as the lyrics view's, so that it is often
/// on disk already.
const PUBLISHED_COVER_PIXELS: u32 = 512;
const COVER_HOVER_OPACITY: f32 = 0.08;
const COVER_PRESS_OPACITY: f32 = 0.14;

pub enum PlayerEvent {
    NowPlaying {
        song: Option<SongId>,
        /// Whether it is playing or about to, rather than paused or stopped.
        playing: bool,
    },
    /// Entries were added, removed, or moved. A change of the current entry
    /// comes as [`PlayerEvent::NowPlaying`], also when a shuffled queue starts
    /// over in a new order with it.
    QueueChanged,
    /// The queue was shuffled or put back in order, after
    /// [`PlayerEvent::QueueChanged`] about the same change.
    ShuffleChanged,
    RepeatChanged,
    /// A play of a song has been reported to the server, as far as it could
    /// be, so what the server says was played last may have changed.
    Played,
    /// A cover was looked for to put on the Discord status, and why it could
    /// not be published, if it could not.
    CoverSought {
        error: Option<SharedString>,
    },
}

/// What lays them out knows this, and the player shows it on the buttons that toggle
/// them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Panels {
    pub is_lyrics_open: bool,
    pub is_mini_player_open: bool,
    /// Whether the queue is to be shown, when it fits.
    pub is_queue_shown: bool,
    pub queue_fits: bool,
}

type PanelsSource = Box<dyn Fn(&Window, &App) -> Panels>;

/// Dropping it, or quitting the app, stops playback and returns the output device to
/// its sample rate from before.
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
    following: Option<Following>,
    queue: Queue<Song>,
    state: State,
    /// Whether the engine says the song is silent for want of its data.
    buffering: bool,
    format: Option<PlaybackFormat>,
    /// Where the stopped song picks back up, after its output failed.
    resume: Option<Resume>,
    tally: PlayTally,
    presence: Presence,
    cover_host: Arc<CoverHost>,
    /// Without an API key, no cover goes on the Discord status.
    cover_key: Option<Arc<str>>,
    /// The cover being looked for, or last looked for without success, which
    /// is then not looked for again until another song has had its turn.
    sought_cover: Option<CoverArtId>,
    media_controls: MediaControls,
    /// The cover being looked for, or last looked for without success, for the
    /// system's media controls, which is then not looked for again until the
    /// next song.
    sought_media_cover: Option<CoverArtId>,
    cover_url: Option<SharedString>,
    large_cover_url: Option<SharedString>,
    /// Whether the bar leaves out its cover, for another that stands in for it.
    is_cover_hidden: bool,
    panels: PanelsSource,
    error: Option<SharedString>,
    /// The position as a fraction of the song.
    position: Entity<SliderState>,
    /// The same for the mini player. A slider keeps where it was last painted,
    /// for reading the pointer by, so one slider cannot be in two windows.
    mini_position: Entity<SliderState>,
    /// While the position is being dragged, the fraction under the pointer.
    scrub: Option<f32>,
    /// The position at the last refresh, to redraw when it moves while
    /// paused, as after a seek.
    refreshed_position: Duration,
    /// From `0.0` to `1.0`, as the slider shows it.
    volume: Entity<SliderState>,
    muted: bool,
    is_volume_locked: bool,
    dop_start: DopStart,
    /// Play was asked for with nothing loaded, so the next load rises from silence
    rises_at_load: bool,
    _open: Task<()>,
    _now_playing: Task<()>,
    _media_cover: Task<()>,
    _media_commands: Task<()>,
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

/// How sound starts after DoP has been turned on, which brings it to full level
/// whatever the volume was. The level has to come down at the DAC first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DopStart {
    #[default]
    Settled,
    /// Whatever opens next starts paused, until Play.
    Held,
    /// Play came with nothing open, so what opens next plays, fading in.
    Asked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Stopped,
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
        let is_volume_locked = output.dop().is_some();
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
        let (media_controls, media_commands) = MediaControls::connect(window);
        let media_commands = cx.spawn_in(window, async move |this, cx| {
            while let Ok(command) = media_commands.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.on_media_command(command, window, cx);
                    })
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

        // Plays from before that the server has yet to hear of
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
        let position_slider = || SliderState::new().min(0.).max(1.).step(0.001);
        let position = cx.new(|_| position_slider());
        let mini_position = cx.new(|_| position_slider());
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.01)
                .default_value(level)
        });
        let subscriptions = vec![
            actions::observe_favorites(cx),
            cx.subscribe_in(&position, window, Self::on_position_slider),
            cx.subscribe_in(&mini_position, window, Self::on_position_slider),
            cx.subscribe(&volume, |this, _, event, cx| match event {
                SliderEvent::Change(value) => this.apply_volume(value.end(), cx),
                SliderEvent::Release(value) => this.save_volume(value.end(), cx),
            }),
            // Quitting may not drop entities, and dropping the engine does not
            // wait for it, so shut it down here, while the process still runs.
            cx.on_app_quit(|this, _| {
                this.engine.shutdown();
                this.media_controls.clear();
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
            buffering: false,
            format: None,
            resume: None,
            tally: PlayTally::default(),
            // Tests play made-up songs, which must not reach the user's status
            presence: if cfg!(test) {
                Presence::disconnected()
            } else {
                Presence::spawn()
            },
            cover_host,
            cover_key: None,
            sought_cover: None,
            media_controls,
            sought_media_cover: None,
            cover_url: None,
            large_cover_url: None,
            is_cover_hidden: false,
            panels: Box::new(|_, _| Panels::default()),
            error: None,
            position,
            mini_position,
            scrub: None,
            refreshed_position: Duration::ZERO,
            volume,
            muted: false,
            is_volume_locked,
            dop_start: DopStart::default(),
            rises_at_load: false,
            _open: Task::ready(()),
            _now_playing: Task::ready(()),
            _media_cover: Task::ready(()),
            _media_commands: media_commands,
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
    ///
    /// Turning DoP on holds back the sound until Play: see [`DopStart`].
    pub fn set_output(&mut self, output: OutputSettings, cx: &mut Context<Self>) {
        let is_dop = output.dop().is_some();
        if !is_dop {
            self.dop_start = DopStart::Settled;
        } else if !self.is_volume_locked {
            if self.is_playing() {
                self.toggle_playback(cx);
            }
            self.dop_start = DopStart::Held;
        }
        self.is_volume_locked = is_dop;
        self.engine.set_output(output);
        cx.notify();
        if self.state == State::Stopped && self.resume.is_some() {
            self.open_current(false, cx);
        }
    }

    pub fn set_transitions(&mut self, transitions: Transitions) {
        self.engine.set_transitions(transitions);
    }

    /// Publishes covers with this ImgBB API key from now on, or none without
    /// one, trying again a cover that the last key could not publish.
    pub fn set_cover_key(&mut self, key: Option<&str>) {
        self.cover_key = key.map(Arc::from);
        self.sought_cover = None;
    }

    /// Shows what is playing on the Discord status from now on, or takes it
    /// away and publishes no more covers. Turning it on tries again a cover that
    /// could not be published before.
    pub fn set_presence_on(&mut self, is_on: bool) {
        self.presence.set_on(is_on);
        self.sought_cover = None;
    }

    pub fn has_song(&self) -> bool {
        self.queue.current().is_some()
    }

    /// Whether the queue holds any song, even with playback past its end.
    pub fn has_queued_songs(&self) -> bool {
        !self.queue.is_empty()
    }

    pub fn queued_songs(&self) -> Arc<[Song]> {
        self.queue
            .entries()
            .iter()
            .map(|entry| entry.item().clone())
            .collect()
    }

    /// Leaves out the songs that are skipped. Changes nothing and returns `false`
    /// when the song at `start_ix` is one of them.
    pub fn play(&mut self, songs: &[Song], start_ix: usize, cx: &mut Context<Self>) -> bool {
        if songs
            .get(start_ix)
            .is_some_and(|song| actions::is_skipped(song, cx))
        {
            return false;
        }
        let mut kept = Vec::with_capacity(songs.len());
        let mut kept_start_ix = start_ix;
        for (ix, song) in songs.iter().enumerate() {
            if !actions::is_skipped(song, cx) {
                kept.push(song.clone());
            } else if ix < start_ix {
                kept_start_ix -= 1;
            }
        }
        self.queue.replace(kept, kept_start_ix);
        self.queue_changed(cx);
        self.play_current(cx);
        true
    }

    /// Leaves out the songs that are skipped. Changes nothing and returns `false`
    /// when that is all of them.
    pub fn play_whole(&mut self, songs: &[Song], cx: &mut Context<Self>) -> bool {
        let kept = actions::without_skipped(songs, cx);
        if kept.is_empty() && !songs.is_empty() {
            return false;
        }
        self.queue.replace_from_start(kept);
        self.queue_changed(cx);
        self.play_current(cx);
        true
    }

    /// Adds `songs` to the queue at `placement`. When nothing is playing or
    /// paused, playback starts: from the first of them if there was no
    /// current song, otherwise from the entry after the current one, so
    /// that songs already waiting are not skipped. The current one is
    /// stopped only after it failed.
    ///
    /// Leaves out the songs that are skipped. Changes nothing and returns `false`
    /// when that is all of them.
    pub fn enqueue(
        &mut self,
        songs: &[Song],
        placement: Placement,
        cx: &mut Context<Self>,
    ) -> bool {
        let had_current = self.has_song();
        let kept = actions::without_skipped(songs, cx);
        if self.queue.insert(kept, placement).is_none() {
            return songs.is_empty();
        }
        self.queue_changed(cx);
        if self.state != State::Stopped {
            return true;
        }
        // Without a current song, the first one added has become it
        if !had_current || self.queue.advance().is_some() {
            self.play_current(cx);
        }
        true
    }

    /// Takes out the waiting songs that are skipped. The current one plays on
    /// and goes when it is no longer current.
    pub fn apply_skipping(&mut self, cx: &mut Context<Self>) {
        let is_current_skipped = self
            .queue
            .current()
            .is_some_and(|song| actions::is_skipped(song, cx));
        self.queue.set_current_leaving(is_current_skipped);
        let current = self.queue.current_id();
        let skipped: Vec<_> = self
            .queue
            .entries()
            .iter()
            .filter(|entry| Some(entry.id()) != current && actions::is_skipped(entry.item(), cx))
            .map(|entry| entry.id())
            .collect();
        for id in skipped {
            self.queue.remove(id);
        }
        self.queue_changed(cx);
    }

    fn remove_left(&mut self, cx: &mut Context<Self>) {
        if self.queue.remove_left() {
            self.queue_changed(cx);
        }
    }

    pub fn is_shuffled(&self) -> bool {
        self.queue.is_shuffled()
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        self.queue.set_shuffled(!self.queue.is_shuffled());
        self.queue_changed(cx);
        cx.emit(PlayerEvent::ShuffleChanged);
    }

    pub fn repeat(&self) -> RepeatMode {
        self.queue.repeat()
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode, cx: &mut Context<Self>) {
        self.queue.set_repeat(repeat);
        self.prepare_following(cx);
        cx.emit(PlayerEvent::RepeatChanged);
        cx.notify();
    }

    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        let repeat = match self.repeat() {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        self.set_repeat(repeat, cx);
    }

    pub(crate) fn queue(&self) -> &Queue<Song> {
        &self.queue
    }

    /// Whether the current song is playing or about to, rather than paused
    /// or stopped.
    pub(crate) fn is_playing(&self) -> bool {
        match self.state {
            State::Playing => true,
            State::Opening => !self.resume.is_some_and(|resume| resume.paused),
            State::Paused | State::Stopped => false,
        }
    }

    pub(crate) fn is_buffering(&self) -> bool {
        self.buffering && self.is_playing()
    }

    /// Asks `panels` how they stand whenever the toggles are drawn, as that can
    /// follow from the window's size.
    pub fn set_panels(&mut self, panels: impl Fn(&Window, &App) -> Panels + 'static) {
        self.panels = Box::new(panels);
    }

    pub(crate) fn panels(&self, window: &Window, cx: &App) -> Panels {
        (self.panels)(window, cx)
    }

    pub(crate) fn set_cover_hidden(&mut self, is_hidden: bool, cx: &mut Context<Self>) {
        self.is_cover_hidden = is_hidden;
        cx.notify();
    }

    pub(crate) fn current_song(&self) -> Option<&Song> {
        self.queue.current()
    }

    pub(crate) fn cover_urls(&self) -> (Option<&SharedString>, Option<&SharedString>) {
        (self.cover_url.as_ref(), self.large_cover_url.as_ref())
    }

    pub(crate) fn format_text(&self) -> Option<String> {
        self.format
            .as_ref()
            .map(|format| describe_format(format, self.queue.current().and_then(Song::suffix)))
    }

    pub(crate) fn position_state(&self) -> &Entity<SliderState> {
        &self.position
    }

    pub(crate) fn mini_position_state(&self) -> &Entity<SliderState> {
        &self.mini_position
    }

    pub(crate) fn volume_state(&self) -> &Entity<SliderState> {
        &self.volume
    }

    pub(crate) fn is_muted(&self) -> bool {
        self.muted
    }

    /// The position to show, which follows a drag of the slider.
    pub(crate) fn shown_position(&self) -> Duration {
        match (self.scrub, self.duration()) {
            (Some(fraction), Some(duration)) => duration.mul_f32(fraction),
            _ => self.position_now(),
        }
    }

    pub(crate) fn play_entry(&mut self, id: EntryId, cx: &mut Context<Self>) {
        if self.queue.jump_to(id).is_some() {
            self.play_current(cx);
        }
    }

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

    pub(crate) fn move_entry_to_next(&mut self, id: EntryId, cx: &mut Context<Self>) {
        if self.queue.move_to_next(id) {
            self.queue_changed(cx);
        }
    }

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
                self.dop_start = DopStart::Settled;
                self.engine.resume();
                self.set_state(State::Playing, cx);
                self.announce_current(cx);
            }
            State::Stopped if self.resume.is_some() => {
                self.release_dop_start();
                self.rises_at_load = true;
                // Asked to play, whatever it was when it stopped
                if let Some(resume) = &mut self.resume {
                    resume.paused = false;
                }
                self.open_current(false, cx);
            }
            State::Stopped if self.has_song() => {
                self.release_dop_start();
                self.rises_at_load = true;
                self.play_current(cx);
            }
            State::Opening => {
                let resume = self.resume.get_or_insert_default();
                resume.paused = !resume.paused;
                let paused = resume.paused;
                if !paused {
                    self.release_dop_start();
                }
                // Otherwise it is yet to load, which goes by `resume`
                if self.load.is_some() {
                    if paused {
                        self.engine.pause();
                    } else {
                        self.engine.resume();
                    }
                }
                self.set_state(State::Opening, cx);
            }
            State::Stopped => {}
        }
    }

    fn release_dop_start(&mut self) {
        if self.dop_start == DopStart::Held {
            self.dop_start = DopStart::Asked;
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
        self.seek_forward_by(SEEK_STEP, window, cx);
    }

    pub fn seek_backward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.seek_backward_by(SEEK_STEP, window, cx);
    }

    fn seek_forward_by(&mut self, by: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.engine.position().saturating_add(by);
        self.seek_to(target, window, cx);
    }

    fn seek_backward_by(&mut self, by: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.engine.position().saturating_sub(by);
        self.seek_to(target, window, cx);
    }

    /// Whether the volume can only be changed on the DAC, as with DoP.
    pub(crate) fn is_volume_locked(&self) -> bool {
        self.is_volume_locked
    }

    pub fn volume_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_volume(VOLUME_STEP, window, cx);
    }

    pub fn volume_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_volume(-VOLUME_STEP, window, cx);
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        if self.is_volume_locked {
            return;
        }
        self.muted = !self.muted;
        let level = self.level(cx);
        self.engine.set_volume(gain(level, self.muted));
        cx.notify();
    }

    pub(crate) fn is_started(&self) -> bool {
        matches!(self.state, State::Playing | State::Paused)
    }

    /// The position to show: where the song is, or where it would pick up.
    pub(crate) fn position_now(&self) -> Duration {
        if self.is_started() {
            self.engine.position()
        } else {
            self.resume.unwrap_or_default().position
        }
    }

    pub(crate) fn duration(&self) -> Option<Duration> {
        self.format
            .as_ref()
            .and_then(PlaybackFormat::duration)
            .or_else(|| self.queue.current().and_then(Song::duration))
            .filter(|duration| !duration.is_zero())
    }

    /// Jumps to `target` in the current song; at or past its end, moves on
    /// to the next one.
    pub(crate) fn seek_to(
        &mut self,
        target: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.show_in_media_controls(cx);
        if self.scrub.is_none() {
            self.show_position(position, window, cx);
        }
        if position != self.refreshed_position {
            self.refreshed_position = position;
            cx.notify();
        }
    }

    /// A song opening keeps the status of the one before, so that skipping does not make
    /// it flicker.
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
        if !self.presence.is_on() {
            return;
        }
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
        let url = self.session.cover_art_url(&cover, PUBLISHED_COVER_PIXELS);
        let http = cx.http_client();
        let host = self.cover_host.clone();
        let link = cx.background_spawn(async move {
            let image = fetch_image(http, url).await?;
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

    fn show_in_media_controls(&mut self, cx: &mut Context<Self>) {
        let status = self.media_status();
        let (position, duration) = (self.position_now(), self.duration());
        self.media_controls
            .show(self.queue.current(), duration, status, position);
        self.seek_media_cover(cx);
    }

    fn media_status(&self) -> MediaStatus {
        if self.state == State::Stopped {
            MediaStatus::Stopped
        } else if self.is_playing() {
            MediaStatus::Playing
        } else {
            MediaStatus::Paused
        }
    }

    /// Finds the current song's cover for the system's media controls, calling
    /// off the search for the song before. They show the song without it if it
    /// cannot be had, so a failure is not shown.
    fn seek_media_cover(&mut self, cx: &mut Context<Self>) {
        if !self.media_controls.is_connected() {
            return;
        }
        let Some(cover) = self
            .queue
            .current()
            .and_then(Song::cover_art)
            .filter(|cover| {
                self.sought_media_cover.as_ref() != Some(*cover)
                    && !self.media_controls.has_cover(cover)
            })
            .cloned()
        else {
            return;
        };
        self.sought_media_cover = Some(cover.clone());
        let url = self.session.cover_art_url(&cover, PUBLISHED_COVER_PIXELS);
        let http = cx.http_client();
        let file = cx.background_spawn(async move {
            let image = fetch_image(http, url).await?;
            CoverFile::write(&image).ok()
        });
        self._media_cover = cx.spawn(async move |this, cx| {
            if let Some(file) = file.await {
                this.update(cx, |this, _| this.media_controls.set_cover(cover, file))
                    .ok();
            }
        });
    }

    fn on_media_command(
        &mut self,
        command: MediaCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            MediaCommand::Play if !self.is_playing() => self.toggle_playback(cx),
            MediaCommand::Pause if self.is_playing() => self.toggle_playback(cx),
            MediaCommand::Play | MediaCommand::Pause => {}
            MediaCommand::Toggle => self.toggle_playback(cx),
            // Pauses at the start rather than stopping: the player stops only
            // past the end of the queue or on a failure, and what is added to the
            // queue after that goes on from the song after the current one.
            MediaCommand::Stop => {
                if self.is_playing() {
                    self.toggle_playback(cx);
                }
                self.seek_to(Duration::ZERO, window, cx);
            }
            MediaCommand::Next => self.next(cx),
            MediaCommand::Previous => self.previous(cx),
            MediaCommand::SeekForward(by) => {
                self.seek_forward_by(by.unwrap_or(SEEK_STEP), window, cx);
            }
            MediaCommand::SeekBackward(by) => {
                self.seek_backward_by(by.unwrap_or(SEEK_STEP), window, cx);
            }
            MediaCommand::SeekTo(target) => self.seek_to(target, window, cx),
            MediaCommand::Raise => {
                cx.activate(true);
                window.activate_window();
            }
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

    fn show_position(&self, position: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let fraction = self.duration().map_or(0.0, |duration| {
            (position.as_secs_f32() / duration.as_secs_f32()).min(1.0)
        });
        for slider in [&self.position, &self.mini_position] {
            if slider.read(cx).value().end() != fraction {
                slider.update(cx, |slider, cx| slider.set_value(fraction, window, cx));
            }
        }
    }

    /// Ends a drag that closing the window cut short, which no release would
    /// otherwise end. It seeks to where it had got to if the window is still there
    /// to pass that on.
    pub(crate) fn finish_mini_position_drag(&mut self, cx: &mut Context<Self>) {
        self.mini_position
            .update(cx, |slider, cx| slider.handle_release(cx));
        self.scrub = None;
        cx.notify();
    }

    fn on_position_slider(
        &mut self,
        _: &Entity<SliderState>,
        event: &SliderEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SliderEvent::Change(value) => {
                self.scrub = Some(value.end());
                cx.notify();
            }
            SliderEvent::Release(value) => {
                self.scrub = None;
                if let Some(duration) = self.duration() {
                    self.seek_to(duration.mul_f32(value.end()), window, cx);
                }
            }
        }
    }

    pub(crate) fn level(&self, cx: &App) -> f32 {
        self.volume.read(cx).value().end()
    }

    fn apply_volume(&mut self, level: f32, cx: &mut Context<Self>) {
        // Moving the volume up is a way to unmute
        if level > 0.0 {
            self.muted = false;
        }
        self.engine.set_volume(gain(level, self.muted));
        cx.notify();
    }

    fn step_volume(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_volume_locked {
            return;
        }
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
        self.engine.stop_fading();
        self.load = None;
        self.arrival = None;
        self.following = None;
        self.format = None;
        self.buffering = false;
        self.resume = None;
        if !self.has_song() {
            self.cover_url = None;
        }
        self.set_state(State::Stopped, cx);
        self.remove_left(cx);
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
        if self.dop_start == DopStart::Held {
            self.resume.get_or_insert_default().paused = true;
        }
        let rises = std::mem::take(&mut self.rises_at_load);
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
        // Replacing the task drops an earlier open, and with it its download
        self._open = cx.spawn(async move |this, cx| {
            // Removing and opening a kept song read and write the disk
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
                        .start_paused(resume.paused)
                        .fade_in(this.dop_start != DopStart::Settled)
                        .rise_in(rises);
                    this.load = Some(this.engine.load(track));
                }
                Err(error) => this.fail(&song, &session::describe(&error), cx),
            })
            .ok();
        });
    }

    fn show_opening(&mut self, song: &Song, cx: &mut Context<Self>) {
        self.sought_media_cover = None;
        self.format = None;
        self.buffering = false;
        self.error = None;
        self.scrub = None;
        self.cover_url = song
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, COVER_PIXELS).into());
        self.large_cover_url = song.cover_art().map(|cover_art| {
            self.session
                .cover_art_url(cover_art, LARGE_COVER_PIXELS)
                .into()
        });
        self.set_state(State::Opening, cx);
        self.remove_left(cx);
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
                if self.dop_start == DopStart::Asked {
                    self.dop_start = DopStart::Settled;
                }
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
            EventKind::Buffering(buffering) => {
                self.buffering = buffering;
                cx.notify();
            }
            EventKind::Ended { next } => self.song_ended(next, cx),
            // The song fetched again plays from a download, not a kept
            // file, so a song is fetched again at most once per play.
            EventKind::Failed(error) if self.loaded_kept && error.is_track_error() => {
                self.open_current(true, cx);
            }
            // It opened a while ago, and what stopped it then may have passed
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
        self.buffering = false;
        self.error = Some(format!("Couldn't play “{}”. {reason}", song.title()).into());
        self.set_state(State::Stopped, cx);
    }

    fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        cx.emit(PlayerEvent::NowPlaying {
            song: self.queue.current().map(|song| song.id().clone()),
            playing: self.is_playing(),
        });
        self.show_in_media_controls(cx);
        cx.notify();
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(crate) fn render_song_status(&self, song: &Song, cx: &App) -> AnyElement {
        match &self.error {
            Some(error) => div()
                .debug_selector(|| "player-error".to_owned())
                .text_xs()
                .truncate()
                .text_color(cx.theme().danger)
                .child(error.clone())
                .into_any_element(),
            None => song_details(song, cx).into_any_element(),
        }
    }

    fn render_song(&self, shown: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let song = self.queue.current();
        let is_cover_hidden = self.is_cover_hidden;
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .opacity(shown)
            .when_some(song, |this, song| {
                this.child(
                    div()
                        .id("player-cover")
                        .debug_selector(|| "player-cover".to_owned())
                        .flex_none()
                        .role(Role::Button)
                        .aria_label("Lyrics")
                        .relative()
                        .when(is_cover_hidden, |this| this.opacity(0.))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(ToggleLyrics), cx);
                        })
                        .child(CoverArt::new(self.cover_url.clone()).small())
                        .child(cover_hover("player-cover-hover", cx.theme().radius, cx)),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .child(SharedString::from(song.title().to_owned())),
                        )
                        .child(self.render_song_status(song, cx)),
                )
                .child(
                    div()
                        .debug_selector(|| "player-favorite".to_owned())
                        .flex_none()
                        .child(
                            ui::favorite_button(
                                "player-favorite",
                                Favorite::Song(song.clone()),
                                cx,
                            )
                            .small(),
                        ),
                )
            })
    }
}

pub(crate) fn cover_hover(
    id: &'static str,
    radius: gpui_kit::Pixels,
    cx: &App,
) -> impl IntoElement + use<> {
    let hovered = cx.theme().foreground.opacity(COVER_HOVER_OPACITY);
    let pressed = cx.theme().foreground.opacity(COVER_PRESS_OPACITY);
    div()
        .id(id)
        .absolute()
        .inset_0()
        .rounded(radius)
        .hover(move |this| this.bg(hovered))
        .active(move |this| this.bg(pressed))
}

fn song_details(song: &Song, cx: &App) -> impl IntoElement + use<> {
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

/// Eases from 0 to 1 when `shown` turns true, and back when it turns false. The
/// first frame takes its place at once, so a bar that opens with a song holds
/// still.
fn reveal(id: &'static str, shown: bool, window: &mut Window, cx: &mut Context<Player>) -> f32 {
    let motion = cx.theme().motion_tokens();
    let policy = Transition::new(motion.duration_slow).easing(motion.easing_move.clone());
    transition(id, if shown { 1. } else { 0. }, policy, window, cx)
}

async fn fetch_image(http: Arc<dyn HttpClient>, url: Arc<str>) -> Option<Vec<u8>> {
    let mut response = http
        .get(url.as_ref(), AsyncBody::empty(), true)
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut image = Vec::new();
    response.body_mut().read_to_end(&mut image).await.ok()?;
    Some(image)
}

fn song_cache(cx: &Context<Player>) -> Option<playback::SongCache> {
    cx.try_global::<GlobalSongCache>()
        .and_then(|song_cache| song_cache.0.clone())
}

/// The engine gain for a volume slider at `level`. Squared, so equal slider
/// steps sound like roughly equal changes in loudness.
fn gain(level: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { level * level }
}

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
        audio::Error::UnsupportedDopRate => {
            "The output device can't run at this DSD rate. Choose another in Settings.".to_owned()
        }
        audio::Error::DeviceHeld => {
            "Another app is holding the output device. Close that app and press Play.".to_owned()
        }
        audio::Error::ExclusiveNotAllowed => {
            "Windows doesn't allow exclusive control of the output device. Turn it on in the device's advanced Sound settings, or turn off Exclusive mode."
                .to_owned()
        }
        audio::Error::ExclusiveOutput(_) => {
            "hiphile couldn't take the output device for itself. Press Play to try again."
                .to_owned()
        }
        audio::Error::DsdUnstable => {
            "DSD output stopped because the modulator became unstable. Lower Noise gain or Input level in Settings, then press Play."
                .to_owned()
        }
        audio::Error::Output(_) => "The audio output stopped. Press Play to try again.".to_owned(),
        audio::Error::UnsupportedFormat(_) => "hiphile can't play this file format.".to_owned(),
        audio::Error::Decode(_) | audio::Error::Read(_) => {
            "The file couldn't be read. Press Play to try again.".to_owned()
        }
    }
}

impl Render for Player {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let format_text = self.format_text();
        let muted_foreground = cx.theme().muted_foreground;
        let song_shown = reveal("player-song-shown", self.has_song(), window, cx);
        let format_shown = reveal("player-format-shown", format_text.is_some(), window, cx);
        h_flex()
            .debug_selector(|| "player-bar".to_owned())
            .flex_none()
            .h_16()
            .px_4()
            .gap_6()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(self.render_song(song_shown, cx))
            .child(
                v_flex()
                    .flex_basis(relative(0.4))
                    .min_w_80()
                    .max_w(rems(35.))
                    .items_center()
                    .child(TransportControls::new(cx.entity()))
                    .child(PositionLine::in_bar(cx.entity(), song_shown)),
            )
            .child(
                // Never narrower than its contents: it is the song's lane that
                // gives way, as its text truncates. The volume keeps the edge,
                // so the format coming and going with each song never moves it.
                h_flex()
                    .flex_1()
                    .justify_end()
                    .gap_4()
                    .text_xs()
                    .text_color(muted_foreground)
                    .when_some(format_text, |this, text| {
                        this.child(div().flex_none().opacity(format_shown).child(text))
                    })
                    .child(PanelToggles::new(cx.entity()))
                    .child(VolumeControl::new(cx.entity())),
            )
    }
}

#[cfg(test)]
impl Player {
    pub(crate) fn fill_queue(&mut self, songs: &[Song], current_ix: usize) {
        self.queue.replace(songs.to_vec(), current_ix);
    }

    pub(crate) fn place_at(&mut self, position: Duration) {
        self.resume = Some(Resume {
            position,
            paused: true,
        });
    }

    pub(crate) fn pretend_playing(&mut self, playing: bool, cx: &mut Context<Self>) {
        let state = if playing {
            State::Playing
        } else {
            State::Paused
        };
        self.set_state(state, cx);
    }

    pub(crate) fn is_cover_hidden(&self) -> bool {
        self.is_cover_hidden
    }

    pub(crate) fn is_scrubbing(&self) -> bool {
        self.scrub.is_some()
    }

    pub(crate) fn set_error(&mut self, error: &'static str) {
        self.error = Some(error.into());
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use gpui_kit::{TestAppContext, VisualTestContext};
    use serde_json::json;
    use subsonic::Credentials;

    use super::*;

    const RATE: u32 = 44_100;
    const WAIT: Duration = Duration::from_secs(15);

    struct Details(Song);

    impl gpui_kit::Render for Details {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(song_details(&self.0, cx))
        }
    }

    /// A player that plays nothing, so nothing reaches a device or the server.
    fn idle_player(cx: &mut TestAppContext) -> (Entity<Player>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let net = net::Client::new("hiphile-test").unwrap();
        let server = client::Server::new("http://127.0.0.1:9", Credentials::api_key("k")).unwrap();
        let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
        let session = session::Session::new(
            client::Client::new(net, server),
            "alice".to_owned(),
            info,
            None,
            None,
        );
        let (player, cx) = cx.add_window_view(|window, cx| {
            Player::new(session, OutputSettings::default(), window, cx)
        });
        cx.run_until_parked();
        (player, cx)
    }

    fn position_height(cx: &mut VisualTestContext) -> gpui_kit::Pixels {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.debug_bounds("player-position").unwrap().size.height
    }

    /// How far the middle of the controls is above the middle of the bar's
    /// content, which starts under the bar's one-pixel top border.
    fn controls_lift(cx: &mut VisualTestContext) -> gpui_kit::Pixels {
        let bar = cx.debug_bounds("player-bar").unwrap();
        let controls = cx.debug_bounds("player-controls").unwrap();
        bar.center().y + gpui_kit::px(0.5) - controls.center().y
    }

    #[gpui_kit::test]
    fn the_bars_star_follows_the_song_and_the_favorites(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        let song = named_song("a");
        let asked = Rc::new(RefCell::new(Vec::new()));
        let notified = Rc::new(Cell::new(0));
        cx.update(|window, cx| {
            actions::set_known_favorites(Some(HashSet::new()), cx);
            let asked = asked.clone();
            cx.on_action(move |action: &actions::SetFavorite, _| {
                asked.borrow_mut().push(action.clone());
            });
            let notified = notified.clone();
            cx.observe(&player, move |_, _| notified.set(notified.get() + 1))
                .detach();
            player.update(cx, |player, _| {
                player.fill_queue(std::slice::from_ref(&song), 0);
            });
            window.refresh();
        });
        cx.run_until_parked();

        let star = cx.debug_bounds("player-favorite").unwrap();
        let cover = cx.debug_bounds("player-cover").unwrap();
        assert!(star.left() > cover.right());
        cx.simulate_click(star.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        let favorite = Favorite::Song(song);
        assert_eq!(
            asked.take(),
            [actions::SetFavorite::new(favorite.clone(), true)]
        );

        let before = notified.get();
        cx.update(|_, cx| {
            actions::set_known_favorites(Some(HashSet::from([favorite.target()])), cx);
        });
        cx.run_until_parked();
        assert!(notified.get() > before);
    }

    #[gpui_kit::test]
    fn play_and_pause_from_the_system_only_change_what_they_name(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        let song: Song = serde_json::from_value(json!({ "id": "a", "title": "a" })).unwrap();
        let send = |command, cx: &mut VisualTestContext| {
            cx.update(|window, cx| {
                player.update(cx, |player, cx| {
                    player.on_media_command(command, window, cx);
                });
            });
        };
        let is_playing = |cx: &mut VisualTestContext| player.read_with(cx, |p, _| p.is_playing());
        player.update(cx, |player, cx| {
            player.fill_queue(std::slice::from_ref(&song), 0);
            player.pretend_playing(true, cx);
        });

        send(MediaCommand::Play, cx);
        assert!(is_playing(cx));
        send(MediaCommand::Pause, cx);
        assert!(!is_playing(cx));
        send(MediaCommand::Pause, cx);
        assert!(!is_playing(cx));
        send(MediaCommand::Play, cx);
        assert!(is_playing(cx));
        send(MediaCommand::Toggle, cx);
        assert!(!is_playing(cx));
    }

    fn named_song(id: &str) -> Song {
        serde_json::from_value(json!({ "id": id, "title": id, "duration": 100 })).unwrap()
    }

    fn send_media_command(
        player: &Entity<Player>,
        command: MediaCommand,
        cx: &mut VisualTestContext,
    ) {
        cx.update(|window, cx| {
            player.update(cx, |player, cx| {
                player.on_media_command(command, window, cx);
            });
        });
    }

    fn current_title(player: &Entity<Player>, cx: &mut VisualTestContext) -> Option<String> {
        player.read_with(cx, |player, _| {
            player.current_song().map(|song| song.title().to_owned())
        })
    }

    #[gpui_kit::test]
    fn next_and_previous_from_the_system_move_in_the_queue(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        player.update(cx, |player, _| {
            player.fill_queue(&[named_song("a"), named_song("b")], 0);
        });
        send_media_command(&player, MediaCommand::Next, cx);
        assert_eq!(current_title(&player, cx).as_deref(), Some("b"));
        send_media_command(&player, MediaCommand::Previous, cx);
        assert_eq!(current_title(&player, cx).as_deref(), Some("a"));
    }

    #[gpui_kit::test]
    fn stop_from_the_system_keeps_the_song_for_what_is_added_next(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        player.update(cx, |player, cx| {
            player.fill_queue(&[named_song("a"), named_song("b")], 0);
            player.pretend_playing(true, cx);
        });
        send_media_command(&player, MediaCommand::Stop, cx);
        assert!(!player.read_with(cx, |player, _| player.is_playing()));

        player.update(cx, |player, cx| {
            player.enqueue(&[named_song("c")], Placement::Last, cx);
        });
        assert_eq!(current_title(&player, cx).as_deref(), Some("a"));
        assert!(!player.read_with(cx, |player, _| player.is_playing()));
    }

    fn queued_titles(player: &Entity<Player>, cx: &mut VisualTestContext) -> Vec<String> {
        player.read_with(cx, |player, _| {
            let songs = player.queued_songs();
            songs.iter().map(|song| song.title().to_owned()).collect()
        })
    }

    #[gpui_kit::test]
    fn skipped_songs_stay_out_of_the_queue(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        cx.update(|_, cx| actions::set_skip_instrumentals(true, cx));
        let inst = named_song("a [instrumental]");
        let vocal = named_song("b");
        let other = named_song("c");
        let songs = [inst.clone(), vocal.clone(), inst.clone(), other.clone()];

        player.update(cx, |player, cx| {
            player.fill_queue(std::slice::from_ref(&vocal), 0);
            player.pretend_playing(true, cx);
            assert!(player.enqueue(&songs, Placement::Last, cx));
        });
        assert_eq!(queued_titles(&player, cx), ["b", "b", "c"]);

        player.update(cx, |player, cx| {
            assert!(!player.enqueue(std::slice::from_ref(&inst), Placement::Next, cx));
            assert!(!player.play(&songs, 2, cx));
            assert!(!player.play_whole(std::slice::from_ref(&inst), cx));
        });
        assert_eq!(queued_titles(&player, cx), ["b", "b", "c"]);

        player.update(cx, |player, cx| assert!(player.play(&songs, 3, cx)));
        assert_eq!(queued_titles(&player, cx), ["b", "c"]);
        assert_eq!(current_title(&player, cx).as_deref(), Some("c"));

        player.update(cx, |player, cx| assert!(player.play_whole(&songs, cx)));
        assert_eq!(queued_titles(&player, cx), ["b", "c"]);
    }

    #[gpui_kit::test]
    fn turning_on_skipping_takes_out_what_waits_but_not_what_plays(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        player.update(cx, |player, cx| {
            player.fill_queue(
                &[
                    named_song("a"),
                    named_song("b (Off Vocal)"),
                    named_song("c [instrumental]"),
                    named_song("d"),
                    named_song("e (オリジナル・カラオケ)"),
                ],
                2,
            );
            player.pretend_playing(true, cx);
            player.apply_skipping(cx);
        });
        assert_eq!(queued_titles(&player, cx).len(), 5);

        cx.update(|_, cx| actions::set_skip_instrumentals(true, cx));
        player.update(cx, |player, cx| player.apply_skipping(cx));
        assert_eq!(queued_titles(&player, cx), ["a", "c [instrumental]", "d"]);
        assert_eq!(
            current_title(&player, cx).as_deref(),
            Some("c [instrumental]")
        );
    }

    #[gpui_kit::test]
    fn a_skipped_song_goes_when_the_player_moves_on_and_never_comes_back(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        cx.update(|_, cx| actions::set_skip_instrumentals(true, cx));
        let changes = Rc::new(Cell::new(0));
        let _changes = cx.update(|_, cx| {
            let changes = changes.clone();
            cx.subscribe(&player, move |_, event: &PlayerEvent, _| {
                if matches!(event, PlayerEvent::QueueChanged) {
                    changes.set(changes.get() + 1);
                }
            })
        });
        player.update(cx, |player, cx| {
            player.fill_queue(
                &[
                    named_song("a"),
                    named_song("b [instrumental]"),
                    named_song("c"),
                ],
                1,
            );
            player.set_repeat(RepeatMode::All, cx);
            player.pretend_playing(true, cx);
            player.apply_skipping(cx);
        });
        assert_eq!(queued_titles(&player, cx).len(), 3);
        assert_eq!(changes.get(), 1);

        player.update(cx, |player, cx| player.next(cx));
        assert_eq!(current_title(&player, cx).as_deref(), Some("c"));
        assert_eq!(queued_titles(&player, cx), ["a", "c"]);
        assert_eq!(changes.get(), 2);

        player.update(cx, |player, cx| player.next(cx));
        assert_eq!(current_title(&player, cx).as_deref(), Some("a"));
        assert_eq!(queued_titles(&player, cx), ["a", "c"]);
    }

    #[gpui_kit::test]
    fn turning_off_skipping_keeps_the_current_song(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        cx.update(|_, cx| actions::set_skip_instrumentals(true, cx));
        player.update(cx, |player, cx| {
            player.fill_queue(&[named_song("a [instrumental]"), named_song("b")], 0);
            player.pretend_playing(true, cx);
            player.apply_skipping(cx);
        });
        cx.update(|_, cx| actions::set_skip_instrumentals(false, cx));
        player.update(cx, |player, cx| {
            player.apply_skipping(cx);
            player.next(cx);
        });
        assert_eq!(queued_titles(&player, cx), ["a [instrumental]", "b"]);
    }

    #[gpui_kit::test]
    fn seeks_from_the_system_move_the_position(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        player.update(cx, |player, cx| {
            player.fill_queue(&[named_song("a"), named_song("b")], 0);
            player.pretend_playing(true, cx);
        });
        let shown = |cx: &mut VisualTestContext| {
            player.read_with(cx, |player, cx| {
                player.position_state().read(cx).value().end()
            })
        };
        let seconds = Duration::from_secs;

        send_media_command(&player, MediaCommand::SeekTo(seconds(50)), cx);
        assert_eq!(shown(cx), 0.5);
        let in_mini_player = player.read_with(cx, |player, cx| {
            player.mini_position_state().read(cx).value().end()
        });
        assert_eq!(in_mini_player, 0.5);
        send_media_command(&player, MediaCommand::SeekForward(None), cx);
        assert_eq!(shown(cx), 0.1);
        send_media_command(&player, MediaCommand::SeekForward(Some(seconds(20))), cx);
        assert_eq!(shown(cx), 0.2);
        send_media_command(&player, MediaCommand::SeekBackward(None), cx);
        assert_eq!(shown(cx), 0.0);

        send_media_command(&player, MediaCommand::SeekTo(seconds(100)), cx);
        assert_eq!(current_title(&player, cx).as_deref(), Some("b"));
    }

    #[gpui_kit::test]
    fn the_system_hears_how_the_song_stands(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        player.update(cx, |player, cx| {
            player.fill_queue(&[named_song("a")], 0);
            assert_eq!(player.media_status(), MediaStatus::Stopped);

            player.pretend_playing(true, cx);
            assert_eq!(player.media_status(), MediaStatus::Playing);
            player.pretend_playing(false, cx);
            assert_eq!(player.media_status(), MediaStatus::Paused);

            // Opening is playing, unless a pause has come in the meantime
            player.state = State::Opening;
            assert_eq!(player.media_status(), MediaStatus::Playing);
            player.resume = Some(Resume {
                position: Duration::ZERO,
                paused: true,
            });
            assert_eq!(player.media_status(), MediaStatus::Paused);
        });
    }

    #[gpui_kit::test]
    fn a_cover_not_found_is_looked_for_again_with_the_next_song(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        player.update(cx, |player, cx| {
            player.fill_queue(&[named_song("a"), named_song("b")], 0);
            player.sought_media_cover = Some(CoverArtId::new("c"));
            player.next(cx);
            assert_eq!(player.sought_media_cover, None);
        });
    }

    #[gpui_kit::test]
    fn the_controls_ease_up_from_the_middle_as_a_song_arrives(cx: &mut TestAppContext) {
        let (player, cx) = idle_player(cx);
        assert_eq!(position_height(cx), gpui_kit::px(0.));
        assert_eq!(controls_lift(cx), gpui_kit::px(0.));

        let song: Song = serde_json::from_value(json!({ "id": "a", "title": "a" })).unwrap();
        player.update(cx, |player, cx| {
            player.fill_queue(&[song], 0);
            cx.notify();
        });
        assert_eq!(position_height(cx), gpui_kit::px(0.));

        let (duration, rem) =
            cx.update(|window, cx| (cx.theme().motion_tokens().duration_slow, window.rem_size()));
        cx.executor().advance_clock(duration / 4);
        let partway = position_height(cx);
        assert!(gpui_kit::px(0.) < partway && partway < rem);
        // Layout rounds to whole pixels, so halfway through it can be half off
        assert!((controls_lift(cx) - partway / 2.).abs() <= gpui_kit::px(0.5));

        cx.executor().advance_clock(duration);
        assert_eq!(position_height(cx), rem);
        assert_eq!(controls_lift(cx), rem / 2.);
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
            // Until the client has gone, so that it decides when a body ends
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

    /// A player whose engine ends with the test, also one that fails. The
    /// harness ends a test with `App::quit`, which on its platform does nothing,
    /// so the player's own shutdown never runs. An engine left behind goes on
    /// playing, and reopens and restores rates on the device under later tests,
    /// until no stream there gets called any more.
    struct TestPlayer {
        player: Entity<Player>,
        app: TestAppContext,
    }

    impl std::ops::Deref for TestPlayer {
        type Target = Entity<Player>;

        fn deref(&self) -> &Self::Target {
            &self.player
        }
    }

    impl Drop for TestPlayer {
        fn drop(&mut self) {
            // A test that failed while updating the player has it still leased
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.app.update(|cx| {
                    self.player.update(cx, |player, _| player.engine.shutdown());
                });
            }))
            .ok();
        }
    }

    fn player<'a>(
        server: &Server,
        cx: &'a mut TestAppContext,
    ) -> (TestPlayer, &'a mut VisualTestContext) {
        // The engine and the network wake the test from their own threads
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
        let app = cx.clone();
        let (player, cx) =
            cx.add_window_view(|window, cx| Player::new(session, output, window, cx));
        (TestPlayer { player, app }, cx)
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

    fn dop_output() -> OutputSettings {
        let device = player_device();
        OutputSettings::new(Some(device), None).with_dop(Some(audio::DsdSettings::default()))
    }

    fn player_device() -> String {
        let name = std::env::var("HIPHILE_TEST_DEVICE")
            .expect("name a virtual device in HIPHILE_TEST_DEVICE");
        audio::output_devices()
            .unwrap()
            .into_iter()
            .find(|device| device.name().trim() == name)
            .and_then(|device| device.id().map(str::to_owned))
            .expect("no such device")
    }

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn turning_dop_on_pauses_a_playing_song_and_locks_the_volume(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(5_000))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "a to play", |player| {
            is_playing_song(player, "a")
        });

        player.update(cx, |player, cx| {
            let output = dop_output();
            player.set_output(output, cx);
        });
        assert_eq!(
            player.read_with(cx, |player, _| player.state),
            State::Paused
        );
        assert!(player.read_with(cx, |player, _| player.is_volume_locked()));

        player.update_in(cx, |player, window, cx| {
            let before = player.level(cx);
            player.volume_down(window, cx);
            player.toggle_mute(cx);
            assert!((player.level(cx) - before).abs() < f32::EPSILON);
            assert!(!player.is_muted());
        });

        player.update(cx, |player, cx| {
            let device = player_device();
            player.set_output(OutputSettings::new(Some(device), None), cx);
        });
        assert!(!player.read_with(cx, |player, _| player.is_volume_locked()));
    }

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn after_turning_dop_on_songs_wait_for_play(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(5_000)), server.add("b", wav(5_000))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.set_output(dop_output(), cx));
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        assert!(!player.read_with(cx, |player, _| player.is_playing()));
        wait_until(&player, cx, "a to wait", |player| {
            current(player) == Some("a") && player.state == State::Paused
        });

        player.update(cx, |player, cx| player.next(cx));
        wait_until(&player, cx, "b to wait", |player| {
            current(player) == Some("b") && player.state == State::Paused
        });

        player.update(cx, |player, cx| player.toggle_playback(cx));
        assert_eq!(
            player.read_with(cx, |player, _| player.state),
            State::Playing
        );
        assert_eq!(
            player.read_with(cx, |player, _| player.dop_start),
            DopStart::Settled
        );

        player.update(cx, |player, cx| player.previous(cx));
        wait_until(&player, cx, "a to play", |player| {
            is_playing_song(player, "a")
        });
    }

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn turning_dop_on_while_a_song_opens_makes_it_wait(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(5_000))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| {
            player.play(&songs, 0, cx);
            player.set_output(dop_output(), cx);
        });
        wait_until(&player, cx, "a to wait", |player| {
            current(player) == Some("a") && player.state == State::Paused
        });
    }

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn play_goes_straight_on_with_nothing_open_after_turning_dop_on(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(5_000))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| {
            player.fill_queue(&songs, 0);
            player.set_output(dop_output(), cx);
            player.toggle_playback(cx);
        });
        wait_until(&player, cx, "a to play", |player| {
            is_playing_song(player, "a")
        });
        assert_eq!(
            player.read_with(cx, |player, _| player.dop_start),
            DopStart::Settled
        );
    }

    #[test]
    fn a_held_device_is_told_apart_from_another_failure() {
        assert!(describe(&audio::Error::DeviceHeld).contains("Another app"));
        assert!(!describe(&audio::Error::ExclusiveOutput(String::new())).contains("Another app"));
        assert!(describe(&audio::Error::ExclusiveNotAllowed).contains("Exclusive mode"));
    }

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

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn a_crossfade_shows_the_next_song_as_it_starts_fading_in(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(2_000)), server.add("b", wav(5_000))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| {
            player.set_transitions(Transitions {
                trim_silence: false,
                crossfade: Duration::from_secs(1),
                fades_play_and_pause: false,
            });
            player.play(&songs, 0, cx);
        });
        wait_until(&player, cx, "a to play", |player| {
            is_playing_song(player, "a")
        });
        let started = std::time::Instant::now();
        wait_until(&player, cx, "b to play", |player| {
            is_playing_song(player, "b")
        });
        let played = started.elapsed();
        assert!(played < Duration::from_millis(1_500), "{played:?}");
        assert!(played > Duration::from_millis(500), "{played:?}");
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
        // Within what the engine writes ahead of the end of the song
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

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn repeating_one_song_plays_it_until_repeat_is_off(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(300)), server.add("b", wav(300))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| {
            player.set_repeat(RepeatMode::One, cx);
            player.play(&songs, 0, cx);
        });
        wait_until(&player, cx, "a to come round again", |_| {
            server.streamed().iter().filter(|id| *id == "a").count() >= 3
        });
        player.read_with(cx, |player, _| assert_eq!(current(player), Some("a")));
        assert!(!server.streamed().contains(&"b".to_owned()));

        player.update(cx, |player, cx| player.set_repeat(RepeatMode::Off, cx));
        wait_until(&player, cx, "the end", |player| {
            player.state == State::Stopped
        });
        player.read_with(cx, |player, _| {
            assert_eq!(current(player), None);
            assert!(player.error.is_none());
        });
    }

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn repeating_all_songs_goes_back_to_the_first(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(1_500)), server.add("b", wav(1_500))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 1, cx));
        wait_until(&player, cx, "b to play", |player| {
            is_playing_song(player, "b")
        });

        player.update(cx, |player, cx| player.set_repeat(RepeatMode::All, cx));
        wait_until(&player, cx, "a to be set", |player| {
            is_following_set(player, "a")
        });
        wait_until(&player, cx, "a to play", |player| {
            is_playing_song(player, "a")
        });
    }

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn shuffling_while_a_song_plays_changes_what_follows_it(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs: Vec<Song> = (0..30)
            .map(|ix| server.add(&format!("s{ix}"), wav(1_000)))
            .collect();
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "s1 to be set", |player| {
            is_following_set(player, "s1")
        });

        let following = |player: &Player| {
            player
                .queue
                .following()
                .map(|song| song.id().as_str().to_owned())
        };
        // One time in 29, the shuffle leaves s1 next
        let next = loop {
            player.update(cx, |player, cx| player.toggle_shuffle(cx));
            let next = player.read_with(cx, |player, _| following(player)).unwrap();
            if next != "s1" {
                break next;
            }
            player.update(cx, |player, cx| player.toggle_shuffle(cx));
        };
        wait_until(&player, cx, "the shuffled song to be set", |player| {
            is_following_set(player, &next)
        });
        wait_until(&player, cx, "the shuffled song to play", |player| {
            is_playing_song(player, &next)
        });
    }

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

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn a_pause_while_a_song_opens_holds(cx: &mut TestAppContext) {
        let server = Server::default();
        let songs = [server.add("a", wav(5_000))];
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| {
            player.play(&songs, 0, cx);
            assert_eq!(player.state, State::Opening);
            assert!(player.is_playing());
            player.toggle_playback(cx);
            assert_eq!(player.state, State::Opening);
            assert!(!player.is_playing());
        });
        wait_until(&player, cx, "the song to start", |player| {
            player.is_started()
        });
        player.update(cx, |player, cx| {
            assert_eq!(player.state, State::Paused);
            player.toggle_playback(cx);
            assert_eq!(player.state, State::Playing);
        });
        player.update(cx, |player, cx| player.stop(cx));
    }

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn a_pause_does_not_outlast_a_song_that_failed_to_open(cx: &mut TestAppContext) {
        let server = Server::default();
        let song = server.add("a", wav(1_000));
        let body = server.songs.lock().unwrap().remove("a").unwrap();
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| {
            player.play(std::slice::from_ref(&song), 0, cx);
            player.toggle_playback(cx);
        });
        wait_until(&player, cx, "the failure", |player| player.error.is_some());

        server.songs.lock().unwrap().insert("a".to_owned(), body);
        player.update(cx, |player, cx| player.toggle_playback(cx));
        wait_until(&player, cx, "the song to start", |player| {
            player.is_started()
        });
        player.read_with(cx, |player, _| assert_eq!(player.state, State::Playing));
        player.update(cx, |player, cx| player.stop(cx));
    }

    #[gpui_kit::test]
    #[ignore = "needs a virtual output device named in HIPHILE_TEST_DEVICE"]
    fn a_song_waiting_for_its_data_answers_the_controls(cx: &mut TestAppContext) {
        let server = Server::default();
        // Half of it arrives, which sounds before it runs out
        let songs = [server.add("a", wav(6_000))];
        *server.held.lock().unwrap() = Some("a".to_owned());
        let (player, cx) = player(&server, cx);
        player.update(cx, |player, cx| player.play(&songs, 0, cx));
        wait_until(&player, cx, "the song to wait", |player| {
            player.is_started() && player.is_buffering()
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(cx.debug_bounds("player-buffering").is_some());

        player.update(cx, |player, cx| {
            player.toggle_playback(cx);
            assert_eq!(player.state, State::Paused);
            assert!(!player.is_buffering());
        });
        wait_until(&player, cx, "the wait to stop counting", |player| {
            !player.buffering
        });
        player.update(cx, |player, cx| {
            player.toggle_playback(cx);
            assert_eq!(player.state, State::Playing);
        });
        wait_until(&player, cx, "the song to wait again", |player| {
            player.is_buffering()
        });

        server.released.store(true, Ordering::SeqCst);
        wait_until(&player, cx, "the song to sound", |player| {
            !player.is_buffering()
        });
        player.update(cx, |player, cx| player.stop(cx));
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

        // Without running the player, so that the events wait for it
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
