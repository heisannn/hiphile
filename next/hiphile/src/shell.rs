use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{IconButtonStyle, snackbars, styled_icon_button};
use anti_webui_md3e::icons;
use hiphile_core::navigation::Destination;
use hiphile_core::settings::AppearanceSnapshot;
use hiphile_core::{Hiphile, HiphileSnapshot, Workspace};
use next_account::{sign_in, sign_in_dialog};
use next_host::follow;
use next_player::{Panels, cover_seed, now_playing_cover, player_bar};
use next_settings::settings_popup;
use next_ui::cover::install_covers;
use next_ui::theme::{follow_theme, is_dark};
use next_ui::type_scale::body;
use next_ui::window::moves_window;
use next_ui::{Command, Commands, Screen, app_bar};

use crate::MenuBar;
use crate::menu_bar::Shown;
use crate::navigation::{Navigation, index_of};
use crate::rail::rail;
use crate::reach::{Reach, show_outside};

/// Narrower than this, the rail closes over its labels.
const WIDE: f32 = 1000.0;

/// What fills the window: signing in, or the workspace of whoever is signed in. Its colors follow
/// the appearance chosen, `system_is_dark` when that follows the system, and the cover of what
/// plays. The workspace takes what is chosen in `menu_bar`, and shows there where it is.
pub fn app(hiphile: Hiphile, system_is_dark: Signal<bool>, menu_bar: MenuBar, reach: Reach) -> Div {
    let appearance = follow(hiphile.settings().appearance().subscribe());
    let seed = signal(None::<Color>);
    follow_theme(
        move || {
            is_dark(
                appearance.with(AppearanceSnapshot::appearance),
                system_is_dark.get(),
            )
        },
        move || seed.get(),
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
                Some(workspace) => {
                    untrack(|| signed_in(hiphile, workspace, shown, menu_bar, seed, reach))
                }
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

fn signed_in(
    hiphile: Hiphile,
    workspace: Workspace,
    shown: Signal<HiphileSnapshot>,
    menu_bar: MenuBar,
    seed: Signal<Option<Color>>,
    reach: Reach,
) -> Div {
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
    let panels = Panels::new();
    let navigation = Navigation::new();
    effect(move || workspace.showing(navigation.destination()));
    let width = signal(crate::WINDOW.0 as f32);
    let wide = memo(move || width.get() >= WIDE);
    let expanded = signal(true);
    effect(move || expanded.set(wide.get()));
    let settings_open = signal(false);
    let sign_in_request = signal(None);
    let commands = Commands::new(move |command| match command {
        Command::Show(location) => navigation.go(location),
        Command::GoBack => navigation.back(),
        Command::GoForward => navigation.forward(),
        Command::OpenSettings => settings_open.set(true),
        Command::OpenSearch => {}
    });
    menu_bar.attach(commands.clone(), move || Shown {
        destination: Some(navigation.destination()),
        can_go_back: navigation.can_go_back(),
        can_go_forward: navigation.can_go_forward(),
        covered: settings_open.get() || sign_in_request.with(Option::is_some),
    });
    div()
        .size_full()
        .flex()
        .flex_row()
        .on_resized(move |w, _| width.set(w))
        .child(rail(
            navigation,
            expanded,
            hiphile.clone(),
            shown,
            commands.clone(),
            sign_in_request,
        ))
        .child(
            div()
                .flex_1()
                .min_w(0)
                .h_full()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h(0).flex().flex_col().child(for_each(
                    move || vec![navigation.destination()],
                    |destination| index_of(*destination),
                    move |destination| {
                        let Screen {
                            title,
                            actions,
                            body,
                        } = screen(destination);
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .child(app_bar(going(navigation), title, actions))
                            .child(pane(body))
                    },
                )))
                .child(player_bar(player, cover, panels).mr(16)),
        )
        .child(settings_popup(hiphile.clone(), settings_open))
        .child(sign_in_dialog(hiphile, sign_in_request))
}

/// What `destination` shows.
fn screen(destination: Destination) -> Screen {
    Screen::new(destination.label(), div())
}

/// The screen goes in a pane of its own, apart from the rail and the bar.
fn pane(body: Div) -> Div {
    div()
        .flex_1()
        .min_h(0)
        .mr(16)
        .rounded(28.0_f32)
        .overflow_hidden()
        .dyn_style(|s| s.bg(color_scheme().surface_container))
        .child(body.size_full())
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
