use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};

/// Commits with its button, or with Enter by way of the dialog. What that does is
/// up to `on_submit`, which closes the dialog when it is done or reports back
/// through [`Self::set_failed`].
pub(crate) struct NameForm {
    input: Entity<InputState>,
    commit: SharedString,
    /// What else the commit affects, when the dialog can't show it otherwise.
    scope: Option<SharedString>,
    submitting: bool,
    error: Option<SharedString>,
    on_submit: OnSubmit,
    _input_events: Subscription,
}

type OnSubmit = Rc<dyn Fn(String, Entity<NameForm>, &mut Window, &mut App)>;

impl NameForm {
    pub(crate) fn new(
        name: &str,
        commit: &'static str,
        scope: Option<SharedString>,
        on_submit: impl Fn(String, Entity<NameForm>, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(name.to_owned(), window, cx);
            input
        });
        let input_events = cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.error = None;
                cx.notify();
            }
        });
        Self {
            input,
            commit: commit.into(),
            scope,
            submitting: false,
            error: None,
            on_submit: Rc::new(on_submit),
            _input_events: input_events,
        }
    }

    pub(crate) fn focus_input(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// The name as it would be saved.
    fn name(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_owned()
    }

    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name(cx);
        if name.is_empty() || self.submitting {
            return;
        }
        self.submitting = true;
        self.error = None;
        cx.notify();
        // Runs while this form is being updated, so it must not update the form
        // itself until later.
        let on_submit = self.on_submit.clone();
        on_submit(name, cx.entity(), window, cx);
    }

    pub(crate) fn is_submitting(&self) -> bool {
        self.submitting
    }

    pub(crate) fn set_failed(&mut self, error: SharedString, cx: &mut Context<Self>) {
        self.submitting = false;
        self.error = Some(error);
        cx.notify();
    }
}

impl Render for NameForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.name(cx).is_empty();
        v_flex()
            .gap_3()
            .when_some(self.scope.clone(), |this, scope| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(scope),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Name"))
                    .child(Input::new(&self.input))
                    .when_some(self.error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    }),
            )
            .child(
                h_flex()
                    .pt_1()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("cancel-name")
                            .label("Cancel")
                            .disabled(self.submitting)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("commit-name")
                            .primary()
                            .label(self.commit.clone())
                            .disabled(empty || self.submitting)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
impl NameForm {
    pub(crate) fn set_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;

    /// A form opened on `name` that counts its submissions.
    fn form<'a>(
        name: &'static str,
        cx: &'a mut TestAppContext,
    ) -> (Entity<NameForm>, Rc<Cell<usize>>, &'a mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let submissions = Rc::new(Cell::new(0));
        let counted = submissions.clone();
        let (handle, form) = cx
            .update(|cx| {
                gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        NameForm::new(
                            name,
                            "Create",
                            None,
                            move |_, _, _, _| counted.set(counted.get() + 1),
                            window,
                            cx,
                        )
                    })
                })
            })
            .unwrap();
        (
            form,
            submissions,
            VisualTestContext::from_window(handle, cx).into_mut(),
        )
    }

    #[gpui_kit::test]
    fn a_second_commit_waits_for_the_first_to_fail(cx: &mut TestAppContext) {
        let (form, submissions, cx) = form("Road Trip", cx);
        let submit = |cx: &mut VisualTestContext| {
            cx.update(|window, cx| form.update(cx, |form, cx| form.submit(window, cx)));
        };

        submit(cx);
        submit(cx);
        assert_eq!(submissions.get(), 1);

        cx.update(|_, cx| {
            form.update(cx, |form, cx| form.set_failed("Offline".into(), cx));
        });
        submit(cx);
        assert_eq!(submissions.get(), 2);
        cx.update(|_, cx| assert_eq!(form.read(cx).error(), None));
    }

    #[gpui_kit::test]
    fn a_blank_name_commits_nothing(cx: &mut TestAppContext) {
        let (form, submissions, cx) = form("   ", cx);
        cx.update(|window, cx| form.update(cx, |form, cx| form.submit(window, cx)));
        assert_eq!(submissions.get(), 0);
    }
}
