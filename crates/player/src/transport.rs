use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
    slider::SliderState,
    spinner::Spinner,
};
use gpui_kit::{
    App, Entity, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Styled as _,
    Window, div, prelude::FluentBuilder as _, rems,
};
use playback::RepeatMode;
use ui::format_duration;

use crate::hover_slider::HoverSlider;
use crate::player::Player;
use crate::{
    CycleRepeat, NextSong, PreviousSong, ToggleLyrics, ToggleMiniPlayer, TogglePlayback,
    ToggleQueue, ToggleShuffle,
};

/// The kit's play and pause icons, filled in, as it has no such icons.
const PLAY_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/>
</svg>"#;
const PAUSE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
<rect x="14" y="3" width="5" height="18" rx="1"/>
<rect x="5" y="3" width="5" height="18" rx="1"/>
</svg>"#;

#[derive(IntoElement)]
pub(crate) struct TransportControls {
    player: Entity<Player>,
    roomy: bool,
}

impl TransportControls {
    pub(crate) fn new(player: Entity<Player>) -> Self {
        Self {
            player,
            roomy: false,
        }
    }

    /// Larger, and spread over the width of the parent.
    pub(crate) fn roomy(mut self) -> Self {
        self.roomy = true;
        self
    }
}

impl RenderOnce for TransportControls {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let core = self.player.read(cx).core().clone();
        let snapshot = core.snapshot();
        let has_song = snapshot.has_song();
        let playing = snapshot.is_playing();
        let shuffled = snapshot.is_shuffled();
        let repeat = snapshot.repeat();
        let (icon, label) = if playing {
            (PAUSE_ICON, "Pause")
        } else {
            (PLAY_ICON, "Play")
        };
        let (repeat_icon, repeat_label) = match repeat {
            RepeatMode::Off => (IconName::Repeat, "Repeat off"),
            RepeatMode::All => (IconName::Repeat, "Repeat all"),
            RepeatMode::One => (IconName::Repeat1, "Repeat one"),
        };
        let size = if self.roomy {
            Size::Large
        } else {
            Size::Medium
        };
        let on_click = |command: fn(&playback::Player)| {
            let core = core.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, _: &mut App| command(&core)
        };
        h_flex()
            .when(self.roomy, |this| this.w_full().justify_between())
            .when(!self.roomy, |this| {
                this.debug_selector(|| "player-controls".to_owned())
            })
            .flex_none()
            .gap_1()
            .child(
                Button::new("toggle-shuffle")
                    .ghost()
                    .with_size(size)
                    .icon(IconName::Shuffle)
                    .accessibility_label("Shuffle")
                    .tooltip_with_action("Shuffle", &ToggleShuffle, None)
                    .selected(shuffled)
                    .toggled(shuffled)
                    .on_click(on_click(playback::Player::toggle_shuffle)),
            )
            .child(
                Button::new("previous-song")
                    .ghost()
                    .with_size(size)
                    .icon(IconName::SkipBack)
                    .accessibility_label("Previous")
                    .tooltip_with_action("Previous", &PreviousSong, None)
                    .disabled(!has_song)
                    .on_click(on_click(playback::Player::previous)),
            )
            .child(
                Button::new("toggle-playback")
                    .ghost()
                    .with_size(size)
                    .icon(Icon::default().data(icon))
                    .accessibility_label(label)
                    .tooltip_with_action(label, &TogglePlayback, None)
                    .disabled(!has_song)
                    .on_click(on_click(playback::Player::toggle_playback)),
            )
            .child(
                Button::new("next-song")
                    .ghost()
                    .with_size(size)
                    .icon(IconName::SkipForward)
                    .accessibility_label("Next")
                    .tooltip_with_action("Next", &NextSong, None)
                    .disabled(!has_song)
                    .on_click(on_click(playback::Player::next)),
            )
            .child(
                Button::new("cycle-repeat")
                    .ghost()
                    .with_size(size)
                    .icon(repeat_icon)
                    .accessibility_label(repeat_label)
                    .tooltip_with_action(repeat_label, &CycleRepeat, None)
                    .selected(repeat != RepeatMode::Off)
                    .on_click(on_click(playback::Player::cycle_repeat)),
            )
    }
}

#[derive(IntoElement)]
pub(crate) struct PositionLine {
    player: Entity<Player>,
    /// How far open it is, from `0.0` to `1.0`.
    shown: f32,
    in_bar: bool,
    /// In place of the player's main slider, for a line in another window.
    slider: Option<Entity<SliderState>>,
}

impl PositionLine {
    /// The bar's line, which opens from nothing by `shown`. Fixed lanes for
    /// the times keep the line from moving as the digits change.
    pub(crate) fn in_bar(player: Entity<Player>, shown: f32) -> Self {
        Self {
            player,
            shown,
            in_bar: true,
            slider: None,
        }
    }

    pub(crate) fn full_width(player: Entity<Player>) -> Self {
        Self {
            player,
            shown: 1.,
            in_bar: false,
            slider: None,
        }
    }

    pub(crate) fn slider(mut self, slider: Entity<SliderState>) -> Self {
        self.slider = Some(slider);
        self
    }
}

impl RenderOnce for PositionLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = self.player.read(cx);
        let snapshot = player.snapshot();
        let duration = snapshot.duration();
        let seekable = snapshot.is_seekable();
        let position = player.shown_position();
        let buffering = snapshot.is_buffering();
        let slider = self
            .slider
            .unwrap_or_else(|| player.position_state().clone());
        h_flex()
            .when(self.in_bar, |this| {
                this.debug_selector(|| "player-position".to_owned())
            })
            .w_full()
            .h(rems(self.shown))
            .opacity(self.shown)
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .when(snapshot.has_song(), |this| {
                this.child(
                    h_flex()
                        .flex_none()
                        .when(self.in_bar, |this| this.min_w_9().justify_end())
                        .relative()
                        // Keeps its room, so that the line does not move
                        .child(
                            div()
                                .when(buffering, |this| this.opacity(0.))
                                .child(format_duration(position)),
                        )
                        .when(buffering, |this| {
                            this.child(
                                h_flex()
                                    .debug_selector(|| "player-buffering".to_owned())
                                    .absolute()
                                    .inset_0()
                                    .justify_end()
                                    .child(
                                        Spinner::new().xsmall().color(cx.theme().muted_foreground),
                                    ),
                            )
                        }),
                )
                .child(HoverSlider::new(&slider, "Position").disabled(!seekable))
                .child(
                    div()
                        .flex_none()
                        .when(self.in_bar, |this| this.min_w_9())
                        .child(duration.map(format_duration).unwrap_or_default()),
                )
            })
    }
}

#[derive(IntoElement)]
pub(crate) struct PanelToggles {
    player: Entity<Player>,
}

impl PanelToggles {
    pub(crate) fn new(player: Entity<Player>) -> Self {
        Self { player }
    }
}

impl RenderOnce for PanelToggles {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = self.player.read(cx);
        let has_song = player.snapshot().has_song();
        let panels = player.panels(window, cx);
        let is_queue_selected =
            panels.is_queue_shown && panels.queue_fits && !panels.is_lyrics_open;
        h_flex()
            .debug_selector(|| "panel-toggles".to_owned())
            .flex_none()
            .gap_1()
            .child(
                Button::new("toggle-lyrics")
                    .ghost()
                    .small()
                    .icon(IconName::MicVocal)
                    .accessibility_label("Lyrics")
                    .tooltip_with_action("Lyrics", &ToggleLyrics, None)
                    .selected(panels.is_lyrics_open)
                    .toggled(panels.is_lyrics_open)
                    .disabled(!has_song)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleLyrics), cx)),
            )
            .child(
                Button::new("toggle-queue")
                    .ghost()
                    .small()
                    .icon(IconName::ListMusic)
                    .accessibility_label("Queue")
                    .map(|this| {
                        if panels.queue_fits {
                            this.tooltip_with_action("Queue", &ToggleQueue, None)
                        } else {
                            this.tooltip("Widen the window to show the queue.")
                        }
                    })
                    .selected(is_queue_selected)
                    .toggled(is_queue_selected)
                    .disabled(!panels.queue_fits)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleQueue), cx)),
            )
            .child(
                Button::new("toggle-mini-player")
                    .ghost()
                    .small()
                    .icon(IconName::PictureInPicture2)
                    .accessibility_label("Mini player")
                    .tooltip_with_action("Mini player", &ToggleMiniPlayer, None)
                    .selected(panels.is_mini_player_open)
                    .toggled(panels.is_mini_player_open)
                    .disabled(!has_song)
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ToggleMiniPlayer), cx);
                    }),
            )
    }
}

#[derive(IntoElement)]
pub(crate) struct VolumeControl {
    player: Entity<Player>,
}

impl VolumeControl {
    pub(crate) fn new(player: Entity<Player>) -> Self {
        Self { player }
    }
}

impl RenderOnce for VolumeControl {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = self.player.read(cx);
        let shown = player.snapshot();
        let level = shown.volume();
        let (icon, label) = if shown.is_silent() {
            (IconName::VolumeX, "Unmute")
        } else if level < 0.5 {
            (IconName::Volume1, "Mute")
        } else {
            (IconName::Volume2, "Mute")
        };
        let slider = player.volume_state().clone();
        let is_locked = shown.is_volume_locked();
        let owner = self.player.clone();
        h_flex()
            .debug_selector(|| "volume-control".to_owned())
            .flex_none()
            .gap_2()
            .child(
                Button::new("toggle-mute")
                    .ghost()
                    .small()
                    .disabled(is_locked)
                    .icon(icon)
                    .accessibility_label(label)
                    .map(|button| {
                        if is_locked {
                            button.tooltip("Set the volume on the DAC while playing DSD")
                        } else {
                            button.tooltip_with_action(label, &crate::ToggleMute, None)
                        }
                    })
                    .on_click(move |_, _, cx| {
                        owner.read(cx).core().toggle_mute();
                    }),
            )
            .child(
                h_flex()
                    .w_20()
                    .child(HoverSlider::new(&slider, "Volume").disabled(is_locked)),
            )
    }
}

#[cfg(test)]
mod tests;
