use gpui_kit::TestAppContext;

use super::*;
use crate::testing::{NOWHERE, hiphile, settle, window};

#[gpui_kit::test]
fn a_profile_shown_calls_off_signing_in_again_as_whoever_did_last(cx: &mut TestAppContext) {
    // The server's answer would come from the network's own thread
    cx.executor().allow_parking();
    let (_dir, hiphile) = hiphile(
        &format!("server = \"{NOWHERE}\"\nusername = \"alice\"\npassword = \"secret\"\n"),
        cx,
    );
    let cx = window(cx);
    let view = cx.update(|window, cx| cx.new(|cx| SignIn::new(hiphile.clone(), window, cx)));
    let bob = config::Profile::new(NOWHERE, "bob");

    cx.update(|window, cx| view.update(cx, |view, cx| view.show_profile(&bob, None, window, cx)));
    settle(cx);

    assert!(!hiphile.snapshot().is_signing_in());
    cx.update(|_, cx| {
        let form = view.read(cx).form.read(cx);
        // Signing in as alice would have said the server can't be reached
        assert_eq!(form.failure(), None);
        assert_eq!(form.values(cx)[1], "bob");
    });
}
