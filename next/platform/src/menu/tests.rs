use super::*;

#[test]
fn commands_are_tagged_in_the_order_they_come_through_submenus() {
    let menus = vec![
        Menu::new(
            "app",
            vec![
                Item::command("Settings…", "settings", Shortcut::command(',')),
                Item::Separator,
                Item::standard("Quit", Standard::Quit),
            ],
        ),
        Menu::new("Library", Vec::new()),
        Menu::new(
            "Controls",
            vec![
                Item::command("Shuffle", "shuffle", None),
                Item::Submenu(Menu::new(
                    "Repeat",
                    vec![
                        Item::command("Off", "repeat off", None),
                        Item::command("All", "repeat all", None),
                    ],
                )),
                Item::command("Mute", "mute", None),
            ],
        ),
    ];

    assert_eq!(
        commands_of(&menus),
        ["settings", "shuffle", "repeat off", "repeat all", "mute"]
    );
}

#[test]
fn a_function_key_is_named_by_its_code() {
    assert_eq!(key_equivalent(Key::Char('[')), "[");
    assert_eq!(key_equivalent(Key::Up), "\u{F700}");
    assert_eq!(key_equivalent(Key::Down), "\u{F701}");
    assert_eq!(key_equivalent(Key::Left), "\u{F702}");
    assert_eq!(key_equivalent(Key::Right), "\u{F703}");
}

#[test]
fn a_shortcut_adds_modifiers_to_command() {
    let shortcut = Shortcut::command('p').shift().option();

    assert_eq!(shortcut.key, Key::Char('p'));
    assert!(shortcut.shift && shortcut.option && !shortcut.control);
}

#[test]
fn an_item_is_unchecked_unless_said() {
    assert!(!ItemState::enabled(true).is_checked());
    assert!(ItemState::enabled(false).checked(true).is_checked());
    assert!(!ItemState::default().is_enabled());
}

#[test]
fn only_the_arrows_a_text_field_moves_its_caret_with_go_back_to_it() {
    for key in [Key::Left, Key::Right, Key::Up, Key::Down] {
        assert!(Shortcut::command(key).is_for_text_fields());
        assert!(Shortcut::command(key).shift().is_for_text_fields());
        assert!(!Shortcut::command(key).option().is_for_text_fields());
        assert!(!Shortcut::command(key).control().is_for_text_fields());
    }
    assert!(!Shortcut::command('n').is_for_text_fields());
    assert!(!Shortcut::command('[').is_for_text_fields());
}
