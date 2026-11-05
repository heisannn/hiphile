//! The controls of a player, which both the bar and the lyrics view show.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    h_flex,
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
    CycleRepeat, NextSong, PreviousSong, ToggleLyrics, TogglePlayback, ToggleQueue, ToggleShuffle,
};

/// The kit's play and pause icons, filled in, as it has no such icons.
const PLAY_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/>
</svg>"#;
const PAUSE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
<rect x="14" y="3" width="5" height="18" rx="1"/>
<rect x="5" y="3" width="5" height="18" rx="1"/>
</svg>"#;

/// Shuffle, the way through the queue, and repeat.
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
        let player = self.player.read(cx);
        let has_song = player.has_song();
        let playing = player.is_playing();
        let shuffled = player.is_shuffled();
        let repeat = player.repeat();
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
        let on_click = |update: fn(&mut Player, &mut gpui_kit::Context<Player>)| {
            let player = self.player.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                player.update(cx, update);
            }
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
                    .on_click(on_click(Player::toggle_shuffle)),
            )
            .child(
                Button::new("previous-song")
                    .ghost()
                    .with_size(size)
                    .icon(IconName::SkipBack)
                    .accessibility_label("Previous")
                    .tooltip_with_action("Previous", &PreviousSong, None)
                    .disabled(!has_song)
                    .on_click(on_click(Player::previous)),
            )
            .child(
                Button::new("toggle-playback")
                    .ghost()
                    .with_size(size)
                    .icon(Icon::default().data(icon))
                    .accessibility_label(label)
                    .tooltip_with_action(label, &TogglePlayback, None)
                    .disabled(!has_song)
                    .on_click(on_click(Player::toggle_playback)),
            )
            .child(
                Button::new("next-song")
                    .ghost()
                    .with_size(size)
                    .icon(IconName::SkipForward)
                    .accessibility_label("Next")
                    .tooltip_with_action("Next", &NextSong, None)
                    .disabled(!has_song)
                    .on_click(on_click(Player::next)),
            )
            .child(
                Button::new("cycle-repeat")
                    .ghost()
                    .with_size(size)
                    .icon(repeat_icon)
                    .accessibility_label(repeat_label)
                    .tooltip_with_action(repeat_label, &CycleRepeat, None)
                    .selected(repeat != RepeatMode::Off)
                    .on_click(on_click(Player::cycle_repeat)),
            )
    }
}

/// Where the song is, on a line between the time gone and its length.
#[derive(IntoElement)]
pub(crate) struct PositionLine {
    player: Entity<Player>,
    /// How far open it is, from `0.0` to `1.0`.
    shown: f32,
    in_bar: bool,
}

impl PositionLine {
    /// The bar's line, which opens from nothing by `shown`. Fixed lanes for
    /// the times keep the line from moving as the digits change.
    pub(crate) fn in_bar(player: Entity<Player>, shown: f32) -> Self {
        Self {
            player,
            shown,
            in_bar: true,
        }
    }

    /// A line whose times reach the edges of the parent.
    pub(crate) fn full_width(player: Entity<Player>) -> Self {
        Self {
            player,
            shown: 1.,
            in_bar: false,
        }
    }
}

impl RenderOnce for PositionLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = self.player.read(cx);
        let duration = player.duration();
        let seekable = player.is_started() && duration.is_some();
        let position = player.shown_position();
        let buffering = player.is_buffering();
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
            .when(player.has_song(), |this| {
                this.child(
                    h_flex()
                        .flex_none()
                        .when(self.in_bar, |this| this.min_w_9().justify_end())
                        .relative()
                        // Keeps its room, so that the line does not move.
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
                .child(HoverSlider::new(player.position_state(), "Position").disabled(!seekable))
                .child(
                    div()
                        .flex_none()
                        .when(self.in_bar, |this| this.min_w_9())
                        .child(duration.map(format_duration).unwrap_or_default()),
                )
            })
    }
}

/// The lyrics and the queue, whoever lays those out.
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
        let has_song = player.has_song();
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
    }
}

/// Mute and the volume.
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
        let level = player.level(cx);
        let (icon, label) = if player.is_muted() || level == 0.0 {
            (IconName::VolumeX, "Unmute")
        } else if level < 0.5 {
            (IconName::Volume1, "Mute")
        } else {
            (IconName::Volume2, "Mute")
        };
        let slider = player.volume_state().clone();
        let owner = self.player.clone();
        h_flex()
            .debug_selector(|| "volume-control".to_owned())
            .flex_none()
            .gap_2()
            .child(
                Button::new("toggle-mute")
                    .ghost()
                    .small()
                    .icon(icon)
                    .accessibility_label(label)
                    .tooltip_with_action(label, &crate::ToggleMute, None)
                    .on_click(move |_, _, cx| {
                        owner.update(cx, |player, cx| player.toggle_mute(cx));
                    }),
            )
            .child(h_flex().w_20().child(HoverSlider::new(&slider, "Volume")))
    }
}

#[cfg(test)]
mod tests;
