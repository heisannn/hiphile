use account::{SignIn, SignInEvent};
use actions::PlaySongs;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use library::Library;
use player::{NextSong, Player, PreviousSong, TogglePlayback};
use session::Session;
use ui::CoverCache;

use crate::SignOut;

/// Key context of the whole window; player shortcuts are bound to it.
pub const WORKSPACE_CONTEXT: &str = "Workspace";

/// Root view of the main window. Decides between signing in and the signed-in
/// workspace, and routes commands between features; it owns no feature state.
pub struct AppShell {
    net: net::Client,
    /// Keeps a focus target in the window, so menu and keyboard commands
    /// always reach this view's handlers.
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
    library: Entity<Library>,
    player: Entity<Player>,
    /// Holds the session's decoded covers, so signing out frees them.
    covers: Entity<CoverCache>,
}

impl AppShell {
    pub fn new(net: net::Client, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title("hiphile");
        let focus = cx.focus_handle();
        let screen = Self::sign_in(&net, window, cx);
        Self { net, focus, screen }
    }

    fn sign_in(net: &net::Client, window: &mut Window, cx: &mut Context<Self>) -> Screen {
        let view = cx.new(|cx| SignIn::new(net.clone(), window, cx));
        let subscription = cx.subscribe_in(&view, window, |_, _, event, window, cx| {
            let SignInEvent::SignedIn(session) = event;
            let session = session.clone();
            // Replacing the screen drops this subscription; do it after the
            // event has been delivered.
            cx.defer_in(window, move |this, window, cx| {
                this.open_workspace(session, window, cx)
            });
        });
        Screen::SignIn {
            view,
            _subscription: subscription,
        }
    }

    fn open_workspace(&mut self, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        let library = cx.new(|cx| Library::new(session.clone(), window, cx));
        let player = cx.new(|cx| Player::new(session.clone(), cx));
        let covers = CoverCache::new(cx);
        self.screen = Screen::Workspace(Box::new(Workspace {
            session,
            library,
            player,
            covers,
        }));
        cx.notify();
    }

    /// Drops the workspace, which stops playback, and returns to signing in.
    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.screen, Screen::Workspace(_)) {
            self.focus.focus(window, cx);
            self.screen = Self::sign_in(&self.net, window, cx);
            cx.notify();
        }
    }

    fn player(&self) -> Option<&Entity<Player>> {
        match &self.screen {
            Screen::Workspace(workspace) => Some(&workspace.player),
            Screen::SignIn { .. } => None,
        }
    }

    fn update_player(
        &self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Player, &mut Context<Player>),
    ) {
        if let Some(player) = self.player() {
            player.update(cx, update);
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
        v_flex()
            .when_some(covers, |this, covers| this.image_cache(covers))
            .id("app-shell")
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &SignOut, window, cx| this.sign_out(window, cx)))
            .on_action(cx.listener(|this, action: &PlaySongs, _, cx| {
                this.update_player(cx, |player, cx| {
                    player.play(action.songs(), action.start_ix(), cx)
                });
            }))
            .on_action(cx.listener(|this, _: &TogglePlayback, _, cx| {
                this.update_player(cx, Player::toggle_playback);
            }))
            .on_action(cx.listener(|this, _: &NextSong, _, cx| {
                this.update_player(cx, Player::next);
            }))
            .on_action(cx.listener(|this, _: &PreviousSong, _, cx| {
                this.update_player(cx, Player::previous);
            }))
            .child(self.render_title_bar(cx))
            .child(div().flex_1().min_h_0().child(content))
            .when_some(self.player().cloned(), |this, player| this.child(player))
    }
}
