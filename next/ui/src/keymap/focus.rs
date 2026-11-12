//! Where the focus is, and the window's own root. AntiWebUI has no public way to either, so this
//! reads and changes its document, which it leaves out of its docs and its semver promises; the
//! revision it is pinned to has the fields used.

use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::tree::{NodeId, with_doc};
use anti_webui::prelude::{KeyboardEvent, NodeRef, untrack};

type KeyDown = Rc<dyn Fn(&KeyboardEvent)>;

/// What [`on_root_key_down`] gave the window's root, and what the root had before.
struct Hooked {
    ours: KeyDown,
    before: Option<KeyDown>,
}

thread_local! {
    /// Known by the handler itself rather than by the root: the root of a window made after one
    /// closed can have the same id.
    static HOOKED: RefCell<Option<Hooked>> = const { RefCell::new(None) };
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

/// Gives the window's own root, which an app cannot reach, `handler` for the keys pressed, once
/// it is there. A root that has it already is left as it is.
pub(super) fn on_root_key_down(handler: fn(&KeyboardEvent)) {
    with_doc(|d| {
        let Some(node) = d.root.and_then(|root| d.get_mut(root)) else {
            return;
        };
        let is_ours = HOOKED.with(|hooked| {
            let hooked = hooked.borrow();
            let (Some(hooked), Some(now)) = (hooked.as_ref(), &node.listeners.key_down) else {
                return false;
            };
            Rc::ptr_eq(&hooked.ours, now)
        });
        if is_ours {
            return;
        }
        let before = node.listeners.key_down.take();
        let ours: KeyDown = Rc::new({
            let before = before.clone();
            move |e: &KeyboardEvent| {
                if let Some(before) = &before {
                    before(e);
                }
                handler(e);
            }
        });
        node.listeners.key_down = Some(ours.clone());
        HOOKED.with(|hooked| *hooked.borrow_mut() = Some(Hooked { ours, before }));
    });
}

/// Gives the window's root back what it had before [`on_root_key_down`], if it has that still.
pub(super) fn unhook_root() {
    let Some(Hooked { ours, before }) = HOOKED.with(|hooked| hooked.borrow_mut().take()) else {
        return;
    };
    with_doc(|d| {
        let Some(node) = d.root.and_then(|root| d.get_mut(root)) else {
            return;
        };
        if node
            .listeners
            .key_down
            .as_ref()
            .is_some_and(|now| Rc::ptr_eq(now, &ours))
        {
            node.listeners.key_down = before;
        }
    });
}
