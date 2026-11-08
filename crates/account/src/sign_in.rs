use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    form::{Field, Form},
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use session::{Accounts, Session, SignInField, SignedIn};

pub enum SignInEvent {
    SignedIn {
        session: Session,
        profile: config::Profile,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Signs in with the last profile's saved password, if there is one.
    Resume,
    Fresh,
}

pub struct SignIn {
    accounts: Accounts,
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    status: Status,
    _resume: Task<()>,
    _attempt: Task<()>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, PartialEq)]
enum Status {
    Idle,
    SigningIn,
    Invalid(SignInField, SharedString),
    Failed(SharedString),
}

impl EventEmitter<SignInEvent> for SignIn {}

impl SignIn {
    pub fn new(
        accounts: Accounts,
        start: Start,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let profile = accounts.last_profile();
        let server = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://music.example.com")
                .default_value(
                    profile
                        .as_ref()
                        .map(|profile| profile.server().to_owned())
                        .unwrap_or_default(),
                )
        });
        let username = cx.new(|cx| {
            InputState::new(window, cx).default_value(
                profile
                    .as_ref()
                    .map(|profile| profile.username().to_owned())
                    .unwrap_or_default(),
            )
        });
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));

        let submit_on_enter = |this: &mut Self,
                               _: &Entity<InputState>,
                               event: &InputEvent,
                               window: &mut Window,
                               cx: &mut Context<Self>| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.sign_in(window, cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&server, window, submit_on_enter),
            cx.subscribe_in(&username, window, submit_on_enter),
            cx.subscribe_in(&password, window, submit_on_enter),
        ];

        let first_empty = [&server, &username, &password]
            .into_iter()
            .find(|input| input.read(cx).value().is_empty())
            .unwrap_or(&password)
            .clone();
        first_empty.update(cx, |input, cx| input.focus(window, cx));

        let mut this = Self {
            accounts,
            server,
            username,
            password,
            status: Status::Idle,
            _resume: Task::ready(()),
            _attempt: Task::ready(()),
            _subscriptions: subscriptions,
        };
        if start == Start::Resume
            && let Some(profile) = profile
        {
            this.resume(profile, window, cx);
        }
        this
    }

    /// Without a saved password, or if the keychain fails, the form waits for the user.
    fn resume(&mut self, profile: config::Profile, window: &mut Window, cx: &mut Context<Self>) {
        let lookup = self.accounts.saved_password(profile);
        self._resume = cx.spawn_in(window, async move |this, cx| {
            let Some(password) = lookup.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                // The user may have started typing meanwhile
                if this.password.read(cx).value().is_empty() {
                    this.password
                        .update(cx, |input, cx| input.set_value(password, window, cx));
                    this.sign_in(window, cx);
                }
            })
            .ok();
        });
    }

    fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status == Status::SigningIn {
            return;
        }
        let attempt = self.accounts.sign_in(
            &self.server.read(cx).value(),
            &self.username.read(cx).value(),
            &self.password.read(cx).value(),
        );
        let attempt = match attempt {
            Ok(attempt) => attempt,
            Err(invalid) => {
                self.status = Status::Invalid(invalid.field(), invalid.message().to_owned().into());
                self.input(invalid.field())
                    .update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
                return;
            }
        };
        self.status = Status::SigningIn;
        cx.notify();

        self._attempt = cx.spawn(async move |this, cx| {
            let result = attempt.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(SignedIn {
                        session, profile, ..
                    }) => {
                        this.status = Status::Idle;
                        cx.emit(SignInEvent::SignedIn { session, profile });
                    }
                    Err(error) => this.status = Status::Failed(error.into()),
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn input(&self, field: SignInField) -> &Entity<InputState> {
        match field {
            SignInField::Server => &self.server,
            SignInField::Username => &self.username,
            SignInField::Password => &self.password,
        }
    }

    fn field_error(&self, field: SignInField) -> Option<SharedString> {
        match &self.status {
            Status::Invalid(invalid, message) if *invalid == field => Some(message.clone()),
            _ => None,
        }
    }
}

impl Render for SignIn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let signing_in = self.status == Status::SigningIn;
        let failure = match &self.status {
            Status::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let field = |label: &'static str, field: SignInField, input: Input, cx: &Context<Self>| {
            Field::new()
                .label(label)
                .child(input.disabled(signing_in))
                .when_some(self.field_error(field), |this, message| {
                    this.child(div().text_sm().text_color(cx.theme().danger).child(message))
                })
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .w_96()
                    .gap_6()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_xl().font_semibold().child("Sign in"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Connect to a Navidrome or other Subsonic server."),
                            ),
                    )
                    .child(
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
                            .footer(
                                Button::new("sign-in")
                                    .primary()
                                    .label("Sign in")
                                    .loading(signing_in)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.sign_in(window, cx)),
                                    ),
                            ),
                    )
                    .when_some(failure, |this, message| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(message))
                    }),
            )
    }
}
