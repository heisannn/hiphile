use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::reactive::effect_with;
use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{
    ButtonStyle, IconButtonStyle, TextFieldStyle, icon_toggle_button,
    indeterminate_circular_progress, outlined_text_field, styled_button,
};
use anti_webui_md3e::icons;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::{Hiphile, SignInRefused};
use next_host::{Pending, Scope, follow, on_ui};
use next_ui::type_scale::{error_text, muted_text, styled_text};
use session::SignInField;

const WIDTH: i32 = 400;

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Idle,
    Invalid(SignInField, String),
    Failed(String),
}

/// Signs in to `hiphile`, which then opens the workspace. Shown, it signs in again as whoever did
/// last, as the core allows, until the user starts typing.
pub fn sign_in(hiphile: Hiphile) -> Div {
    let profile = hiphile.last_profile();
    let server = signal(
        profile
            .as_ref()
            .map_or_else(String::new, |p| p.server().to_owned()),
    );
    let username = signal(
        profile
            .as_ref()
            .map_or_else(String::new, |p| p.username().to_owned()),
    );
    let password = signal(String::new());
    let status = signal(Status::Idle);
    let shown = follow(hiphile.subscribe());
    let signing_in = move || shown.with(|shown| shown.is_signing_in());

    let finish = move |result: Result<(), String>| {
        status.set(match result {
            Ok(()) => Status::Idle,
            Err(error) => Status::Failed(error),
        });
    };
    let resuming: Rc<RefCell<Option<Pending>>> = Rc::new(RefCell::new(
        hiphile.resume().map(|task| on_ui(task, finish)),
    ));
    effect_with({
        let resuming = resuming.clone();
        move |started: Option<()>| {
            server.with(|_| ());
            username.with(|_| ());
            password.with(|_| ());
            if started.is_some()
                && let Some(resume) = resuming.borrow_mut().take()
            {
                resume.cancel();
            }
        }
    });

    let scope = Scope::here();
    let attempt: Rc<RefCell<Option<Pending>>> = Rc::default();
    let submit = Rc::new(move || {
        let tried = hiphile.sign_in(
            &server.get_untracked(),
            &username.get_untracked(),
            &password.get_untracked(),
        );
        match tried {
            Ok(task) => {
                status.set(Status::Idle);
                *attempt.borrow_mut() = Some(scope.on_ui(task, finish));
            }
            Err(SignInRefused::Invalid(invalid)) => {
                status.set(Status::Invalid(
                    invalid.field(),
                    invalid.message().to_owned(),
                ));
            }
            Err(SignInRefused::SigningIn) => {}
        }
    });

    let fields = div()
        .flex()
        .flex_col()
        .gap(4.0_f32)
        .child(field(
            server,
            "Server",
            SignInField::Server,
            status,
            submit.clone(),
            signing_in,
        ))
        .child(field(
            username,
            "Username",
            SignInField::Username,
            status,
            submit.clone(),
            signing_in,
        ))
        .child(password_field(password, status, submit.clone(), signing_in));
    let failure = show(
        move || matches!(status.get(), Status::Failed(_)),
        move || {
            error_text(move || match status.get() {
                Status::Failed(error) => error,
                _ => String::new(),
            })
        },
    );
    let actions = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(16.0_f32)
        .child(show(signing_in, indeterminate_circular_progress))
        .child(
            styled_button("Sign in", ButtonStyle::filled().disabled_when(signing_in))
                .on_click(move |_| submit()),
        );

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .dyn_style(|s| s.bg(color_scheme().surface))
        .child(
            div()
                .w(WIDTH)
                .flex()
                .flex_col()
                .gap(24.0_f32)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(8.0_f32)
                        .child(
                            styled_text(TextStyle::HEADLINE_SMALL, "Sign in")
                                .access(|a| a.role(Role::Heading).level(1)),
                        )
                        .child(muted_text(
                            TextStyle::BODY_MEDIUM,
                            "Connect to a Navidrome or other Subsonic server.",
                        )),
                )
                .child(fields)
                .child(failure)
                .child(actions),
        )
}

fn field(
    value: Signal<String>,
    label: &'static str,
    which: SignInField,
    status: Signal<Status>,
    submit: Rc<dyn Fn()>,
    signing_in: impl Fn() -> bool + 'static,
) -> Div {
    let mut style = TextFieldStyle::new()
        .label(label)
        .on_submit(move |_| submit())
        .disabled_when(signing_in);
    if which == SignInField::Server {
        style = style.placeholder("https://music.example.com");
    }
    outlined_text_field(value, with_error(style, which, status)).w(WIDTH)
}

/// Its look is chosen when it is made, so showing what is typed makes it anew.
fn password_field(
    value: Signal<String>,
    status: Signal<Status>,
    submit: Rc<dyn Fn()>,
    signing_in: impl Fn() -> bool + Clone + 'static,
) -> View {
    let visible = signal(false);
    dyn_view(move || {
        let shown = visible.get();
        let (submit, signing_in) = (submit.clone(), signing_in.clone());
        untrack(move || {
            let mut style = TextFieldStyle::new()
                .label("Password")
                .on_submit(move |_| submit())
                .disabled_when(signing_in)
                .trailing(
                    icon_toggle_button(icons::VISIBILITY, visible, IconButtonStyle::standard())
                        .access(move |a| a.label("Show Password")),
                );
            if !shown {
                style = style.password();
            }
            outlined_text_field(value, with_error(style, SignInField::Password, status)).w(WIDTH)
        })
    })
}

fn with_error(style: TextFieldStyle, which: SignInField, status: Signal<Status>) -> TextFieldStyle {
    let error = move || match status.get() {
        Status::Invalid(field, message) if field == which => Some(message),
        _ => None,
    };
    let is_error = error;
    style
        .error_when(move || is_error().is_some())
        .supporting(move || error().unwrap_or_default())
}

#[cfg(test)]
mod tests;
