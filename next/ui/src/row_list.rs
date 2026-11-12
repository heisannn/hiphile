//! The long lists of the library: rows of one height, of which only those in view exist, with a
//! selection that stays with the rows' ids as they are sorted and filtered.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::Ink;
use hiphile_core::Shown;

use crate::keymap::{Keystroke, handles_keys};

mod selection;

pub use selection::{Motion, Selected};

use selection::Order;

/// Rows with covers. Other rows can be lower, but all of one list are as high.
pub const ROW_HEIGHT: f32 = 48.0;
/// From the edge of the list to the rows, whose corners are rounded.
const INSET: f32 = 8.0;
/// From a row's edge to what it shows, which puts that on the line of the rest of a pane.
pub const ROW_PADDING: f32 = crate::pane::INSET - INSET;

/// What a row list shows: items in the order they show.
pub trait RowSource: 'static {
    type Item: 'static;

    fn len(&self) -> usize;
    fn get(&self, position: usize) -> Option<&Self::Item>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: 'static> RowSource for Shown<T> {
    type Item = T;

    fn len(&self) -> usize {
        self.visible().len()
    }

    fn get(&self, position: usize) -> Option<&T> {
        Shown::get(self, position)
    }
}

impl<T: 'static> RowSource for Arc<[T]> {
    type Item = T;

    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    fn get(&self, position: usize) -> Option<&T> {
        <[T]>::get(self, position)
    }
}

impl<T: 'static> RowSource for Rc<[T]> {
    type Item = T;

    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    fn get(&self, position: usize) -> Option<&T> {
        <[T]>::get(self, position)
    }
}

/// Which rows of a list are selected, which its owner reads and changes. Cheap to copy.
pub struct Selection<K: 'static>(Signal<Selected<K>>);

impl<K> Clone for Selection<K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for Selection<K> {}

impl<K: Clone + Eq + Hash + 'static> Default for Selection<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + Eq + Hash + 'static> Selection<K> {
    pub fn new() -> Self {
        Self(signal(Selected::default()))
    }

    /// Reads it, which tracks it.
    pub fn with<R>(&self, f: impl FnOnce(&Selected<K>) -> R) -> R {
        self.0.with(f)
    }

    pub fn contains(&self, id: &K) -> bool {
        self.0.with(|selected| selected.contains(id))
    }

    pub fn cursor(&self) -> Option<K> {
        self.0.with(|selected| selected.cursor().cloned())
    }

    /// Those of `ids` that are selected, in their order: what a command on the selection takes,
    /// given the ids of the rows in the order they show.
    pub fn in_order(&self, ids: impl IntoIterator<Item = K>) -> Vec<K> {
        self.0
            .with(|selected| ids.into_iter().filter(|id| selected.contains(id)).collect())
    }

    /// Selects `id` and nothing else.
    pub fn select(&self, id: K) {
        self.0.update(|selected| selected.select_only(id));
    }

    pub fn clear(&self) {
        self.0.update(Selected::clear);
    }
}

/// What a row's look is given: which row it is and whether it is selected.
pub struct RowState<K: 'static> {
    id: K,
    position: usize,
    selection: Selection<K>,
}

impl<K: Clone + Eq + Hash + 'static> RowState<K> {
    pub fn id(&self) -> &K {
        &self.id
    }

    /// Where it is among the rows. Another row is made for another place.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Tracks it.
    pub fn is_selected(&self) -> bool {
        self.selection.contains(&self.id)
    }
}

type Look<S, K> = Rc<dyn Fn(&<S as RowSource>::Item, RowState<K>) -> Div>;
type IdOf<S, K> = Rc<dyn Fn(&<S as RowSource>::Item) -> K>;
type LabelOf<S> = Rc<dyn Fn(&<S as RowSource>::Item) -> String>;
type OnReorder<K> = Rc<dyn Fn(Vec<K>, usize)>;

/// A row list to be made by [`row_list`].
pub struct RowList<S: RowSource, K: 'static> {
    label: String,
    rows: Rc<dyn Fn() -> S>,
    id: IdOf<S, K>,
    row: Look<S, K>,
    row_label: Option<LabelOf<S>>,
    selection: Option<Selection<K>>,
    row_height: f32,
    node: Option<NodeRef>,
    on_open: Option<Rc<dyn Fn(K)>>,
    on_context_menu: Option<Rc<dyn Fn(K, PointF)>>,
    on_reorder: Option<OnReorder<K>>,
}

impl<S: RowSource, K: Clone + Eq + Hash + 'static> RowList<S, K> {
    /// `label` names the list for assistive technology. `rows` may read signals, and the list
    /// follows them. `row` makes what a row shows; it runs again when another row comes to its
    /// place, but not for a change to the item of the same id, which it should follow itself.
    pub fn new(
        label: impl Into<String>,
        rows: impl Fn() -> S + 'static,
        id: impl Fn(&S::Item) -> K + 'static,
        row: impl Fn(&S::Item, RowState<K>) -> Div + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            rows: Rc::new(rows),
            id: Rc::new(id),
            row: Rc::new(row),
            row_label: None,
            selection: None,
            row_height: ROW_HEIGHT,
            node: None,
            on_open: None,
            on_context_menu: None,
            on_reorder: None,
        }
    }

    /// The selection, for the owner to read and change. The list has one of its own otherwise.
    pub fn selection(mut self, selection: Selection<K>) -> Self {
        self.selection = Some(selection);
        self
    }

    /// What a row is called, for assistive technology. It is what the row shows otherwise. It may
    /// read signals, such as whether the row's song plays, and follows them.
    pub fn row_label(mut self, label: impl Fn(&S::Item) -> String + 'static) -> Self {
        self.row_label = Some(Rc::new(label));
        self
    }

    pub fn row_height(mut self, height: f32) -> Self {
        self.row_height = height;
        self
    }

    /// Refers to what takes the focus and scrolls.
    pub fn node_ref(mut self, node: NodeRef) -> Self {
        self.node = Some(node);
        self
    }

    /// Enter on the cursor's row, and double-clicking a row, open it, or play it.
    pub fn on_open(mut self, open: impl Fn(K) + 'static) -> Self {
        self.on_open = Some(Rc::new(open));
        self
    }

    /// The secondary button on a row, or Control and the primary one, which first selects the row
    /// unless it is already. Given where it was pressed, in logical pixels of the window.
    pub fn on_context_menu(mut self, open: impl Fn(K, PointF) + 'static) -> Self {
        self.on_context_menu = Some(Rc::new(open));
        self
    }

    /// Makes rows draggable onto the other rows of the list: the selected ones when one of them is
    /// dragged, and else the one dragged. Given their ids, in their order, and the position they
    /// go before, which is the number of rows for after the last.
    pub fn on_reorder(mut self, reorder: impl Fn(Vec<K>, usize) + 'static) -> Self {
        self.on_reorder = Some(Rc::new(reorder));
        self
    }
}

/// The rows, the selection and the cursor, as one list knows them.
struct Shared<S: RowSource, K: 'static> {
    rows: Signal<S>,
    id: IdOf<S, K>,
    /// Where each id is, made when first asked for after the rows change.
    positions: RefCell<Option<Rc<HashMap<K, usize>>>>,
}

impl<S: RowSource, K: Clone + Eq + Hash + 'static> Order<K> for Shared<S, K> {
    fn len(&self) -> usize {
        self.rows.with_untracked(RowSource::len)
    }

    fn id_at(&self, position: usize) -> Option<K> {
        self.rows
            .with_untracked(|rows| rows.get(position).map(|item| (self.id)(item)))
    }

    fn position_of(&self, id: &K) -> Option<usize> {
        let positions = self
            .positions
            .borrow_mut()
            .get_or_insert_with(|| {
                Rc::new(self.rows.with_untracked(|rows| {
                    (0..rows.len())
                        .filter_map(|position| Some(((self.id)(rows.get(position)?), position)))
                        .collect()
                }))
            })
            .clone();
        positions.get(id).copied()
    }
}

/// The rows dragged to another place, which only the list they come from takes.
#[derive(Clone)]
struct DraggedRows<K> {
    list: u64,
    ids: Vec<K>,
}

thread_local! {
    static NEXT_LIST: Cell<u64> = const { Cell::new(0) };
}

/// The list, which takes the focus as one: the keys move the cursor and select, Shift with them
/// selects from the anchor, Home, End, Page Up and Page Down go further, ⌘A selects every row and
/// Enter opens the cursor's row. Clicking selects a row, Shift-clicking those from the anchor and
/// ⌘-clicking adds or takes away one. It takes the room it is given.
pub fn row_list<S: RowSource, K: Clone + Eq + Hash + 'static>(list: RowList<S, K>) -> Div {
    let RowList {
        label,
        rows,
        id,
        row,
        row_label,
        selection,
        row_height,
        node,
        on_open,
        on_context_menu,
        on_reorder,
    } = list;
    let selection = selection.unwrap_or_else(Selection::new);
    let node = node.unwrap_or_else(node_ref);
    let focused = signal(false);
    let shown = signal(untrack(|| rows()));
    let shared = Rc::new(Shared {
        rows: shown,
        id: id.clone(),
        positions: RefCell::default(),
    });
    {
        let shared = shared.clone();
        effect(move || {
            let now = rows();
            untrack(|| follow_rows(&shared, now, selection, node, row_height));
        });
    }
    let me = NEXT_LIST.with(|next| next.replace(next.get() + 1));
    let drop_at = signal(None::<(usize, bool)>);
    let place = Place {
        shared: shared.clone(),
        selection,
        focused,
        node,
        row,
        row_label,
        on_open: on_open.clone(),
        on_context_menu,
        reorder: on_reorder.map(|reorder| Reorder {
            list: me,
            reorder,
            drop_at,
        }),
    };
    let place = Rc::new(place);
    let list = virtual_list(
        move || shown.with(RowSource::len),
        row_height,
        move |position| place.row_at(position),
    )
    .node_ref(node)
    .focusable()
    .select_none()
    .access(move |a| a.role(Role::ListBox).label(label.clone()).multiselectable())
    .on_focus_change(move |on| focused.set(on));
    handles_keys(list, move |stroke| {
        take_key(
            stroke,
            &shared,
            selection,
            node,
            row_height,
            on_open.as_deref(),
        )
    })
}

/// Keeps the ids' positions to the rows, and the cursor in view if it was before they changed.
fn follow_rows<S: RowSource, K: Clone + Eq + Hash + 'static>(
    shared: &Shared<S, K>,
    rows: S,
    selection: Selection<K>,
    node: NodeRef,
    row_height: f32,
) {
    let cursor = selection
        .0
        .with_untracked(|selected| selected.cursor().cloned());
    let before = cursor.as_ref().and_then(|cursor| {
        in_view(node, row_height).find(|position| shared.id_at(*position).as_ref() == Some(cursor))
    });
    shared.positions.replace(None);
    shared.rows.set(rows);
    let (Some(cursor), Some(before)) = (cursor, before) else {
        return;
    };
    if let Some(after) = shared.position_of(&cursor)
        && after != before
    {
        reveal(node, after, row_height);
    }
}

/// The positions of the rows in view, in part or whole.
fn in_view(node: NodeRef, row_height: f32) -> std::ops::Range<usize> {
    let Some(rect) = node.logical_rect() else {
        return 0..0;
    };
    let each = device_row_height(row_height);
    let top = node.scroll_offset().y;
    let first = (top / each).floor().max(0.0) as usize;
    let last = ((top + rect.height) / each).ceil().max(0.0) as usize;
    first..last
}

/// A row's height on the device's pixels, in logical pixels, as the list places them.
fn device_row_height(row_height: f32) -> f32 {
    let scale = untrack(window_scale);
    scale.px(row_height) as f32 / scale.as_f32()
}

/// Scrolls the list just far enough to show the row at `position`, once it is laid out, which it
/// may not be yet for rows that just came.
fn reveal(node: NodeRef, position: usize, row_height: f32) {
    let Some(rect) = node.logical_rect() else {
        return;
    };
    let each = device_row_height(row_height);
    let at = node.scroll_offset();
    let top = each * position as f32;
    let y = if top < at.y {
        top
    } else if top + each > at.y + rect.height {
        top + each - rect.height
    } else {
        return;
    };
    let scale = untrack(window_scale);
    node.scroll_to_after_layout(scale.px(at.x), scale.px(y));
}

/// Whole rows in view, at least one.
fn rows_in_view(node: NodeRef, row_height: f32) -> usize {
    node.logical_rect()
        .map_or(1, |rect| (rect.height / row_height).floor() as usize)
        .max(1)
}

fn take_key<S: RowSource, K: Clone + Eq + Hash + 'static>(
    stroke: &Keystroke,
    shared: &Shared<S, K>,
    selection: Selection<K>,
    node: NodeRef,
    row_height: f32,
    on_open: Option<&dyn Fn(K)>,
) -> bool {
    let m = stroke.modifiers();
    if m.primary() && !m.shift && !m.alt && stroke.key().is_char('a') {
        selection.0.update(|selected| selected.select_all(shared));
        return true;
    }
    let Key::Named(key) = stroke.key() else {
        return false;
    };
    if m.ctrl || m.alt || m.logo {
        return false;
    }
    let motion = match key {
        NamedKey::Enter if !m.shift => {
            let (Some(open), Some(cursor)) = (on_open, selection.cursor()) else {
                return false;
            };
            if shared.position_of(&cursor).is_none() {
                return false;
            }
            open(cursor);
            return true;
        }
        NamedKey::ArrowDown => Motion::By(1),
        NamedKey::ArrowUp => Motion::By(-1),
        NamedKey::PageDown => Motion::Pages(1),
        NamedKey::PageUp => Motion::Pages(-1),
        NamedKey::Home => Motion::First,
        NamedKey::End => Motion::Last,
        _ => return false,
    };
    let page = rows_in_view(node, row_height);
    let to = selection
        .0
        .update(|selected| selected.move_by(motion, m.shift, shared, page));
    if let Some(to) = to {
        reveal(node, to, row_height);
    }
    true
}

struct Reorder<K: 'static> {
    list: u64,
    reorder: OnReorder<K>,
    /// The row a drag is over, and whether past its middle.
    drop_at: Signal<Option<(usize, bool)>>,
}

/// What every row of one list shares.
struct Place<S: RowSource, K: 'static> {
    shared: Rc<Shared<S, K>>,
    selection: Selection<K>,
    focused: Signal<bool>,
    node: NodeRef,
    row: Look<S, K>,
    row_label: Option<LabelOf<S>>,
    on_open: Option<Rc<dyn Fn(K)>>,
    on_context_menu: Option<Rc<dyn Fn(K, PointF)>>,
    reorder: Option<Reorder<K>>,
}

impl<S: RowSource, K: Clone + Eq + Hash + 'static> Place<S, K> {
    /// The row at `position`, made again when another id comes there.
    fn row_at(self: &Rc<Self>, position: usize) -> View {
        let rows = self.shared.rows;
        let id = self.shared.id.clone();
        let here = memo(move || rows.with(|rows| rows.get(position).map(|item| id(item))));
        let this = self.clone();
        dyn_view(move || {
            let Some(id) = here.get() else {
                return div();
            };
            untrack(|| {
                rows.with(|rows| rows.get(position).map(|item| this.row(item, id, position)))
            })
            .unwrap_or_else(div)
        })
    }

    fn row(&self, item: &S::Item, id: K, position: usize) -> Div {
        let selection = self.selection;
        let focused = self.focused;
        let selected = {
            let id = id.clone();
            memo(move || selection.contains(&id))
        };
        let label = self.row_label.clone().map(|label| {
            let rows = self.shared.rows;
            move || rows.with(|rows| rows.get(position).map(|item| label(item)))
        });
        let look = (self.row)(
            item,
            RowState {
                id: id.clone(),
                position,
                selection,
            },
        );
        let ink = Ink::new(
            move |c| {
                if selected.get() && focused.get() {
                    c.on_secondary_container
                } else {
                    c.on_surface
                }
            },
            || false,
        );
        let frame = div()
            .absolute()
            .top(0)
            .bottom(0)
            .left(INSET)
            .right(INSET)
            .flex()
            .flex_row()
            .items_center()
            .rounded(12.0_f32)
            .access(move |a| {
                let a = a.role(Role::ListOption).selected(selected.get());
                match label.as_ref().and_then(|label| label()) {
                    Some(label) => a.label(label),
                    None => a,
                }
            })
            .dyn_style(move |s| {
                let c = color_scheme();
                match (selected.get(), focused.get()) {
                    (true, true) => s
                        .bg(c.secondary_container)
                        .text_color(c.on_secondary_container),
                    (true, false) => s.bg(c.surface_container_highest).text_color(c.on_surface),
                    (false, _) => s.text_color(c.on_surface),
                }
            })
            .on_click(self.click(id.clone()))
            .on_pointer_down(self.press(id.clone()))
            .child(look.flex_1().min_w(0).h_full());
        let frame = match &self.reorder {
            Some(reorder) => self.reorderable(frame, reorder, id, position),
            None => frame,
        };
        ink.paint(ink.listen(frame))
    }

    fn click(&self, id: K) -> impl Fn(&PointerEvent) + 'static {
        let (shared, selection, on_open) =
            (self.shared.clone(), self.selection, self.on_open.clone());
        move |e| {
            let m = e.modifiers;
            if is_context_click(e) {
                return;
            }
            if m.primary() {
                selection.0.update(|selected| selected.toggle(id.clone()));
            } else if m.shift {
                selection
                    .0
                    .update(|selected| selected.extend_to(id.clone(), &*shared));
            } else {
                selection
                    .0
                    .update(|selected| selected.select_only(id.clone()));
                if e.click_count == 2
                    && let Some(open) = &on_open
                {
                    open(id.clone());
                }
            }
        }
    }

    fn press(&self, id: K) -> impl Fn(&PointerEvent) + 'static {
        let (selection, node, on_context_menu) =
            (self.selection, self.node, self.on_context_menu.clone());
        move |e| {
            let Some(open) = &on_context_menu else {
                return;
            };
            if !is_context_click(e) {
                return;
            }
            if !selection.contains(&id) {
                selection.select(id.clone());
            }
            node.focus();
            let scale = window_scale();
            open(
                id.clone(),
                PointF::new(
                    scale.to_logical(e.position.x),
                    scale.to_logical(e.position.y),
                ),
            );
        }
    }

    fn reorderable(&self, frame: Div, reorder: &Reorder<K>, id: K, position: usize) -> Div {
        let (shared, selection) = (self.shared.clone(), self.selection);
        let (list, drop_at) = (reorder.list, reorder.drop_at);
        let to_drop = reorder.reorder.clone();
        let line = memo(move || {
            drop_at
                .get()
                .filter(|(at, _)| *at == position)
                .map(|(_, after)| after)
        });
        frame
            .draggable(move || DraggedRows {
                list,
                ids: if selection
                    .0
                    .with_untracked(|selected| selected.contains(&id))
                {
                    selected_in_order(&shared, selection)
                } else {
                    vec![id.clone()]
                },
            })
            .dragging(|s| s.opacity(0.4_f32))
            .drop_accepts(move |dragged: &DraggedRows<K>| dragged.list == list)
            .on_drag_over(move |_: &DraggedRows<K>, e| {
                drop_at.set_if_changed(Some((position, e.fraction().y >= 0.5)));
            })
            .on_drag_hover(move |on| {
                if !on {
                    drop_at.update(|at| {
                        if at.is_some_and(|(at, _)| at == position) {
                            *at = None;
                        }
                    });
                }
            })
            .drop_target(move |dragged: &DraggedRows<K>, e| {
                drop_at.set(None);
                let to = if e.fraction().y >= 0.5 {
                    position + 1
                } else {
                    position
                };
                to_drop(dragged.ids.clone(), to);
            })
            .child(
                div()
                    .absolute()
                    .left(0)
                    .right(0)
                    .h(2)
                    .rounded(1.0_f32)
                    .pointer_events_none()
                    .dyn_style(move |s| match line.get() {
                        Some(after) => {
                            let s = s.bg(color_scheme().primary);
                            if after { s.bottom(-1) } else { s.top(-1) }
                        }
                        None => s.opacity(0.0_f32),
                    }),
            )
    }
}

fn selected_in_order<S: RowSource, K: Clone + Eq + Hash + 'static>(
    shared: &Shared<S, K>,
    selection: Selection<K>,
) -> Vec<K> {
    selection.0.with_untracked(|selected| {
        (0..shared.len())
            .filter_map(|position| shared.id_at(position))
            .filter(|id| selected.contains(id))
            .collect()
    })
}

/// Makes `control` in a row do only what it does itself: pressing it does not select, open or
/// ripple the row. Opening the row's menu with the secondary button still reaches the row.
pub fn row_control(control: Div) -> Div {
    control
        .on_pointer_down(|e| {
            if !is_context_click(e) {
                e.stop_propagation();
            }
        })
        .on_click(|e| {
            if !is_context_click(e) {
                e.stop_propagation();
            }
        })
}

/// The secondary button, or Control and the primary one on macOS.
pub(crate) fn is_context_click(e: &PointerEvent) -> bool {
    match e.button {
        Some(PointerButton::Secondary) => true,
        Some(PointerButton::Primary) => cfg!(target_os = "macos") && e.modifiers.ctrl,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
