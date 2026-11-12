//! What the screens on AntiWebUI share: the theme, the type, keys, and the parts that more than
//! one screen shows.

pub mod card_grid;
mod command;
pub mod cover;
pub mod detail_header;
pub mod details;
pub mod keymap;
pub mod list_detail;
pub mod list_pane;
pub mod load_state;
pub mod now_playing;
pub mod pane;
pub mod row_list;
pub mod row_text;
mod screen;
pub mod section;
pub mod sort_menu;
pub mod theme;
pub mod type_scale;
pub mod window;

pub use command::{Command, Commands};
pub use screen::{Screen, app_bar};

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
