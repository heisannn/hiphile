use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::{
    Modifiers, MouseDownEvent, MouseUpEvent, PlatformInput, Point, TestAppContext,
    VisualTestContext, point, px,
};
use hiphile_core::test_support::{self, FavoritesServer};
use subsonic::{SongId, StarTarget};

use super::*;

const IN_CONTROLS: Point<Pixels> = point(px(4.), px(4.));
const ON_ITS_OWN: Point<Pixels> = point(px(4.), px(44.));

struct Bar {
    clicks: Rc<Cell<u32>>,
}

impl Render for Bar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        let on_click = move |_: &gpui_kit::ClickEvent, _: &mut Window, _: &mut App| {
            clicks.set(clicks.get() + 1);
        };
        v_flex()
            .size_full()
            .child(
                div().h_10().child(
                    title_bar_controls().child(
                        Button::new("in-controls")
                            .label("In")
                            .on_click(on_click.clone()),
                    ),
                ),
            )
            .child(Button::new("on-its-own").label("Own").on_click(on_click))
    }
}

fn bar(cx: &mut TestAppContext) -> (Rc<Cell<u32>>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let clicks = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view({
        let clicks = clicks.clone();
        move |_, _| Bar { clicks }
    });
    cx.run_until_parked();
    (clicks, cx)
}

/// Presses at `position`, and says whether the press went on past the window's
/// contents, which on Windows hands it to the system.
fn press(position: Point<Pixels>, cx: &mut VisualTestContext) -> bool {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.update(|window, cx| {
        window
            .dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            )
            .propagate
    })
}

#[gpui_kit::test]
fn a_press_on_a_title_bar_control_goes_no_further_and_still_clicks(cx: &mut TestAppContext) {
    let (clicks, cx) = bar(cx);

    assert!(!press(IN_CONTROLS, cx));
    cx.update(|window, cx| {
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: IN_CONTROLS,
                modifiers: Modifiers::default(),
                click_count: 1,
            }),
            cx,
        );
    });

    assert_eq!(clicks.get(), 1);
}

#[gpui_kit::test]
fn a_press_on_a_button_elsewhere_goes_on(cx: &mut TestAppContext) {
    let (_, cx) = bar(cx);

    assert!(press(ON_ITS_OWN, cx));
}

struct Follower {
    _favorites: Task<()>,
}

impl Render for Follower {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui_kit::test]
fn a_new_session_forgets_the_favorites_of_whoever_came_before(cx: &mut TestAppContext) {
    let server = FavoritesServer::new(&["a"]);
    server.hold("getStarred2");
    // The requests wake the test from the network's own thread
    cx.executor().allow_parking();
    let before = StarTarget::Song(SongId::new("from-before"));
    cx.update(|cx| actions::set_known_favorites(Some(HashSet::from([before.clone()])), cx));
    let session = test_support::session(server.url());
    let favorites = FavoritesState::new(
        cx.update(|cx| gpui_host::executor(cx)),
        session.client().clone(),
    );

    let _follower = cx.new(|cx| Follower {
        _favorites: follow_favorites(&favorites, cx),
    });

    cx.update(|cx| assert_eq!(actions::is_favorite(&before, cx), None));
    server.release("getStarred2");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !favorites.snapshot().is_loaded() {
        assert!(Instant::now() < deadline, "the favorites never loaded");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(actions::is_favorite(&before, cx), Some(false));
        let a = StarTarget::Song(SongId::new("a"));
        assert_eq!(actions::is_favorite(&a, cx), Some(true));
    });
}
