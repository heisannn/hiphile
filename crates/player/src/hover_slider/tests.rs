use gpui_kit::{
    AppContext as _, Context, Modifiers, MouseButton, Pixels, Point, Render, TestAppContext,
    VisualTestContext, point, px,
};

use super::*;

const LINE_START: f32 = 20.;
const LINE_END: f32 = 180.;
const ROW: f32 = 12.;

struct Harness {
    state: Entity<SliderState>,
    disabled: bool,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w(px(LINE_END + LINE_START))
            .h(px(2. * ROW))
            .px(px(LINE_START))
            .child(HoverSlider::new(&self.state, "Test").disabled(self.disabled))
    }
}

fn at(x: f32) -> Point<Pixels> {
    point(px(x), px(ROW))
}

fn harness(
    disabled: bool,
    cx: &mut TestAppContext,
) -> (Entity<SliderState>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        // The thumb comes and goes at once, with no spring to wait for
        cx.set_reduce_motion(true);
    });
    let state = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
    let view_state = state.clone();
    let (_, cx) = cx.add_window_view(move |_, _| Harness {
        state: view_state,
        disabled,
    });
    cx.run_until_parked();
    (state, cx)
}

fn thumb(cx: &mut VisualTestContext) -> Option<gpui_kit::Bounds<Pixels>> {
    cx.run_until_parked();
    cx.debug_bounds("hover-slider-thumb")
}

#[gpui_kit::test]
fn shows_the_thumb_only_while_the_pointer_is_on_the_slider(cx: &mut TestAppContext) {
    let (_, cx) = harness(false, cx);
    assert!(thumb(cx).is_none());

    cx.simulate_mouse_move(at(100.), None, Modifiers::default());
    assert!(thumb(cx).is_some());

    cx.simulate_mouse_move(point(px(100.), px(100.)), None, Modifiers::default());
    assert!(thumb(cx).is_none());
}

#[gpui_kit::test]
fn keeps_the_thumb_through_a_drag_off_the_slider(cx: &mut TestAppContext) {
    let (state, cx) = harness(false, cx);
    let away = point(px(140.), px(100.));
    cx.simulate_mouse_move(at(100.), None, Modifiers::default());
    cx.simulate_mouse_down(at(100.), MouseButton::Left, Modifiers::default());
    // The first move starts the drag and the second is the first it hears of
    cx.simulate_mouse_move(away, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(away, MouseButton::Left, Modifiers::default());
    assert!(thumb(cx).is_some());
    cx.update(|_, cx| {
        assert!((state.read(cx).value().end() - 0.75).abs() < 0.01);
    });

    cx.simulate_mouse_up(away, MouseButton::Left, Modifiers::default());
    assert!(thumb(cx).is_none());
}

#[gpui_kit::test]
fn keeps_the_thumb_centred_on_the_ends_of_the_line(cx: &mut TestAppContext) {
    let (state, cx) = harness(false, cx);
    cx.simulate_mouse_move(at(100.), None, Modifiers::default());

    // Past the end of the line, where a thumb at the end sticks out
    cx.simulate_click(at(LINE_END + 4.), Modifiers::default());
    cx.update(|_, cx| assert_eq!(state.read(cx).value().end(), 1.));
    let bounds = thumb(cx).expect("the pointer is on the slider");
    assert_eq!(bounds.center().x, px(LINE_END));
    assert_eq!(bounds.center().y, px(ROW));

    cx.simulate_click(at(LINE_START - 4.), Modifiers::default());
    cx.update(|_, cx| assert_eq!(state.read(cx).value().end(), 0.));
    let bounds = thumb(cx).expect("the pointer is on the slider");
    assert_eq!(bounds.center().x, px(LINE_START));
}

#[gpui_kit::test]
fn takes_no_value_and_shows_no_thumb_when_disabled(cx: &mut TestAppContext) {
    let (state, cx) = harness(true, cx);
    cx.simulate_mouse_move(at(100.), None, Modifiers::default());
    assert!(thumb(cx).is_none());

    cx.simulate_click(at(100.), Modifiers::default());
    cx.update(|_, cx| assert_eq!(state.read(cx).value().end(), 0.));
    assert!(thumb(cx).is_none());
}
