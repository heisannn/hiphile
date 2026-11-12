use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::core::access::AccessNode;
use anti_webui::core::event::{Key, Modifiers};
use hiphile_core::Hiphile;
use hiphile_core::test_support::FavoritesServer;
use next_ui::testing::{
    NOWHERE, TestCore, double_click, drag, find, focus, frames, has_text, key, key_with, nodes,
    press, recording_commands, right_click, screen_context, signed_in_core, start, wait_until,
};
use serde_json::json;

use super::*;

fn song(id: &str) -> Song {
    serde_json::from_value(json!({
        "id": id,
        "title": id,
        "artist": "Artist",
        "artistId": "ar-1",
        "album": "Album",
        "albumId": "al-1",
        "duration": 200,
    }))
    .unwrap()
}

fn songs(ids: &[&str]) -> Vec<Song> {
    ids.iter().copied().map(song).collect()
}

struct Shown {
    _core: TestCore,
    ui: Ui,
    player: playback::Player,
    sent: Rc<RefCell<Vec<Command>>>,
    panels: Panels,
    known: Known,
}

/// The queue of a player that plays nothing, holding a song for each id with the one at `current`
/// current. What it asks of the shell is kept in `sent`.
fn queue_of(ids: &[&str], current: usize) -> Shown {
    queue_on(NOWHERE, ids, current)
}

fn queue_on(server: &str, ids: &[&str], current: usize) -> Shown {
    let (core, hiphile) = signed_in_core(server);
    let player = hiphile.workspace().unwrap().player().clone();
    player.fill_queue(&songs(ids), current);
    let (commands, sent) = recording_commands();
    let made = Rc::new(RefCell::new(None));
    let ui = start({
        let made = made.clone();
        move || view(&hiphile, commands, &made)
    });
    let (panels, known) = made.take().expect("the view to have been made");
    Shown {
        _core: core,
        ui,
        player,
        sent,
        panels,
        known,
    }
}

fn view(hiphile: &Hiphile, commands: Commands, made: &RefCell<Option<(Panels, Known)>>) -> Div {
    let cx = screen_context(hiphile, commands);
    let panels = Panels::new();
    made.replace(Some((panels, cx.known)));
    div()
        .size_full()
        .flex()
        .flex_row()
        .child(div().flex_1())
        .child(queue(&cx, panels))
}

impl Shown {
    fn ids(&self) -> Vec<String> {
        self.player
            .snapshot()
            .queue()
            .entries()
            .iter()
            .map(|entry| entry.item().id().as_str().to_owned())
            .collect()
    }

    fn id_of(&self, song: &str) -> EntryId {
        let shown = self.player.snapshot();
        shown
            .queue()
            .entries()
            .iter()
            .find(|entry| entry.item().id().as_str() == song)
            .expect("the song in the queue")
            .id()
    }

    /// Where the selected entries are, in order.
    fn selected(&self) -> Vec<usize> {
        let shown = self.player.snapshot();
        let queue = shown.queue();
        let listed = queue.entries().iter().map(|entry| entry.id());
        self.panels
            .queue_selection
            .in_order(listed)
            .into_iter()
            .filter_map(|id| queue.position(id))
            .collect()
    }

    /// Lets what changed in the core come to the view.
    fn settle(&mut self) {
        frames(&mut self.ui, 50);
    }

    fn focus_list(&mut self) {
        focus(&mut self.ui, Role::ListBox, "Queue");
    }

    fn key(&mut self, named: NamedKey) {
        key(&mut self.ui, named);
        self.settle();
    }

    fn key_with(&mut self, named: NamedKey, modifiers: Modifiers) {
        key_with(&mut self.ui, Key::Named(named), modifiers);
        self.settle();
    }

    fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

fn row(label: &str) -> AccessNode {
    find(Role::ListOption, label).unwrap_or_else(|| panic!("no row {label:?}"))
}

/// A point in the row named `label`, `down` of the way down it.
fn in_row(label: &str, down: f32) -> (f32, f32) {
    let bounds = row(label).bounds;
    (
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height * down,
    )
}

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Modifiers::default()
    }
}

#[test]
fn a_rows_name_says_how_far_along_it_is() {
    let mut shown = queue_of(&["a", "b", "c"], 1);
    assert!(find(Role::ListOption, "a, played").is_some());
    assert!(find(Role::ListOption, "b, paused").is_some());
    assert!(find(Role::ListOption, "c").is_some());

    shown.player.pretend_playing(true);
    wait_until(&mut shown.ui, || {
        find(Role::ListOption, "b, playing").is_some()
    });
}

#[test]
fn double_clicking_a_row_plays_its_entry() {
    let mut shown = queue_of(&["a", "b", "c"], 0);
    let (x, y) = in_row("c", 0.5);

    double_click(&mut shown.ui, x, y);

    assert_eq!(shown.sent(), [Command::PlayEntry(shown.id_of("c"))]);
    assert_eq!(shown.selected(), [2]);
}

#[test]
fn the_keys_move_the_selection_starting_from_the_current_entry() {
    let mut shown = queue_of(&["a", "b", "c", "d"], 1);
    shown.focus_list();

    for (key, at) in [
        (NamedKey::ArrowDown, 1),
        (NamedKey::ArrowDown, 2),
        (NamedKey::ArrowUp, 1),
        (NamedKey::End, 3),
        (NamedKey::Home, 0),
    ] {
        shown.key(key);
        assert_eq!(shown.selected(), [at], "after {key:?}");
    }

    shown.key(NamedKey::Enter);
    assert_eq!(shown.sent(), [Command::PlayEntry(shown.id_of("a"))]);
}

#[test]
fn the_page_keys_move_by_the_rows_in_view() {
    let ids: Vec<String> = (0..100).map(|ix| ix.to_string()).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut shown = queue_of(&ids, 0);
    shown.focus_list();
    let height = find(Role::ListBox, "Queue").unwrap().bounds.height;
    let page = (height / ROW_HEIGHT).floor() as usize;
    assert!(page > 1, "the window shows {page} rows");

    shown.key(NamedKey::Home);
    shown.key(NamedKey::PageDown);
    assert_eq!(shown.selected(), [page]);
    shown.key(NamedKey::PageDown);
    assert_eq!(shown.selected(), [page * 2]);
    shown.key(NamedKey::PageUp);
    assert_eq!(shown.selected(), [page]);
}

#[test]
fn an_entry_dragged_down_lands_after_the_row_dropped_on() {
    let mut shown = queue_of(&["a", "b", "c"], 1);

    drag(&mut shown.ui, in_row("a, played", 0.5), in_row("c", 0.75));

    assert_eq!(shown.ids(), ["b", "c", "a"]);
    assert_eq!(shown.selected(), [2]);
    assert_eq!(shown.player.snapshot().current_song(), Some(&song("b")));
}

#[test]
fn an_entry_dragged_up_lands_before_the_row_dropped_on() {
    let mut shown = queue_of(&["a", "b", "c"], 0);

    drag(&mut shown.ui, in_row("c", 0.5), in_row("a, paused", 0.25));

    assert_eq!(shown.ids(), ["c", "a", "b"]);
    assert_eq!(shown.selected(), [0]);
}

#[test]
fn the_selected_entries_are_dragged_together() {
    let mut shown = queue_of(&["a", "b", "c", "d", "e"], 0);
    shown.focus_list();
    shown.key(NamedKey::End);
    shown.key_with(NamedKey::ArrowUp, Modifiers::default().with_shift());
    assert_eq!(shown.selected(), [3, 4]);

    drag(&mut shown.ui, in_row("d", 0.5), in_row("b", 0.25));

    assert_eq!(shown.ids(), ["a", "d", "e", "b", "c"]);
}

#[test]
fn option_and_the_arrows_move_the_selected_entry() {
    let mut shown = queue_of(&["a", "b", "c"], 0);
    shown.focus_list();
    shown.key(NamedKey::End);

    shown.key_with(NamedKey::ArrowUp, alt());
    assert_eq!(shown.ids(), ["a", "c", "b"]);
    shown.key_with(NamedKey::ArrowUp, alt());
    assert_eq!(shown.ids(), ["c", "a", "b"]);
    assert_eq!(shown.selected(), [0]);
    shown.key_with(NamedKey::ArrowDown, alt());
    assert_eq!(shown.ids(), ["a", "c", "b"]);
}

#[test]
fn option_enter_plays_the_selected_entries_next() {
    let mut shown = queue_of(&["a", "b", "c", "d"], 0);
    shown.focus_list();
    shown.key(NamedKey::End);

    shown.key_with(NamedKey::Enter, alt());

    assert_eq!(shown.ids(), ["a", "d", "b", "c"]);
    assert!(shown.sent().is_empty());
}

#[test]
fn removing_an_entry_selects_the_one_in_its_place() {
    let mut shown = queue_of(&["a", "b", "c"], 0);
    shown.focus_list();
    shown.key(NamedKey::End);
    shown.key(NamedKey::ArrowUp);

    shown.key(NamedKey::Backspace);
    assert_eq!(shown.ids(), ["a", "c"]);
    assert_eq!(shown.selected(), [1]);

    shown.key(NamedKey::Delete);
    assert_eq!(shown.ids(), ["a"]);
    assert_eq!(shown.selected(), [0]);
}

#[test]
fn the_current_entry_stays() {
    let mut shown = queue_of(&["a", "b"], 0);
    shown.focus_list();
    shown.key(NamedKey::ArrowDown);
    assert_eq!(shown.selected(), [0]);

    shown.key(NamedKey::Backspace);

    assert_eq!(shown.ids(), ["a", "b"]);
}

#[test]
fn the_selection_stays_on_its_entry_through_a_shuffle() {
    let mut shown = queue_of(&["a", "b", "c", "d", "e", "f", "g", "h"], 0);
    shown.focus_list();
    shown.key(NamedKey::End);
    let selected = shown.panels.queue_selection.cursor();

    shown.player.toggle_shuffle();
    shown.settle();
    assert_eq!(shown.panels.queue_selection.cursor(), selected);
    assert_eq!(shown.ids()[0], "a");

    shown.player.toggle_shuffle();
    shown.settle();
    assert_eq!(shown.panels.queue_selection.cursor(), selected);
    assert_eq!(shown.selected(), [7]);
}

#[test]
fn going_back_in_order_brings_the_current_entry_into_view() {
    let ids: Vec<String> = (0..100).map(|ix| ix.to_string()).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut shown = queue_of(&ids, 0);
    shown.player.toggle_shuffle();
    // Shuffled, the song chosen comes first, wherever it was
    shown.player.fill_queue(&songs(&ids), 80);
    wait_until(&mut shown.ui, || {
        find(Role::ListOption, "80, paused").is_some()
    });

    shown.player.toggle_shuffle();

    wait_until(&mut shown.ui, || {
        find(Role::ListOption, "79, played").is_some()
            && find(Role::ListOption, "80, paused").is_some()
    });
}

#[test]
fn removing_an_entry_before_the_current_one_leaves_the_rows_where_they_are() {
    let ids: Vec<String> = (0..100).map(|ix| ix.to_string()).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut shown = queue_of(&ids, 80);
    wait_until(&mut shown.ui, || {
        find(Role::ListOption, "80, paused").is_some()
    });
    shown.focus_list();
    shown.key(NamedKey::Home);
    wait_until(&mut shown.ui, || {
        find(Role::ListOption, "0, played").is_some()
    });

    shown.key(NamedKey::Backspace);

    wait_until(&mut shown.ui, || {
        find(Role::ListOption, "0, played").is_none()
    });
    shown.settle();
    assert!(find(Role::ListOption, "1, played").is_some());
    assert!(find(Role::ListOption, "80, paused").is_none());
}

#[test]
fn the_menu_acts_on_the_entry_under_the_pointer() {
    let mut shown = queue_of(&["a", "b", "c"], 0);
    let (x, y) = in_row("c", 0.5);

    right_click(&mut shown.ui, x, y);
    press(&mut shown.ui, Role::MenuItem, "Play ↩");
    assert_eq!(shown.sent(), [Command::PlayEntry(shown.id_of("c"))]);
    frames(&mut shown.ui, 400);

    right_click(&mut shown.ui, x, y);
    press(&mut shown.ui, Role::MenuItem, "Remove ⌫");
    assert_eq!(shown.ids(), ["a", "b"]);
}

#[test]
fn the_menu_of_the_current_entry_cannot_remove_it_or_play_it_next() {
    let mut shown = queue_of(&["a", "b", "c"], 1);
    let (x, y) = in_row("b, paused", 0.5);

    right_click(&mut shown.ui, x, y);

    // An item's name ends with the key shown after it
    let item = |label| find(Role::MenuItem, label).unwrap_or_else(|| panic!("no {label}"));
    assert!(item("Remove ⌫").disabled);
    assert!(item("Play Next ⌥↩").disabled);
    assert!(!item("Move Up ⌥↑").disabled);
    assert!(!item("Move Down ⌥↓").disabled);
    assert!(!item("Go to Artist ⇧⌘A").disabled);
    assert!(!item("Go to Album").disabled);
}

#[test]
fn the_favorite_key_follows_the_entry_selected() {
    let server = FavoritesServer::new(&[]);
    let mut shown = queue_on(server.url(), &["a", "b"], 0);
    let known = shown.known;
    wait_until(&mut shown.ui, || {
        known
            .is_favorite(&Favorite::Song(song("a")).target())
            .is_some()
    });
    shown.focus_list();
    shown.key(NamedKey::End);

    key_with(
        &mut shown.ui,
        Key::Character("d".into()),
        Modifiers::PRIMARY,
    );

    assert_eq!(
        shown.sent(),
        [Command::ToggleFavorite(Favorite::Song(song("b")))]
    );
}

#[test]
fn the_keys_go_to_the_artist_add_to_a_playlist_and_play_or_pause() {
    let mut shown = queue_of(&["a", "b"], 0);
    shown.focus_list();
    shown.key(NamedKey::End);

    key_with(
        &mut shown.ui,
        Key::Character("a".into()),
        Modifiers::PRIMARY.with_shift(),
    );
    key_with(
        &mut shown.ui,
        Key::Character("p".into()),
        Modifiers::PRIMARY,
    );
    shown.key(NamedKey::Space);

    assert_eq!(
        shown.sent(),
        [
            Command::Show(Location::Artists(Some(subsonic::ArtistId::new("ar-1")))),
            Command::ChoosePlaylist(Arc::from([song("b")])),
            Command::TogglePlayback,
        ]
    );
}

#[test]
fn saving_asks_the_shell_and_clearing_keeps_the_current_entry() {
    let mut shown = queue_of(&["a", "b", "c"], 1);

    press(&mut shown.ui, Role::Button, "Save as Playlist…");
    assert_eq!(shown.sent(), [Command::SaveQueue]);

    press(&mut shown.ui, Role::Button, "Clear");
    assert_eq!(shown.ids(), ["b"]);
    wait_until(&mut shown.ui, || {
        find(Role::Button, "Clear").unwrap().disabled
    });
}

#[test]
fn an_empty_queue_says_how_to_fill_it() {
    let _shown = queue_of(&[], 0);

    assert!(has_text("Add songs with Play Next or Add to Queue."));
    assert!(find(Role::Button, "Save as Playlist…").unwrap().disabled);
    assert!(find(Role::Button, "Clear").unwrap().disabled);
    assert!(find(Role::ListBox, "Queue").is_some());
}

#[test]
fn it_counts_its_songs() {
    let mut shown = queue_of(&["a", "b", "c"], 0);
    assert!(has_text("3 songs"));

    shown.player.fill_queue(&songs(&["a", "b", "c", "d"]), 0);
    wait_until(&mut shown.ui, || has_text("4 songs"));
    assert_eq!(nodes(Role::ListOption).len(), 4);
}

#[test]
fn moving_several_entries_keeps_their_order() {
    let shown = queue_of(&["a", "b", "c", "d", "e"], 0);
    let ids = [shown.id_of("a"), shown.id_of("c")];

    move_entries(&shown.player, &ids, 4);
    assert_eq!(shown.ids(), ["b", "d", "a", "c", "e"]);

    move_entries(&shown.player, &ids, 0);
    assert_eq!(shown.ids(), ["a", "c", "b", "d", "e"]);

    move_entries(&shown.player, &ids, 5);
    assert_eq!(shown.ids(), ["b", "d", "e", "a", "c"]);

    move_entries(&shown.player, &ids, 3);
    assert_eq!(shown.ids(), ["b", "d", "e", "a", "c"]);
}
