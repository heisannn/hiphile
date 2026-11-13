//! The lyrics over all of the window: the cover, which flies there from the bar and back, what
//! plays and its controls under it, and the lines beside it.

use std::cell::{Cell, RefCell};

use anti_webui::core::geometry::RectF;
use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::motion::{default_spatial, slow_effects};
use anti_webui_md3e::typography::TextStyle;
use next_host::follow;
use next_ui::cover::{cover_over, picture_at_once};
use next_ui::keymap::{
    FocusBefore, KeyBinding, KeyContext, Keystroke, bind_keys, is_bound, key_context,
};
use next_ui::window::moves_window;
use next_ui::{Command, ScreenContext};
use playback::PlayerSnapshot;
use subsonic::Song;

use crate::bar::{
    COVER_CORNER as BAR_COVER_CORNER, controls, cover_toggle, format_text, lyrics_toggle,
    picture_of, queue_toggle, song_favorite, song_words,
};
use crate::cover::Cover;
use crate::panels::Panels;
use crate::position::{LANE, position_line};
use crate::volume::volume;

mod lines;

const LYRICS_CONTEXT: &str = "Lyrics";

/// The strip along the top, where the window's buttons are and dragging moves the window.
const TOP: f32 = 44.0;
/// The width of the column with the cover: the part of the window it takes, and the least and
/// the most it may be.
const COLUMN_SHARE: f32 = 0.38;
const COLUMN_MIN: f32 = 340.0;
const COLUMN_MAX: f32 = 520.0;
/// At either side of what is in the column.
const COLUMN_INSET: f32 = 40.0;
/// The least above and below what is in the column.
const COLUMN_MARGIN: f32 = 24.0;
const DETAILS_GAP: f32 = 20.0;
/// What is under the cover: the song and the favorite, the position, the controls, and the
/// format with the panels and the volume, with the gaps between them.
const DETAILS_HEIGHT: f32 = 200.0;
/// What the controls take in a row, which the details are never narrower than.
const DETAILS_MIN_WIDTH: f32 = 280.0;
const COVER_MIN: f32 = 96.0;
/// The extra-large corners of a large cover.
const COVER_CORNER: f32 = 28.0;
/// What the cover is asked for at when it shows large.
const LARGE_COVER_PIXELS: u32 = 512;
/// How far the details and the lines rise as they come.
const RISE: f32 = 24.0;
/// Of the way to opaque, where the backdrop has hidden what is under it.
const BACKDROP: std::ops::Range<f32> = 0.0..0.5;
const DETAILS: std::ops::Range<f32> = 0.3..0.9;
const LINES: std::ops::Range<f32> = 0.4..1.0;

#[derive(Clone, Debug, PartialEq)]
enum LyricsKey {
    Close,
    TogglePlayback,
}

/// Once for each thread, as its keymap is.
fn bind_lyrics_keys() {
    if is_bound::<LyricsKey>() {
        return;
    }
    bind_keys([
        KeyBinding::new(
            Keystroke::named(NamedKey::Escape),
            LyricsKey::Close,
            LYRICS_CONTEXT,
        ),
        KeyBinding::new(
            Keystroke::named(NamedKey::Space),
            LyricsKey::TogglePlayback,
            LYRICS_CONTEXT,
        ),
    ]);
}

/// The lyrics of the workspace's player while `panels` asks for them, over all of what is beside
/// it in the shell, which lays it out at the window's size. Its cover stands in for the bar's,
/// whose `cover` it shows. The focus goes into it as it opens and back as it closes. While
/// `reduces_motion`, nothing moves on the way.
pub fn lyrics_overlay(
    cx: &ScreenContext,
    cover: Signal<Option<Cover>>,
    panels: Panels,
    reduces_motion: Signal<bool>,
) -> Show {
    bind_lyrics_keys();
    let open = panels.lyrics;
    let flight = animated(0.0_f32);
    let fade = animated(0.0_f32);
    let wanted = Cell::new(0.0);
    effect(move || {
        let target = if open.get() { 1.0 } else { 0.0 };
        if wanted.replace(target) == target {
            return;
        }
        untrack(|| {
            if reduces_motion.get() {
                flight.set(target);
                fade.set(target);
            } else {
                flight.spring_to(target, default_spatial());
                fade.spring_to(target, slow_effects());
            }
        });
    });
    let (shown, covering) = (panels.lyrics_shown, panels.lyrics_cover);
    // One step for the overlay and the bar's cover, so that a frame never has both covers or
    // neither
    effect(move || {
        let (open, flight, fade) = (open.get(), flight.get(), fade.get());
        untrack(|| {
            shown.set_if_changed(open || flight != 0.0 || fade != 0.0);
            covering.set_if_changed(open && fade >= BACKDROP.end);
        });
    });
    // Kept while it is closed, so that it opens again at its size
    let room = signal(None::<(f32, f32)>);
    let cx = cx.clone();
    show(
        move || shown.get(),
        move || {
            let way = Way { flight, fade, room };
            overlay(&cx, cover, panels, way, reduces_motion)
        },
    )
}

/// How far the overlay has come, and the room it has.
#[derive(Clone, Copy)]
struct Way {
    /// From the bar, at `0.0`, to its place, at `1.0`: the cover. A spring, which may go past.
    flight: Animated<f32>,
    /// From nothing to all of it: the rest.
    fade: Animated<f32>,
    room: Signal<Option<(f32, f32)>>,
}

impl Way {
    fn stage(&self) -> Stage {
        let (width, height) = self.room.get().unwrap_or((0.0, 0.0));
        Stage::new(width, height)
    }

    fn part(&self, part: std::ops::Range<f32>) -> f32 {
        through(self.fade.get(), part)
    }
}

fn through(progress: f32, part: std::ops::Range<f32>) -> f32 {
    ((progress - part.start) / (part.end - part.start)).clamp(0.0, 1.0)
}

fn overlay(
    cx: &ScreenContext,
    cover: Signal<Option<Cover>>,
    panels: Panels,
    way: Way,
    reduces_motion: Signal<bool>,
) -> Div {
    let player = cx.workspace.player().clone();
    let playing = follow(player.subscribe());
    let root = node_ref();
    let has_focus = signal(false);
    let has_song = memo(move || playing.with(PlayerSnapshot::has_song));
    let commands = cx.commands.clone();
    let keys = KeyContext::new(
        LYRICS_CONTEXT,
        move |key| match key {
            LyricsKey::Close => true,
            // A focused button takes Space for itself
            LyricsKey::TogglePlayback => has_focus.get() && has_song.get(),
        },
        move |key| match key {
            LyricsKey::Close => panels.lyrics.set(false),
            LyricsKey::TogglePlayback => commands.send(Command::TogglePlayback),
        },
    );
    let backdrop = div()
        .absolute()
        .inset(0.0_f32)
        .dyn_style(move |s| s.bg(color_scheme().surface).opacity(way.part(BACKDROP)));
    let lines = div()
        .absolute()
        .top(TOP)
        .right(0.0_f32)
        .bottom(0.0_f32)
        .dyn_style(move |s| {
            let shown = way.part(LINES);
            s.left(way.stage().column)
                .opacity(shown)
                .translate_y(RISE * (1.0 - shown))
        })
        .child(lines::lines(
            player.clone(),
            playing,
            cx.workspace.lyrics().clone(),
            panels.lyrics,
            reduces_motion,
        ));
    let details = details(cx, player, playing, panels).dyn_style(move |s| {
        let shown = way.part(DETAILS);
        let at = way.stage().details;
        s.left(at.x)
            .top(at.y)
            .w(at.width)
            .opacity(shown)
            .translate_y(RISE * (1.0 - shown))
    });
    let view = key_context(
        div()
            .absolute()
            .inset(0.0_f32)
            .focusable()
            .node_ref(root)
            .access(|a| a.role(Role::Dialog).label("Lyrics"))
            .on_resized(move |width, height| way.room.set(Some((width, height))))
            .on_focus_change(move |on| has_focus.set(on)),
        &keys,
    )
    .child(backdrop)
    .child(moves_window(
        div()
            .absolute()
            .top(0.0_f32)
            .left(0.0_f32)
            .right(0.0_f32)
            .h(TOP),
    ))
    .child(lines)
    .child(details)
    .child(flying_cover(cx, cover, panels, way, root))
    .into_view();
    take_focus(root, panels.lyrics);
    div().absolute().inset(0.0_f32).child(view)
}

/// Gives the focus to `root` while `open`, and back to what had it before as it closes.
fn take_focus(root: NodeRef, open: Signal<bool>) {
    let before = std::rc::Rc::new(RefCell::new(None::<FocusBefore>));
    let kept = before.clone();
    on_cleanup(move || {
        if let Some(before) = kept.borrow_mut().take() {
            before.give_back();
        }
    });
    effect(move || {
        let open = open.get();
        untrack(|| {
            let mut before = before.borrow_mut();
            if open && before.is_none() {
                *before = Some(FocusBefore::now());
                root.focus();
            } else if !open && let Some(before) = before.take() {
                before.give_back();
            }
        });
    });
}

/// Where what is in the overlay goes, in a window of the overlay's size.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stage {
    /// How wide the column with the cover is, from the window's start.
    column: f32,
    cover: RectF,
    details: RectF,
}

impl Stage {
    /// Layout cannot say where a cover between two places should be, so this does.
    fn new(width: f32, height: f32) -> Self {
        let room = height - TOP;
        let column = (width * COLUMN_SHARE).clamp(COLUMN_MIN, COLUMN_MAX);
        let inner = column - 2.0 * COLUMN_INSET;
        let side = inner
            .min(room - (2.0 * COLUMN_MARGIN + DETAILS_GAP + DETAILS_HEIGHT))
            .max(COVER_MIN)
            .floor();
        let taken = side + DETAILS_GAP + DETAILS_HEIGHT;
        let top = TOP + ((room - taken) / 2.0).max(0.0).floor();
        let left = ((column - side) / 2.0).floor();
        let width = side.max(DETAILS_MIN_WIDTH);
        Self {
            column,
            cover: RectF::new(left, top, side, side),
            details: RectF::new(
                ((column - width) / 2.0).floor(),
                top + side + DETAILS_GAP,
                width,
                DETAILS_HEIGHT,
            ),
        }
    }
}

fn between(from: f32, to: f32, moved: f32) -> f32 {
    from + (to - from) * moved
}

fn rect_between(from: RectF, to: RectF, moved: f32) -> RectF {
    RectF::new(
        between(from.x, to.x, moved),
        between(from.y, to.y, moved),
        between(from.width, to.width, moved),
        between(from.height, to.height, moved),
    )
}

/// The bar's cover at first, with the large one over it once that has come. It flies from where
/// the bar's is, in the overlay `root`, and is drawn at once, as it is in view already.
fn flying_cover(
    cx: &ScreenContext,
    cover: Signal<Option<Cover>>,
    panels: Panels,
    way: Way,
    root: NodeRef,
) -> Div {
    let session = cx.workspace.session().clone();
    let playing = follow(cx.workspace.player().subscribe());
    let large = memo(move || {
        playing.with(|shown| {
            shown
                .current_song()
                .and_then(Song::cover_art)
                .map(|cover| session.cover_art_url(cover, LARGE_COVER_PIXELS))
        })
    });
    let large = dyn_view(move || match large.get() {
        Some(url) => untrack(|| cover_over(url).absolute().inset(0.0_f32).into_view()),
        None => untrack(|| div().into_view()),
    });
    let picture = div()
        .relative()
        .size_full()
        .child(picture_of(cover, |cover| {
            picture_at_once(move || cover.image())
                .size_full()
                .into_view()
        }))
        .child(large);
    let moved = move || way.flight.get().clamp(0.0, 1.0);
    let corner = move || between(BAR_COVER_CORNER, COVER_CORNER, moved());
    // Where the bar's cover was last laid out, which it stays at once the bar has none
    let bar = Cell::new(RectF::new(0.0, 0.0, 0.0, 0.0));
    cover_toggle(picture, corner, panels.lyrics)
        .absolute()
        .dyn_style(move |s| {
            let to = way.stage().cover;
            let from = untrack(|| bar_cover_in(panels.bar_cover, root)).unwrap_or(bar.get());
            bar.set(from);
            let at = rect_between(from, to, way.flight.get());
            s.left(at.x).top(at.y).w(at.width).h(at.height)
        })
}

/// Where the bar's cover is in the overlay.
fn bar_cover_in(bar_cover: NodeRef, root: NodeRef) -> Option<RectF> {
    let cover = bar_cover.logical_rect()?;
    let (x, y) = root
        .logical_rect()
        .map_or((0.0, 0.0), |root| (root.x, root.y));
    Some(RectF::new(
        cover.x - x,
        cover.y - y,
        cover.width,
        cover.height,
    ))
}

/// What plays and what controls it, under the cover.
fn details(
    cx: &ScreenContext,
    player: playback::Player,
    playing: Signal<PlayerSnapshot>,
    panels: Panels,
) -> Div {
    let has_song = memo(move || playing.with(PlayerSnapshot::has_song));
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(8.0_f32)
        .child(song_words(playing, TextStyle::TITLE_LARGE).flex_1())
        .child(song_favorite(cx, playing));
    let position = {
        let player = player.clone();
        div().h(LANE).mt(16.0_f32).child(show(
            move || has_song.get(),
            move || position_line(player.clone(), playing, || false),
        ))
    };
    let panels_and_volume = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(4.0_f32)
        .child(lyrics_toggle(playing, panels))
        .child(queue_toggle(panels))
        .child(volume(player.clone(), playing));
    let foot = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(8.0_f32)
        .mt(8.0_f32)
        .child(format_text(playing))
        .child(panels_and_volume);
    div()
        .absolute()
        .flex()
        .flex_col()
        .access(|a| a.role(Role::Group).label("Player"))
        .child(header)
        .child(position)
        .child(controls(player, playing).mt(8.0_f32))
        .child(foot)
}

#[cfg(test)]
mod tests;
