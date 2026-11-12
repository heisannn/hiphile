//! The menu bar, in place of the one AntiWebUI puts there. AppKit takes the shortcuts of its items
//! before the window sees a key, except those of disabled items, and those that a focused text
//! field moves its caret with, which go back to the window.

use std::rc::Rc;

use anti_webui::prelude::ui_channel;

/// A menu of the menu bar. macOS titles the first with the app's name, whatever it says.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Menu<C> {
    pub title: String,
    pub items: Vec<Item<C>>,
}

impl<C> Menu<C> {
    pub fn new(title: impl Into<String>, items: Vec<Item<C>>) -> Self {
        Self {
            title: title.into(),
            items,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Item<C> {
    /// Sends `command` when chosen, or when `shortcut` is pressed while it is enabled.
    Command {
        title: String,
        command: C,
        shortcut: Option<Shortcut>,
    },
    /// What AppKit does itself, with its usual shortcut.
    Standard {
        title: String,
        action: Standard,
    },
    Submenu(Menu<C>),
    Separator,
}

impl<C> Item<C> {
    pub fn command(
        title: impl Into<String>,
        command: C,
        shortcut: impl Into<Option<Shortcut>>,
    ) -> Self {
        Self::Command {
            title: title.into(),
            command,
            shortcut: shortcut.into(),
        }
    }

    pub fn standard(title: impl Into<String>, action: Standard) -> Self {
        Self::Standard {
            title: title.into(),
            action,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Standard {
    /// ⌘H.
    Hide,
    /// ⌥⌘H.
    HideOthers,
    ShowAll,
    /// ⌘Q, which AntiWebUI turns into a request to close the first window, as it does the Dock's
    /// Quit. The app ends when that window closes.
    Quit,
}

/// A key pressed with ⌘, and maybe ⇧, ⌥ or ⌃ too. Without ⌘ the menu bar would take keys that
/// type, before a text field could.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub(crate) key: Key,
    pub(crate) shift: bool,
    pub(crate) option: bool,
    pub(crate) control: bool,
}

impl Shortcut {
    pub fn command(key: impl Into<Key>) -> Self {
        Self {
            key: key.into(),
            shift: false,
            option: false,
            control: false,
        }
    }

    pub fn shift(self) -> Self {
        Self {
            shift: true,
            ..self
        }
    }

    pub fn option(self) -> Self {
        Self {
            option: true,
            ..self
        }
    }

    pub fn control(self) -> Self {
        Self {
            control: true,
            ..self
        }
    }

    pub fn key(&self) -> Key {
        self.key
    }

    pub fn has_shift(&self) -> bool {
        self.shift
    }

    pub fn has_option(&self) -> bool {
        self.option
    }

    pub fn has_control(&self) -> bool {
        self.control
    }

    /// Whether a text field uses it, to move its caret or its selection to the start or the end of
    /// a line or of the text.
    pub(crate) fn is_for_text_fields(&self) -> bool {
        matches!(self.key, Key::Left | Key::Right | Key::Up | Key::Down)
            && !self.option
            && !self.control
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Key {
    /// What the key types without modifiers, such as `'['` or `'1'`.
    Char(char),
    Left,
    Right,
    Up,
    Down,
}

impl From<char> for Key {
    fn from(key: char) -> Self {
        Self::Char(key)
    }
}

/// Whether an item can be chosen, and whether it has a check mark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemState {
    enabled: bool,
    checked: bool,
}

impl ItemState {
    pub fn enabled(enabled: bool) -> Self {
        Self {
            enabled,
            checked: false,
        }
    }

    pub fn checked(self, checked: bool) -> Self {
        Self { checked, ..self }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_checked(&self) -> bool {
        self.checked
    }
}

/// The menu bar put in place. Cheap to clone.
pub struct MainMenu<C> {
    /// Each item's command, at the item's tag.
    commands: Rc<[C]>,
    #[cfg(target_os = "macos")]
    native: Option<Rc<crate::mac::menu::Native>>,
}

impl<C> Clone for MainMenu<C> {
    fn clone(&self) -> Self {
        Self {
            commands: self.commands.clone(),
            #[cfg(target_os = "macos")]
            native: self.native.clone(),
        }
    }
}

impl<C: Clone + 'static> MainMenu<C> {
    /// Puts `menus` in the menu bar in place of AntiWebUI's, which AntiWebUI puts there once as it
    /// starts and never again. What is chosen comes to `on_command` later, on the UI thread, rather
    /// than inside AppKit's callback. A menu with no items is left out.
    ///
    /// While `is_typing` says a text field has the focus, the shortcuts it uses go to the window
    /// in place of the items they are of. It is asked inside AppKit's callback.
    ///
    /// Call inside `App::run`, in an owner: before AntiWebUI starts, its own menu would replace
    /// these, and without an owner what is chosen goes nowhere. Off the main thread nothing changes.
    pub fn install(
        menus: &[Menu<C>],
        on_command: impl Fn(C) + 'static,
        is_typing: fn() -> bool,
    ) -> Self {
        let commands: Rc<[C]> = commands_of(menus).into();
        let chosen = {
            let commands = commands.clone();
            ui_channel(move |tag: usize| {
                if let Some(command) = commands.get(tag) {
                    on_command(command.clone());
                }
            })
        };
        #[cfg(not(target_os = "macos"))]
        let _ = (chosen, is_typing);
        Self {
            #[cfg(target_os = "macos")]
            native: crate::mac::menu::install(menus, chosen, is_typing).map(Rc::new),
            commands,
        }
    }

    /// Enables and checks each item of a command as `state_of` says. In an `effect`, the items
    /// follow what `state_of` reads.
    pub fn update(&self, state_of: impl Fn(&C) -> ItemState) {
        for (tag, command) in self.commands.iter().enumerate() {
            let state = state_of(command);
            #[cfg(target_os = "macos")]
            if let Some(native) = &self.native {
                native.set(tag, state);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (tag, state);
        }
    }
}

/// The commands of `menus`' items in the order they come, which is the order of their tags: the
/// AppKit side walks them in the same order.
pub(crate) fn commands_of<C: Clone>(menus: &[Menu<C>]) -> Vec<C> {
    fn walk<C: Clone>(items: &[Item<C>], commands: &mut Vec<C>) {
        for item in items {
            match item {
                Item::Command { command, .. } => commands.push(command.clone()),
                Item::Submenu(menu) => walk(&menu.items, commands),
                Item::Standard { .. } | Item::Separator => {}
            }
        }
    }
    let mut commands = Vec::new();
    for menu in menus {
        walk(&menu.items, &mut commands);
    }
    commands
}

/// What AppKit calls `key` in a key equivalent: the character, or the code of the function key.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn key_equivalent(key: Key) -> String {
    match key {
        Key::Char(key) => key.to_string(),
        Key::Up => '\u{F700}'.to_string(),
        Key::Down => '\u{F701}'.to_string(),
        Key::Left => '\u{F702}'.to_string(),
        Key::Right => '\u{F703}'.to_string(),
    }
}

#[cfg(test)]
mod tests;
