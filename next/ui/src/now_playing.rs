//! What a song's row shows of the player: whether it is the song playing or paused on, and whether
//! it will be skipped.

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::sized_icon;
use anti_webui_md3e::icons;
use subsonic::{Song, SongId};

/// The icon's size, and the width of its lane, which is there with no icon too, so that what comes
/// after it stays in line.
pub const PLAY_STATE_WIDTH: f32 = 20.0;
/// What a skipped song's row shows its words and cover at.
pub const SKIPPED_OPACITY: f32 = 0.38;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
}

impl PlayState {
    pub fn new(playing: bool) -> Self {
        if playing { Self::Playing } else { Self::Paused }
    }
}

/// The song the player is on, and whether it plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NowPlaying {
    song: SongId,
    state: PlayState,
}

impl NowPlaying {
    pub fn new(song: Option<SongId>, playing: bool) -> Option<Self> {
        song.map(|song| Self {
            song,
            state: PlayState::new(playing),
        })
    }

    /// `None` for any other song than the one the player is on.
    pub fn state_of(&self, song: &SongId) -> Option<PlayState> {
        (&self.song == song).then_some(self.state)
    }
}

/// The icon of `state`, in a lane of its own. It says nothing to assistive technology, as the
/// row's name says it: [`song_row_label`].
pub fn play_state_icon(state: impl Fn() -> Option<PlayState> + 'static) -> Div {
    let state = memo(state);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(PLAY_STATE_WIDTH)
        .access(|a| a.hidden())
        .child(dyn_view(move || match state.get() {
            None => untrack(|| div().into_view()),
            Some(state) => untrack(|| {
                let symbol = match state {
                    PlayState::Playing => icons::GRAPHIC_EQ,
                    PlayState::Paused => icons::PAUSE,
                };
                sized_icon(symbol, PLAY_STATE_WIDTH)
                    .dyn_style(move |s| {
                        let c = color_scheme();
                        s.text_color(match state {
                            PlayState::Playing => c.primary,
                            PlayState::Paused => c.on_surface_variant,
                        })
                    })
                    .into_view()
            }),
        }))
}

/// What a song's row is called for assistive technology: its title, and what its icon and its
/// look say.
pub fn song_row_label(title: &str, state: Option<PlayState>, is_skipped: bool) -> String {
    let state = match state {
        Some(PlayState::Playing) => ", playing",
        Some(PlayState::Paused) => ", paused",
        None => "",
    };
    let skipped = if is_skipped { ", skipped" } else { "" };
    format!("{title}{state}{skipped}")
}

/// Whether the player passes `song` over, given whether it skips instrumentals.
pub fn is_skipped(song: &Song, is_skipping: bool) -> bool {
    playback::is_skipped(song, is_skipping)
}

/// `content`, faded while `skipped` says so. It is what a row holds, not the row itself, whose
/// selection and hover keep their strength.
pub fn skippable(content: Div, skipped: impl Fn() -> bool + 'static) -> Div {
    content.dyn_style(move |s| {
        if skipped() {
            s.opacity(SKIPPED_OPACITY)
        } else {
            s
        }
    })
}

#[cfg(test)]
mod tests;
