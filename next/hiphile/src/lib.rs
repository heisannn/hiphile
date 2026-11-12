//! The app's one window: signing in, then the rail, the screens and what opens over them.

mod navigation;
mod rail;
mod server_menu;
mod settings_popup;
mod shell;

pub use shell::app;

/// The size the window opens at, and the least it can be made.
pub const WINDOW: (i32, i32) = (1200, 760);
pub const SMALLEST_WINDOW: (i32, i32) = (900, 560);
