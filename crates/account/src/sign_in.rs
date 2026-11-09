use gpui_kit::component::{ActiveTheme as _, StyledExt as _, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Task, Window, div,
};
use hiphile_core::Hiphile;

use crate::sign_in_form::{Edited, SignInForm};

/// Signs in to `hiphile`, which then opens the workspace. Made, it signs in again as whoever did
/// last, as the core allows, until the user starts typing.
pub struct SignIn {
    form: Entity<SignInForm>,
    _resume: Task<()>,
    _edits: Subscription,
}

impl SignIn {
    pub fn new(hiphile: Hiphile, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let profile = hiphile.last_profile();
        let form =
            cx.new(|cx| SignInForm::new(hiphile.clone(), profile.as_ref(), false, window, cx));
        form.update(cx, |form, cx| form.focus_first_empty(window, cx));
        let edits = cx.subscribe(&form, |this, _, _: &Edited, _| {
            this._resume = Task::ready(());
        });
        let resume = match hiphile.resume() {
            Some(resume) => cx.spawn(async move |this, cx| {
                let result = resume.await;
                this.update(cx, |this, cx| {
                    this.form.update(cx, |form, cx| form.finish(result, cx));
                })
                .ok();
            }),
            None => Task::ready(()),
        };
        Self {
            form,
            _resume: resume,
            _edits: edits,
        }
    }

    /// Puts `profile` in the fields for its password to be typed, with `failure` if signing in
    /// with the saved one went wrong.
    pub fn show_profile(
        &mut self,
        profile: &config::Profile,
        failure: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._resume = Task::ready(());
        self.form.update(cx, |form, cx| {
            form.show_profile(profile, failure, window, cx);
        });
    }
}

impl Render for SignIn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(self.form.clone()),
            )
    }
}

#[cfg(test)]
mod tests;
