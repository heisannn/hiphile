//! The plain controls of the rows: switches, radio buttons and the button that resets a setting.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{
    RadioStyle, SwitchStyle, icon_button, styled_radio_button, styled_switch, tooltip,
};
use anti_webui_md3e::icons;
use anti_webui_md3e::typography::TextStyle;

use crate::group::When;

pub(crate) fn never() -> When {
    Rc::new(|| false)
}

pub(crate) fn when(f: impl Fn() -> bool + 'static) -> When {
    Rc::new(f)
}

/// A switch named `name` for assistive technology, which the row's headline says on the screen.
pub(crate) fn switch(
    name: &'static str,
    on: impl Fn() -> bool + 'static,
    set: impl Fn(bool) + 'static,
    disabled: When,
) -> Div {
    styled_switch(
        Binding::new(on, set),
        SwitchStyle::new().disabled_when(move || disabled()),
    )
    .access(move |a| a.label(name))
}

/// [`switch`] for the setting that `on` reads from what `shown` holds and `set` changes.
pub(crate) fn switch_of<S: Clone + 'static, T: 'static>(
    name: &'static str,
    settings: &S,
    shown: Signal<T>,
    on: fn(&T) -> bool,
    set: fn(&S, bool),
    disabled: When,
) -> Div {
    let settings = settings.clone();
    switch(
        name,
        move || shown.with(on),
        move |is_on| set(&settings, is_on),
        disabled,
    )
}

/// [`radios`] for `labels`, the one that `selected` reads from what `shown` holds chosen.
pub(crate) fn radios_of<S: Clone + 'static, T: 'static>(
    name: &'static str,
    labels: &[&str],
    settings: &S,
    shown: Signal<T>,
    selected: fn(&T) -> usize,
    select: fn(&S, usize),
    disabled: When,
) -> Div {
    let settings = settings.clone();
    radios(
        name,
        labels.iter().map(|label| (*label).to_owned()).collect(),
        move || shown.with(selected),
        move |ix| select(&settings, ix),
        disabled,
    )
}

/// A radio button with its label for each of `labels`, the one at `selected` chosen. Choosing
/// another calls `select` with where it is.
pub(crate) fn radios(
    name: &'static str,
    labels: Vec<String>,
    selected: impl Fn() -> usize + 'static,
    select: impl Fn(usize) + 'static,
    disabled: When,
) -> Div {
    let selected = memo(selected);
    let chosen = Binding::new(move || selected.get(), select);
    let options = labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            let (off, faded, still) = (disabled.clone(), disabled.clone(), disabled.clone());
            let told = label.clone();
            div()
                .flex()
                .flex_row()
                .items_center()
                .pr(12.0_f32)
                .child(
                    styled_radio_button(chosen, i, RadioStyle::new().disabled_when(move || off()))
                        .access(move |a| a.label(told.clone())),
                )
                .child(
                    TextStyle::BODY_MEDIUM
                        .apply(div().child(text(label)))
                        .select_none()
                        .dyn_style(move |s| {
                            let c = color_scheme();
                            if faded() {
                                s.text_color(c.on_surface.with_alpha(0.38))
                            } else {
                                s.text_color(c.on_surface).cursor(CursorIcon::Pointer)
                            }
                        })
                        .on_click(move |_| {
                            if !still() {
                                chosen.set(i);
                            }
                        }),
                )
        })
        .collect::<Vec<_>>();
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .access(move |a| a.role(Role::RadioGroup).label(name))
        .children(options)
}

/// Puts the setting `name` back as it was at first, shown while it is not.
pub(crate) fn reset_button(
    name: impl Into<String>,
    resettable: impl Fn() -> bool + 'static,
    reset: impl Fn() + 'static,
) -> Show {
    let label: Rc<str> = format!("Reset {}", name.into()).into();
    let reset: Rc<dyn Fn()> = Rc::new(reset);
    show(resettable, move || {
        let (reset, label) = (reset.clone(), label.clone());
        tooltip(
            icon_button(icons::RESTART_ALT)
                .access(move |a| a.label(&*label))
                .on_click(move |_| reset()),
            "Reset",
        )
    })
}

/// `control` with the button that resets it before it.
pub(crate) fn resettable(control: Div, reset: Show) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(4.0_f32)
        .child(reset)
        .child(control)
}
