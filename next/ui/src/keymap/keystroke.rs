use std::fmt;

use anti_webui::prelude::{Key, Modifiers, NamedKey};

/// A key and the modifiers held with it. A character is kept in lower case, so a stroke made from
/// what was pressed equals the one a binding names.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Keystroke {
    key: Key,
    modifiers: Modifiers,
}

impl Keystroke {
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        let key = match key {
            Key::Character(c) => Key::Character(c.to_lowercase()),
            key => key,
        };
        Self { key, modifiers }
    }

    pub fn named(key: NamedKey) -> Self {
        Self::new(Key::Named(key), Modifiers::default())
    }

    pub fn character(c: char) -> Self {
        Self::new(Key::Character(c.to_string()), Modifiers::default())
    }

    /// `c` with the modifier of shortcuts, ⌘ on macOS.
    pub fn primary(c: char) -> Self {
        Self::new(Key::Character(c.to_string()), Modifiers::PRIMARY)
    }

    pub fn with_shift(mut self) -> Self {
        self.modifiers.shift = true;
        self
    }

    pub fn with_alt(mut self) -> Self {
        self.modifiers.alt = true;
        self
    }

    pub fn with_ctrl(mut self) -> Self {
        self.modifiers.ctrl = true;
        self
    }

    pub fn with_primary(mut self) -> Self {
        if cfg!(target_os = "macos") {
            self.modifiers.logo = true;
        } else {
            self.modifiers.ctrl = true;
        }
        self
    }

    pub fn key(&self) -> &Key {
        &self.key
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Whether a focused text field uses this stroke itself, for typing, moving the caret,
    /// deleting, submitting or the clipboard, which comes before the keymap there.
    pub fn is_for_text_fields(&self) -> bool {
        let m = self.modifiers;
        match &self.key {
            Key::Character(c) => {
                m.types_text() || (m.primary() && matches!(c.as_str(), "a" | "c" | "x" | "v" | "z"))
            }
            Key::Named(NamedKey::Space | NamedKey::Enter) => m.types_text(),
            Key::Named(
                NamedKey::Backspace
                | NamedKey::Delete
                | NamedKey::Insert
                | NamedKey::ArrowLeft
                | NamedKey::ArrowRight
                | NamedKey::ArrowUp
                | NamedKey::ArrowDown
                | NamedKey::Home
                | NamedKey::End,
            ) => true,
            _ => false,
        }
    }
}

/// As menus show it: `⌥⇧↩` on macOS, `Ctrl+Shift+Enter` elsewhere.
impl fmt::Display for Keystroke {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        let key = key_name(&self.key);
        if cfg!(target_os = "macos") {
            for (held, glyph) in [(m.ctrl, "⌃"), (m.alt, "⌥"), (m.shift, "⇧"), (m.logo, "⌘")]
            {
                if held {
                    f.write_str(glyph)?;
                }
            }
            f.write_str(&key)
        } else {
            for (held, name) in [
                (m.ctrl, "Ctrl"),
                (m.alt, "Alt"),
                (m.shift, "Shift"),
                (m.logo, "Super"),
            ] {
                if held {
                    write!(f, "{name}+")?;
                }
            }
            f.write_str(&key)
        }
    }
}

fn key_name(key: &Key) -> String {
    let mac = cfg!(target_os = "macos");
    let named = |glyph: &'static str, word: &'static str| if mac { glyph } else { word };
    match key {
        Key::Character(c) => c.to_uppercase(),
        Key::Named(key) => match key {
            NamedKey::Enter => named("↩", "Enter"),
            NamedKey::Tab => named("⇥", "Tab"),
            NamedKey::Space => "Space",
            NamedKey::Backspace => named("⌫", "Backspace"),
            NamedKey::Delete => named("⌦", "Delete"),
            NamedKey::Escape => named("⎋", "Esc"),
            NamedKey::ArrowLeft => "←",
            NamedKey::ArrowRight => "→",
            NamedKey::ArrowUp => "↑",
            NamedKey::ArrowDown => "↓",
            NamedKey::Home => named("↖", "Home"),
            NamedKey::End => named("↘", "End"),
            NamedKey::PageUp => named("⇞", "Page Up"),
            NamedKey::PageDown => named("⇟", "Page Down"),
            NamedKey::Insert => "Insert",
            NamedKey::Shift => named("⇧", "Shift"),
            NamedKey::Control => named("⌃", "Ctrl"),
            NamedKey::Alt => named("⌥", "Alt"),
            NamedKey::Super => named("⌘", "Super"),
            NamedKey::CapsLock => named("⇪", "Caps Lock"),
            NamedKey::F(n) => return format!("F{n}"),
        }
        .to_owned(),
        Key::Unidentified => String::new(),
    }
}
