use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, StyledExt as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, DisplayId, Entity, FocusHandle, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowId,
    WindowKind, WindowOptions, div, point, px, size,
};
use subsonic::Song;
use ui::list_navigation::CONTROL_CONTEXT;
use ui::{CoverArt, CoverCache};

use lyrics::CurrentLyrics;

use crate::lyrics_view::{LinesFit, LinesType, LyricsLines};
use crate::player::Player;
use crate::transport::{PositionLine, TransportControls};
use crate::{
    CloseMiniPlayer, CycleRepeat, NextSong, PreviousSong, SeekBackward, SeekForward, SetRepeat,
    ToggleMiniPlayer, ToggleMute, TogglePlayback, ToggleShuffle, VolumeDown, VolumeUp,
};

pub(crate) const MINI_PLAYER_CONTEXT: &str = "MiniPlayer";

/// Height of what is around the lines, in rems, near enough for where they rest.
const AROUND_LINES: f32 = 11.;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    bounds: Bounds<Pixels>,
    display: Option<DisplayId>,
}

struct CurrentMiniPlayer(WeakEntity<MiniPlayer>);

impl Global for CurrentMiniPlayer {}

/// On macOS the window, being a panel, can become the main window, which GPUI
/// then takes for no window at all, so that menu commands reach only global
/// listeners. This is what lets View > Mini Player close it from there.
pub(crate) fn init(cx: &mut App) {
    cx.on_action(|_: &ToggleMiniPlayer, cx| {
        let Some(CurrentMiniPlayer(mini_player)) = cx.try_global::<CurrentMiniPlayer>() else {
            return;
        };
        let mini_player = mini_player.clone();
        mini_player
            .update(cx, |mini_player, cx| mini_player.close(cx))
            .ok();
    });
}

/// Owns the window of the mini player, which goes when this does, when the song
/// runs out, or when the window it was made for closes.
pub struct MiniPlayer {
    player: Entity<Player>,
    lyrics: CurrentLyrics,
    window: Option<AnyWindowHandle>,
    /// Where the window last was, for it to return there. The window reports it
    /// while this may be in use, opening it, so it is not reached through this.
    placement: Rc<Cell<Option<Placement>>>,
    _windows_closed: Subscription,
    _released: Subscription,
    _player_changes: Task<()>,
}

impl MiniPlayer {
    pub fn new(
        player: Entity<Player>,
        lyrics: CurrentLyrics,
        main_window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let main_window = main_window.window_handle().window_id();
        let this = cx.weak_entity();
        let windows_closed = cx.on_window_closed(move |cx, closed| {
            this.update(cx, |this, cx| {
                this.on_window_closed(closed, main_window, cx);
            })
            .ok();
        });
        let released = cx.on_release(|this, cx| this.close_window(cx));
        let changes = player.read(cx).core().subscribe();
        let player_changes = gpui_host::observe(changes, cx, |this, shown, cx| {
            if !shown.has_song() {
                this.close(cx);
            }
        });
        let this = cx.weak_entity();
        cx.set_global(CurrentMiniPlayer(this));
        Self {
            player,
            lyrics,
            window: None,
            placement: Rc::default(),
            _windows_closed: windows_closed,
            _released: released,
            _player_changes: player_changes,
        }
    }

    pub fn is_open(&self) -> bool {
        self.window.is_some()
    }

    pub fn toggle(&mut self, main_window: &Window, cx: &mut Context<Self>) {
        if self.is_open() {
            self.close(cx);
        } else {
            self.open(main_window, cx);
        }
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.close_window(cx);
        cx.notify();
    }

    fn close_window(&mut self, cx: &mut App) {
        if let Some(window) = self.window.take() {
            close_mini_window(&self.player, window, cx);
        }
    }

    fn on_window_closed(
        &mut self,
        closed: WindowId,
        main_window: WindowId,
        cx: &mut Context<Self>,
    ) {
        if closed == main_window {
            self.close(cx);
        } else if self
            .window
            .is_some_and(|window| window.window_id() == closed)
        {
            self.window = None;
            self.player.update(cx, Player::finish_mini_position_drag);
            cx.notify();
        }
    }

    fn open(&mut self, main_window: &Window, cx: &mut Context<Self>) {
        // Window geometry is a platform boundary, so it is given in pixels
        let placement = self.placement.get().unwrap_or_else(|| {
            let main = main_window.bounds();
            let width = px(300.);
            Placement {
                bounds: Bounds::new(
                    point(main.right() - width - px(16.), main.top() + px(50.)),
                    size(width, px(420.)),
                ),
                display: main_window.display(cx).map(|display| display.id()),
            }
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(placement.bounds)),
            window_min_size: Some(size(px(260.), px(320.))),
            display_id: placement.display,
            // Above the windows of every app, without taking the keyboard from
            // what the user was doing
            kind: WindowKind::PopUp,
            focus: false,
            is_minimizable: false,
            titlebar: Some(TitlebarOptions {
                title: Some("Mini Player".into()),
                ..TitleBar::title_bar_options()
            }),
            ..TitleBar::window_options()
        };
        let player = self.player.clone();
        let lyrics = self.lyrics.clone();
        let last_placement = self.placement.clone();
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| MiniPlayerView::new(player, lyrics, last_placement, window, cx))
        });
        if let Ok((window, _)) = opened {
            self.window = Some(window);
            cx.notify();
        }
    }
}

/// Closes `window` only once what is under way has been told. A slider's events
/// reach the player through the window that last drew it, which may be this one.
/// Waiting also spares the caller, as a closing window tells every observer at
/// once, and that includes the [`MiniPlayer`], which the caller may be using.
fn close_mini_window(player: &Entity<Player>, window: AnyWindowHandle, cx: &mut App) {
    player.update(cx, Player::finish_mini_position_drag);
    cx.defer(move |cx| {
        window
            .update(cx, |_, window, _| window.remove_window())
            .ok();
    });
}

struct MiniPlayerView {
    player: Entity<Player>,
    lines: Entity<LyricsLines>,
    /// One for this window, as a cache drops what the window it is in stops using.
    covers: Entity<CoverCache>,
    focus: FocusHandle,
    _subscriptions: [Subscription; 3],
}

impl MiniPlayerView {
    fn new(
        player: Entity<Player>,
        lyrics: CurrentLyrics,
        last_placement: Rc<Cell<Option<Placement>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let lines = cx.new(|cx| {
            let mut lines = LyricsLines::new(player.clone(), lyrics, lines_fit, cx);
            lines.set_active(true, cx);
            lines
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let subscriptions = [
            cx.observe(&player, |_, _, cx| cx.notify()),
            cx.observe_window_bounds(window, move |_, window, cx| {
                let WindowBounds::Windowed(bounds) = window.window_bounds() else {
                    return;
                };
                let display = window.display(cx).map(|display| display.id());
                last_placement.set(Some(Placement { bounds, display }));
            }),
            // A focused control that goes away, such as Retry once loading starts,
            // would leave no focus and no working shortcuts.
            cx.on_focus_lost(window, |this, window, cx| this.focus.focus(window, cx)),
        ];
        Self {
            player,
            lines,
            covers: CoverCache::new(cx),
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn render_song(&self, song: &Song, cx: &Context<Self>) -> impl IntoElement + use<> {
        let player = self.player.read(cx);
        let cover = player.cover_urls().0;
        let theme = cx.theme();
        // No links as in the bar: where they lead is in the other window
        let status = match player.snapshot().error() {
            Some(error) => div()
                .debug_selector(|| "mini-player-error".to_owned())
                .text_color(theme.danger)
                .child(SharedString::from(error.to_owned())),
            None => div()
                .text_color(theme.muted_foreground)
                .child(song_details(song)),
        };
        h_flex()
            .debug_selector(|| "mini-player-song".to_owned())
            .flex_none()
            .gap_3()
            .px_4()
            .py_3()
            .child(
                div()
                    .flex_none()
                    .size_12()
                    .child(CoverArt::new(cover).fill().rounded_by(theme.radius)),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .truncate()
                            .child(SharedString::from(song.title().to_owned())),
                    )
                    .child(status.text_xs().truncate()),
            )
    }
}

impl Render for MiniPlayerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let song = self.player.read(cx).snapshot().current_song().cloned();
        let song = song.map(|song| self.render_song(&song, cx));
        let position = self.player.read(cx).mini_position_state().clone();
        v_flex()
            .image_cache(self.covers.clone())
            .id("mini-player")
            .key_context(MINI_PLAYER_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &ToggleMiniPlayer, window, cx| {
                close_mini_window(&this.player, window.window_handle(), cx);
            }))
            .on_action(cx.listener(|this, _: &CloseMiniPlayer, window, cx| {
                close_mini_window(&this.player, window.window_handle(), cx);
            }))
            .on_action(cx.listener(|this, _: &TogglePlayback, _, cx| {
                this.player.read(cx).core().toggle_playback();
            }))
            .on_action(cx.listener(|this, _: &NextSong, _, cx| {
                this.player.read(cx).core().next();
            }))
            .on_action(cx.listener(|this, _: &PreviousSong, _, cx| {
                this.player.read(cx).core().previous();
            }))
            .on_action(cx.listener(|this, _: &ToggleMute, _, cx| {
                this.player.read(cx).core().toggle_mute();
            }))
            .on_action(cx.listener(|this, _: &ToggleShuffle, _, cx| {
                this.player.read(cx).core().toggle_shuffle();
            }))
            .on_action(cx.listener(|this, _: &CycleRepeat, _, cx| {
                this.player.read(cx).core().cycle_repeat();
            }))
            .on_action(cx.listener(|this, action: &SetRepeat, _, cx| {
                this.player.read(cx).core().set_repeat(action.repeat());
            }))
            .on_action(cx.listener(|this, _: &SeekForward, window, cx| {
                this.player
                    .update(cx, |player, cx| player.seek_forward(window, cx));
            }))
            .on_action(cx.listener(|this, _: &SeekBackward, window, cx| {
                this.player
                    .update(cx, |player, cx| player.seek_backward(window, cx));
            }))
            .on_action(cx.listener(|this, _: &VolumeUp, _, cx| {
                this.player.read(cx).core().volume_up();
            }))
            .on_action(cx.listener(|this, _: &VolumeDown, _, cx| {
                this.player.read(cx).core().volume_down();
            }))
            .child(TitleBar::new())
            .children(song)
            .child(div().flex_1().min_h_0().child(self.lines.clone()))
            .child(
                v_flex()
                    .debug_selector(|| "mini-player-controls".to_owned())
                    .key_context(CONTROL_CONTEXT)
                    .flex_none()
                    .gap_1()
                    .px_4()
                    .pt_3()
                    .pb_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .debug_selector(|| "mini-player-position".to_owned())
                            .child(PositionLine::full_width(self.player.clone()).slider(position)),
                    )
                    .child(
                        h_flex()
                            .justify_center()
                            .child(TransportControls::new(self.player.clone())),
                    ),
            )
    }
}

fn lines_fit(window: &Window) -> LinesFit {
    LinesFit {
        height: window.viewport_size().height - window.rem_size() * AROUND_LINES,
        line_type: LinesType::Small,
    }
}

fn song_details(song: &Song) -> SharedString {
    let parts: Vec<&str> = [song.artist(), song.album()]
        .into_iter()
        .flatten()
        .collect();
    parts.join(" — ").into()
}

#[cfg(test)]
mod tests;
