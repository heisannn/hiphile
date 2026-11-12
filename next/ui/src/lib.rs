//! What the screens on AntiWebUI share: the theme, the type, keys, and the parts that more than
//! one screen shows.

pub mod card_grid;
mod command;
pub mod context_menu;
pub mod cover;
pub mod detail_header;
pub mod details;
pub mod favorite_button;
pub mod favorite_songs;
pub mod keymap;
mod known;
pub mod link;
pub mod list_detail;
pub mod list_pane;
pub mod load_state;
pub mod menus;
pub mod now_playing;
pub mod page_row;
pub mod pane;
pub mod pane_handle;
pub mod playlist_dialogs;
pub mod row_cover;
pub mod row_list;
pub mod row_text;
mod screen;
mod screen_context;
pub mod section;
pub mod song_row;
pub mod sort_menu;
pub mod theme;
pub mod type_scale;
pub mod window;

pub use command::{AlbumAction, Command, Commands};
pub use known::Known;
pub use screen::{APP_BAR_HEIGHT, Screen, app_bar};
pub use screen_context::ScreenContext;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
