use std::rc::Rc;

use anti_webui::core::geometry::{PointF, RectF};
use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    IconButtonStyle, MenuItem, MenuStyle, icon_toggle_button, styled_menu,
};
use anti_webui_md3e::icons;

use crate::Covers;
use crate::row_list::{is_context_click, row_control};

/// A menu of the items given as it opens: where the pointer was pressed, or hanging from a button.
/// One serves a whole screen, as only one menu is open at a time. Cheap to copy.
///
/// Its [`view`](Self::view) goes anywhere in the screen.
#[derive(Clone, Copy)]
pub struct ContextMenu {
    open: Signal<bool>,
    /// What the menu hangs from, in logical pixels of the window: where the pointer was pressed,
    /// or the button it was opened from as the button was then.
    at: Signal<RectF>,
    placement: Signal<Placement>,
    /// The button it was opened from. The menu hangs from a copy of where the button was rather
    /// than from the button, which can go while the menu still closes.
    from: Signal<Option<NodeRef>>,
    groups: Signal<Rc<Vec<Vec<MenuItem>>>>,
    /// How many times it has opened, as each time is a menu of its own.
    opened: Signal<u64>,
    point: NodeRef,
}

impl ContextMenu {
    /// Counted in `covers` while it is open, for as long as the current owner lives.
    pub fn new(covers: Covers) -> Self {
        let open = signal(false);
        covers.add(move || open.get());
        Self {
            open,
            at: signal(RectF::new(0.0, 0.0, 0.0, 0.0)),
            placement: signal(Placement::BelowStart),
            from: signal(None),
            groups: signal(Rc::new(Vec::new())),
            opened: signal(0),
            point: node_ref(),
        }
    }

    /// Opens `groups` at `at`, in logical pixels of the window, as
    /// [`RowList::on_context_menu`](crate::row_list::RowList::on_context_menu) gives it.
    pub fn open_at(&self, at: PointF, groups: Vec<Vec<MenuItem>>) {
        self.at.set(RectF::new(at.x, at.y, 0.0, 0.0));
        self.placement.set(Placement::BelowStart);
        self.from.set(None);
        self.show(groups);
    }

    /// Opens `groups` hanging from `anchor`.
    pub fn open_from(&self, anchor: NodeRef, placement: Placement, groups: Vec<Vec<MenuItem>>) {
        if let Some(rect) = anchor.logical_rect() {
            self.at.set(rect);
        }
        self.placement.set(placement);
        self.from.set(Some(anchor));
        self.show(groups);
    }

    fn show(&self, groups: Vec<Vec<MenuItem>>) {
        self.groups.set(Rc::new(groups));
        self.opened.update(|opened| *opened += 1);
        self.open.set(true);
    }

    pub fn close(&self) {
        self.open.set(false);
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Whether it is open and hangs from `anchor`.
    pub fn is_open_from(&self, anchor: NodeRef) -> bool {
        self.open.get() && self.from.with(|from| *from == Some(anchor))
    }

    pub fn view(&self) -> Div {
        let Self {
            open,
            at,
            placement,
            groups,
            opened,
            point,
            ..
        } = *self;
        let menu = dyn_view(move || {
            opened.get();
            untrack(|| {
                styled_menu(
                    move || open.get(),
                    move || open.set(false),
                    groups.with(|groups| groups.as_ref().clone()),
                    MenuStyle::new().anchor(point).placement(placement.get()),
                )
            })
        });
        // Popups hang from nodes, so what the menu hangs from is one that can't be pressed. In a
        // portal, it is placed in the window whatever is around the view.
        let point = portal().child(
            div()
                .node_ref(point)
                .absolute()
                .pointer_events_none()
                .access(|a| a.hidden())
                .dyn_style(move |s| {
                    let at = at.get();
                    s.left(at.x).top(at.y).w(at.width).h(at.height)
                }),
        );
        div().absolute().w(0).h(0).child(point).child(menu)
    }
}

/// Where `e` was pressed, in logical pixels of the window, as a menu opens at.
pub fn pressed_at(e: &PointerEvent) -> PointF {
    let scale = window_scale();
    PointF::new(
        scale.to_logical(e.position.x),
        scale.to_logical(e.position.y),
    )
}

/// The button at the end of a row or a bar that opens `menu` with what `groups` makes then, below
/// it. It looks pressed while that is open. Pressing it does nothing to the row it is in.
pub fn more_button(menu: ContextMenu, groups: impl Fn() -> Vec<Vec<MenuItem>> + 'static) -> Div {
    let anchor = node_ref();
    // What a press opens is told below, which leaves a Control-click to the row's own menu
    let open = Binding::new(move || menu.is_open_from(anchor), |_| {});
    let button = icon_toggle_button(icons::MORE_HORIZ, open, IconButtonStyle::standard())
        .node_ref(anchor)
        .access(move |a| {
            a.role(Role::Button)
                .label("More")
                .expanded(menu.is_open_from(anchor))
        })
        .on_click(move |e| {
            if !is_context_click(e) {
                menu.open_from(anchor, Placement::BelowEnd, groups());
            }
        });
    row_control(button)
}

#[cfg(test)]
mod tests;
