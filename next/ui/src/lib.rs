//! What the screens on AntiWebUI share: the theme, the type, and the parts that more than one
//! screen shows.

mod command;
pub mod cover;
mod screen;
pub mod theme;
pub mod type_scale;
pub mod window;

pub use command::{Command, Commands};
pub use screen::{Screen, app_bar};

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
