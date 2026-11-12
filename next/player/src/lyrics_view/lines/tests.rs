use std::time::Instant;

use anti_webui::core::geometry::RectF;
use config::LyricsHighlight;
use next_ui::testing::{WINDOW, click, find, frames, press, scroll, wait_until};

use super::*;
use crate::lyrics_view::TOP;
use crate::lyrics_view::tests::{Stand, stand, timed};

/// Where the line being sung rests in the window, by its middle.
fn resting_place() -> f32 {
    let height = WINDOW.1 as f32 - TOP;
    TOP + height * CURRENT_LINE_AT
}

fn line(ix: usize) -> RectF {
    find(Role::Button, &format!("line {ix}"))
        .unwrap_or_else(|| panic!("no line {ix}"))
        .bounds
}

fn middle(rect: RectF) -> (f32, f32) {
    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

fn sync() -> Option<RectF> {
    find(Role::Button, "Sync").map(|button| button.bounds)
}

/// Lets the timers that are due by then run, and the frames after them.
fn after(stand: &mut Stand, wait: Duration) {
    stand.ui.tick(Instant::now() + wait);
    stand.ui.update();
}

/// Open on lyrics of thirty lines, stopped at `seconds` in.
fn open_at(seconds: u64) -> Stand {
    let mut stand = stand(WINDOW);
    stand.finds.answer_with(Ok(timed(30)));
    stand.player.place_at(Duration::from_secs(seconds));
    stand.open();
    wait_until(&mut stand.ui, || find(Role::Button, "line 0").is_some());
    stand.settle();
    stand
}

fn assert_resting(ix: usize) {
    let (_, y) = middle(line(ix));
    assert!(
        (y - resting_place()).abs() <= 1.0,
        "line {ix} is at {y}, not {}",
        resting_place(),
    );
}

#[test]
fn the_line_being_sung_comes_to_rest_at_its_place() {
    let mut stand = open_at(61);
    assert_resting(10);

    stand.player.place_at(Duration::from_secs(71));
    after(&mut stand, REFRESH * 2);
    stand.settle();
    assert_resting(12);
}

#[test]
fn before_the_first_line_it_rests_in_the_place_of_the_one_being_sung() {
    let _stand = open_at(0);
    assert_resting(0);
}

#[test]
fn the_lines_move_on_from_where_they_are() {
    let mut stand = open_at(61);
    let before = line(12).y;

    stand.player.place_at(Duration::from_secs(71));
    after(&mut stand, REFRESH * 2);
    frames(&mut stand.ui, 48);
    let midway = line(12).y;
    assert!(midway < before && midway > before - 2.0 * line(12).height - 8.0);
}

#[test]
fn with_motion_reduced_the_lines_jump() {
    let mut stand = open_at(61);
    stand.reduces_motion.set(true);

    stand.player.place_at(Duration::from_secs(71));
    after(&mut stand, REFRESH * 2);
    frames(&mut stand.ui, 0);
    assert_resting(12);
}

#[test]
fn scrolling_stops_following_until_asked_to() {
    let mut stand = open_at(61);
    assert!(sync().is_none());

    let (x, y) = middle(line(10));
    scroll(&mut stand.ui, x, y, 200.0);
    let left = line(10).y;
    assert!(left < resting_place() - 100.0);
    stand.player.place_at(Duration::from_secs(71));
    after(&mut stand, REFRESH * 2);
    stand.settle();
    assert_eq!(line(10).y, left);

    press(&mut stand.ui, Role::Button, "Sync");
    // From where the user left them
    frames(&mut stand.ui, 0);
    assert!((line(10).y - left).abs() < 20.0);
    stand.settle();
    assert_resting(12);
    assert!(sync().is_none());
}

#[test]
fn choosing_a_line_follows_again_from_it() {
    let mut stand = open_at(61);
    stand.player.pretend_playing(false);
    let (x, y) = middle(line(10));
    scroll(&mut stand.ui, x, y, 120.0);
    assert!(sync().is_some());

    // Where the player says it went, as its engine has nothing loaded to move
    let (x, y) = middle(line(14));
    click(&mut stand.ui, x, y);
    assert!(sync().is_none());
    stand.settle();
    assert_resting(14);
}

#[test]
fn a_click_on_sync_is_not_for_the_line_behind_it() {
    let mut stand = open_at(61);
    stand.player.pretend_playing(false);
    let (x, y) = middle(line(10));
    scroll(&mut stand.ui, x, y, 120.0);
    let before = stand.player.position();

    let (x, y) = middle(sync().unwrap());
    click(&mut stand.ui, x, y);
    assert_eq!(stand.player.position(), before);
    assert!(sync().is_none());
}

#[test]
fn the_wheel_over_sync_still_scrolls_the_lines() {
    let mut stand = open_at(61);
    let (x, y) = middle(line(10));
    scroll(&mut stand.ui, x, y, 120.0);
    let before = line(10).y;

    let (x, y) = middle(sync().unwrap());
    scroll(&mut stand.ui, x, y, 100.0);
    stand.ui.update();
    assert_eq!(line(10).y, before - 100.0);
}

#[test]
fn the_lines_are_looked_up_only_while_open() {
    let mut stand = stand(WINDOW);
    stand.finds.answer_with(Ok(timed(3)));
    stand.settle();
    assert_eq!(stand.finds.asked(), 0);

    stand.open();
    wait_until(&mut stand.ui, || find(Role::Button, "line 0").is_some());
    assert_eq!(stand.finds.asked(), 1);
}

fn lit_at(lines: usize, position: Duration, highlight: LyricsHighlight) -> Option<LitLine> {
    let Lyrics::Timed(lyrics) = timed(lines) else {
        panic!("timed lyrics");
    };
    lit_lines(&lyrics, position, false, highlight)
        .lines()
        .first()
        .cloned()
}

#[test]
fn by_word_the_parts_light_as_they_are_sung() {
    let lit = lit_at(1, Duration::from_millis(11_000), LyricsHighlight::ByWord).unwrap();
    assert_eq!(lit_parts("line 0".len(), &lit), [(0..5, 1.0), (5..6, 0.0)]);

    let lit = lit_at(1, Duration::from_millis(12_500), LyricsHighlight::ByWord).unwrap();
    assert_eq!(lit_parts("line 0".len(), &lit), [(0..5, 1.0), (5..6, 1.0)]);
}

#[test]
fn by_line_all_of_it_lights_at_once() {
    let lit = lit_at(1, Duration::from_millis(10_100), LyricsHighlight::ByLine).unwrap();
    assert_eq!(lit_parts("line 0".len(), &lit), [(0..6, 1.0)]);
}
