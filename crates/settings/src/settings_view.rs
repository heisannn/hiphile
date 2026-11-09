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
    AppContext as _, Axis, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window, div,
};
use hiphile_core::settings::{
    APPEARANCES, AppearanceSettings, AudioSettings, AudioSnapshot, CacheLimit, DiscordSettings,
    LYRICS_HIGHLIGHTS, LyricsSettings, MATCH_SONG, QueueSettings, RESAMPLERS, RUBATO_QUALITIES,
    SYSTEM_DEFAULT, SongCacheUsage, format_size,
};
use session::{Readings, ReadingsState};
use updater::{UpdateState, Updater};

use crate::audio_options::dropdown;
use crate::cache_limit_field::CacheLimitField;
use crate::dsd_section::dsd_groups;
use crate::imgbb_key_field::ImgbbKeyField;
use crate::r8brain_section::{R8brainFields, r8brain_groups, r8brain_quality_item};
use crate::soxr_section::{soxr_groups, soxr_quality_item};
use crate::{CloseWindow, SETTINGS_CONTEXT};

/// How often the devices are listed and the kept songs counted again while the
/// window is open, so a device plugged in or a song kept meanwhile shows up.
/// Neither reports a change that reaches here.
const REFRESH: Duration = Duration::from_secs(2);

const PCM_ONLY: &str = "DSD plays songs whole while it's on.";

pub(crate) struct SettingsView {
    appearance: AppearanceSettings,
    audio: AudioSettings,
    r8brain_fields: R8brainFields,
    song_cache: CacheLimit,
    song_cache_field: Entity<CacheLimitField>,
    cover_cache: CacheLimit,
    cover_cache_field: Entity<CacheLimitField>,
    song_cache_usage: SongCacheUsage,
    discord: DiscordSettings,
    imgbb_key_field: Entity<ImgbbKeyField>,
    lyrics: LyricsSettings,
    queue: QueueSettings,
    readings: Readings,
    /// `None` in a build that does not update itself.
    updater: Option<Updater>,
    focus: FocusHandle,
    _appearance_changes: Task<()>,
    _system_appearance_changes: Subscription,
    _audio_changes: Task<()>,
    _song_cache_changes: Task<()>,
    _cover_cache_changes: Task<()>,
    _song_cache_usage_changes: Task<()>,
    _discord_changes: Task<()>,
    _lyrics_changes: Task<()>,
    _queue_changes: Task<()>,
    _readings_changes: Task<()>,
    _updater_changes: Option<Task<()>>,
    _refresh: Task<()>,
}

fn exclusive_item(audio: &AudioSettings, is_dsd_on: bool) -> SettingItem {
    let field = SettingField::switch(
        {
            let audio = audio.clone();
            move |_| audio.snapshot().is_exclusive_on()
        },
        {
            let audio = audio.clone();
            move |is_on, _| audio.set_exclusive(is_on)
        },
    );
    let item = SettingItem::new("Exclusive mode", field).disabled(is_dsd_on);
    if is_dsd_on {
        item.description("DSD needs it while it's on.")
    } else {
        item
    }
}

impl SettingsView {
    pub(crate) fn new(
        settings: hiphile_core::settings::Settings,
        readings: Readings,
        song_cache_usage: SongCacheUsage,
        updater: Option<Updater>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let appearance = settings.appearance().clone();
        let audio = settings.audio().clone();
        let song_cache = settings.song_cache().clone();
        let cover_cache = settings.cover_cache().clone();
        let discord = settings.discord().clone();
        let lyrics = settings.lyrics().clone();
        let queue = settings.queue().clone();
        // Holds focus so the window's commands work before anything is clicked
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        audio.list_devices();
        let refresh = cx.spawn({
            let audio = audio.clone();
            let usage = song_cache_usage.clone();
            async move |_, cx| {
                loop {
                    cx.background_executor().timer(REFRESH).await;
                    audio.list_devices();
                    usage.count();
                }
            }
        });
        let r8brain_fields = R8brainFields::new(&audio, window, cx);
        let song_cache_field = cx.new(|cx| CacheLimitField::new(song_cache.clone(), window, cx));
        let cover_cache_field = cx.new(|cx| CacheLimitField::new(cover_cache.clone(), window, cx));
        let imgbb_key_field = cx.new(|cx| ImgbbKeyField::new(discord.clone(), window, cx));
        Self {
            _updater_changes: updater
                .as_ref()
                .map(|updater| gpui_host::notify_on_change(updater.subscribe(), cx)),
            updater,
            _readings_changes: gpui_host::notify_on_change(readings.subscribe(), cx),
            readings,
            _appearance_changes: gpui_host::notify_on_change(appearance.subscribe(), cx),
            _system_appearance_changes: crate::follow_system_appearance(appearance.clone(), window),
            appearance,
            _audio_changes: gpui_host::notify_on_change(audio.subscribe(), cx),
            _song_cache_changes: gpui_host::notify_on_change(song_cache.subscribe(), cx),
            _cover_cache_changes: gpui_host::notify_on_change(cover_cache.subscribe(), cx),
            _song_cache_usage_changes: gpui_host::notify_on_change(
                song_cache_usage.subscribe(),
                cx,
            ),
            _discord_changes: gpui_host::notify_on_change(discord.subscribe(), cx),
            discord,
            imgbb_key_field,
            _lyrics_changes: gpui_host::notify_on_change(lyrics.subscribe(), cx),
            lyrics,
            _queue_changes: gpui_host::notify_on_change(queue.subscribe(), cx),
            queue,
            r8brain_fields,
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

    fn appearance_page(&self) -> SettingPage {
        let theme = SettingField::render({
            let appearance = self.appearance.clone();
            move |_, _, _| {
                let selected = appearance.snapshot().appearance();
                let appearance = appearance.clone();
                RadioGroup::horizontal("appearance")
                    .selected_index(
                        APPEARANCES
                            .iter()
                            .position(|(choice, _)| *choice == selected),
                    )
                    .children(
                        APPEARANCES
                            .iter()
                            .enumerate()
                            .map(|(ix, (_, label))| Radio::new(("appearance", ix)).label(*label)),
                    )
                    .on_click(move |ix, _, cx| {
                        let (choice, _) = APPEARANCES[*ix];
                        crate::appearance_settings::set_appearance(&appearance, choice, cx);
                    })
            }
        });

        let mut group = SettingGroup::new().item(SettingItem::new("Theme", theme));
        if let Some(error) = error_text(self.appearance.snapshot().error()) {
            group =
                group.footer(move |_, cx| div().text_color(cx.theme().danger).child(error.clone()));
        }
        SettingPage::new("Appearance")
            .icon(IconName::Palette)
            .group(group)
    }

    fn lyrics_page(&self) -> SettingPage {
        let highlight =
            SettingField::render({
                let lyrics = self.lyrics.clone();
                move |_, _, _| {
                    let selected = lyrics.snapshot().highlight();
                    let lyrics = lyrics.clone();
                    let choices = LYRICS_HIGHLIGHTS;
                    RadioGroup::horizontal("lyrics-highlight")
                        .selected_index(choices.iter().position(|(choice, _)| *choice == selected))
                        .children(choices.iter().enumerate().map(|(ix, (_, label))| {
                            Radio::new(("lyrics-highlight", ix)).label(*label)
                        }))
                        .on_click(move |ix, _, _| {
                            let (choice, _) = choices[*ix];
                            lyrics.set_highlight(choice);
                        })
                }
            });

        let mut group = SettingGroup::new().item(SettingItem::new("Highlight", highlight));
        if let Some(error) = error_text(self.lyrics.snapshot().error()) {
            group =
                group.footer(move |_, cx| div().text_color(cx.theme().danger).child(error.clone()));
        }
        SettingPage::new("Lyrics")
            .icon(IconName::MicVocal)
            .group(group)
    }

    fn queue_page(&self) -> SettingPage {
        let skip_instrumentals = SettingField::switch(
            {
                let queue = self.queue.clone();
                move |_| queue.snapshot().is_skip_instrumentals_on()
            },
            {
                let queue = self.queue.clone();
                move |is_on, _| queue.set_skip_instrumentals(is_on)
            },
        );
        let mut group =
            SettingGroup::new().item(SettingItem::new("Skip instrumentals", skip_instrumentals));
        if let Some(error) = error_text(self.queue.snapshot().error()) {
            group =
                group.footer(move |_, cx| div().text_color(cx.theme().danger).child(error.clone()));
        }
        SettingPage::new("Queue")
            .icon(IconName::ListMusic)
            .group(group)
    }

    fn audio_page(&self) -> SettingPage {
        let audio = self.audio.snapshot();
        let device = dropdown(
            &self.audio,
            audio.device_options(),
            AudioSnapshot::device_value,
            AudioSettings::set_device,
        )
        .default_value(SYSTEM_DEFAULT);
        let rate = dropdown(
            &self.audio,
            audio.rate_options(),
            AudioSnapshot::rate_value,
            AudioSettings::set_rate,
        )
        .default_value(MATCH_SONG);

        let errors: Vec<SharedString> = audio.errors().map(SharedString::new).collect();
        let mut rate = SettingItem::new("Sample rate", rate).disabled(audio.is_dsd_on());
        if audio.is_dsd_on() {
            rate = rate.description("DSD sets the rate while it's on.");
        }
        let mut items = vec![
            SettingItem::new("Output device", device).disabled(!audio.has_devices()),
            rate,
        ];
        if AudioSettings::is_exclusive_offered() {
            items.push(exclusive_item(&self.audio, audio.is_dsd_on()));
        }
        let mut output = SettingGroup::new().items(items);
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
            .group(self.play_and_pause_group(&audio))
            .group(self.between_songs_group(&audio))
            .group(self.resampling_group(&audio))
            .groups(r8brain_groups(&self.audio, &self.r8brain_fields))
            .groups(soxr_groups(&self.audio))
            .groups(dsd_groups(&self.audio))
    }

    fn play_and_pause_group(&self, audio: &AudioSnapshot) -> SettingGroup {
        let fade = SettingField::switch(
            {
                let audio = self.audio.clone();
                move |_| audio.snapshot().is_play_pause_fade_on()
            },
            {
                let audio = self.audio.clone();
                move |is_on, _| audio.set_play_pause_fade(is_on)
            },
        );
        let is_dsd_on = audio.is_dsd_on();
        let mut fade = SettingItem::new("Fade", fade).disabled(is_dsd_on);
        if is_dsd_on {
            fade = fade.description("DSD cuts in and out at once while it's on.");
        }
        SettingGroup::new().title("Play and pause").items([fade])
    }

    fn between_songs_group(&self, audio: &AudioSnapshot) -> SettingGroup {
        let trim_silence = SettingField::switch(
            {
                let audio = self.audio.clone();
                move |_| audio.snapshot().is_trim_silence_on()
            },
            {
                let audio = self.audio.clone();
                move |is_on, _| audio.set_trim_silence(is_on)
            },
        );
        let crossfade = dropdown(
            &self.audio,
            AudioSettings::crossfade_options(),
            AudioSnapshot::crossfade_value,
            AudioSettings::set_crossfade,
        );
        let is_dsd_on = audio.is_dsd_on();
        let trim_silence = SettingItem::new("Trim silence", trim_silence)
            .disabled(is_dsd_on)
            .description(if is_dsd_on {
                PCM_ONLY
            } else {
                "Silence longer than 10 seconds inside a song goes too."
            });
        let mut crossfade = SettingItem::new("Crossfade", crossfade).disabled(is_dsd_on);
        let note = if is_dsd_on {
            Some(PCM_ONLY)
        } else {
            audio.crossfade_note()
        };
        if let Some(note) = note {
            crossfade = crossfade.description(note);
        }
        SettingGroup::new()
            .title("Between songs")
            .items([trim_silence, crossfade])
    }

    fn resampling_group(&self, audio: &AudioSnapshot) -> SettingGroup {
        let resampler = SettingField::render({
            let audio = self.audio.clone();
            move |_, _, _| {
                let settings = audio.snapshot();
                let selected = settings.resampler_ix();
                let is_dsd_on = settings.is_dsd_on();
                let audio = audio.clone();
                RadioGroup::horizontal("resampler")
                    .selected_index(Some(selected))
                    .disabled(is_dsd_on)
                    .children(
                        RESAMPLERS
                            .iter()
                            .enumerate()
                            .map(|(ix, label)| Radio::new(("resampler", ix)).label(*label)),
                    )
                    .on_click(move |ix, _, _| audio.select_resampler(*ix))
            }
        });
        let is_dsd_on = audio.is_dsd_on();
        let mut items = vec![SettingItem::new("Resampler", resampler)];
        if audio.uses_r8brain() {
            items.push(r8brain_quality_item(&self.audio));
        } else if audio.uses_soxr() {
            items.push(soxr_quality_item(&self.audio));
        } else {
            items.extend(self.rubato_items(audio));
        }
        let mut group = SettingGroup::new().title("Resampling").items(
            items
                .into_iter()
                .map(|item| item.keywords(["resampling"]).disabled(is_dsd_on)),
        );
        if is_dsd_on {
            group = group.description("DSD uses the DSD filter settings below while it's on.");
        }
        group
    }

    fn rubato_items(&self, audio: &AudioSnapshot) -> [SettingItem; 3] {
        let quality = SettingField::render({
            let audio = self.audio.clone();
            move |_, _, _| {
                let settings = audio.snapshot();
                let selected = settings.rubato_quality_ix();
                let is_dsd_on = settings.is_dsd_on();
                let audio = audio.clone();
                RadioGroup::horizontal("rubato-quality")
                    .selected_index(Some(selected))
                    .disabled(is_dsd_on)
                    .children(
                        RUBATO_QUALITIES
                            .iter()
                            .enumerate()
                            .map(|(ix, label)| Radio::new(("rubato-quality", ix)).label(*label)),
                    )
                    .on_click(move |ix, _, _| audio.select_rubato_quality(*ix))
            }
        });
        let fft_size = dropdown(
            &self.audio,
            audio.fft_size_options(),
            AudioSnapshot::fft_size_value,
            AudioSettings::set_fft_size,
        );
        let window = dropdown(
            &self.audio,
            audio.window_options(),
            AudioSnapshot::window_value,
            AudioSettings::set_window,
        );
        [
            SettingItem::new("Quality", quality),
            SettingItem::new("FFT size", fft_size),
            SettingItem::new("Window", window),
        ]
    }

    fn discord_page(&self) -> SettingPage {
        let key = SettingField::render({
            let field = self.imgbb_key_field.clone();
            move |_, _, _| field.clone()
        });
        let status = SettingField::switch(
            {
                let discord = self.discord.clone();
                move |_| discord.snapshot().is_presence_on()
            },
            {
                let discord = self.discord.clone();
                move |is_on, _| discord.set_presence_on(is_on)
            },
        );
        let discord = self.discord.snapshot();
        let is_presence_on = discord.is_presence_on();
        let errors: Vec<SharedString> = discord.errors().map(SharedString::new).collect();
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

    fn storage_page(&self) -> SettingPage {
        let used = SettingField::render({
            let usage = self.song_cache_usage.clone();
            move |_, _, _| {
                let (kept, clearing) = {
                    let usage = usage.snapshot();
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
                        .on_click(move |_, _, _| usage.clear()),
                )
            }
        });

        let errors: Vec<SharedString> = [
            self.song_cache.snapshot().error(),
            self.song_cache_usage.snapshot().error(),
        ]
        .into_iter()
        .flatten()
        .map(SharedString::new)
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
        if let Some(error) = error_text(self.cover_cache.snapshot().error()) {
            covers = covers
                .footer(move |_, cx| div().text_color(cx.theme().danger).child(error.clone()));
        }
        SettingPage::new("Storage")
            .icon(IconName::HardDrive)
            .group(songs)
            .group(covers)
            .group(romaji_search_group(&self.readings))
    }
}

fn updates_page(updater: &Updater) -> SettingPage {
    let automatic = SettingField::switch(
        {
            let updater = updater.clone();
            move |_| updater.status().is_automatic()
        },
        {
            let updater = updater.clone();
            move |is_on, _| updater.set_automatic(is_on)
        },
    );
    let action = SettingField::render({
        let updater = updater.clone();
        move |_, _, _| {
            let state = updater.status().state().clone();
            let updater = updater.clone();
            match state {
                UpdateState::Ready(_) => Button::new("restart-to-update")
                    .outline()
                    .label("Restart to update")
                    .on_click(|_, _, cx| cx.restart()),
                UpdateState::Available(ref new) | UpdateState::Installing(ref new) => {
                    let is_installing = matches!(state, UpdateState::Installing(_));
                    Button::new("install-update")
                        .outline()
                        .label(format!("Install version {new}"))
                        .loading(is_installing)
                        .on_click(move |_, _, _| updater.install())
                }
                _ => Button::new("check-for-updates")
                    .outline()
                    .label("Check for updates")
                    .loading(matches!(state, UpdateState::Checking))
                    .on_click(move |_, _, _| updater.check()),
            }
        }
    });

    let status = updater.status();
    let state = status.state();
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
    let errors: Vec<SharedString> = status.errors().map(SharedString::new).collect();
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

fn romaji_search_group(readings: &Readings) -> SettingGroup {
    let toggle = SettingField::switch(
        {
            let readings = readings.clone();
            move |_| readings.status().is_on()
        },
        {
            let readings = readings.clone();
            move |is_on, _| readings.set_on(is_on)
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
    let status = readings.status();
    let state = status.state();
    let errors = [
        readings_error(state),
        status.choice_error().map(SharedString::new),
    ];
    if matches!(
        state,
        ReadingsState::Downloading(_) | ReadingsState::Failed(_)
    ) {
        let status = SettingField::render(move |_, _, _| match readings.status().state() {
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
                    .on_click(move |_, _, _| readings.retry())
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
        ReadingsState::Failed(error) => Some(error.into()),
        _ => None,
    }
}

fn error_text(error: Option<&str>) -> Option<SharedString> {
    error.map(SharedString::new)
}

fn limit_item(field: &Entity<CacheLimitField>, keyword: &'static str) -> SettingItem {
    let limit = SettingField::render({
        let field = field.clone();
        move |_, _, _| field.clone()
    })
    .on_reset(
        {
            let field = field.clone();
            move |cx| field.read(cx).is_resettable()
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
                        .page(self.audio_page())
                        .page(self.queue_page())
                        .page(self.lyrics_page())
                        .page(self.storage_page())
                        .page(self.discord_page())
                        .page(self.appearance_page())
                        .pages(self.updater.iter().map(updates_page)),
                ),
            )
    }
}
