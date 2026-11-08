use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::{TestAppContext, VisualTestContext};
use serde_json::json;

use super::*;
use crate::testing;

struct Details(Song);

impl gpui_kit::Render for Details {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(song_details(&self.0, cx))
    }
}

/// A player that plays nothing, so nothing reaches a device or the server.
fn idle_player(cx: &mut TestAppContext) -> (Entity<Player>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let (player, cx) = cx.add_window_view(testing::idle_player);
    cx.run_until_parked();
    (player, cx)
}

fn position_height(cx: &mut VisualTestContext) -> gpui_kit::Pixels {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.debug_bounds("player-position").unwrap().size.height
}

/// How far the middle of the controls is above the middle of the bar's
/// content, which starts under the bar's one-pixel top border.
fn controls_lift(cx: &mut VisualTestContext) -> gpui_kit::Pixels {
    let bar = cx.debug_bounds("player-bar").unwrap();
    let controls = cx.debug_bounds("player-controls").unwrap();
    bar.center().y + gpui_kit::px(0.5) - controls.center().y
}

fn core(player: &Entity<Player>, cx: &mut VisualTestContext) -> playback::Player {
    player.read_with(cx, |player, _| player.core().clone())
}

#[gpui_kit::test]
fn the_bars_star_follows_the_song_and_the_favorites(cx: &mut TestAppContext) {
    let (player, cx) = idle_player(cx);
    let song = named_song("a");
    let asked = Rc::new(RefCell::new(Vec::new()));
    let notified = Rc::new(Cell::new(0));
    core(&player, cx).fill_queue(std::slice::from_ref(&song), 0);
    cx.update(|window, cx| {
        actions::set_known_favorites(Some(HashSet::new()), cx);
        let asked = asked.clone();
        cx.on_action(move |action: &actions::SetFavorite, _| {
            asked.borrow_mut().push(action.clone());
        });
        let notified = notified.clone();
        cx.observe(&player, move |_, _| notified.set(notified.get() + 1))
            .detach();
        window.refresh();
    });
    cx.run_until_parked();

    let star = cx.debug_bounds("player-favorite").unwrap();
    let cover = cx.debug_bounds("player-cover").unwrap();
    assert!(star.left() > cover.right());
    cx.simulate_click(star.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let favorite = Favorite::Song(song);
    assert_eq!(
        asked.take(),
        [actions::SetFavorite::new(favorite.clone(), true)]
    );

    let before = notified.get();
    cx.update(|_, cx| {
        actions::set_known_favorites(Some(HashSet::from([favorite.target()])), cx);
    });
    cx.run_until_parked();
    assert!(notified.get() > before);
}

fn named_song(id: &str) -> Song {
    serde_json::from_value(json!({ "id": id, "title": id, "duration": 100 })).unwrap()
}

fn current_title(player: &Entity<Player>, cx: &mut VisualTestContext) -> Option<String> {
    core(player, cx)
        .snapshot()
        .current_song()
        .map(|song| song.title().to_owned())
}

#[gpui_kit::test]
fn seeks_move_the_position(cx: &mut TestAppContext) {
    let (player, cx) = idle_player(cx);
    let core = core(&player, cx);
    core.fill_queue(&[named_song("a"), named_song("b")], 0);
    core.pretend_playing(true);
    let shown = |cx: &mut VisualTestContext| {
        player.read_with(cx, |player, cx| {
            player.position_state().read(cx).value().end()
        })
    };
    let seek = |cx: &mut VisualTestContext,
                seek: fn(&mut Player, &mut Window, &mut Context<Player>)| {
        cx.update(|window, cx| player.update(cx, |player, cx| seek(player, window, cx)));
    };

    seek(cx, |player, window, cx| {
        player.seek_to(Duration::from_secs(50), window, cx);
    });
    assert_eq!(shown(cx), 0.5);
    let in_mini_player = player.read_with(cx, |player, cx| {
        player.mini_position_state().read(cx).value().end()
    });
    assert_eq!(in_mini_player, 0.5);
    seek(cx, Player::seek_forward);
    assert_eq!(shown(cx), 0.1);
    seek(cx, Player::seek_backward);
    assert_eq!(shown(cx), 0.0);

    seek(cx, |player, window, cx| {
        player.seek_to(Duration::from_secs(100), window, cx);
    });
    assert_eq!(current_title(&player, cx).as_deref(), Some("b"));
}

#[gpui_kit::test]
fn a_song_waiting_for_its_data_shows_a_spinner(cx: &mut TestAppContext) {
    let (player, cx) = idle_player(cx);
    let core = core(&player, cx);
    core.fill_queue(&[named_song("a")], 0);
    core.pretend_playing(true);
    core.pretend_buffering(true);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.debug_bounds("player-buffering").is_some());

    core.toggle_playback();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.debug_bounds("player-buffering").is_none());
}

#[gpui_kit::test]
fn the_controls_ease_up_from_the_middle_as_a_song_arrives(cx: &mut TestAppContext) {
    let (player, cx) = idle_player(cx);
    assert_eq!(position_height(cx), gpui_kit::px(0.));
    assert_eq!(controls_lift(cx), gpui_kit::px(0.));

    let song: Song = serde_json::from_value(json!({ "id": "a", "title": "a" })).unwrap();
    core(&player, cx).fill_queue(&[song], 0);
    assert_eq!(position_height(cx), gpui_kit::px(0.));

    let (duration, rem) =
        cx.update(|window, cx| (cx.theme().motion_tokens().duration_slow, window.rem_size()));
    cx.executor().advance_clock(duration / 4);
    let partway = position_height(cx);
    assert!(gpui_kit::px(0.) < partway && partway < rem);
    // Layout rounds to whole pixels, so halfway through it can be half off
    assert!((controls_lift(cx) - partway / 2.).abs() <= gpui_kit::px(0.5));

    cx.executor().advance_clock(duration);
    assert_eq!(position_height(cx), rem);
    assert_eq!(controls_lift(cx), rem / 2.);
}

#[gpui_kit::test]
fn clicking_the_album_under_the_song_shows_it(cx: &mut TestAppContext) {
    let shown = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|cx| {
        gpui_kit::init(cx);
        let shown = shown.clone();
        cx.on_action(move |action: &actions::ShowAlbum, _| {
            shown.borrow_mut().push(action.album().clone());
        });
    });
    let song = serde_json::from_value(json!({
        "id": "s", "title": "So What", "album": "Kind of Blue", "albumId": "al1",
    }))
    .unwrap();
    let (_, cx) = cx.add_window_view(|_, _| Details(song));
    let cx: &mut VisualTestContext = cx;
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(400.), gpui_kit::px(100.)));
    cx.run_until_parked();

    let position = gpui_kit::point(gpui_kit::px(10.), gpui_kit::px(6.));
    cx.simulate_click(position, Default::default());
    cx.run_until_parked();

    assert_eq!(shown.take(), [subsonic::AlbumId::new("al1")]);
}

#[gpui_kit::test]
fn the_volume_slider_follows_the_volume(cx: &mut TestAppContext) {
    let (player, cx) = idle_player(cx);
    core(&player, cx).volume_down();
    cx.run_until_parked();
    let shown = player.read_with(cx, |player, cx| {
        player.volume_state().read(cx).value().end()
    });
    assert_eq!(shown, 0.9);
}
