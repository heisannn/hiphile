use std::cell::Cell;

use anti_webui::core::Ui;
use anti_webui::core::access::Checked;
use anti_webui::core::event::NamedKey;
use hiphile_core::Hiphile;
use hiphile_core::navigation::Location;
use next_ui::testing::{
    NOWHERE, core, find, frames, key, nodes, press, signed_in_core, start, start_at, wait_until,
};
use serde_json::json;
use subsonic::Song;

use super::*;
use crate::app;

fn at(destination: Destination) -> Shown {
    Shown {
        destination: Some(destination),
        ..Shown::default()
    }
}

/// Someone signed in, with a song that plays and the queue open.
fn playing() -> Shown {
    Shown {
        can_go_back: true,
        can_go_forward: true,
        has_song: true,
        has_queue: true,
        repeat: Some(RepeatMode::Off),
        queue_fits: true,
        queue_shown: true,
        ..at(Destination::Home)
    }
}

/// The items of the menus, with the menus they are in, in order.
fn every_item() -> Vec<(Command, Option<Shortcut>)> {
    fn walk(items: Vec<Item<Command>>, found: &mut Vec<(Command, Option<Shortcut>)>) {
        for item in items {
            match item {
                Item::Command {
                    command, shortcut, ..
                } => found.push((command, shortcut)),
                Item::Submenu(menu) => walk(menu.items, found),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    for menu in menus() {
        walk(menu.items, &mut found);
    }
    found
}

fn every_command() -> Vec<Command> {
    every_item()
        .into_iter()
        .map(|(command, _)| command)
        .collect()
}

#[test]
fn signed_out_nothing_can_be_chosen() {
    for command in every_command() {
        assert_eq!(
            state_of(&Shown::default(), &command),
            ItemState::default(),
            "{command:?}"
        );
    }
}

#[test]
fn nothing_can_be_chosen_while_something_covers_the_screens() {
    let shown = Shown {
        covered: true,
        ..playing()
    };
    for command in every_command() {
        assert!(!state_of(&shown, &command).is_enabled(), "{command:?}");
    }
}

#[test]
fn the_destination_shown_is_checked() {
    let shown = at(Destination::Albums);
    for destination in Destination::ALL {
        let state = state_of(&shown, &Command::Show(location_of(destination)));
        assert!(state.is_enabled(), "{destination:?}");
        assert_eq!(
            state.is_checked(),
            destination == Destination::Albums,
            "{destination:?}"
        );
    }
    let in_an_album = state_of(&shown, &Command::Show(Location::Albums(None)));
    assert!(in_an_album.is_checked());
}

#[test]
fn back_and_forward_go_only_where_there_is_history() {
    let home = at(Destination::Home);
    assert!(!state_of(&home, &Command::GoBack).is_enabled());
    assert!(!state_of(&home, &Command::GoForward).is_enabled());

    let between = Shown {
        can_go_back: true,
        can_go_forward: true,
        ..home
    };
    assert!(state_of(&between, &Command::GoBack).is_enabled());
    assert!(state_of(&between, &Command::GoForward).is_enabled());
}

#[test]
fn with_a_song_and_the_queue_open_everything_can_be_chosen() {
    for command in every_command() {
        assert!(state_of(&playing(), &command).is_enabled(), "{command:?}");
    }
}

#[test]
fn what_acts_on_the_song_waits_for_one() {
    let shown = Shown {
        has_song: false,
        ..playing()
    };
    for command in [
        Command::TogglePlayback,
        Command::Next,
        Command::Previous,
        Command::SeekForward,
        Command::SeekBackward,
    ] {
        assert!(!state_of(&shown, &command).is_enabled(), "{command:?}");
    }
    for command in [
        Command::VolumeUp,
        Command::VolumeDown,
        Command::ToggleMute,
        Command::ToggleShuffle,
        Command::SetRepeat(RepeatMode::All),
        Command::SaveQueue,
    ] {
        assert!(state_of(&shown, &command).is_enabled(), "{command:?}");
    }
}

#[test]
fn an_empty_queue_cannot_be_saved() {
    let shown = Shown {
        has_song: false,
        has_queue: false,
        ..playing()
    };
    assert!(!state_of(&shown, &Command::SaveQueue).is_enabled());
}

#[test]
fn the_queue_is_checked_while_it_shows_and_cannot_open_where_it_does_not_fit() {
    let queue = Command::ToggleQueue;
    assert!(state_of(&playing(), &queue).is_checked());

    let closed = Shown {
        queue_shown: false,
        ..playing()
    };
    assert!(!state_of(&closed, &queue).is_checked());
    assert!(state_of(&closed, &queue).is_enabled());

    let narrow = Shown {
        queue_fits: false,
        queue_shown: false,
        ..playing()
    };
    assert!(!state_of(&narrow, &queue).is_enabled());
}

#[test]
fn the_lyrics_open_on_a_song_and_are_checked_while_open() {
    let lyrics = Command::ToggleLyrics;
    let no_song = Shown {
        has_song: false,
        ..playing()
    };
    assert!(!state_of(&no_song, &lyrics).is_enabled());
    assert!(!state_of(&playing(), &lyrics).is_checked());

    let open = Shown {
        lyrics_open: true,
        queue_shown: false,
        ..playing()
    };
    assert!(state_of(&open, &lyrics).is_checked());
}

#[test]
fn over_the_lyrics_back_closes_them_and_forward_waits() {
    let open = Shown {
        can_go_back: false,
        lyrics_open: true,
        ..playing()
    };
    assert!(state_of(&open, &Command::GoBack).is_enabled());
    assert!(!state_of(&open, &Command::GoForward).is_enabled());
}

#[test]
fn shuffle_and_the_repeat_mode_are_checked() {
    let shown = Shown {
        shuffled: true,
        repeat: Some(RepeatMode::One),
        ..playing()
    };
    assert!(state_of(&shown, &Command::ToggleShuffle).is_checked());
    let checked: Vec<_> = [RepeatMode::Off, RepeatMode::All, RepeatMode::One]
        .into_iter()
        .filter(|mode| state_of(&shown, &Command::SetRepeat(*mode)).is_checked())
        .collect();
    assert_eq!(checked, [RepeatMode::One]);
}

#[test]
fn no_two_items_have_the_same_key() {
    let keys: Vec<_> = every_item()
        .into_iter()
        .filter_map(|(_, shortcut)| shortcut)
        .collect();
    for (at, key) in keys.iter().enumerate() {
        assert!(!keys[at + 1..].contains(key), "{key:?}");
    }
}

#[test]
fn the_controls_have_the_keys_of_the_player() {
    let key_of = |command: Command| {
        every_item()
            .into_iter()
            .find(|(each, _)| *each == command)
            .unwrap_or_else(|| panic!("no {command:?}"))
            .1
    };
    assert_eq!(key_of(Command::TogglePlayback), None);
    assert_eq!(key_of(Command::Next), Some(Shortcut::command(Key::Right)));
    assert_eq!(
        key_of(Command::Previous),
        Some(Shortcut::command(Key::Left))
    );
    assert_eq!(
        key_of(Command::SeekForward),
        Some(Shortcut::command(Key::Right).shift())
    );
    assert_eq!(
        key_of(Command::SeekBackward),
        Some(Shortcut::command(Key::Left).shift())
    );
    assert_eq!(key_of(Command::VolumeUp), Some(Shortcut::command(Key::Up)));
    assert_eq!(
        key_of(Command::VolumeDown),
        Some(Shortcut::command(Key::Down))
    );
    assert_eq!(
        key_of(Command::ToggleMute),
        Some(Shortcut::command(Key::Down).option())
    );
    assert_eq!(
        key_of(Command::ToggleShuffle),
        Some(Shortcut::command('s').option())
    );
    assert_eq!(
        key_of(Command::ToggleQueue),
        Some(Shortcut::command('u').option())
    );
    assert_eq!(key_of(Command::SaveQueue), None);
}

#[test]
fn the_menus_are_hiphile_library_view_and_controls() {
    let titles: Vec<_> = menus().into_iter().map(|menu| menu.title).collect();
    assert_eq!(titles, ["hiphile", "Library", "View", "Controls"]);
}

#[test]
fn library_refreshes_and_makes_a_playlist() {
    let library = menus().remove(1);
    assert_eq!(
        library.items[..3],
        [
            Item::command("Refresh Library", Command::Refresh, Shortcut::command('r')),
            Item::Separator,
            Item::command(
                "New Playlist…",
                Command::NewPlaylist(Arc::from([])),
                Shortcut::command('n'),
            ),
        ]
    );
}

#[test]
fn view_goes_to_each_destination_by_its_number() {
    let view = menus().remove(2);
    let destinations: Vec<_> = view
        .items
        .into_iter()
        .filter(|item| {
            matches!(
                item,
                Item::Command {
                    command: Command::Show(_),
                    ..
                }
            )
        })
        .collect();
    let expected: Vec<_> = Destination::ALL
        .iter()
        .zip('1'..)
        .map(|(destination, key)| {
            Item::command(
                destination.label(),
                Command::Show(location_of(*destination)),
                Shortcut::command(key),
            )
        })
        .collect();
    assert_eq!(destinations, expected);
}

/// The app's window with a menu bar the test can choose from.
fn start_with_menu_bar(hiphile: Hiphile) -> (Ui, MenuBar) {
    let made = Cell::new(None);
    let ui = start(|| {
        let menu_bar = MenuBar::new();
        made.set(Some(menu_bar));
        app(
            hiphile,
            crate::System::fixed(false),
            menu_bar,
            crate::Reach::Window,
        )
    });
    (ui, made.get().expect("the root to have run"))
}

#[test]
fn the_menu_bar_takes_the_window_to_a_destination_and_back() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    let songs = Command::Show(Location::Songs);
    assert!(!menu_bar.state_of(&songs).is_checked());
    assert!(!menu_bar.state_of(&Command::GoBack).is_enabled());

    menu_bar.choose(songs.clone());
    ui.update();
    assert_eq!(find(Role::Button, "Songs").unwrap().selected, Some(true));
    assert!(menu_bar.state_of(&songs).is_checked());
    assert!(menu_bar.state_of(&Command::GoBack).is_enabled());

    menu_bar.choose(Command::GoBack);
    ui.update();
    assert_eq!(find(Role::Button, "Home").unwrap().selected, Some(true));
}

#[test]
fn while_the_settings_are_open_the_menu_bar_goes_nowhere() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    press(&mut ui, Role::Button, "Settings");
    let songs = Command::Show(Location::Songs);
    assert!(!menu_bar.state_of(&songs).is_enabled());

    menu_bar.choose(songs);
    ui.update();
    assert_eq!(find(Role::Button, "Home").unwrap().selected, Some(true));
}

#[test]
fn signed_out_the_menu_bar_goes_nowhere() {
    let (_core, hiphile) = core("");
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);

    menu_bar.choose(Command::OpenSettings);
    ui.update();
    assert!(!menu_bar.state_of(&Command::OpenSettings).is_enabled());
    assert!(nodes(Role::Dialog).is_empty());
}

#[test]
fn once_signed_out_the_menu_bar_goes_nowhere_again() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    assert!(menu_bar.state_of(&Command::OpenSettings).is_enabled());

    press(&mut ui, Role::Button, "Servers");
    press(&mut ui, Role::MenuItem, "Sign Out");
    wait_until(&mut ui, || find(Role::Navigation, "Destinations").is_none());

    assert!(!menu_bar.state_of(&Command::OpenSettings).is_enabled());
    menu_bar.choose(Command::OpenSettings);
    ui.update();
    assert!(nodes(Role::Dialog).is_empty());
}

#[test]
fn while_the_search_is_open_the_menu_bar_goes_nowhere() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    assert!(menu_bar.state_of(&Command::OpenSearch).is_enabled());

    menu_bar.choose(Command::OpenSearch);
    ui.update();
    assert!(!menu_bar.state_of(&Command::OpenSearch).is_enabled());
    assert!(
        !menu_bar
            .state_of(&Command::Show(Location::Songs))
            .is_enabled()
    );

    key(&mut ui, NamedKey::Escape);
    frames(&mut ui, 1000);
    assert!(menu_bar.state_of(&Command::OpenSearch).is_enabled());
}

fn songs(ids: &[&str]) -> Vec<Song> {
    ids.iter()
        .map(|id| serde_json::from_value(json!({ "id": id, "title": id })).unwrap())
        .collect()
}

#[test]
fn the_menu_bar_opens_the_queue_beside_the_screens() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    assert!(find(Role::ListBox, "Queue").is_none());
    assert!(!menu_bar.state_of(&Command::ToggleQueue).is_checked());

    menu_bar.choose(Command::ToggleQueue);
    ui.update();
    assert!(find(Role::ListBox, "Queue").is_some());
    assert!(menu_bar.state_of(&Command::ToggleQueue).is_checked());
    assert_eq!(
        find(Role::ToggleButton, "Queue").unwrap().checked,
        Some(Checked::True)
    );
}

#[test]
fn in_the_narrowest_window_the_queue_cannot_open() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let made = Cell::new(None);
    let mut ui = start_at(
        || {
            let menu_bar = MenuBar::new();
            made.set(Some(menu_bar));
            app(
                hiphile,
                crate::System::fixed(false),
                menu_bar,
                crate::Reach::Window,
            )
        },
        (900, 560),
    );
    let menu_bar = made.get().unwrap();
    assert!(!menu_bar.state_of(&Command::ToggleQueue).is_enabled());

    menu_bar.choose(Command::ToggleQueue);
    ui.update();
    assert!(find(Role::ListBox, "Queue").is_none());
    assert!(find(Role::ToggleButton, "Queue").unwrap().disabled);
}

#[test]
fn the_menu_bar_follows_the_player() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let player = hiphile.workspace().unwrap().player().clone();
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    assert!(!menu_bar.state_of(&Command::SaveQueue).is_enabled());
    assert!(!menu_bar.state_of(&Command::Next).is_enabled());

    player.fill_queue(&songs(&["a", "b"]), 0);
    menu_bar.choose(Command::ToggleShuffle);
    menu_bar.choose(Command::SetRepeat(RepeatMode::All));
    wait_until(&mut ui, || {
        menu_bar.state_of(&Command::ToggleShuffle).is_checked()
    });

    let shown = player.snapshot();
    assert!(shown.is_shuffled());
    assert_eq!(shown.repeat(), RepeatMode::All);
    assert!(
        menu_bar
            .state_of(&Command::SetRepeat(RepeatMode::All))
            .is_checked()
    );
    assert!(menu_bar.state_of(&Command::SaveQueue).is_enabled());
    assert!(menu_bar.state_of(&Command::Next).is_enabled());
}

#[test]
fn while_a_playlist_dialog_is_open_the_menu_bar_goes_nowhere() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    let songs = Command::Show(Location::Songs);

    menu_bar.choose(Command::NewPlaylist(Arc::from([])));
    frames(&mut ui, 400);
    assert!(find(Role::Heading, "New Playlist").is_some());
    assert!(!menu_bar.state_of(&songs).is_enabled());

    menu_bar.choose(songs);
    ui.update();
    assert_eq!(find(Role::Button, "Home").unwrap().selected, Some(true));
}

#[test]
fn saving_the_queue_asks_for_the_new_playlists_name() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let player = hiphile.workspace().unwrap().player().clone();
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    player.fill_queue(&songs(&["a", "b"]), 0);
    wait_until(&mut ui, || {
        menu_bar.state_of(&Command::SaveQueue).is_enabled()
    });

    menu_bar.choose(Command::SaveQueue);
    frames(&mut ui, 400);
    assert!(find(Role::Heading, "New Playlist").is_some());
}

/// The window, signed in, on a song whose lyrics are not found.
fn on_a_song() -> (next_ui::testing::TestCore, Ui, MenuBar, playback::Player) {
    let (core, hiphile) = signed_in_core(NOWHERE);
    let player = hiphile.workspace().unwrap().player().clone();
    player.fill_queue(&songs(&["a", "b"]), 0);
    let (mut ui, menu_bar) = start_with_menu_bar(hiphile);
    wait_until(&mut ui, || {
        menu_bar.state_of(&Command::ToggleLyrics).is_enabled()
    });
    (core, ui, menu_bar, player)
}

#[test]
fn the_menu_bar_opens_the_lyrics_over_the_window_and_back_closes_them() {
    let (_core, mut ui, menu_bar, _player) = on_a_song();
    menu_bar.choose(Command::Show(Location::Songs));
    menu_bar.choose(Command::ToggleQueue);
    ui.update();

    menu_bar.choose(Command::ToggleLyrics);
    ui.update();
    assert!(find(Role::Dialog, "Lyrics").is_some());
    assert!(menu_bar.state_of(&Command::ToggleLyrics).is_checked());
    assert!(!menu_bar.state_of(&Command::ToggleQueue).is_checked());
    frames(&mut ui, 1500);
    assert!(find(Role::Button, "Albums").unwrap().disabled);

    menu_bar.choose(Command::GoBack);
    ui.update();
    assert!(!menu_bar.state_of(&Command::ToggleLyrics).is_checked());
    assert!(menu_bar.state_of(&Command::ToggleQueue).is_checked());
    assert!(!find(Role::Button, "Albums").unwrap().disabled);
    assert_eq!(find(Role::Button, "Songs").unwrap().selected, Some(true));
    frames(&mut ui, 1500);
    assert!(find(Role::Dialog, "Lyrics").is_none());
}

#[test]
fn going_somewhere_closes_the_lyrics() {
    let (_core, mut ui, menu_bar, _player) = on_a_song();
    menu_bar.choose(Command::ToggleLyrics);
    ui.update();

    menu_bar.choose(Command::Show(Location::Albums(None)));
    ui.update();
    assert!(!menu_bar.state_of(&Command::ToggleLyrics).is_checked());
    assert_eq!(find(Role::Button, "Albums").unwrap().selected, Some(true));
}

#[test]
fn the_lyrics_close_when_there_is_no_song() {
    let (_core, mut ui, menu_bar, player) = on_a_song();
    menu_bar.choose(Command::ToggleLyrics);
    ui.update();

    player.fill_queue(&[], 0);
    wait_until(&mut ui, || {
        !menu_bar.state_of(&Command::ToggleLyrics).is_checked()
    });
    assert!(!menu_bar.state_of(&Command::ToggleLyrics).is_enabled());
}
