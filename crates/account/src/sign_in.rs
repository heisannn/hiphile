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
use session::Session;
use subsonic::Credentials;

/// Emitted once the server accepted the credentials.
pub enum SignInEvent {
    SignedIn(Session),
}

/// The sign-in form. Owns its fields and the in-flight connection attempt;
/// the app shell decides what happens after [`SignInEvent::SignedIn`].
pub struct SignIn {
    net: net::Client,
    store: Option<config::Store>,
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    status: Status,
    _attempt: Task<()>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, PartialEq)]
enum Status {
    Idle,
    SigningIn,
    /// A field that must change before signing in can succeed.
    Invalid(InputField, SharedString),
    /// The server or network refused; the fields may be fine.
    Failed(SharedString),
}

#[derive(Clone, Copy, PartialEq)]
enum InputField {
    Server,
    Username,
    Password,
}

impl EventEmitter<SignInEvent> for SignIn {}

impl SignIn {
    pub fn new(net: net::Client, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = config::Store::platform().ok();
        let profile = store
            .as_ref()
            .and_then(|store| store.last_profile().ok().flatten());
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

        // Start where typing is needed: the first empty field.
        let first_empty = [&server, &username, &password]
            .into_iter()
            .find(|input| input.read(cx).value().is_empty())
            .unwrap_or(&password)
            .clone();
        first_empty.update(cx, |input, cx| input.focus(window, cx));

        Self {
            net,
            store,
            server,
            username,
            password,
            status: Status::Idle,
            _attempt: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status == Status::SigningIn {
            return;
        }
        let address = self.server.read(cx).value().trim().to_owned();
        let username = self.username.read(cx).value().trim().to_owned();
        let password = self.password.read(cx).value().to_string();

        let invalid = if address.is_empty() {
            Some((InputField::Server, "Enter the server address."))
        } else if username.is_empty() {
            Some((InputField::Username, "Enter your username."))
        } else if password.is_empty() {
            Some((InputField::Password, "Enter your password."))
        } else {
            None
        };
        if let Some((field, message)) = invalid {
            self.status = Status::Invalid(field, message.into());
            self.input(field)
                .update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }

        let credentials = Credentials::password(username.clone(), password);
        let server = match client::Server::new(&address, credentials) {
            Ok(server) => server,
            Err(error) => {
                self.status = Status::Invalid(InputField::Server, format!("{error}.").into());
                self.server.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
                return;
            }
        };
        let client = client::Client::new(self.net.clone(), server);
        self.status = Status::SigningIn;
        cx.notify();

        self._attempt = cx.spawn(async move |this, cx| {
            let result = client.ping().await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(info) => {
                        this.status = Status::Idle;
                        this.remember(&address, &username);
                        cx.emit(SignInEvent::SignedIn(Session::new(client, info)));
                    }
                    Err(error) => this.status = Status::Failed(describe(&error).into()),
                }
                cx.notify();
            })
            .ok();
        });
    }

    /// Saves where the user signed in, never the password. A failure only
    /// means the fields start empty next time, so it is not shown.
    fn remember(&self, server: &str, username: &str) {
        if let Some(store) = &self.store {
            store
                .save_profile(&config::Profile::new(server, username))
                .ok();
        }
    }

    fn input(&self, field: InputField) -> &Entity<InputState> {
        match field {
            InputField::Server => &self.server,
            InputField::Username => &self.username,
            InputField::Password => &self.password,
        }
    }

    fn field_error(&self, field: InputField) -> Option<SharedString> {
        match &self.status {
            Status::Invalid(invalid, message) if *invalid == field => Some(message.clone()),
            _ => None,
        }
    }
}

/// What went wrong, and what to try next.
fn describe(error: &client::Error) -> String {
    match error {
        client::Error::Protocol(subsonic::Error::Server { code, .. })
            if code.is_authentication() =>
        {
            "Wrong username or password.".to_owned()
        }
        client::Error::Protocol(subsonic::Error::Server { message, .. }) => {
            format!("The server refused to sign in: {message}")
        }
        client::Error::Protocol(subsonic::Error::InvalidResponse(_))
        | client::Error::HttpStatus(_)
        | client::Error::NotAudio(_) => {
            "No Subsonic server answered at this address. Check the address.".to_owned()
        }
        client::Error::Network(_) => {
            "Couldn't reach the server. Check the address and your connection.".to_owned()
        }
        client::Error::InvalidServerUrl(_) => format!("{error}."),
    }
}

impl Render for SignIn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let signing_in = self.status == Status::SigningIn;
        let failure = match &self.status {
            Status::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let field = |label: &'static str, field: InputField, input: Input, cx: &Context<Self>| {
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
                                InputField::Server,
                                Input::new(&self.server),
                                cx,
                            ))
                            .child(field(
                                "Username",
                                InputField::Username,
                                Input::new(&self.username),
                                cx,
                            ))
                            .child(field(
                                "Password",
                                InputField::Password,
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
