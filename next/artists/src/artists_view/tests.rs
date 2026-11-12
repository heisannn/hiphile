use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::core::access::{AccessNode, snapshot};
use hiphile_core::test_support::ArtistsServer;
use hiphile_core::{Hiphile, Shelf};
use next_ui::testing::{
    TestCore, click_text, find, focus, frames, has_text, key, key_with, middle_of, nodes, press,
    right_click, screen_context, signed_in_core, start, typed, wait_until,
};
use next_ui::{Commands, Screen};
use serde_json::{Value, json};
use subsonic::StarTarget;

use super::page::reckoned_row;
use super::*;

fn artist_json(id: &str, name: &str, albums: u32) -> Value {
    json!({ "id": id, "name": name, "albumCount": albums })
}

fn album_json(id: &str, year: u32) -> Value {
    json!({
        "id": id, "name": format!("Album {id}"), "year": year, "songCount": 2,
        "artistId": "m", "artist": "Miles Davis",
    })
}

fn song_json(ix: usize) -> Value {
    json!({
        "id": format!("s{ix}"), "title": format!("Song {ix}"), "albumId": "a1",
        "album": "Album a1", "artistId": "m", "artist": "Miles Davis",
    })
}

/// Bill Evans and Miles Davis. Miles has two albums of two songs each, `top_songs` top songs, and
/// Bill as an artist like him.
fn server(top_songs: usize) -> ArtistsServer {
    let server = ArtistsServer::new();
    server.set_artists(vec![
        artist_json("b", "Bill Evans", 1),
        artist_json("m", "Miles Davis", 2),
    ]);
    server.set_albums(
        "b",
        vec![json!({ "id": "b1", "name": "Album b1", "artistId": "b", "artist": "Bill Evans" })],
    );
    server.set_albums("m", vec![album_json("a1", 1959), album_json("a2", 1970)]);
    server.set_top_songs("Miles Davis", (0..top_songs).map(song_json).collect());
    server.set_similar("m", vec![artist_json("b", "Bill Evans", 1)]);
    server.set_album_songs("a1", vec![song_json(0), song_json(1)]);
    server.set_album_songs("a2", vec![song_json(2), song_json(3)]);
    server
}

fn many_artists() -> Vec<Value> {
    (0..500)
        .map(|ix| artist_json(&format!("a{ix}"), &format!("Artist {ix:03}"), 1))
        .collect()
}

fn artist(id: &str) -> Location {
    Location::Artists(Some(ArtistId::new(id)))
}

struct Shown {
    ui: Ui,
    sent: Rc<RefCell<Vec<Command>>>,
    location: Signal<Location>,
    away: Signal<bool>,
    made: Rc<RefCell<Option<Rc<View>>>>,
    view: Rc<View>,
    hiphile: Hiphile,
    _core: TestCore,
}

impl Shown {
    /// The screen on `server` with the window at `at`. The commands it sends are kept, and those
    /// that go somewhere take the window there, as the shell's do.
    fn at(server: &ArtistsServer, at: Location) -> Self {
        let (core, hiphile) = signed_in_core(server.url());
        let sent = Rc::new(RefCell::new(Vec::new()));
        let made = Rc::new(RefCell::new(None));
        let kept = hiphile.clone();
        let places = Rc::new(RefCell::new(None));
        let ui = start({
            let (sent, made, places) = (sent.clone(), made.clone(), places.clone());
            move || {
                let location = signal(at);
                let away = signal(false);
                // Kept between visits, as the shell keeps it
                let split = Split::new();
                *places.borrow_mut() = Some((location, away));
                let commands = Commands::new(move |command| {
                    if let Command::Show(to) | Command::ShowInPlace(to) = &command {
                        location.set_if_changed(to.clone());
                    }
                    sent.borrow_mut().push(command);
                });
                let cx = screen_context(&kept, commands);
                // Made again on each visit, as the shell makes it
                let screen = dyn_view(move || {
                    if away.get() {
                        return div();
                    }
                    untrack(|| {
                        let (view, Screen { body, .. }) = artists_view(location, split, &cx);
                        *made.borrow_mut() = Some(view);
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .child(body.flex_1().min_h(0))
                            .child(cx.snackbars.view())
                    })
                });
                div().size_full().child(screen)
            }
        });
        let (location, away) = places.borrow_mut().take().unwrap();
        let view = made.borrow_mut().take().expect("the screen made");
        Self {
            ui,
            sent,
            location,
            away,
            made,
            view,
            hiphile,
            _core: core,
        }
    }

    /// Leaves the screen and comes back to it at `to`.
    fn visit(&mut self, to: Location) {
        self.location.set(Location::Home);
        self.away.set(true);
        self.ui.update();
        self.location.set(to);
        self.away.set(false);
        self.ui.update();
        self.view = self
            .made
            .borrow_mut()
            .take()
            .expect("the screen made again");
    }

    fn on(server: &ArtistsServer) -> Self {
        Self::at(server, Location::Artists(None))
    }

    /// Waits until the page of `id` has every part it loads.
    fn page_of(&mut self, id: &str) {
        let page = self.view.page;
        let id = ArtistId::new(id);
        wait_until(&mut self.ui, || {
            page.with_untracked(|page| {
                page.as_ref().is_some_and(|page| {
                    page.id == id
                        && page.name.is_some()
                        && !matches!(page.albums, Shelf::Loading)
                        && !matches!(page.top_songs, Shelf::Loading)
                        && !matches!(page.similar, Shelf::Loading)
                })
            })
        });
        frames(&mut self.ui, 50);
    }

    /// Waits until the list has `count` rows.
    fn listed(&mut self, count: usize) {
        let status = self.view.status;
        wait_until(&mut self.ui, || {
            status.with_untracked(|status| {
                !status.is_refreshing()
                    && status
                        .shown()
                        .is_some_and(|shown| shown.visible().len() == count)
            })
        });
        frames(&mut self.ui, 50);
    }

    fn location(&self) -> Location {
        self.location.get_untracked()
    }

    fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    fn last_sent(&self) -> Option<Command> {
        self.sent.borrow().last().cloned()
    }

    /// Waits for a command that `is` holds for, which it gives.
    fn wait_for_sent(&mut self, is: impl Fn(&Command) -> bool) -> Command {
        let sent = self.sent.clone();
        wait_until(&mut self.ui, || sent.borrow().iter().any(&is));
        let sent = self.sent.borrow();
        sent.iter().find(|command| is(command)).cloned().unwrap()
    }

    fn keyed(&mut self, named: NamedKey, modifiers: Modifiers) {
        key_with(&mut self.ui, Key::Named(named), modifiers);
    }

    fn focus_page(&mut self) {
        self.view.page_node.focus();
        self.ui.update();
    }

    fn focus_list(&mut self) {
        focus(&mut self.ui, Role::ListBox, "Artists");
    }

    fn select(&mut self, item: Item) {
        self.view.item.set(Some(item));
        self.focus_page();
    }

    fn item(&self) -> Option<Item> {
        self.view.item.get_untracked()
    }

    /// Waits until the favorites are known, which the commands on favorites go by.
    fn favorites_known(&mut self) {
        let favorites = self.hiphile.workspace().unwrap().favorites().clone();
        favorites.refresh();
        let known = self.view.cx.known;
        let target = StarTarget::Artist(ArtistId::new("m"));
        wait_until(&mut self.ui, || {
            untrack(|| known.is_favorite(&target)).is_some()
        });
    }

    fn list_scroll(&mut self) -> f32 {
        frames(&mut self.ui, 50);
        self.view.list_node.scroll_offset().y
    }
}

fn focused() -> Option<AccessNode> {
    let tree = snapshot();
    let focus = tree.focus?;
    tree.nodes.into_iter().find(|node| node.id == focus)
}

fn list_width() -> f64 {
    let handle = find(Role::Separator, "List Width").expect("the handle");
    handle.value.expect("a width").now
}

fn focused_label() -> Option<String> {
    focused().and_then(|node| node.label)
}

fn song_rows() -> usize {
    nodes(Role::ListOption)
        .into_iter()
        .filter_map(|node| node.label)
        .filter(|label| label.starts_with("Song "))
        .count()
}

fn primary() -> Modifiers {
    Modifiers::PRIMARY
}

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Modifiers::default()
    }
}

fn ids(songs: &[Song]) -> Vec<String> {
    songs.iter().map(|song| song.id().to_string()).collect()
}

#[test]
fn showing_the_screen_fills_the_list_and_the_first_artists_page() {
    let server = server(10);
    let mut shown = Shown::on(&server);
    shown.listed(2);
    shown.page_of("b");

    assert_eq!(shown.location(), artist("b"));
    assert!(
        shown
            .sent()
            .iter()
            .all(|command| !matches!(command, Command::Show(_))),
        "starting on an artist is nowhere to go back to"
    );
    assert!(find(Role::Heading, "Bill Evans").is_some());
    assert!(has_text("Album b1"));

    shown.focus_list();
    key(&mut shown.ui, NamedKey::ArrowDown);
    shown.page_of("m");
    assert_eq!(shown.location(), artist("m"));
    // Newest first
    assert!(middle_of("Album a2").0 < middle_of("Album a1").0);
    assert_eq!(song_rows(), 5);
    assert!(find(Role::Heading, "Similar artists").is_some());
}

#[test]
fn a_failed_section_leaves_the_rest_of_the_page() {
    let server = server(3);
    server.refuse("getTopSongs");
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    assert!(find(Role::Heading, "Top songs").is_some());
    assert!(find(Role::Button, "Retry").is_some());
    assert!(has_text("Album a1"));

    server.accept("getTopSongs");
    press(&mut shown.ui, Role::Button, "Retry");
    let ui = &mut shown.ui;
    wait_until(ui, || song_rows() == 3);
}

#[test]
fn nothing_to_say_takes_top_songs_and_similar_artists_away() {
    let server = server(0);
    server.set_similar("m", Vec::new());
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    assert!(find(Role::Heading, "Albums").is_some());
    assert!(find(Role::Heading, "Top songs").is_none());
    assert!(find(Role::Heading, "Similar artists").is_none());
}

#[test]
fn five_top_songs_show_until_see_more_is_pressed() {
    let server = server(10);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    assert_eq!(song_rows(), 5);

    press(&mut shown.ui, Role::Button, "See more");
    assert_eq!(song_rows(), 10);

    press(&mut shown.ui, Role::Button, "Show less");
    assert_eq!(song_rows(), 5);
}

#[test]
fn see_more_goes_away_with_no_more_to_see() {
    let server = server(5);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    assert_eq!(song_rows(), 5);
    assert!(find(Role::Button, "See more").is_none());
}

#[test]
fn the_see_more_button_works_from_the_keyboard_and_keeps_the_focus() {
    let server = server(10);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    focus(&mut shown.ui, Role::Button, "See more");
    key(&mut shown.ui, NamedKey::Enter);
    assert!(shown.view.top_songs_expanded.get_untracked());
    assert_eq!(focused_label().as_deref(), Some("Show less"));

    key(&mut shown.ui, NamedKey::Space);
    assert!(!shown.view.top_songs_expanded.get_untracked());
}

#[test]
fn another_artist_starts_with_five_top_songs() {
    let server = server(10);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    press(&mut shown.ui, Role::Button, "See more");

    shown.location.set(artist("b"));
    shown.ui.update();

    assert!(!shown.view.top_songs_expanded.get_untracked());
}

#[test]
fn commands_leave_a_song_that_show_less_took_off_the_page() {
    let server = server(10);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    press(&mut shown.ui, Role::Button, "See more");
    shown.select(Item::Song(SongId::new("s9")));
    assert!(shown.view.item_on_show().is_some());

    press(&mut shown.ui, Role::Button, "Show less");

    assert_eq!(shown.view.item_on_show(), None);
    shown.keyed(NamedKey::Enter, Modifiers::default());
    assert!(
        !shown
            .sent()
            .iter()
            .any(|command| matches!(command, Command::Play { .. }))
    );
}

#[test]
fn enter_plays_only_the_top_songs_on_show() {
    let server = server(10);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    click_text(&mut shown.ui, "Song 2");
    assert_eq!(shown.item(), Some(Item::Song(SongId::new("s2"))));

    key(&mut shown.ui, NamedKey::Enter);

    let Some(Command::Play { songs, start }) = shown.last_sent() else {
        panic!("no Play");
    };
    assert_eq!(songs.len(), 5);
    assert_eq!(start, Some(2));
}

#[test]
fn enter_shows_the_album_or_the_artist_selected() {
    let server = server(1);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    shown.select(Item::Album(AlbumId::new("a1")));
    key(&mut shown.ui, NamedKey::Enter);
    assert_eq!(
        shown.last_sent(),
        Some(Command::Show(Location::Albums(Some(AlbumId::new("a1")))))
    );

    shown.location.set(artist("m"));
    shown.select(Item::Artist(ArtistId::new("b")));
    key(&mut shown.ui, NamedKey::Enter);
    assert_eq!(shown.last_sent(), Some(Command::Show(artist("b"))));
}

#[test]
fn clicking_a_card_shows_what_it_is() {
    let server = server(1);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    click_text(&mut shown.ui, "Album a2");

    assert_eq!(shown.item(), Some(Item::Album(AlbumId::new("a2"))));
    assert_eq!(
        shown.last_sent(),
        Some(Command::Show(Location::Albums(Some(AlbumId::new("a2")))))
    );
}

#[test]
fn the_arrows_move_through_songs_then_cards() {
    let server = server(1);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    shown.focus_list();

    // On to the page, where the song comes first
    key(&mut shown.ui, NamedKey::Enter);
    assert_eq!(focused_label().as_deref(), Some("Artist"));
    assert_eq!(shown.item(), Some(Item::Song(SongId::new("s0"))));

    key(&mut shown.ui, NamedKey::ArrowDown);
    key(&mut shown.ui, NamedKey::ArrowRight);
    assert_eq!(shown.item(), Some(Item::Album(AlbumId::new("a1"))));

    key(&mut shown.ui, NamedKey::ArrowDown);
    assert_eq!(shown.item(), Some(Item::Artist(ArtistId::new("b"))));

    key(&mut shown.ui, NamedKey::Escape);
    assert_eq!(focused_label().as_deref(), Some("Artists"));
}

#[test]
fn right_goes_from_the_list_to_the_page() {
    let server = server(1);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    shown.focus_list();

    key(&mut shown.ui, NamedKey::ArrowRight);

    assert_eq!(focused_label().as_deref(), Some("Artist"));
    assert_eq!(shown.item(), Some(Item::Song(SongId::new("s0"))));
}

#[test]
fn the_go_to_artist_key_follows_the_song_selected() {
    let server = server(1);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    click_text(&mut shown.ui, "Song 0");

    let modifiers = Modifiers {
        shift: true,
        ..primary()
    };
    key_with(&mut shown.ui, Key::Character("a".to_owned()), modifiers);

    assert_eq!(shown.last_sent(), Some(Command::Show(artist("m"))));
}

#[test]
fn the_go_to_album_key_follows_what_is_selected() {
    let server = server(1);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    for (item, album) in [
        (Item::Song(SongId::new("s0")), "a1"),
        (Item::Album(AlbumId::new("a2")), "a2"),
    ] {
        shown.location.set(artist("m"));
        shown.select(item);
        key_with(&mut shown.ui, Key::Character("l".to_owned()), primary());
        let to = Location::Albums(Some(AlbumId::new(album)));
        assert_eq!(shown.last_sent(), Some(Command::Show(to)));
    }
}

#[test]
fn the_favorite_key_follows_what_is_selected_in_each_pane() {
    let server = server(1);
    server.set_similar("m", vec![artist_json("x", "Similar", 1)]);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    shown.favorites_known();
    let toggled = |shown: &Shown| match shown.last_sent() {
        Some(Command::ToggleFavorite(favorite)) => favorite.target(),
        other => panic!("{other:?} toggles no favorite"),
    };

    shown.focus_list();
    key_with(&mut shown.ui, Key::Character("d".to_owned()), primary());
    assert_eq!(toggled(&shown), StarTarget::Artist(ArtistId::new("m")));

    for (item, target) in [
        (
            Item::Song(SongId::new("s0")),
            StarTarget::Song(SongId::new("s0")),
        ),
        (
            Item::Album(AlbumId::new("a1")),
            StarTarget::Album(AlbumId::new("a1")),
        ),
        (
            Item::Artist(ArtistId::new("x")),
            StarTarget::Artist(ArtistId::new("x")),
        ),
    ] {
        shown.select(item);
        key_with(&mut shown.ui, Key::Character("d".to_owned()), primary());
        assert_eq!(toggled(&shown), target);
    }
}

#[test]
fn an_artist_cannot_become_a_favorite_before_their_name_is_known() {
    let server = server(0);
    server.set_albums("unlisted", Vec::new());
    server.hold("getArtist");
    let mut shown = Shown::at(&server, artist("unlisted"));
    shown.listed(2);
    shown.favorites_known();

    shown.focus_list();
    key_with(&mut shown.ui, Key::Character("d".to_owned()), primary());
    let toggles = |shown: &Shown| {
        shown
            .sent()
            .into_iter()
            .filter_map(|command| match command {
                Command::ToggleFavorite(Favorite::Artist(artist)) => Some(artist),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(toggles(&shown).is_empty());

    server.release("getArtist");
    shown.page_of("unlisted");
    shown.focus_list();
    key_with(&mut shown.ui, Key::Character("d".to_owned()), primary());
    let toggled = toggles(&shown);
    assert_eq!(toggled.len(), 1);
    assert_eq!(toggled[0].name(), "Artist unlisted");
}

#[test]
fn revealing_an_artist_the_filter_hides_clears_the_filter() {
    let server = server(0);
    let mut shown = Shown::at(&server, artist("m"));
    shown.listed(2);
    focus(&mut shown.ui, Role::TextInput, "Filter Artists");
    typed(&mut shown.ui, "Miles");
    shown.listed(1);

    shown.location.set(artist("b"));
    shown.listed(2);

    assert_eq!(shown.view.filter.get_untracked(), "");
    assert_eq!(shown.location(), artist("b"));
    assert_eq!(focused_label().as_deref(), Some("Artists"));
}

#[test]
fn the_filter_outlasts_the_screen() {
    let server = server(0);
    let mut shown = Shown::at(&server, artist("m"));
    shown.listed(2);
    focus(&mut shown.ui, Role::TextInput, "Filter Artists");
    typed(&mut shown.ui, "Miles");
    shown.listed(1);

    shown.visit(Location::Artists(None));
    shown.listed(1);

    assert_eq!(shown.view.filter.get_untracked(), "Miles");
    assert_eq!(shown.location(), artist("m"));
}

#[test]
fn enter_in_the_filter_goes_on_to_the_list() {
    let server = server(0);
    let mut shown = Shown::on(&server);
    shown.listed(2);
    focus(&mut shown.ui, Role::TextInput, "Filter Artists");
    typed(&mut shown.ui, "Miles");
    shown.listed(1);

    key(&mut shown.ui, NamedKey::Enter);

    assert_eq!(focused_label().as_deref(), Some("Artists"));
}

#[test]
fn the_list_comes_back_as_wide_as_it_was_left() {
    let server = server(0);
    let mut shown = Shown::on(&server);
    shown.listed(2);
    let first = list_width();
    focus(&mut shown.ui, Role::Separator, "List Width");
    key(&mut shown.ui, NamedKey::ArrowRight);
    let widened = list_width();
    assert!(widened > first, "{widened} from {first}");

    shown.visit(Location::Artists(None));
    shown.listed(2);

    assert_eq!(list_width(), widened);
}

#[test]
fn coming_back_to_no_artist_shows_the_last_one() {
    let server = server(0);
    let mut shown = Shown::on(&server);
    shown.listed(2);
    shown.location.set(artist("m"));
    shown.page_of("m");

    shown.visit(Location::Artists(None));
    shown.page_of("m");

    assert_eq!(shown.location(), artist("m"));
    assert_eq!(focused_label().as_deref(), Some("Artists"));
}

#[test]
fn the_list_comes_to_an_artist_revealed_before_it_arrived() {
    let server = server(0);
    server.set_artists(many_artists());
    server.hold("getArtists");
    let mut shown = Shown::at(&server, artist("a400"));
    assert_eq!(shown.list_scroll(), 0.0);

    server.release("getArtists");
    shown.listed(500);

    assert!(shown.list_scroll() > 300.0 * ROW_HEIGHT);
}

#[test]
fn a_later_list_leaves_the_scroll_alone() {
    let server = server(0);
    server.set_artists(many_artists());
    server.hold("getArtists");
    let mut shown = Shown::at(&server, artist("a400"));
    server.release("getArtists");
    shown.listed(500);
    assert!(shown.list_scroll() > 300.0 * ROW_HEIGHT);

    shown.view.list_node.scroll_to(0.0, 0.0);
    assert_eq!(shown.list_scroll(), 0.0);
    let mut moved = many_artists();
    moved.reverse();
    server.set_artists(moved);
    shown.view.list.reload();
    let status = shown.view.status;
    wait_until(&mut shown.ui, || {
        status.with_untracked(|status| {
            status
                .shown()
                .is_some_and(|shown| shown.items()[0].id() == &ArtistId::new("a499"))
        })
    });

    assert_eq!(shown.list_scroll(), 0.0);
}

#[test]
fn choosing_another_artist_stops_waiting_to_reveal() {
    let server = server(0);
    server.set_artists(many_artists());
    server.hold("getArtists");
    let mut shown = Shown::at(&server, artist("a400"));

    // As the keys would
    shown.view.selection.select(ArtistId::new("a1"));
    shown.ui.update();
    assert_eq!(shown.location(), artist("a1"));
    server.release("getArtists");
    shown.listed(500);

    assert!(shown.list_scroll() < 10.0 * ROW_HEIGHT);
}

#[test]
fn left_goes_back_to_the_list_from_the_start_of_a_row() {
    let server = server(2);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    // From a song, which has nothing to its left
    shown.select(Item::Song(SongId::new("s0")));
    key(&mut shown.ui, NamedKey::ArrowLeft);
    assert_eq!(focused_label().as_deref(), Some("Artists"));

    // From the second card of a row, along the row first
    shown.select(Item::Album(AlbumId::new("a1")));
    key(&mut shown.ui, NamedKey::ArrowLeft);
    assert_eq!(focused_label().as_deref(), Some("Artist"));
    assert_eq!(shown.item(), Some(Item::Album(AlbumId::new("a2"))));

    key(&mut shown.ui, NamedKey::ArrowLeft);
    assert_eq!(focused_label().as_deref(), Some("Artists"));
}

#[test]
fn an_artist_with_no_songs_at_all_says_so_under_albums() {
    let server = server(0);
    server.set_albums("m", Vec::new());
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    assert!(find(Role::Heading, "Albums").is_some());
    assert!(has_text("This artist has no albums."));
    let play = find(Role::Button, "Play").expect("Play");
    assert!(play.disabled);
}

#[test]
fn a_new_list_keeps_the_selection_unless_the_artist_has_gone() {
    let server = server(0);
    let mut shown = Shown::at(&server, artist("m"));
    shown.listed(2);

    server.set_artists(vec![artist_json("m", "Miles Davis", 2)]);
    shown.view.list.reload();
    shown.listed(1);
    assert_eq!(shown.location(), artist("m"));

    server.set_artists(vec![artist_json("z", "Zoot Sims", 1)]);
    shown.view.list.reload();
    let location = shown.location;
    wait_until(&mut shown.ui, || location.get_untracked() == artist("z"));
    assert_eq!(
        shown.last_sent(),
        Some(Command::ShowInPlace(artist("z"))),
        "the artist who has gone is nowhere to go back to"
    );
}

#[test]
fn a_new_list_keeps_an_artist_who_was_never_on_it() {
    let server = server(0);
    let mut shown = Shown::at(&server, artist("only-on-songs"));
    shown.listed(2);

    server.set_artists(vec![artist_json("m", "Miles Davis", 2)]);
    shown.view.list.reload();
    shown.listed(1);

    assert_eq!(shown.location(), artist("only-on-songs"));
}

#[test]
fn play_next_queues_every_song_of_the_artist_album_by_album() {
    let server = server(0);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    shown.focus_list();

    shown.keyed(NamedKey::Enter, alt());

    let Command::Enqueue { songs, placement } =
        shown.wait_for_sent(|command| matches!(command, Command::Enqueue { .. }))
    else {
        unreachable!();
    };
    assert_eq!(placement, Placement::Next);
    assert_eq!(ids(&songs), ["s2", "s3", "s0", "s1"]);
}

#[test]
fn shuffle_plays_every_song_of_the_artist_shuffled() {
    let server = server(0);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    press(&mut shown.ui, Role::Button, "Shuffle");

    let Command::Shuffle(songs) =
        shown.wait_for_sent(|command| matches!(command, Command::Shuffle(_)))
    else {
        unreachable!();
    };
    assert_eq!(songs.len(), 4);
}

#[test]
fn play_shows_it_is_fetching_until_the_songs_arrive() {
    let server = server(0);
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    server.hold("getAlbum");

    press(&mut shown.ui, Role::Button, "Play");
    shown.ui.update();

    assert_eq!(
        shown.view.fetching.get_untracked(),
        Some(ArtistId::new("m"))
    );
    assert!(find(Role::ProgressBar, "Loading the songs").is_some());
    assert!(find(Role::Button, "Play").unwrap().disabled);

    server.release("getAlbum");
    let Command::Play { songs, start } =
        shown.wait_for_sent(|command| matches!(command, Command::Play { .. }))
    else {
        unreachable!();
    };
    assert_eq!((songs.len(), start), (4, None));
    assert_eq!(shown.view.fetching.get_untracked(), None);
}

#[test]
fn a_click_is_somewhere_to_go_back_to_and_the_keys_are_not() {
    let server = server(0);
    let mut shown = Shown::on(&server);
    shown.listed(2);
    shown.page_of("b");

    click_text(&mut shown.ui, "Miles Davis");
    assert_eq!(shown.last_sent(), Some(Command::Show(artist("m"))));
    assert_eq!(shown.location(), artist("m"));

    key(&mut shown.ui, NamedKey::ArrowUp);
    assert_eq!(shown.last_sent(), Some(Command::ShowInPlace(artist("b"))));
    assert_eq!(shown.location(), artist("b"));
}

#[test]
fn the_secondary_button_opens_the_menu_of_the_artist_under_it() {
    let server = server(0);
    let mut shown = Shown::on(&server);
    shown.listed(2);
    shown.page_of("b");

    let (x, y) = middle_of("Miles Davis");
    right_click(&mut shown.ui, x, y);
    assert_eq!(shown.location(), artist("m"));
    press(&mut shown.ui, Role::MenuItem, "Add to Queue");

    let Command::Enqueue { songs, placement } =
        shown.wait_for_sent(|command| matches!(command, Command::Enqueue { .. }))
    else {
        unreachable!();
    };
    assert_eq!((songs.len(), placement), (4, Placement::Last));
}

#[test]
fn the_list_says_why_it_could_not_load_and_tries_again() {
    let server = server(0);
    server.refuse("getArtists");
    let mut shown = Shown::on(&server);
    let ui = &mut shown.ui;
    wait_until(ui, || {
        find(Role::Heading, "Couldn't load artists").is_some()
    });

    server.accept("getArtists");
    press(&mut shown.ui, Role::Button, "Retry");
    shown.listed(2);
    assert_eq!(shown.location(), artist("b"));
}

#[test]
fn a_refresh_shows_it_is_updating_and_why_it_failed() {
    let server = server(0);
    let mut shown = Shown::on(&server);
    shown.listed(2);

    server.hold("getArtists");
    shown.view.list.reload();
    let ui = &mut shown.ui;
    wait_until(ui, || {
        find(Role::ProgressBar, "Updating the artists").is_some()
    });

    server.refuse("getArtists");
    server.release("getArtists");
    let ui = &mut shown.ui;
    wait_until(ui, || has_text("Couldn't refresh the artists"));
    assert!(find(Role::ListOption, "Miles Davis").is_some());
}

#[test]
fn the_order_chosen_in_the_bar_sorts_the_list() {
    let server = server(0);
    let (core, hiphile) = signed_in_core(server.url());
    let made = Rc::new(RefCell::new(None));
    let mut ui = start({
        let made = made.clone();
        move || {
            let (commands, _) = next_ui::testing::recording_commands();
            let cx = screen_context(&hiphile, commands);
            let location = signal(Location::Artists(None));
            let (view, Screen { actions, .. }) = artists_view(location, Split::new(), &cx);
            *made.borrow_mut() = Some(view);
            div().children(actions)
        }
    });
    let view = made.borrow_mut().take().unwrap();

    press(&mut ui, Role::Button, "Sort by Name, Ascending");
    press(&mut ui, Role::MenuItem, "Release Date");

    let sort = view.list.status().sort();
    assert_eq!(
        (sort.order, sort.is_descending),
        (ArtistOrder::Released, true)
    );
    drop(core);
}

fn albums(count: usize) -> Vec<Value> {
    (0..count)
        .map(|ix| album_json(&format!("x{ix}"), 1970))
        .collect()
}

#[test]
fn a_long_artist_makes_only_the_cards_near_the_view() {
    let server = server(10);
    server.set_albums("m", albums(500));
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    assert!(has_text("Album x0"));
    assert!(!has_text("Album x499"));
    let cards = nodes(Role::ListOption)
        .into_iter()
        .filter(|node| {
            node.label
                .as_deref()
                .is_some_and(|l| l.starts_with("Album "))
        })
        .count();
    assert!(cards < 60, "{cards} cards made");
}

#[test]
fn a_row_of_cards_is_as_high_as_reckoned() {
    let server = server(0);
    server.set_albums("m", albums(40));
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");

    let layout = &shown.view.layout;
    let reckoned = reckoned_row(
        layout.width.get_untracked(),
        untrack(|| layout.columns.get()),
    );
    let laid_out = layout.album_row.get_untracked();
    assert!(
        (laid_out - reckoned).abs() < 1.5,
        "{laid_out} high, reckoned {reckoned}"
    );
}

#[test]
fn end_and_home_reach_albums_not_made_yet() {
    let server = server(10);
    server.set_albums("m", albums(500));
    server.set_similar("m", Vec::new());
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    shown.select(Item::Song(SongId::new("s0")));

    key(&mut shown.ui, NamedKey::End);
    frames(&mut shown.ui, 100);
    let last = shown
        .view
        .page_on_show()
        .and_then(|page| page.albums.items().last().map(|album| album.id().clone()))
        .unwrap();
    assert_eq!(shown.item(), Some(Item::Album(last.clone())));
    let page = shown.view.page_node.logical_rect().unwrap();
    let (_, y) = middle_of(&format!("Album {last}"));
    assert!(
        y > page.y && y < page.y + page.height,
        "the last album is in view"
    );

    key(&mut shown.ui, NamedKey::Home);
    frames(&mut shown.ui, 100);
    assert_eq!(shown.item(), Some(Item::Song(SongId::new("s0"))));
    let (_, y) = middle_of("Top songs");
    assert!(
        y > page.y && y < page.y + page.height,
        "the heading is in view"
    );
}

#[test]
fn another_artist_starts_at_the_top() {
    let server = server(10);
    server.set_albums("m", albums(500));
    server.set_albums("b", albums(500));
    let mut shown = Shown::at(&server, artist("m"));
    shown.page_of("m");
    shown.select(Item::Song(SongId::new("s0")));
    key(&mut shown.ui, NamedKey::End);
    frames(&mut shown.ui, 100);
    assert!(shown.view.page_node.scroll_offset().y > 0.0);

    shown.location.set(artist("b"));
    shown.page_of("b");

    assert_eq!(shown.view.page_node.scroll_offset().y, 0.0);
    assert_eq!(shown.item(), None);
}
