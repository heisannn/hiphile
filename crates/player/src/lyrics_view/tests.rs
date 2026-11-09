use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gpui_kit::{Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point};
use serde_json::json;

use config::LyricsHighlight;
use futures::FutureExt as _;
use hiphile_core::favorites::Favorite;
use lyrics::timing::lit_lines;
use lyrics::{FindLyrics, Lookup, Lyrics};

use super::lines::{CURRENT_LINE_AT, SCROLL_DURATION, line_runs};
use super::*;
use crate::CloseLyrics;

const ROOM: Size<Pixels> = Size {
    width: px(1200.),
    height: px(726.),
};
const REM: Pixels = px(16.);

fn secs(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}

fn song(id: &str) -> Song {
    serde_json::from_value(json!({
        "id": id,
        "title": format!("Title of {id}"),
        "artist": "Artist",
        "album": "Album",
    }))
    .unwrap()
}

/// Lyrics of `lines` lines, five seconds apart from ten seconds in.
fn lyrics(lines: usize) -> Lyrics {
    let body: Vec<_> = (0..lines)
        .map(|ix| {
            let start = 10. + ix as f64 * 5.;
            json!({
                "ts": start,
                "te": start + 4.5,
                "x": format!("line {ix}"),
                "l": [
                    { "c": "line ", "o": 0.0 },
                    { "c": format!("{ix}"), "o": 2.0 },
                ],
            })
        })
        .collect();
    let body = serde_json::Value::Array(body).to_string();
    lyrics::lyrics_from_richsync_body(&body)
}

/// The shell's arrangement: the room under a title bar, with the screen and the
/// bar in it and the view over them.
struct Stand {
    player: Entity<Player>,
    view: Entity<LyricsView>,
    focus: FocusHandle,
    /// What lookups were asked about, and what they find; once those run out,
    /// a lookup finds nothing for as long as the test goes on.
    finds: Finds,
    _songs: Task<()>,
}

#[derive(Clone, Default)]
struct Finds {
    asked: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<VecDeque<Result<Lyrics, lyrics::Error>>>>,
}

impl Finds {
    fn answer_with(&self, answer: Result<Lyrics, lyrics::Error>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    fn source(&self) -> FindLyrics {
        let finds = self.clone();
        Arc::new(move |query| {
            finds.asked.lock().unwrap().push(format!("{query:?}"));
            match finds.answers.lock().unwrap().pop_front() {
                Some(answer) => futures::future::ready(answer).boxed(),
                None => futures::future::pending().boxed(),
            }
        })
    }
}

impl Render for Stand {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let reveal = self.view.read(cx).reveal(cx);
        if reveal.is_moving {
            window.request_animation_frame();
        }
        v_flex()
            .id("stand")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(|this, _: &ToggleLyrics, _, cx| {
                let is_open = this.view.read(cx).is_open();
                this.view.update(cx, |view, cx| view.set_open(!is_open, cx));
            }))
            .on_action(cx.listener(|this, _: &CloseLyrics, _, cx| {
                this.view.update(cx, |view, cx| view.set_open(false, cx));
            }))
            .child(div().flex_none().h(TITLE_BAR_HEIGHT))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(!reveal.is_covering, |this| {
                        this.child(
                            v_flex()
                                .size_full()
                                .opacity(reveal.under_opacity)
                                .child(div().flex_1().min_h_0())
                                .child(self.player.clone()),
                        )
                    })
                    .when(reveal.is_shown, |this| this.child(self.view.clone())),
            )
    }
}

/// A view of a player that plays nothing, so nothing reaches a device or the
/// server, on a song, in a window of `width` by `height`.
fn stand(
    width: f32,
    height: f32,
    cx: &mut TestAppContext,
) -> (Entity<Stand>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let finds = Finds::default();
    let (stand, cx) = cx.add_window_view(|window, cx| {
        let player = cx.new(|cx| {
            let mut player = crate::testing::idle_player(window, cx);
            player.fill_queue(&[song("a"), song("b")], 0);
            player
        });
        let lyrics = CurrentLyrics::with_find_lyrics(gpui_host::executor(cx), finds.source());
        let songs = crate::testing::follow_songs(player.read(cx).core(), &lyrics, cx);
        let view = cx.new(|cx| LyricsView::new(player.clone(), lyrics, cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Stand {
            player,
            view,
            focus,
            finds,
            _songs: songs,
        }
    });
    cx.simulate_resize(size(px(width), px(height)));
    cx.run_until_parked();
    (stand, cx)
}

fn view(stand: &Entity<Stand>, cx: &mut VisualTestContext) -> Entity<LyricsView> {
    cx.read(|cx| stand.read(cx).view.clone())
}

fn lines<'a>(view: &Entity<LyricsView>, cx: &'a App) -> &'a LyricsLines {
    view.read(cx).lines.read(cx)
}

fn lookup(view: &Entity<LyricsView>, cx: &App) -> Lookup {
    lines(view, cx).lyrics.snapshot().lookup().clone()
}

fn update_lines<R>(
    view: &Entity<LyricsView>,
    cx: &mut VisualTestContext,
    update: impl FnOnce(&mut LyricsLines, &mut Context<LyricsLines>) -> R,
) -> R {
    let lines = cx.read(|cx| view.read(cx).lines.clone());
    lines.update(cx, update)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn advance(duration: Duration, cx: &mut VisualTestContext) {
    cx.executor().advance_clock(duration);
    draw(cx);
}

fn set_open(stand: &Entity<Stand>, is_open: bool, cx: &mut VisualTestContext) {
    let view = view(stand, cx);
    view.update(cx, |view, cx| view.set_open(is_open, cx));
    draw(cx);
}

fn open_with(stand: &Entity<Stand>, found: Lyrics, cx: &mut VisualTestContext) {
    cx.read(|cx| stand.read(cx).finds.clone())
        .answer_with(Ok(found));
    set_open(stand, true, cx);
    advance(OPEN_DURATION, cx);
    draw(cx);
}

fn cover(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds("lyrics-cover").expect("the view's cover")
}

fn assert_near(a: Bounds<Pixels>, b: Bounds<Pixels>) {
    let near = |a: Pixels, b: Pixels| (a - b).abs() <= px(1.);
    assert!(
        near(a.left(), b.left())
            && near(a.top(), b.top())
            && near(a.size.width, b.size.width)
            && near(a.size.height, b.size.height),
        "{a:?} is not near {b:?}"
    );
}

#[gpui_kit::test]
fn the_cover_leaves_from_the_bar_and_returns_to_it(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    draw(cx);
    let in_bar = cx.debug_bounds("player-cover").expect("the bar's cover");
    assert!(cx.debug_bounds("lyrics-cover").is_none());

    set_open(&stand, true, cx);
    assert_near(cover(cx), in_bar);

    advance(OPEN_DURATION / 4, cx);
    let partway = cover(cx);
    let large = Stage::new(ROOM, REM).cover;
    assert!(in_bar.size.width < partway.size.width && partway.size.width < large.size.width);
    assert!(partway.top() < in_bar.top());

    advance(OPEN_DURATION, cx);
    assert_eq!(cover(cx).size, large.size);
    assert_eq!(cover(cx).left(), large.left());
    assert_eq!(cover(cx).top(), large.top() + TITLE_BAR_HEIGHT);
    // Nothing out of sight is left to take Tab or a frame
    assert!(cx.debug_bounds("player-cover").is_none());

    set_open(&stand, false, cx);
    assert!(cx.debug_bounds("player-cover").is_some());
    advance(CLOSE_DURATION, cx);
    assert!(cx.debug_bounds("lyrics-cover").is_none());
    assert_eq!(cx.debug_bounds("player-cover"), Some(in_bar));
}

#[gpui_kit::test]
fn the_cover_keeps_rounded_corners_all_the_way(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        let (bar, large) = (cx.theme().radius, cx.theme().radius_lg);
        assert!(bar > px(0.));
        assert_eq!(cover_radius(0., cx), bar);
        assert_eq!(cover_radius(1., cx), large);
        let midway = cover_radius(0.5, cx);
        assert!(bar <= midway && midway <= large, "{midway:?}");
    });
}

#[gpui_kit::test]
fn only_one_cover_shows_while_it_moves(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let player = cx.read(|cx| stand.read(cx).player.clone());
    let is_hidden = |cx: &mut VisualTestContext| cx.read(|cx| player.read(cx).is_cover_hidden());
    draw(cx);
    assert!(!is_hidden(cx));

    set_open(&stand, true, cx);
    assert!(is_hidden(cx));
    advance(OPEN_DURATION, cx);
    assert!(is_hidden(cx));

    set_open(&stand, false, cx);
    advance(CLOSE_DURATION - Duration::from_millis(1), cx);
    assert!(cx.debug_bounds("lyrics-cover").is_some());
    assert!(is_hidden(cx));

    // It comes back as the other goes, with no frame without either
    advance(Duration::from_millis(1), cx);
    assert!(cx.debug_bounds("lyrics-cover").is_none());
    assert!(!is_hidden(cx));

    // Turning back keeps the view from ending when closing would have
    set_open(&stand, true, cx);
    advance(OPEN_DURATION, cx);
    set_open(&stand, false, cx);
    set_open(&stand, true, cx);
    advance(CLOSE_DURATION, cx);
    assert!(cx.debug_bounds("lyrics-cover").is_some());
    assert!(is_hidden(cx));
}

#[gpui_kit::test]
fn turning_back_midway_continues_from_where_it_was(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    set_open(&stand, true, cx);
    advance(OPEN_DURATION / 3, cx);
    let at_turn = cover(cx);

    set_open(&stand, false, cx);
    assert_near(cover(cx), at_turn);

    // Only as long as what is left of the way, not all of closing
    advance(CLOSE_DURATION / 2, cx);
    assert!(cx.debug_bounds("lyrics-cover").is_none());
}

#[gpui_kit::test]
fn with_motion_reduced_it_goes_at_once(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    cx.update(|_, cx| cx.set_reduce_motion(true));

    set_open(&stand, true, cx);
    assert_eq!(cover(cx).size, Stage::new(ROOM, REM).cover.size);
    assert!(cx.debug_bounds("player-cover").is_none());

    set_open(&stand, false, cx);
    assert!(cx.debug_bounds("lyrics-cover").is_none());
}

#[gpui_kit::test]
fn what_is_under_the_cover_fits_in_the_smallest_window(cx: &mut TestAppContext) {
    let (stand, cx) = stand(900., 560., cx);
    set_open(&stand, true, cx);
    advance(OPEN_DURATION, cx);

    let details = cx.debug_bounds("lyrics-details").expect("the details");
    assert_eq!(details.left(), cover(cx).left());
    assert_eq!(details.size.width, cover(cx).size.width);
    assert!(details.top() >= cover(cx).bottom());
    assert!(details.bottom() <= px(560.), "{details:?}");

    // The room set aside for them is what they take
    let reserved = details.top() + REM * DETAILS_HEIGHT;
    assert!(details.bottom() <= reserved, "{details:?}");
    assert!(reserved - details.bottom() < REM, "{details:?}");
}

#[gpui_kit::test]
fn the_star_ends_the_details_and_asks_for_the_song(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let asked = Arc::new(Mutex::new(Vec::new()));
    cx.update(|_, cx| {
        actions::set_known_favorites(Some(Default::default()), cx);
        let asked = asked.clone();
        cx.on_action(move |action: &actions::ToggleFavoriteOf, _| {
            asked.lock().unwrap().push(action.clone());
        });
    });
    set_open(&stand, true, cx);
    advance(OPEN_DURATION, cx);

    let star = cx.debug_bounds("lyrics-favorite").expect("the star");
    let details = cx.debug_bounds("lyrics-details").expect("the details");
    assert_eq!(star.right(), details.right());
    assert!(star.top() >= details.top());

    cx.simulate_click(star.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        *asked.lock().unwrap(),
        [actions::ToggleFavoriteOf::new(Favorite::Song(song("a")))]
    );
}

#[gpui_kit::test]
fn the_cover_and_the_bar_toggle_the_view(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    draw(cx);

    let in_bar = cx.debug_bounds("player-cover").unwrap();
    cx.simulate_click(in_bar.center(), Modifiers::default());
    draw(cx);
    assert!(cx.read(|cx| view.read(cx).is_open()));

    advance(OPEN_DURATION, cx);
    let large = cover(cx);
    cx.simulate_click(large.center(), Modifiers::default());
    draw(cx);
    assert!(!cx.read(|cx| view.read(cx).is_open()));
}

#[gpui_kit::test]
fn escape_closes(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    cx.update(|_, cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "escape",
            CloseLyrics,
            Some(LYRICS_CONTEXT),
        )]);
    });
    set_open(&stand, true, cx);
    advance(OPEN_DURATION, cx);
    cx.update(|window, cx| {
        let focus = view.read(cx).focus.clone();
        focus.focus(window, cx);
    });
    draw(cx);

    cx.simulate_keystrokes("escape");
    assert!(!cx.read(|cx| view.read(cx).is_open()));
}

#[gpui_kit::test]
fn the_current_line_comes_to_rest_at_its_place(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    open_with(&stand, lyrics(40), cx);

    let player = cx.read(|cx| stand.read(cx).player.clone());
    player.update(cx, |player, _| player.place_at(secs(100.)));
    draw(cx);
    advance(SCROLL_DURATION * 2, cx);

    let scroll = cx.read(|cx| lines(&view, cx).scroll.clone());
    let list = scroll.bounds();
    // The room before the lines is the first child, so line 18 is child 19
    let line = scroll.bounds_for_item(19).unwrap();
    let rests_at = line.center().y + scroll.offset().y - list.top();
    let wanted = list.size.height * CURRENT_LINE_AT;
    assert!(
        (rests_at - wanted).abs() <= px(1.),
        "{rests_at:?} {wanted:?}"
    );

    // On the way to a far line, it moves but has not arrived
    player.update(cx, |player, _| player.place_at(secs(150.)));
    draw(cx);
    let before = scroll.offset().y;
    advance(SCROLL_DURATION / 4, cx);
    let partway = scroll.offset().y;
    assert!(partway < before);
    advance(SCROLL_DURATION, cx);
    assert!(scroll.offset().y < partway);
}

#[gpui_kit::test]
fn scrolling_stops_following_until_asked_to_or_a_line_is_chosen(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    open_with(&stand, lyrics(40), cx);
    assert!(cx.debug_bounds("lyrics-back-to-current").is_none());

    let scroll = cx.read(|cx| lines(&view, cx).scroll.clone());
    let over_lines = scroll.bounds().center();
    cx.simulate_event(ScrollEvent {
        position: over_lines,
        delta: ScrollDelta::Pixels(point(px(0.), px(-400.))),
        ..ScrollEvent::default()
    });
    draw(cx);
    assert!(!cx.read(|cx| lines(&view, cx).is_following));
    let scrolled_to = scroll.offset().y;
    assert!(scrolled_to < px(0.));
    // The lines stay where the user took them as the song goes on
    let player = cx.read(|cx| stand.read(cx).player.clone());
    player.update(cx, |player, _| player.place_at(secs(100.)));
    advance(SCROLL_DURATION, cx);
    assert_eq!(scroll.offset().y, scrolled_to);

    let back = cx
        .debug_bounds("lyrics-back-to-current")
        .expect("the button");
    cx.simulate_click(back.center(), Modifiers::default());
    assert!(cx.read(|cx| lines(&view, cx).is_following));
    draw(cx);
    assert!(cx.debug_bounds("lyrics-back-to-current").is_none());

    // Choosing a line follows again too
    cx.simulate_event(ScrollEvent {
        position: over_lines,
        delta: ScrollDelta::Pixels(point(px(0.), px(40.))),
        ..ScrollEvent::default()
    });
    draw(cx);
    assert!(!cx.read(|cx| lines(&view, cx).is_following));
    cx.simulate_click(over_lines, Modifiers::default());
    assert!(cx.read(|cx| lines(&view, cx).is_following));
}

#[gpui_kit::test]
fn the_sync_button_sits_over_the_lines_and_keeps_its_click_from_them(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    let player = cx.read(|cx| stand.read(cx).player.clone());
    let long_song = serde_json::from_value(json!({
        "id": "long",
        "title": "Long",
        "duration": 1000,
    }))
    .unwrap();
    player.update(cx, |player, cx| {
        player.fill_queue(&[long_song], 0);
        player.pretend_playing(true, cx);
    });
    cx.run_until_parked();
    open_with(&stand, lyrics(40), cx);

    let scroll = cx.read(|cx| lines(&view, cx).scroll.clone());
    cx.simulate_event(ScrollEvent {
        position: scroll.bounds().center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-400.))),
        ..ScrollEvent::default()
    });
    draw(cx);

    let button = cx
        .debug_bounds("lyrics-back-to-current")
        .expect("the button");
    let center = button.center();
    assert!((center.x - scroll.bounds().center().x).abs() < px(1.));
    assert!(button.bottom() <= scroll.bounds().bottom());
    let line_behind = (0..scroll.children_count())
        .filter_map(|ix| scroll.bounds_for_item(ix))
        .any(|bounds| bounds.contains(&center));
    assert!(line_behind, "a line is behind the button");

    let shown = |cx: &mut VisualTestContext| {
        cx.read(|cx| player.read(cx).position_state().read(cx).value().end())
    };
    let before = shown(cx);
    cx.simulate_click(center, Modifiers::default());
    assert!(cx.read(|cx| lines(&view, cx).is_following));
    assert_eq!(shown(cx), before, "the line behind was not sought to");
}

#[gpui_kit::test]
fn the_wheel_over_the_sync_button_still_scrolls_the_lines(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    open_with(&stand, lyrics(40), cx);

    let scroll = cx.read(|cx| lines(&view, cx).scroll.clone());
    cx.simulate_event(ScrollEvent {
        position: scroll.bounds().center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-400.))),
        ..ScrollEvent::default()
    });
    draw(cx);
    let scrolled_to = scroll.offset().y;

    let button = cx
        .debug_bounds("lyrics-back-to-current")
        .expect("the button");
    cx.simulate_event(ScrollEvent {
        position: button.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
        ..ScrollEvent::default()
    });
    draw(cx);
    assert!(scroll.offset().y < scrolled_to);
}

#[gpui_kit::test]
fn opening_looks_for_the_current_song_once(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let finds = cx.read(|cx| stand.read(cx).finds.clone());
    assert!(finds.asked().is_empty(), "closed, nothing is looked for");

    set_open(&stand, true, cx);
    set_open(&stand, false, cx);
    set_open(&stand, true, cx);

    assert_eq!(finds.asked().len(), 1);
    assert!(finds.asked()[0].contains("Title of a"));
}

#[gpui_kit::test]
fn all_that_show_the_lyrics_share_one_lookup(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    let finds = cx.read(|cx| stand.read(cx).finds.clone());
    let player = cx.read(|cx| stand.read(cx).player.clone());
    let current = cx.read(|cx| lines(&view, cx).lyrics.clone());
    let other =
        cx.update(|_, cx| cx.new(|cx| LyricsLines::new(player.clone(), current, lines_fit, cx)));

    open_with(&stand, lyrics(3), cx);
    other.update(cx, |other, cx| other.set_active(true, cx));
    draw(cx);
    assert_eq!(finds.asked().len(), 1);

    // Either finding new lines puts both back on the current one
    update_lines(&view, cx, |lines, _| lines.is_following = false);
    other.update(cx, |other, _| other.is_following = false);
    player.update(cx, |player, cx| {
        player.fill_queue(&[song("b"), song("a")], 0);
        player.pretend_playing(true, cx);
    });
    draw(cx);
    assert_eq!(finds.asked().len(), 2);
    assert!(cx.read(|cx| lines(&view, cx).is_following));
    assert!(cx.read(|cx| other.read(cx).is_following));
}

#[gpui_kit::test]
fn a_failed_lookup_says_so_and_can_be_retried(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    let finds = cx.read(|cx| stand.read(cx).finds.clone());
    finds.answer_with(Err(lyrics::Error::InvalidResponse));
    set_open(&stand, true, cx);
    advance(OPEN_DURATION, cx);
    cx.read(|cx| assert!(matches!(lookup(&view, cx), Lookup::Failed(..))));

    finds.answer_with(Ok(lyrics(2)));
    update_lines(&view, cx, |lines, _| lines.retry());
    draw(cx);
    cx.read(|cx| assert!(matches!(lookup(&view, cx), Lookup::Done(..))));
    assert_eq!(finds.asked().len(), 2);
}

#[gpui_kit::test]
fn pausing_or_resuming_does_not_look_again_but_a_new_song_does(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    let finds = cx.read(|cx| stand.read(cx).finds.clone());
    let player = cx.read(|cx| stand.read(cx).player.clone());
    finds.answer_with(Err(lyrics::Error::InvalidResponse));
    set_open(&stand, true, cx);
    advance(OPEN_DURATION, cx);
    cx.read(|cx| assert!(matches!(lookup(&view, cx), Lookup::Failed(..))));

    player.update(cx, |player, cx| player.pretend_playing(true, cx));
    player.update(cx, |player, cx| player.pretend_playing(false, cx));
    draw(cx);
    cx.read(|cx| assert!(matches!(lookup(&view, cx), Lookup::Failed(..))));
    assert_eq!(finds.asked().len(), 1);

    player.update(cx, |player, cx| {
        player.fill_queue(&[song("b"), song("a")], 0);
        player.pretend_playing(true, cx);
    });
    draw(cx);
    assert_eq!(finds.asked().len(), 2);
}

#[gpui_kit::test]
fn the_refresh_runs_only_while_open_and_playing(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    let player = cx.read(|cx| stand.read(cx).player.clone());
    let is_running = |cx: &mut VisualTestContext| cx.read(|cx| lines(&view, cx).refresh.is_some());

    player.update(cx, |player, cx| player.pretend_playing(true, cx));
    draw(cx);
    assert!(!is_running(cx), "closed");

    set_open(&stand, true, cx);
    assert!(is_running(cx));

    player.update(cx, |player, cx| player.pretend_playing(false, cx));
    draw(cx);
    assert!(!is_running(cx), "paused");

    player.update(cx, |player, cx| player.pretend_playing(true, cx));
    draw(cx);
    assert!(is_running(cx));

    set_open(&stand, false, cx);
    assert!(!is_running(cx), "closing");
}

#[gpui_kit::test]
fn the_view_says_why_the_song_stopped(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let player = cx.read(|cx| stand.read(cx).player.clone());
    open_with(&stand, lyrics(2), cx);
    assert!(cx.debug_bounds("player-error").is_none());

    player.update(cx, |player, cx| {
        player.set_error("Couldn't play it.");
        cx.notify();
    });
    draw(cx);
    let error = cx.debug_bounds("player-error").expect("the reason");
    let details = cx.debug_bounds("lyrics-details").unwrap();
    assert!(details.contains(&error.origin));
}

#[gpui_kit::test]
fn following_again_eases_from_where_the_user_left_the_lines(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    let player = cx.read(|cx| stand.read(cx).player.clone());
    open_with(&stand, lyrics(40), cx);
    let scroll = cx.read(|cx| lines(&view, cx).scroll.clone());
    cx.simulate_event(ScrollEvent {
        position: scroll.bounds().center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-200.))),
        ..ScrollEvent::default()
    });
    draw(cx);
    let left_at = scroll.offset().y;
    player.update(cx, |player, _| player.place_at(secs(150.)));

    update_lines(&view, cx, |lines, cx| lines.follow_again(cx));
    draw(cx);
    assert!((scroll.offset().y - left_at).abs() < px(1.));

    advance(SCROLL_DURATION / 2, cx);
    let midway = scroll.offset().y;
    assert!(midway < left_at);
    advance(SCROLL_DURATION, cx);
    assert!(scroll.offset().y < midway);
}

#[test]
fn opening_takes_as_long_as_is_left_when_it_turns_back() {
    let start = Instant::now();
    let motion = MotionTokens::default();
    let mut opening = Opening::closed(start);
    assert_eq!(opening.progress(start), 0.);

    opening.set_open(true, false, &motion, start);
    let halfway = start + OPEN_DURATION / 2;
    assert!((opening.progress(halfway) - 0.5).abs() < 1e-3);
    assert_eq!(opening.progress(start + OPEN_DURATION), 1.);

    opening.set_open(false, false, &motion, halfway);
    assert!((opening.progress(halfway) - 0.5).abs() < 1e-3);
    let closed_at = halfway + CLOSE_DURATION / 2;
    assert!(opening.progress(closed_at - Duration::from_millis(1)) > 0.);
    assert_eq!(opening.progress(closed_at), 0.);
}

#[test]
fn opening_again_while_open_does_not_start_over() {
    let start = Instant::now();
    let motion = MotionTokens::default();
    let mut opening = Opening::closed(start);
    opening.set_open(true, false, &motion, start);
    let halfway = start + OPEN_DURATION / 2;
    opening.set_open(true, false, &motion, halfway);
    assert_eq!(opening.progress(start + OPEN_DURATION), 1.);
}

#[test]
fn closing_moves_the_cover_at_once_and_settles_softly() {
    let start = Instant::now();
    let motion = MotionTokens::default();
    let mut opening = Opening::closed(start);
    opening.set_open(true, false, &motion, start);
    let opened_at = start + OPEN_DURATION;
    assert_eq!(opening.shown(&motion, opened_at).cover, 1.);

    opening.set_open(false, false, &motion, opened_at);
    let at = |part: f32| opening.shown(&motion, opened_at + CLOSE_DURATION.mul_f32(part));
    assert!(
        at(0.05).cover < 1.,
        "it has not started: {}",
        at(0.05).cover
    );
    assert!(at(0.05).lines < 1. && at(0.05).details < 1.);
    assert_eq!(at(0.05).backdrop, 1.);

    let (early, middle, late) = (at(0.25).cover, at(0.5).cover, at(0.75).cover);
    assert!(middle < 0.5, "over half way at half the time: {middle}");
    assert!(early - middle > middle - late, "{early} {middle} {late}");
    assert_eq!(at(1.), Shown::default());
}

#[test]
fn turning_back_past_a_parts_share_lets_it_take_the_rest_of_the_way() {
    let start = Instant::now();
    let motion = MotionTokens::default();
    let mut opening = Opening::closed(start);
    opening.set_open(true, false, &motion, start);
    let opened_at = start + OPEN_DURATION;
    opening.set_open(false, false, &motion, opened_at);

    // Back to opening while the progress is past where the cover's share of
    // opening ends, with the cover well on its way back.
    let turned_at = opened_at + CLOSE_DURATION.mul_f32(0.2);
    let before = opening.shown(&motion, turned_at);
    assert!(opening.progress(turned_at) > OPENING_PARTS.cover.end);
    assert!(before.cover < 1.);
    opening.set_open(true, false, &motion, turned_at);
    assert!((opening.shown(&motion, turned_at).cover - before.cover).abs() < 1e-3);

    let later = opening
        .shown(&motion, turned_at + Duration::from_millis(40))
        .cover;
    assert!(
        before.cover < later && later < 1.,
        "{} {later}",
        before.cover
    );
    assert_eq!(opening.shown(&motion, opened_at + OPEN_DURATION).cover, 1.);
}

#[test]
fn turning_back_leaves_every_part_where_it_was() {
    let start = Instant::now();
    let motion = MotionTokens::default();
    let mut opening = Opening::closed(start);
    opening.set_open(true, false, &motion, start);

    let near = |a: Shown, b: Shown| {
        [
            (a.cover, b.cover),
            (a.under, b.under),
            (a.backdrop, b.backdrop),
            (a.details, b.details),
            (a.lines, b.lines),
        ]
        .into_iter()
        .all(|(a, b)| (a - b).abs() < 1e-3)
    };
    let mut now = start;
    for (is_open, after) in [(false, 0.3), (true, 0.1), (false, 0.2)] {
        now += OPEN_DURATION.mul_f32(after);
        let before = opening.shown(&motion, now);
        opening.set_open(is_open, false, &motion, now);
        assert!(near(before, opening.shown(&motion, now)), "{before:?}");
    }

    let closed_at = now + CLOSE_DURATION;
    assert_eq!(opening.shown(&motion, closed_at), Shown::default());
}

#[test]
fn each_part_moves_within_its_share_of_the_way() {
    assert_eq!(through(0.1, OPENING_PARTS.details), 0.);
    assert_eq!(through(0.25, OPENING_PARTS.details), 0.);
    assert_eq!(through(0.69, OPENING_PARTS.details), 1.);
    assert_eq!(through(1., OPENING_PARTS.details), 1.);
    assert!((through(0.67, OPENING_PARTS.lines) - 0.5).abs() < 1e-3);
}

#[test]
fn the_column_stays_within_its_limits_and_the_cover_within_the_column() {
    let narrow = Stage::new(size(px(500.), px(526.)), REM);
    assert_eq!(narrow.column, REM * COLUMN_MIN);
    let wide = Stage::new(size(px(3000.), px(1400.)), REM);
    assert_eq!(wide.column, REM * COLUMN_MAX);

    for stage in [narrow, wide, Stage::new(ROOM, REM)] {
        assert_eq!(stage.cover.size.width, stage.cover.size.height);
        assert!(stage.cover.left() > px(0.));
        assert!(stage.cover.right() < stage.column);
    }
    assert!(!narrow.has_large_type);
    assert!(wide.has_large_type);
}

#[test]
fn a_short_window_takes_it_out_of_the_cover() {
    let room = size(px(1200.), px(526.));
    let stage = Stage::new(room, REM);
    assert!(stage.details_top + REM * DETAILS_HEIGHT <= room.height);
    assert!(stage.cover.top() >= REM * COLUMN_MARGIN - px(1.));
}

#[test]
fn zooming_keeps_the_proportions() {
    let room = size(px(1200.), px(726.));
    let zoomed = Stage::new(room, px(20.));
    assert_eq!(zoomed.bar_cover.size.width, px(20.) * BAR_COVER);
    assert!(zoomed.details_top + px(20.) * DETAILS_HEIGHT <= room.height);
}

#[test]
fn a_part_takes_its_colour_from_how_lit_it_is() {
    let found = lyrics(2);
    let Lyrics::Timed(found) = &found else {
        panic!("expected timed lyrics");
    };
    let (dim, bright) = (
        gpui_kit::hsla(0., 0., 0.5, 1.),
        gpui_kit::hsla(0., 0., 1., 1.),
    );
    let runs_at = |seconds| {
        let lit = lit_lines(found, secs(seconds), true, LyricsHighlight::ByWord);
        line_runs(lit.line(0), dim, bright)
    };

    assert!(runs_at(9.).is_empty());
    let runs = runs_at(11.);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, 0.."line ".len());
    assert_eq!(runs[0].1.color, Some(bright));
    let runs = runs_at(12.06);
    assert_eq!(runs.len(), 2);
    assert_ne!(runs[1].1.color, Some(bright));
    assert_ne!(runs[1].1.color, Some(dim));
}

#[gpui_kit::test]
fn the_view_lights_as_it_is_told(cx: &mut TestAppContext) {
    let (stand, cx) = stand(1200., 760., cx);
    let view = view(&stand, cx);
    let player = cx.read(|cx| stand.read(cx).player.clone());
    open_with(&stand, lyrics(2), cx);
    player.update(cx, |player, _| player.place_at(secs(12.5)));
    draw(cx);
    let lit_elements = |cx: &mut VisualTestContext| cx.read(|cx| lines(&view, cx).lit.elements);
    assert_eq!(lit_elements(cx), 2);

    cx.read(|cx| lines(&view, cx).lyrics.clone())
        .set_highlight(LyricsHighlight::ByLine);
    draw(cx);
    assert_eq!(lit_elements(cx), 0);
}
