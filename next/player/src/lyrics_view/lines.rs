//! The lines of the lyrics, kept on the one being sung, or what stands in for them.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anti_webui::prelude::*;
use anti_webui_md3e::color::ColorScheme;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{ButtonStyle, Ink, styled_button};
use anti_webui_md3e::motion::default_spatial;
use anti_webui_md3e::typography::TextStyle;
use lyrics::timing::{Lit, LitLine, lit_lines};
use lyrics::{CurrentLyrics, Lookup, Lyrics, LyricsSnapshot, PlainLyrics, TimedLyrics, Wanted};
use next_host::follow;
use next_ui::load_state::{empty_state, load_failure, pane_loading};
use next_ui::type_scale::{muted_text, styled_text};
use playback::PlayerSnapshot;

/// How often the position is read while the lines show: the parts of a line light at finer steps
/// than the bar moves.
pub(super) const REFRESH: Duration = Duration::from_millis(50);
/// How far down the lines the one being sung rests, as a part of their height.
pub(super) const CURRENT_LINE_AT: f32 = 0.36;
/// The room after the last line, as a part of the height of the lines, for it to reach where the
/// one being sung rests.
const ROOM_AFTER_LINES: f32 = 0.5;
const LINE_MAX_WIDTH: f32 = 680.0;
/// Before the lines, which the inset of each brings to the column's edge, and after them, where
/// the scrollbar is.
const START: f32 = 16.0;
const END: f32 = 40.0;
/// Of Roboto Flex's grade, which makes letters bolder without making them wider, at the most a
/// lit part has. Hiragino Sans has no grade, which leaves what is not Latin to the color.
const LIT_GRADE: f32 = 100.0;
/// What a part has of its own color before it lights.
const DIM_ALPHA: f32 = 0.6;

/// The lines of the song `player` is on, looked up while `open`, or what stands in for them.
/// While `reduces_motion`, they jump to the line being sung.
pub(super) fn lines(
    player: playback::Player,
    playing: Signal<PlayerSnapshot>,
    lyrics: CurrentLyrics,
    open: Signal<bool>,
    reduces_motion: Signal<bool>,
) -> Div {
    want_while(&lyrics, open);
    let shown = follow(lyrics.subscribe());
    let lookup = memo(move || shown.with(|shown| shown.lookup().clone()));
    let highlight = memo(move || shown.with(LyricsSnapshot::highlight));
    let position = signal(player.position());
    keep_reading(player.clone(), position);
    let is_playing = memo(move || playing.with(PlayerSnapshot::is_playing));
    let song = Playing {
        player,
        position,
        is_playing,
    };
    div().size_full().flex().flex_col().child(dyn_view(move || {
        let lookup = lookup.get();
        untrack(|| match lookup {
            Lookup::Nothing => div().into_view(),
            Lookup::Loading(_) => pane_loading("Loading lyrics").into_view(),
            Lookup::Failed(_, reason) => {
                let lyrics = lyrics.clone();
                load_failure("Lyrics didn't load", reason.to_string(), move || {
                    lyrics.retry()
                })
                .into_view()
            }
            Lookup::Done(_, found) => match &*found {
                Lyrics::Timed(timed) => timed_lines(
                    Rc::new(timed.clone()),
                    song.clone(),
                    highlight,
                    reduces_motion,
                )
                .into_view(),
                Lyrics::Plain(plain) => plain_text(plain).into_view(),
                Lyrics::Instrumental => {
                    empty_state("This song is instrumental.", None, None).into_view()
                }
                Lyrics::Restricted => {
                    empty_state("The lyrics of this song aren't available.", None, None).into_view()
                }
                Lyrics::NotFound => {
                    empty_state("No lyrics found for this song.", None, None).into_view()
                }
            },
        })
    }))
}

/// Holds what looks the lyrics up for as long as `open`, and while the current owner lives.
fn want_while(lyrics: &CurrentLyrics, open: Signal<bool>) {
    let held: Rc<RefCell<Option<Wanted>>> = Rc::default();
    let dropped = held.clone();
    on_cleanup(move || drop(dropped.borrow_mut().take()));
    let lyrics = lyrics.clone();
    effect(move || {
        let open = open.get();
        let mut held = held.borrow_mut();
        if open != held.is_some() {
            *held = open.then(|| lyrics.want());
        }
    });
}

/// What the lines follow of the player.
#[derive(Clone)]
struct Playing {
    player: playback::Player,
    position: Signal<Duration>,
    is_playing: Memo<bool>,
}

impl Playing {
    fn read(&self) {
        read(&self.player, self.position);
    }
}

/// Keeps `position` to where the song is, reading it every [`REFRESH`] while the current owner
/// lives.
fn keep_reading(player: playback::Player, position: Signal<Duration>) {
    let next: Rc<Cell<Option<Timeout>>> = Rc::default();
    let called_off = next.clone();
    on_cleanup(move || {
        if let Some(next) = called_off.take() {
            next.cancel();
        }
    });
    read_after(player, position, next);
}

fn read(player: &playback::Player, position: Signal<Duration>) {
    let now = player.position();
    if position.get_untracked() != now {
        position.set(now);
    }
}

/// Timers run outside any owner, which keeps no cleanup for them: each one after the first is
/// called off through `next` alone.
fn read_after(
    player: playback::Player,
    position: Signal<Duration>,
    next: Rc<Cell<Option<Timeout>>>,
) {
    let again = next.clone();
    next.set(Some(set_timeout(REFRESH, move || {
        if !position.is_alive() {
            return;
        }
        read(&player, position);
        read_after(player, position, again);
    })));
}

/// While a part is between dim and lit, each frame reads where the song is, so that it fades
/// smoothly between the reads of [`keep_reading`].
fn read_while_fading(song: Playing, is_fading: Memo<bool>) {
    // Gone with the memo, which cannot say so itself
    let alive = signal(());
    let running = Rc::new(Cell::new(false));
    effect(move || {
        if is_fading.get() && !running.replace(true) {
            untrack(|| read_each_frame(song.clone(), is_fading, alive, running.clone()));
        }
    });
}

fn read_each_frame(
    song: Playing,
    is_fading: Memo<bool>,
    alive: Signal<()>,
    running: Rc<Cell<bool>>,
) {
    request_frame(move |_| {
        if !alive.is_alive() || !untrack(|| is_fading.get()) {
            running.set(false);
            return;
        }
        song.read();
        read_each_frame(song, is_fading, alive, running);
    });
}

/// What the lines scroll in, the lines with what is before and after them, and the room before
/// them, which is as tall as the list makes it.
#[derive(Clone, Copy)]
struct ListNodes {
    list: NodeRef,
    words: NodeRef,
    room_before: NodeRef,
}

/// Whether the lines keep the one being sung in place, and how far they have gone there.
#[derive(Clone, Copy)]
struct Following {
    /// Until the user scrolls them, and again once they ask.
    on: Signal<bool>,
    scroll: Animated<f32>,
    /// The lines have been put in place once, after which they move there.
    placed: Signal<bool>,
}

fn timed_lines(
    lyrics: Rc<TimedLyrics>,
    song: Playing,
    highlight: Memo<config::LyricsHighlight>,
    reduces_motion: Signal<bool>,
) -> Div {
    let (position, is_playing) = (song.position, song.is_playing);
    let current = {
        let lyrics = lyrics.clone();
        memo(move || Lit::of(&lyrics, position.get(), highlight.get()).line)
    };
    let lit = {
        let lyrics = lyrics.clone();
        memo(move || lit_lines(&lyrics, position.get(), is_playing.get(), highlight.get()))
    };
    read_while_fading(song.clone(), memo(move || lit.with(|lit| lit.is_fading())));

    let nodes = ListNodes {
        list: node_ref(),
        words: node_ref(),
        room_before: node_ref(),
    };
    let scroller = nodes.list;
    let height = signal(0.0_f32);
    let following = Following {
        on: signal(true),
        scroll: animated(0.0_f32),
        placed: signal(false),
    };
    let line_nodes: Rc<[NodeRef]> = lyrics.lines().iter().map(|_| node_ref()).collect();
    effect(move || {
        let y = following.scroll.get();
        untrack(|| scroller.scroll_to(0.0, y));
    });
    {
        let line_nodes = line_nodes.clone();
        effect(move || {
            let line = current.get().unwrap_or(0);
            let (on, height) = (following.on.get(), height.get());
            if on && height > 0.0 {
                untrack(|| follow_line(line_nodes[line], nodes, height, following, reduces_motion));
            }
        });
    }
    let stop_following = move || {
        if following.on.get_untracked() {
            following.on.set(false);
            following.scroll.stop();
        }
    };

    let lines = lyrics.lines().iter().enumerate().map(|(ix, line)| {
        let lit_line = memo(move || lit.with(|lit| lit.line(ix).cloned()));
        let start = line.start();
        let player = song.player.clone();
        line_view(line.text().clone(), lit_line, line_nodes[ix]).on_click(move |_| {
            if let Some(at) = player.seek_to(start) {
                position.set(at);
            }
            following.on.set(true);
        })
    });
    let words = TextStyle::HEADLINE_LARGE
        .apply(div())
        .node_ref(nodes.words)
        .flex()
        .flex_col()
        .items_start()
        .gap(4.0_f32)
        .pl(START)
        .pr(END)
        .access(|a| a.role(Role::Group))
        .dyn_style(move |s| s.text_color(dim(&color_scheme())))
        .child(
            div()
                .flex_none()
                .node_ref(nodes.room_before)
                .dyn_style(move |s| s.h(height.get() * CURRENT_LINE_AT)),
        )
        .children(lines)
        .children(lyrics.copyright().cloned().map(copyright))
        .child(
            div()
                .flex_none()
                .dyn_style(move |s| s.h(height.get() * ROOM_AFTER_LINES)),
        );
    let list = div()
        .size_full()
        .node_ref(scroller)
        .overflow_y_scroll()
        .on_resized(move |_, h| height.set_if_changed(h))
        // Before it scrolls, which takes the lines from the one being sung
        .on_scroll(move |_| stop_following())
        .on_pointer_down(move |e| {
            if scroller.is_over_scrollbar(e.position) {
                stop_following();
            }
        })
        .child(words);
    div().relative().size_full().child(list).child(show(
        move || !following.on.get(),
        move || sync_button(scroller, following),
    ))
}

/// Scrolls the lines so that `line` rests where the one being sung does: at once the first time
/// and while motion is reduced, and else from where they are. The layout comes from the last
/// frame, so until there is one it waits for the next.
fn follow_line(
    line: NodeRef,
    nodes: ListNodes,
    height: f32,
    following: Following,
    reduces_motion: Signal<bool>,
) {
    let scroller = nodes.list;
    let laid_out = nodes
        .room_before
        .logical_rect()
        .is_some_and(|room| (room.height - height * CURRENT_LINE_AT).abs() < 1.0);
    let (true, Some(at), Some(rect), Some(whole)) = (
        laid_out,
        line.offset_in(scroller),
        line.logical_rect(),
        nodes.words.logical_rect(),
    ) else {
        request_frame(move |_| {
            if following.on.is_alive() && following.on.get_untracked() {
                follow_line(line, nodes, height, following, reduces_motion);
            }
        });
        return;
    };
    let most = (whole.height - height).max(0.0);
    let target = (at.y + rect.height / 2.0 - height * CURRENT_LINE_AT).clamp(0.0, most);
    let scroll = following.scroll;
    if !following.placed.get_untracked() || reduces_motion.get_untracked() {
        following.placed.set(true);
        scroll.set(target);
        return;
    }
    // From where the user left them
    let now = scroller.scroll_offset().y;
    if (scroll.get_untracked() - now).abs() >= 0.5 {
        scroll.set(now);
    }
    scroll.spring_to(target, default_spatial());
}

/// What a part has of its own color before it lights.
fn dim(c: &ColorScheme) -> Color {
    c.on_surface_variant.with_alpha(DIM_ALPHA)
}

/// A line, which shows what of it is lit by color, and by grade where its letters have one.
/// Choosing it plays the song from where it starts.
fn line_view(text: Arc<str>, lit: Memo<Option<LitLine>>, node: NodeRef) -> Div {
    let words = {
        let text = text.clone();
        dyn_view(move || {
            let lit = lit.get();
            // Only a lit line has colors of its own, which change with the scheme
            let colors = lit.as_ref().map(|_| color_scheme());
            untrack(|| line_runs(&text, lit.as_ref(), colors.as_ref()))
        })
    };
    let ink = Ink::new(|c| c.on_surface, || false);
    let root = div()
        .node_ref(node)
        .flex_none()
        .max_w(LINE_MAX_WIDTH)
        .px(12.0_f32)
        .py(6.0_f32)
        .rounded(16.0_f32)
        .cursor(CursorIcon::Pointer)
        .select_none()
        .access(move |a| a.role(Role::Button).label(text.to_string()))
        .child(words);
    ink.paint(ink.listen(root))
}

fn line_runs(text: &str, lit: Option<&LitLine>, colors: Option<&ColorScheme>) -> RichText {
    let (Some(lit), Some(c)) = (lit, colors) else {
        return rich_text((text.to_owned(),));
    };
    let (dim, bright) = (dim(c), c.on_surface);
    let runs: Vec<Run> = lit_parts(text.len(), lit)
        .into_iter()
        .map(|(range, emphasis)| {
            let words = &text[range];
            if emphasis == 0.0 {
                return words.to_owned().into();
            }
            span(words)
                .text_color(dim.mix(bright, emphasis))
                .font_axis(b"GRAD", LIT_GRADE * emphasis)
                .into()
        })
        .collect();
    rich_text(runs)
}

/// The whole of a line `len` bytes long in parts, each with how lit it is, `0.0` for dim.
fn lit_parts(len: usize, lit: &LitLine) -> Vec<(Range<usize>, f32)> {
    let mut parts = Vec::new();
    let mut at = 0;
    for part in lit.parts() {
        let range = part.range();
        if range.start > at {
            parts.push((at..range.start, 0.0));
        }
        at = range.end;
        parts.push((range, part.emphasis()));
    }
    if at < len {
        parts.push((at..len, 0.0));
    }
    parts
}

fn copyright(text: Arc<str>) -> Div {
    muted_text(TextStyle::BODY_SMALL, text.to_string())
        .flex_none()
        .max_w(LINE_MAX_WIDTH)
        .px(12.0_f32)
        .pt(24.0_f32)
}

/// Takes the lines back to the one being sung. A click on it is not for the line behind, but the
/// wheel over it scrolls them.
fn sync_button(scroller: NodeRef, following: Following) -> Div {
    let button = styled_button("Sync", ButtonStyle::tonal())
        .pointer_events_auto()
        .on_click(move |_| following.on.set(true));
    div()
        .absolute()
        .left(0.0_f32)
        .right(0.0_f32)
        .bottom(24.0_f32)
        .flex()
        .justify_center()
        .pointer_events_none()
        .on_scroll(move |e| {
            let by = untrack(window_scale).to_logical(e.delta_y);
            scroller.scroll_to(0.0, scroller.scroll_offset().y + by);
        })
        .child(button)
}

/// Plain text, which scrolls as the user likes.
fn plain_text(plain: &PlainLyrics) -> Div {
    let words = styled_text(TextStyle::TITLE_LARGE, plain.text().to_string())
        .flex_none()
        .max_w(LINE_MAX_WIDTH)
        .px(12.0_f32);
    div().size_full().overflow_y_scroll().child(
        div()
            .flex()
            .flex_col()
            .items_start()
            .pl(START)
            .pr(END)
            .py(32.0_f32)
            .dyn_style(|s| s.text_color(color_scheme().on_surface))
            .child(words)
            .children(plain.copyright().cloned().map(copyright)),
    )
}

#[cfg(test)]
mod tests;
