use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{IconButtonStyle, Snackbars, snackbars, styled_icon_button};
use anti_webui_md3e::icons;
use hiphile_core::navigation::{Destination, Location};
use hiphile_core::settings::AppearanceSnapshot;
use hiphile_core::{Hiphile, HiphileSnapshot, Workspace};
use next_account::{sign_in, sign_in_dialog};
use next_host::follow;
use next_player::{Panels, cover_seed, lyrics_overlay, now_playing_cover, player_bar, queue};
use next_search::search;
use next_settings::settings_popup;
use next_ui::cover::install_covers;
use next_ui::list_detail::{DETAIL_MIN_WIDTH, LIST_MIN_WIDTH};
use next_ui::pane::GAP;
use next_ui::theme::{follow_theme, is_dark};
use next_ui::type_scale::body;
use next_ui::window::moves_window;
use next_ui::{Screen, ScreenContext, app_bar};

use crate::MenuBar;
use crate::menu_bar::Shown;
use crate::navigation::{Navigation, index_of};
use crate::rail::rail;
use crate::reach::{Reach, show_outside};
use crate::top_end::top_end;

mod commands;

use commands::Shell;

/// Narrower than this, the rail closes over its labels.
const WIDE: f32 = 1000.0;
/// What the screens keep beside the queue: a list and a detail at their least.
const SCREENS_MIN_WIDTH: f32 = LIST_MIN_WIDTH + GAP + DETAIL_MIN_WIDTH;

/// What the system asks of the window.
#[derive(Clone, Copy)]
pub struct System {
    pub is_dark: Signal<bool>,
    /// Whether what moves on its way somewhere goes there at once instead.
    pub reduces_motion: Signal<bool>,
}

impl System {
    /// A system that stays dark or light and asks for every motion.
    pub fn fixed(is_dark: bool) -> Self {
        Self {
            is_dark: signal(is_dark),
            reduces_motion: signal(false),
        }
    }
}

/// What fills the window: signing in, or the workspace of whoever is signed in. Its colors follow
/// the appearance chosen, the system's when that follows it, and the cover of what plays. The
/// workspace takes what is chosen in `menu_bar`, and shows there where it is.
pub fn app(hiphile: Hiphile, system: System, menu_bar: MenuBar, reach: Reach) -> Div {
    let appearance = follow(hiphile.settings().appearance().subscribe());
    let seed = signal(None::<Color>);
    follow_theme(
        move || {
            is_dark(
                appearance.with(AppearanceSnapshot::appearance),
                system.is_dark.get(),
            )
        },
        move || seed.get(),
        move || system.reduces_motion.get(),
    );
    install_covers(hiphile.clone());
    let shown = follow(hiphile.subscribe());
    let workspace = memo(move || shown.with(|shown| shown.workspace().cloned()));
    let snackbars = snackbars();
    body(div())
        .size_full()
        .relative()
        .dyn_style(|s| {
            let c = color_scheme();
            s.bg(c.surface).text_color(c.on_surface)
        })
        .child(dyn_view(move || {
            let hiphile = hiphile.clone();
            match workspace.get() {
                None => untrack(|| signed_out(hiphile)),
                Some(workspace) => untrack(|| {
                    let parts = Parts {
                        snackbars,
                        menu_bar,
                        seed,
                        reach,
                        reduces_motion: system.reduces_motion,
                    };
                    signed_in(hiphile, workspace, shown, parts)
                }),
            }
        }))
        .child(snackbars.view())
}

fn signed_out(hiphile: Hiphile) -> Div {
    div()
        .size_full()
        .relative()
        .child(sign_in(hiphile))
        .child(moves_window(div().absolute().top(0).left(0).right(0).h(44)))
}

/// What the workspace's window is made with besides the workspace.
struct Parts {
    snackbars: Snackbars,
    menu_bar: MenuBar,
    /// Where the colors taken from the cover go.
    seed: Signal<Option<Color>>,
    reach: Reach,
    reduces_motion: Signal<bool>,
}

fn signed_in(
    hiphile: Hiphile,
    workspace: Workspace,
    shown: Signal<HiphileSnapshot>,
    parts: Parts,
) -> Div {
    let Parts {
        snackbars,
        menu_bar,
        seed,
        reach,
        reduces_motion,
    } = parts;
    let cover = now_playing_cover(&hiphile, &workspace);
    let colors = cover_seed(cover);
    effect(move || seed.set(colors.get()));
    on_cleanup(move || {
        if seed.is_alive() {
            seed.set(None);
        }
    });
    show_outside(&workspace, reach);
    let player = workspace.player().clone();
    let playing = follow(player.subscribe());
    let navigation = Navigation::new();
    effect({
        let workspace = workspace.clone();
        move || workspace.showing(navigation.destination())
    });
    let width = signal(crate::WINDOW.0 as f32);
    let wide = memo(move || width.get() >= WIDE);
    let expanded = signal(true);
    effect(move || expanded.set(wide.get()));
    let settings_open = signal(false);
    let search_open = signal(false);
    let sign_in_request = signal(None);
    let top_end_width = signal(0.0_f32);
    let shell = Shell::new(
        &hiphile,
        workspace,
        navigation,
        settings_open,
        search_open,
        snackbars,
    );
    let cx = shell.context();
    let (panels, dialogs, covers) = (shell.panels, shell.dialogs, cx.covers);
    // The lyrics are of the song playing
    let has_song = memo(move || playing.with(|playing| playing.has_song()));
    effect(move || {
        if !has_song.get() {
            untrack(|| panels.lyrics.set_if_changed(false));
        }
    });
    menu_bar.attach(cx.commands.clone(), move || {
        let (has_song, has_queue, shuffled, repeat) = playing.with(|playing| {
            (
                playing.has_song(),
                playing.has_queued_songs(),
                playing.is_shuffled(),
                playing.repeat(),
            )
        });
        Shown {
            destination: Some(navigation.destination()),
            can_go_back: navigation.can_go_back(),
            can_go_forward: navigation.can_go_forward(),
            covered: settings_open.get()
                || search_open.get()
                || sign_in_request.with(Option::is_some)
                || dialogs.is_open()
                || covers.any(),
            has_song,
            has_queue,
            shuffled,
            repeat: Some(repeat),
            queue_fits: panels.queue_fits(),
            queue_shown: panels.is_queue_in_view(),
            lyrics_open: panels.lyrics.get(),
        }
    });
    // What the lyrics cover takes no keys and no Tab
    let covered = move || panels.is_covered();
    div()
        .size_full()
        .relative()
        .flex()
        .flex_row()
        .on_resized(move |w, _| width.set(w))
        .child(rail(navigation, expanded, cx.commands.clone()).disabled(covered))
        .child(
            div()
                .disabled(covered)
                .flex_1()
                .min_w(0)
                .h_full()
                .relative()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_1()
                        .min_h(0)
                        .flex()
                        .flex_row()
                        // Less the handle before the queue and the gap after it
                        .on_resized(move |w, _| {
                            panels.set_queue_room(w - SCREENS_MIN_WIDTH - 2.0 * GAP);
                        })
                        .child(div().flex_1().min_w(0).flex().flex_col().child(for_each(
                            move || vec![navigation.destination()],
                            |destination| index_of(*destination),
                            {
                                let cx = cx.clone();
                                move |destination| {
                                    let Screen {
                                        title,
                                        actions,
                                        body,
                                    } = screen(destination, navigation.location(), &cx);
                                    // The queue's own bar takes the window's top end while it shows
                                    let bar = div().flex_none().dyn_style(move |s| {
                                        s.pr(if panels.is_queue_shown() {
                                            0.0
                                        } else {
                                            top_end_width.get()
                                        })
                                    });
                                    div()
                                        .size_full()
                                        .flex()
                                        .flex_col()
                                        .child(bar.child(app_bar(
                                            going(navigation),
                                            title,
                                            actions,
                                        )))
                                        .child(room_for(body, panels))
                                }
                            },
                        )))
                        .child(show(move || panels.is_queue_shown(), {
                            let cx = cx.clone();
                            move || queue(&cx, panels).mr(GAP)
                        })),
                )
                .child(player_bar(player, cover, panels).mr(16))
                .child(top_end(
                    hiphile.clone(),
                    shown,
                    cx.commands.clone(),
                    sign_in_request,
                    top_end_width,
                ))
                .child(search(&cx, search_open)),
        )
        .child(lyrics_overlay(&cx, cover, panels, reduces_motion))
        .child(shell.dialogs.view(&cx))
        .child(settings_popup(hiphile.clone(), settings_open))
        .child(sign_in_dialog(hiphile, sign_in_request))
}

/// What `destination` shows.
/// What `destination` shows, which reads from `location` what is selected on it.
fn screen(destination: Destination, location: Signal<Location>, cx: &ScreenContext) -> Screen {
    match destination {
        Destination::Home => next_home::home(cx),
        Destination::Artists => next_artists::artists(location, cx),
        Destination::Albums => next_library::albums(location, cx),
        Destination::Songs => next_songs::songs(cx),
        Destination::Playlists => next_playlists::playlists(location, cx),
        Destination::Favorites => next_favorites::favorites(cx),
    }
}

/// Where a screen's panes go, apart from the rail, the bar and the queue.
fn room_for(body: Div, panels: Panels) -> Div {
    div()
        .flex_1()
        .min_h(0)
        .flex()
        .flex_col()
        // The queue's handle is the gap while it shows
        .dyn_style(move |s| s.mr(if panels.is_queue_shown() { 0.0 } else { GAP }))
        .child(body.flex_1().min_h(0))
}

/// Back and forward, at the start of the bar.
fn going(navigation: Navigation) -> Vec<Div> {
    let back = styled_icon_button(
        icons::ARROW_BACK,
        IconButtonStyle::standard().disabled_when(move || !navigation.can_go_back()),
    )
    .access(|a| a.label("Back"))
    .on_click(move |_| navigation.back());
    let forward = styled_icon_button(
        icons::ARROW_FORWARD,
        IconButtonStyle::standard().disabled_when(move || !navigation.can_go_forward()),
    )
    .access(|a| a.label("Forward"))
    .on_click(move |_| navigation.forward());
    vec![back, forward]
}

#[cfg(test)]
mod tests;
