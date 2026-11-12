use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    MenuItem, MenuStyle, Snackbar, Snackbars, initials_avatar, menu_item, styled_menu,
};
use hiphile_core::format::format_profile;
use hiphile_core::{Hiphile, HiphileSnapshot, SwitchFailed};
use next_host::Scope;
use next_ui::window::kept_still;

/// Who is signed in, which opens the servers to switch to and signing out.
pub(crate) fn server_menu(
    hiphile: Hiphile,
    shown: Signal<HiphileSnapshot>,
    snackbars: Snackbars,
) -> Div {
    let open = signal(false);
    let signed_in = memo(move || shown.with(|shown| shown.profile().cloned()));
    let profiles = memo(move || shown.with(|shown| shown.profiles().to_vec()));
    let anchor = node_ref();
    let scope = Scope::here();
    let avatar = dyn_view(move || {
        let name = signed_in
            .get()
            .map(|profile| profile.username().to_owned())
            .unwrap_or_default();
        untrack(|| initials_avatar(name))
    });
    let menu = dyn_view(move || {
        let (profiles, signed_in) = (profiles.get(), signed_in.get());
        let hiphile = hiphile.clone();
        untrack(move || {
            let switching = profiles
                .into_iter()
                .map(|profile| {
                    let is_signed_in = signed_in.as_ref() == Some(&profile);
                    let hiphile = hiphile.clone();
                    menu_item(format_profile(&profile))
                        .selected_when(move || is_signed_in)
                        .on_click(move || switch(&hiphile, profile.clone(), scope, snackbars))
                })
                .collect::<Vec<MenuItem>>();
            let sign_out = menu_item("Sign Out").on_click(move || hiphile.sign_out());
            styled_menu(
                move || open.get(),
                move || open.set(false),
                vec![switching, vec![sign_out]],
                MenuStyle::new()
                    .anchor(anchor)
                    .placement(Placement::AboveStart),
            )
        })
    });
    div()
        .child(kept_still(
            div()
                .node_ref(anchor)
                .flex_none()
                .rounded(20.0_f32)
                .focusable()
                .access(|a| a.role(Role::Button).label("Servers"))
                .on_click(move |_| open.update(|open| *open = !*open))
                .child(avatar),
        ))
        .child(menu)
}

/// Signs in with `profile` and its saved password in place of whoever is signed in.
fn switch(hiphile: &Hiphile, profile: config::Profile, scope: Scope, snackbars: Snackbars) {
    let switching = hiphile.switch_to(profile.clone());
    scope.on_ui(switching, move |switched| {
        let message = match switched {
            Ok(()) => return,
            Err(SwitchFailed::NoPassword) => format!(
                "No password is saved for {}. Sign out, then sign in to it.",
                format_profile(&profile)
            ),
            Err(SwitchFailed::SignIn(error)) => error,
        };
        snackbars.show(Snackbar::new(message).closable());
    });
}
