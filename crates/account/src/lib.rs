//! Signing in, and switching between the profiles signed in with.

mod server_menu;
mod sign_in;
mod sign_in_dialog;
mod sign_in_form;
#[cfg(test)]
mod testing;

use gpui_kit::{Action, actions};

pub use server_menu::server_menu;
pub use sign_in::SignIn;
pub use sign_in_dialog::open_sign_in_dialog;

actions!(hiphile, [AddServer, SignOut]);

/// Signs in with `profile` in place of whoever is signed in.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = hiphile, no_json)]
pub struct SwitchServer {
    profile: config::Profile,
}

impl SwitchServer {
    pub fn new(profile: config::Profile) -> Self {
        Self { profile }
    }

    pub fn profile(&self) -> &config::Profile {
        &self.profile
    }
}
