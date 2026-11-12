use anti_webui::prelude::*;
use hiphile_core::Workspace;
use hiphile_core::desktop::MediaNotice;
use media_controls::MediaControls;
use next_host::notices;
use next_platform::window::MainWindow;
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
    // AntiWebUI gives no window handle, and macOS takes the controls for the process anyway
    let media = workspace.connect_media_controls(MediaControls::connect_without_window());
    notices(media.notices(), |notice| match notice {
        MediaNotice::Raise => {
            if let Some(window) = MainWindow::find(crate::TITLE) {
                window.bring_to_front();
            }
        }
    });
    on_cleanup(move || {
        drop(discord);
        drop(media);
    });
}
