use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    FabStyle, IconButtonStyle, nav_item, styled_extended_fab, styled_icon_button,
    wide_navigation_rail,
};
use anti_webui_md3e::icons::{self, Symbol};
use hiphile_core::navigation::Destination;
use next_ui::window::{kept_still, moves_window};
use next_ui::{Command, Commands};

use crate::navigation::{Navigation, index_of, location_of};

/// The rail's own room at the top, where the window's buttons are.
const TOP: f32 = 44.0;
/// From the rail's edge to what is 40 wide in it, and to what is 56 wide, which centers them while
/// it is closed.
const NARROW_INSET: f32 = 28.0;
const WIDE_INSET: f32 = 20.0;

/// The destinations down the side of the window, with searching at their head. It reaches the top
/// of the window, which it moves.
pub(crate) fn rail(navigation: Navigation, expanded: Signal<bool>, commands: Commands) -> Div {
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
    let search = kept_still(
        styled_extended_fab(
            icons::SEARCH,
            "Search",
            FabStyle::new().expanded_when(move || expanded.get()),
        )
        .access(|a| a.label("Search"))
        .on_click(move |_| commands.send(Command::OpenSearch)),
    );
    let header = div()
        .flex()
        .flex_col()
        .items_start()
        .gap(16.0_f32)
        .pl(WIDE_INSET)
        .child(toggle)
        .child(search);
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
