//! The words of details: how many there are and how long they last, joined into one line.

pub use hiphile_core::format::{
    format_album_count, format_duration, format_song_count, join_details,
};

pub fn format_artist_count(count: usize) -> String {
    match count {
        1 => "1 artist".to_owned(),
        count => format!("{count} artists"),
    }
}

pub fn format_playlist_count(count: usize) -> String {
    match count {
        1 => "1 playlist".to_owned(),
        count => format!("{count} playlists"),
    }
}

#[cfg(test)]
mod tests;
