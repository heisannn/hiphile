use std::sync::Arc;

use gpui_kit::IntoElement;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use hiphile_core::HiphileSnapshot;
use hiphile_core::format::format_profile;

use crate::{AddServer, SignOut, SwitchServer};

/// The profiles to switch to and, signed in, adding one and signing out, for the title bar.
/// `None` signed out with none to switch to.
pub fn server_menu(accounts: &HiphileSnapshot) -> Option<impl IntoElement + use<>> {
    let signed_in = accounts.profile().cloned();
    if signed_in.is_none() && accounts.profiles().is_empty() {
        return None;
    }
    let profiles: Arc<[config::Profile]> = accounts.profiles().into();
    Some(
        Button::new("servers")
            .ghost()
            .small()
            .icon(IconName::Server)
            .loading(accounts.is_signing_in())
            .accessibility_label("Servers")
            .tooltip("Servers")
            .dropdown_menu(move |menu, _, _| {
                let menu = profiles.iter().fold(menu, |menu, profile| {
                    menu.item(
                        PopupMenuItem::new(format_profile(profile))
                            .checked(signed_in.as_ref() == Some(profile))
                            .action(Box::new(SwitchServer::new(profile.clone()))),
                    )
                });
                if signed_in.is_none() {
                    return menu;
                }
                menu.separator()
                    .menu("Add Server…", Box::new(AddServer))
                    .separator()
                    .menu("Sign Out", Box::new(SignOut))
            }),
    )
}
