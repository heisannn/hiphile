//! Keys for commands. A binding names a stroke, a command and the context it works in. An element
//! made a context by [`key_context`] runs the bindings of that context whose commands are enabled
//! there. A key goes from what has the focus outwards, so the innermost context that takes it wins.
//!
//! The keymap is the thread's, one for each type of command, and a window has the thread to
//! itself.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use anti_webui::core::tree::NodeId;
use anti_webui::prelude::{
    Div, Key, KeyboardEvent, NamedKey, NodeRef, node_ref, on_cleanup, untrack,
};

mod focus;
mod keystroke;

pub use keystroke::Keystroke;

/// `keystroke` runs `command` where the focus is in an element of `context`.
#[derive(Clone, Debug)]
pub struct KeyBinding<C> {
    keystroke: Keystroke,
    command: C,
    context: &'static str,
}

impl<C> KeyBinding<C> {
    pub fn new(keystroke: Keystroke, command: C, context: &'static str) -> Self {
        Self {
            keystroke,
            command,
            context,
        }
    }

    pub fn keystroke(&self) -> &Keystroke {
        &self.keystroke
    }

    pub fn command(&self) -> &C {
        &self.command
    }

    pub fn context(&self) -> &'static str {
        self.context
    }
}

type Bindings<C> = RefCell<Vec<KeyBinding<C>>>;
type Take = Rc<dyn Fn(&Keystroke) -> bool>;
type Fallback = Rc<dyn Fn() -> Option<NodeRef>>;

thread_local! {
    static KEYMAPS: RefCell<HashMap<TypeId, Rc<dyn Any>>> = RefCell::default();
    static HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
    static NEXT_HANDLER: Cell<u64> = const { Cell::new(0) };
    /// The handler whose element had the focus last.
    static LAST_FOCUSED: Cell<Option<u64>> = const { Cell::new(None) };
    static KEEPER: RefCell<Option<Fallback>> = RefCell::default();
}

/// Adds to this thread's keymap of `C`. Of the bindings of one stroke in one context, the last
/// one whose command is enabled wins. A menu bar reads the bindings when it is built, so bind
/// before.
pub fn bind_keys<C: 'static>(bindings: impl IntoIterator<Item = KeyBinding<C>>) {
    keymap::<C>().borrow_mut().extend(bindings);
}

/// The bindings of `context`, in the order they were bound.
pub fn bindings_in<C: Clone + 'static>(context: &str) -> Vec<KeyBinding<C>> {
    keymap::<C>()
        .borrow()
        .iter()
        .filter(|binding| binding.context == context)
        .cloned()
        .collect()
}

/// What runs `command` in `context`, as a menu or a tooltip shows it.
pub fn keystroke_for<C: PartialEq + 'static>(command: &C, context: &str) -> Option<Keystroke> {
    keymap::<C>()
        .borrow()
        .iter()
        .rev()
        .find(|binding| binding.context == context && binding.command == *command)
        .map(|binding| binding.keystroke.clone())
}

fn keymap<C: 'static>() -> Rc<Bindings<C>> {
    let any = KEYMAPS.with(|maps| {
        maps.borrow_mut()
            .entry(TypeId::of::<C>())
            .or_insert_with(|| Rc::new(Bindings::<C>::default()))
            .clone()
    });
    any.downcast().expect("kept under its own type")
}

/// A context: its name in bindings, which commands are enabled in it, and how it runs them. Cheap
/// to clone.
pub struct KeyContext<C> {
    name: &'static str,
    enabled: Rc<dyn Fn(&C) -> bool>,
    run: Rc<dyn Fn(C)>,
}

impl<C> Clone for KeyContext<C> {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            enabled: self.enabled.clone(),
            run: self.run.clone(),
        }
    }
}

impl<C: Clone + 'static> KeyContext<C> {
    /// `enabled` may read signals, which then make what reads [`is_enabled`](Self::is_enabled)
    /// run again, such as a menu. `run` is only given enabled commands.
    pub fn new(
        name: &'static str,
        enabled: impl Fn(&C) -> bool + 'static,
        run: impl Fn(C) + 'static,
    ) -> Self {
        Self {
            name,
            enabled: Rc::new(enabled),
            run: Rc::new(run),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn is_enabled(&self, command: &C) -> bool {
        (self.enabled)(command)
    }

    /// Runs `command` if it is enabled, as a menu item does. Whether it ran.
    pub fn run(&self, command: C) -> bool {
        if !untrack(|| self.is_enabled(&command)) {
            return false;
        }
        (self.run)(command);
        true
    }

    fn take(&self, stroke: &Keystroke) -> bool {
        if stroke.is_for_text_fields() && focus::is_in_text_field() {
            return false;
        }
        let command = keymap::<C>()
            .borrow()
            .iter()
            .rev()
            .filter(|binding| binding.context == self.name && binding.keystroke == *stroke)
            .map(|binding| binding.command.clone())
            .find(|command| untrack(|| self.is_enabled(command)));
        let Some(command) = command else {
            return false;
        };
        (self.run)(command);
        true
    }
}

/// Whether a text field has the focus, which keeps the keys it uses for itself.
pub fn is_typing() -> bool {
    focus::is_in_text_field()
}

/// Makes `element` the context `context`: while the focus is in it, the bindings of the context
/// run its enabled commands.
pub fn key_context<C: Clone + 'static>(element: Div, context: &KeyContext<C>) -> Div {
    let context = context.clone();
    handles_keys(element, move |stroke| context.take(stroke))
}

struct Handler {
    id: u64,
    node: NodeRef,
    take: Take,
}

/// Gives `element` the keys pressed while the focus is in it, before what it is in. `take` says
/// whether it used a key, which then goes no further and does nothing else, such as clicking what
/// has the focus with Enter or Space. Such an element that has the focus is where
/// [`keeps_focus`] gives it back to.
pub fn handles_keys(element: Div, take: impl Fn(&Keystroke) -> bool + 'static) -> Div {
    let node = node_ref();
    let take: Take = Rc::new(take);
    let id = NEXT_HANDLER.with(|next| next.replace(next.get() + 1));
    HANDLERS.with(|handlers| {
        handlers.borrow_mut().push(Handler {
            id,
            node,
            take: take.clone(),
        })
    });
    on_cleanup(move || {
        HANDLERS.with(|handlers| handlers.borrow_mut().retain(|handler| handler.id != id));
        LAST_FOCUSED.with(|last| {
            if last.get() == Some(id) {
                last.set(None);
            }
        });
    });
    element
        .node_ref(node)
        .on_focus_change(move |on| {
            if on {
                LAST_FOCUSED.with(|last| last.set(Some(id)));
            }
        })
        .on_key_down(move |e| {
            if e.is_default_prevented() {
                return;
            }
            if take(&Keystroke::new(e.key.clone(), e.modifiers)) {
                e.stop_propagation();
                e.prevent_default();
            }
        })
}

/// Makes `root`, the window's outermost element, give the focus back when nothing has it: to the
/// element of [`handles_keys`] that had it last, while that is there, and else to `fallback`. A
/// key pressed meanwhile goes there, as it would have had the focus never gone, and so does a
/// press on what takes no focus. Without it, a key pressed while nothing has the focus does
/// nothing.
pub fn keeps_focus(root: Div, fallback: impl Fn() -> Option<NodeRef> + 'static) -> Div {
    KEEPER.with(|keeper| *keeper.borrow_mut() = Some(Rc::new(fallback)));
    on_cleanup(|| drop(KEEPER.with(|keeper| keeper.borrow_mut().take())));
    // The window's own root holds `root`, and is all that a key reaches while nothing has the
    // focus. It is there once the window is laid out.
    root.on_resized(|_, _| focus::on_root_key_down(give_back_key))
        .on_pointer_down(|_| {
            if focus::focused().is_none()
                && !focus::is_modal_open()
                && let Some((home, _)) = home()
            {
                home.focus();
            }
        })
}

/// Sends a key pressed while nothing has the focus to where the focus goes back.
fn give_back_key(e: &KeyboardEvent) {
    if e.is_default_prevented() || focus::focused().is_some() || focus::is_modal_open() {
        return;
    }
    let Some((home, node)) = home() else {
        return;
    };
    let stroke = Keystroke::new(e.key.clone(), e.modifiers);
    // Tab finds its own way
    if stroke.key() != &Key::Named(NamedKey::Tab) {
        home.focus();
    }
    if take_from(node, &stroke) {
        e.stop_propagation();
        e.prevent_default();
    }
}

fn home() -> Option<(NodeRef, NodeId)> {
    let last = LAST_FOCUSED.with(Cell::get).and_then(|last| {
        HANDLERS.with(|handlers| {
            handlers
                .borrow()
                .iter()
                .find(|handler| handler.id == last)
                .map(|handler| handler.node)
        })
    });
    let fallback = || {
        let keeper = KEEPER.with(|keeper| keeper.borrow().clone())?;
        untrack(|| keeper())
    };
    [last, fallback()]
        .into_iter()
        .flatten()
        .find_map(|home| Some((home, focus::node_of(home)?)))
}

/// Gives `stroke` to the handlers from `node` outwards, until one takes it.
fn take_from(node: NodeId, stroke: &Keystroke) -> bool {
    let chain = focus::chain_from(node);
    let takers: Vec<(NodeId, Take)> = HANDLERS.with(|handlers| {
        handlers
            .borrow()
            .iter()
            .filter_map(|handler| Some((focus::node_of(handler.node)?, handler.take.clone())))
            .collect()
    });
    chain.into_iter().any(|at| {
        takers
            .iter()
            .filter(|(node, _)| *node == at)
            .any(|(_, take)| take(stroke))
    })
}

#[cfg(test)]
mod tests;
