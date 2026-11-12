use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    ButtonStyle, IconButtonStyle, TextFieldStyle, icon_toggle_button,
    indeterminate_circular_progress, outlined_text_field, styled_button,
};
use anti_webui_md3e::icons;
use hiphile_core::{Hiphile, SignInRefused};
use next_host::{Pending, Scope, follow};
use next_ui::dialog_parts::{focus_field, focused_field, focused_once_laid_out};
use next_ui::type_scale::error_text;
use session::SignInField;

pub(crate) const WIDTH: i32 = 400;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Status {
    Idle,
    Invalid(SignInField, String),
    Failed(String),
}

/// What is typed to sign in, and how the last try went. Cheap to copy.
#[derive(Clone, Copy)]
pub(crate) struct Fields {
    pub(crate) server: Signal<String>,
    pub(crate) username: Signal<String>,
    pub(crate) password: Signal<String>,
    pub(crate) status: Signal<Status>,
}

impl Fields {
    /// With `profile` in place for its password to be typed, and `failure` if signing in with the
    /// saved one went wrong.
    pub(crate) fn new(profile: Option<&config::Profile>, failure: Option<String>) -> Self {
        let named = |part: fn(&config::Profile) -> &str| {
            signal(profile.map_or_else(String::new, |p| part(p).to_owned()))
        };
        Self {
            server: named(config::Profile::server),
            username: named(config::Profile::username),
            password: signal(String::new()),
            status: signal(failure.map_or(Status::Idle, Status::Failed)),
        }
    }

    /// Reads every field, so that what calls it follows any typing.
    pub(crate) fn track(&self) {
        self.server.with(|_| ());
        self.username.with(|_| ());
        self.password.with(|_| ());
    }

    pub(crate) fn finish(&self, result: Result<(), String>) {
        self.status.set(match result {
            Ok(()) => Status::Idle,
            Err(error) => Status::Failed(error),
        });
    }

    /// The first field with nothing in it, where typing starts.
    fn first_empty(&self) -> Option<SignInField> {
        [
            (self.server, SignInField::Server),
            (self.username, SignInField::Username),
            (self.password, SignInField::Password),
        ]
        .into_iter()
        .find(|(value, _)| value.with_untracked(String::is_empty))
        .map(|(_, which)| which)
    }

    /// The profile the fields name, as typed.
    pub(crate) fn profile(&self) -> config::Profile {
        config::Profile::new(
            self.server.with(|server| server.trim().to_owned()),
            self.username.with(|username| username.trim().to_owned()),
        )
    }
}

/// Where each field is, to give it the focus.
#[derive(Clone, Copy)]
struct Places {
    server: NodeRef,
    username: NodeRef,
    password: NodeRef,
}

impl Places {
    fn new() -> Self {
        Self {
            server: node_ref(),
            username: node_ref(),
            password: node_ref(),
        }
    }

    fn of(&self, field: SignInField) -> NodeRef {
        match field {
            SignInField::Server => self.server,
            SignInField::Username => self.username,
            SignInField::Password => self.password,
        }
    }
}

/// The fields, a failure, and a row of `actions` before the button that signs in. The first empty
/// field takes the focus, and so does one that is wrong.
pub(crate) fn form(hiphile: Hiphile, fields: Fields, actions: Vec<Div>) -> Div {
    let places = Places::new();
    let shown = follow(hiphile.subscribe());
    let signing_in = move || shown.with(|shown| shown.is_signing_in());
    let scope = Scope::here();
    let attempt: Rc<RefCell<Option<Pending>>> = Rc::default();
    let submit: Rc<dyn Fn()> = Rc::new(move || {
        let tried = hiphile.sign_in(
            &fields.server.get_untracked(),
            &fields.username.get_untracked(),
            &fields.password.get_untracked(),
        );
        match tried {
            Ok(task) => {
                fields.status.set(Status::Idle);
                *attempt.borrow_mut() =
                    Some(scope.on_ui(task, move |result| fields.finish(result)));
            }
            Err(SignInRefused::Invalid(invalid)) => {
                fields.status.set(Status::Invalid(
                    invalid.field(),
                    invalid.message().to_owned(),
                ));
                focus_field(places.of(invalid.field()));
            }
            Err(SignInRefused::SigningIn) => {}
        }
    });

    let status = fields.status;
    let first = fields.first_empty();
    let placed = move |which: SignInField, field: Div| {
        let field = field.node_ref(places.of(which));
        if first == Some(which) {
            focused_field(field)
        } else {
            field
        }
    };
    let inputs = div()
        .flex()
        .flex_col()
        .gap(4.0_f32)
        .child(placed(
            SignInField::Server,
            field(
                fields.server,
                "Server",
                SignInField::Server,
                status,
                submit.clone(),
                signing_in,
            ),
        ))
        .child(placed(
            SignInField::Username,
            field(
                fields.username,
                "Username",
                SignInField::Username,
                status,
                submit.clone(),
                signing_in,
            ),
        ))
        .child(password_field(
            fields.password,
            status,
            submit.clone(),
            signing_in,
            (places.password, first == Some(SignInField::Password)),
        ));
    let failure = show(
        move || matches!(status.get(), Status::Failed(_)),
        move || {
            error_text(move || match status.get() {
                Status::Failed(error) => error,
                _ => String::new(),
            })
        },
    );
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(8.0_f32)
        .children(actions)
        .child(show(signing_in, || indeterminate_circular_progress().ml(8)))
        .child(
            styled_button("Sign In", ButtonStyle::filled().disabled_when(signing_in))
                .ml(8)
                .on_click(move |_| submit()),
        );
    div()
        .flex()
        .flex_col()
        .gap(24.0_f32)
        .child(inputs)
        .child(failure)
        .child(row)
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

/// Its look is chosen when it is made, so showing what is typed makes it anew. It stays at `at`,
/// and takes the focus when it is first made if `focused` says so.
fn password_field(
    value: Signal<String>,
    status: Signal<Status>,
    submit: Rc<dyn Fn()>,
    signing_in: impl Fn() -> bool + Clone + 'static,
    (at, focused): (NodeRef, bool),
) -> View {
    let visible = signal(false);
    let made = Cell::new(false);
    // The button that goes is not told it lost the focus, so the one made anew takes it back
    let toggle_focused = Rc::new(Cell::new(false));
    dyn_view(move || {
        let shown = visible.get();
        let (submit, signing_in) = (submit.clone(), signing_in.clone());
        let toggle_focused = toggle_focused.clone();
        let again = made.replace(true);
        let field = untrack(move || {
            let mut toggle =
                icon_toggle_button(icons::VISIBILITY, visible, IconButtonStyle::standard())
                    .access(move |a| a.label("Show Password"));
            if toggle_focused.get() {
                toggle = focused_once_laid_out(toggle, |_, id| Some(id));
            }
            let toggle = toggle.on_focus_change(move |focused| toggle_focused.set(focused));
            let mut style = TextFieldStyle::new()
                .label("Password")
                .on_submit(move |_| submit())
                .disabled_when(signing_in)
                .trailing(toggle);
            if !shown {
                style = style.password();
            }
            outlined_text_field(value, with_error(style, SignInField::Password, status))
                .w(WIDTH)
                .node_ref(at)
        });
        if focused && !again {
            focused_field(field)
        } else {
            field
        }
    })
}

fn with_error(style: TextFieldStyle, which: SignInField, status: Signal<Status>) -> TextFieldStyle {
    let error = move || match status.get() {
        Status::Invalid(field, message) if field == which => Some(message),
        _ => None,
    };
    style
        .error_when(move || error().is_some())
        .supporting(move || error().unwrap_or_default())
}
