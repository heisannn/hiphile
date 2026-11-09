//! Stands in for `NumberInput` of GPUI Kit 0.9, whose frame and editor both track the focus of
//! one `InputState`. While assistive technology is connected, both report the focus in the same
//! frame, and GPUI panics on that in debug builds.

use gpui_kit::base::{Button, Decrement, Increment, StepAction, step_value};
use gpui_kit::component::input::{Input, InputState, MaskPattern};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::{
    App, Context, Entity, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    TextAlign, Window, div,
};
use hiphile_core::settings::NumberRange;

const CONTEXT: &str = "SettingsNumberField";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", Increment, Some(CONTEXT)),
        KeyBinding::new("down", Decrement, Some(CONTEXT)),
    ]);
}

/// An input that holds numbers to `range` when it loses focus.
pub(crate) fn number_state(
    range: NumberRange,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> InputState {
    InputState::new(window, cx)
        .mask_pattern(MaskPattern::Number {
            separator: None,
            fraction: range.fraction(),
        })
        .min(range.min())
        .max(range.max())
}

/// Moves the number in `state` one step and moves focus there, reporting the change as typing
/// would.
fn step(
    range: NumberRange,
    state: &Entity<InputState>,
    action: StepAction,
    window: &mut Window,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        state.focus(window, cx);
        let value = step_value(
            &state.value(),
            action,
            range.step(),
            Some(range.min()),
            Some(range.max()),
        );
        if let Some(value) = value {
            state.replace_all(value, window, cx);
        }
    });
}

/// Shows and edits the number in `state`, which [`number_state`] made.
#[derive(IntoElement)]
pub(crate) struct NumberField {
    state: Entity<InputState>,
    range: NumberRange,
    name: SharedString,
    unit: SharedString,
}

impl NumberField {
    /// `name` says what the number is, for screen readers. `unit` is shown after the number.
    pub(crate) fn new(
        state: &Entity<InputState>,
        range: NumberRange,
        name: impl Into<SharedString>,
        unit: impl Into<SharedString>,
    ) -> Self {
        Self {
            state: state.clone(),
            range,
            name: name.into(),
            unit: unit.into(),
        }
    }

    /// A button that takes one step. It takes no focus, so that the up and down keys go on
    /// working after it.
    fn step_button(&self, action: StepAction, cx: &App) -> Button {
        let (id, name, icon) = match action {
            StepAction::Decrement => ("number-field-decrease", "Decrease", IconName::Minus),
            StepAction::Increment => ("number-field-increase", "Increase", IconName::Plus),
        };
        let hover: Hsla = cx.theme().input.opacity(0.4);
        let active: Hsla = cx.theme().input.opacity(0.6);
        let (state, range) = (self.state.clone(), self.range);
        Button::new((id, self.state.entity_id()))
            .focusable(false)
            .accessibility_label(name)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size_5()
            .rounded(cx.theme().radius)
            .text_color(cx.theme().secondary_foreground)
            .hover(move |this| this.bg(hover))
            .active(move |this| this.bg(active))
            .child(Icon::new(icon).xsmall())
            .on_click(move |_, window, cx| step(range, &state, action, window, cx))
    }
}

impl RenderOnce for NumberField {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let decrease = self.step_button(StepAction::Decrement, cx);
        let increase = self.step_button(StepAction::Increment, cx);
        let input = Input::new(&self.state)
            .aria_label(self.name)
            .text_align(TextAlign::Center)
            .prefix(decrease)
            .suffix(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.unit),
                    )
                    .child(increase),
            );
        let (up, down) = (self.state.clone(), self.state);
        let range = self.range;
        div()
            .flex_none()
            .w_40()
            .key_context(CONTEXT)
            .on_action(move |_: &Increment, window, cx| {
                step(range, &up, StepAction::Increment, window, cx)
            })
            .on_action(move |_: &Decrement, window, cx| {
                step(range, &down, StepAction::Decrement, window, cx)
            })
            .child(input)
    }
}
