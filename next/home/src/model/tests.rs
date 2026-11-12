use anti_webui::prelude::NamedKey;
use serde_json::json;
use subsonic::{AlbumId, ArtistId};

use super::*;
use next_ui::card_grid::step;

fn song(id: &str) -> Song {
    serde_json::from_value(json!({
        "id": id, "title": id, "played": "2026-11-03T22:36:15Z", "albumId": "a1", "artistId": "ar1",
    }))
    .unwrap()
}

fn album(id: &str) -> Album {
    serde_json::from_value(json!({ "id": id, "name": id, "artistId": format!("by-{id}") })).unwrap()
}

fn albums(prefix: &str, count: usize) -> Shelf<Album> {
    Shelf::Loaded(
        (0..count)
            .map(|ix| album(&format!("{prefix}{ix}")))
            .collect(),
    )
}

fn songs(ids: &[&str]) -> Shelf<Song> {
    Shelf::Loaded(ids.iter().map(|id| song(id)).collect())
}

/// No recommendations and nothing played most.
fn shelves(recent: &[&str], new_albums: usize) -> Shelves {
    Shelves {
        recommended: Shelf::Loaded(Arc::from([])),
        recent: songs(recent),
        most_played: Shelf::Loaded(Arc::from([])),
        new_albums: albums("a", new_albums),
    }
}

fn ids(row: &[Item]) -> Vec<String> {
    row.iter()
        .map(|item| match item {
            Item::Song(id) => id.to_string(),
            Item::Album(_, id) => id.to_string(),
        })
        .collect()
}

fn rows(shelves: &Shelves, columns: usize) -> Vec<Vec<String>> {
    shelves.rows(columns).iter().map(|row| ids(row)).collect()
}

#[test]
fn no_plays_takes_the_song_shelf_away() {
    let shelves = shelves(&[], 1);
    assert_eq!(shelves.sections(), [HomeSection::NewAlbums]);
    assert_eq!(rows(&shelves, 4), [["a0"]]);
}

#[test]
fn no_plays_takes_the_most_played_shelf_away() {
    assert!(
        !shelves(&[], 1)
            .sections()
            .contains(&HomeSection::MostPlayed)
    );
}

#[test]
fn shelves_that_all_failed_alike_fail_once() {
    let failed = |reason: &str| Shelves {
        recommended: Shelf::Failed("offline".into()),
        recent: Shelf::Failed("offline".into()),
        most_played: Shelf::Failed("offline".into()),
        new_albums: Shelf::Failed(reason.into()),
    };

    assert_eq!(
        failed("offline").common_failure().as_deref(),
        Some("offline")
    );
    assert_eq!(failed("HTTP 500").common_failure(), None);
}

#[test]
fn a_shelf_that_did_not_fail_keeps_the_failures_apart() {
    let one_loading = Shelves {
        recommended: Shelf::Failed("offline".into()),
        recent: Shelf::Loading,
        most_played: Shelf::Failed("offline".into()),
        new_albums: Shelf::Failed("offline".into()),
    };

    assert_eq!(one_loading.common_failure(), None);
    assert_eq!(shelves(&[], 0).common_failure(), None);
}

#[test]
fn loading_and_failing_shelves_keep_their_place() {
    let shelves = Shelves {
        recommended: Shelf::Failed("offline".into()),
        recent: Shelf::Loading,
        most_played: Shelf::Failed("offline".into()),
        new_albums: Shelf::Failed("offline".into()),
    };
    assert_eq!(
        shelves.sections(),
        [
            HomeSection::Recommended,
            HomeSection::RecentlyPlayed,
            HomeSection::MostPlayed,
            HomeSection::NewAlbums,
        ]
    );
    assert!(shelves.rows(4).is_empty());
}

#[test]
fn new_albums_show_in_two_rows_of_as_many_as_fit() {
    assert_eq!(
        rows(&shelves(&[], 24), 3),
        [["a0", "a1", "a2"], ["a3", "a4", "a5"]]
    );
    assert_eq!(
        rows(&shelves(&[], 4), 3),
        [vec!["a0", "a1", "a2"], vec!["a3"]]
    );
}

#[test]
fn recommendations_come_first_in_one_row_of_as_many_as_fit() {
    let mut shelves = shelves(&["s1"], 1);
    shelves.recommended = albums("r", 12);
    assert_eq!(
        rows(&shelves, 3),
        [vec!["r0", "r1", "r2"], vec!["s1"], vec!["a0"]]
    );
    shelves.recommended = albums("r", 2);
    assert_eq!(rows(&shelves, 3)[0], ["r0", "r1"]);
}

#[test]
fn most_played_comes_above_new_albums_in_one_row_of_as_many_as_fit() {
    let mut shelves = shelves(&["s1"], 1);
    shelves.most_played = albums("m", 12);
    assert_eq!(
        rows(&shelves, 3),
        [vec!["s1"], vec!["m0", "m1", "m2"], vec!["a0"]]
    );
}

#[test]
fn a_recommended_album_past_the_row_is_off_the_screen() {
    let mut shelves = shelves(&[], 0);
    shelves.recommended = albums("r", 6);
    let r5 = Item::Album(AlbumShelf::Recommended, AlbumId::new("r5"));
    assert!(shelves.target(&r5, 6).is_some());
    assert_eq!(shelves.target(&r5, 4), None);
}

#[test]
fn a_narrower_pane_takes_a_selection_off_the_screen_until_it_widens() {
    let shelves = shelves(&[], 24);
    let a13 = Item::Album(AlbumShelf::New, AlbumId::new("a13"));
    assert!(shelves.target(&a13, 7).is_some());
    assert_eq!(shelves.target(&a13, 2), None);
    assert!(shelves.target(&a13, 7).is_some());
}

#[test]
fn a_song_is_found_again_where_a_reload_puts_it() {
    let mut shelves = shelves(&["s1", "s2"], 0);
    shelves.recent = songs(&["s2", "s1"]);
    let Some(Target::Song { ix, .. }) = shelves.target(&Item::Song(SongId::new("s2")), 4) else {
        panic!("the song is still there");
    };
    assert_eq!(ix, 0);
}

#[test]
fn an_album_on_two_shelves_is_two_items() {
    let mut shelves = shelves(&[], 1);
    shelves.most_played = albums("a", 1);
    let on = |shelf| Item::Album(shelf, AlbumId::new("a0"));
    let rows = shelves.rows(4);
    assert_eq!(
        rows,
        [vec![on(AlbumShelf::MostPlayed)], vec![on(AlbumShelf::New)]]
    );
    assert_eq!(
        step(&rows, Some(&on(AlbumShelf::MostPlayed)), GridMotion::Down),
        Some(on(AlbumShelf::New))
    );
    assert_eq!(
        step(&rows, Some(&on(AlbumShelf::New)), GridMotion::Up),
        Some(on(AlbumShelf::MostPlayed))
    );
}

fn on_album() -> Target {
    Target::Album(album("al"))
}

fn on_song() -> Target {
    Target::Song {
        songs: Arc::from([song("s0"), song("s1")]),
        ix: 1,
    }
}

#[test]
fn opening_an_album_shows_it_and_opening_a_song_plays_from_it() {
    assert_eq!(
        on_album().command(Intent::Open),
        Some(Command::Show(Location::Albums(Some(AlbumId::new("al")))))
    );
    let Some(Command::Play { songs, start }) = on_song().command(Intent::Open) else {
        panic!("the songs play");
    };
    assert_eq!(songs.len(), 2);
    assert_eq!(start, Some(1));
}

#[test]
fn an_album_is_played_and_queued_once_its_songs_are_had() {
    assert_eq!(
        on_album().command(Intent::Play),
        Some(Command::ActOnAlbum {
            album: AlbumId::new("al"),
            action: AlbumAction::Play,
        })
    );
    assert_eq!(
        on_album().command(Intent::Enqueue(Placement::Next)),
        Some(Command::ActOnAlbum {
            album: AlbumId::new("al"),
            action: AlbumAction::Enqueue(Placement::Next),
        })
    );
}

#[test]
fn a_song_is_queued_alone() {
    let Some(Command::Enqueue { songs, placement }) =
        on_song().command(Intent::Enqueue(Placement::Last))
    else {
        panic!("the song is queued");
    };
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].id().as_str(), "s1");
    assert_eq!(placement, Placement::Last);
}

#[test]
fn a_song_shows_its_album_and_both_go_to_their_artist() {
    assert_eq!(
        on_song().command(Intent::ShowInLibrary),
        Some(Command::Show(Location::Albums(Some(AlbumId::new("a1")))))
    );
    assert_eq!(
        on_song().command(Intent::GoToArtist),
        Some(Command::Show(Location::Artists(Some(ArtistId::new("ar1")))))
    );
    assert_eq!(
        on_album().command(Intent::GoToArtist),
        Some(Command::Show(Location::Artists(Some(ArtistId::new(
            "by-al"
        )))))
    );
}

#[test]
fn what_has_nowhere_to_go_does_nothing() {
    let unknown: Song = serde_json::from_value(json!({ "id": "s", "title": "s" })).unwrap();
    let target = Target::Song {
        songs: Arc::from([unknown]),
        ix: 0,
    };
    assert_eq!(target.command(Intent::ShowInLibrary), None);
    assert_eq!(target.command(Intent::GoToArtist), None);
}

#[test]
fn the_keys_ask_for_what_the_gpui_app_bound_them_to() {
    let enter = || Keystroke::named(NamedKey::Enter);
    let asked = |stroke: Keystroke| {
        Asked::strokes()
            .into_iter()
            .find(|(bound, _)| *bound == stroke)
            .map(|(_, asked)| asked)
    };
    assert_eq!(
        asked(Keystroke::named(NamedKey::ArrowRight)),
        Some(Asked::Move(GridMotion::Right))
    );
    assert_eq!(asked(enter()), Some(Asked::Do(Intent::Open)));
    assert_eq!(asked(enter().with_primary()), Some(Asked::Do(Intent::Play)));
    assert_eq!(
        asked(enter().with_alt()),
        Some(Asked::Do(Intent::Enqueue(Placement::Next)))
    );
    assert_eq!(
        asked(enter().with_shift()),
        Some(Asked::Do(Intent::Enqueue(Placement::Last)))
    );
    assert_eq!(
        asked(Keystroke::primary('l')),
        Some(Asked::Do(Intent::ShowInLibrary))
    );
    assert_eq!(
        asked(Keystroke::primary('a').with_shift()),
        Some(Asked::Do(Intent::GoToArtist))
    );
    assert_eq!(
        asked(Keystroke::primary('d')),
        Some(Asked::Do(Intent::ToggleFavorite))
    );
    assert_eq!(
        asked(Keystroke::primary('p')),
        Some(Asked::Do(Intent::ChoosePlaylist))
    );
    assert_eq!(asked(Keystroke::character('d')), None);
    assert_eq!(asked(Keystroke::named(NamedKey::Space)), None);
}

#[test]
fn only_what_enter_does_gives_way_to_a_focused_button() {
    assert!(Asked::Do(Intent::Open).is_for_buttons());
    assert!(Asked::Do(Intent::Enqueue(Placement::Last)).is_for_buttons());
    assert!(!Asked::Move(GridMotion::Down).is_for_buttons());
    assert!(!Asked::Do(Intent::ToggleFavorite).is_for_buttons());
}
