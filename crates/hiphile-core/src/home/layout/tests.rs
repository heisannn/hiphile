use std::sync::Arc;

use super::*;

#[test]
fn shelves_with_nothing_to_say_go_but_the_new_albums_stay() {
    let none = || Shelf::<Album>::Loaded(Arc::from([]));
    let no_songs = Shelf::<Song>::Loaded(Arc::from([]));

    assert_eq!(
        sections(&none(), &no_songs, &none()),
        [HomeSection::NewAlbums]
    );
}

#[test]
fn shelves_loading_or_failed_keep_their_place_in_order() {
    let failed = || Shelf::<Album>::Failed("offline".into());

    assert_eq!(
        sections(&failed(), &Shelf::Loading, &failed()),
        [
            HomeSection::Recommended,
            HomeSection::RecentlyPlayed,
            HomeSection::MostPlayed,
            HomeSection::NewAlbums,
        ]
    );
}

#[test]
fn each_shelf_asks_for_as_many_albums_as_its_rows_hold_at_their_widest() {
    assert_eq!(AlbumShelf::Recommended.room(), MAX_COLUMNS);
    assert_eq!(AlbumShelf::MostPlayed.room(), MAX_COLUMNS);
    assert_eq!(AlbumShelf::New.room(), 2 * MAX_COLUMNS);
}
