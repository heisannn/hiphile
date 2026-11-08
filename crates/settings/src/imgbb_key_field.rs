use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, Render, Styled as _, Subscription, Task, Window,
};
use hiphile_core::settings::DiscordSettings;

/// Only this field changes the key, so what it shows never has to follow the settings, and they
/// cannot replace what is being typed.
pub(crate) struct ImgbbKeyField {
    settings: DiscordSettings,
    input: Entity<InputState>,
    _input_events: Subscription,
    _settings_changes: Task<()>,
}

impl ImgbbKeyField {
    pub(crate) fn new(
        settings: DiscordSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let key = settings
            .snapshot()
            .imgbb_key()
            .unwrap_or_default()
            .to_owned();
        let input = cx.new(|cx| InputState::new(window, cx).masked(true).default_value(key));
        let input_events = cx.subscribe(&input, {
            let settings = settings.clone();
            move |_, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value();
                    settings.set_typed(&text);
                }
            }
        });
        Self {
            _settings_changes: gpui_host::notify_on_change(settings.subscribe(), cx),
            settings,
            input,
            _input_events: input_events,
        }
    }
}

impl Render for ImgbbKeyField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Input::new(&self.input)
            .aria_label("ImgBB API key")
            .disabled(!self.settings.snapshot().is_presence_on())
            .mask_toggle()
            .w_full()
    }
}
