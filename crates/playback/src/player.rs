use std::mem;
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime};

use audio::{Engine, Event, EventKind, LoadId, OutputSettings, PlaybackFormat, Transitions};
use executor::{Executor, Publish, Published, Saver, Task};
use futures::executor::block_on;
use subsonic::Song;
use tokio::sync::{broadcast, watch};

use crate::{
    Arrival, EntryId, Placement, PlayTally, Queue, RepeatMode, SongCache, describe_format,
    is_skipped, without_skipped,
};

/// Within this much of a song's start, Previous goes to the previous song;
/// later, it restarts the current one.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
const SEEK_STEP: Duration = Duration::from_secs(10);
const VOLUME_STEP: f32 = 0.1;
/// Where unmuting at `0.0` goes with no level above it known: halfway, so that it is heard
/// without being loud at a DAC turned up.
const AUDIBLE_VOLUME: f32 = 0.5;
/// How often a song playing is looked at, to tell when it has been listened to enough to count
/// as played.
const TALLY_INTERVAL: Duration = Duration::from_millis(250);
const NOTICES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerNotice {
    /// A play of a song has been reported to the server, as far as it could
    /// be, so what the server says was played last may have changed.
    Played,
    /// Songs to play or queue were left out, as instrumentals are skipped,
    /// and nothing changed.
    Skipped,
}

/// What the player shows of itself. The position is not in it: it changes all the time, so it is
/// read with [`Player::position`] while it is shown.
#[derive(Clone, Debug)]
pub struct PlayerSnapshot {
    queue: Queue<Song>,
    state: State,
    /// Where the stopped song picks back up, after its output failed, or where the song opening
    /// starts.
    resume: Option<Resume>,
    /// Whether the engine says the song is silent for want of its data.
    buffering: bool,
    format: Option<PlaybackFormat>,
    error: Option<Arc<str>>,
    /// From `0.0` to `1.0`, as a volume slider shows it.
    volume: f32,
    is_muted: bool,
    is_volume_locked: bool,
}

impl PartialEq for PlayerSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.queue.is_same(&other.queue)
            && self.state == other.state
            && self.resume == other.resume
            && self.buffering == other.buffering
            && self.format == other.format
            && self.error == other.error
            && self.volume == other.volume
            && self.is_muted == other.is_muted
            && self.is_volume_locked == other.is_volume_locked
    }
}

impl PlayerSnapshot {
    pub fn queue(&self) -> &Queue<Song> {
        &self.queue
    }

    pub fn current_song(&self) -> Option<&Song> {
        self.queue.current()
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

    pub fn is_shuffled(&self) -> bool {
        self.queue.is_shuffled()
    }

    pub fn repeat(&self) -> RepeatMode {
        self.queue.repeat()
    }

    /// Whether the current song is playing or about to, rather than paused
    /// or stopped.
    pub fn is_playing(&self) -> bool {
        match self.state {
            State::Playing => true,
            State::Opening => !self.resume.is_some_and(|resume| resume.paused),
            State::Paused | State::Stopped => false,
        }
    }

    /// Whether the song has started, playing or paused, rather than opening or stopped.
    pub fn is_started(&self) -> bool {
        matches!(self.state, State::Playing | State::Paused)
    }

    pub fn is_stopped(&self) -> bool {
        self.state == State::Stopped
    }

    pub fn is_buffering(&self) -> bool {
        self.buffering && self.is_playing()
    }

    pub fn duration(&self) -> Option<Duration> {
        self.format
            .as_ref()
            .and_then(PlaybackFormat::duration)
            .or_else(|| self.queue.current().and_then(Song::duration))
            .filter(|duration| !duration.is_zero())
    }

    /// What the song plays as, once it has started.
    pub fn format(&self) -> Option<&PlaybackFormat> {
        self.format.as_ref()
    }

    /// What the song plays as, once it has started.
    pub fn format_text(&self) -> Option<String> {
        self.format
            .as_ref()
            .map(|format| describe_format(format, self.queue.current().and_then(Song::suffix)))
    }

    /// Why the current song stopped, which Play tries again.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// From `0.0` to `1.0`, as a volume slider shows it, also while muted.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted
    }

    /// Whether the volume can only be changed on the DAC, as with DoP.
    pub fn is_volume_locked(&self) -> bool {
        self.is_volume_locked
    }

    /// Whether nothing is heard, muted or at `0.0`, which [`Player::toggle_mute`] then ends.
    pub fn is_silent(&self) -> bool {
        self.is_muted || self.volume == 0.0
    }

    /// Whether [`Player::seek_to`] can move the song.
    pub fn is_seekable(&self) -> bool {
        self.is_started() && self.duration().is_some()
    }
}

/// Plays the queue through the audio engine. Every command changes the snapshot before it
/// returns. Dropping the last handle stops playback and returns the output device to its sample
/// rate from before, without waiting; [`Player::shutdown`] waits.
#[derive(Clone)]
pub struct Player(Arc<Inner>);

struct WeakPlayer(Weak<Inner>);

struct Inner {
    executor: Executor,
    session: session::Session,
    core: Published<Core>,
    notices: broadcast::Sender<PlayerNotice>,
    /// `None` without a home directory: the volume then lasts until quitting.
    volume_saver: Option<Saver<f32>>,
    store: Option<config::Store>,
}

struct Core {
    shown: PlayerSnapshot,
    /// Before the engine, so that it is dropped first: the engine's thread closing the events
    /// would otherwise wake it from that thread, which a test's scheduler takes for a failure.
    _events: Task<()>,
    engine: Engine,
    song_cache: Option<SongCache>,
    is_skipping: bool,
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
    /// Counts the entries got ready to follow, so that one replaced while it was opening on
    /// another thread cannot be set to follow after its successor.
    following_id: u64,
    /// Counts the opens, so that one replaced while it was finishing on another thread cannot
    /// load after its successor.
    open_id: u64,
    dop_start: DopStart,
    /// Play was asked for with nothing loaded, so the next load rises from silence
    rises_at_load: bool,
    tally: PlayTally,
    /// The last level kept above `0.0`, which unmuting at `0.0` goes back to.
    audible: f32,
    /// Looks at the song while it plays, for the tally.
    _tally: Option<Task<()>>,
    _open: Task<()>,
    _announce: Task<()>,
}

/// The queue's next entry, whose download and decoder are started ahead of
/// its turn. Dropping it stops getting it ready; a load the engine already has
/// is left for the owner to take back.
struct Following {
    id: u64,
    entry: EntryId,
    /// Once it is open and set to follow in the engine. Stays `None` if that
    /// fails: its turn comes the usual way, and shows why then.
    load: Option<LoadId>,
    kept: bool,
    arrival: Option<Arrival>,
    _open: Task<()>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

impl Player {
    /// Plays on `output`, keeping what it downloads in `song_cache`, at the volume saved in
    /// `store`.
    pub fn new(
        executor: Executor,
        session: session::Session,
        store: Result<config::Store, config::Error>,
        output: OutputSettings,
        song_cache: Option<SongCache>,
    ) -> Self {
        let store = store.ok();
        // A volume that cannot be read starts at full, and only the next launch would show
        // another, so it is not shown
        let volume = store
            .as_ref()
            .and_then(|store| store.volume().ok().flatten())
            .unwrap_or(1.0);
        let (engine, events) = Engine::spawn();
        let is_volume_locked = output.dop().is_some();
        engine.set_output(output);
        engine.set_volume(gain(volume, false));
        let volume_saver = store.clone().map(|store| {
            Saver::new(executor.clone(), move |volume| {
                store.save_volume(volume).ok();
            })
        });
        let shown = PlayerSnapshot {
            queue: Queue::new(),
            state: State::Stopped,
            resume: None,
            buffering: false,
            format: None,
            error: None,
            volume,
            is_muted: false,
            is_volume_locked,
        };
        let this = Self(Arc::new(Inner {
            executor,
            session,
            core: Published::new(Core {
                shown,
                engine,
                song_cache,
                is_skipping: false,
                load: None,
                loaded_kept: false,
                loaded_ahead: false,
                arrival: None,
                following: None,
                following_id: 0,
                open_id: 0,
                dop_start: DopStart::default(),
                rises_at_load: false,
                tally: PlayTally::default(),
                audible: if volume > 0.0 { volume } else { AUDIBLE_VOLUME },
                _tally: None,
                _open: Task::ready(()),
                _announce: Task::ready(()),
                _events: Task::ready(()),
            }),
            notices: broadcast::Sender::new(NOTICES),
            volume_saver,
            store,
        }));
        let weak = this.downgrade();
        let events = this.0.executor.spawn(async move {
            while let Ok(event) = events.recv().await {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                this.change(|core| core.on_engine_event(&this, event));
            }
        });
        this.0.core.update(|core| core._events = events);
        this.submit_scrobbles();
        this
    }

    /// Sends the plays from before that the server has yet to hear of.
    fn submit_scrobbles(&self) {
        let Some(scrobbles) = self.0.session.scrobbles().cloned() else {
            return;
        };
        let client = self.0.session.client().clone();
        let notices = self.0.notices.clone();
        // Keeping the plays reads and writes the disk
        let sent = self
            .0
            .executor
            .spawn_blocking(move || block_on(scrobbles.submit(&client)));
        self.0
            .executor
            .spawn(async move {
                if matches!(sent.await, Ok(1..)) {
                    notices.send(PlayerNotice::Played).ok();
                }
            })
            .detach();
    }

    pub fn snapshot(&self) -> PlayerSnapshot {
        self.0.core.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling the player, which
    /// publishes under its own lock.
    pub fn subscribe(&self) -> watch::Receiver<PlayerSnapshot> {
        self.0.core.subscribe()
    }

    /// Hears each notice from now on.
    pub fn notices(&self) -> broadcast::Receiver<PlayerNotice> {
        self.0.notices.subscribe()
    }

    /// Where the song is, or where it would pick up.
    pub fn position(&self) -> Duration {
        self.0.core.read(Core::position)
    }

    /// Plays on `output` from now on, moving a song already playing there and
    /// picking up one that the last output stopped.
    ///
    /// Turning DoP on holds back the sound until Play: see [`DopStart`].
    pub fn set_output(&self, output: OutputSettings) {
        self.change(|core| core.set_output(self, output));
    }

    pub fn set_transitions(&self, transitions: Transitions) {
        self.0
            .core
            .read(|core| core.engine.set_transitions(transitions));
    }

    /// Keeps at most what `song_cache` holds of the songs downloaded from now on.
    pub fn set_song_cache(&self, song_cache: Option<SongCache>) {
        self.change(|core| core.song_cache = song_cache);
    }

    /// Takes out the waiting songs that are skipped from now on. The current one plays on and goes
    /// when it is no longer current.
    pub fn set_skip_instrumentals(&self, is_on: bool) {
        self.change(|core| {
            core.is_skipping = is_on;
            core.apply_skipping();
        });
    }

    /// Plays at `volume`, from `0.0` to `1.0`, without saving it, as while a slider is dragged.
    /// Any volume above `0.0` unmutes. Changes nothing while the volume is locked.
    pub fn set_volume(&self, volume: f32) {
        self.0.core.update(|core| core.set_volume(volume));
    }

    /// Remembers the volume for the next launch. A failure only means the next launch starts at
    /// the old volume, so it is not shown.
    pub fn save_volume(&self) {
        self.0.core.update(|core| {
            // Under the lock, so that the levels reach the saver in the order they were set
            if let Some(saver) = &self.0.volume_saver {
                saver.save(core.shown.volume);
            }
            if core.shown.volume > 0.0 {
                core.audible = core.shown.volume;
            }
        });
    }

    pub fn volume_up(&self) {
        self.step_volume(VOLUME_STEP);
    }

    pub fn volume_down(&self) {
        self.step_volume(-VOLUME_STEP);
    }

    fn step_volume(&self, by: f32) {
        if self.snapshot().is_volume_locked() {
            return;
        }
        self.0
            .core
            .update(|core| core.set_volume(core.shown.volume + by));
        self.save_volume();
    }

    /// Mutes, or makes the song heard again: at `0.0` that brings back the last level kept above
    /// it, and keeps that for the next launch.
    pub fn toggle_mute(&self) {
        let is_raised = self.0.core.update(|core| {
            if core.shown.is_volume_locked {
                return false;
            }
            if !core.shown.is_silent() {
                core.shown.is_muted = true;
                core.apply_volume();
                return false;
            }
            core.shown.is_muted = false;
            let is_raised = core.shown.volume == 0.0;
            if is_raised {
                core.shown.volume = core.audible;
            }
            core.apply_volume();
            is_raised
        });
        if is_raised {
            self.save_volume();
        }
    }

    /// Leaves out the songs that are skipped. Changes nothing and returns `false`
    /// when the song at `start_ix` is one of them.
    pub fn play(&self, songs: &[Song], start_ix: usize) -> bool {
        let is_played = self.change(|core| {
            if songs
                .get(start_ix)
                .is_some_and(|song| is_skipped(song, core.is_skipping))
            {
                return false;
            }
            let mut kept = Vec::with_capacity(songs.len());
            let mut kept_start_ix = start_ix;
            for (ix, song) in songs.iter().enumerate() {
                if !is_skipped(song, core.is_skipping) {
                    kept.push(song.clone());
                } else if ix < start_ix {
                    kept_start_ix -= 1;
                }
            }
            core.shown.queue.replace(kept, kept_start_ix);
            core.play_current(self);
            true
        });
        self.tell_if_skipped(is_played)
    }

    /// Leaves out the songs that are skipped. Changes nothing and returns `false`
    /// when that is all of them.
    pub fn play_whole(&self, songs: &[Song]) -> bool {
        let is_played = self.change(|core| {
            let kept = without_skipped(songs, core.is_skipping);
            if kept.is_empty() && !songs.is_empty() {
                return false;
            }
            core.shown.queue.replace_from_start(kept);
            core.play_current(self);
            true
        });
        self.tell_if_skipped(is_played)
    }

    /// Adds `songs` to the queue at `placement`. When nothing is playing or
    /// paused, playback starts: from the first of them if there was no
    /// current song, otherwise from the entry after the current one, so
    /// that songs already waiting are not skipped. The current one is
    /// stopped only after it failed.
    ///
    /// Leaves out the songs that are skipped. Changes nothing and returns `false`
    /// when that is all of them.
    pub fn enqueue(&self, songs: &[Song], placement: Placement) -> bool {
        let is_queued = self.change(|core| {
            let had_current = core.shown.has_song();
            let kept = without_skipped(songs, core.is_skipping);
            if core.shown.queue.insert(kept, placement).is_none() {
                return songs.is_empty();
            }
            if core.shown.state != State::Stopped {
                return true;
            }
            // Without a current song, the first one added has become it
            if !had_current || core.shown.queue.advance().is_some() {
                core.play_current(self);
            }
            true
        });
        self.tell_if_skipped(is_queued)
    }

    /// Passes `is_kept` on, telling [`PlayerNotice::Skipped`] when it is `false`.
    fn tell_if_skipped(&self, is_kept: bool) -> bool {
        if !is_kept {
            // Fails only when nobody listens
            self.0.notices.send(PlayerNotice::Skipped).ok();
        }
        is_kept
    }

    pub fn play_entry(&self, id: EntryId) {
        self.change(|core| {
            if core.shown.queue.jump_to(id).is_some() {
                core.play_current(self);
            }
        });
    }

    pub fn remove_entry(&self, id: EntryId) {
        self.change(|core| core.shown.queue.remove(id));
    }

    pub fn move_entry(&self, id: EntryId, to_ix: usize) {
        self.change(|core| core.shown.queue.move_entry(id, to_ix));
    }

    pub fn move_entry_to_next(&self, id: EntryId) {
        self.change(|core| core.shown.queue.move_to_next(id));
    }

    /// Removes every entry but the current one.
    pub fn clear_queue(&self) {
        self.change(|core| core.shown.queue.clear());
    }

    pub fn toggle_shuffle(&self) {
        self.change(|core| {
            let queue = &mut core.shown.queue;
            queue.set_shuffled(!queue.is_shuffled());
        });
    }

    pub fn set_repeat(&self, repeat: RepeatMode) {
        self.change(|core| core.shown.queue.set_repeat(repeat));
    }

    pub fn cycle_repeat(&self) {
        self.change(|core| {
            let repeat = match core.shown.queue.repeat() {
                RepeatMode::Off => RepeatMode::All,
                RepeatMode::All => RepeatMode::One,
                RepeatMode::One => RepeatMode::Off,
            };
            core.shown.queue.set_repeat(repeat);
        });
    }

    pub fn toggle_playback(&self) {
        self.change(|core| core.toggle_playback(self));
    }

    /// Plays or pauses, unless it already does.
    pub fn set_playing(&self, is_playing: bool) {
        self.change(|core| {
            if core.shown.is_playing() != is_playing {
                core.toggle_playback(self);
            }
        });
    }

    /// Pauses at the start of the song rather than stopping: the player stops only
    /// past the end of the queue or on a failure, and what is added to the
    /// queue after that goes on from the song after the current one.
    pub fn stop(&self) {
        self.change(|core| {
            if core.shown.is_playing() {
                core.toggle_playback(self);
            }
            core.seek_to(self, Duration::ZERO);
        });
    }

    pub fn next(&self) {
        self.change(|core| core.next(self));
    }

    pub fn previous(&self) {
        self.change(|core| {
            if !core.shown.has_song() {
                return;
            }
            let restart = core.shown.is_started() && core.engine.position() > RESTART_THRESHOLD;
            if !restart {
                core.shown.queue.retreat();
            }
            core.play_current(self);
        });
    }

    /// Jumps to `target` in the current song; at or past its end, moves on to the next one.
    /// Returns where it jumped to, if it did.
    pub fn seek_to(&self, target: Duration) -> Option<Duration> {
        self.change(|core| core.seek_to(self, target))
    }

    pub fn seek_forward(&self) -> Option<Duration> {
        self.seek_forward_by(SEEK_STEP)
    }

    pub fn seek_backward(&self) -> Option<Duration> {
        self.seek_backward_by(SEEK_STEP)
    }

    pub fn seek_forward_by(&self, by: Duration) -> Option<Duration> {
        self.change(|core| {
            let target = core.engine.position().saturating_add(by);
            core.seek_to(self, target)
        })
    }

    pub fn seek_backward_by(&self, by: Duration) -> Option<Duration> {
        self.change(|core| {
            let target = core.engine.position().saturating_sub(by);
            core.seek_to(self, target)
        })
    }

    /// Stops playback, returns the device to the sample rate it had before, waits for the
    /// engine to finish, which can take a second or more, and writes a volume still waiting to
    /// be saved. For when the app quits: quitting may not drop the player, and dropping it does
    /// not wait. The player plays nothing after.
    pub fn shutdown(&self) {
        self.0.core.update(|core| core.engine.shutdown());
        if let (Some(store), Some(saver)) = (&self.0.store, &self.0.volume_saver)
            && let Some(volume) = saver.take_waiting()
        {
            store.save_volume(volume).ok();
        }
    }

    /// Changes the state, then gets ready the entry that follows the current one, which the
    /// change may have made another.
    fn change<R>(&self, change: impl FnOnce(&mut Core) -> R) -> R {
        self.0.core.update(|core| {
            let result = change(core);
            core.prepare_following(self);
            core.keep_tally(self);
            result
        })
    }

    fn downgrade(&self) -> WeakPlayer {
        WeakPlayer(Arc::downgrade(&self.0))
    }
}

impl WeakPlayer {
    fn upgrade(&self) -> Option<Player> {
        self.0.upgrade().map(Player)
    }
}

impl Core {
    fn set_volume(&mut self, volume: f32) {
        if self.shown.is_volume_locked {
            return;
        }
        let volume = volume.clamp(0.0, 1.0);
        // Moving the volume up is a way to unmute
        if volume > 0.0 {
            self.shown.is_muted = false;
        }
        self.shown.volume = volume;
        self.apply_volume();
    }

    fn apply_volume(&self) {
        self.engine
            .set_volume(gain(self.shown.volume, self.shown.is_muted));
    }

    fn position(&self) -> Duration {
        if self.shown.is_started() {
            self.engine.position()
        } else {
            self.shown.resume.unwrap_or_default().position
        }
    }

    fn set_output(&mut self, this: &Player, output: OutputSettings) {
        let is_dop = output.dop().is_some();
        if !is_dop {
            self.dop_start = DopStart::Settled;
        } else if !self.shown.is_volume_locked {
            if self.shown.is_playing() {
                self.toggle_playback(this);
            }
            self.dop_start = DopStart::Held;
        }
        self.shown.is_volume_locked = is_dop;
        self.engine.set_output(output);
        if self.shown.state == State::Stopped && self.shown.resume.is_some() {
            self.open_current(this, false);
        }
    }

    /// Takes out the waiting songs that are skipped. The current one plays on
    /// and goes when it is no longer current.
    fn apply_skipping(&mut self) {
        let is_skipping = self.is_skipping;
        let queue = &mut self.shown.queue;
        let is_current_skipped = queue
            .current()
            .is_some_and(|song| is_skipped(song, is_skipping));
        queue.set_current_leaving(is_current_skipped);
        let current = queue.current_id();
        let skipped: Vec<_> = queue
            .entries()
            .iter()
            .filter(|entry| Some(entry.id()) != current && is_skipped(entry.item(), is_skipping))
            .map(|entry| entry.id())
            .collect();
        for id in skipped {
            queue.remove(id);
        }
    }

    fn toggle_playback(&mut self, this: &Player) {
        match self.shown.state {
            State::Playing => {
                self.engine.pause();
                self.shown.state = State::Paused;
            }
            State::Paused => {
                self.dop_start = DopStart::Settled;
                self.engine.resume();
                self.shown.state = State::Playing;
                self.announce_current(this);
            }
            State::Stopped if self.shown.resume.is_some() => {
                self.release_dop_start();
                self.rises_at_load = true;
                // Asked to play, whatever it was when it stopped
                if let Some(resume) = &mut self.shown.resume {
                    resume.paused = false;
                }
                self.open_current(this, false);
            }
            State::Stopped if self.shown.has_song() => {
                self.release_dop_start();
                self.rises_at_load = true;
                self.play_current(this);
            }
            State::Opening => {
                let resume = self.shown.resume.get_or_insert_default();
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
            }
            State::Stopped => {}
        }
    }

    fn release_dop_start(&mut self) {
        if self.dop_start == DopStart::Held {
            self.dop_start = DopStart::Asked;
        }
    }

    fn next(&mut self, this: &Player) {
        if !self.shown.has_song() {
            return;
        }
        if self.shown.queue.advance().is_some() {
            self.play_current(this);
        } else {
            self.stop_playing();
        }
    }

    fn seek_to(&mut self, this: &Player, target: Duration) -> Option<Duration> {
        let duration = self.shown.duration().filter(|_| self.shown.is_started())?;
        if target >= duration {
            self.next(this);
            return None;
        }
        self.engine.seek(target);
        Some(target)
    }

    fn stop_playing(&mut self) {
        self.cancel_open();
        self.engine.stop_fading();
        self.load = None;
        self.arrival = None;
        self.following = None;
        self.shown.format = None;
        self.shown.buffering = false;
        self.shown.resume = None;
        self.shown.state = State::Stopped;
        self.shown.queue.remove_left();
    }

    fn cancel_open(&mut self) {
        self._open = Task::ready(());
        self.open_id += 1;
    }

    fn play_current(&mut self, this: &Player) {
        self.shown.resume = None;
        self.tally = PlayTally::default();
        self.open_current(this, false);
    }

    /// Plays the current song from `resume`, or else from its start; with
    /// `forget_kept`, from the server after removing its kept file.
    fn open_current(&mut self, this: &Player, forget_kept: bool) {
        let Some(song) = self.shown.queue.current().cloned() else {
            self.stop_playing();
            return;
        };
        if self.dop_start == DopStart::Held {
            self.shown.resume.get_or_insert_default().paused = true;
        }
        let rises = mem::take(&mut self.rises_at_load);
        // Silence the previous song first, and ignore what is still on its
        // way about it, so its end cannot advance the queue a second time.
        self.engine.stop();
        self.load = None;
        self.arrival = None;
        self.following = None;
        self.loaded_ahead = false;
        self.show_opening();

        self.cancel_open();
        let open_id = self.open_id;
        let client = this.0.session.client().clone();
        let song_cache = self.song_cache.clone();
        let executor = this.0.executor.clone();
        let weak = this.downgrade();
        self._open = this.0.executor.spawn(async move {
            // Removing and opening a kept song read and write the disk
            let opened = executor
                .spawn_blocking({
                    let song = song.clone();
                    move || {
                        // A file that cannot be removed would fail again, so
                        // the song then comes from the server uncached.
                        let song_cache = song_cache.filter(|song_cache| {
                            !forget_kept || song_cache.forget(&client, song.id()).is_ok()
                        });
                        block_on(crate::open_track(&client, song_cache.as_ref(), &song))
                    }
                })
                .await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.change(|core| {
                if core.open_id != open_id {
                    return;
                }
                match opened {
                    Ok(opened) => {
                        core.loaded_kept = opened.is_kept();
                        core.arrival = opened.arrival();
                        let resume = core.shown.resume.unwrap_or_default();
                        let track = opened
                            .into_track()
                            .start_at(resume.position)
                            .start_paused(resume.paused)
                            .fade_in(core.dop_start != DopStart::Settled)
                            .rise_in(rises);
                        core.load = Some(core.engine.load(track));
                    }
                    Err(error) => core.fail(&song, &session::describe(&error)),
                }
            });
        });
    }

    fn show_opening(&mut self) {
        self.shown.format = None;
        self.shown.buffering = false;
        self.shown.error = None;
        self.shown.state = State::Opening;
        self.shown.queue.remove_left();
    }

    /// Makes the entry that follows the current one ready to go on to,
    /// replacing what was being made ready if that is no longer next. Waits
    /// for the current song's download, so that the two never compete for the
    /// connection and leave the song playing short of data.
    fn prepare_following(&mut self, this: &Player) {
        if !self.shown.is_started() {
            return;
        }
        let entry = self.shown.queue.following_id();
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
        let Some((entry, song)) = entry.zip(self.shown.queue.following().cloned()) else {
            return;
        };
        self.following_id += 1;
        let id = self.following_id;
        let client = this.0.session.client().clone();
        let song_cache = self.song_cache.clone();
        let arrival = self.arrival.clone();
        let executor = this.0.executor.clone();
        let weak = this.downgrade();
        let open = this.0.executor.spawn(async move {
            if let Some(arrival) = arrival {
                arrival.wait().await;
            }
            let opened = executor
                .spawn_blocking(move || {
                    block_on(crate::open_track(&client, song_cache.as_ref(), &song))
                })
                .await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.0.core.update(|core| {
                let following = core
                    .following
                    .as_mut()
                    .filter(|following| following.id == id);
                if let (Some(following), Ok(opened)) = (following, opened) {
                    following.kept = opened.is_kept();
                    following.arrival = opened.arrival();
                    following.load = Some(core.engine.set_next(opened.into_track()));
                }
            });
        });
        self.following = Some(Following {
            id,
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
    fn song_ended(&mut self, this: &Player, next: Option<LoadId>) {
        let paused = self.shown.state == State::Paused;
        let following_id = self.shown.queue.following_id();
        let following = self.following.take().filter(|following| {
            following.load.is_some()
                && following.load == next
                && Some(following.entry) == following_id
        });
        if self.shown.queue.follow().is_none() {
            self.stop_playing();
            return;
        }
        self.tally = PlayTally::default();
        self.shown.resume = paused.then_some(Resume {
            position: Duration::ZERO,
            paused,
        });
        let Some(following) = following else {
            self.open_current(this, false);
            return;
        };
        self.load = following.load;
        self.loaded_kept = following.kept;
        self.loaded_ahead = true;
        self.arrival = following.arrival;
        self.show_opening();
    }

    fn on_engine_event(&mut self, this: &Player, event: Event) {
        if self.load != Some(event.load) {
            return;
        }
        match event.kind {
            EventKind::Started(format) => {
                self.shown.format = Some(format);
                self.shown.error = None;
                self.loaded_ahead = false;
                if self.dop_start == DopStart::Asked {
                    self.dop_start = DopStart::Settled;
                }
                let paused = self.shown.resume.take().is_some_and(|resume| resume.paused);
                self.shown.state = if paused {
                    State::Paused
                } else {
                    State::Playing
                };
                if !paused {
                    self.announce_current(this);
                }
            }
            EventKind::OutputChanged(format) => self.shown.format = Some(format),
            EventKind::Buffering(buffering) => self.shown.buffering = buffering,
            EventKind::Ended { next } => self.song_ended(this, next),
            // The song fetched again plays from a download, not a kept
            // file, so a song is fetched again at most once per play.
            EventKind::Failed(error) if self.loaded_kept && error.is_track_error() => {
                self.open_current(this, true);
            }
            // It opened a while ago, and what stopped it then may have passed
            EventKind::Failed(error) if self.loaded_ahead && error.is_track_error() => {
                self.open_current(this, false);
            }
            EventKind::Failed(error) => {
                // The output is at fault, not the song, so it can pick up
                // once there is one that works.
                self.shown.resume = (!error.is_track_error()).then(|| self.resume_point());
                if let Some(song) = self.shown.queue.current().cloned() {
                    self.fail(&song, &describe(&error));
                }
            }
        }
    }

    /// Looks at the song for as long as it plays.
    fn keep_tally(&mut self, this: &Player) {
        if self.shown.state != State::Playing {
            self._tally = None;
            return;
        }
        if self._tally.is_some() {
            return;
        }
        let executor = this.0.executor.clone();
        let weak = this.downgrade();
        self._tally = Some(this.0.executor.spawn(async move {
            loop {
                executor.timer(TALLY_INTERVAL).await;
                let Some(this) = weak.upgrade() else {
                    return;
                };
                this.0.core.update(|core| core.observe_play(&this));
            }
        }));
    }

    fn observe_play(&mut self, this: &Player) {
        let position = self.position();
        let duration = self.shown.duration();
        if self.shown.state == State::Playing && self.tally.observe(position, duration) {
            self.scrobble_current(this);
        }
    }

    /// Tells the server the current song was played. The play is kept first,
    /// so that it still gets there if it cannot be sent now; failing to send
    /// it is not shown, as nothing about playback depends on it.
    fn scrobble_current(&self, this: &Player) {
        let Some(song) = self.shown.queue.current().map(|song| song.id().clone()) else {
            return;
        };
        let played_at = SystemTime::now();
        let client = this.0.session.client().clone();
        let scrobbles = this.0.session.scrobbles().cloned();
        let notices = this.0.notices.clone();
        // Keeping the play reads and writes the disk
        let sent = this.0.executor.spawn_blocking(move || {
            block_on(async {
                match scrobbles {
                    Some(scrobbles) if scrobbles.record(&song, played_at).is_ok() => {
                        scrobbles.submit(&client).await.ok();
                    }
                    _ => {
                        client.scrobble(&song, played_at).await.ok();
                    }
                }
            });
        });
        this.0
            .executor
            .spawn(async move {
                sent.await;
                notices.send(PlayerNotice::Played).ok();
            })
            .detach();
    }

    /// Tells the server the current song is playing now, also after a pause,
    /// which the server may have taken for the end of it. Replaces a notice
    /// still on its way, which is out of date, and is not sent again if it
    /// fails: by then it would be out of date too.
    fn announce_current(&mut self, this: &Player) {
        let Some(song) = self.shown.queue.current().map(|song| song.id().clone()) else {
            return;
        };
        let client = this.0.session.client().clone();
        self._announce = this.0.executor.spawn(async move {
            client.now_playing(&song).await.ok();
        });
    }

    /// Where the current song is, or was to pick up if it had yet to start.
    fn resume_point(&self) -> Resume {
        match self.shown.state {
            State::Playing | State::Paused => Resume {
                position: self.engine.position(),
                paused: self.shown.state == State::Paused,
            },
            State::Opening | State::Stopped => self.shown.resume.unwrap_or_default(),
        }
    }

    /// Stops on the failed song and says why, so Play retries it.
    fn fail(&mut self, song: &Song, reason: &str) {
        self.cancel_open();
        self.engine.stop();
        self.load = None;
        self.arrival = None;
        self.following = None;
        self.shown.format = None;
        self.shown.buffering = false;
        self.shown.error = Some(format!("Couldn't play “{}”. {reason}", song.title()).into());
        self.shown.state = State::Stopped;
    }
}

impl Publish for Core {
    type Snapshot = PlayerSnapshot;

    fn snapshot(&self) -> &PlayerSnapshot {
        &self.shown
    }
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

#[cfg(any(test, feature = "test-support"))]
impl Player {
    /// Puts `songs` in the queue on `current_ix`, without playing.
    pub fn fill_queue(&self, songs: &[Song], current_ix: usize) {
        self.0
            .core
            .update(|core| core.shown.queue.replace(songs.to_vec(), current_ix));
    }

    /// Stands as though the current song were playing or paused.
    pub fn pretend_playing(&self, playing: bool) {
        self.0.core.update(|core| {
            core.shown.state = if playing {
                State::Playing
            } else {
                State::Paused
            };
        });
    }

    /// Stands stopped, to pick up paused at `position`.
    pub fn place_at(&self, position: Duration) {
        self.0.core.update(|core| {
            core.shown.resume = Some(Resume {
                position,
                paused: true,
            });
        });
    }

    pub fn pretend_buffering(&self, buffering: bool) {
        self.0.core.update(|core| core.shown.buffering = buffering);
    }

    pub fn set_error(&self, error: &str) {
        self.0
            .core
            .update(|core| core.shown.error = Some(error.into()));
    }
}

#[cfg(test)]
mod tests;
