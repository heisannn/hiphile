use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, Render, Subscription, Window};
use hiphile_core::settings::{CacheKind, CacheLimit};

use crate::number_field::{NumberField, number_state};

/// Only this field changes the limit, so what it shows never has to follow the settings, and
/// they cannot replace what is being typed.
pub(crate) struct CacheLimitField {
    settings: CacheLimit,
    input: Entity<InputState>,
    _input_events: Subscription,
}

impl CacheLimitField {
    pub(crate) fn new(settings: CacheLimit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = settings.snapshot().gib_text();
        let input = cx.new(|cx| number_state(CacheLimit::range(), window, cx).default_value(text));
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
        match event {
            InputEvent::Change => self.settings.set_typed(&input.read(cx).value()),
            // Text that is no number gives way to the limit, and `007` becomes `7`
            InputEvent::Blur => self.show_limit(window, cx),
            _ => {}
        }
    }

    fn show_limit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.settings.snapshot().gib_text();
        if self.input.read(cx).value().as_ref() != text {
            self.input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    pub(crate) fn is_resettable(&self) -> bool {
        !self.settings.snapshot().is_default()
    }

    pub(crate) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.reset();
        self.show_limit(window, cx);
    }
}

impl Render for CacheLimitField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let name = match self.settings.snapshot().kind() {
            CacheKind::Songs => "Song cache limit",
            CacheKind::Covers => "Cover cache limit",
        };
        NumberField::new(&self.input, CacheLimit::range(), name, "GiB")
    }
}

#[cfg(test)]
mod tests;
