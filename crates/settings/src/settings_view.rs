use std::time::Duration;

use fuzzy::bundle::BUNDLE;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, StyledExt as _, TitleBar,
    button::Button,
    group_box::GroupBoxVariant,
    h_flex,
    progress::Progress,
    radio::{Radio, RadioGroup},
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Axis, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window, div,
};
use session::{Readings, ReadingsState};
use updater::{UpdateState, Updater};

use crate::appearance_settings::{AppearanceSettings, CHOICES};
use crate::audio_settings::{AudioSettings, MATCH_SONG, QUALITIES, SYSTEM_DEFAULT};
use crate::cache_limit::CacheLimit;
use crate::cache_limit_field::CacheLimitField;
use crate::discord_settings::DiscordSettings;
use crate::dsd_section::dsd_groups;
use crate::imgbb_key_field::ImgbbKeyField;
use crate::lyrics_settings::{self, LyricsSettings};
use crate::song_cache_usage::{SongCacheUsage, format_size};
use crate::{CloseWindow, SETTINGS_CONTEXT};

/// How often the devices are listed and the kept songs counted again while the
/// window is open, so a device plugged in or a song kept meanwhile shows up.
/// Neither reports a change that reaches here.
const REFRESH: Duration = Duration::from_secs(2);

pub(crate) struct SettingsView {
    appearance: Entity<AppearanceSettings>,
    audio: Entity<AudioSettings>,
    song_cache: Entity<CacheLimit>,
    song_cache_field: Entity<CacheLimitField>,
    cover_cache: Entity<CacheLimit>,
    cover_cache_field: Entity<CacheLimitField>,
    song_cache_usage: Entity<SongCacheUsage>,
    discord: Entity<DiscordSettings>,
    imgbb_key_field: Entity<ImgbbKeyField>,
    lyrics: Entity<LyricsSettings>,
    readings: Option<Entity<Readings>>,
    /// `None` in a build that does not update itself.
    updater: Option<Entity<Updater>>,
    focus: FocusHandle,
    _appearance_changes: Subscription,
    _system_appearance_changes: Subscription,
    _audio_changes: Subscription,
    _song_cache_changes: Subscription,
    _cover_cache_changes: Subscription,
    _song_cache_usage_changes: Subscription,
    _discord_changes: Subscription,
    _lyrics_changes: Subscription,
    _readings_changes: Option<Subscription>,
    _updater_changes: Option<Subscription>,
    _refresh: Task<()>,
}

impl SettingsView {
    pub(crate) fn new(
        appearance: Entity<AppearanceSettings>,
        audio: Entity<AudioSettings>,
        song_cache: Entity<CacheLimit>,
        cover_cache: Entity<CacheLimit>,
        discord: Entity<DiscordSettings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Holds focus so the window's commands work before anything is clicked
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
        let song_cache_field = cx.new(|cx| CacheLimitField::new(song_cache.clone(), window, cx));
        let cover_cache_field = cx.new(|cx| CacheLimitField::new(cover_cache.clone(), window, cx));
        let imgbb_key_field = cx.new(|cx| ImgbbKeyField::new(discord.clone(), window, cx));
        let lyrics = crate::lyrics_settings(cx);
        let readings = session::readings(cx);
        let updater = updater::updater(cx);
        Self {
            _updater_changes: updater
                .as_ref()
                .map(|updater| cx.observe(updater, |_, _, cx| cx.notify())),
            updater,
            _readings_changes: readings
                .as_ref()
                .map(|readings| cx.observe(readings, |_, _, cx| cx.notify())),
            readings,
            _appearance_changes: cx.observe(&appearance, |_, _, cx| cx.notify()),
            _system_appearance_changes: crate::follow_system_appearance(window),
            appearance,
            _audio_changes: cx.observe(&audio, |_, _, cx| cx.notify()),
            _song_cache_changes: cx.observe(&song_cache, |_, _, cx| cx.notify()),
            _cover_cache_changes: cx.observe(&cover_cache, |_, _, cx| cx.notify()),
            _song_cache_usage_changes: cx.observe(&song_cache_usage, |_, _, cx| cx.notify()),
            _discord_changes: cx.observe(&discord, |_, _, cx| cx.notify()),
            discord,
            imgbb_key_field,
            _lyrics_changes: cx.observe(&lyrics, |_, _, cx| cx.notify()),
            lyrics,
            audio,
            song_cache,
            song_cache_field,
            cover_cache,
            cover_cache_field,
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

    fn lyrics_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let highlight =
            SettingField::render({
                let lyrics = self.lyrics.clone();
                move |_, _, cx| {
                    let selected = lyrics.read(cx).highlight();
                    let lyrics = lyrics.clone();
                    let choices = lyrics_settings::CHOICES;
                    RadioGroup::horizontal("lyrics-highlight")
                        .selected_index(choices.iter().position(|(choice, _)| *choice == selected))
                        .children(choices.iter().enumerate().map(|(ix, (_, label))| {
                            Radio::new(("lyrics-highlight", ix)).label(*label)
                        }))
                        .on_click(move |ix, _, cx| {
                            let (choice, _) = choices[*ix];
                            lyrics.update(cx, |lyrics, cx| lyrics.set_highlight(choice, cx));
                        })
                }
            });

        let mut group = SettingGroup::new().item(SettingItem::new("Highlight", highlight));
        if let Some(error) = self.lyrics.read(cx).error().cloned() {
            group =
                group.footer(move |_, cx| div().text_color(cx.theme().danger).child(error.clone()));
        }
        SettingPage::new("Lyrics")
            .icon(IconName::MicVocal)
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
        let mut rate = SettingItem::new("Sample rate", rate).disabled(audio.is_dsd_on());
        if audio.is_dsd_on() {
            rate = rate.description("DSD sets the rate while it's on.");
        }
        let mut output = SettingGroup::new().items([
            SettingItem::new("Output device", device).disabled(!audio.has_devices()),
            rate,
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
            .group(self.resampling_group(cx))
            .groups(dsd_groups(&self.audio, cx))
    }

    fn resampling_group(&self, cx: &mut Context<Self>) -> SettingGroup {
        let quality =
            SettingField::render({
                let audio = self.audio.clone();
                move |_, _, cx| {
                    let selected = audio.read(cx).quality_ix();
                    let is_dsd_on = audio.read(cx).is_dsd_on();
                    let audio = audio.clone();
                    RadioGroup::horizontal("resampling-quality")
                        .selected_index(Some(selected))
                        .disabled(is_dsd_on)
                        .children(QUALITIES.iter().enumerate().map(|(ix, label)| {
                            Radio::new(("resampling-quality", ix)).label(*label)
                        }))
                        .on_click(move |ix, _, cx| {
                            audio.update(cx, |audio, cx| audio.select_quality(*ix, cx));
                        })
                }
            });
        let audio = self.audio.read(cx);
        let fft_size = SettingField::scrollable_dropdown(
            audio.fft_size_options(),
            {
                let audio = self.audio.clone();
                move |cx| audio.read(cx).fft_size_value()
            },
            {
                let audio = self.audio.clone();
                move |value: SharedString, cx| {
                    audio.update(cx, |audio, cx| audio.set_fft_size(&value, cx));
                }
            },
        );
        let window = SettingField::scrollable_dropdown(
            audio.window_options(),
            {
                let audio = self.audio.clone();
                move |cx| audio.read(cx).window_value()
            },
            {
                let audio = self.audio.clone();
                move |value: SharedString, cx| {
                    audio.update(cx, |audio, cx| audio.set_window(&value, cx));
                }
            },
        );
        let is_dsd_on = audio.is_dsd_on();
        let mut group = SettingGroup::new().title("Resampling").items(
            [
                SettingItem::new("Quality", quality),
                SettingItem::new("FFT size", fft_size),
                SettingItem::new("Window", window),
            ]
            .map(|item| item.keywords(["resampling"]).disabled(is_dsd_on)),
        );
        if is_dsd_on {
            group = group.description("DSD uses the DSD filter settings below while it's on.");
        }
        group
    }

    fn discord_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let key = SettingField::render({
            let field = self.imgbb_key_field.clone();
            move |_, _, _| field.clone()
        });
        let status = SettingField::switch(
            {
                let discord = self.discord.clone();
                move |cx| discord.read(cx).is_presence_on()
            },
            {
                let discord = self.discord.clone();
                move |is_on, cx| discord.update(cx, |it, cx| it.set_presence_on(is_on, cx))
            },
        );
        let discord = self.discord.read(cx);
        let is_presence_on = discord.is_presence_on();
        let errors: Vec<SharedString> = discord.errors().cloned().collect();
        let mut group = SettingGroup::new().items([
            SettingItem::new("Show current song as status", status)
                .keywords(["presence", "rich presence"]),
            SettingItem::new("ImgBB API key", key)
                .disabled(!is_presence_on)
                .layout(Axis::Vertical)
                .description(
                    "Shows covers on your Discord status by uploading them to ImgBB, where \
                     anyone with the link can see them for 180 days. Get a key at api.imgbb.com.",
                )
                .keywords(["cover", "presence", "upload"]),
        ]);
        if !errors.is_empty() {
            group = group.footer(move |_, cx| {
                v_flex()
                    .gap_1()
                    .text_color(cx.theme().danger)
                    .children(errors.iter().cloned())
            });
        }
        SettingPage::new("Discord")
            .icon(IconName::Globe)
            .group(group)
    }

    fn storage_page(&self, cx: &mut Context<Self>) -> SettingPage {
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
            limit_item(&self.song_cache_field, "song cache"),
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
        let mut covers = SettingGroup::new()
            .title("Cover cache")
            .items([limit_item(&self.cover_cache_field, "cover cache")]);
        if let Some(error) = self.cover_cache.read(cx).error().cloned() {
            covers = covers
                .footer(move |_, cx| div().text_color(cx.theme().danger).child(error.clone()));
        }
        let mut page = SettingPage::new("Storage")
            .icon(IconName::HardDrive)
            .group(songs)
            .group(covers);
        if let Some(readings) = &self.readings {
            page = page.group(romaji_search_group(readings, cx));
        }
        page
    }
}

fn updates_page(updater: &Entity<Updater>, cx: &App) -> SettingPage {
    let automatic = SettingField::switch(
        {
            let updater = updater.clone();
            move |cx| updater.read(cx).is_automatic()
        },
        {
            let updater = updater.clone();
            move |is_on, cx| updater.update(cx, |it, cx| it.set_automatic(is_on, cx))
        },
    );
    let action = SettingField::render({
        let updater = updater.clone();
        move |_, _, cx| {
            let state = updater.read(cx).state().clone();
            let updater = updater.clone();
            match state {
                UpdateState::Ready(_) => Button::new("restart-to-update")
                    .outline()
                    .label("Restart to update")
                    .on_click(|_, _, cx| updater::restart(cx)),
                UpdateState::Available(ref new) | UpdateState::Installing(ref new) => {
                    let is_installing = matches!(state, UpdateState::Installing(_));
                    Button::new("install-update")
                        .outline()
                        .label(format!("Install version {new}"))
                        .loading(is_installing)
                        .on_click(move |_, _, cx| updater.update(cx, |it, cx| it.install(cx)))
                }
                _ => Button::new("check-for-updates")
                    .outline()
                    .label("Check for updates")
                    .loading(matches!(state, UpdateState::Checking))
                    .on_click(move |_, _, cx| updater.update(cx, |it, cx| it.check(cx))),
            }
        }
    });

    let state = updater.read(cx).state();
    let mut version = SettingItem::new(format!("Version {}", updater::VERSION), action);
    match state {
        UpdateState::UpToDate => version = version.description("Up to date"),
        UpdateState::Ready(new) => {
            version = version.description(format!("Version {new} starts after a restart"));
        }
        UpdateState::Idle
        | UpdateState::Checking
        | UpdateState::Available(_)
        | UpdateState::Installing(_)
        | UpdateState::Failed(_) => {}
    }
    let mut group = SettingGroup::new().items([
        SettingItem::new("Install updates automatically", automatic),
        version,
    ]);
    let errors: Vec<SharedString> = updater.read(cx).errors().cloned().collect();
    if !errors.is_empty() {
        group = group.footer(move |_, cx| {
            v_flex()
                .gap_1()
                .text_color(cx.theme().danger)
                .children(errors.iter().cloned())
        });
    }
    SettingPage::new("Updates")
        .icon(IconName::RefreshCw)
        .group(group)
}

fn romaji_search_group(readings: &Entity<Readings>, cx: &App) -> SettingGroup {
    let toggle = SettingField::switch(
        {
            let readings = readings.clone();
            move |cx| readings.read(cx).is_on()
        },
        {
            let readings = readings.clone();
            move |is_on, cx| readings.update(cx, |readings, cx| readings.set_on(is_on, cx))
        },
    );
    let description = format!(
        "Downloads a dictionary of {}, which takes {} on this computer.",
        format_size(BUNDLE.download_bytes),
        format_size(BUNDLE.disk_bytes),
    );
    let mut group = SettingGroup::new().title("Romaji search").item(
        SettingItem::new("Find kanji by romaji", toggle)
            .description(description)
            .keywords(["romaji", "dictionary", "search"]),
    );

    let readings = readings.clone();
    let state = readings.read(cx).state().clone();
    let errors = [
        readings_error(&state),
        readings.read(cx).choice_error().cloned(),
    ];
    if matches!(
        state,
        ReadingsState::Downloading(_) | ReadingsState::Failed(_)
    ) {
        let status = SettingField::render(move |_, _, cx| match readings.read(cx).state() {
            ReadingsState::Downloading(fraction) => {
                let percent = (fraction * 100.).round();
                h_flex()
                    .gap_3()
                    .child(
                        Progress::new("readings-download")
                            .w_32()
                            .value(percent)
                            .accessibility_label("Dictionary download"),
                    )
                    .child(div().text_sm().child(format!("{percent}%")))
                    .into_any_element()
            }
            _ => {
                let readings = readings.clone();
                Button::new("retry-readings")
                    .outline()
                    .label("Retry")
                    .accessibility_label("Retry downloading the dictionary")
                    .on_click(move |_, _, cx| readings.update(cx, |it, cx| it.retry(cx)))
                    .into_any_element()
            }
        });
        group = group.item(SettingItem::new("Dictionary", status));
    }
    if errors.iter().any(Option::is_some) {
        group = group.footer(move |_, cx| {
            v_flex()
                .gap_1()
                .text_color(cx.theme().danger)
                .children(errors.iter().flatten().cloned())
        });
    }
    group
}

fn readings_error(state: &ReadingsState) -> Option<SharedString> {
    match state {
        ReadingsState::Failed(error) => Some(error.clone()),
        _ => None,
    }
}

fn limit_item(field: &Entity<CacheLimitField>, keyword: &'static str) -> SettingItem {
    let limit = SettingField::render({
        let field = field.clone();
        move |_, _, _| field.clone()
    })
    .on_reset(
        {
            let field = field.clone();
            move |cx| field.read(cx).is_resettable(cx)
        },
        {
            let field = field.clone();
            move |window, cx| field.update(cx, |field, cx| field.reset(window, cx))
        },
    );
    SettingItem::new("Limit", limit)
        .description("0 means no limit.")
        .keywords([keyword])
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
                        .page(self.lyrics_page(cx))
                        .page(self.storage_page(cx))
                        .page(self.discord_page(cx))
                        .page(self.appearance_page(cx))
                        .pages(self.updater.iter().map(|it| updates_page(it, cx))),
                ),
            )
    }
}
