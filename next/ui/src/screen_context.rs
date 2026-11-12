use anti_webui_md3e::components::Snackbars;
use hiphile_core::Workspace;

use crate::{Commands, Known};

/// What a screen of the workspace is made with: the workspace it shows, where it sends what it
/// asks of the shell, what every screen goes by, and where it tells of what it did. AntiWebUI has
/// no context to find these in, so the shell hands them down. Cheap to clone.
#[derive(Clone)]
pub struct ScreenContext {
    pub workspace: Workspace,
    pub commands: Commands,
    pub known: Known,
    pub snackbars: Snackbars,
}

impl ScreenContext {
    pub fn new(
        workspace: Workspace,
        commands: Commands,
        known: Known,
        snackbars: Snackbars,
    ) -> Self {
        Self {
            workspace,
            commands,
            known,
            snackbars,
        }
    }
}
