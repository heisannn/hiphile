use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::reactive::effect_with;
use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::Hiphile;
use next_host::{Pending, on_ui};
use next_ui::type_scale::{muted_text, styled_text};

use crate::form::{Fields, WIDTH, form};

/// Signs in to `hiphile`, which then opens the workspace. Shown, it signs in again as whoever did
/// last, as the core allows, until the user starts typing.
pub fn sign_in(hiphile: Hiphile) -> Div {
    let fields = Fields::new(hiphile.last_profile().as_ref(), None);
    let resuming: Rc<RefCell<Option<Pending>>> = Rc::new(RefCell::new(
        hiphile
            .resume()
            .map(|task| on_ui(task, move |result| fields.finish(result))),
    ));
    effect_with(move |started: Option<()>| {
        fields.track();
        if started.is_some()
            && let Some(resume) = resuming.borrow_mut().take()
        {
            resume.cancel();
        }
    });

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
                            styled_text(TextStyle::HEADLINE_SMALL, "Sign In")
                                .access(|a| a.role(Role::Heading).level(1)),
                        )
                        .child(muted_text(
                            TextStyle::BODY_MEDIUM,
                            "Connect to a Navidrome or other Subsonic server.",
                        )),
                )
                .child(form(hiphile, fields, Vec::new())),
        )
}

#[cfg(test)]
mod tests;
