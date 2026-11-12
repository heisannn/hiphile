use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{
    Ink, MenuItem, MenuStyle, initials_avatar, menu_item, styled_menu,
};
use hiphile_core::format::format_profile;
use hiphile_core::{Hiphile, HiphileSnapshot, SwitchFailed};
use next_account::SignInRequest;
use next_host::{Pending, Scope};
use next_ui::Covers;
use next_ui::focus_ring::focus_ring;
use next_ui::window::kept_still;

/// The avatar's, which is round.
const RADIUS: f32 = 20.0;
/// The state layer of a press in Material 3, which stays while the menu is open.
const PRESSED: f32 = 0.10;

/// Who is signed in, which opens the servers to switch to, adding one and signing out. Adding
/// one, or a password that has to be typed, puts a request in `sign_in`. The menu is counted in
/// `covers` while it is open.
pub(crate) fn server_menu(
    hiphile: Hiphile,
    shown: Signal<HiphileSnapshot>,
    sign_in: Signal<Option<SignInRequest>>,
    covers: Covers,
) -> Div {
    let open = signal(false);
    covers.add(move || open.get());
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
                        .on_click(move || {
                            switch(&hiphile, profile.clone(), scope, sign_in);
                        })
                })
                .collect::<Vec<MenuItem>>();
            let add = menu_item("Add Server…")
                .on_click(move || sign_in.set(Some(SignInRequest::AddServer)));
            let sign_out = menu_item("Sign Out").on_click(move || hiphile.sign_out());
            styled_menu(
                move || open.get(),
                move || open.set(false),
                vec![switching, vec![add], vec![sign_out]],
                MenuStyle::new()
                    .anchor(anchor)
                    .placement(Placement::BelowEnd),
            )
        })
    });
    div()
        .child(kept_still(avatar_button(avatar, anchor, open)))
        .child(menu)
}

/// `avatar` as the button that opens the menu while `open`, with the state of a button over it.
fn avatar_button(avatar: View, anchor: NodeRef, open: Signal<bool>) -> Div {
    let ink = Ink::new(|c| c.on_primary_container, || false);
    let focused = ink.focused;
    // Over the avatar, whose fill would hide what is drawn under the children
    let layer = ink.paint(
        div()
            .absolute()
            .inset(0)
            .rounded(RADIUS)
            .pointer_events_none()
            .dyn_style(move |s| {
                if open.get() {
                    s.bg(color_scheme().on_primary_container.with_alpha(PRESSED))
                } else {
                    s
                }
            }),
    );
    let button = div()
        .node_ref(anchor)
        .relative()
        .flex_none()
        .rounded(RADIUS)
        .focusable()
        .access(move |a| a.role(Role::Button).label("Servers").expanded(open.get()))
        .dyn_style(move |s| focus_ring(s, focused.get(), &color_scheme()))
        .on_click(move |_| open.update(|open| *open = !*open))
        .child(avatar)
        .child(layer);
    ink.listen(button)
}

/// Signs in with `profile` and its saved password in place of whoever is signed in, or asks for
/// the password unless the dialog is open for what was asked since.
fn switch(
    hiphile: &Hiphile,
    profile: config::Profile,
    scope: Scope,
    sign_in: Signal<Option<SignInRequest>>,
) -> Pending {
    let switching = hiphile.switch_to(profile.clone());
    scope.on_ui(switching, move |switched| {
        let failure = match switched {
            Ok(()) => return,
            Err(SwitchFailed::NoPassword) => None,
            Err(SwitchFailed::SignIn(error)) => Some(error),
        };
        if sign_in.with_untracked(Option::is_none) {
            sign_in.set(Some(SignInRequest::Password { profile, failure }));
        }
    })
}

#[cfg(test)]
mod tests;
