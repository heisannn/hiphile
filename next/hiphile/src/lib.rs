//! The app's one window: signing in, then the rail, the screens and what opens over them.

pub mod menu_bar;
mod navigation;
mod rail;
mod reach;
mod server_menu;
mod shell;
mod top_end;

pub use menu_bar::MenuBar;
pub use reach::Reach;
pub use shell::{System, app};

/// The window's title, which the platform finds the window by.
pub const TITLE: &str = "Hiphile";

/// The size the window opens at, and the least it can be made.
pub const WINDOW: (i32, i32) = (1200, 760);
pub const SMALLEST_WINDOW: (i32, i32) = (900, 560);
