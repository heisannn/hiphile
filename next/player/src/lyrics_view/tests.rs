use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anti_webui::core::Ui;
use anti_webui::core::access::{AccessNode, Checked, snapshot};
use anti_webui::core::event::NamedKey;
use futures::FutureExt as _;
use lyrics::{FindLyrics, Lyrics};
use next_ui::testing::{
    NOWHERE, TestCore, WINDOW, find, frames, key, nodes, press, recording_commands, screen_context,
    signed_in_core_with_lyrics, start_at, wait_until,
};
use serde_json::json;

use super::*;
use crate::player_bar;

pub(super) fn song(id: &str) -> Song {
    serde_json::from_value(json!({
        "id": id,
        "title": format!("Title of {id}"),
        "artist": "Artist",
        "album": "Album",
        "duration": 200,
    }))
    .unwrap()
}

/// Lyrics of `lines` lines, five seconds apart from ten seconds in, each lit in two parts.
pub(super) fn timed(lines: usize) -> Lyrics {
    let body: Vec<_> = (0..lines)
        .map(|ix| {
            let start = 10.0 + ix as f64 * 5.0;
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
    lyrics::lyrics_from_richsync_body(&serde_json::Value::Array(body).to_string())
}

/// What lookups were asked about, and what they find; once those run out, a lookup finds nothing
/// for as long as the test goes on.
#[derive(Clone, Default)]
pub(super) struct Finds {
    asked: Arc<Mutex<usize>>,
    answers: Arc<Mutex<VecDeque<Result<Lyrics, lyrics::Error>>>>,
}

impl Finds {
    pub(super) fn answer_with(&self, answer: Result<Lyrics, lyrics::Error>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    pub(super) fn asked(&self) -> usize {
        *self.asked.lock().unwrap()
    }

    fn source(&self) -> FindLyrics {
        let finds = self.clone();
        Arc::new(move |_| {
            *finds.asked.lock().unwrap() += 1;
            match finds.answers.lock().unwrap().pop_front() {
                Some(answer) => futures::future::ready(answer).boxed(),
                None => futures::future::pending().boxed(),
            }
        })
    }
}

/// The bar at the foot of a window and the lyrics over it, as the shell has them, for a player
/// that plays nothing, so nothing reaches a device or the server.
pub(super) struct Stand {
    _core: TestCore,
    pub(super) ui: Ui,
    pub(super) player: playback::Player,
    pub(super) panels: Panels,
    pub(super) reduces_motion: Signal<bool>,
    pub(super) finds: Finds,
    pub(super) sent: Rc<RefCell<Vec<Command>>>,
}

pub(super) fn stand(size: (i32, i32)) -> Stand {
    let finds = Finds::default();
    let (core, hiphile) = signed_in_core_with_lyrics(NOWHERE, finds.source());
    let player = hiphile.workspace().unwrap().player().clone();
    player.fill_queue(&[song("a"), song("b")], 0);
    let made = Rc::new(Cell::new(None));
    let kept = made.clone();
    let (commands, sent) = recording_commands();
    let shown = player.clone();
    let ui = start_at(
        move || {
            let panels = Panels::new();
            let reduces_motion = signal(false);
            kept.set(Some((panels, reduces_motion)));
            let cx = screen_context(&hiphile, commands);
            div()
                .size_full()
                .relative()
                .flex()
                .flex_col()
                .child(div().flex_1())
                .child(player_bar(&cx, shown, signal(None), panels))
                .child(lyrics_overlay(&cx, signal(None), panels, reduces_motion))
        },
        size,
    );
    let (panels, reduces_motion) = made.get().unwrap();
    let mut stand = Stand {
        _core: core,
        ui,
        player,
        panels,
        reduces_motion,
        finds,
        sent,
    };
    // The workspace tells the lyrics of the song from the core's threads
    wait_until(&mut stand.ui, || {
        find(Role::Button, "Show Lyrics").is_some()
    });
    stand
}

impl Stand {
    pub(super) fn open(&mut self) {
        self.panels.lyrics.set(true);
        self.ui.update();
    }

    pub(super) fn close(&mut self) {
        self.panels.lyrics.set(false);
        self.ui.update();
    }

    /// Long enough for every spring to come to rest.
    pub(super) fn settle(&mut self) {
        frames(&mut self.ui, 1500);
    }

    /// Draws the frames of `milliseconds` from `start`, taken before what is drawn began to move.
    /// A motion steps by the time since its last frame, and by none for a frame from before it
    /// began, so the first frame times it from `start` however long the test took to come to it.
    pub(super) fn frames_from(&mut self, start: Instant, milliseconds: u64) {
        for t in (0..=milliseconds).step_by(8) {
            let now = start + Duration::from_millis(t);
            self.ui.animate(now);
            self.ui.update();
            let _ = self.ui.paint(now);
        }
    }
}

/// The cover that opens the lyrics, in the bar or over it.
fn covers() -> Vec<AccessNode> {
    nodes(Role::Button)
        .into_iter()
        .filter(|node| matches!(node.label.as_deref(), Some("Show Lyrics" | "Hide Lyrics")))
        .collect()
}

fn bar_cover() -> RectF {
    covers()[0].bounds
}

/// The overlay's cover, while it shows.
fn flying_cover() -> Option<RectF> {
    covers().get(1).map(|cover| cover.bounds)
}

/// Where the overlay has a control the bar has too, as it comes after the bar.
fn in_overlay(role: Role, label: &str) -> RectF {
    nodes(role)
        .into_iter()
        .rev()
        .find(|node| node.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {role:?} named {label:?}"))
        .bounds
}

fn assert_near(a: RectF, b: RectF) {
    let near = |a: f32, b: f32| (a - b).abs() <= 1.0;
    assert!(
        near(a.x, b.x) && near(a.y, b.y) && near(a.width, b.width) && near(a.height, b.height),
        "{a:?} is not {b:?}",
    );
}

#[test]
fn the_cover_leaves_from_the_bar_and_returns_to_it() {
    let mut stand = stand(WINDOW);
    let from = bar_cover();

    stand.open();
    assert_near(flying_cover().unwrap(), from);
    frames(&mut stand.ui, 100);
    let midway = flying_cover().unwrap();
    assert!(midway.y < from.y && midway.width > from.width);
    stand.settle();
    let stage = Stage::new(WINDOW.0 as f32, WINDOW.1 as f32);
    assert_near(flying_cover().unwrap(), stage.cover);

    stand.close();
    stand.settle();
    assert!(flying_cover().is_none());
    assert_near(bar_cover(), from);
}

#[test]
fn the_bars_cover_hides_for_as_long_as_the_lyrics_show() {
    let mut stand = stand(WINDOW);
    let shown = stand.panels.lyrics_shown;
    stand.open();
    for _ in 0..40 {
        frames(&mut stand.ui, 16);
        assert_eq!(shown.get_untracked(), flying_cover().is_some());
    }
    assert!(shown.get_untracked());

    stand.close();
    let mut went = false;
    for _ in 0..200 {
        frames(&mut stand.ui, 16);
        assert_eq!(shown.get_untracked(), flying_cover().is_some());
        went |= !shown.get_untracked();
    }
    assert!(went);
}

#[test]
fn turning_back_midway_goes_on_from_where_the_cover_is() {
    let mut stand = stand(WINDOW);
    stand.open();
    frames(&mut stand.ui, 100);
    let before = flying_cover().unwrap();

    let closed = Instant::now();
    stand.close();
    stand.frames_from(closed, 16);
    let after = flying_cover().unwrap();
    assert!((after.y - before.y).abs() < 40.0, "{before:?} to {after:?}");
}

#[test]
fn with_motion_reduced_it_goes_at_once() {
    let mut stand = stand(WINDOW);
    stand.reduces_motion.set(true);
    let stage = Stage::new(WINDOW.0 as f32, WINDOW.1 as f32);

    stand.open();
    frames(&mut stand.ui, 16);
    assert_near(flying_cover().unwrap(), stage.cover);

    stand.close();
    frames(&mut stand.ui, 16);
    assert!(flying_cover().is_none());
}

#[test]
fn the_stage_keeps_the_column_within_its_limits_and_the_cover_within_the_column() {
    for (width, height) in [(900.0, 560.0), (1200.0, 760.0), (2400.0, 1400.0)] {
        let stage = Stage::new(width, height);
        assert!((COLUMN_MIN..=COLUMN_MAX).contains(&stage.column));
        assert!(stage.cover.x >= 0.0 && stage.cover.x + stage.cover.width <= stage.column);
        assert!(stage.cover.y >= TOP);
        assert!(stage.details.x >= 0.0 && stage.details.x + stage.details.width <= stage.column);
        assert!(stage.details.y + stage.details.height <= height);
    }
}

#[test]
fn a_short_window_takes_from_the_cover_down_to_its_least() {
    let stage = Stage::new(1200.0, 300.0);
    assert_eq!(stage.cover.width, COVER_MIN);
}

#[test]
fn what_is_under_the_cover_fits_in_the_smallest_window() {
    let smallest = (900, 560);
    let mut stand = stand(smallest);
    stand.player.pretend_playing(false);
    stand.open();
    stand.settle();
    let stage = Stage::new(smallest.0 as f32, smallest.1 as f32);

    let (start, end) = (stage.details.x, stage.details.x + stage.details.width);
    let volume = in_overlay(Role::Slider, "Volume");
    assert!(volume.x + volume.width <= end + 1.0, "{volume:?}");
    let shuffle = in_overlay(Role::ToggleButton, "Shuffle");
    assert!(shuffle.x >= start - 1.0, "{shuffle:?}");
    let repeat = in_overlay(Role::ToggleButton, "Repeat");
    assert!(repeat.x + repeat.width <= end + 1.0, "{repeat:?}");
    assert!(volume.y + volume.height <= smallest.1 as f32);
}

#[test]
fn the_details_have_the_song_its_favorite_and_its_controls() {
    let mut stand = stand(WINDOW);
    stand.open();
    stand.settle();

    let titles = anti_webui::core::tree::with_doc(|d| {
        d.nodes
            .iter()
            .filter(|(_, node)| {
                matches!(&node.kind, anti_webui::core::tree::NodeKind::Text(text)
                    if text.text == "Title of a")
            })
            .count()
    });
    assert_eq!(titles, 2);
    let favorites = nodes(Role::ToggleButton)
        .into_iter()
        .filter(|node| node.label.as_deref() == Some("Favorite"))
        .count();
    assert_eq!(favorites, 2);
    let shuffles = nodes(Role::ToggleButton)
        .into_iter()
        .filter(|node| node.label.as_deref() == Some("Shuffle"))
        .count();
    assert_eq!(shuffles, 2);
}

#[test]
fn the_bar_opens_the_lyrics_and_their_cover_closes_them() {
    let mut stand = stand(WINDOW);

    press(&mut stand.ui, Role::ToggleButton, "Lyrics");
    assert!(stand.panels.lyrics.get_untracked());
    stand.settle();

    let cover = covers().pop().unwrap();
    next_ui::testing::click(
        &mut stand.ui,
        cover.bounds.x + cover.bounds.width / 2.0,
        cover.bounds.y + cover.bounds.height / 2.0,
    );
    assert!(!stand.panels.lyrics.get_untracked());
}

#[test]
fn the_lyrics_toggle_under_the_cover_is_checked_while_they_show() {
    let mut stand = stand(WINDOW);
    stand.open();
    stand.settle();

    let toggles: Vec<_> = nodes(Role::ToggleButton)
        .into_iter()
        .filter(|node| node.label.as_deref() == Some("Lyrics"))
        .collect();
    assert_eq!(toggles.len(), 2);
    assert!(
        toggles
            .iter()
            .all(|toggle| toggle.checked == Some(Checked::True))
    );
}

fn focused() -> Option<AccessNode> {
    let tree = snapshot();
    tree.get(tree.focus?).cloned()
}

#[test]
fn escape_closes_and_the_focus_goes_back() {
    let mut stand = stand(WINDOW);
    next_ui::testing::focus(&mut stand.ui, Role::ToggleButton, "Lyrics");
    key(&mut stand.ui, NamedKey::Space);
    assert!(stand.panels.lyrics.get_untracked());
    let has_it = focused().unwrap();
    assert_eq!(
        (has_it.role, has_it.label.as_deref()),
        (Role::Dialog, Some("Lyrics"))
    );

    key(&mut stand.ui, NamedKey::Escape);
    assert!(!stand.panels.lyrics.get_untracked());
    let has_it = focused().unwrap();
    assert_eq!(
        (has_it.role, has_it.label.as_deref()),
        (Role::ToggleButton, Some("Lyrics"))
    );
}

#[test]
fn space_plays_or_pauses_where_no_button_has_the_focus() {
    let mut stand = stand(WINDOW);
    stand.open();
    stand.settle();

    key(&mut stand.ui, NamedKey::Space);
    assert_eq!(*stand.sent.borrow(), [Command::TogglePlayback]);

    next_ui::testing::focus(&mut stand.ui, Role::ToggleButton, "Shuffle");
    key(&mut stand.ui, NamedKey::Space);
    assert_eq!(*stand.sent.borrow(), [Command::TogglePlayback]);
}

#[test]
fn it_covers_what_is_under_it_once_its_backdrop_has_come() {
    let mut stand = stand(WINDOW);
    stand.open();
    assert!(!stand.panels.is_covered());
    stand.settle();
    assert!(stand.panels.is_covered());

    stand.close();
    assert!(!stand.panels.is_covered());
}

#[test]
fn opening_looks_for_the_song_once_and_shows_what_was_found() {
    let mut stand = stand(WINDOW);
    stand.finds.answer_with(Ok(timed(3)));
    assert_eq!(stand.finds.asked(), 0);

    stand.open();
    wait_until(&mut stand.ui, || find(Role::Button, "line 0").is_some());
    stand.close();
    stand.settle();
    stand.open();
    stand.settle();
    assert!(find(Role::Button, "line 0").is_some());
    assert_eq!(stand.finds.asked(), 1);
}

#[test]
fn a_lookup_that_failed_says_so_and_can_be_tried_again() {
    let mut stand = stand(WINDOW);
    stand.finds.answer_with(Err(lyrics::Error::InvalidResponse));
    stand.finds.answer_with(Ok(timed(2)));

    stand.open();
    wait_until(&mut stand.ui, || {
        find(Role::Heading, "Lyrics didn't load").is_some()
    });
    assert!(next_ui::testing::has_text(
        "The lyrics services aren't answering as they should. Try again later."
    ));
    press(&mut stand.ui, Role::Button, "Retry");
    wait_until(&mut stand.ui, || find(Role::Button, "line 1").is_some());
    assert_eq!(stand.finds.asked(), 2);
}

#[test]
fn what_stands_in_for_lyrics_says_why() {
    for (found, told) in [
        (Lyrics::Instrumental, "This song is instrumental."),
        (
            Lyrics::Restricted,
            "The lyrics of this song aren't available.",
        ),
        (Lyrics::NotFound, "No lyrics found for this song."),
    ] {
        let mut stand = stand(WINDOW);
        stand.finds.answer_with(Ok(found));
        stand.open();
        wait_until(&mut stand.ui, || next_ui::testing::has_text(told));
    }
}

#[test]
fn plain_lyrics_show_as_they_are_with_their_copyright() {
    let mut stand = stand(WINDOW);
    let plain = lyrics::plain_lyrics("first\nsecond", Some("Written by someone"));
    stand.finds.answer_with(Ok(plain));
    stand.open();
    wait_until(&mut stand.ui, || {
        next_ui::testing::has_text("Written by someone")
    });
    assert!(next_ui::testing::has_text("first\nsecond"));
}

#[test]
fn while_loading_it_says_so() {
    let mut stand = stand(WINDOW);
    stand.open();
    wait_until(&mut stand.ui, || {
        find(Role::ProgressBar, "Loading Lyrics").is_some()
    });
}

#[test]
fn a_song_that_stopped_says_why_under_the_cover() {
    const ERROR: &str = "The file couldn't be read. Press Play to try again.";
    let mut stand = stand(WINDOW);
    stand.open();
    stand.settle();

    stand.player.set_error(ERROR);
    let told = || {
        anti_webui::core::tree::with_doc(|d| {
            d.nodes
                .iter()
                .filter(|(_, node)| {
                    matches!(&node.kind, anti_webui::core::tree::NodeKind::Text(text)
                        if text.text == ERROR)
                })
                .count()
        })
    };
    // In the bar and under the cover
    wait_until(&mut stand.ui, || told() == 2);
}
