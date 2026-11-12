use std::cell::Cell;

use anti_webui::core::Ui;
use anti_webui::core::event::NamedKey;
use hiphile_core::Hiphile;
use hiphile_core::navigation::Location;
use next_ui::testing::{
    NOWHERE, core, find, frames, key, nodes, press, signed_in_core, start, wait_until,
};

use super::*;
use crate::app;

fn at(destination: Destination) -> Shown {
    Shown {
        destination: Some(destination),
        ..Shown::default()
    }
}

fn every_command() -> Vec<Command> {
    Destination::ALL
        .iter()
        .map(|destination| Command::Show(location_of(*destination)))
        .chain([
            Command::GoBack,
            Command::GoForward,
            Command::OpenSearch,
            Command::OpenSettings,
        ])
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
        can_go_back: true,
        can_go_forward: true,
        covered: true,
        ..at(Destination::Home)
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
fn the_menus_are_hiphile_library_view_and_controls() {
    let titles: Vec<_> = menus().into_iter().map(|menu| menu.title).collect();
    assert_eq!(titles, ["hiphile", "Library", "View", "Controls"]);
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
        app(hiphile, signal(false), menu_bar, crate::Reach::Window)
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
