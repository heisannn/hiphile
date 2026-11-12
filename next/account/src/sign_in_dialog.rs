use anti_webui::prelude::*;
use anti_webui_md3e::components::{ButtonStyle, dialog, styled_button, text_button};
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::Hiphile;
use next_ui::type_scale::styled_text;

use crate::form::{Fields, form};

/// Why the sign-in dialog is open.
#[derive(Clone, Debug, PartialEq)]
pub enum SignInRequest {
    /// To sign in to another server.
    AddServer,
    /// To type the password of `profile`, with `failure` if signing in with the saved one went
    /// wrong.
    Password {
        profile: config::Profile,
        failure: Option<String>,
    },
}

/// Signs in over the workspace, which stays until the new one replaces it. It is open while
/// `request` holds one, and closing it calls the attempt off.
pub fn sign_in_dialog(hiphile: Hiphile, request: Signal<Option<SignInRequest>>) -> Show {
    dialog(
        move || request.with(Option::is_some),
        move || request.set(None),
        move || {
            let Some(asked) = request.get_untracked() else {
                return div();
            };
            let hiphile = hiphile.clone();
            let (title, fields, saved) = match asked {
                SignInRequest::AddServer => ("Add Server", Fields::new(None, None), None),
                SignInRequest::Password { profile, failure } => (
                    "Sign In",
                    Fields::new(Some(&profile), failure),
                    Some(profile),
                ),
            };
            let remove = saved.map(|saved| {
                let hiphile = hiphile.clone();
                let named = saved.clone();
                show(
                    move || fields.profile() == named,
                    move || {
                        let (hiphile, saved) = (hiphile.clone(), saved.clone());
                        styled_button("Remove Server", ButtonStyle::outlined()).on_click(
                            move |_| {
                                hiphile.forget(saved.clone());
                                request.set(None);
                            },
                        )
                    },
                )
            });
            let actions = vec![
                div().children(remove),
                div().flex_1(),
                text_button("Cancel").on_click(move |_| request.set(None)),
            ];
            div()
                .flex()
                .flex_col()
                .gap(16.0_f32)
                .p(24.0_f32)
                .child(
                    styled_text(TextStyle::HEADLINE_SMALL, title)
                        .access(|a| a.role(Role::Heading).level(2)),
                )
                .child(form(hiphile, fields, actions))
        },
    )
}

#[cfg(test)]
mod tests;
