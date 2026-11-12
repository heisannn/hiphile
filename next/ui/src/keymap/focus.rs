//! Where the focus is, and the window's own root. AntiWebUI has no public way to either, so this
//! reads and changes its document, which it leaves out of its docs and its semver promises; the
//! revision it is pinned to has the fields used.

use std::cell::Cell;
use std::rc::Rc;

use anti_webui::core::tree::{NodeId, with_doc};
use anti_webui::prelude::{KeyboardEvent, NodeRef, untrack};

thread_local! {
    /// The window's root that [`on_root_key_down`] gave a handler to.
    static HOOKED: Cell<Option<NodeId>> = const { Cell::new(None) };
}

pub(super) fn focused() -> Option<NodeId> {
    with_doc(|d| d.state.focused)
}

/// Moves the focus to `node` with the next update, if it is still in the window.
pub(super) fn give(node: NodeId) {
    with_doc(|d| {
        if d.get(node).is_some() {
            d.pending_focus = Some(Some(node));
        }
    });
}

pub(super) fn is_in_text_field() -> bool {
    with_doc(|d| {
        d.state
            .focused
            .and_then(|id| d.get(id))
            .is_some_and(|node| node.is_edit() || node.is_editor())
    })
}

/// `node` if it is still in the window.
pub(super) fn node_of(node: NodeRef) -> Option<NodeId> {
    let id = untrack(|| node.get())?;
    with_doc(|d| d.get(id).is_some()).then_some(id)
}

/// `node` and what it is in, from the inside out.
pub(super) fn chain_from(node: NodeId) -> Vec<NodeId> {
    with_doc(|d| d.ancestors(node))
}

pub(super) fn is_modal_open() -> bool {
    with_doc(|d| {
        d.popups.iter().any(|id| {
            d.get(*id)
                .is_some_and(|node| node.visible && node.popup().is_some_and(|popup| popup.modal))
        })
    })
}

/// Gives the window's own root, which an app cannot reach, `handler` for the keys pressed. Once
/// for each root, after it is there.
pub(super) fn on_root_key_down(handler: fn(&KeyboardEvent)) {
    with_doc(|d| {
        let Some(root) = d.root else {
            return;
        };
        if HOOKED.with(Cell::get) == Some(root) {
            return;
        }
        let Some(node) = d.get_mut(root) else {
            return;
        };
        let before = node.listeners.key_down.take();
        node.listeners.key_down = Some(Rc::new(move |e: &KeyboardEvent| {
            if let Some(before) = &before {
                before(e);
            }
            handler(e);
        }));
        HOOKED.with(|hooked| hooked.set(Some(root)));
    });
}
