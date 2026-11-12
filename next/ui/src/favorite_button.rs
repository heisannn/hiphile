use anti_webui::prelude::*;
use anti_webui_md3e::components::{IconButtonStyle, icon_toggle_button};
use anti_webui_md3e::icons;
use hiphile_core::favorites::Favorite;

use crate::row_list::row_control;
use crate::{Command, Commands, Known};

/// What toggling `favorite` is called in a menu, by whether it is one.
pub fn favorite_label(is_favorite: Option<bool>) -> &'static str {
    if is_favorite == Some(true) {
        "Remove from Favorites"
    } else {
        "Add to Favorites"
    }
}

/// A button that is on while `favorite` is one of the favorites and toggles it. It is disabled
/// until the favorites are known, as it could not say which way it goes. Pressing it does nothing
/// to the row or card it is in.
///
/// Its name stays "Favorite" while it toggles, as that of a toggle button does; whether it is on
/// is its checked state.
pub fn favorite_button(favorite: Favorite, commands: &Commands, known: Known) -> Div {
    let target = favorite.target();
    let is_favorite = move || known.is_favorite(&target);
    let commands = commands.clone();
    let is_known = is_favorite.clone();
    let on = Binding::new(
        move || is_favorite() == Some(true),
        move |_| commands.send(Command::ToggleFavorite(favorite.clone())),
    );
    let button = icon_toggle_button(
        icons::FAVORITE,
        on,
        IconButtonStyle::standard().disabled_when(move || is_known().is_none()),
    )
    .access(|a| a.label("Favorite"));
    row_control(button)
}

#[cfg(test)]
mod tests;
