//! What the app's own dialogs are made of, in `anti_webui_md3e::components::dialog`: a heading
//! above what the dialog asks, the buttons at its end, and the focus given to what it asks first.

use std::cell::Cell;

use anti_webui::core::tree::{Document, NodeId, with_doc};
use anti_webui::prelude::*;
use anti_webui_md3e::typography::TextStyle;

use crate::type_scale::styled_text;

/// The dialog's content, below a heading of `title`. Add what it asks and then
/// [`dialog_actions`].
pub fn dialog_content(title: &'static str) -> Div {
    div().flex().flex_col().gap(16.0_f32).p(24.0_f32).child(
        styled_text(TextStyle::HEADLINE_SMALL, title).access(|a| a.role(Role::Heading).level(2)),
    )
}

/// The row of buttons at the end of a dialog, the one that commits last.
pub fn dialog_actions() -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(8.0_f32)
        .pt(8.0_f32)
}

/// `field`, an `outlined_text_field`, whose input takes the focus as the dialog opens, with the
/// caret after what it holds.
pub fn focused_field(field: Div) -> Div {
    once_laid_out(field, |doc, id| {
        let Some(input) = first_input(doc, id) else {
            return;
        };
        if let Some(edit) = doc.get_mut(input).and_then(|node| node.edit_mut()) {
            let end = edit.text.len();
            edit.select_range(end, end);
        }
        doc.pending_focus = Some(Some(input));
    })
}

/// Focuses what `target` finds in `node` once `node` is laid out. A dialog takes the focus itself
/// when it is built, after its content, so its content can't take it any sooner.
pub fn focused_once_laid_out(
    node: Div,
    target: impl Fn(&Document, NodeId) -> Option<NodeId> + 'static,
) -> Div {
    once_laid_out(node, move |doc, id| {
        if let Some(target) = target(doc, id) {
            doc.pending_focus = Some(Some(target));
        }
    })
}

/// Runs `f` with `node` the first time it is laid out.
fn once_laid_out(node: Div, f: impl Fn(&mut Document, NodeId) + 'static) -> Div {
    let at = node_ref();
    let done = Cell::new(false);
    node.node_ref(at).on_resized(move |_, _| {
        if done.replace(true) {
            return;
        }
        if let Some(id) = at.get() {
            with_doc(|doc| f(doc, id));
        }
    })
}

/// The text field has no handle to the input it is made of, so it is found among its nodes.
fn first_input(doc: &Document, id: NodeId) -> Option<NodeId> {
    let node = doc.get(id)?;
    if node.is_edit() {
        return Some(id);
    }
    node.children
        .iter()
        .find_map(|child| first_input(doc, *child))
}
