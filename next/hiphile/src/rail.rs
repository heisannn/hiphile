use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    FabStyle, IconButtonStyle, Snackbars, icon_button, nav_item, styled_extended_fab,
    styled_icon_button, wide_navigation_rail,
};
use anti_webui_md3e::icons::{self, Symbol};
use hiphile_core::navigation::Destination;
use hiphile_core::{Hiphile, HiphileSnapshot};
use next_ui::window::{kept_still, moves_window};
use next_ui::{Command, Commands};

use crate::navigation::{Navigation, index_of, location_of};
use crate::server_menu::server_menu;

/// The rail's own room at the top, where the window's buttons are.
const TOP: f32 = 44.0;
/// From the rail's edge to what is 40 wide in it, and to what is 56 wide, which centers them while
/// it is closed.
const NARROW_INSET: f32 = 28.0;
const WIDE_INSET: f32 = 20.0;

/// The destinations down the side of the window, with searching at their head and the servers and
/// the settings at their foot. It reaches the top of the window, which it moves.
pub(crate) fn rail(
    navigation: Navigation,
    expanded: Signal<bool>,
    hiphile: Hiphile,
    shown: Signal<HiphileSnapshot>,
    commands: Commands,
    snackbars: Snackbars,
) -> Div {
    let destinations = Destination::ALL
        .iter()
        .map(|destination| nav_item(icon_of(*destination), destination.label()))
        .collect();
    let toggle = kept_still(
        styled_icon_button(icons::MENU, IconButtonStyle::standard())
            .access(move |a| {
                a.label(if expanded.get() {
                    "Close Navigation"
                } else {
                    "Open Navigation"
                })
            })
            .on_click(move |_| expanded.update(|open| *open = !*open)),
    )
    .ml(NARROW_INSET - WIDE_INSET);
    let search = {
        let commands = commands.clone();
        kept_still(
            styled_extended_fab(
                icons::SEARCH,
                "Search",
                FabStyle::new().expanded_when(move || expanded.get()),
            )
            .access(|a| a.label("Search"))
            .on_click(move |_| commands.send(Command::OpenSearch)),
        )
    };
    let header = div()
        .flex()
        .flex_col()
        .items_start()
        .gap(16.0_f32)
        .pl(WIDE_INSET)
        .child(toggle)
        .child(search);
    let settings = kept_still(
        icon_button(icons::SETTINGS)
            .access(|a| a.label("Settings"))
            .on_click(move |_| commands.send(Command::OpenSettings)),
    );
    let foot = div()
        .flex()
        .flex_col()
        .items_start()
        .gap(12.0_f32)
        .pl(NARROW_INSET)
        .pb(20.0_f32)
        .child(server_menu(hiphile, shown, snackbars))
        .child(settings);
    wide_navigation_rail(
        destinations,
        move || index_of(navigation.destination()),
        move |i| navigation.go(location_of(Destination::ALL[i])),
        move || expanded.get(),
        Some(header),
    )
    .relative()
    .h_full()
    .access(|a| a.role(Role::Navigation).label("Destinations"))
    .child(div().flex_1())
    .child(foot)
    .child(moves_window(
        div().absolute().top(0).left(0).right(0).h(TOP),
    ))
}

fn icon_of(destination: Destination) -> &'static Symbol {
    match destination {
        Destination::Home => icons::HOME,
        Destination::Artists => icons::ARTIST,
        Destination::Albums => icons::ALBUM,
        Destination::Songs => icons::LIBRARY_MUSIC,
        Destination::Playlists => icons::PLAYLIST_PLAY,
        Destination::Favorites => icons::FAVORITE,
    }
}
