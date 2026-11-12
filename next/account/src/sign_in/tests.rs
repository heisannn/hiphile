use std::net::TcpListener;

use anti_webui::core::access::{AccessNode, snapshot};
use anti_webui::prelude::Role;
use next_ui::testing::{NOWHERE, core, find, focus, has_text, press, start, typed, wait_until};

use super::*;

const UNREACHABLE: &str = "Couldn't reach the server. Check the address and your connection.";

/// `view` as the window draws it the second time, once what takes the focus as it opens has.
fn shown<V: IntoView>(view: impl FnOnce() -> V) -> anti_webui::core::Ui {
    let mut ui = start(view);
    ui.update();
    ui
}

fn focused() -> Option<AccessNode> {
    let tree = snapshot();
    tree.get(tree.focus?).cloned()
}

fn focused_field() -> Option<String> {
    focused()
        .filter(|node| node.role == Role::TextInput)
        .and_then(|node| node.label)
}

#[test]
fn typing_starts_at_the_server_with_no_one_to_sign_in_again() {
    let (_core, hiphile) = core("");
    let _ui = shown(move || sign_in(hiphile));

    assert_eq!(focused_field().as_deref(), Some("Server"));
}

#[test]
fn typing_starts_at_what_whoever_signed_in_last_leaves_empty() {
    let (_core, hiphile) = core(&format!(
        "server = \"{NOWHERE}\"\nusername = \"alice\"\npassword = \"secret\"\n"
    ));
    let mut ui = shown(move || sign_in(hiphile));

    assert_eq!(focused_field().as_deref(), Some("Password"));
    wait_until(&mut ui, || has_text(UNREACHABLE));
}

#[test]
fn a_field_that_is_wrong_takes_the_focus() {
    let (_core, hiphile) = core("");
    let mut ui = shown(move || sign_in(hiphile));
    typed(&mut ui, NOWHERE);

    press(&mut ui, Role::Button, "Sign In");

    assert!(has_text("Enter your username."));
    assert_eq!(focused_field().as_deref(), Some("Username"));
}

#[test]
fn showing_the_password_from_the_keys_keeps_the_focus_on_the_button() {
    let (_core, hiphile) = core("");
    let mut ui = shown(move || sign_in(hiphile));
    let shows = || {
        find(Role::ToggleButton, "Show Password")
            .and_then(|node| node.checked)
            .map(|checked| checked == anti_webui::core::access::Checked::True)
    };
    focus(&mut ui, Role::ToggleButton, "Show Password");
    assert_eq!(shows(), Some(false));

    for showing in [true, false] {
        next_ui::testing::key(&mut ui, anti_webui::core::event::NamedKey::Space);
        ui.update();
        assert_eq!(shows(), Some(showing));
        let focused = focused().expect("something focused");
        assert_eq!(focused.label.as_deref(), Some("Show Password"));
    }
}

#[test]
fn an_empty_field_says_what_it_needs() {
    let (_core, hiphile) = core("");
    let mut ui = shown({
        let hiphile = hiphile.clone();
        move || sign_in(hiphile)
    });

    press(&mut ui, Role::Button, "Sign In");

    assert!(has_text("Enter the server address."));
    assert!(!hiphile.snapshot().is_signing_in());
}

#[test]
fn a_server_that_cannot_be_reached_is_said_so_and_the_form_comes_back() {
    let (_core, hiphile) = core("");
    let mut ui = shown({
        let hiphile = hiphile.clone();
        move || sign_in(hiphile)
    });
    for (field, value) in [
        ("Server", NOWHERE),
        ("Username", "alice"),
        ("Password", "secret"),
    ] {
        focus(&mut ui, Role::TextInput, field);
        typed(&mut ui, value);
    }

    press(&mut ui, Role::Button, "Sign In");
    assert!(hiphile.snapshot().is_signing_in());
    wait_until(&mut ui, || has_text(UNREACHABLE));

    assert!(!hiphile.snapshot().is_signing_in());
    assert!(!find(Role::Button, "Sign In").unwrap().disabled);
}

#[test]
fn it_signs_in_again_as_whoever_did_last() {
    let (_core, hiphile) = core(&format!(
        "server = \"{NOWHERE}\"\nusername = \"alice\"\npassword = \"secret\"\n"
    ));
    let mut ui = shown(move || sign_in(hiphile));

    wait_until(&mut ui, || has_text(UNREACHABLE));
    assert_eq!(
        find(Role::TextInput, "Username")
            .unwrap()
            .value_text
            .as_deref(),
        Some("alice")
    );
}

#[test]
fn typing_calls_off_signing_in_again() {
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let (_core, hiphile) = core(&format!(
        "server = \"http://{}\"\nusername = \"alice\"\npassword = \"secret\"\n",
        silent.local_addr().unwrap()
    ));
    let mut ui = shown({
        let hiphile = hiphile.clone();
        move || sign_in(hiphile)
    });
    assert!(hiphile.snapshot().is_signing_in());

    focus(&mut ui, Role::TextInput, "Username");
    typed(&mut ui, "2");

    // The server takes the connection and never answers, so only calling it off ends signing in
    // this soon
    wait_until(&mut ui, || {
        !hiphile.snapshot().is_signing_in() && !find(Role::Button, "Sign In").unwrap().disabled
    });
}
