use std::cell::Cell;
use std::rc::Rc;

use audio::OutputSettings;
use gpui_kit::component::v_flex;
use gpui_kit::{
    AppContext as _, Bounds, Context, FocusHandle, Modifiers, Pixels, Render, TestAppContext,
    VisualTestContext, point, px, size,
};
use serde_json::json;
use subsonic::{Credentials, Song};

use super::*;
use crate::Panels;

/// A player at the bottom of a window, whose panels only stand as told.
struct Stand {
    player: Entity<Player>,
    focus: FocusHandle,
    panels: Panels,
    lyrics_toggled: Rc<Cell<u32>>,
    queue_toggled: Rc<Cell<u32>>,
}

impl Render for Stand {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lyrics_toggled = self.lyrics_toggled.clone();
        let queue_toggled = self.queue_toggled.clone();
        v_flex()
            .id("stand")
            .track_focus(&self.focus)
            .size_full()
            .on_action(move |_: &ToggleLyrics, _, _| lyrics_toggled.set(lyrics_toggled.get() + 1))
            .on_action(move |_: &ToggleQueue, _, _| queue_toggled.set(queue_toggled.get() + 1))
            .child(div().flex_1().min_h_0())
            .child(self.player.clone())
            .text_color(cx.theme().foreground)
    }
}

fn song() -> Song {
    serde_json::from_value(json!({ "id": "a", "title": "A" })).unwrap()
}

fn stand(with_song: bool, cx: &mut TestAppContext) -> (Entity<Stand>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let net = net::Client::new("hiphile-test").unwrap();
    let server = client::Server::new("http://127.0.0.1:9", Credentials::api_key("k")).unwrap();
    let info = serde_json::from_value(json!({ "version": "1.16.1" })).unwrap();
    let session = session::Session::new(
        client::Client::new(net, server),
        "alice".to_owned(),
        info,
        None,
        None,
    );
    let (stand, cx) = cx.add_window_view(|window, cx: &mut Context<Stand>| {
        let stand = cx.weak_entity();
        let player = cx.new(|cx| {
            let mut player = Player::new(session, OutputSettings::default(), window, cx);
            if with_song {
                player.fill_queue(&[song()], 0);
            }
            player.set_panels(move |_, cx| {
                stand
                    .upgrade()
                    .map_or_else(Panels::default, |stand| stand.read(cx).panels)
            });
            player
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Stand {
            player,
            focus,
            panels: Panels::default(),
            lyrics_toggled: Rc::default(),
            queue_toggled: Rc::default(),
        }
    });
    cx.simulate_resize(size(px(1200.), px(400.)));
    cx.run_until_parked();
    (stand, cx)
}

fn toggles(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.debug_bounds("panel-toggles").expect("the toggles")
}

/// Clicks the first or the second button in `bounds`.
fn click_toggle(second: bool, bounds: Bounds<Pixels>, cx: &mut VisualTestContext) {
    let quarter = bounds.size.width / 4.;
    let x = if second {
        bounds.right() - quarter
    } else {
        bounds.left() + quarter
    };
    cx.simulate_click(point(x, bounds.center().y), Modifiers::default());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_toggles_stand_just_left_of_the_volume(cx: &mut TestAppContext) {
    let (_, cx) = stand(true, cx);

    let toggles = toggles(cx);
    let volume = cx.debug_bounds("volume-control").expect("the volume");
    let bar = cx.debug_bounds("player-bar").expect("the bar");

    assert!(toggles.right() <= volume.left());
    assert!(volume.left() - toggles.right() < px(24.));
    assert!(toggles.top() >= bar.top() && toggles.bottom() <= bar.bottom());
}

#[gpui_kit::test]
fn the_lyrics_toggle_asks_for_the_lyrics_when_there_is_a_song(cx: &mut TestAppContext) {
    let (stand_with_song, cx) = stand(true, cx);

    click_toggle(false, toggles(cx), cx);

    let toggled = cx.read(|cx| stand_with_song.read(cx).lyrics_toggled.get());
    assert_eq!(toggled, 1);
}

#[gpui_kit::test]
fn the_lyrics_toggle_does_nothing_without_a_song(cx: &mut TestAppContext) {
    let (stand, cx) = stand(false, cx);

    click_toggle(false, toggles(cx), cx);

    assert_eq!(cx.read(|cx| stand.read(cx).lyrics_toggled.get()), 0);
}

#[gpui_kit::test]
fn the_queue_toggle_follows_whether_the_queue_fits(cx: &mut TestAppContext) {
    let (stand, cx) = stand(true, cx);

    click_toggle(true, toggles(cx), cx);
    assert_eq!(cx.read(|cx| stand.read(cx).queue_toggled.get()), 0);

    // Told only to what lays the panels out, as when the window has been widened.
    stand.update(cx, |stand, cx| {
        stand.panels.queue_fits = true;
        cx.notify();
    });
    cx.run_until_parked();

    let bounds = cx.debug_bounds("panel-toggles").expect("the toggles");
    click_toggle(true, bounds, cx);
    assert_eq!(cx.read(|cx| stand.read(cx).queue_toggled.get()), 1);
}
