use gpui_kit::{
    App, Context, InteractiveElement, KeyBinding, NoAction, Pixels, UniformListScrollHandle,
    Window, actions, px,
};

/// Key context around a button inside a list, whose own use of Enter and Space
/// comes before the list's.
pub const CONTROL_CONTEXT: &str = "ListControl";

actions!(
    list,
    [
        SelectNext,
        SelectPrevious,
        SelectFirst,
        SelectLast,
        SelectPageDown,
        SelectPageUp
    ]
);

/// Leaves Enter and Space to what is inside [`CONTROL_CONTEXT`]. Call once at
/// startup, before building the menu bar.
pub fn bind_control_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", NoAction, Some(CONTROL_CONTEXT)),
        KeyBinding::new("space", NoAction, Some(CONTROL_CONTEXT)),
    ]);
}

pub fn bind_list_navigation(context: &'static str, cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some(context)),
        KeyBinding::new("up", SelectPrevious, Some(context)),
        KeyBinding::new("home", SelectFirst, Some(context)),
        KeyBinding::new("end", SelectLast, Some(context)),
        KeyBinding::new("pagedown", SelectPageDown, Some(context)),
        KeyBinding::new("pageup", SelectPageUp, Some(context)),
    ]);
}

pub trait ListNavigationExt: InteractiveElement + Sized {
    fn on_list_navigation<V: 'static>(
        self,
        on_motion: impl Fn(&mut V, Motion, &mut Window, &mut Context<V>) + Copy + 'static,
        cx: &mut Context<V>,
    ) -> Self {
        self.on_action(cx.listener(move |this, _: &SelectNext, window, cx| {
            on_motion(this, Motion::By(1), window, cx);
        }))
        .on_action(cx.listener(move |this, _: &SelectPrevious, window, cx| {
            on_motion(this, Motion::By(-1), window, cx);
        }))
        .on_action(cx.listener(move |this, _: &SelectFirst, window, cx| {
            on_motion(this, Motion::First, window, cx);
        }))
        .on_action(cx.listener(move |this, _: &SelectLast, window, cx| {
            on_motion(this, Motion::Last, window, cx);
        }))
        .on_action(cx.listener(move |this, _: &SelectPageDown, window, cx| {
            on_motion(this, Motion::Pages(1), window, cx);
        }))
        .on_action(cx.listener(move |this, _: &SelectPageUp, window, cx| {
            on_motion(this, Motion::Pages(-1), window, cx);
        }))
    }
}

impl<E: InteractiveElement + Sized> ListNavigationExt for E {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    By(isize),
    Pages(isize),
    First,
    Last,
}

impl Motion {
    /// The position to select in a list of `len` rows with `page` rows in
    /// view, starting from `current`. With nothing selected, a move starts
    /// at the first row, or the last for [`Motion::Last`].
    pub fn apply(self, current: Option<usize>, len: usize, page: usize) -> Option<usize> {
        let last = len.checked_sub(1)?;
        let Some(current) = current else {
            return Some(if matches!(self, Self::Last) { last } else { 0 });
        };
        Some(match self {
            Self::By(delta) => current.saturating_add_signed(delta).min(last),
            Self::Pages(pages) => current
                .saturating_add_signed(pages.saturating_mul(page as isize))
                .min(last),
            Self::First => 0,
            Self::Last => last,
        })
    }
}

/// Rows that fit in the viewport of a list of `len` rows, at least one, from
/// its last layout.
///
/// Reads the handle's public state, which the list writes while it lays
/// out; should that be under way, a page is one row rather than a panic.
pub fn page_rows(scroll: &UniformListScrollHandle, len: usize) -> usize {
    let Ok(state) = scroll.0.try_borrow() else {
        return 1;
    };
    let viewport = state.base_handle.bounds().size.height;
    let contents = state.last_item_size.map(|size| size.contents.height);
    contents.map_or(1, |contents| rows_in_view(viewport, contents, len))
}

/// `contents` is the height of all `len` rows, which are of one height.
fn rows_in_view(viewport: Pixels, contents: Pixels, len: usize) -> usize {
    if len == 0 || contents <= px(0.) {
        return 1;
    }
    ((viewport / (contents / len as f32)).floor() as usize).max(1)
}

#[cfg(test)]
mod tests {
    use gpui_kit::{
        Entity, FocusHandle, IntoElement, ParentElement as _, Render, TestAppContext,
        VisualTestContext, div,
    };

    use super::*;

    actions!(list_test, [Open]);

    const LIST: &str = "TestList";

    struct TestList {
        focus: FocusHandle,
        control_focus: FocusHandle,
        motions: Vec<Motion>,
        opened: usize,
    }

    impl Render for TestList {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .key_context(LIST)
                .track_focus(&self.focus)
                .on_list_navigation(
                    |this: &mut Self, motion, _, _| this.motions.push(motion),
                    cx,
                )
                .on_action(cx.listener(|this, _: &Open, _, _| this.opened += 1))
                .child(
                    div()
                        .key_context(CONTROL_CONTEXT)
                        .track_focus(&self.control_focus),
                )
        }
    }

    fn list(cx: &mut TestAppContext) -> (Entity<TestList>, &mut VisualTestContext) {
        cx.update(|cx| {
            bind_control_keys(cx);
            bind_list_navigation(LIST, cx);
            cx.bind_keys([
                KeyBinding::new("enter", Open, Some(LIST)),
                KeyBinding::new("space", Open, Some(LIST)),
            ]);
        });
        cx.add_window_view(|_, cx| TestList {
            focus: cx.focus_handle(),
            control_focus: cx.focus_handle(),
            motions: Vec::new(),
            opened: 0,
        })
    }

    fn focus(
        handle: impl Fn(&TestList) -> FocusHandle,
        list: &Entity<TestList>,
        cx: &mut VisualTestContext,
    ) {
        cx.update(|window, cx| handle(list.read(cx)).focus(window, cx));
        cx.run_until_parked();
    }

    #[test]
    fn a_page_is_the_whole_rows_in_view() {
        // Rows of 48 in a viewport of 480, and of 500 with one partly in view
        assert_eq!(rows_in_view(px(480.), px(4800.), 100), 10);
        assert_eq!(rows_in_view(px(500.), px(4800.), 100), 10);
        // A list shorter than its viewport still pages by what would fit
        assert_eq!(rows_in_view(px(480.), px(144.), 3), 10);
    }

    #[test]
    fn a_page_is_a_row_with_nothing_to_measure() {
        assert_eq!(rows_in_view(px(480.), px(0.), 0), 1);
        assert_eq!(rows_in_view(px(480.), px(0.), 3), 1);
        assert_eq!(rows_in_view(px(0.), px(4800.), 100), 1);
        assert_eq!(rows_in_view(px(40.), px(4800.), 100), 1);
    }

    #[gpui_kit::test]
    fn each_key_asks_for_its_move(cx: &mut TestAppContext) {
        let (list, cx) = list(cx);
        focus(|list| list.focus.clone(), &list, cx);

        cx.simulate_keystrokes("down up home end pagedown pageup");

        cx.update(|_, cx| {
            assert_eq!(
                list.read(cx).motions,
                [
                    Motion::By(1),
                    Motion::By(-1),
                    Motion::First,
                    Motion::Last,
                    Motion::Pages(1),
                    Motion::Pages(-1),
                ]
            );
        });
    }

    #[gpui_kit::test]
    fn enter_and_space_stay_with_a_control_inside_the_list(cx: &mut TestAppContext) {
        let (list, cx) = list(cx);
        focus(|list| list.control_focus.clone(), &list, cx);

        cx.simulate_keystrokes("enter space");
        cx.update(|_, cx| assert_eq!(list.read(cx).opened, 0));

        focus(|list| list.focus.clone(), &list, cx);
        cx.simulate_keystrokes("enter space");
        cx.update(|_, cx| assert_eq!(list.read(cx).opened, 2));
    }

    #[test]
    fn motions_stay_within_the_list() {
        assert_eq!(Motion::By(1).apply(Some(9), 10, 4), Some(9));
        assert_eq!(Motion::By(-1).apply(Some(0), 10, 4), Some(0));
        assert_eq!(Motion::Pages(1).apply(Some(2), 10, 4), Some(6));
        assert_eq!(Motion::Pages(1).apply(Some(8), 10, 4), Some(9));
        assert_eq!(Motion::Pages(-1).apply(Some(2), 10, 4), Some(0));
        assert_eq!(Motion::First.apply(Some(5), 10, 4), Some(0));
        assert_eq!(Motion::Last.apply(Some(5), 10, 4), Some(9));
    }

    #[test]
    fn a_motion_from_no_selection_starts_at_an_end() {
        assert_eq!(Motion::By(1).apply(None, 10, 4), Some(0));
        assert_eq!(Motion::Pages(-1).apply(None, 10, 4), Some(0));
        assert_eq!(Motion::Last.apply(None, 10, 4), Some(9));
        assert_eq!(Motion::By(1).apply(None, 0, 4), None);
    }
}
