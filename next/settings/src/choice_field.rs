use std::cell::Cell;
use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{MenuItem, MenuStyle, menu_item, sized_icon, styled_menu};
use anti_webui_md3e::icons;
use anti_webui_md3e::motion::fast_spatial;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::settings::Choice;

use crate::group::When;

const WIDTH: i32 = 280;
const HEIGHT: i32 = 56;

/// The exposed dropdown menu of Material 3 for one of `choices`, the one whose value is `value`.
/// Choosing another calls `set` with its value.
///
/// `anti_webui_md3e::components::dropdown_field` takes a fixed list of labels, opens only on a
/// press, and puts the focus on a text input that is named only by a label shown in it. This one
/// has choices that change, keys that open it, and is one stop for Tab that says what it is.
pub(crate) fn choice_field(
    name: &'static str,
    choices: impl Fn() -> Vec<Choice> + 'static,
    value: impl Fn() -> String + 'static,
    set: impl Fn(&str) + 'static,
    disabled: When,
) -> Div {
    let choices = memo(choices);
    let value = memo(value);
    let label = memo(move || {
        let value = value.get();
        choices.with(|choices| {
            choices
                .iter()
                .find(|choice| choice.value() == value)
                .map(|choice| choice.label().to_owned())
                .unwrap_or_default()
        })
    });
    let open = signal(false);
    let (focused, hovered) = (signal(false), signal(false));
    // A press on the field while the menu is open first closes it, as a press outside it does
    let was_open = Rc::new(Cell::new(false));
    let turn = follow(
        move || if open.get() { 180.0f32 } else { 0.0 },
        fast_spatial(),
    );
    let anchor = node_ref();
    let set: Rc<dyn Fn(&str)> = Rc::new(set);
    let menu = dyn_view(move || {
        let choices = choices.get();
        let set = set.clone();
        untrack(move || {
            let items = choices
                .into_iter()
                .map(|choice| {
                    let (mine, chosen) = (choice.value().to_owned(), choice.value().to_owned());
                    let set = set.clone();
                    menu_item(choice.label())
                        .selected_when(move || value.with(|value| *value == mine))
                        .on_click(move || {
                            open.set(false);
                            set(&chosen);
                            anchor.focus();
                        })
                })
                .collect::<Vec<MenuItem>>();
            styled_menu(
                move || open.get(),
                move || open.set(false),
                vec![items],
                MenuStyle::new().anchor(anchor).match_anchor_width(),
            )
        })
    });
    let (look_off, text_off, arrow_off) = (disabled.clone(), disabled.clone(), disabled.clone());
    let shown = TextStyle::BODY_LARGE
        .apply(div().flex_1().min_w(0).child(text(move || label.get())))
        .dyn_style(move |s| {
            let c = color_scheme();
            s.text_color(if text_off() {
                c.on_surface.with_alpha(0.38)
            } else {
                c.on_surface
            })
        });
    let arrow = sized_icon(icons::ARROW_DROP_DOWN, 24.0).dyn_style(move |s| {
        let c = color_scheme();
        s.rotate(turn.get()).text_color(if arrow_off() {
            c.on_surface.with_alpha(0.38)
        } else {
            c.on_surface_variant
        })
    });
    let pressed = was_open.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(4.0_f32)
        .w(WIDTH)
        .h(HEIGHT)
        .rounded(4.0_f32)
        .select_none()
        .focusable()
        .node_ref(anchor)
        .dyn_style(move |s| {
            let c = color_scheme();
            let (color, thick) = if look_off() {
                (c.on_surface.with_alpha(0.12), false)
            } else if focused.get() || open.get() {
                (c.primary, true)
            } else if hovered.get() {
                (c.on_surface, false)
            } else {
                (c.outline, false)
            };
            // The line thickens inwards, so that what is in the field stays put
            let (line, room) = if thick {
                (2.0_f32, 15.0_f32)
            } else {
                (1.0, 16.0)
            };
            let s = s.border(line).border_color(color).pl(room).pr(room - 4.0);
            if look_off() {
                s
            } else {
                s.cursor(CursorIcon::Pointer)
            }
        })
        .disabled(move || disabled())
        .access(move |a| {
            a.role(Role::ComboBox)
                .label(name)
                .value_text(label.get())
                .expanded(open.get())
        })
        .on_focus_change(move |on| focused.set(on))
        .on_hover(move |on| hovered.set(on))
        .on_pointer_down(move |_| pressed.set(open.get_untracked()))
        .on_click(move |_| {
            if !was_open.replace(false) {
                open.set(true);
            }
        })
        .on_key_down(move |e| {
            let opens = matches!(
                e.key,
                Key::Named(
                    NamedKey::Enter | NamedKey::Space | NamedKey::ArrowDown | NamedKey::ArrowUp
                )
            ) || matches!(&e.key, Key::Character(c) if c == " ");
            if opens {
                e.prevent_default();
                open.set(true);
            }
        })
        .child(shown)
        .child(arrow)
        .child(menu)
}

/// [`choice_field`] for the setting that `options` and `value` read from what `shown` holds and
/// `set` changes.
pub(crate) fn choice_of<S: Clone + 'static, T: 'static>(
    name: &'static str,
    settings: &S,
    shown: Signal<T>,
    options: fn(&T) -> Vec<Choice>,
    value: fn(&T) -> String,
    set: fn(&S, &str),
    disabled: When,
) -> Div {
    let settings = settings.clone();
    choice_field(
        name,
        move || shown.with(options),
        move || shown.with(value),
        move |chosen| set(&settings, chosen),
        disabled,
    )
}

#[cfg(test)]
mod tests;
