use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext};
use lyrics::Lyrics;
use serde_json::json;

use super::*;
use lyrics::FindLyrics;

fn song(id: &str) -> Song {
    serde_json::from_value(json!({
        "id": id,
        "title": format!("Title of {id}"),
        "artist": "Artist",
        "album": "Album",
        "duration": 100,
    }))
    .unwrap()
}

struct Main {
    mini_player: Option<Entity<MiniPlayer>>,
    _songs: Task<()>,
}

impl Render for Main {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

/// What lookups find; once these run out, a lookup finds nothing for as long as
/// the test goes on.
type Answers = Arc<Mutex<VecDeque<Result<Lyrics, lyrics::Error>>>>;

fn find_from(answers: Answers) -> FindLyrics {
    Arc::new(move |_| match answers.lock().unwrap().pop_front() {
        Some(answer) => futures::future::ready(answer).boxed(),
        None => futures::future::pending().boxed(),
    })
}

fn main_window_answering(
    answers: Answers,
    cx: &mut TestAppContext,
) -> (Entity<Main>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ui::list_navigation::bind_control_keys(cx);
        crate::bind_commands("Workspace", cx);
    });
    cx.add_window_view(|window, cx| {
        let player = cx.new(|cx| {
            let mut player = crate::testing::idle_player(window, cx);
            player.fill_queue(&[song("a")], 0);
            player
        });
        let lyrics = CurrentLyrics::with_find_lyrics(gpui_host::executor(cx), find_from(answers));
        let songs = crate::testing::follow_songs(player.read(cx).core(), &lyrics, cx);
        let mini_player = cx.new(|cx| MiniPlayer::new(player, lyrics, window, cx));
        Main {
            mini_player: Some(mini_player),
            _songs: songs,
        }
    })
}

fn main_window(cx: &mut TestAppContext) -> (Entity<Main>, &mut VisualTestContext) {
    main_window_answering(Answers::default(), cx)
}

fn mini_player(main: &Entity<Main>, cx: &mut VisualTestContext) -> Entity<MiniPlayer> {
    cx.read(|cx| main.read(cx).mini_player.clone().unwrap())
}

fn player(mini: &Entity<MiniPlayer>, cx: &mut VisualTestContext) -> Entity<Player> {
    cx.read(|cx| mini.read(cx).player.clone())
}

fn toggle(mini: &Entity<MiniPlayer>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| mini.update(cx, |mini, cx| mini.toggle(window, cx)));
    cx.run_until_parked();
}

fn window_count(cx: &mut VisualTestContext) -> usize {
    cx.read(|cx| cx.windows().len())
}

fn is_open(mini: &Entity<MiniPlayer>, cx: &mut VisualTestContext) -> bool {
    cx.read(|cx| mini.read(cx).is_open())
}

/// Opens the mini player and gives a context for its window, drawn.
fn open(mini: &Entity<MiniPlayer>, cx: &mut VisualTestContext) -> VisualTestContext {
    toggle(mini, cx);
    let window = cx.read(|cx| mini.read(cx).window).unwrap();
    let mut mini_cx = VisualTestContext::from_window(window, cx);
    draw(&mut mini_cx);
    mini_cx
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn is_playing(player: &Entity<Player>, cx: &mut VisualTestContext) -> bool {
    cx.read(|cx| player.read(cx).snapshot().is_playing())
}

fn secondary(cx: &mut VisualTestContext) -> &'static str {
    if cx.read(|cx| cx.platform_info().is_mac()) {
        "cmd"
    } else {
        "ctrl"
    }
}

/// Starts dragging the slider of the mini player from its middle.
fn press_position(player: &Entity<Player>, cx: &mut VisualTestContext) {
    player.update(cx, |player, cx| player.pretend_playing(true, cx));
    draw(cx);
    let middle = cx
        .debug_bounds("mini-player-position")
        .expect("the position")
        .center();
    cx.simulate_mouse_move(middle, None, Modifiers::default());
    cx.simulate_mouse_down(middle, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn toggling_opens_and_closes_its_window(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    assert_eq!(window_count(cx), 1);

    toggle(&mini, cx);
    assert!(is_open(&mini, cx));
    assert_eq!(window_count(cx), 2);

    toggle(&mini, cx);
    assert!(!is_open(&mini, cx));
    assert_eq!(window_count(cx), 1);
}

#[gpui_kit::test]
fn it_knows_when_its_window_closed_on_its_own(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    toggle(&mini, cx);
    let window = cx.read(|cx| mini.read(cx).window).unwrap();

    cx.update(|_, cx| {
        window
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    cx.run_until_parked();

    assert!(!is_open(&mini, cx));
    assert_eq!(window_count(cx), 1);
}

#[gpui_kit::test]
fn it_closes_with_the_window_it_was_made_for(cx: &mut TestAppContext) {
    let (main, visual) = main_window(cx);
    let mini = mini_player(&main, visual);
    // Dropping it would have the audio thread wake a task, which tests do not allow
    std::mem::forget(player(&mini, visual));
    toggle(&mini, visual);
    assert_eq!(window_count(visual), 2);

    visual.update(|window, _| window.remove_window());
    cx.run_until_parked();

    assert_eq!(cx.read(|cx| cx.windows().len()), 0);
}

#[gpui_kit::test]
fn letting_it_go_closes_it_and_all_that_holds_on_to_the_player(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    toggle(&mini, cx);
    let player = player(&mini, cx).downgrade();
    drop(mini);

    main.update(cx, |main, _| main.mini_player = None);
    cx.run_until_parked();

    assert_eq!(window_count(cx), 1);
    assert!(player.upgrade().is_none(), "playback would go on");
}

#[gpui_kit::test]
fn it_closes_when_the_song_runs_out(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let player = player(&mini, cx);
    let core = cx.read(|cx| player.read(cx).core().clone());
    toggle(&mini, cx);

    core.fill_queue(&[song("a"), song("b")], 0);
    core.next();
    cx.run_until_parked();
    assert!(is_open(&mini, cx), "another song is no cause to close");

    core.next();
    cx.run_until_parked();
    assert!(!is_open(&mini, cx));
    assert_eq!(window_count(cx), 1);
}

#[gpui_kit::test]
fn the_menu_closes_it_when_no_window_counts_as_active(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let mut mini_cx = open(&mini, cx);
    mini_cx.deactivate_window();
    cx.deactivate_window();

    cx.update(|_, cx| {
        assert_eq!(cx.active_window(), None);
        assert!(cx.is_action_available(&ToggleMiniPlayer));
        cx.dispatch_action(&ToggleMiniPlayer);
    });
    cx.run_until_parked();

    assert!(!is_open(&mini, cx));
    assert_eq!(window_count(cx), 1);
}

#[gpui_kit::test]
fn it_returns_to_where_it_was_and_at_the_size_it_was(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let mut mini_cx = open(&mini, cx);
    let first_origin = mini_cx.update(|window, _| window.bounds().origin);
    let resized = size(px(280.), px(400.));
    mini_cx.simulate_resize(resized);
    mini_cx.run_until_parked();
    toggle(&mini, cx);

    // Opening where it first did would now be somewhere else
    cx.simulate_resize(size(px(1500.), px(900.)));
    cx.run_until_parked();
    let mut mini_cx = open(&mini, cx);

    let bounds = mini_cx.update(|window, _| window.bounds());
    assert_eq!(bounds.size, resized);
    assert_eq!(bounds.origin, first_origin);
}

#[gpui_kit::test]
fn it_shows_the_song_and_the_controls(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let mut mini_cx = open(&mini, cx);

    let song = mini_cx.debug_bounds("mini-player-song").expect("the song");
    let controls = mini_cx
        .debug_bounds("mini-player-controls")
        .expect("the controls");
    assert!(song.bottom() < controls.top());
}

#[gpui_kit::test]
fn the_buttons_stay_together_in_the_middle(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let mut mini_cx = open(&mini, cx);

    let area = mini_cx
        .debug_bounds("mini-player-controls")
        .expect("the controls");
    let buttons = mini_cx
        .debug_bounds("player-controls")
        .expect("the buttons");
    assert!(buttons.size.width < area.size.width * 0.75);
    assert!((buttons.center().x - area.center().x).abs() <= px(1.));
}

#[gpui_kit::test]
fn its_shortcuts_work_in_its_window(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let player = player(&mini, cx);
    player.update(cx, |player, cx| player.pretend_playing(true, cx));
    let secondary = secondary(cx);
    let mut mini_cx = open(&mini, cx);

    mini_cx.simulate_keystrokes("space");
    mini_cx.run_until_parked();
    assert!(!is_playing(&player, cx));

    mini_cx.simulate_keystrokes(&format!("{secondary}-alt-s"));
    mini_cx.run_until_parked();
    assert!(cx.read(|cx| player.read(cx).snapshot().is_shuffled()));
}

#[gpui_kit::test]
fn the_close_key_closes_it(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let secondary = secondary(cx);
    let mut mini_cx = open(&mini, cx);

    mini_cx.simulate_keystrokes(&format!("{secondary}-w"));
    cx.run_until_parked();

    assert!(!is_open(&mini, cx));
}

#[gpui_kit::test]
fn the_toggle_key_closes_it_from_its_window(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let secondary = secondary(cx);
    let mut mini_cx = open(&mini, cx);

    mini_cx.simulate_keystrokes(&format!("{secondary}-alt-m"));
    cx.run_until_parked();

    assert!(!is_open(&mini, cx));
}

#[gpui_kit::test]
fn space_is_left_to_a_focused_button(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let player = player(&mini, cx);
    player.update(cx, |player, cx| player.pretend_playing(true, cx));
    let mut mini_cx = open(&mini, cx);

    mini_cx.update(|window, cx| window.focus_prev(cx));

    mini_cx.simulate_keystrokes("space");
    mini_cx.run_until_parked();
    assert!(is_playing(&player, cx));
}

#[gpui_kit::test]
fn focus_comes_back_when_the_focused_button_goes(cx: &mut TestAppContext) {
    let answers = Answers::default();
    answers
        .lock()
        .unwrap()
        .push_back(Err(lyrics::Error::InvalidResponse));
    let (main, cx) = main_window_answering(answers, cx);
    let mini = mini_player(&main, cx);
    let player = player(&mini, cx);
    player.update(cx, |player, cx| player.pretend_playing(true, cx));
    let mut mini_cx = open(&mini, cx);

    let view_focus = mini_cx.update(|window, cx| window.focused(cx));
    assert!(mini_cx.debug_bounds("lyrics-retry").is_some());
    mini_cx.update(|window, cx| window.focus_next(cx));
    assert_ne!(mini_cx.update(|window, cx| window.focused(cx)), view_focus);

    cx.read(|cx| mini.read(cx).lyrics.clone()).retry();
    draw(&mut mini_cx);
    assert!(mini_cx.debug_bounds("lyrics-retry").is_none());

    assert_eq!(mini_cx.update(|window, cx| window.focused(cx)), view_focus);
    mini_cx.simulate_keystrokes("space");
    mini_cx.run_until_parked();
    assert!(!is_playing(&player, cx));
}

#[gpui_kit::test]
fn dragging_its_slider_seeks(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let player = player(&mini, cx);
    let mut mini_cx = open(&mini, cx);

    press_position(&player, &mut mini_cx);
    let at = mini_cx
        .debug_bounds("mini-player-position")
        .expect("the position")
        .center();
    assert!(cx.read(|cx| player.read(cx).is_scrubbing()));

    mini_cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    mini_cx.run_until_parked();

    cx.read(|cx| {
        let player = player.read(cx);
        assert!(!player.is_scrubbing());
        let in_bar = player.position_state().read(cx).value().end();
        assert!((in_bar - 0.5).abs() < 0.1, "{in_bar}");
    });
}

#[gpui_kit::test]
fn closing_it_from_outside_ends_a_drag(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let player = player(&mini, cx);
    let mut mini_cx = open(&mini, cx);
    press_position(&player, &mut mini_cx);
    assert!(cx.read(|cx| player.read(cx).is_scrubbing()));

    toggle(&mini, cx);

    cx.read(|cx| {
        let player = player.read(cx);
        assert!(!player.is_scrubbing());
        let in_bar = player.position_state().read(cx).value().end();
        assert!(
            (in_bar - 0.5).abs() < 0.1,
            "seeks to where it got to: {in_bar}"
        );
    });
}

#[gpui_kit::test]
fn the_close_key_seeks_to_where_a_drag_got_to(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let player = player(&mini, cx);
    let secondary = secondary(cx);
    let mut mini_cx = open(&mini, cx);
    press_position(&player, &mut mini_cx);

    mini_cx.simulate_keystrokes(&format!("{secondary}-w"));
    cx.run_until_parked();

    cx.read(|cx| {
        let player = player.read(cx);
        assert!(!player.is_scrubbing());
        let in_bar = player.position_state().read(cx).value().end();
        assert!((in_bar - 0.5).abs() < 0.1, "{in_bar}");
    });
}

#[gpui_kit::test]
fn closing_its_window_ends_a_drag(cx: &mut TestAppContext) {
    let (main, cx) = main_window(cx);
    let mini = mini_player(&main, cx);
    let player = player(&mini, cx);
    let mut mini_cx = open(&mini, cx);
    press_position(&player, &mut mini_cx);
    assert!(cx.read(|cx| player.read(cx).is_scrubbing()));

    mini_cx.update(|window, _| window.remove_window());
    cx.run_until_parked();

    assert!(!cx.read(|cx| player.read(cx).is_scrubbing()));
}

#[test]
fn the_details_hold_what_the_song_says() {
    assert_eq!(song_details(&song("a")), "Artist — Album");
    let bare: Song = serde_json::from_value(json!({ "id": "a", "title": "A" })).unwrap();
    assert_eq!(song_details(&bare), "");
}
