use std::time::Duration;

use audio::{Engine, Event, EventKind, LoadId, PlaybackFormat};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    progress::Progress,
    v_flex,
};
use gpui_kit::{
    Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Task, Window, div, prelude::FluentBuilder as _,
};
use playback::Queue;
use subsonic::Song;
use ui::{CoverArt, format_duration};

use crate::format;
use crate::{NextSong, PreviousSong, TogglePlayback};

/// How often the position redraws while playing.
const POSITION_REFRESH: Duration = Duration::from_millis(250);
/// Within this much of a song's start, Previous goes to the previous song;
/// later, it restarts the current one.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);
/// Pixel size requested for the bar's cover: twice its on-screen size.
const COVER_PIXELS: u32 = 96;

/// The play queue, the audio engine, and the now-playing bar that controls
/// them. Dropping it, or quitting the app, stops playback and returns the
/// output device to its sample rate from before.
pub struct Player {
    session: session::Session,
    engine: Engine,
    /// The engine load of the current song, once it has one. Events about
    /// any other load are stale.
    load: Option<LoadId>,
    queue: Queue<Song>,
    state: State,
    format: Option<PlaybackFormat>,
    cover_url: Option<SharedString>,
    error: Option<SharedString>,
    _open: Task<()>,
    _events: Task<()>,
    _refresh: Task<()>,
    _quit: Subscription,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Stopped,
    /// The current song's download is starting or its decoder is opening.
    Opening,
    Playing,
    Paused,
}

impl Player {
    pub fn new(session: session::Session, cx: &mut Context<Self>) -> Self {
        let (engine, events) = Engine::spawn();
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
        // Quitting may not drop entities, and dropping the engine does not wait
        // for it, so shut it down here, while the process still runs.
        let quit = cx.on_app_quit(|this, _| {
            this.engine.shutdown();
            async {}
        });
        Self {
            session,
            engine,
            load: None,
            queue: Queue::new(),
            state: State::Stopped,
            format: None,
            cover_url: None,
            error: None,
            _open: Task::ready(()),
            _events: events,
            _refresh: Task::ready(()),
            _quit: quit,
        }
    }

    /// Replaces the queue with `songs` and plays from `start_ix`.
    pub fn play(&mut self, songs: &[Song], start_ix: usize, cx: &mut Context<Self>) {
        self.queue.replace(songs.to_vec(), start_ix);
        self.play_current(cx);
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
            }
            State::Stopped if self.queue.current().is_some() => self.play_current(cx),
            State::Stopped | State::Opening => {}
        }
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        if self.queue.current().is_none() {
            return;
        }
        if self.queue.advance().is_some() {
            self.play_current(cx);
        } else {
            self.stop(cx);
        }
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        if self.queue.current().is_none() {
            return;
        }
        let restart = matches!(self.state, State::Playing | State::Paused)
            && self.engine.position() > RESTART_THRESHOLD;
        if !restart {
            self.queue.retreat();
        }
        self.play_current(cx);
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self._open = Task::ready(());
        self.engine.stop();
        self.load = None;
        self.format = None;
        if self.queue.current().is_none() {
            self.cover_url = None;
        }
        self.set_state(State::Stopped, cx);
    }

    fn play_current(&mut self, cx: &mut Context<Self>) {
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
        self.cover_url = song
            .cover_art()
            .map(|cover_art| self.session.cover_art_url(cover_art, COVER_PIXELS));
        self.set_state(State::Opening, cx);

        let client = self.session.client().clone();
        // Replacing the task drops an earlier open, and with it its download.
        self._open = cx.spawn(async move |this, cx| {
            let track = playback::open_track(&client, &song).await;
            this.update(cx, |this, cx| match track {
                Ok(track) => this.load = Some(this.engine.load(track)),
                Err(error) => this.fail(&song, &error.to_string(), cx),
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
                self.set_state(State::Playing, cx);
            }
            EventKind::Ended => {
                if self.queue.advance().is_some() {
                    self.play_current(cx);
                } else {
                    self.stop(cx);
                }
            }
            EventKind::Failed(error) => {
                if let Some(song) = self.queue.current().cloned() {
                    self.fail(&song, &error.to_string(), cx);
                }
            }
        }
    }

    /// Stops on the failed song and says why, so Play retries it.
    fn fail(&mut self, song: &Song, reason: &str, cx: &mut Context<Self>) {
        self.engine.stop();
        self.load = None;
        self.format = None;
        self.error = Some(format!("Couldn't play “{}”: {reason}.", song.title()).into());
        self.set_state(State::Stopped, cx);
    }

    fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        self._refresh = if state == State::Playing {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(POSITION_REFRESH).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
        } else {
            Task::ready(())
        };
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
            .child(CoverArt::new(self.cover_url.clone()).small())
            .child(
                v_flex()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(match song {
                        Some(song) => SharedString::from(song.title().to_owned()),
                        None => "Not playing".into(),
                    }))
                    .when_some(secondary, |this, (text, color)| {
                        this.child(div().text_xs().truncate().text_color(color).child(text))
                    }),
            )
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_song = self.queue.current().is_some();
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

    fn render_position(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let song = self.queue.current();
        let started = matches!(self.state, State::Playing | State::Paused);
        let position = if started {
            self.engine.position()
        } else {
            Duration::ZERO
        };
        let duration = self
            .format
            .as_ref()
            .and_then(PlaybackFormat::duration)
            .or_else(|| song.and_then(Song::duration));
        let percent = match duration {
            Some(duration) if !duration.is_zero() => {
                (position.as_secs_f32() / duration.as_secs_f32() * 100.0).min(100.0)
            }
            _ => 0.0,
        };
        let format_text = self
            .format
            .as_ref()
            .map(|format| format::describe(format, song.and_then(Song::suffix)));
        let muted_foreground = cx.theme().muted_foreground;
        h_flex()
            .flex_1()
            .min_w_0()
            .justify_end()
            .gap_3()
            .text_xs()
            .text_color(muted_foreground)
            .when(song.is_some(), |this| {
                this.child(div().flex_none().child(format_duration(position)))
                    .child(
                        div().w_48().child(
                            Progress::new("song-position")
                                .small()
                                .value(percent)
                                .accessibility_label("Position"),
                        ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .child(duration.map(format_duration).unwrap_or_default()),
                    )
            })
            .when_some(format_text, |this, text| {
                this.child(div().flex_none().child(text))
            })
    }
}

impl Render for Player {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(self.render_position(cx))
    }
}
