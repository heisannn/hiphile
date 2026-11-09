use std::time::Duration;

use gpui_kit::base::{Transition, transition};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, h_flex,
    slider::{SliderEvent, SliderState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, div, prelude::FluentBuilder as _, relative, rems,
};
use hiphile_core::favorites::Favorite;
use playback::{EntryId, PlayerSnapshot};
use subsonic::Song;
use ui::{AlbumLink, ArtistLink, CoverArt, song_artist_link};

use crate::ToggleLyrics;
use crate::transport::{PanelToggles, PositionLine, TransportControls, VolumeControl};

const POSITION_REFRESH: Duration = Duration::from_millis(250);
/// Pixel size requested for the bar's cover. The renderer aliases an image drawn
/// at less than half its size, so this stays close to the on-screen size.
const COVER_PIXELS: u32 = 48;
const LARGE_COVER_PIXELS: u32 = 512;
const COVER_HOVER_OPACITY: f32 = 0.08;
const COVER_PRESS_OPACITY: f32 = 0.14;

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

/// The bar that shows what `playback::Player` plays, and the sliders and covers the other
/// views share with it.
pub struct Player {
    core: playback::Player,
    session: session::Session,
    /// The current entry, as the last snapshot showed it.
    followed: Option<EntryId>,
    /// Whether the bar leaves out its cover, for another that stands in for it.
    is_cover_hidden: bool,
    panels: PanelsSource,
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
    _changes: Task<()>,
    _refresh: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Player {
    pub fn new(
        core: playback::Player,
        session: session::Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let refresh = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POSITION_REFRESH).await;
                let refreshed = gpui_host::update_in_window(&this, cx, |this, window, cx| {
                    this.refresh(window, cx)
                });
                if refreshed.is_none() {
                    break;
                }
            }
        });
        let changes =
            gpui_host::observe_in(core.subscribe(), window, cx, |this, shown, window, cx| {
                this.follow(&shown, window, cx);
            });

        let level = core.snapshot().volume();
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
            cx.subscribe(&volume, |this, _, event, _| match event {
                SliderEvent::Change(value) => this.core.set_volume(value.end()),
                SliderEvent::Release(value) => {
                    this.core.set_volume(value.end());
                    this.core.save_volume();
                }
            }),
        ];
        Self {
            core,
            session,
            followed: None,
            is_cover_hidden: false,
            panels: Box::new(|_, _| Panels::default()),
            position,
            mini_position,
            scrub: None,
            refreshed_position: Duration::ZERO,
            volume,
            _changes: changes,
            _refresh: refresh,
            _subscriptions: subscriptions,
        }
    }

    pub fn core(&self) -> &playback::Player {
        &self.core
    }

    pub(crate) fn snapshot(&self) -> PlayerSnapshot {
        self.core.snapshot()
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

    /// The current song's cover, small and large.
    pub(crate) fn cover_urls(&self) -> (Option<SharedString>, Option<SharedString>) {
        let snapshot = self.core.snapshot();
        let cover = snapshot.current_song().and_then(Song::cover_art);
        let url = |pixels| cover.map(|cover| self.session.cover_art_url(cover, pixels).into());
        (url(COVER_PIXELS), url(LARGE_COVER_PIXELS))
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

    /// The position to show, which follows a drag of the slider.
    pub(crate) fn shown_position(&self) -> Duration {
        match (self.scrub, self.core.snapshot().duration()) {
            (Some(fraction), Some(duration)) => duration.mul_f32(fraction),
            _ => self.core.position(),
        }
    }

    pub fn seek_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.core.seek_forward();
        self.show_seek(target, window, cx);
    }

    pub fn seek_backward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.core.seek_backward();
        self.show_seek(target, window, cx);
    }

    /// Jumps to `target` in the current song; at or past its end, moves on
    /// to the next one.
    pub(crate) fn seek_to(
        &mut self,
        target: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.core.seek_to(target);
        self.show_seek(target, window, cx);
    }

    fn show_seek(&self, target: Option<Duration>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = target {
            self.show_position(target, window, cx);
            cx.notify();
        }
    }

    fn follow(&mut self, shown: &PlayerSnapshot, window: &mut Window, cx: &mut Context<Self>) {
        let entry = shown.queue().current_id();
        if std::mem::replace(&mut self.followed, entry) != entry {
            self.scrub = None;
        }
        if self.volume.read(cx).value().end() != shown.volume() {
            self.volume.update(cx, |slider, cx| {
                slider.set_value(shown.volume(), window, cx)
            });
        }
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let position = self.core.position();
        if self.scrub.is_none() {
            self.show_position(position, window, cx);
        }
        if position != self.refreshed_position {
            self.refreshed_position = position;
            cx.notify();
        }
    }

    fn show_position(&self, position: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let fraction = self.core.snapshot().duration().map_or(0.0, |duration| {
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
                if let Some(duration) = self.core.snapshot().duration() {
                    self.seek_to(duration.mul_f32(value.end()), window, cx);
                }
            }
        }
    }

    pub(crate) fn render_song_status(&self, song: &Song, cx: &App) -> AnyElement {
        match self.core.snapshot().error() {
            Some(error) => div()
                .debug_selector(|| "player-error".to_owned())
                .text_xs()
                .truncate()
                .text_color(cx.theme().danger)
                .child(SharedString::from(error.to_owned()))
                .into_any_element(),
            None => song_details(song, cx).into_any_element(),
        }
    }

    fn render_song(&self, shown: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.core.snapshot();
        let song = snapshot.current_song();
        let is_cover_hidden = self.is_cover_hidden;
        let (cover_url, _) = self.cover_urls();
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
                        .child(CoverArt::new(cover_url).small())
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

impl Render for Player {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.core.snapshot();
        let format_text = snapshot.format_text();
        let muted_foreground = cx.theme().muted_foreground;
        let song_shown = reveal("player-song-shown", snapshot.has_song(), window, cx);
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
        self.core.fill_queue(songs, current_ix);
    }

    pub(crate) fn place_at(&mut self, position: Duration) {
        self.core.place_at(position);
    }

    pub(crate) fn pretend_playing(&mut self, playing: bool, _: &mut Context<Self>) {
        self.core.pretend_playing(playing);
    }

    pub(crate) fn set_error(&mut self, error: &str) {
        self.core.set_error(error);
    }

    pub(crate) fn is_cover_hidden(&self) -> bool {
        self.is_cover_hidden
    }

    pub(crate) fn is_scrubbing(&self) -> bool {
        self.scrub.is_some()
    }
}

#[cfg(test)]
mod tests;
