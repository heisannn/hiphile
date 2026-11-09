use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AppContext as _, Entity, ParentElement as _, SharedString, Window};
use hiphile_core::Hiphile;

use crate::sign_in_form::SignInForm;

/// Signs in over the workspace, which stays until the new one replaces it: to another server, or
/// with `profile` when its password has to be typed, with `failure` if signing in with the saved
/// one went wrong. Closing it calls the attempt off.
pub fn open_sign_in_dialog(
    hiphile: &Hiphile,
    profile: Option<&config::Profile>,
    failure: Option<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    open(hiphile, profile, failure, window, cx);
}

/// The form open in the dialog.
pub(crate) fn open(
    hiphile: &Hiphile,
    profile: Option<&config::Profile>,
    failure: Option<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SignInForm> {
    let title = if profile.is_some() {
        "Sign In"
    } else {
        "Add Server"
    };
    let form = cx.new(|cx| {
        let mut form = SignInForm::new(hiphile.clone(), None, true, window, cx);
        if let Some(profile) = profile {
            form.show_profile(profile, failure, window, cx);
        }
        form
    });
    let body = form.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        let submitted = body.clone();
        dialog
            .title(title)
            .child(body.clone())
            // Enter reaches the dialog from the fields. A failure has to stay on show, and
            // signing in closes every dialog as the new workspace opens.
            .on_ok(move |_, window, cx| {
                submitted.update(cx, |form, cx| form.submit(window, cx));
                false
            })
    });
    form.update(cx, |form, cx| form.focus_first_empty(window, cx));
    form
}

#[cfg(test)]
mod tests;
