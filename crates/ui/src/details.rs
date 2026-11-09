use gpui_kit::SharedString;

pub use hiphile_core::format::{format_album_count, format_song_count};

pub fn join_details<S: AsRef<str>>(parts: impl IntoIterator<Item = Option<S>>) -> SharedString {
    hiphile_core::format::join_details(parts).into()
}
