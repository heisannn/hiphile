use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use config::LyricsHighlight;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui_kit::base::{Easing, Transition, transition};
use gpui_kit::component::{
    ActiveTheme as _, MotionTokens, StyledExt as _, TITLE_BAR_HEIGHT, button::Button, h_flex,
    scroll::Scrollbar, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Div, ElementId, Entity, FocusHandle,
    Focusable, HighlightStyle, Hsla, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, Pixels, Render, Role, ScrollHandle, SharedString, Size,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, StyledText, Subscription, Task,
    Window, div, img, point, prelude::FluentBuilder as _, px, rems, size,
};
use lyrics::{Finder, Lyrics, TimedLine, TimedLyrics, TrackQuery};
use subsonic::{Song, SongId};
use ui::list_navigation::CONTROL_CONTEXT;
use ui::{CoverArt, centered, muted_text, section_failure, section_loading};

use crate::ToggleLyrics;
use crate::player::{Player, PlayerEvent, cover_hover};
use crate::transport::{PanelToggles, PositionLine, TransportControls, VolumeControl};

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

const ELEMENT_FADE: Duration = Duration::from_millis(120);
const LINE_FADE: Duration = Duration::from_millis(180);
const SCROLL_DURATION: Duration = Duration::from_millis(460);
/// How often to check whether the lines should change, as the player's own
/// refresh is too slow for parts of a line.
const HIGHLIGHT_REFRESH: Duration = Duration::from_millis(50);
/// Added to where the song is for lighting the lines. Seeking lands on the frame
/// before the time asked for, and a line chosen should be the one that lights.
const SEEK_ROUNDING: Duration = Duration::from_millis(1);
/// How far down the lines the current one rests, as a part of their height.
const CURRENT_LINE_AT: f32 = 0.36;
/// How much room follows the last line, as a part of the height of the lines,
/// for it to reach where the current line rests.
const ROOM_AFTER_LINES: f32 = 0.5;
/// How many songs' lyrics stay in memory, for going back and forth in a queue.
const FOUND_KEPT: usize = 64;

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
/// In rems.
const LINE_MAX_WIDTH: f32 = 42.5;

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
    room_before_lines: Pixels,
    room_after_lines: Pixels,
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
            room_before_lines: room.height * CURRENT_LINE_AT,
            room_after_lines: room.height * ROOM_AFTER_LINES,
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

type FindLyrics =
    Arc<dyn Fn(TrackQuery) -> BoxFuture<'static, Result<Lyrics, lyrics::Error>> + Send + Sync>;

enum Lookup {
    Nothing,
    Loading(SongId),
    Done(SongId, Arc<Lyrics>),
    Failed(SongId, SharedString),
}

impl Lookup {
    fn song(&self) -> Option<&SongId> {
        match self {
            Self::Nothing => None,
            Self::Loading(song) | Self::Done(song, _) | Self::Failed(song, _) => Some(song),
        }
    }
}

/// Its owner renders it within the room under the title bar, as [`LyricsView::reveal`]
/// says, and looks after where focus goes as it opens and closes.
pub struct LyricsView {
    player: Entity<Player>,
    find_lyrics: FindLyrics,
    focus: FocusHandle,
    opening: Opening,
    /// Whether any of the view shows, and its cover stands in for the bar's.
    is_shown: bool,
    lookup: Lookup,
    /// Songs whose lyrics were found, or found not to exist, oldest first.
    found: Vec<(SongId, Arc<Lyrics>)>,
    scroll: ScrollHandle,
    /// Whether the lines keep the current one in view, until the user scrolls.
    is_following: bool,
    /// Changes with each new set of lines, which then take their place at once.
    scroll_key: usize,
    highlight: LyricsHighlight,
    /// Which line and how much of it the last frame lit.
    lit: Lit,
    _closing: Task<()>,
    refresh: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

impl LyricsView {
    pub fn new(player: Entity<Player>, finder: Finder, cx: &mut Context<Self>) -> Self {
        let find_lyrics = Arc::new(move |query: TrackQuery| {
            let finder = finder.clone();
            async move { finder.find_lyrics(&query).await }.boxed()
        });
        Self::with_find_lyrics(player, find_lyrics, cx)
    }

    fn with_find_lyrics(
        player: Entity<Player>,
        find_lyrics: FindLyrics,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = [
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.subscribe(&player, |this, _, event, cx| {
                if matches!(event, PlayerEvent::NowPlaying { .. }) {
                    this.sync_refresh(cx);
                    // It says so for pausing too, which is no cause to look again
                    if this.opening.is_open && !this.is_current_song_looked_up(cx) {
                        this.look_up_current(cx);
                    }
                }
            }),
        ];
        Self {
            player,
            find_lyrics,
            focus: cx.focus_handle(),
            opening: Opening::closed(cx.background_executor().now()),
            is_shown: false,
            lookup: Lookup::Nothing,
            found: Vec::new(),
            scroll: ScrollHandle::new(),
            is_following: true,
            scroll_key: 0,
            highlight: LyricsHighlight::default(),
            lit: Lit::default(),
            _closing: Task::ready(()),
            refresh: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_highlight(&mut self, highlight: LyricsHighlight, cx: &mut Context<Self>) {
        self.highlight = highlight;
        cx.notify();
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
            self.look_up_current(cx);
        } else if was_open {
            self.finish_closing(cx);
        }
        self.sync_refresh(cx);
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

    /// Checks whether the lines should change for as long as the view is open
    /// and the song moving. Otherwise only the player's own changes move them.
    fn sync_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.opening.is_open || !self.player.read(cx).is_playing() {
            self.refresh = None;
        } else if self.refresh.is_none() {
            self.refresh = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(HIGHLIGHT_REFRESH).await;
                    let refreshed = this.update(cx, |this, cx| this.refresh_lit(cx));
                    if refreshed.is_err() {
                        break;
                    }
                }
            }));
        }
    }

    fn refresh_lit(&mut self, cx: &mut Context<Self>) {
        let Lookup::Done(_, lyrics) = &self.lookup else {
            return;
        };
        let Lyrics::Timed(lyrics) = &**lyrics else {
            return;
        };
        let lit = Lit::of(lyrics, lines_position(self.player.read(cx)), self.highlight);
        if lit != self.lit {
            cx.notify();
        }
    }

    fn is_current_song_looked_up(&self, cx: &App) -> bool {
        self.lookup.song() == self.player.read(cx).current_song().map(Song::id)
    }

    /// Looks for the lyrics of the current song, unless they are on show or on
    /// their way. A failed search is tried again.
    fn look_up_current(&mut self, cx: &mut Context<Self>) {
        let Some(song) = self.player.read(cx).current_song().cloned() else {
            self.lookup = Lookup::Nothing;
            cx.notify();
            return;
        };
        let is_failed = matches!(self.lookup, Lookup::Failed(..));
        if self.lookup.song() == Some(song.id()) && !is_failed {
            return;
        }
        self.show_new_lines();
        let id = song.id().clone();
        if let Some((_, lyrics)) = self.found.iter().find(|(found, _)| *found == id) {
            self.lookup = Lookup::Done(id, lyrics.clone());
            cx.notify();
            return;
        }
        self.lookup = Lookup::Loading(id.clone());
        let found = cx.background_spawn((self.find_lyrics)(query_for(&song)));
        // It goes on when the song changes: the token it may be waiting for is
        // what the next song needs, and asking for another could be refused.
        cx.spawn(async move |this, cx| {
            let found = found.await;
            this.update(cx, |this, cx| this.finish_lookup(id, found, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

    fn finish_lookup(
        &mut self,
        song: SongId,
        found: Result<Lyrics, lyrics::Error>,
        cx: &mut Context<Self>,
    ) {
        let is_awaited = matches!(&self.lookup, Lookup::Loading(awaited) if *awaited == song);
        let lookup = match found {
            Ok(lyrics) => {
                let lyrics = self.keep_found(&song, lyrics);
                Lookup::Done(song, lyrics)
            }
            Err(error) => Lookup::Failed(song, describe(&error).into()),
        };
        if is_awaited {
            self.lookup = lookup;
            self.show_new_lines();
            cx.notify();
        }
    }

    fn keep_found(&mut self, song: &SongId, lyrics: Lyrics) -> Arc<Lyrics> {
        if let Some((_, kept)) = self.found.iter().find(|(found, _)| found == song) {
            return kept.clone();
        }
        let lyrics = Arc::new(lyrics);
        if self.found.len() >= FOUND_KEPT {
            self.found.remove(0);
        }
        self.found.push((song.clone(), lyrics.clone()));
        lyrics
    }

    fn show_new_lines(&mut self) {
        self.is_following = true;
        self.scroll_key = self.scroll_key.wrapping_add(1);
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.lit = Lit::default();
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.look_up_current(cx);
    }

    fn follow_again(&mut self, cx: &mut Context<Self>) {
        self.is_following = true;
        cx.notify();
    }

    fn stop_following(&mut self, cx: &mut Context<Self>) {
        if self.is_following {
            self.is_following = false;
            cx.notify();
        }
    }

    fn seek_to(&mut self, start: Duration, window: &mut Window, cx: &mut Context<Self>) {
        self.is_following = true;
        self.player
            .update(cx, |player, cx| player.seek_to(start, window, cx));
        cx.notify();
    }

    /// The layout comes from the last frame, so nothing moves until there has been one.
    fn follow_current_line(
        &self,
        lyrics: &TimedLyrics,
        position: Duration,
        window: &mut Window,
        cx: &mut App,
    ) {
        let id = ElementId::from(("lyrics-scroll", self.scroll_key));
        if !self.is_following {
            // Easing state lasts only while each frame asks for it, and following
            // again starts from where the user left the lines.
            let top = -f32::from(self.scroll.offset().y);
            transition(id, top, Transition::new(Duration::ZERO), window, cx);
            return;
        }
        // The room before the lines is the list's first child
        let child_ix = lyrics.line_at(position).unwrap_or(0) + 1;
        let Some(line) = self.scroll.bounds_for_item(child_ix) else {
            window.request_animation_frame();
            return;
        };
        let list = self.scroll.bounds();
        let top = (line.center().y - list.top() - list.size.height * CURRENT_LINE_AT)
            .max(px(0.))
            .min(self.scroll.max_offset().y.abs());
        let motion = cx.theme().motion_tokens();
        let policy = Transition::new(SCROLL_DURATION).easing(motion.easing_move.clone());
        let top: f32 = transition(id, f32::from(top), policy, window, cx);
        self.scroll.set_offset(point(px(0.), px(-top)));
    }

    fn render_lines(
        &mut self,
        lyrics: &TimedLyrics,
        stage: &Stage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let player = self.player.read(cx);
        let position = lines_position(player);
        let is_playing = player.is_playing();
        self.lit = Lit::of(lyrics, position, self.highlight);
        self.follow_current_line(lyrics, position, window, cx);

        let theme = cx.theme();
        let (dim, bright, hover) = (
            theme.muted_foreground,
            theme.foreground,
            theme.muted.opacity(0.7),
        );
        let radius = theme.radius_lg;
        let highlight = self.highlight;
        let mut is_fading = false;
        let lines = lyrics.lines().iter().enumerate().map(|(ix, line)| {
            let (runs, fading) = line_runs(line, position, is_playing, highlight, dim, bright);
            is_fading |= fading;
            let start = line.start();
            div()
                .id(("lyrics-line", ix))
                .role(Role::Button)
                .aria_label(SharedString::from(line.text().clone()))
                .flex_none()
                .max_w(rems(LINE_MAX_WIDTH))
                .px_3()
                .py_1p5()
                .rounded(radius)
                .hover(move |this| this.bg(hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.seek_to(start, window, cx);
                }))
                .child(StyledText::new(line.text().clone()).with_highlights(runs))
                .into_any_element()
        });
        let lines: Vec<AnyElement> = lines.collect();
        // A frame for each step of a fade, and between fades the refresh checks
        if is_fading && is_playing {
            window.request_animation_frame();
        }
        let is_following = self.is_following;
        let copyright = lyrics.copyright().cloned();
        let scrollbar_from = self.scroll.bounds().right() - window.rem_size();
        div()
            .size_full()
            .relative()
            .capture_any_mouse_down(cx.listener(
                move |this, event: &gpui_kit::MouseDownEvent, _, cx| {
                    if event.position.x >= scrollbar_from {
                        this.stop_following(cx);
                    }
                },
            ))
            .child(
                v_flex()
                    .id("lyrics-lines")
                    .role(Role::List)
                    .aria_label("Lyrics")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pl_4()
                    .pr_10()
                    .gap_1()
                    .text_color(dim)
                    .font_semibold()
                    .line_height(rems(if stage.has_large_type { 2.5 } else { 2. }))
                    .map(|this| {
                        if stage.has_large_type {
                            this.text_3xl()
                        } else {
                            this.text_2xl()
                        }
                    })
                    .on_scroll(cx.listener(|this, _, _, cx| this.stop_following(cx)))
                    .child(div().flex_none().h(stage.room_before_lines))
                    .children(lines)
                    .children(copyright.map(copyright_line))
                    .child(div().flex_none().h(stage.room_after_lines)),
            )
            .child(Scrollbar::vertical(&self.scroll))
            .when(!is_following, |this| {
                this.child(
                    h_flex()
                        .absolute()
                        .bottom_6()
                        .left_0()
                        .right_0()
                        .justify_center()
                        .child(
                            div()
                                .debug_selector(|| "lyrics-back-to-current".to_owned())
                                .key_context(CONTROL_CONTEXT)
                                // A click here is not for the line behind
                                .block_mouse_except_scroll()
                                .child(
                                    Button::new("back-to-current-line").label("Sync").on_click(
                                        cx.listener(|this, _, _, cx| this.follow_again(cx)),
                                    ),
                                ),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_plain(&self, text: SharedString, copyright: Option<Arc<str>>) -> AnyElement {
        div()
            .size_full()
            .relative()
            .child(
                v_flex()
                    .id("lyrics-text")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pl_7()
                    .pr_10()
                    .py_8()
                    .text_lg()
                    .child(div().flex_none().max_w(rems(LINE_MAX_WIDTH)).child(text))
                    .children(copyright.map(copyright_line)),
            )
            .child(Scrollbar::vertical(&self.scroll))
            .into_any_element()
    }

    fn render_lyrics(
        &mut self,
        stage: &Stage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let empty =
            |text: &'static str, cx: &App| centered(muted_text(text, cx)).into_any_element();
        match &self.lookup {
            Lookup::Nothing => div().into_any_element(),
            Lookup::Loading(_) => {
                centered(section_loading("Loading lyrics", cx)).into_any_element()
            }
            Lookup::Failed(_, reason) => {
                let retry = Button::new("retry-lyrics")
                    .label("Retry")
                    .on_click(cx.listener(|this, _, _, cx| this.retry(cx)));
                v_flex()
                    .size_full()
                    .justify_center()
                    .child(section_failure(reason.clone(), retry, cx))
                    .into_any_element()
            }
            Lookup::Done(_, lyrics) => {
                let lyrics = lyrics.clone();
                match &*lyrics {
                    Lyrics::Timed(lyrics) => self.render_lines(lyrics, stage, window, cx),
                    Lyrics::Plain(plain) => self.render_plain(
                        SharedString::from(plain.text().clone()),
                        plain.copyright().cloned(),
                    ),
                    Lyrics::Instrumental => empty("This song is instrumental.", cx),
                    Lyrics::Restricted => empty("The lyrics of this song aren't available.", cx),
                    Lyrics::NotFound => empty("No lyrics found for this song.", cx),
                }
            }
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
        let viewport = window.viewport_size();
        let room = size(viewport.width, viewport.height - TITLE_BAR_HEIGHT);
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
        let lyrics = self.render_lyrics(&stage, window, cx);
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
                    .child(lyrics),
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

/// Which line a position lights and how much of it, for telling when that
/// changes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Lit {
    line: Option<usize>,
    elements: usize,
    has_ended: bool,
}

impl Lit {
    fn of(lyrics: &TimedLyrics, position: Duration, highlight: LyricsHighlight) -> Self {
        let Some(ix) = lyrics.line_at(position) else {
            return Self::default();
        };
        let line = &lyrics.lines()[ix];
        Self {
            line: Some(ix),
            elements: match highlight {
                LyricsHighlight::ByWord => line
                    .elements()
                    .partition_point(|element| element.start() <= position),
                LyricsHighlight::ByLine => 0,
            },
            has_ended: line.end().is_some_and(|end| end <= position),
        }
    }
}

fn lines_position(player: &Player) -> Duration {
    player.shown_position() + SEEK_ROUNDING
}

/// How lit a part of a line starting at `start` is at `position`, from `0.0` to
/// `1.0`. It lights up as it starts and dims when its line ends, over time if the
/// song is playing. A song standing still would hold it partway.
fn emphasis(line: &TimedLine, start: Duration, position: Duration, is_playing: bool) -> f32 {
    let Some(lit_for) = position.checked_sub(start) else {
        return 0.;
    };
    let ended_for = line.end().and_then(|end| position.checked_sub(end));
    if !is_playing {
        return if ended_for.is_some() { 0. } else { 1. };
    }
    let rise = (lit_for.as_secs_f32() / ELEMENT_FADE.as_secs_f32()).min(1.);
    let fall = ended_for.map_or(0., |ended_for| {
        (ended_for.as_secs_f32() / LINE_FADE.as_secs_f32()).min(1.)
    });
    rise * (1. - fall)
}

/// The colors of the lit parts of `line`, and whether any is changing.
fn line_runs(
    line: &TimedLine,
    position: Duration,
    is_playing: bool,
    highlight: LyricsHighlight,
    dim: Hsla,
    bright: Hsla,
) -> (Vec<(Range<usize>, HighlightStyle)>, bool) {
    let mut is_fading = false;
    let mut runs = Vec::new();
    let mut light = |range: Range<usize>, start: Duration| {
        let lit = emphasis(line, start, position, is_playing);
        is_fading |= lit > 0. && lit < 1.;
        if lit > 0. {
            let color = dim.blend(bright.opacity(lit));
            let style = HighlightStyle {
                color: Some(color),
                ..HighlightStyle::default()
            };
            runs.push((range, style));
        }
    };
    match highlight {
        LyricsHighlight::ByWord => {
            for element in line.elements() {
                light(element.range(), element.start());
            }
        }
        LyricsHighlight::ByLine => light(0..line.text().len(), line.start()),
    }
    (runs, is_fading)
}

fn copyright_line(text: Arc<str>) -> impl IntoElement {
    div()
        .flex_none()
        .max_w(rems(LINE_MAX_WIDTH))
        .px_3()
        .pt_6()
        .text_xs()
        .line_height(rems(1.25))
        .font_normal()
        .child(SharedString::from(text))
}

fn query_for(song: &Song) -> TrackQuery {
    TrackQuery::new(song.title())
        .artist(song.artist())
        .album_artist(song.album_artist())
        .album(song.album())
        // A server that does not know a song's length may say zero
        .duration(song.duration().filter(|duration| !duration.is_zero()))
}

fn describe(error: &lyrics::Error) -> String {
    match error {
        lyrics::Error::Network(_) => {
            "Couldn't load lyrics. Check your connection and try again.".to_owned()
        }
        lyrics::Error::HttpStatus(_)
        | lyrics::Error::Api { .. }
        | lyrics::Error::InvalidResponse
        | lyrics::Error::TokenUnavailable => {
            "Couldn't load lyrics. The lyrics services aren't answering as they should, so try again later."
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests;
