use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, StyledExt as _, TitleBar,
    button::Button,
    group_box::GroupBoxVariant,
    h_flex,
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window, div,
};

use crate::appearance_settings::{AppearanceSettings, CHOICES};
use crate::audio_settings::{AudioSettings, MATCH_SONG, SYSTEM_DEFAULT};
use crate::song_cache_field::SongCacheField;
use crate::song_cache_settings::SongCacheSettings;
use crate::song_cache_usage::{SongCacheUsage, format_size};
use crate::{CloseWindow, SETTINGS_CONTEXT};

/// How often the devices are listed and the kept songs counted again while the
/// window is open, so a device plugged in or a song kept meanwhile shows up.
/// Neither reports a change that reaches here.
const REFRESH: Duration = Duration::from_secs(2);

/// Root view of the settings window. The choices live in
/// [`AppearanceSettings`], [`AudioSettings`] and [`SongCacheSettings`].
pub(crate) struct SettingsView {
    appearance: Entity<AppearanceSettings>,
    audio: Entity<AudioSettings>,
    song_cache: Entity<SongCacheSettings>,
    song_cache_field: Entity<SongCacheField>,
    song_cache_usage: Entity<SongCacheUsage>,
    focus: FocusHandle,
    _appearance_changes: Subscription,
    _system_appearance_changes: Subscription,
    _audio_changes: Subscription,
    _song_cache_changes: Subscription,
    _song_cache_usage_changes: Subscription,
    _refresh: Task<()>,
}

impl SettingsView {
    pub(crate) fn new(
        appearance: Entity<AppearanceSettings>,
        audio: Entity<AudioSettings>,
        song_cache: Entity<SongCacheSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Holds focus so the window's commands work before anything is clicked.
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        audio.update(cx, |audio, cx| audio.list_devices(cx));
        let song_cache_usage = cx.new(SongCacheUsage::new);
        let refresh = cx.spawn({
            let audio = audio.downgrade();
            let usage = song_cache_usage.downgrade();
            async move |_, cx| {
                loop {
                    cx.background_executor().timer(REFRESH).await;
                    let listed = audio.update(cx, |audio, cx| audio.list_devices(cx));
                    let counted = usage.update(cx, |usage, cx| usage.count(cx));
                    if listed.is_err() || counted.is_err() {
                        break;
                    }
                }
            }
        });
        let song_cache_field = cx.new(|cx| SongCacheField::new(song_cache.clone(), window, cx));
        Self {
            _appearance_changes: cx.observe(&appearance, |_, _, cx| cx.notify()),
            _system_appearance_changes: crate::follow_system_appearance(window),
            appearance,
            _audio_changes: cx.observe(&audio, |_, _, cx| cx.notify()),
            _song_cache_changes: cx.observe(&song_cache, |_, _, cx| cx.notify()),
            _song_cache_usage_changes: cx.observe(&song_cache_usage, |_, _, cx| cx.notify()),
            audio,
            song_cache,
            song_cache_field,
            song_cache_usage,
            focus,
            _refresh: refresh,
        }
    }

    fn appearance_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let theme = SettingField::render({
            let appearance = self.appearance.clone();
            move |_, _, cx| {
                let selected = appearance.read(cx).appearance();
                let appearance = appearance.clone();
                RadioGroup::horizontal("appearance")
                    .selected_index(CHOICES.iter().position(|(choice, _)| *choice == selected))
                    .children(
                        CHOICES
                            .iter()
                            .enumerate()
                            .map(|(ix, (_, label))| Radio::new(("appearance", ix)).label(*label)),
                    )
                    .on_click(move |ix, _, cx| {
                        let (choice, _) = CHOICES[*ix];
                        appearance.update(cx, |appearance, cx| {
                            appearance.set_appearance(choice, cx);
                        });
                    })
            }
        });

        let mut group = SettingGroup::new().item(SettingItem::new("Theme", theme));
        if let Some(error) = self.appearance.read(cx).error().cloned() {
            group =
                group.footer(move |_, cx| div().text_color(cx.theme().danger).child(error.clone()));
        }
        SettingPage::new("Appearance")
            .icon(IconName::Palette)
            .group(group)
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

    fn storage_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let limit = SettingField::render({
            let field = self.song_cache_field.clone();
            move |_, _, _| field.clone()
        })
        .on_reset(
            {
                let field = self.song_cache_field.clone();
                move |cx| field.read(cx).is_resettable(cx)
            },
            {
                let field = self.song_cache_field.clone();
                move |window, cx| field.update(cx, |field, cx| field.reset(window, cx))
            },
        );

        let used = SettingField::render({
            let usage = self.song_cache_usage.clone();
            move |_, _, cx| {
                let (kept, clearing) = {
                    let usage = usage.read(cx);
                    (usage.kept(), usage.is_clearing())
                };
                let size = kept.map(format_size).unwrap_or_default();
                let usage = usage.clone();
                h_flex().gap_3().child(div().text_sm().child(size)).child(
                    Button::new("clear-song-cache")
                        .outline()
                        .label("Clear")
                        .accessibility_label("Clear song cache")
                        .loading(clearing)
                        .disabled(kept.is_none_or(|kept| kept == 0))
                        .on_click(move |_, _, cx| usage.update(cx, |usage, cx| usage.clear(cx))),
                )
            }
        });

        let errors: Vec<SharedString> = [
            self.song_cache.read(cx).error(),
            self.song_cache_usage.read(cx).error(),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
        let mut songs = SettingGroup::new().title("Song cache").items([
            SettingItem::new("Limit", limit)
                .description("0 means no limit.")
                .keywords(["song cache"]),
            SettingItem::new("Used", used).keywords(["song cache", "clear"]),
        ]);
        if !errors.is_empty() {
            songs = songs.footer(move |_, cx| {
                v_flex()
                    .gap_1()
                    .text_color(cx.theme().danger)
                    .children(errors.iter().cloned())
            });
        }
        SettingPage::new("Storage")
            .icon(IconName::HardDrive)
            .group(songs)
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
                        .page(self.audio_page(cx))
                        .page(self.storage_page(cx))
                        .page(self.appearance_page(cx)),
                ),
            )
    }
}
