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
