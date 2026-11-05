use subsonic::{AlbumId, SongId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlbumShelf {
    Recommended,
    MostPlayed,
    New,
}

/// By identity, so that the selection survives a reload. An album can be on more
/// than one shelf, and each of its cards is an item of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Song(SongId),
    Album(AlbumShelf, AlbumId),
}

#[cfg(test)]
mod tests {
    use ui::card_grid::{GridMotion, step};

    use super::*;

    #[test]
    fn an_album_on_two_shelves_does_not_hold_up_a_move() {
        let on = |shelf| Item::Album(shelf, AlbumId::new("a"));
        let rows = [vec![on(AlbumShelf::MostPlayed)], vec![on(AlbumShelf::New)]];
        assert_eq!(
            step(&rows, Some(&on(AlbumShelf::MostPlayed)), GridMotion::Down),
            Some(on(AlbumShelf::New))
        );
        assert_eq!(
            step(&rows, Some(&on(AlbumShelf::New)), GridMotion::Up),
            Some(on(AlbumShelf::MostPlayed))
        );
    }
}
