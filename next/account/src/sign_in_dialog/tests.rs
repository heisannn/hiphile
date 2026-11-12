use anti_webui::core::Ui;
use anti_webui::core::access::snapshot;
use anti_webui::core::event::NamedKey;
use next_ui::testing::{NOWHERE, core, find, focus, frames, has_text, key, nodes, press, start};

use super::*;

fn value_of(field: &str) -> Option<String> {
    find(Role::TextInput, field).and_then(|node| node.value_text)
}

/// The dialog on `request`, as the window draws it the second time, once what takes the focus as
/// it opens has.
fn open(hiphile: Hiphile, request: Signal<Option<SignInRequest>>) -> Ui {
    let mut ui = start(move || div().size_full().child(sign_in_dialog(hiphile, request)));
    ui.update();
    ui
}

fn focused_field() -> Option<String> {
    let tree = snapshot();
    let node = tree.get(tree.focus?)?;
    (node.role == Role::TextInput).then(|| node.label.clone())?
}

#[test]
fn adding_a_server_starts_at_its_address() {
    let (_core, hiphile) = core("");
    let _ui = open(hiphile, signal(Some(SignInRequest::AddServer)));

    assert_eq!(focused_field().as_deref(), Some("Server"));
}

#[test]
fn a_password_asked_for_is_typed_at_once() {
    let (_core, hiphile) = core("");
    let request = signal(Some(SignInRequest::Password {
        profile: config::Profile::new(NOWHERE, "alice"),
        failure: None,
    }));
    let _ui = open(hiphile, request);

    assert_eq!(focused_field().as_deref(), Some("Password"));
}

#[test]
fn a_profile_whose_password_is_needed_comes_with_why() {
    let (_core, hiphile) = core("");
    let request = signal(Some(SignInRequest::Password {
        profile: config::Profile::new(NOWHERE, "alice"),
        failure: Some("Wrong username or password.".to_owned()),
    }));
    let _ui = open(hiphile, request);

    assert!(find(Role::Heading, "Sign In").is_some());
    assert_eq!(value_of("Server").as_deref(), Some(NOWHERE));
    assert_eq!(value_of("Username").as_deref(), Some("alice"));
    assert!(has_text("Wrong username or password."));
}

#[test]
fn a_mistake_keeps_it_open_and_cancel_closes_it() {
    let (_core, hiphile) = core("");
    let request = signal(Some(SignInRequest::AddServer));
    let mut ui = open(hiphile, request);
    assert!(find(Role::Heading, "Add Server").is_some());

    press(&mut ui, Role::Button, "Sign In");
    assert!(has_text("Enter the server address."));
    assert_eq!(nodes(Role::Dialog).len(), 1);

    press(&mut ui, Role::Button, "Cancel");
    frames(&mut ui, 600);
    assert!(request.get_untracked().is_none());
    assert!(nodes(Role::Dialog).is_empty());
}

#[test]
fn escape_closes_it() {
    let (_core, hiphile) = core("");
    let request = signal(Some(SignInRequest::AddServer));
    let mut ui = open(hiphile, request);

    key(&mut ui, NamedKey::Escape);

    assert!(request.get_untracked().is_none());
}

#[test]
fn a_profile_signed_in_with_before_can_be_removed_while_the_fields_name_it() {
    let profile =
        |username| format!("[[profiles]]\nserver = \"{NOWHERE}\"\nusername = \"{username}\"\n");
    let (_core, hiphile) = core(&format!("{}{}", profile("alice"), profile("bob")));
    let request = signal(Some(SignInRequest::Password {
        profile: config::Profile::new(NOWHERE, "bob"),
        failure: None,
    }));
    let mut ui = open(hiphile.clone(), request);
    assert!(find(Role::Button, "Remove Server").is_some());

    focus(&mut ui, Role::TextInput, "Username");
    next_ui::testing::typed(&mut ui, "2");
    assert!(find(Role::Button, "Remove Server").is_none());
    key(&mut ui, NamedKey::Backspace);
    press(&mut ui, Role::Button, "Remove Server");

    assert_eq!(
        hiphile.snapshot().profiles(),
        [config::Profile::new(NOWHERE, "alice")]
    );
    assert!(request.get_untracked().is_none());
}
