mod current;
mod lines;

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::base::Easing;
use gpui_kit::component::{
    ActiveTheme as _, MotionTokens, StyledExt as _, TITLE_BAR_HEIGHT, h_flex, v_flex,
};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Div, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, Pixels, Render, Role,
    SharedString, Size, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    Subscription, Task, Window, div, img, point, prelude::FluentBuilder as _, px, size,
};
use subsonic::Song;
use ui::CoverArt;
use ui::list_navigation::CONTROL_CONTEXT;

use crate::ToggleLyrics;
use crate::player::{Player, cover_hover};
use crate::transport::{PanelToggles, PositionLine, TransportControls, VolumeControl};

pub use current::CurrentLyrics;
#[cfg(test)]
pub(crate) use current::FindLyrics;
pub(crate) use lines::{LinesFit, LinesType, LyricsLines};

pub const LYRICS_CONTEXT: &str = "Lyrics";

const OPEN_DURATION: Duration = Duration::from_millis(640);
/// Each part eases the same way as when opening, so that it leaves fast and settles softly.
const CLOSE_DURATION: Duration = Duration::from_millis(400);
/// When each part moves, as parts of the way from closed to open.
const OPENING_PARTS: Parts = Parts {
    cover: 0.0..0.72,
    under: 0.0..0.44,
    backdrop: 0.0..0.44,
    details: 0.25..0.69,
    lines: 0.34..1.0,
};
/// The same on the way back, which runs from `1.0` down to `0.0`. The cover and
/// what it flies over leave at once, and the screen returns as the cover nears.
const CLOSING_PARTS: Parts = Parts {
    cover: 0.0..1.0,
    under: 0.0..0.75,
    backdrop: 0.0..0.75,
    details: 0.5..1.0,
    lines: 0.5..1.0,
};

/// Width of the column with the cover: the part of the window it takes, and the
/// least and most it may be in rems.
const COLUMN_SHARE: f32 = 0.38;
const COLUMN_MIN: f32 = 21.25;
const COLUMN_MAX: f32 = 32.5;
/// Space around the column's contents, in rems.
const COLUMN_INSET: f32 = 2.5;
const COLUMN_MARGIN: f32 = 1.5;
/// From the cover to what is below it, in rems.
const DETAILS_GAP: f32 = 1.25;
/// Height of what is below the cover, in rems.
const DETAILS_HEIGHT: f32 = 9.75;
/// In rems.
const COVER_MIN: f32 = 6.;
/// Height of the bar and size of its cover, in rems, for the cover to leave
/// from and return to the bar.
const BAR_HEIGHT: f32 = 4.;
const BAR_INSET: f32 = 1.;
const BAR_COVER: f32 = 2.5;
/// Half of the bar's top border, which its content centres below. A border is
/// a device pixel at any zoom, so this is in pixels.
const BAR_BORDER_SHIFT: Pixels = px(0.5);
/// In rems.
const LARGE_TYPE_FROM: f32 = 62.5;

struct Parts {
    cover: Range<f32>,
    under: Range<f32>,
    backdrop: Range<f32>,
    details: Range<f32>,
    lines: Range<f32>,
}

/// How far each part of the view has come, from `0.0`, closed, to `1.0`, open.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Shown {
    cover: f32,
    /// How far what is under the view has gone.
    under: f32,
    backdrop: f32,
    details: f32,
    lines: f32,
}

#[derive(Clone, Copy, Debug)]
struct Opening {
    is_open: bool,
    from: f32,
    /// What showed when it last turned, for each part to go on from there.
    from_shown: Shown,
    started_at: Instant,
    duration: Duration,
}

impl Opening {
    fn closed(now: Instant) -> Self {
        Self {
            is_open: false,
            from: 0.,
            from_shown: Shown::default(),
            started_at: now,
            duration: Duration::ZERO,
        }
    }

    fn target(&self) -> f32 {
        if self.is_open { 1. } else { 0. }
    }

    /// From `0.0`, closed, to `1.0`, open, at a steady pace.
    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started_at);
        if self.duration.is_zero() || elapsed >= self.duration {
            return self.target();
        }
        let done = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        self.from + (self.target() - self.from) * done
    }

    /// Each part eases from where it was when the view last turned to where it is going.
    fn shown(&self, motion: &MotionTokens, now: Instant) -> Shown {
        let progress = self.progress(now);
        let from = &self.from_shown;
        let parts = if self.is_open {
            OPENING_PARTS
        } else {
            CLOSING_PARTS
        };
        // What goes away speeds up, and what comes back slows down
        let under = if self.is_open {
            &motion.easing_exit
        } else {
            &motion.easing_enter
        };
        Shown {
            cover: self.part(progress, parts.cover, &motion.easing_move, from.cover),
            under: self.part(progress, parts.under, under, from.under),
            backdrop: self.part(
                progress,
                parts.backdrop,
                &motion.easing_enter,
                from.backdrop,
            ),
            details: self.part(progress, parts.details, &motion.easing_enter, from.details),
            lines: self.part(progress, parts.lines, &motion.easing_enter, from.lines),
        }
    }

    fn part(&self, progress: f32, part: Range<f32>, easing: &Easing, from: f32) -> f32 {
        let here = through(progress, part.clone());
        let start = through(self.from, part);
        let (mut gone, mut whole) = if self.is_open {
            (here - start, 1. - start)
        } else {
            (start - here, start)
        };
        // It turned with this part's share behind it in the new direction, though
        // not at its end: it takes the rest of the way instead.
        if whole <= 0. {
            gone = (progress - self.from).abs();
            whole = (self.target() - self.from).abs();
        }
        if gone >= whole {
            return self.target();
        }
        from + (self.target() - from) * easing.sample(gone / whole)
    }

    /// Turns back from where it is, taking only as long as what is left.
    fn set_open(
        &mut self,
        is_open: bool,
        reduce_motion: bool,
        motion: &MotionTokens,
        now: Instant,
    ) {
        if self.is_open == is_open {
            return;
        }
        let from = self.progress(now);
        self.from_shown = self.shown(motion, now);
        self.is_open = is_open;
        self.from = from;
        self.started_at = now;
        let whole = if is_open {
            OPEN_DURATION
        } else {
            CLOSE_DURATION
        };
        self.duration = if reduce_motion {
            Duration::ZERO
        } else {
            whole.mul_f32((self.target() - from).abs())
        };
    }
}

fn through(progress: f32, part: Range<f32>) -> f32 {
    ((progress - part.start) / (part.end - part.start)).clamp(0., 1.)
}

/// The corners of the cover `moved` of the way from the bar to its place, where
/// they meet those of the bar's cover and those of a large cover.
fn cover_radius(moved: f32, cx: &App) -> Pixels {
    let theme = cx.theme();
    theme.radius + (theme.radius_lg - theme.radius) * moved
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LyricsReveal {
    pub is_shown: bool,
    /// Whether it hides all that is under it, which then need not be rendered.
    pub is_covering: bool,
    /// Whether it is on its way, so the owner should draw the next frame.
    pub is_moving: bool,
    pub under_opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Stage {
    column: Pixels,
    cover: Bounds<Pixels>,
    bar_cover: Bounds<Pixels>,
    details_top: Pixels,
    has_large_type: bool,
}

impl Stage {
    /// In the room there is under the title bar, as ordinary layout can't say
    /// where a cover moving between two layouts should be.
    fn new(room: Size<Pixels>, rem: Pixels) -> Self {
        let column = (room.width * COLUMN_SHARE)
            .max(rem * COLUMN_MIN)
            .min(rem * COLUMN_MAX);
        let side = (column - rem * COLUMN_INSET * 2.)
            .min(room.height - rem * (COLUMN_MARGIN * 2. + DETAILS_GAP + DETAILS_HEIGHT))
            .max(rem * COVER_MIN)
            .floor();
        let height = side + rem * (DETAILS_GAP + DETAILS_HEIGHT);
        let top = ((room.height - height) / 2.).max(px(0.)).floor();
        let left = ((column - side) / 2.).floor();
        let bar_side = (rem * BAR_HEIGHT - rem * BAR_COVER) / 2.;
        Self {
            column,
            cover: Bounds::new(point(left, top), size(side, side)),
            bar_cover: Bounds::new(
                point(
                    rem * BAR_INSET,
                    room.height - rem * BAR_HEIGHT + bar_side + BAR_BORDER_SHIFT,
                ),
                size(rem * BAR_COVER, rem * BAR_COVER),
            ),
            details_top: top + side + rem * DETAILS_GAP,
            has_large_type: room.width >= rem * LARGE_TYPE_FROM,
        }
    }

    /// The cover `moved` of the way from the bar to its place.
    fn cover_at(&self, moved: f32) -> Bounds<Pixels> {
        let between = |from: Pixels, to: Pixels| from + (to - from) * moved;
        Bounds::new(
            point(
                between(self.bar_cover.left(), self.cover.left()),
                between(self.bar_cover.top(), self.cover.top()),
            ),
            size(
                between(self.bar_cover.size.width, self.cover.size.width),
                between(self.bar_cover.size.height, self.cover.size.height),
            ),
        )
    }
}

/// Its owner renders it within the room under the title bar, as [`LyricsView::reveal`]
/// says, and looks after where focus goes as it opens and closes.
pub struct LyricsView {
    player: Entity<Player>,
    lines: Entity<LyricsLines>,
    focus: FocusHandle,
    opening: Opening,
    /// Whether any of the view shows, and its cover stands in for the bar's.
    is_shown: bool,
    _closing: Task<()>,
    _player_changes: Subscription,
}

impl LyricsView {
    pub fn new(
        player: Entity<Player>,
        lyrics: Entity<CurrentLyrics>,
        cx: &mut Context<Self>,
    ) -> Self {
        let lines = cx.new(|cx| LyricsLines::new(player.clone(), lyrics, lines_fit, cx));
        let player_changes = cx.observe(&player, |_, _, cx| cx.notify());
        Self {
            player,
            lines,
            focus: cx.focus_handle(),
            opening: Opening::closed(cx.background_executor().now()),
            is_shown: false,
            _closing: Task::ready(()),
            _player_changes: player_changes,
        }
    }

    pub fn is_open(&self) -> bool {
        self.opening.is_open
    }

    pub fn set_open(&mut self, is_open: bool, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let motion = cx.theme().motion_tokens().clone();
        let was_open = self.opening.is_open;
        self.opening
            .set_open(is_open, cx.reduce_motion(), &motion, now);
        if is_open {
            self._closing = Task::ready(());
            self.set_shown(true, cx);
        } else if was_open {
            self.finish_closing(cx);
        }
        self.lines
            .update(cx, |lines, cx| lines.set_active(is_open, cx));
        cx.notify();
    }

    fn finish_closing(&mut self, cx: &mut Context<Self>) {
        let left = self.opening.duration;
        if left.is_zero() {
            self.set_shown(false, cx);
            return;
        }
        self._closing = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(left).await;
            this.update(cx, |this, cx| this.set_shown(false, cx)).ok();
        });
    }

    /// One step for the view and the bar's cover, so that a frame never has both
    /// covers or neither.
    fn set_shown(&mut self, is_shown: bool, cx: &mut Context<Self>) {
        self.is_shown = is_shown;
        self.player
            .update(cx, |player, cx| player.set_cover_hidden(is_shown, cx));
        cx.notify();
    }

    pub fn reveal(&self, cx: &App) -> LyricsReveal {
        let now = cx.background_executor().now();
        let progress = self.opening.progress(now);
        let shown = self.opening.shown(cx.theme().motion_tokens(), now);
        LyricsReveal {
            is_shown: self.is_shown,
            is_covering: self.opening.is_open && progress >= 1.,
            is_moving: progress != self.opening.target(),
            under_opacity: 1. - shown.under,
        }
    }

    fn render_details(&self, song: &Song, cx: &mut Context<Self>) -> Div {
        let player = self.player.read(cx);
        let format = player.format_text();
        let color = cx.theme().muted_foreground;
        v_flex()
            .debug_selector(|| "lyrics-details".to_owned())
            .key_context(CONTROL_CONTEXT)
            .child(
                div()
                    .text_lg()
                    .font_semibold()
                    .truncate()
                    .child(SharedString::from(song.title().to_owned())),
            )
            .child(self.player.read(cx).render_song_status(song, cx))
            .child(
                div()
                    .mt_4()
                    .child(PositionLine::full_width(self.player.clone())),
            )
            .child(
                div()
                    .mt_2()
                    .child(TransportControls::new(self.player.clone()).roomy()),
            )
            .child(
                h_flex()
                    .mt_2()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .text_color(color)
                    .child(div().min_w_0().truncate().children(format))
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_2()
                            .child(PanelToggles::new(self.player.clone()))
                            .child(VolumeControl::new(self.player.clone())),
                    ),
            )
    }

    /// The cover, which is the one in the bar at first, and comes to its large
    /// self when that has arrived.
    fn render_cover(
        &self,
        stage: &Stage,
        moved: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let bounds = stage.cover_at(moved);
        let radius = cover_radius(moved, cx);
        let (small, large) = self.player.read(cx).cover_urls();
        let (small, large) = (small.cloned(), large.cloned());
        div()
            .id("lyrics-cover")
            .debug_selector(|| "lyrics-cover".to_owned())
            .role(Role::Button)
            .aria_label("Lyrics")
            .absolute()
            .left(bounds.left())
            .top(bounds.top())
            .w(bounds.size.width)
            .h(bounds.size.height)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleLyrics), cx))
            .child(CoverArt::new(small).fill().rounded_by(radius))
            .when_some(large, |this, large| {
                this.child(
                    img(large)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .rounded(radius),
                )
            })
            .child(cover_hover("lyrics-cover-hover", radius, cx))
    }
}

impl Focusable for LyricsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LyricsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let room = room_under_title_bar(window);
        let rem = window.rem_size();
        let stage = Stage::new(room, rem);

        let now = cx.background_executor().now();
        let progress = self.opening.progress(now);
        if progress != self.opening.target() {
            window.request_animation_frame();
        }
        let motion = cx.theme().motion_tokens();
        let rise = rem * motion.distance_medium.0;
        let shown = self.opening.shown(motion, now);
        let background = cx.theme().background;

        let song = self.player.read(cx).current_song().cloned();
        let details = song.map(|song| self.render_details(&song, cx));
        let cover = self.render_cover(&stage, shown.cover, cx);
        div()
            .id("lyrics-view")
            .key_context(LYRICS_CONTEXT)
            .track_focus(&self.focus)
            .absolute()
            .inset_0()
            .occlude()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(background)
                    .opacity(shown.backdrop),
            )
            .child(
                div()
                    .absolute()
                    .left(stage.column)
                    .right_0()
                    .top(rise * (1. - shown.lines))
                    .h(room.height)
                    .opacity(shown.lines)
                    .child(self.lines.clone()),
            )
            .children(details.map(|details| {
                details
                    .absolute()
                    .left(stage.cover.left())
                    .top(stage.details_top + rise * (1. - shown.details))
                    .w(stage.cover.size.width)
                    .opacity(shown.details)
            }))
            .child(cover)
    }
}

fn room_under_title_bar(window: &Window) -> Size<Pixels> {
    let viewport = window.viewport_size();
    size(viewport.width, viewport.height - TITLE_BAR_HEIGHT)
}

fn lines_fit(window: &Window) -> LinesFit {
    let room = room_under_title_bar(window);
    let stage = Stage::new(room, window.rem_size());
    LinesFit {
        height: room.height,
        line_type: if stage.has_large_type {
            LinesType::Large
        } else {
            LinesType::Medium
        },
    }
}

#[cfg(test)]
mod tests;
