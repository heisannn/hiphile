use anti_webui::prelude::Role;
use next_ui::testing::{NOWHERE, core, find, focus, has_text, press, start, typed, wait_until};

use super::*;

const UNREACHABLE: &str = "Couldn't reach the server. Check the address and your connection.";

#[test]
fn an_empty_field_says_what_it_needs() {
    let (_core, hiphile) = core("");
    let mut ui = start({
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
    let mut ui = start({
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
    let mut ui = start(move || sign_in(hiphile));

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
    let (_core, hiphile) = core(&format!(
        "server = \"{NOWHERE}\"\nusername = \"alice\"\npassword = \"secret\"\n"
    ));
    let mut ui = start({
        let hiphile = hiphile.clone();
        move || sign_in(hiphile)
    });

    focus(&mut ui, Role::TextInput, "Username");
    typed(&mut ui, "2");
    wait_until(&mut ui, || !hiphile.snapshot().is_signing_in());
    std::thread::sleep(std::time::Duration::from_millis(50));
    ui.poll_images();
    ui.update();

    assert!(!has_text(UNREACHABLE));
}
