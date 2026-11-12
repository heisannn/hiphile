use std::cell::Cell;
use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{IconButtonStyle, styled_icon_button};
use anti_webui_md3e::icons;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::settings::{
    AudioSettings, AudioSnapshot, CacheKind, CacheLimit, CacheLimitSnapshot, NumberRange,
    R8brainNumber, R8brainSetting,
};
use next_ui::type_scale::error_text;

use crate::controls::{reset_button, resettable};
use crate::group::When;

const WIDTH: i32 = 216;
const HEIGHT: i32 = 56;

/// What a number field edits, as its setting words it.
pub(crate) struct Number {
    /// What the number is, for assistive technology: the row's headline says it on the screen.
    pub(crate) name: &'static str,
    pub(crate) unit: &'static str,
    pub(crate) range: NumberRange,
    /// What an empty field stands for, where it stands for something.
    pub(crate) placeholder: Option<&'static str>,
    /// What the field shows now, read from the setting itself rather than a signal, which only
    /// hears of a change later.
    pub(crate) now: Rc<dyn Fn() -> String>,
    /// What the field follows while it is shown, for a setting that something else can change
    /// too. Without it, the field shows the setting again only once it is left.
    pub(crate) follows: Option<Rc<dyn Fn() -> String>>,
    /// Told of each change typed.
    pub(crate) typed: Rc<dyn Fn(&str)>,
    /// Told when Enter is pressed or the field is left, and as it goes while what was typed in it
    /// is not left yet.
    pub(crate) left: Rc<dyn Fn()>,
    pub(crate) reset: Option<Reset>,
}

/// Puts the number back as it was at first.
pub(crate) struct Reset {
    pub(crate) resettable: Rc<dyn Fn() -> bool>,
    pub(crate) reset: Rc<dyn Fn()>,
}

/// One of the r8brain parameters that is typed as a number: what was typed until it is left, or
/// else the number. A preset reaches it while it is typed into.
pub(crate) fn r8brain_number_field(
    audio: &AudioSettings,
    shown: Signal<AudioSnapshot>,
    number: R8brainNumber,
    disabled: When,
) -> Div {
    let setting = R8brainSetting::Number(number);
    let (now, typed, left, reset) = (audio.clone(), audio.clone(), audio.clone(), audio.clone());
    number_field(
        Number {
            name: number.name(),
            unit: number.unit(),
            range: number.range(),
            placeholder: number.placeholder(),
            now: Rc::new(move || now.snapshot().r8brain_text(number)),
            follows: Some(Rc::new(move || {
                shown.with(|shown| shown.r8brain_text(number))
            })),
            typed: Rc::new(move |text| typed.type_r8brain(number, text)),
            left: Rc::new(move || left.commit_r8brain(number)),
            reset: Some(Reset {
                resettable: Rc::new(move || {
                    shown.with(|shown| shown.is_r8brain_resettable(setting))
                }),
                reset: Rc::new(move || reset.reset_r8brain(setting)),
            }),
        },
        disabled,
    )
}

/// The most a cache keeps, in GiB. Only this field changes it, so it does not follow the setting,
/// which cannot replace what is being typed; leaving it shows the limit as it counts.
pub(crate) fn cache_limit_field(limit: &CacheLimit, shown: Signal<CacheLimitSnapshot>) -> Div {
    let name = match limit.snapshot().kind() {
        CacheKind::Songs => "Song cache limit",
        CacheKind::Covers => "Cover cache limit",
    };
    let (now, typed, reset) = (limit.clone(), limit.clone(), limit.clone());
    number_field(
        Number {
            name,
            unit: "GiB",
            range: CacheLimit::range(),
            placeholder: None,
            now: Rc::new(move || now.snapshot().gib_text()),
            follows: None,
            typed: Rc::new(move |text| typed.set_typed(text)),
            left: Rc::new(|| {}),
            reset: Some(Reset {
                resettable: Rc::new(move || shown.with(|shown| !shown.is_default())),
                reset: Rc::new(move || reset.reset()),
            }),
        },
        Rc::new(|| false),
    )
}

/// What the parts of a number field share.
#[derive(Clone)]
struct Parts {
    name: &'static str,
    range: NumberRange,
    /// What the field holds.
    value: Signal<String>,
    focused: Signal<bool>,
    /// Whether what it holds is beyond the range or no number.
    wrong: Memo<bool>,
    input: NodeRef,
    /// Steps up with `true` and down with `false`.
    step: Rc<dyn Fn(bool)>,
    disabled: When,
}

/// A field for `number`, with buttons that step it and Up and Down that do too. What is typed
/// beyond its range, or is no number, is said to be so until the field is left, when the setting
/// gives the number it took.
pub(crate) fn number_field(number: Number, disabled: When) -> Div {
    let Number {
        name,
        unit,
        range,
        placeholder,
        now,
        follows,
        typed,
        left,
        reset,
    } = number;
    let value = signal(now());
    if let Some(follows) = follows {
        effect(move || {
            let text = follows();
            untrack(|| value.set_if_changed(text));
        });
    }
    let show_now = {
        let now = now.clone();
        Rc::new(move || value.set_if_changed(now()))
    };
    // Typed into and not left yet. A field can go without losing the focus, as when another page
    // is chosen, and what was typed then counts as it goes
    let is_typed_into = Rc::new(Cell::new(false));
    {
        let (is_typed_into, left) = (is_typed_into.clone(), left.clone());
        on_cleanup(move || {
            if is_typed_into.get() {
                left();
            }
        });
    }
    let typed: Rc<dyn Fn(&str)> = {
        let is_typed_into = is_typed_into.clone();
        Rc::new(move |text| {
            is_typed_into.set(true);
            typed(text);
        })
    };
    let leave = {
        let show_now = show_now.clone();
        Rc::new(move || {
            is_typed_into.set(false);
            left();
            show_now();
        })
    };
    let step = {
        let typed = typed.clone();
        Rc::new(move |up: bool| {
            let from = value
                .with_untracked(|text| text.trim().parse::<f64>().ok())
                .or_else(|| now().trim().parse().ok())
                .unwrap_or(range.min());
            let by = if up { range.step() } else { -range.step() };
            let to = format_number((from + by).clamp(range.min(), range.max()), range);
            value.set(to.clone());
            typed(&to);
        })
    };
    let wrong = memo(move || {
        value.with(|text| {
            let text = text.trim();
            match text.parse::<f64>() {
                Ok(number) => !(range.min()..=range.max()).contains(&number),
                Err(_) => !text.is_empty(),
            }
        })
    });
    let parts = Parts {
        name,
        range,
        value,
        focused: signal(false),
        wrong,
        input: node_ref(),
        step,
        disabled,
    };
    let field = input(&parts, placeholder.unwrap_or_default(), typed, leave);
    let off = parts.disabled.clone();
    let column = div()
        .flex()
        .flex_col()
        .disabled(move || off())
        .child(frame(&parts, unit, field))
        .child(out_of_range(&parts));
    match reset {
        Some(Reset {
            resettable: can,
            reset,
        }) => resettable(
            column,
            reset_button(
                name,
                move || can(),
                move || {
                    reset();
                    show_now();
                },
            ),
        ),
        None => column,
    }
}

fn input(
    parts: &Parts,
    placeholder: &'static str,
    typed: Rc<dyn Fn(&str)>,
    leave: Rc<dyn Fn()>,
) -> TextInput {
    let Parts {
        name,
        range,
        value,
        focused,
        wrong,
        ..
    } = parts.clone();
    let (off, step) = (parts.disabled.clone(), parts.step.clone());
    text_input()
        .value(value)
        .placeholder(placeholder)
        .access(move |a| a.label(name))
        .node_ref(parts.input)
        .flex_grow(1.0_f32)
        .flex_basis(0.0)
        .min_w(0)
        .text_align(TextAlign::Center)
        .bg(Color::TRANSPARENT)
        .dyn_style(move |s| {
            let c = color_scheme();
            let s = s
                .text_color(if off() {
                    c.on_surface.with_alpha(0.38)
                } else {
                    c.on_surface
                })
                .caret_color(if wrong.get() { c.error } else { c.primary })
                .selection_color(c.primary.with_alpha(0.4));
            if off() { s.pointer_events_none() } else { s }
        })
        .on_input(move |text| {
            let kept = keep_number(text, range);
            if kept != text {
                value.set(kept.clone());
            }
            typed(&kept);
        })
        .on_submit({
            let leave = leave.clone();
            move |_| leave()
        })
        .on_focus_change(move |on| {
            focused.set(on);
            if !on {
                leave();
            }
        })
        .on_key_down(move |e| {
            let up = match e.key {
                Key::Named(NamedKey::ArrowUp) => true,
                Key::Named(NamedKey::ArrowDown) => false,
                _ => return,
            };
            e.prevent_default();
            step(up);
        })
}

/// The outline around `field`, with the buttons that step it and the unit.
fn frame(parts: &Parts, unit: &'static str, field: TextInput) -> Div {
    let unit = (!unit.is_empty()).then(|| {
        TextStyle::BODY_MEDIUM
            .apply(div().flex_none().child(text(unit)))
            .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
    });
    let (off, wrong, focused) = (parts.disabled.clone(), parts.wrong, parts.focused);
    div()
        .flex()
        .flex_row()
        .items_center()
        .w(WIDTH)
        .h(HEIGHT)
        .rounded(4.0_f32)
        .dyn_style(move |s| {
            let c = color_scheme();
            let (color, thick) = if off() {
                (c.on_surface.with_alpha(0.12), false)
            } else if wrong.get() {
                (c.error, focused.get())
            } else if focused.get() {
                (c.primary, true)
            } else {
                (c.outline, false)
            };
            // The line thickens inwards, so that what is in the field stays put
            let (line, room) = if thick {
                (2.0_f32, 0.0_f32)
            } else {
                (1.0, 1.0)
            };
            s.border(line).border_color(color).p(room)
        })
        .child(stepper(parts, false))
        .child(TextStyle::BODY_LARGE.apply(field))
        .children(unit)
        .child(stepper(parts, true))
}

fn stepper(parts: &Parts, up: bool) -> Div {
    let (step, off, input) = (parts.step.clone(), parts.disabled.clone(), parts.input);
    let (symbol, label) = if up {
        (icons::ADD, format!("Increase {}", parts.name))
    } else {
        (icons::REMOVE, format!("Decrease {}", parts.name))
    };
    styled_icon_button(
        symbol,
        IconButtonStyle::standard().disabled_when(move || off()),
    )
    .access(move |a| a.label(label.clone()))
    .on_click(move |_| {
        step(up);
        input.focus();
    })
}

/// Says what the field takes while what it holds is not that.
fn out_of_range(parts: &Parts) -> Show {
    let (wrong, range) = (parts.wrong, parts.range);
    show(
        move || wrong.get(),
        move || {
            error_text(format!(
                "Enter {} to {}.",
                format_number(range.min(), range),
                format_number(range.max(), range)
            ))
            .px(16.0_f32)
            .pt(4.0_f32)
        },
    )
}

/// `text` without what no number of `range` has.
fn keep_number(text: &str, range: NumberRange) -> String {
    let point = range.fraction() != Some(0);
    let minus = range.min() < 0.0;
    text.chars()
        .filter(|c| c.is_ascii_digit() || (point && *c == '.') || (minus && *c == '-'))
        .collect()
}

/// `number` as a field shows it: whole where the range is, and without the noise of binary
/// fractions.
fn format_number(number: f64, range: NumberRange) -> String {
    if range.fraction() == Some(0) {
        return format!("{}", number.round() as i64);
    }
    let rounded = (number * 1e9).round() / 1e9;
    format!("{}", rounded + 0.0)
}

#[cfg(test)]
mod tests;
