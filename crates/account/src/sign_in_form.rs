use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    form::{Field, Form},
    h_flex,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use hiphile_core::{Hiphile, SignInRefused};
use session::SignInField;

/// The fields that sign in to `hiphile`, which then opens the workspace in place of any other.
pub(crate) struct SignInForm {
    hiphile: Hiphile,
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    status: Status,
    /// In a dialog, Enter reaches the dialog, which submits, and Cancel closes it.
    in_dialog: bool,
    /// The saved profile put in the fields, which a dialog can take off the profiles.
    profile: Option<config::Profile>,
    _attempt: Task<()>,
    _signing_in: Task<()>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, PartialEq)]
enum Status {
    Idle,
    Invalid(SignInField, SharedString),
    Failed(SharedString),
}

/// The user typed in a field.
pub(crate) struct Edited;

impl EventEmitter<Edited> for SignInForm {}

impl SignInForm {
    /// Starts on `profile`, if any, for its password to be typed.
    pub(crate) fn new(
        hiphile: Hiphile,
        profile: Option<&config::Profile>,
        in_dialog: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let server = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://music.example.com")
                .default_value(
                    profile
                        .map(|profile| profile.server().to_owned())
                        .unwrap_or_default(),
                )
        });
        let username = cx.new(|cx| {
            InputState::new(window, cx).default_value(
                profile
                    .map(|profile| profile.username().to_owned())
                    .unwrap_or_default(),
            )
        });
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));

        let on_input = |this: &mut Self,
                        _: &Entity<InputState>,
                        event: &InputEvent,
                        window: &mut Window,
                        cx: &mut Context<Self>| match event {
            InputEvent::PressEnter { .. } if !this.in_dialog => this.submit(window, cx),
            InputEvent::Change => {
                cx.emit(Edited);
                cx.notify();
            }
            _ => {}
        };
        let subscriptions = vec![
            cx.subscribe_in(&server, window, on_input),
            cx.subscribe_in(&username, window, on_input),
            cx.subscribe_in(&password, window, on_input),
        ];
        Self {
            _signing_in: gpui_host::notify_on_change(hiphile.subscribe(), cx),
            hiphile,
            server,
            username,
            password,
            status: Status::Idle,
            in_dialog,
            profile: None,
            _attempt: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    /// The first field left to fill, or else the password.
    pub(crate) fn focus_first_empty(&self, window: &mut Window, cx: &mut Context<Self>) {
        [&self.server, &self.username, &self.password]
            .into_iter()
            .find(|input| input.read(cx).value().is_empty())
            .unwrap_or(&self.password)
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Puts `profile` in the fields for its password to be typed, with `failure` if signing in
    /// with the saved one went wrong.
    pub(crate) fn show_profile(
        &mut self,
        profile: &config::Profile,
        failure: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fields = [
            (&self.server, profile.server()),
            (&self.username, profile.username()),
            (&self.password, ""),
        ];
        for (input, value) in fields {
            input.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            });
        }
        self.status = failure.map_or(Status::Idle, Status::Failed);
        self.profile = Some(profile.clone());
        self.password
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let attempt = self.hiphile.sign_in(
            &self.server.read(cx).value(),
            &self.username.read(cx).value(),
            &self.password.read(cx).value(),
        );
        let attempt = match attempt {
            Ok(attempt) => attempt,
            Err(SignInRefused::Invalid(invalid)) => {
                self.status = Status::Invalid(invalid.field(), invalid.message().to_owned().into());
                self.input(invalid.field())
                    .update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
                return;
            }
            Err(SignInRefused::SigningIn) => return,
        };
        self.status = Status::Idle;
        cx.notify();

        self._attempt = cx.spawn(async move |this, cx| {
            let result = attempt.await;
            this.update(cx, |this, cx| this.finish(result, cx)).ok();
        });
    }

    /// Takes the saved profile off the profiles and closes the dialog.
    pub(crate) fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(profile) = self.removable(cx) {
            self.hiphile.forget(profile);
            window.close_dialog(cx);
        }
    }

    pub(crate) fn finish(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.status = match result {
            Ok(()) => Status::Idle,
            Err(error) => Status::Failed(error.into()),
        };
        cx.notify();
    }

    pub(crate) fn input(&self, field: SignInField) -> &Entity<InputState> {
        match field {
            SignInField::Server => &self.server,
            SignInField::Username => &self.username,
            SignInField::Password => &self.password,
        }
    }

    pub(crate) fn field_error(&self, field: SignInField) -> Option<SharedString> {
        match &self.status {
            Status::Invalid(invalid, message) if *invalid == field => Some(message.clone()),
            _ => None,
        }
    }

    /// The saved profile, in a dialog while the fields still name it.
    fn removable(&self, cx: &App) -> Option<config::Profile> {
        let profile = self.profile.as_ref().filter(|_| self.in_dialog)?;
        let named = config::Profile::new(
            self.server.read(cx).value().trim(),
            self.username.read(cx).value().trim(),
        );
        (named == *profile).then(|| profile.clone())
    }
}

impl Render for SignInForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let signing_in = self.hiphile.snapshot().is_signing_in();
        let failure = match &self.status {
            Status::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let error = |message: SharedString, cx: &Context<Self>| {
            div().text_sm().text_color(cx.theme().danger).child(message)
        };
        let field = |label: &'static str, field: SignInField, input: Input, cx: &Context<Self>| {
            Field::new()
                .label(label)
                .child(input.disabled(signing_in))
                .when_some(self.field_error(field), |this, message| {
                    this.child(error(message, cx))
                })
        };
        Form::new()
            .child(field(
                "Server",
                SignInField::Server,
                Input::new(&self.server),
                cx,
            ))
            .child(field(
                "Username",
                SignInField::Username,
                Input::new(&self.username),
                cx,
            ))
            .child(field(
                "Password",
                SignInField::Password,
                Input::new(&self.password).mask_toggle(),
                cx,
            ))
            .when_some(failure, |this, message| {
                this.child(Field::new().label_indent(false).child(error(message, cx)))
            })
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .when(self.removable(cx).is_some(), |this| {
                        this.child(
                            Button::new("remove-server")
                                .danger()
                                .outline()
                                .label("Remove Server")
                                .disabled(signing_in)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.remove(window, cx)),
                                ),
                        )
                    })
                    .child(div().flex_1())
                    .when(self.in_dialog, |this| {
                        this.child(
                            Button::new("cancel-sign-in")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                    })
                    .child(
                        Button::new("sign-in")
                            .primary()
                            .label("Sign in")
                            .loading(signing_in)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
impl SignInForm {
    pub(crate) fn has_remove(&self, cx: &App) -> bool {
        self.removable(cx).is_some()
    }

    pub(crate) fn failure(&self) -> Option<&SharedString> {
        match &self.status {
            Status::Failed(message) => Some(message),
            _ => None,
        }
    }

    pub(crate) fn values(&self, cx: &App) -> [SharedString; 3] {
        [&self.server, &self.username, &self.password].map(|input| input.read(cx).value())
    }
}
