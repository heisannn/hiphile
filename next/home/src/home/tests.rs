use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use anti_webui::core::Ui;
use anti_webui::core::event::WindowEvent;
use anti_webui::core::geometry::{Scale, SizeI};
use anti_webui::core::tree::{NodeKind, with_doc};
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::Location;
use hiphile_core::test_support::HomeServer;
use next_ui::cover::install_covers;
use next_ui::keymap::Keystroke;
use next_ui::testing::{
    TestCore, click_text, double_click, find, focus, key, key_with, middle_of, nodes, press,
    recording_commands, right_click, screen_context, signed_in_core, start_at, wait_until,
};
use next_ui::{AlbumAction, Known};
use playback::Placement;
use subsonic::{AlbumId, ArtistId, SongId, StarTarget};

use super::*;

const HEIGHT: i32 = 760;
/// As wide as the window that has room for `n` cards across, from 2 to 12.
const TWO_ACROSS: i32 = 400;
const THREE_ACROSS: i32 = 600;
const FOUR_ACROSS: i32 = 736;
const SIX_ACROSS: i32 = 1200;
const SEVEN_ACROSS: i32 = 1280;

struct Shown {
    ui: Ui,
    server: HomeServer,
    known: Known,
    workspace: hiphile_core::Workspace,
    sent: Rc<RefCell<Vec<Command>>>,
    _core: TestCore,
}

/// Home on `server`, in a window `width` wide, before anything has come from the server.
fn home_on(server: HomeServer, width: i32) -> Shown {
    let (core, hiphile) = signed_in_core(server.url());
    let (commands, sent) = recording_commands();
    let made = Rc::new(RefCell::new(None));
    let ui = start_at(
        {
            let made = made.clone();
            move || {
                install_covers(hiphile.clone());
                let cx = screen_context(&hiphile, commands);
                made.replace(Some(cx.clone()));
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(home(&cx).body.flex_1().min_h(0))
                    .child(cx.snackbars.view())
            }
        },
        (width, HEIGHT),
    );
    let cx: ScreenContext = made.take().unwrap();
    let mut shown = Shown {
        ui,
        server,
        known: cx.known,
        workspace: cx.workspace.clone(),
        sent,
        _core: core,
    };
    shown.lay_out();
    shown
}

/// Home on `server` once every shelf has come, and whether each album and song is a favorite.
fn settled_home_on(server: HomeServer, width: i32) -> Shown {
    let mut shown = home_on(server, width);
    shown.settle();
    shown
}

impl Shown {
    fn settle(&mut self) {
        let known = self.known;
        let anything = StarTarget::Song(SongId::new("anything"));
        wait_until(&mut self.ui, || {
            nodes(Role::ProgressBar).is_empty() && known.is_favorite(&anything).is_some()
        });
        self.lay_out();
    }

    /// Lays out and draws what changed, which tells the page how wide it is.
    fn lay_out(&mut self) {
        self.ui.update();
        let _ = self.ui.paint(Instant::now());
        self.ui.update();
    }

    fn resize(&mut self, width: i32) {
        self.ui.handle_event(WindowEvent::Resized {
            size: SizeI::new(width, HEIGHT),
            scale: Scale::ONE,
        });
        self.lay_out();
    }

    fn keys(&mut self, keys: &[NamedKey]) {
        for &named in keys {
            key(&mut self.ui, named);
        }
    }

    fn stroke(&mut self, stroke: Keystroke) {
        key_with(&mut self.ui, stroke.key().clone(), stroke.modifiers());
    }

    fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

fn enter() -> Keystroke {
    Keystroke::named(NamedKey::Enter)
}

/// When the song at `ix` of those played last was played, the latest first.
fn played(ix: usize) -> String {
    format!("2026-11-03T22:{:02}:00Z", 59 - ix)
}

/// Albums named after their ids, `prefix` and a number from 0, each by an artist of its own.
fn add_albums(server: &HomeServer, prefix: &str, count: usize) -> Vec<String> {
    (0..count)
        .map(|ix| {
            let id = format!("{prefix}{ix}");
            server.add_album(&id, &id, &format!("Artist {id}"));
            id
        })
        .collect()
}

fn set_list(server: &HomeServer, list: &str, albums: &[String]) {
    let albums: Vec<&str> = albums.iter().map(String::as_str).collect();
    server.set_list(list, &albums);
}

/// A server whose songs played last are `songs`, the latest first, on the album Played, and whose
/// newest albums are `new_albums` from a0 on. It recommends nothing and has played nothing most.
fn server(songs: &[&str], new_albums: usize) -> HomeServer {
    let server = HomeServer::new();
    if !songs.is_empty() {
        server.add_album("p", "Played", "Player");
        for (ix, id) in songs.iter().enumerate() {
            server.add_song("p", id, id, Some(&played(ix)));
        }
        server.set_list("recent", &["p"]);
    }
    let new = add_albums(&server, "a", new_albums);
    set_list(&server, "newest", &new);
    server
}

/// Has `count` albums from r0 on to recommend: the random ones, as nothing else is to pick from.
fn recommending(server: HomeServer, count: usize) -> HomeServer {
    let picked = add_albums(&server, "r", count);
    set_list(&server, "random", &picked);
    server
}

fn headings() -> Vec<String> {
    nodes(Role::Heading)
        .into_iter()
        .filter(|node| node.level == Some(2))
        .filter_map(|node| node.label)
        .collect()
}

/// The cards and the rows of songs, from the top.
fn options() -> Vec<String> {
    nodes(Role::ListOption)
        .into_iter()
        .filter_map(|node| node.label)
        .collect()
}

/// The option selected, and how far down it is.
fn selected() -> Option<(String, f32)> {
    nodes(Role::ListOption)
        .into_iter()
        .find(|node| node.selected == Some(true))
        .map(|node| (node.label.unwrap_or_default(), node.bounds.y))
}

fn selected_label() -> Option<String> {
    selected().map(|(label, _)| label)
}

fn shows_text_starting(start: &str) -> bool {
    with_doc(|doc| {
        doc.nodes.iter().any(
            |(_, node)| matches!(&node.kind, NodeKind::Text(shown) if shown.text.starts_with(start)),
        )
    })
}

fn show_album(id: &str) -> Command {
    Command::Show(Location::Albums(Some(AlbumId::new(id))))
}

#[test]
fn no_plays_takes_the_song_and_most_played_shelves_away() {
    let _shown = settled_home_on(server(&[], 1), SIX_ACROSS);

    assert_eq!(headings(), ["New albums"]);
    assert_eq!(options(), ["a0"]);
}

#[test]
fn a_server_with_no_albums_says_so() {
    let _shown = settled_home_on(server(&[], 0), SIX_ACROSS);

    assert_eq!(headings(), ["New albums"]);
    assert!(shows_text_starting("None on this server"));
}

#[test]
fn loading_and_failing_shelves_keep_their_place() {
    let server = server(&["s1"], 1);
    server.hold("recent");
    server.refuse("frequent");
    server.refuse("newest");
    let mut shown = home_on(server, SIX_ACROSS);
    wait_until(&mut shown.ui, || {
        find(Role::Button, "Retry").is_some() && nodes(Role::ProgressBar).len() == 1
    });

    assert_eq!(
        headings(),
        [
            "Recommended",
            "Recently played",
            "Most played",
            "New albums"
        ]
    );
    assert!(find(Role::ProgressBar, "Loading Recently Played Songs").is_some());
    let retries = nodes(Role::Button)
        .into_iter()
        .filter(|button| {
            button
                .label
                .as_deref()
                .is_some_and(|l| l.ends_with("Retry"))
        })
        .count();
    assert_eq!(retries, 3);
    shown.server.release("recent");
}

fn retries() -> usize {
    nodes(Role::Button)
        .into_iter()
        .filter(|button| {
            button
                .label
                .as_deref()
                .is_some_and(|l| l.ends_with("Retry"))
        })
        .count()
}

#[test]
fn shelves_that_all_failed_alike_say_so_once_and_retry_together() {
    let server = recommending(server(&["s1"], 1), 1);
    let lists = ["newest", "recent", "frequent", "random"];
    for list in lists {
        server.refuse(list);
    }
    let mut shown = home_on(server, SIX_ACROSS);
    wait_until(&mut shown.ui, || {
        find(Role::Heading, "Couldn't load Home").is_some()
    });

    assert_eq!(headings(), ["Couldn't load Home"]);
    assert_eq!(retries(), 1);

    for list in lists {
        shown.server.accept(list);
    }
    press(&mut shown.ui, Role::Button, "Retry");
    wait_until(&mut shown.ui, || options() == ["r0", "s1", "a0"]);
    assert_eq!(headings(), ["Recommended", "Recently played", "New albums"]);
}

#[test]
fn new_albums_show_in_two_rows_of_as_many_as_fit() {
    let _shown = settled_home_on(server(&[], 24), THREE_ACROSS);
    assert_eq!(options(), ["a0", "a1", "a2", "a3", "a4", "a5"]);
}

#[test]
fn new_albums_fewer_than_fit_show_all() {
    let _shown = settled_home_on(server(&[], 4), THREE_ACROSS);
    assert_eq!(options(), ["a0", "a1", "a2", "a3"]);
}

#[test]
fn recommendations_come_first_in_one_row_of_as_many_as_fit() {
    let _shown = settled_home_on(recommending(server(&["s1"], 1), 12), THREE_ACROSS);

    assert_eq!(headings(), ["Recommended", "Recently played", "New albums"]);
    assert_eq!(options(), ["r0", "r1", "r2", "s1", "a0"]);
}

#[test]
fn most_played_comes_above_new_albums_in_one_row_of_as_many_as_fit() {
    let server = server(&["s1"], 1);
    let most = add_albums(&server, "m", 12);
    set_list(&server, "frequent", &most);
    let _shown = settled_home_on(server, THREE_ACROSS);

    assert_eq!(headings(), ["Recently played", "Most played", "New albums"]);
    assert_eq!(options(), ["s1", "m0", "m1", "m2", "a0"]);
}

#[test]
fn the_cards_across_follow_the_width_of_the_window() {
    let mut shown = settled_home_on(server(&["s1"], 24), SEVEN_ACROSS);
    let new_cards = || options().iter().filter(|o| o.starts_with('a')).count();

    assert_eq!(new_cards(), 2 * 7);
    shown.resize(FOUR_ACROSS);
    assert_eq!(new_cards(), 2 * 4);
    // No more than the shelves have albums for
    shown.resize(300 * 16);
    assert_eq!(new_cards(), 2 * MAX_COLUMNS);
}

#[test]
fn the_keys_pass_through_an_album_on_two_shelves() {
    let server = server(&[], 1);
    server.set_list("frequent", &["a0"]);
    let mut shown = settled_home_on(server, SIX_ACROSS);

    shown.keys(&[NamedKey::ArrowDown]);
    let (label, most_played) = selected().unwrap();
    assert_eq!(label, "a0");
    shown.keys(&[NamedKey::ArrowDown]);
    let (label, new) = selected().unwrap();
    assert_eq!(label, "a0");
    assert!(new > most_played);
    shown.keys(&[NamedKey::ArrowUp]);
    assert_eq!(selected().unwrap().1, most_played);
}

#[test]
fn the_arrow_keys_move_from_the_recommendations_to_the_songs() {
    let mut shown = settled_home_on(recommending(server(&["s1"], 1), 2), SIX_ACROSS);

    shown.keys(&[NamedKey::ArrowDown]);
    assert_eq!(selected_label().as_deref(), Some("r0"));
    shown.keys(&[NamedKey::ArrowRight]);
    assert_eq!(selected_label().as_deref(), Some("r1"));
    shown.keys(&[NamedKey::ArrowDown]);
    assert_eq!(selected_label().as_deref(), Some("s1"));
}

#[test]
fn the_arrow_keys_move_through_the_songs_into_the_albums() {
    let mut shown = settled_home_on(server(&["s1", "s2"], 4), TWO_ACROSS);
    let mut moved = |keys: &[NamedKey]| {
        shown.keys(keys);
        selected_label().unwrap()
    };

    assert_eq!(moved(&[NamedKey::ArrowDown]), "s1");
    assert_eq!(moved(&[NamedKey::ArrowDown, NamedKey::ArrowDown]), "a0");
    assert_eq!(moved(&[NamedKey::ArrowRight]), "a1");
    assert_eq!(moved(&[NamedKey::ArrowDown]), "a3");
    assert_eq!(moved(&[NamedKey::PageUp]), "s1");
    assert_eq!(moved(&[NamedKey::PageDown]), "a2");
    assert_eq!(moved(&[NamedKey::Home]), "s1");
    assert_eq!(moved(&[NamedKey::End]), "a3");
}

#[test]
fn enter_shows_a_recommended_album_in_albums() {
    let mut shown = settled_home_on(recommending(server(&[], 0), 1), SIX_ACROSS);

    shown.keys(&[NamedKey::ArrowDown, NamedKey::Enter]);

    assert_eq!(shown.sent(), [show_album("r0")]);
}

#[test]
fn the_keys_with_enter_play_and_queue_an_album_once_its_songs_are_had() {
    let mut shown = settled_home_on(server(&[], 1), SIX_ACROSS);

    shown.keys(&[NamedKey::ArrowDown]);
    shown.stroke(enter().with_primary());
    shown.stroke(enter().with_alt());
    shown.stroke(enter().with_shift());

    let acting = |action| Command::ActOnAlbum {
        album: AlbumId::new("a0"),
        action,
    };
    assert_eq!(
        shown.sent(),
        [
            acting(AlbumAction::Play),
            acting(AlbumAction::Enqueue(Placement::Next)),
            acting(AlbumAction::Enqueue(Placement::Last)),
        ]
    );
}

#[test]
fn enter_plays_the_songs_from_the_one_selected() {
    let mut shown = settled_home_on(server(&["s1", "s2"], 0), SIX_ACROSS);

    shown.keys(&[NamedKey::ArrowDown, NamedKey::ArrowDown, NamedKey::Enter]);

    let [Command::Play { songs, start }] = &shown.sent()[..] else {
        panic!("the songs play");
    };
    let ids: Vec<&str> = songs.iter().map(|song| song.id().as_str()).collect();
    assert_eq!(ids, ["s1", "s2"]);
    assert_eq!(*start, Some(1));
}

#[test]
fn a_song_goes_to_its_album_and_its_artist() {
    let mut shown = settled_home_on(server(&["s1"], 0), SIX_ACROSS);

    shown.keys(&[NamedKey::ArrowDown]);
    shown.stroke(Keystroke::primary('l'));
    shown.stroke(Keystroke::primary('a').with_shift());

    assert_eq!(
        shown.sent(),
        [
            show_album("p"),
            Command::Show(Location::Artists(Some(ArtistId::new("artist-Player")))),
        ]
    );
}

#[test]
fn the_favorite_key_follows_the_song_or_album_selected() {
    let mut shown = settled_home_on(recommending(server(&["s"], 0), 1), SIX_ACROSS);
    let key = Keystroke::primary('d');

    shown.keys(&[NamedKey::ArrowDown]);
    shown.stroke(key.clone());
    shown.keys(&[NamedKey::ArrowDown]);
    shown.stroke(key);

    let asked: Vec<StarTarget> = shown
        .sent()
        .into_iter()
        .map(|command| match command {
            Command::ToggleFavorite(favorite) => favorite.target(),
            command => panic!("{command:?} sent"),
        })
        .collect();
    assert_eq!(
        asked,
        [
            StarTarget::Album(AlbumId::new("r0")),
            StarTarget::Song(SongId::new("s")),
        ]
    );
}

#[test]
fn the_favorite_key_waits_until_the_favorites_are_known() {
    let server = server(&[], 1);
    server.hold("getStarred2");
    let mut shown = home_on(server, SIX_ACROSS);
    wait_until(&mut shown.ui, || find(Role::ListOption, "a0").is_some());

    shown.keys(&[NamedKey::ArrowDown]);
    shown.stroke(Keystroke::primary('d'));

    assert!(shown.sent().is_empty());
    shown.server.release("getStarred2");
}

#[test]
fn commands_ignore_a_selection_a_narrower_window_took_off_the_screen() {
    let mut shown = settled_home_on(recommending(server(&[], 0), 6), SIX_ACROSS);
    shown.keys(&[NamedKey::ArrowDown, NamedKey::End]);
    assert_eq!(selected_label().as_deref(), Some("r5"));

    shown.resize(FOUR_ACROSS);
    shown.keys(&[NamedKey::Enter]);
    assert!(shown.sent().is_empty());

    shown.resize(SIX_ACROSS);
    assert_eq!(selected_label().as_deref(), Some("r5"));
    shown.keys(&[NamedKey::Enter]);
    assert_eq!(shown.sent(), [show_album("r5")]);
}

#[test]
fn a_reload_keeps_the_selection_of_a_song_still_there() {
    let mut shown = settled_home_on(server(&["s1", "s2"], 0), SIX_ACROSS);
    shown.keys(&[NamedKey::ArrowDown, NamedKey::ArrowDown]);
    let asked = shown.server.asked("recent");

    shown.workspace.shelves().reload_recently_played();
    let server = &shown.server;
    wait_until(&mut shown.ui, || {
        server.asked("recent") > asked && nodes(Role::ProgressBar).is_empty()
    });

    assert_eq!(selected_label().as_deref(), Some("s2"));
}

/// `key` presses a focused Reload, which picks again though today's albums are on show. The
/// server refuses, so they stay and a snackbar says so.
fn key_reloads_the_recommendations(key: NamedKey) {
    let mut shown = settled_home_on(recommending(server(&[], 1), 1), SIX_ACROSS);
    shown.keys(&[NamedKey::ArrowDown]);
    shown.server.refuse("random");
    focus(&mut shown.ui, Role::Button, "Reload Recommendations");

    shown.keys(&[key]);

    let server = &shown.server;
    wait_until(&mut shown.ui, || {
        server.asked("random") == 2
            && find(Role::Button, "Reload Recommendations").is_some()
            && shows_text_starting("Couldn't get new recommendations.")
    });
    assert_eq!(options(), ["r0", "a0"]);
    assert!(shown.sent().is_empty());
    // The page has kept the focus while the button was away
    shown.keys(&[NamedKey::ArrowDown]);
    assert_eq!(selected_label().as_deref(), Some("a0"));
}

#[test]
fn enter_reloads_the_recommendations() {
    key_reloads_the_recommendations(NamedKey::Enter);
}

#[test]
fn space_reloads_the_recommendations() {
    key_reloads_the_recommendations(NamedKey::Space);
}

/// `key` presses a focused Retry, which Home's own use of the key must not take.
fn key_presses_retry(key: NamedKey) {
    let server = server(&["s1"], 1);
    server.refuse("getAlbum");
    let mut shown = settled_home_on(server, SIX_ACROSS);
    assert_eq!(options(), ["a0"]);
    shown.keys(&[NamedKey::ArrowDown]);
    shown.server.accept("getAlbum");
    focus(&mut shown.ui, Role::Button, "Retry");

    shown.keys(&[key]);

    wait_until(&mut shown.ui, || find(Role::ListOption, "s1").is_some());
    assert!(shown.sent().is_empty());
    // The page has kept the focus while the button was away
    shown.keys(&[NamedKey::ArrowUp]);
    assert_eq!(selected_label().as_deref(), Some("s1"));
}

#[test]
fn enter_presses_a_focused_retry() {
    key_presses_retry(NamedKey::Enter);
}

#[test]
fn space_presses_a_focused_retry() {
    key_presses_retry(NamedKey::Space);
}

#[test]
fn clicking_a_card_shows_its_album() {
    let mut shown = settled_home_on(server(&[], 2), SIX_ACROSS);

    click_text(&mut shown.ui, "a1");

    assert_eq!(selected_label().as_deref(), Some("a1"));
    assert_eq!(shown.sent(), [show_album("a1")]);
}

#[test]
fn the_artist_on_a_card_goes_to_them_and_not_to_the_album() {
    let mut shown = settled_home_on(server(&[], 1), SIX_ACROSS);

    click_text(&mut shown.ui, "Artist a0");

    assert_eq!(
        shown.sent(),
        [Command::Show(Location::Artists(Some(ArtistId::new(
            "artist-Artist a0"
        ))))]
    );
}

#[test]
fn a_click_selects_a_song_and_a_double_click_plays_from_it() {
    let mut shown = settled_home_on(server(&["s1", "s2"], 0), SIX_ACROSS);
    let (x, y) = middle_of("s2");

    next_ui::testing::click(&mut shown.ui, x, y);
    assert_eq!(selected_label().as_deref(), Some("s2"));
    assert!(shown.sent().is_empty());

    double_click(&mut shown.ui, x, y);
    let [Command::Play { start, .. }] = &shown.sent()[..] else {
        panic!("the songs play");
    };
    assert_eq!(*start, Some(1));
}

#[test]
fn the_menu_of_an_album_selects_it_and_shows_it_in_albums() {
    let mut shown = settled_home_on(server(&[], 2), SIX_ACROSS);
    let (x, y) = middle_of("a1");

    right_click(&mut shown.ui, x, y);

    assert_eq!(selected_label().as_deref(), Some("a1"));
    assert!(shown.sent().is_empty());
    press(&mut shown.ui, Role::MenuItem, "Show in Albums");
    assert_eq!(shown.sent(), [show_album("a1")]);
}

#[test]
fn the_menu_of_a_song_plays_from_it() {
    let mut shown = settled_home_on(server(&["s1", "s2"], 0), SIX_ACROSS);
    let (x, y) = middle_of("s2");

    right_click(&mut shown.ui, x, y);
    press(&mut shown.ui, Role::MenuItem, "Play");

    let [Command::Play { start, .. }] = &shown.sent()[..] else {
        panic!("the songs play");
    };
    assert_eq!(*start, Some(1));
}

#[test]
fn the_menu_of_an_album_makes_it_a_favorite() {
    let mut shown = settled_home_on(server(&[], 1), SIX_ACROSS);
    let (x, y) = middle_of("a0");

    right_click(&mut shown.ui, x, y);
    press(&mut shown.ui, Role::MenuItem, "Add to Favorites");

    let [Command::ToggleFavorite(Favorite::Album(album))] = &shown.sent()[..] else {
        panic!("the album is a favorite");
    };
    assert_eq!(album.id().as_str(), "a0");
}
