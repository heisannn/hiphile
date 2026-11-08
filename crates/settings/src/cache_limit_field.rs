use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, Render, Subscription, Window};
use hiphile_core::settings::{CacheKind, CacheLimit};

use crate::number_field::{NumberField, NumberRange};

/// The most the field accepts, far beyond any disk.
const MAX_GIB: f64 = (1u32 << 20) as f64;

const RANGE: NumberRange = NumberRange {
    min: 0.,
    max: MAX_GIB,
    step: 1.,
    fraction: None,
};

/// Only this field changes the limit, so what it shows never has to follow the settings, and
/// they cannot replace what is being typed.
pub(crate) struct CacheLimitField {
    settings: CacheLimit,
    input: Entity<InputState>,
    _input_events: Subscription,
}

impl CacheLimitField {
    pub(crate) fn new(settings: CacheLimit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = settings.snapshot().gib().to_string();
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
            InputEvent::Change => match RANGE.parse(&value) {
                Some(gib) => self.settings.set_gib(gib),
                None => self.settings.revert(),
            },
            // Leaving the field shows the number as it counts: text that is no number gives
            // way to the limit, and `007` becomes `7`.
            InputEvent::Blur => {
                let text = self.settings.snapshot().gib().to_string();
                if value.as_ref() != text {
                    input.update(cx, |input, cx| input.set_value(text, window, cx));
                }
            }
            _ => {}
        }
    }

    pub(crate) fn is_resettable(&self) -> bool {
        let settings = self.settings.snapshot();
        settings.gib() != settings.kind().default_gib()
    }

    /// Goes back to the default limit, reporting the change as typing would.
    pub(crate) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let default = self.settings.snapshot().kind().default_gib();
        self.input.update(cx, |input, cx| {
            input.replace_all(default.to_string(), window, cx)
        });
    }
}

impl Render for CacheLimitField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let name = match self.settings.snapshot().kind() {
            CacheKind::Songs => "Song cache limit",
            CacheKind::Covers => "Cover cache limit",
        };
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
