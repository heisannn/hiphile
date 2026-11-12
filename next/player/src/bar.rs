use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{
    ButtonSize, IconButtonStyle, Ink, button_group, icon_toggle_button, image, sized_icon,
    styled_icon_button, tooltip,
};
use anti_webui_md3e::icons::{self, Symbol};
use anti_webui_md3e::typography::TextStyle;
use next_host::follow;
use next_ui::type_scale::{error_text, muted_text, styled_text};
use playback::{PlayerSnapshot, RepeatMode};
use subsonic::Song;

use crate::control::{PLAY, STANDARD_TOGGLE, icon_control};
use crate::cover::Cover;
use crate::panels::Panels;
use crate::position::{LANE, position_line};
use crate::volume::volume;

const COVER: f32 = 56.0;
const COVER_CORNER: f32 = 12.0;

/// What `player` plays, with `cover` as its cover, and what controls it.
pub fn player_bar(player: playback::Player, cover: Signal<Option<Cover>>, panels: Panels) -> Div {
    let shown = follow(player.subscribe());
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(24.0_f32)
        .py(12.0_f32)
        .access(|a| a.role(Role::Toolbar).label("Player"))
        .child(now_playing(shown, cover, panels.lyrics))
        .child(transport(player.clone(), shown))
        .child(extras(player, shown, panels))
}

/// A lane at either side, which takes as much room as the other, so that the transport stays in
/// the middle.
fn side() -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_1()
        .flex_basis(0.0_f32)
}

fn now_playing(
    shown: Signal<PlayerSnapshot>,
    cover: Signal<Option<Cover>>,
    lyrics: Signal<bool>,
) -> Div {
    let has_song = memo(move || shown.with(PlayerSnapshot::has_song));
    let title =
        memo(move || shown.with(|shown| shown.current_song().map(|song| song.title().to_owned())));
    let details = memo(move || shown.with(|shown| shown.current_song().map(details_of)));
    let error = memo(move || shown.with(|shown| shown.error().map(str::to_owned)));
    // An error wraps, as it tells how to go on
    let status = dyn_view(move || match error.get() {
        Some(error) => untrack(|| error_text(error)),
        None => untrack(|| {
            muted_text(TextStyle::BODY_MEDIUM, move || {
                details.get().unwrap_or_default()
            })
            .whitespace_nowrap()
        }),
    });
    let title = styled_text(TextStyle::TITLE_MEDIUM, move || {
        title.get().unwrap_or_default()
    })
    .whitespace_nowrap();
    let words = div()
        .flex()
        .flex_col()
        .min_w(0.0_f32)
        .overflow_clip()
        .child(title)
        .child(status);
    // The lane that gives way, as the other keeps the volume at the edge
    side()
        .min_w(0.0_f32)
        .gap(12.0_f32)
        .child(show(
            move || has_song.get(),
            move || cover_button(cover, lyrics),
        ))
        .child(words)
}

/// The artist and the album, as far as the song has them.
fn details_of(song: &Song) -> String {
    match (song.artist(), song.album()) {
        (Some(artist), Some(album)) => format!("{artist} — {album}"),
        (Some(only), None) | (None, Some(only)) => only.to_owned(),
        (None, None) => String::new(),
    }
}

fn cover_button(cover: Signal<Option<Cover>>, lyrics: Signal<bool>) -> Div {
    let ink = Ink::new(|c| c.on_surface, || false);
    let focused = ink.focused;
    let picture = dyn_view(move || match cover.get() {
        Some(cover) => untrack(|| image(move || cover.image()).size_full().into_view()),
        None => untrack(|| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .dyn_style(|s| {
                    let c = color_scheme();
                    s.bg(c.surface_container_highest)
                        .text_color(c.on_surface_variant)
                })
                .child(sized_icon(icons::ALBUM, 24.0))
                .into_view()
        }),
    });
    // Over the picture, which would hide what is drawn under the children
    let layer = ink.paint(div().absolute().inset(0.0_f32).rounded(COVER_CORNER));
    let root = div()
        .relative()
        .flex_none()
        .w(COVER)
        .h(COVER)
        .rounded(COVER_CORNER)
        .overflow_clip()
        .select_none()
        .cursor(CursorIcon::Pointer)
        .focusable()
        .access(|a| a.role(Role::Button).label("Lyrics"))
        .dyn_style(move |s| {
            if focused.get() {
                s.outline(3.0_f32)
                    .outline_offset(2.0_f32)
                    .outline_color(color_scheme().secondary)
            } else {
                s
            }
        })
        .on_click(move |_| lyrics.set(!lyrics.get_untracked()))
        .child(picture)
        .child(layer);
    ink.listen(root)
}

fn small() -> IconButtonStyle {
    IconButtonStyle::standard().size(ButtonSize::Small)
}

fn transport(player: playback::Player, shown: Signal<PlayerSnapshot>) -> Div {
    let has_song = memo(move || shown.with(PlayerSnapshot::has_song));
    let is_playing = memo(move || shown.with(PlayerSnapshot::is_playing));
    let is_shuffled = memo(move || shown.with(PlayerSnapshot::is_shuffled));
    let repeat = memo(move || shown.with(PlayerSnapshot::repeat));
    let no_song = move || !has_song.get();

    let shuffling = player.clone();
    let shuffle = icon_toggle_button(
        icons::SHUFFLE,
        Binding::new(
            move || is_shuffled.get(),
            move |_| shuffling.toggle_shuffle(),
        ),
        small(),
    )
    .access(|a| a.label("Shuffle"));
    let going_back = player.clone();
    let previous = styled_icon_button(icons::SKIP_PREVIOUS, small().disabled_when(no_song))
        .access(|a| a.label("Previous"))
        .on_click(move |_| going_back.previous());
    let playing = player.clone();
    let play_symbol = move || -> &'static Symbol {
        if is_playing.get() {
            icons::PAUSE
        } else {
            icons::PLAY_ARROW
        }
    };
    let play = icon_control(play_symbol, PLAY, move || is_playing.get(), no_song)
        .access(move |a| a.label(if is_playing.get() { "Pause" } else { "Play" }))
        .on_click(move |_| playing.toggle_playback());
    let going_on = player.clone();
    let next = styled_icon_button(icons::SKIP_NEXT, small().disabled_when(no_song))
        .access(|a| a.label("Next"))
        .on_click(move |_| going_on.next());
    let repeating = player.clone();
    let is_repeating = move || repeat.get() != RepeatMode::Off;
    let repeat_symbol = move || -> &'static Symbol {
        match repeat.get() {
            RepeatMode::One => icons::REPEAT_ONE,
            RepeatMode::Off | RepeatMode::All => icons::REPEAT,
        }
    };
    let repeat_button = toggle_with(
        Binding::new(is_repeating, move |_| repeating.cycle_repeat()),
        move |_| icon_control(repeat_symbol, STANDARD_TOGGLE, is_repeating, || false),
    )
    .access(move |a| {
        let mode = match repeat.get() {
            RepeatMode::Off => "Off",
            RepeatMode::All => "All",
            RepeatMode::One => "One",
        };
        a.label("Repeat").value_text(mode)
    });

    let controls = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(8.0_f32)
        .child(shuffle)
        .child(button_group([previous, play, next]))
        .child(repeat_button);
    let position = div().h(LANE).child(show(
        move || has_song.get(),
        move || position_line(player.clone(), shown),
    ));
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(4.0_f32)
        .w(pct(40.0))
        .min_w(320.0_f32)
        .max_w(560.0_f32)
        .child(controls)
        .child(position)
}

fn extras(player: playback::Player, shown: Signal<PlayerSnapshot>, panels: Panels) -> Div {
    let has_song = memo(move || shown.with(PlayerSnapshot::has_song));
    let format = memo(move || shown.with(PlayerSnapshot::format_text));
    let lyrics = icon_toggle_button(
        icons::LYRICS,
        panels.lyrics,
        small().disabled_when(move || !has_song.get()),
    )
    .access(|a| a.label("Lyrics"));
    let format = muted_text(TextStyle::LABEL_MEDIUM, move || {
        format.get().unwrap_or_default()
    })
    .flex_shrink(1.0_f32)
    .min_w(0.0_f32)
    .overflow_clip()
    .whitespace_nowrap()
    .mr(8.0_f32);
    side()
        .justify_end()
        .gap(4.0_f32)
        .child(format)
        .child(lyrics)
        .child(queue_toggle(panels))
        .child(volume(player, shown))
}

/// Disabled where the queue does not fit, which it says while the pointer rests on it.
fn queue_toggle(panels: Panels) -> Div {
    let fits = memo(move || panels.queue_fits());
    let toggle = move || {
        icon_toggle_button(
            icons::QUEUE_MUSIC,
            Binding::new(
                move || panels.is_queue_shown(),
                move |_| panels.toggle_queue(),
            ),
            small().disabled_when(move || !fits.get()),
        )
        .access(|a| a.label("Queue"))
    };
    div().flex().flex_none().child(dyn_view(move || {
        if fits.get() {
            untrack(toggle)
        } else {
            // On what holds it, as what is disabled takes no pointer
            untrack(|| tooltip(div().flex().child(toggle()), NO_ROOM_FOR_QUEUE))
        }
    }))
}

const NO_ROOM_FOR_QUEUE: &str = "Widen the window to show the queue.";

#[cfg(test)]
mod tests;
