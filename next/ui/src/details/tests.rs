use super::*;

#[test]
fn counts_of_one_are_singular() {
    assert_eq!(format_artist_count(1), "1 artist");
    assert_eq!(format_artist_count(0), "0 artists");
    assert_eq!(format_playlist_count(1), "1 playlist");
    assert_eq!(format_playlist_count(12), "12 playlists");
}
