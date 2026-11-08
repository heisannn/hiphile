use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use config::LyricsHighlight;
use gpui_kit::base::{Transition, transition};
use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _, button::Button, h_flex, scroll::Scrollbar, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, HighlightStyle, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledText, Subscription, Task, Window, div,
    point, prelude::FluentBuilder as _, px, rems,
};
use lyrics::timing::{Lit, SEEK_ROUNDING, emphasis};
use lyrics::{CurrentLyrics, Lookup, Lyrics, TimedLine, TimedLyrics};
use playback::PlayerSnapshot;
use subsonic::Song;
use ui::list_navigation::CONTROL_CONTEXT;
use ui::{centered, muted_text, section_failure, section_loading};

use crate::player::Player;

pub(super) const SCROLL_DURATION: Duration = Duration::from_millis(460);
/// How often to check whether the lines should change, as the player's own
/// refresh is too slow for parts of a line.
const HIGHLIGHT_REFRESH: Duration = Duration::from_millis(50);
/// How far down the lines the current one rests, as a part of their height.
pub(super) const CURRENT_LINE_AT: f32 = 0.36;
/// How much room follows the last line, as a part of the height of the lines,
/// for it to reach where the current line rests.
const ROOM_AFTER_LINES: f32 = 0.5;
/// In rems.
const LINE_MAX_WIDTH: f32 = 42.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LinesType {
    Small,
    Medium,
    Large,
}

/// How to set the lines in the window they show in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LinesFit {
    pub(crate) height: Pixels,
    pub(crate) line_type: LinesType,
}

/// The lines of the current lyrics, kept on the current one, or what is there
/// instead of them. One for each place that shows them, as each scrolls apart.
pub(crate) struct LyricsLines {
    player: Entity<Player>,
    pub(super) lyrics: CurrentLyrics,
    fit: fn(&Window) -> LinesFit,
    /// Whether they are on show, and so need looking up and keeping current.
    is_active: bool,
    pub(super) scroll: ScrollHandle,
    /// Whether the lines keep the current one in view, until the user scrolls.
    pub(super) is_following: bool,
    /// Changes with each new set of lines, which then take their place at once.
    scroll_key: usize,
    /// Which line and how much of it the last frame lit.
    pub(super) lit: Lit,
    pub(super) refresh: Option<Task<()>>,
    _player_changes: Task<()>,
    _lyrics_changes: Task<()>,
    _subscription: Subscription,
}

impl LyricsLines {
    pub(crate) fn new(
        player: Entity<Player>,
        lyrics: CurrentLyrics,
        fit: fn(&Window) -> LinesFit,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&player, |_, _, cx| cx.notify());
        let changes = lyrics.subscribe();
        let mut shown = changes.borrow().lookup().clone();
        // Another lookup brings another set of lines, or what stands in for them
        let lyrics_changes = gpui_host::observe(changes, cx, move |this, lyrics, cx| {
            if *lyrics.lookup() != shown {
                shown = lyrics.lookup().clone();
                this.show_new_lines();
            }
            cx.notify();
        });
        let changes = player.read(cx).core().subscribe();
        let now_playing = |shown: &PlayerSnapshot| {
            let song = shown.current_song().map(|song| song.id().clone());
            (song, shown.is_playing())
        };
        let mut followed = now_playing(&changes.borrow());
        let player_changes = gpui_host::observe(changes, cx, move |this, shown, cx| {
            if now_playing(&shown) == followed {
                return;
            }
            followed = now_playing(&shown);
            this.sync_refresh(cx);
            // Pausing is no cause to look again
            if this.is_active && !this.is_current_song_looked_up(cx) {
                this.look_up_current(cx);
            }
        });
        Self {
            player,
            lyrics,
            fit,
            is_active: false,
            scroll: ScrollHandle::new(),
            is_following: true,
            scroll_key: 0,
            lit: Lit::default(),
            refresh: None,
            _player_changes: player_changes,
            _lyrics_changes: lyrics_changes,
            _subscription: subscription,
        }
    }

    pub(crate) fn set_active(&mut self, is_active: bool, cx: &mut Context<Self>) {
        self.is_active = is_active;
        if is_active {
            self.look_up_current(cx);
        }
        self.sync_refresh(cx);
    }

    fn look_up_current(&self, cx: &App) {
        self.lyrics
            .look_up(self.player.read(cx).snapshot().current_song());
    }

    fn is_current_song_looked_up(&self, cx: &App) -> bool {
        self.lyrics.snapshot().lookup().song()
            == self.player.read(cx).snapshot().current_song().map(Song::id)
    }

    /// Checks whether the lines should change for as long as the view is open
    /// and the song moving. Otherwise only the player's own changes move them.
    fn sync_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.is_active || !self.player.read(cx).snapshot().is_playing() {
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
        let current = self.lyrics.snapshot();
        let Lookup::Done(_, lyrics) = current.lookup() else {
            return;
        };
        let Lyrics::Timed(lyrics) = &**lyrics else {
            return;
        };
        let lit = Lit::of(
            lyrics,
            lines_position(self.player.read(cx)),
            current.highlight(),
        );
        if lit != self.lit {
            cx.notify();
        }
    }

    fn show_new_lines(&mut self) {
        self.is_following = true;
        self.scroll_key = self.scroll_key.wrapping_add(1);
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.lit = Lit::default();
    }

    pub(super) fn retry(&mut self, cx: &mut Context<Self>) {
        self.look_up_current(cx);
    }

    pub(super) fn follow_again(&mut self, cx: &mut Context<Self>) {
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
        fit: LinesFit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let player = self.player.read(cx);
        let position = lines_position(player);
        let is_playing = player.snapshot().is_playing();
        let highlight = self.lyrics.snapshot().highlight();
        self.lit = Lit::of(lyrics, position, highlight);
        self.follow_current_line(lyrics, position, window, cx);

        let theme = cx.theme();
        let (dim, bright, hover) = (
            theme.muted_foreground,
            theme.foreground,
            theme.muted.opacity(0.7),
        );
        let radius = theme.radius_lg;
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
                    .text_color(dim)
                    .font_semibold()
                    .map(|this| match fit.line_type {
                        // The line's own inset brings its text to the window's
                        LinesType::Small => this.pl_1().pr_4().text_lg().line_height(rems(1.5)),
                        LinesType::Medium => {
                            this.pl_4().pr_10().gap_1().text_2xl().line_height(rems(2.))
                        }
                        LinesType::Large => this
                            .pl_4()
                            .pr_10()
                            .gap_1()
                            .text_3xl()
                            .line_height(rems(2.5)),
                    })
                    .on_scroll(cx.listener(|this, _, _, cx| this.stop_following(cx)))
                    .child(div().flex_none().h(fit.height * CURRENT_LINE_AT))
                    .children(lines)
                    .children(copyright.map(copyright_line))
                    .child(div().flex_none().h(fit.height * ROOM_AFTER_LINES)),
            )
            .child(Scrollbar::vertical(&self.scroll))
            .when(!is_following, |this| {
                this.child(
                    h_flex()
                        .absolute()
                        .map(|this| match fit.line_type {
                            LinesType::Small => this.bottom_2(),
                            LinesType::Medium | LinesType::Large => this.bottom_6(),
                        })
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

    fn render_plain(
        &self,
        text: SharedString,
        copyright: Option<Arc<str>>,
        fit: LinesFit,
    ) -> AnyElement {
        div()
            .size_full()
            .relative()
            .child(
                v_flex()
                    .id("lyrics-text")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .map(|this| match fit.line_type {
                        LinesType::Small => this.p_4().text_sm(),
                        LinesType::Medium | LinesType::Large => {
                            this.pl_7().pr_10().py_8().text_lg()
                        }
                    })
                    .child(div().flex_none().max_w(rems(LINE_MAX_WIDTH)).child(text))
                    .children(copyright.map(copyright_line)),
            )
            .child(Scrollbar::vertical(&self.scroll))
            .into_any_element()
    }
}

impl Render for LyricsLines {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fit = (self.fit)(window);
        let empty =
            |text: &'static str, cx: &App| centered(muted_text(text, cx)).into_any_element();
        let shown = self.lyrics.snapshot();
        match shown.lookup() {
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
                    .child(section_failure(
                        SharedString::from(reason.clone()),
                        retry.debug_selector(|| "lyrics-retry".to_owned()),
                        cx,
                    ))
                    .into_any_element()
            }
            Lookup::Done(_, lyrics) => {
                let lyrics = lyrics.clone();
                match &*lyrics {
                    Lyrics::Timed(lyrics) => self.render_lines(lyrics, fit, window, cx),
                    Lyrics::Plain(plain) => self.render_plain(
                        SharedString::from(plain.text().clone()),
                        plain.copyright().cloned(),
                        fit,
                    ),
                    Lyrics::Instrumental => empty("This song is instrumental.", cx),
                    Lyrics::Restricted => empty("The lyrics of this song aren't available.", cx),
                    Lyrics::NotFound => empty("No lyrics found for this song.", cx),
                }
            }
        }
    }
}

fn lines_position(player: &Player) -> Duration {
    player.shown_position() + SEEK_ROUNDING
}

/// The colors of the lit parts of `line`, and whether any is changing.
pub(super) fn line_runs(
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
