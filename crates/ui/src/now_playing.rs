use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::{App, IntoElement, RenderOnce, SharedString, Styled as _, Window};
use subsonic::SongId;

/// How the song the player is on is coming along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
}

impl PlayState {
    pub fn new(playing: bool) -> Self {
        if playing { Self::Playing } else { Self::Paused }
    }

    /// A screen reader's name for the row of a song titled `title`, which says
    /// what the row's icon shows.
    pub fn row_label(title: &SharedString, state: Option<Self>) -> SharedString {
        match state {
            Some(Self::Playing) => format!("{title}, playing").into(),
            Some(Self::Paused) => format!("{title}, paused").into(),
            None => title.clone(),
        }
    }
}

/// The song the player is on, marked in the lists it appears in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NowPlaying {
    song: SongId,
    state: PlayState,
}

impl NowPlaying {
    /// `None` without a song.
    pub fn new(song: Option<SongId>, playing: bool) -> Option<Self> {
        song.map(|song| Self {
            song,
            state: PlayState::new(playing),
        })
    }

    /// The state to show on a row of `song`, if it is the one the player is on.
    pub fn state_of(&self, song: &SongId) -> Option<PlayState> {
        (&self.song == song).then_some(self.state)
    }
}

/// The sound icon on the row of the song the player is on, dimmed while paused.
#[derive(IntoElement)]
pub struct PlayStateIcon(PlayState);

impl PlayStateIcon {
    pub fn new(state: PlayState) -> Self {
        Self(state)
    }
}

impl RenderOnce for PlayStateIcon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = match self.0 {
            PlayState::Playing => cx.theme().primary,
            PlayState::Paused => cx.theme().muted_foreground,
        };
        Icon::new(IconName::AudioLines).small().text_color(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_song_the_player_is_on_has_a_state() {
        let now = NowPlaying::new(Some(SongId::new("a")), false).unwrap();
        assert_eq!(now.state_of(&SongId::new("a")), Some(PlayState::Paused));
        assert_eq!(now.state_of(&SongId::new("b")), None);
        assert_eq!(NowPlaying::new(None, true), None);
    }

    #[test]
    fn a_rows_label_says_what_its_icon_shows() {
        let title = SharedString::from("One");
        assert_eq!(
            PlayState::row_label(&title, Some(PlayState::Playing)),
            "One, playing"
        );
        assert_eq!(
            PlayState::row_label(&title, Some(PlayState::Paused)),
            "One, paused"
        );
        assert_eq!(PlayState::row_label(&title, None), "One");
    }
}
