use std::cell::RefCell;

use anti_webui::prelude::UiSender;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEvent, NSEventModifierFlags,
    NSEventType, NSMenu, NSMenuItem,
};
use objc2_foundation::NSString;

use crate::menu::{Item, ItemState, Menu, Shortcut, Standard, key_equivalent};

define_class!(
    // SAFETY: NSObject has no requirements for subclasses, and this has no `Drop`
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HiphileNextMenuTarget"]
    #[ivars = Chooser]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(choose:))]
        fn choose(&self, item: &NSMenuItem) {
            let Ok(tag) = usize::try_from(item.tag()) else {
                return;
            };
            if !self.gives_back(item, tag) {
                self.ivars().chosen.send(tag);
            }
        }
    }
);

struct Chooser {
    chosen: UiSender<usize>,
    is_typing: fn() -> bool,
    /// Whether a text field uses the shortcut of the item at each tag.
    for_text_fields: RefCell<Vec<bool>>,
}

impl Target {
    fn new(
        mtm: MainThreadMarker,
        chosen: UiSender<usize>,
        is_typing: fn() -> bool,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Chooser {
            chosen,
            is_typing,
            for_text_fields: RefCell::default(),
        });
        // SAFETY: `init` of NSObject, on an instance just allocated
        unsafe { msg_send![super(this), init] }
    }

    /// Hands the key that chose `item`, at `tag`, to the window instead, when it is one that the
    /// text field with the focus uses. Whether it did.
    fn gives_back(&self, item: &NSMenuItem, tag: usize) -> bool {
        let chooser = self.ivars();
        let is_for_text_fields = chooser.for_text_fields.borrow().get(tag).copied();
        if is_for_text_fields != Some(true) {
            return false;
        }
        let mtm = self.mtm();
        // Chosen with the pointer, or with Return in the open menu, the current event is not the
        // item's key
        let Some(event) = NSApplication::sharedApplication(mtm)
            .currentEvent()
            .filter(|event| event.r#type() == NSEventType::KeyDown && is_key_of(event, item))
        else {
            return false;
        };
        if !(chooser.is_typing)() {
            return false;
        }
        let Some(responder) = event.window(mtm).and_then(|window| window.firstResponder()) else {
            return false;
        };
        responder.keyDown(&event);
        true
    }
}

/// Whether `event`, a key's, is the key equivalent of `item` with its modifiers.
fn is_key_of(event: &NSEvent, item: &NSMenuItem) -> bool {
    let modifiers = NSEventModifierFlags::Command
        | NSEventModifierFlags::Shift
        | NSEventModifierFlags::Option
        | NSEventModifierFlags::Control;
    // Arrow keys also come with the flags of the function keys and of the keypad
    let pressed = event.modifierFlags() & modifiers;
    pressed == item.keyEquivalentModifierMask() & modifiers
        && event
            .charactersIgnoringModifiers()
            .is_some_and(|typed| typed.to_string() == item.keyEquivalent().to_string())
}

/// The items of the commands in the menu bar, at their tags.
pub(crate) struct Native {
    items: Vec<Retained<NSMenuItem>>,
    /// Items hold their target weakly.
    _target: Retained<Target>,
}

impl Native {
    pub(crate) fn set(&self, tag: usize, state: ItemState) {
        let Some(item) = self.items.get(tag) else {
            return;
        };
        if item.isEnabled() != state.is_enabled() {
            item.setEnabled(state.is_enabled());
        }
        let checked = if state.is_checked() {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        if item.state() != checked {
            item.setState(checked);
        }
    }
}

pub(crate) fn install<C>(
    menus: &[Menu<C>],
    chosen: UiSender<usize>,
    is_typing: fn() -> bool,
) -> Option<Native> {
    let mtm = MainThreadMarker::new()?;
    let target = Target::new(mtm, chosen, is_typing);
    let mut items = Vec::new();
    let bar = NSMenu::new(mtm);
    for menu in menus {
        if let Some(submenu) = build(mtm, menu, &target, &mut items) {
            let top = NSMenuItem::new(mtm);
            top.setTitle(&NSString::from_str(&menu.title));
            top.setSubmenu(Some(&submenu));
            bar.addItem(&top);
        }
    }
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
    Some(Native {
        items,
        _target: target,
    })
}

/// Tags each item of a command with how many came before it, as `commands_of` orders them.
fn build<C>(
    mtm: MainThreadMarker,
    menu: &Menu<C>,
    target: &Target,
    items: &mut Vec<Retained<NSMenuItem>>,
) -> Option<Retained<NSMenu>> {
    if menu.items.is_empty() {
        return None;
    }
    let shown = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(&menu.title));
    // Without this AppKit enables each item by whether its target answers its action, which ours
    // always does
    shown.setAutoenablesItems(false);
    for item in &menu.items {
        match item {
            Item::Command {
                title, shortcut, ..
            } => {
                let added = item_of(mtm, title, sel!(choose:), *shortcut);
                // SAFETY: `target` is a `Target`, which answers `choose:`
                unsafe { added.setTarget(Some(target)) };
                added.setTag(items.len() as isize);
                shown.addItem(&added);
                items.push(added);
                target
                    .ivars()
                    .for_text_fields
                    .borrow_mut()
                    .push(shortcut.is_some_and(|shortcut| shortcut.is_for_text_fields()));
            }
            Item::Standard { title, action } => {
                let (action, shortcut) = standard(*action);
                // Without a target the action goes up to `NSApp`, which answers it
                shown.addItem(&item_of(mtm, title, action, shortcut));
            }
            Item::Submenu(menu) => {
                if let Some(submenu) = build(mtm, menu, target, items) {
                    let added = NSMenuItem::new(mtm);
                    added.setTitle(&NSString::from_str(&menu.title));
                    added.setSubmenu(Some(&submenu));
                    shown.addItem(&added);
                }
            }
            Item::Separator => shown.addItem(&NSMenuItem::separatorItem(mtm)),
        }
    }
    Some(shown)
}

fn item_of(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    shortcut: Option<Shortcut>,
) -> Retained<NSMenuItem> {
    let key = shortcut.map_or_else(String::new, |s| key_equivalent(s.key));
    // SAFETY: `action` is one of the selectors written out in this module
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(&key),
        )
    };
    if let Some(shortcut) = shortcut {
        item.setKeyEquivalentModifierMask(modifiers(shortcut));
    }
    item
}

fn modifiers(shortcut: Shortcut) -> NSEventModifierFlags {
    let mut flags = NSEventModifierFlags::Command;
    if shortcut.shift {
        flags |= NSEventModifierFlags::Shift;
    }
    if shortcut.option {
        flags |= NSEventModifierFlags::Option;
    }
    if shortcut.control {
        flags |= NSEventModifierFlags::Control;
    }
    flags
}

/// The actions of `NSApplication` that do what `action` says, and their shortcuts.
fn standard(action: Standard) -> (Sel, Option<Shortcut>) {
    match action {
        Standard::Hide => (sel!(hide:), Some(Shortcut::command('h'))),
        Standard::HideOthers => (
            sel!(hideOtherApplications:),
            Some(Shortcut::command('h').option()),
        ),
        Standard::ShowAll => (sel!(unhideAllApplications:), None),
        Standard::Quit => (sel!(terminate:), Some(Shortcut::command('q'))),
    }
}
