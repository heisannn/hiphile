use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, AppContext as _, Context, Entity, IntoElement, Render, Subscription, Window};

use crate::cache_limit::{CacheLimit, MAX_GIB};
use crate::number_field::{NumberField, NumberRange};

const RANGE: NumberRange = NumberRange {
    min: 0.,
    max: MAX_GIB,
    step: 1.,
    fraction: None,
};

/// Only this field changes the limit, so what it shows never has to follow the settings, and
/// they cannot replace what is being typed.
pub(crate) struct CacheLimitField {
    settings: Entity<CacheLimit>,
    input: Entity<InputState>,
    _input_events: Subscription,
}

impl CacheLimitField {
    pub(crate) fn new(
        settings: Entity<CacheLimit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = settings.read(cx).gib().to_string();
        let input = cx.new(|cx| RANGE.state(window, cx).default_value(text));
        Self {
            settings,
            _input_events: cx.subscribe_in(&input, window, Self::on_input_event),
            input,
        }
    }

    fn on_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = input.read(cx).value();
        match event {
            InputEvent::Change => {
                self.settings
                    .update(cx, |settings, cx| match RANGE.parse(&value) {
                        Some(gib) => settings.set_gib(gib, cx),
                        None => settings.revert(cx),
                    })
            }
            // Leaving the field shows the number as it counts: text that is no number gives
            // way to the limit, and `007` becomes `7`.
            InputEvent::Blur => {
                let text = self.settings.read(cx).gib().to_string();
                if value.as_ref() != text {
                    input.update(cx, |input, cx| input.set_value(text, window, cx));
                }
            }
            _ => {}
        }
    }

    pub(crate) fn is_resettable(&self, cx: &App) -> bool {
        let settings = self.settings.read(cx);
        settings.gib() != settings.kind().default_gib()
    }

    /// Goes back to the default limit, reporting the change as typing would.
    pub(crate) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let default = self.settings.read(cx).kind().default_gib();
        self.input.update(cx, |input, cx| {
            input.replace_all(default.to_string(), window, cx)
        });
    }
}

impl Render for CacheLimitField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.settings.read(cx).kind().field_name();
        NumberField::new(&self.input, RANGE, name, "GiB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_is_held_to_the_range() {
        assert_eq!(RANGE.parse("0.5"), Some(0.5));
        assert_eq!(RANGE.parse("-1"), Some(0.));
        assert_eq!(RANGE.parse("99999999"), Some(MAX_GIB));
    }

    #[test]
    fn negative_zero_is_zero() {
        assert!(RANGE.parse("-0").unwrap().is_sign_positive());
    }

    #[test]
    fn text_on_its_way_to_a_number_is_none_yet() {
        for text in ["", "-", "."] {
            assert_eq!(RANGE.parse(text), None, "{text:?}");
        }
    }
}
