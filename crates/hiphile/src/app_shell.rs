use account::{SignIn, SignInEvent, Start};
use actions::PlaySongs;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use library::Library;
use player::{
    NextSong, Player, PlayerEvent, PreviousSong, SeekBackward, SeekForward, ToggleMute,
    TogglePlayback, VolumeDown, VolumeUp,
};
use session::Session;
use ui::CoverCache;

use crate::SignOut;

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
    /// Holds the session's decoded covers, so signing out frees them.
    covers: Entity<CoverCache>,
    _player_events: Subscription,
}

impl AppShell {
    pub fn new(net: net::Client, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title("hiphile");
        let focus = cx.focus_handle();
        let keychain = config::Keychain::system().ok();
        let screen = Self::sign_in(&net, keychain.clone(), Start::Resume, window, cx);
        Self {
            net,
            keychain,
            focus,
            screen,
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
        let player = cx.new(|cx| Player::new(session.clone(), window, cx));
        let covers = CoverCache::new(cx);
        // The library marks the song the player is on; the shell also
        // re-renders, which updates which commands are available.
        let player_events = cx.subscribe(&player, {
            let library = library.downgrade();
            move |_, _, event, cx| {
                let PlayerEvent::NowPlaying { song, playing } = event;
                library
                    .update(cx, |library, cx| {
                        library.set_now_playing(song.clone(), *playing, cx)
                    })
                    .ok();
                cx.notify();
            }
        });
        self.screen = Screen::Workspace(Box::new(Workspace {
            session,
            profile,
            library,
            player,
            covers,
            _player_events: player_events,
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

    fn render_title_bar(&self, cx: &mut Context<Self>) -> AnyElement {
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
                    }),
            )
            .into_any_element()
    }
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = match &self.screen {
            Screen::SignIn { view, .. } => view.clone().into_any_element(),
            Screen::Workspace(workspace) => workspace.library.clone().into_any_element(),
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
            .child(self.render_title_bar(cx))
            .child(div().flex_1().min_h_0().child(content))
            .when_some(self.player().cloned(), |this, player| this.child(player))
    }
}
