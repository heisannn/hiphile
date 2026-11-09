use gpui_kit::{Focusable as _, TestAppContext};
use session::SignInField;

use super::*;
use crate::testing::{NOWHERE, hiphile, window};

#[gpui_kit::test]
fn a_profile_whose_password_is_needed_comes_with_why_and_waits_for_it(cx: &mut TestAppContext) {
    let (_dir, hiphile) = hiphile("", cx);
    let cx = window(cx);
    let alice = config::Profile::new(NOWHERE, "alice");
    let failure = SharedString::from("Wrong username or password.");

    let form =
        cx.update(|window, cx| open(&hiphile, Some(&alice), Some(failure.clone()), window, cx));
    cx.run_until_parked();

    cx.update(|window, cx| {
        let form = form.read(cx);
        assert_eq!(form.values(cx), [NOWHERE, "alice", ""]);
        assert_eq!(form.failure(), Some(&failure));
        let password = form.input(SignInField::Password);
        assert!(password.focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn enter_in_the_dialog_signs_in_and_a_mistake_keeps_it_open(cx: &mut TestAppContext) {
    let (_dir, hiphile) = hiphile("", cx);
    let cx = window(cx);
    let form = cx.update(|window, cx| open(&hiphile, None, None, window, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("enter");

    cx.update(|window, cx| {
        assert!(window.has_active_dialog(cx));
        let form = form.read(cx);
        assert!(form.field_error(SignInField::Server).is_some());
        let server = form.input(SignInField::Server);
        assert!(server.focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn a_profile_signed_in_with_before_can_be_removed_while_the_fields_name_it(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let profile =
        |username| format!("[[profiles]]\nserver = \"{NOWHERE}\"\nusername = \"{username}\"\n");
    let (_dir, hiphile) = hiphile(&format!("{}{}", profile("alice"), profile("bob")), cx);
    let alice = config::Profile::new(NOWHERE, "alice");
    let bob = config::Profile::new(NOWHERE, "bob");
    let cx = window(cx);
    let adding = cx.update(|window, cx| open(&hiphile, None, None, window, cx));
    cx.update(|_, cx| assert!(!adding.read(cx).has_remove(cx)));
    cx.update(|window, cx| window.close_dialog(cx));
    let form = cx.update(|window, cx| open(&hiphile, Some(&bob), None, window, cx));

    cx.update(|window, cx| {
        let username = form.read(cx).input(SignInField::Username).clone();
        username.update(cx, |input, cx| input.set_value("carol", window, cx));
        assert!(!form.read(cx).has_remove(cx));
        username.update(cx, |input, cx| input.set_value("bob", window, cx));
        assert!(form.read(cx).has_remove(cx));
        form.update(cx, |form, cx| form.remove(window, cx));
    });
    cx.run_until_parked();

    assert_eq!(hiphile.snapshot().profiles(), [alice]);
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
}
