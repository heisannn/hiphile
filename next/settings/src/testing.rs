//! What the tests of the settings share.

use anti_webui::core::Ui;
use anti_webui::core::event::{Key, Modifiers, NamedKey};
use anti_webui::prelude::Role;
use next_ui::testing::{find, focus, key, key_with, typed};

/// Puts `text` in place of what the field with `role` and `name` holds, as typing it over all of
/// it does.
pub(crate) fn replace(ui: &mut Ui, role: Role, name: &str, text: &str) {
    focus(ui, role, name);
    let primary = Modifiers {
        logo: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        ..Modifiers::default()
    };
    key_with(ui, Key::Character("a".to_owned()), primary);
    if text.is_empty() {
        key(ui, NamedKey::Backspace);
    } else {
        typed(ui, text);
    }
}

pub(crate) fn value(role: Role, name: &str) -> String {
    find(role, name)
        .unwrap_or_else(|| panic!("no {role:?} named {name:?}"))
        .value_text
        .unwrap_or_default()
}
