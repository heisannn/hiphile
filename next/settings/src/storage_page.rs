use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{ButtonStyle, linear_progress, styled_button};
use anti_webui_md3e::typography::TextStyle;
use fuzzy::bundle::BUNDLE;
use hiphile_core::settings::{CacheLimit, SongCacheUsage, format_size};
use session::{Readings, ReadingsState};

use crate::controls::{never, switch};
use crate::group::{Group, Row, page};
use crate::number_field::cache_limit_field;

const PROGRESS: i32 = 128;

pub(crate) fn storage_page(
    songs: &CacheLimit,
    covers: &CacheLimit,
    usage: &SongCacheUsage,
    readings: &Readings,
) -> Div {
    page(vec![
        song_cache_group(songs, usage).into_view(),
        Group::titled("Cover cache")
            .row(limit_row(covers))
            .errors({
                let shown = next_host::follow(covers.subscribe());
                move || shown.with(|shown| shown.error().map(str::to_owned).into_iter().collect())
            })
            .view()
            .into_view(),
        romaji_search_group(readings).into_view(),
    ])
}

fn limit_row(limit: &CacheLimit) -> Row {
    let shown = next_host::follow(limit.subscribe());
    let limit = limit.clone();
    Row::new("Limit", move || cache_limit_field(&limit, shown)).supporting_text("0 means no limit.")
}

fn song_cache_group(songs: &CacheLimit, usage: &SongCacheUsage) -> Div {
    let limit = next_host::follow(songs.subscribe());
    let counted = next_host::follow(usage.subscribe());
    let used = {
        let usage = usage.clone();
        Row::new("Used", move || {
            let usage = usage.clone();
            let size = TextStyle::BODY_MEDIUM
                .apply(div().child(text(move || {
                    counted.with(|counted| counted.kept().map(format_size).unwrap_or_default())
                })))
                .dyn_style(|s| s.text_color(color_scheme().on_surface_variant));
            let nothing_to_clear = move || {
                counted.with(|counted| {
                    counted.is_clearing() || counted.kept().is_none_or(|kept| kept == 0)
                })
            };
            let clear = styled_button(
                "Clear",
                ButtonStyle::outlined().disabled_when(nothing_to_clear),
            )
            .access(|a| a.label("Clear Song Cache"))
            .on_click(move |_| usage.clear());
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(12.0_f32)
                .child(size)
                .child(clear)
        })
    };
    Group::titled("Song cache")
        .rows([limit_row(songs), used])
        .errors(move || {
            let mut errors = Vec::new();
            limit.with(|limit| errors.extend(limit.error().map(str::to_owned)));
            counted.with(|counted| errors.extend(counted.error().map(str::to_owned)));
            errors
        })
        .view()
}

fn romaji_search_group(readings: &Readings) -> Div {
    let status = next_host::follow(readings.subscribe());
    let toggle = {
        let readings = readings.clone();
        Row::new("Find kanji by romaji", move || {
            let readings = readings.clone();
            switch(
                "Find kanji by romaji",
                move || status.with(|status| status.is_on()),
                move |is_on| readings.set_on(is_on),
                never(),
            )
        })
        .supporting_text(format!(
            "Downloads a dictionary of {}, which takes {} on this computer.",
            format_size(BUNDLE.download_bytes),
            format_size(BUNDLE.disk_bytes),
        ))
    };
    let state = memo(move || status.with(|status| status.state().clone()));
    let dictionary = {
        let readings = readings.clone();
        Row::new("Dictionary", move || {
            let readings = readings.clone();
            div().child(dyn_view(move || {
                let downloading = matches!(state.get(), ReadingsState::Downloading(_));
                let readings = readings.clone();
                untrack(move || {
                    if downloading {
                        download_progress(state).into_view()
                    } else {
                        styled_button("Retry", ButtonStyle::outlined())
                            .access(|a| a.label("Retry Downloading the Dictionary"))
                            .on_click(move |_| readings.retry())
                            .into_view()
                    }
                })
            }))
        })
        .visible_when(move || {
            matches!(
                state.get(),
                ReadingsState::Downloading(_) | ReadingsState::Failed(_)
            )
        })
    };
    Group::titled("Romaji search")
        .rows([toggle, dictionary])
        .errors(move || {
            status.with(|status| {
                let failed = match status.state() {
                    ReadingsState::Failed(error) => Some(error.clone()),
                    _ => None,
                };
                failed
                    .into_iter()
                    .chain(status.choice_error().map(str::to_owned))
                    .collect()
            })
        })
        .view()
}

fn download_progress(state: Memo<ReadingsState>) -> Div {
    let fraction = move || match state.get() {
        ReadingsState::Downloading(fraction) => fraction,
        _ => 0.0,
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(12.0_f32)
        .child(
            linear_progress(fraction)
                .w(PROGRESS)
                .access(|a| a.label("Dictionary Download")),
        )
        .child(
            TextStyle::BODY_MEDIUM
                .apply(div().child(text(move || format!("{}%", (fraction() * 100.0).round()))))
                .dyn_style(|s| s.text_color(color_scheme().on_surface_variant)),
        )
}

#[cfg(test)]
mod tests;
