use anti_webui::core::Ui;
use anti_webui::prelude::*;

use super::*;
use crate::testing::{click, focus, key, key_with, start, typed};

#[derive(Clone, Debug, PartialEq)]
enum Test {
    Find,
    Open,
    Close,
}

const OUTER: &str = "Outer";
const INNER: &str = "Inner";

/// What ran, and in which context.
type Ran = Signal<Vec<(&'static str, Test)>>;

struct Window {
    ran: Ran,
    inner_enabled: Signal<bool>,
    clicked: Signal<u32>,
    filter: Signal<String>,
    shows_inner: Signal<bool>,
}

impl Window {
    fn new() -> Self {
        Self {
            ran: signal(Vec::new()),
            inner_enabled: signal(true),
            clicked: signal(0),
            filter: signal(String::new()),
            shows_inner: signal(true),
        }
    }

    fn context(&self, name: &'static str, enabled: Option<Signal<bool>>) -> KeyContext<Test> {
        let ran = self.ran;
        KeyContext::new(
            name,
            move |_| enabled.is_none_or(|enabled| enabled.get()),
            move |command| ran.update(|ran| ran.push((name, command))),
        )
    }

    /// The outer context holds a button and the inner one, which holds a field and a button.
    fn view(&self, keeps: bool) -> Div {
        let (clicked, filter, shows_inner) = (self.clicked, self.filter, self.shows_inner);
        let inner = self.context(INNER, Some(self.inner_enabled));
        let inside = move || {
            key_context(
                div()
                    .focusable()
                    .w(400)
                    .h(200)
                    .access(|a| a.role(Role::Group).label("Inner"))
                    .child(
                        text_input()
                            .value(filter)
                            .access(|a| a.label("Filter"))
                            .w(200),
                    )
                    .child(
                        div()
                            .focusable()
                            .w(80)
                            .h(30)
                            .access(|a| a.role(Role::Button).label("Inner Button"))
                            .on_click(move |_| clicked.update(|n| *n += 1)),
                    ),
                &inner,
            )
        };
        let root = key_context(
            div()
                .size_full()
                .child(
                    div()
                        .focusable()
                        .w(80)
                        .h(30)
                        .access(|a| a.role(Role::Button).label("Outer Button")),
                )
                .child(show(move || shows_inner.get(), inside))
                .child(div().w(400).h(200).child(text("Nothing to focus"))),
            &self.context(OUTER, None),
        );
        if keeps {
            keeps_focus(root, || None)
        } else {
            root
        }
    }

    fn ran(&self) -> Vec<(&'static str, Test)> {
        self.ran.get_untracked()
    }
}

fn shown(window: &Window, keeps: bool) -> Ui {
    let view = window.view(keeps);
    start(move || view)
}

fn enter(ui: &mut Ui) {
    key(ui, NamedKey::Enter);
}

fn focused_label() -> Option<String> {
    let tree = anti_webui::core::access::snapshot();
    tree.get(tree.focus?)?.label.clone()
}

#[test]
fn a_binding_runs_its_command_where_the_focus_is_in_its_context() {
    bind_keys([KeyBinding::new(Keystroke::primary('k'), Test::Find, OUTER)]);
    let window = Window::new();
    let mut ui = shown(&window, false);

    focus(&mut ui, Role::Group, "Inner");
    key_with(&mut ui, Key::Character("k".into()), Modifiers::PRIMARY);

    assert_eq!(window.ran(), [(OUTER, Test::Find)]);
}

#[test]
fn the_innermost_context_that_binds_a_key_wins() {
    bind_keys([
        KeyBinding::new(Keystroke::named(NamedKey::Enter), Test::Open, OUTER),
        KeyBinding::new(Keystroke::named(NamedKey::Enter), Test::Open, INNER),
    ]);
    let window = Window::new();
    let mut ui = shown(&window, false);

    focus(&mut ui, Role::Group, "Inner");
    enter(&mut ui);
    focus(&mut ui, Role::Button, "Outer Button");
    enter(&mut ui);

    assert_eq!(window.ran(), [(INNER, Test::Open), (OUTER, Test::Open)]);
}

#[test]
fn a_disabled_command_leaves_its_key_to_the_context_around() {
    bind_keys([
        KeyBinding::new(Keystroke::named(NamedKey::Enter), Test::Close, OUTER),
        KeyBinding::new(Keystroke::named(NamedKey::Enter), Test::Open, INNER),
    ]);
    let window = Window::new();
    window.inner_enabled.set(false);
    let mut ui = shown(&window, false);

    focus(&mut ui, Role::Group, "Inner");
    enter(&mut ui);

    assert_eq!(window.ran(), [(OUTER, Test::Close)]);
}

#[test]
fn a_key_that_runs_a_command_does_nothing_else() {
    bind_keys([KeyBinding::new(
        Keystroke::named(NamedKey::Enter),
        Test::Open,
        INNER,
    )]);
    let window = Window::new();
    let mut ui = shown(&window, false);

    focus(&mut ui, Role::Button, "Inner Button");
    enter(&mut ui);
    assert_eq!(window.clicked.get_untracked(), 0);

    key(&mut ui, NamedKey::Space);
    assert_eq!(window.clicked.get_untracked(), 1);
    assert_eq!(window.ran(), [(INNER, Test::Open)]);
}

#[test]
fn a_text_field_keeps_the_keys_it_types_and_edits_with() {
    bind_keys([
        KeyBinding::new(Keystroke::character('f'), Test::Find, INNER),
        KeyBinding::new(Keystroke::named(NamedKey::Backspace), Test::Close, INNER),
        KeyBinding::new(Keystroke::primary('f'), Test::Find, INNER),
        KeyBinding::new(Keystroke::named(NamedKey::Escape), Test::Close, INNER),
    ]);
    let window = Window::new();
    let mut ui = shown(&window, false);

    focus(&mut ui, Role::TextInput, "Filter");
    typed(&mut ui, "ff");
    key(&mut ui, NamedKey::Backspace);
    assert_eq!(window.filter.get_untracked(), "f");
    assert!(window.ran().is_empty());

    key_with(&mut ui, Key::Character("f".into()), Modifiers::PRIMARY);
    key(&mut ui, NamedKey::Escape);
    assert_eq!(window.ran(), [(INNER, Test::Find), (INNER, Test::Close)]);
}

#[test]
fn a_key_with_nothing_focused_goes_where_the_focus_last_was() {
    bind_keys([KeyBinding::new(
        Keystroke::named(NamedKey::Enter),
        Test::Open,
        INNER,
    )]);
    let window = Window::new();
    let mut ui = shown(&window, true);
    focus(&mut ui, Role::Group, "Inner");
    focus(&mut ui, Role::Button, "Inner Button");
    blur(&mut ui);
    assert_eq!(focused_label(), None);

    enter(&mut ui);

    assert_eq!(window.ran(), [(INNER, Test::Open)]);
    assert_eq!(focused_label().as_deref(), Some("Inner"));
}

#[test]
fn a_key_with_nothing_focused_goes_to_the_fallback_once_the_last_focus_is_gone() {
    bind_keys([
        KeyBinding::new(Keystroke::named(NamedKey::Enter), Test::Open, OUTER),
        KeyBinding::new(Keystroke::named(NamedKey::Enter), Test::Open, INNER),
    ]);
    let window = Window::new();
    let home = node_ref();
    let view = keeps_focus(
        key_context(
            div()
                .size_full()
                .child(
                    div()
                        .focusable()
                        .node_ref(home)
                        .w(80)
                        .h(30)
                        .access(|a| a.role(Role::Button).label("Home")),
                )
                .child(show(move || window.shows_inner.get(), {
                    let inner = window.context(INNER, None);
                    move || {
                        key_context(
                            div()
                                .focusable()
                                .w(80)
                                .h(30)
                                .access(|a| a.role(Role::Group).label("Inner")),
                            &inner,
                        )
                    }
                })),
            &window.context(OUTER, None),
        ),
        move || Some(home),
    );
    let mut ui = start(move || view);
    focus(&mut ui, Role::Group, "Inner");

    window.shows_inner.set(false);
    ui.update();
    assert_eq!(focused_label(), None);
    enter(&mut ui);

    assert_eq!(window.ran(), [(OUTER, Test::Open)]);
    assert_eq!(focused_label().as_deref(), Some("Home"));
}

#[test]
fn a_press_on_what_takes_no_focus_gives_it_back() {
    let window = Window::new();
    let mut ui = shown(&window, true);
    focus(&mut ui, Role::Group, "Inner");

    let (x, y) = crate::testing::middle_of("Nothing to focus");
    click(&mut ui, x, y);
    ui.update();

    assert_eq!(focused_label().as_deref(), Some("Inner"));
}

#[test]
fn a_menu_follows_whether_a_command_is_enabled() {
    let window = Window::new();
    let context = window.context(INNER, Some(window.inner_enabled));
    let shown = signal(Vec::new());
    let _ui = {
        let context = context.clone();
        start(move || {
            effect(move || {
                let enabled = context.is_enabled(&Test::Open);
                shown.update(|shown| shown.push(enabled));
            });
            div()
        })
    };

    window.inner_enabled.set(false);
    assert!(!context.run(Test::Open));
    window.inner_enabled.set(true);
    assert!(context.run(Test::Open));

    assert_eq!(shown.get_untracked(), [true, false, true]);
    assert_eq!(window.ran(), [(INNER, Test::Open)]);
}

#[test]
fn of_the_bindings_of_a_stroke_the_last_one_bound_wins() {
    bind_keys([
        KeyBinding::new(Keystroke::primary('o'), Test::Open, OUTER),
        KeyBinding::new(Keystroke::primary('o'), Test::Find, OUTER),
        KeyBinding::new(Keystroke::primary('w'), Test::Close, INNER),
    ]);
    let window = Window::new();
    let mut ui = shown(&window, false);

    focus(&mut ui, Role::Button, "Outer Button");
    key_with(&mut ui, Key::Character("o".into()), Modifiers::PRIMARY);

    assert_eq!(window.ran(), [(OUTER, Test::Find)]);
    assert_eq!(
        keystroke_for(&Test::Find, OUTER),
        Some(Keystroke::primary('o'))
    );
    assert_eq!(keystroke_for(&Test::Close, OUTER), None);
    let outer: Vec<Test> = bindings_in::<Test>(OUTER)
        .iter()
        .map(|binding| binding.command().clone())
        .collect();
    assert_eq!(outer, [Test::Open, Test::Find]);
}

#[test]
fn a_stroke_pressed_is_the_one_bound() {
    let pressed = Keystroke::new(Key::Character("L".into()), Modifiers::PRIMARY.with_shift());
    assert_eq!(pressed, Keystroke::primary('l').with_shift());
}

#[cfg(target_os = "macos")]
#[test]
fn a_stroke_shows_as_menus_show_it() {
    assert_eq!(Keystroke::primary('l').with_shift().to_string(), "⇧⌘L");
    assert_eq!(
        Keystroke::named(NamedKey::Enter).with_alt().to_string(),
        "⌥↩"
    );
    assert_eq!(
        Keystroke::named(NamedKey::ArrowUp)
            .with_ctrl()
            .with_primary()
            .to_string(),
        "⌃⌘↑"
    );
    assert_eq!(Keystroke::named(NamedKey::F(5)).to_string(), "F5");
}

#[test]
fn a_text_field_takes_typing_editing_and_the_clipboard() {
    for stroke in [
        Keystroke::character('a'),
        Keystroke::character('a').with_shift(),
        Keystroke::named(NamedKey::Space),
        Keystroke::named(NamedKey::Enter),
        Keystroke::named(NamedKey::Backspace).with_alt(),
        Keystroke::named(NamedKey::ArrowLeft).with_primary(),
        Keystroke::named(NamedKey::Home),
        Keystroke::primary('v'),
        Keystroke::primary('z').with_shift(),
    ] {
        assert!(stroke.is_for_text_fields(), "{stroke:?}");
    }
    for stroke in [
        Keystroke::primary('f'),
        Keystroke::primary('l').with_shift(),
        Keystroke::named(NamedKey::Enter).with_primary(),
        Keystroke::named(NamedKey::Escape),
        Keystroke::named(NamedKey::Tab),
        Keystroke::named(NamedKey::PageDown),
        Keystroke::named(NamedKey::F(5)),
    ] {
        assert!(!stroke.is_for_text_fields(), "{stroke:?}");
    }
}

/// Takes the focus from whatever has it.
fn blur(ui: &mut Ui) {
    anti_webui::core::tree::with_doc(|d| d.pending_focus = Some(None));
    ui.update();
}
