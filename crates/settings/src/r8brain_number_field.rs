use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, Render, Subscription, Task, Window};
use hiphile_core::settings::{AudioSettings, AudioSnapshot, R8brainNumber, R8brainSetting};

use crate::number_field::{NumberField, number_state};

/// Shows and edits one number of the r8brain parameters, as the settings word it: what was typed
/// until it is left, or else the number.
pub(crate) struct R8brainNumberField {
    audio: AudioSettings,
    number: R8brainNumber,
    input: Entity<InputState>,
    _input_events: Subscription,
    _audio_changes: Task<()>,
}

impl R8brainNumberField {
    pub(crate) fn new(
        audio: AudioSettings,
        number: R8brainNumber,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = audio.snapshot().r8brain_text(number);
        let input = cx.new(|cx| {
            let state = number_state(number.range(), window, cx).default_value(text);
            match number.placeholder() {
                Some(placeholder) => state.placeholder(placeholder),
                None => state,
            }
        });
        Self {
            _input_events: cx.subscribe_in(&input, window, Self::on_input_event),
            _audio_changes: gpui_host::observe_in(
                audio.subscribe(),
                window,
                cx,
                Self::on_audio_changed,
            ),
            audio,
            number,
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
            InputEvent::Change => {
                let text = input.read(cx).value();
                self.audio.type_r8brain(self.number, &text);
            }
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                self.audio.commit_r8brain(self.number);
                self.show(&self.audio.snapshot(), window, cx);
            }
            InputEvent::Focus => {}
        }
    }

    fn on_audio_changed(
        &mut self,
        audio: AudioSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show(&audio, window, cx);
    }

    fn show(&self, audio: &AudioSnapshot, window: &mut Window, cx: &mut Context<Self>) {
        let text = audio.r8brain_text(self.number);
        if self.input.read(cx).value().as_ref() != text {
            self.input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    pub(crate) fn is_resettable(&self) -> bool {
        self.audio
            .snapshot()
            .is_r8brain_resettable(R8brainSetting::Number(self.number))
    }

    pub(crate) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.audio
            .reset_r8brain(R8brainSetting::Number(self.number));
        self.show(&self.audio.snapshot(), window, cx);
    }
}

impl Render for R8brainNumberField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        NumberField::new(
            &self.input,
            self.number.range(),
            self.number.name(),
            self.number.unit(),
        )
    }
}

#[cfg(test)]
mod tests;
