use std::cell::Cell;

use anti_webui::core::event::NamedKey;
use next_account::sign_in_dialog;
use next_ui::testing::{
    NOWHERE, core, find, frames, key, press, signed_in_core, start, wait_until,
};

use super::*;

#[test]
fn the_avatar_opens_the_menu_which_covers_the_screens_until_it_closes() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let covers = Covers::new();
    let mut ui = start(move || {
        let shown = next_host::follow(hiphile.subscribe());
        div()
            .size_full()
            .child(server_menu(hiphile, shown, signal(None), covers))
    });
    assert_eq!(find(Role::Button, "Servers").unwrap().expanded, Some(false));
    assert!(!covers.any());

    press(&mut ui, Role::Button, "Servers");
    assert_eq!(find(Role::Button, "Servers").unwrap().expanded, Some(true));
    assert!(find(Role::MenuItem, "Sign Out").is_some());
    assert!(covers.any());

    key(&mut ui, NamedKey::Escape);
    frames(&mut ui, 400);
    assert_eq!(find(Role::Button, "Servers").unwrap().expanded, Some(false));
    assert!(!covers.any());
}

/// Who has no password saved, so that switching to them asks for one.
const BOB: &str = "bob";

/// The sign-in dialog on `request`, and where switching waits.
fn dialog_on(
    hiphile: &Hiphile,
    request: Signal<Option<SignInRequest>>,
) -> (anti_webui::core::Ui, Scope) {
    let scope = Cell::new(None);
    let ui = start(|| {
        scope.set(Some(Scope::here()));
        div()
            .size_full()
            .child(sign_in_dialog(hiphile.clone(), request))
    });
    (ui, scope.get().expect("the root to have run"))
}

fn bob() -> config::Profile {
    config::Profile::new(NOWHERE, BOB)
}

fn with_bob() -> String {
    format!("[[profiles]]\nserver = \"{NOWHERE}\"\nusername = \"{BOB}\"\n")
}

#[test]
fn switching_to_whoever_has_no_password_saved_asks_for_it() {
    let (_core, hiphile) = core(&with_bob());
    let request = signal(None);
    let (mut ui, scope) = dialog_on(&hiphile, request);

    let switching = switch(&hiphile, bob(), scope, request);
    wait_until(&mut ui, || !switching.is_waiting());

    assert_eq!(
        request.get_untracked(),
        Some(SignInRequest::Password {
            profile: bob(),
            failure: None
        })
    );
    wait_until(&mut ui, || find(Role::Heading, "Sign In").is_some());
}

#[test]
fn a_switch_that_ends_while_the_dialog_is_open_leaves_it_as_it_is() {
    let (_core, hiphile) = core(&with_bob());
    let request = signal(None);
    let (mut ui, scope) = dialog_on(&hiphile, request);

    let switching = switch(&hiphile, bob(), scope, request);
    request.set(Some(SignInRequest::AddServer));
    wait_until(&mut ui, || !switching.is_waiting());

    assert_eq!(request.get_untracked(), Some(SignInRequest::AddServer));
    assert!(find(Role::Heading, "Add Server").is_some());
    assert!(find(Role::Heading, "Sign In").is_none());
}
