use std::cmp::Ordering;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, AppContext as _, SharedString, StatefulInteractiveElement, Styled as _, Window,
};

use crate::DragPreview;

/// A row being dragged to a new place. Rows of lists with different `Id`s take
/// no drops from each other.
struct DraggedRow<Id> {
    id: Id,
    /// Where it was when the drag started, which tells the rows under the
    /// pointer which side it would land on.
    from_ix: usize,
    title: SharedString,
}

/// Lets a row be dragged to a new place in its list.
pub trait ReorderExt: StatefulInteractiveElement + Sized {
    /// Makes this the row of `id` at `ix`, titled `title` while dragged.
    /// `on_drop` is given the id of a row dropped on this one.
    fn reorderable<Id: Copy + 'static>(
        self,
        id: Id,
        ix: usize,
        title: SharedString,
        on_drop: impl Fn(Id, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drag(
            DraggedRow {
                id,
                from_ix: ix,
                title,
            },
            |dragged: &DraggedRow<Id>, offset, _, cx| {
                cx.new(|_| DragPreview::new(dragged.title.clone(), offset))
            },
        )
        // A line on the edge the row would land beside.
        .drag_over::<DraggedRow<Id>>(move |style, dragged, _, cx| {
            let line = cx.theme().drag_border;
            match dragged.from_ix.cmp(&ix) {
                Ordering::Less => style.border_b_2().border_color(line),
                Ordering::Greater => style.border_t_2().border_color(line),
                Ordering::Equal => style,
            }
        })
        .on_drop(move |dragged: &DraggedRow<Id>, window, cx| on_drop(dragged.id, window, cx))
    }
}

impl<E: StatefulInteractiveElement + Sized> ReorderExt for E {}

#[cfg(test)]
mod tests {
    use gpui_kit::{
        Context, Entity, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
        ParentElement as _, Pixels, Point, Render, TestAppContext, VisualTestContext, div, point,
        px,
    };

    use super::*;

    /// Three rows of one kind of id above a row of another, one above the other.
    #[derive(Default)]
    struct TestRows {
        /// What was dropped on what.
        drops: Vec<(u32, u32)>,
        other_drops: Vec<(u64, u64)>,
    }

    impl Render for TestRows {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let owner = cx.entity().downgrade();
            let row_owner = owner.clone();
            let rows = (0..3u32).map(move |id| {
                let owner = row_owner.clone();
                div().id(("row", id)).w_full().h_10().reorderable(
                    id,
                    id as usize,
                    "row".into(),
                    move |dragged, _, cx| {
                        owner
                            .update(cx, |this, _| this.drops.push((dragged, id)))
                            .ok();
                    },
                )
            });
            let other = div().id("other").w_full().h_10().reorderable(
                9u64,
                3,
                "other".into(),
                move |dragged, _, cx| {
                    owner
                        .update(cx, |this, _| this.other_drops.push((dragged, 9)))
                        .ok();
                },
            );
            div().size_full().children(rows).child(other)
        }
    }

    fn rows(cx: &mut TestAppContext) -> (Entity<TestRows>, &mut VisualTestContext) {
        // The drag line reads the theme.
        cx.update(gpui_kit::init);
        let (rows, cx) = cx.add_window_view(|_, _| TestRows::default());
        cx.run_until_parked();
        (rows, cx)
    }

    fn row_center(ix: usize, cx: &mut VisualTestContext) -> Point<Pixels> {
        let height = cx.update(|window, _| window.rem_size()) * 2.5;
        point(px(100.), height * (ix as f32 + 0.5))
    }

    fn drag(from: usize, to: usize, cx: &mut VisualTestContext) {
        let (from, to) = (row_center(from, cx), row_center(to, cx));
        let none = Modifiers::default();
        cx.simulate_mouse_move(from, None, none);
        cx.simulate_mouse_down(from, MouseButton::Left, none);
        cx.simulate_mouse_move(to, MouseButton::Left, none);
        cx.simulate_mouse_move(to, MouseButton::Left, none);
        cx.simulate_mouse_up(to, MouseButton::Left, none);
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn a_row_dropped_on_another_tells_that_row_which_it_was(cx: &mut TestAppContext) {
        let (rows, cx) = rows(cx);

        drag(0, 2, cx);
        drag(2, 1, cx);

        cx.update(|_, cx| assert_eq!(rows.read(cx).drops, [(0, 2), (2, 1)]));
    }

    #[gpui_kit::test]
    fn a_row_takes_no_drop_from_a_list_of_another_kind_of_id(cx: &mut TestAppContext) {
        let (rows, cx) = rows(cx);

        drag(0, 3, cx);
        drag(3, 0, cx);

        cx.update(|_, cx| {
            assert!(rows.read(cx).drops.is_empty());
            assert!(rows.read(cx).other_drops.is_empty());
        });
    }

    #[gpui_kit::test]
    fn a_short_press_is_no_drag(cx: &mut TestAppContext) {
        let (rows, cx) = rows(cx);
        let from = row_center(0, cx);
        let none = Modifiers::default();

        cx.simulate_mouse_move(from, None, none);
        cx.simulate_mouse_down(from, MouseButton::Left, none);
        cx.update(|_, cx| assert!(!cx.has_active_drag()));
        cx.simulate_mouse_up(from, MouseButton::Left, none);
        cx.run_until_parked();

        cx.update(|_, cx| assert!(rows.read(cx).drops.is_empty()));
    }
}
