use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    form::{Field, Form},
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use hiphile_core::{Hiphile, SignInRefused};
use session::SignInField;

/// Signs in to `hiphile`, which then opens the workspace. Made, it signs in again as whoever did
/// last, as the core allows, until the user starts typing.
pub struct SignIn {
    hiphile: Hiphile,
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    status: Status,
    _resume: Task<()>,
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

impl SignIn {
    pub fn new(hiphile: Hiphile, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let profile = hiphile.last_profile();
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

        let on_input = |this: &mut Self,
                        _: &Entity<InputState>,
                        event: &InputEvent,
                        window: &mut Window,
                        cx: &mut Context<Self>| match event {
            InputEvent::PressEnter { .. } => this.sign_in(window, cx),
            InputEvent::Change => this._resume = Task::ready(()),
            _ => {}
        };
        let subscriptions = vec![
            cx.subscribe_in(&server, window, on_input),
            cx.subscribe_in(&username, window, on_input),
            cx.subscribe_in(&password, window, on_input),
        ];

        let first_empty = [&server, &username, &password]
            .into_iter()
            .find(|input| input.read(cx).value().is_empty())
            .unwrap_or(&password)
            .clone();
        first_empty.update(cx, |input, cx| input.focus(window, cx));

        let resume = match hiphile.resume() {
            Some(resume) => cx.spawn(async move |this, cx| {
                let result = resume.await;
                this.update(cx, |this, cx| this.finish(result, cx)).ok();
            }),
            None => Task::ready(()),
        };
        Self {
            _signing_in: gpui_host::notify_on_change(hiphile.subscribe(), cx),
            hiphile,
            server,
            username,
            password,
            status: Status::Idle,
            _resume: resume,
            _attempt: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn finish(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.status = match result {
            Ok(()) => Status::Idle,
            Err(error) => Status::Failed(error.into()),
        };
        cx.notify();
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
        let signing_in = self.hiphile.snapshot().is_signing_in();
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
