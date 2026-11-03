use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _, TitleBar,
    group_box::GroupBoxVariant,
    h_flex,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use gpui_kit::{
    Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div,
};

use crate::audio_settings::{AudioSettings, MATCH_SONG, SYSTEM_DEFAULT};
use crate::{CloseWindow, SETTINGS_CONTEXT};

/// How often the devices are listed again while the window is open, so a
/// device plugged in meanwhile shows up. The system's device list has no
/// change notification that reaches here.
const DEVICE_REFRESH: Duration = Duration::from_secs(2);

/// Root view of the settings window. The choices live in [`AudioSettings`].
pub(crate) struct SettingsView {
    audio: Entity<AudioSettings>,
    focus: FocusHandle,
    _audio_changes: Subscription,
    _refresh: Task<()>,
}

impl SettingsView {
    pub(crate) fn new(
        audio: Entity<AudioSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Holds focus so the window's commands work before anything is clicked.
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        audio.update(cx, |audio, cx| audio.list_devices(cx));
        let refresh = cx.spawn({
            let audio = audio.downgrade();
            async move |_, cx| {
                loop {
                    cx.background_executor().timer(DEVICE_REFRESH).await;
                    if audio
                        .update(cx, |audio, cx| audio.list_devices(cx))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });
        Self {
            _audio_changes: cx.observe(&audio, |_, _, cx| cx.notify()),
            audio,
            focus,
            _refresh: refresh,
        }
    }

    fn audio_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let audio = self.audio.read(cx);
        let device = SettingField::scrollable_dropdown(
            audio.device_options(),
            {
                let audio = self.audio.clone();
                move |cx| audio.read(cx).device_value()
            },
            {
                let audio = self.audio.clone();
                move |value: SharedString, cx| {
                    audio.update(cx, |audio, cx| audio.set_device(&value, cx));
                }
            },
        )
        .default_value(SYSTEM_DEFAULT);
        let rate = SettingField::scrollable_dropdown(
            audio.rate_options(),
            {
                let audio = self.audio.clone();
                move |cx| audio.read(cx).rate_value()
            },
            {
                let audio = self.audio.clone();
                move |value: SharedString, cx| {
                    audio.update(cx, |audio, cx| audio.set_rate(&value, cx));
                }
            },
        )
        .default_value(MATCH_SONG);

        let errors: Vec<SharedString> = audio.errors().cloned().collect();
        let mut output = SettingGroup::new().items([
            SettingItem::new("Output device", device).disabled(!audio.has_devices()),
            SettingItem::new("Sample rate", rate),
        ]);
        if !errors.is_empty() {
            output = output.footer(move |_, cx| {
                v_flex()
                    .gap_1()
                    .text_color(cx.theme().danger)
                    .children(errors.iter().cloned())
            });
        }
        SettingPage::new("Audio")
            .icon(IconName::AudioLines)
            .group(output)
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("settings-window")
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus)
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .size_full()
                        .px_1()
                        .child(div().text_sm().font_medium().child("Settings")),
                ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new("settings")
                        .with_group_variant(GroupBoxVariant::Outline)
                        .page(self.audio_page(cx)),
                ),
            )
    }
}
