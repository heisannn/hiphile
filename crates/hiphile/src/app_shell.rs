use account::{SignIn, SignInEvent, Start};
use actions::{EnqueueSongs, PlaySongs};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    resizable::{ResizableState, h_resizable, resizable_panel},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use library::Library;
use player::{
    NextSong, Player, PlayerEvent, PreviousSong, QueueView, SeekBackward, SeekForward, ToggleMute,
    TogglePlayback, VolumeDown, VolumeUp,
};
use session::Session;
use ui::CoverCache;

use crate::{SignOut, ToggleQueue};

/// The queue's width when first shown, in rem.
const QUEUE_WIDTH: f32 = 20.;

/// Key context of the whole window; player shortcuts are bound to it.
pub const WORKSPACE_CONTEXT: &str = "Workspace";

/// Root view of the main window. Decides between signing in and the signed-in
/// workspace, and routes commands between features; it owns no feature state.
pub struct AppShell {
    net: net::Client,
    /// `None` where the system has no credential store; signing in then
    /// always asks for the password.
    keychain: Option<config::Keychain>,
    /// A focus target for the sign-in screen while it has no focused field,
    /// so menu commands still reach this view. Signed in, the library's
    /// lists hold focus instead: a click on empty space then leaves focus
    /// where it was rather than moving it here.
    focus: FocusHandle,
    screen: Screen,
    /// Whether the queue shows beside the library when the window is wide
    /// enough for it.
    queue_shown: bool,
    _song_cache_changes: Subscription,
}

enum Screen {
    SignIn {
        view: Entity<SignIn>,
        _subscription: Subscription,
    },
    Workspace(Box<Workspace>),
}

/// Everything that exists only while signed in.
struct Workspace {
    session: Session,
    profile: config::Profile,
    library: Entity<Library>,
    player: Entity<Player>,
    queue: Entity<QueueView>,
    /// The split between the library and the queue.
    panes: Entity<ResizableState>,
    /// Holds the session's decoded covers, so signing out frees them.
    covers: Entity<CoverCache>,
    _player_events: Subscription,
    _output_changes: Subscription,
}

impl AppShell {
    pub fn new(net: net::Client, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title("hiphile");
        let focus = cx.focus_handle();
        let keychain = config::Keychain::system().ok();
        let screen = Self::sign_in(&net, keychain.clone(), Start::Resume, window, cx);
        let queue_shown = true;
        crate::set_menus(queue_shown, cx);
        let song_cache_changes = cx.subscribe(
            &settings::song_cache_settings(cx),
            |_, song_cache, _: &settings::SongCacheChanged, cx| {
                let capacity = song_cache.read(cx).capacity();
                player::set_song_capacity(capacity, cx);
            },
        );
        Self {
            net,
            keychain,
            focus,
            screen,
            queue_shown,
            _song_cache_changes: song_cache_changes,
        }
    }

    fn sign_in(
        net: &net::Client,
        keychain: Option<config::Keychain>,
        start: Start,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Screen {
        let view = cx.new(|cx| SignIn::new(net.clone(), keychain, start, window, cx));
        let subscription = cx.subscribe_in(&view, window, |_, _, event, window, cx| {
            let SignInEvent::SignedIn { session, profile } = event;
            let (session, profile) = (session.clone(), profile.clone());
            // Replacing the screen drops this subscription; do it after the
            // event has been delivered.
            cx.defer_in(window, move |this, window, cx| {
                this.open_workspace(session, profile, window, cx)
            });
        });
        Screen::SignIn {
            view,
            _subscription: subscription,
        }
    }

    fn open_workspace(
        &mut self,
        session: Session,
        profile: config::Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let library = cx.new(|cx| Library::new(session.clone(), window, cx));
        let audio_settings = settings::audio_settings(cx);
        let output = audio_settings.read(cx).output().clone();
        let player = cx.new(|cx| Player::new(session.clone(), output, window, cx));
        let queue = cx.new(|cx| QueueView::new(player.clone(), window, cx));
        let covers = CoverCache::new(cx);
        // The library marks the song the player is on; the shell also
        // re-renders, which updates which commands are available.
        let player_events = cx.subscribe(&player, {
            let library = library.downgrade();
            move |_, _, event, cx| {
                if let PlayerEvent::NowPlaying { song, playing } = event {
                    library
                        .update(cx, |library, cx| {
                            library.set_now_playing(song.clone(), *playing, cx)
                        })
                        .ok();
                }
                cx.notify();
            }
        });
        let output_changes = cx.subscribe(&audio_settings, {
            let player = player.downgrade();
            move |_, audio_settings, _: &settings::OutputChanged, cx| {
                let output = audio_settings.read(cx).output().clone();
                player
                    .update(cx, |player, cx| player.set_output(output, cx))
                    .ok();
            }
        });
        self.screen = Screen::Workspace(Box::new(Workspace {
            session,
            profile,
            library,
            player,
            queue,
            panes: cx.new(|_| ResizableState::default()),
            covers,
            _player_events: player_events,
            _output_changes: output_changes,
        }));
        cx.notify();
    }

    /// Drops the workspace, which stops playback, forgets the saved
    /// password, and returns to signing in.
    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Workspace(workspace) = &self.screen else {
            return;
        };
        if let Some(keychain) = self.keychain.clone() {
            let profile = workspace.profile.clone();
            cx.background_spawn(async move { keychain.delete_password(&profile) })
                .detach();
        }
        self.focus.focus(window, cx);
        self.screen = Self::sign_in(&self.net, self.keychain.clone(), Start::Fresh, window, cx);
        cx.notify();
    }

    fn player(&self) -> Option<&Entity<Player>> {
        match &self.screen {
            Screen::Workspace(workspace) => Some(&workspace.player),
            Screen::SignIn { .. } => None,
        }
    }

    fn update_player(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Player, &mut Window, &mut Context<Player>),
    ) {
        if let Some(player) = self.player() {
            player.update(cx, |player, cx| update(player, window, cx));
        }
    }

    fn toggle_queue(&mut self, cx: &mut Context<Self>) {
        self.queue_shown = !self.queue_shown;
        crate::set_menus(self.queue_shown, cx);
        cx.notify();
    }

    /// Whether the window is wide enough for the queue beside the library
    /// at the widths their panes have now. The queue keeps its width as the
    /// window narrows, so past this point it closes instead of pushing the
    /// library out of the window.
    fn queue_fits(&self, window: &Window, cx: &App) -> bool {
        let Screen::Workspace(workspace) = &self.screen else {
            return false;
        };
        let queue = workspace
            .panes
            .read(cx)
            .sizes()
            .get(1)
            .copied()
            .filter(|width| !gpui_kit::IsZero::is_zero(width))
            .unwrap_or(window.rem_size() * QUEUE_WIDTH);
        let library = workspace.library.read(cx).min_width(window, cx);
        window.viewport_size().width >= library + queue
    }

    /// The library and, when shown, the queue to its right.
    fn render_workspace(
        &self,
        workspace: &Workspace,
        queue_visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rem = window.rem_size();
        let library_min = workspace.library.read(cx).min_width(window, cx);
        h_resizable("workspace-panes")
            .with_state(&workspace.panes)
            .child(
                resizable_panel()
                    .size_range(library_min..rem * 240.)
                    .child(workspace.library.clone()),
            )
            .child(
                resizable_panel()
                    .size(rem * QUEUE_WIDTH)
                    .size_range(rem * 16. ..rem * 28.)
                    .fixed()
                    .visible(queue_visible)
                    .child(workspace.queue.clone()),
            )
            .into_any_element()
    }

    fn render_title_bar(
        &self,
        queue_visible: bool,
        queue_fits: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let server = match &self.screen {
            Screen::Workspace(workspace) => Some(&workspace.session),
            Screen::SignIn { .. } => None,
        };
        TitleBar::new()
            .child(
                h_flex()
                    .size_full()
                    .px_1()
                    .pr_3()
                    .gap_2()
                    .when_some(server, |this, session| {
                        let software = match (
                            session.server().server_type(),
                            session.server().server_version(),
                        ) {
                            (Some(name), Some(version)) => Some(format!("{name} {version}")),
                            (Some(name), None) => Some(name.to_owned()),
                            _ => None,
                        };
                        this.child(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(SharedString::from(session.host().to_owned())),
                        )
                        .when_some(software, |this, software| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(software),
                            )
                        })
                        .child(div().flex_1())
                        .child(
                            Button::new("toggle-queue")
                                .ghost()
                                .small()
                                .icon(IconName::ListMusic)
                                .selected(queue_visible)
                                .disabled(!queue_fits)
                                .accessibility_label("Queue")
                                .map(|this| {
                                    if queue_fits {
                                        this.tooltip_with_action("Queue", &ToggleQueue, None)
                                    } else {
                                        this.tooltip("Widen the window to show the queue.")
                                    }
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_queue(cx))),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let queue_fits = self.queue_fits(window, cx);
        let queue_visible = self.queue_shown && queue_fits;
        let content: AnyElement = match &self.screen {
            Screen::SignIn { view, .. } => view.clone().into_any_element(),
            Screen::Workspace(workspace) => {
                self.render_workspace(workspace, queue_visible, window, cx)
            }
        };
        let covers = match &self.screen {
            Screen::Workspace(workspace) => Some(workspace.covers.clone()),
            Screen::SignIn { .. } => None,
        };
        let signed_in = matches!(self.screen, Screen::Workspace(_));
        let has_song = self
            .player()
            .is_some_and(|player| player.read(cx).has_song());
        // Commands are registered only where they do something, so menus and
        // shortcuts are disabled otherwise.
        v_flex()
            .when_some(covers, |this, covers| this.image_cache(covers))
            .id("app-shell")
            .key_context(WORKSPACE_CONTEXT)
            .when(!signed_in, |this| this.track_focus(&self.focus))
            .size_full()
            .min_h_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .when(signed_in, |this| {
                this.on_action(
                    cx.listener(|this, _: &SignOut, window, cx| this.sign_out(window, cx)),
                )
                .on_action(cx.listener(|this, action: &PlaySongs, window, cx| {
                    this.update_player(window, cx, |player, _, cx| {
                        player.play(action.songs(), action.start_ix(), cx)
                    });
                }))
                .on_action(cx.listener(|this, action: &EnqueueSongs, window, cx| {
                    let placement = match action.placement() {
                        actions::Placement::Next => player::Placement::Next,
                        actions::Placement::Last => player::Placement::Last,
                    };
                    this.update_player(window, cx, |player, _, cx| {
                        player.enqueue(action.songs(), placement, cx)
                    });
                }))
                .on_action(cx.listener(|this, _: &VolumeUp, window, cx| {
                    this.update_player(window, cx, Player::volume_up);
                }))
                .on_action(cx.listener(|this, _: &VolumeDown, window, cx| {
                    this.update_player(window, cx, Player::volume_down);
                }))
                .on_action(cx.listener(|this, _: &ToggleMute, window, cx| {
                    this.update_player(window, cx, |player, _, cx| player.toggle_mute(cx));
                }))
            })
            .when(queue_fits, |this| {
                this.on_action(cx.listener(|this, _: &ToggleQueue, _, cx| this.toggle_queue(cx)))
            })
            .when(has_song, |this| {
                this.on_action(cx.listener(|this, _: &TogglePlayback, window, cx| {
                    this.update_player(window, cx, |player, _, cx| player.toggle_playback(cx));
                }))
                .on_action(cx.listener(|this, _: &NextSong, window, cx| {
                    this.update_player(window, cx, |player, _, cx| player.next(cx));
                }))
                .on_action(cx.listener(|this, _: &PreviousSong, window, cx| {
                    this.update_player(window, cx, |player, _, cx| player.previous(cx));
                }))
                .on_action(cx.listener(|this, _: &SeekForward, window, cx| {
                    this.update_player(window, cx, Player::seek_forward);
                }))
                .on_action(cx.listener(|this, _: &SeekBackward, window, cx| {
                    this.update_player(window, cx, Player::seek_backward);
                }))
            })
            .child(self.render_title_bar(queue_visible, queue_fits, cx))
            .child(div().flex_1().min_h_0().child(content))
            .when_some(self.player().cloned(), |this, player| this.child(player))
    }
}
