use anti_webui::prelude::*;
use hiphile_core::Workspace;
use presence::Presence;

/// Whether what plays shows outside the window too: as the user's Discord status, and in the
/// system's media controls. Tests and shots keep to the window, so that nothing reaches the
/// user's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    System,
    Window,
}

/// Shows what `workspace` plays outside the window for as long as the owner lasts.
pub(crate) fn show_outside(workspace: &Workspace, reach: Reach) {
    if reach == Reach::Window {
        return;
    }
    let discord = workspace.show_on_discord(Presence::spawn());
    // The system's media controls are not connected: `MediaControls::connect` takes the window's
    // native handle, which AntiWebUI does not give
    on_cleanup(move || drop(discord));
}
