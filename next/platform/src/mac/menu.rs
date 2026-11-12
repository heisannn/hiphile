use anti_webui::prelude::UiSender;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu,
    NSMenuItem,
};
use objc2_foundation::NSString;

use crate::menu::{Item, ItemState, Menu, Shortcut, Standard, key_equivalent};

define_class!(
    // SAFETY: NSObject has no requirements for subclasses, and this has no `Drop`
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HiphileNextMenuTarget"]
    #[ivars = UiSender<usize>]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(choose:))]
        fn choose(&self, item: &NSMenuItem) {
            if let Ok(tag) = usize::try_from(item.tag()) {
                self.ivars().send(tag);
            }
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, chosen: UiSender<usize>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(chosen);
        // SAFETY: `init` of NSObject, on an instance just allocated
        unsafe { msg_send![super(this), init] }
    }
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

pub(crate) fn install<C>(menus: &[Menu<C>], chosen: UiSender<usize>) -> Option<Native> {
    let mtm = MainThreadMarker::new()?;
    let target = Target::new(mtm, chosen);
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
