use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, Render, Styled as _, Subscription, Window,
};

use crate::discord_settings::DiscordSettings;

/// Edits the key in [`DiscordSettings`]. Only this field changes it, so what it shows never has
/// to follow the settings, and they cannot replace what is being typed.
pub(crate) struct ImgbbKeyField {
    settings: Entity<DiscordSettings>,
    input: Entity<InputState>,
    _input_events: Subscription,
    _settings_changes: Subscription,
}

impl ImgbbKeyField {
    pub(crate) fn new(
        settings: Entity<DiscordSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let key = settings.read(cx).imgbb_key().unwrap_or_default().to_owned();
        let input = cx.new(|cx| InputState::new(window, cx).masked(true).default_value(key));
        let input_events = cx.subscribe(&input, {
            let settings = settings.clone();
            move |_, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value();
                    settings.update(cx, |settings, cx| settings.set_typed(&text, cx));
                }
            }
        });
        Self {
            _settings_changes: cx.observe(&settings, |_, _, cx| cx.notify()),
            settings,
            input,
            _input_events: input_events,
        }
    }
}

impl Render for ImgbbKeyField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Input::new(&self.input)
            .aria_label("ImgBB API key")
            .disabled(!self.settings.read(cx).is_presence_on())
            .mask_toggle()
            .w_full()
    }
}
