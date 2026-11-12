//! The pages with a setting or two: Queue, Lyrics and Appearance.

use anti_webui::prelude::*;
use hiphile_core::settings::{
    APPEARANCES, AppearanceSettings, AppearanceSnapshot, LYRICS_HIGHLIGHTS, LyricsSettings,
    LyricsSnapshot, QueueSettings, QueueSnapshot,
};
use next_ui::theme::Schemes;

use crate::controls::{never, radios, switch_of};
use crate::group::{Group, Row, page};
use crate::swatches::swatches;

pub(crate) fn queue_page(queue: &QueueSettings) -> Div {
    let shown = next_host::follow(queue.subscribe());
    let queue = queue.clone();
    let skip = Row::new("Skip instrumentals", move || {
        switch_of(
            "Skip instrumentals",
            &queue,
            shown,
            QueueSnapshot::is_skip_instrumentals_on,
            QueueSettings::set_skip_instrumentals,
            never(),
        )
    });
    page(vec![
        Group::new()
            .row(skip)
            .errors(move || shown.with(|shown| error(shown.error())))
            .view()
            .into_view(),
    ])
}

pub(crate) fn lyrics_page(lyrics: &LyricsSettings) -> Div {
    let shown = next_host::follow(lyrics.subscribe());
    let lyrics = lyrics.clone();
    let highlight = Row::new("Highlight", move || {
        let lyrics = lyrics.clone();
        radios(
            "Highlight",
            labels(&LYRICS_HIGHLIGHTS),
            move || {
                shown.with(|shown| {
                    LYRICS_HIGHLIGHTS
                        .iter()
                        .position(|(choice, _)| *choice == shown.highlight())
                        .unwrap_or_default()
                })
            },
            move |ix| lyrics.set_highlight(LYRICS_HIGHLIGHTS[ix].0),
            never(),
        )
    });
    page(vec![
        Group::new()
            .row(highlight)
            .errors(move || shown.with(|shown: &LyricsSnapshot| error(shown.error())))
            .view()
            .into_view(),
    ])
}

/// The theme and the colors the window shows: the shell follows what is chosen here, the screen
/// behind too. The colors that can be chosen show in `schemes`.
pub(crate) fn appearance_page(appearance: &AppearanceSettings, schemes: Schemes) -> Div {
    let shown = next_host::follow(appearance.subscribe());
    let color = Row::new("Color", {
        let appearance = appearance.clone();
        move || {
            let appearance = appearance.clone();
            swatches(
                move || shown.with(AppearanceSnapshot::theme_color),
                move |color| appearance.set_theme_color(color),
                schemes,
            )
        }
    })
    .stacked();
    let appearance = appearance.clone();
    let theme = Row::new("Theme", move || {
        let appearance = appearance.clone();
        radios(
            "Theme",
            labels(&APPEARANCES),
            move || {
                shown.with(|shown| {
                    APPEARANCES
                        .iter()
                        .position(|(choice, _)| *choice == shown.appearance())
                        .unwrap_or_default()
                })
            },
            move |ix| {
                let (choice, _) = APPEARANCES[ix];
                if appearance.snapshot().appearance() != choice {
                    appearance.set_appearance(choice);
                }
            },
            never(),
        )
    });
    page(vec![
        Group::new()
            .row(theme)
            .row(color)
            .errors(move || {
                shown.with(|shown: &AppearanceSnapshot| shown.errors().map(str::to_owned).collect())
            })
            .view()
            .into_view(),
    ])
}

fn labels<T>(choices: &[(T, &str)]) -> Vec<String> {
    choices
        .iter()
        .map(|(_, label)| (*label).to_owned())
        .collect()
}

pub(crate) fn error(error: Option<&str>) -> Vec<String> {
    error.map(str::to_owned).into_iter().collect()
}

#[cfg(test)]
mod tests;
