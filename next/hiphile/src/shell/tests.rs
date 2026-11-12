use anti_webui::core::event::NamedKey;
use anti_webui::prelude::Role;
use next_player::testing::{CoverServer, png};
use next_ui::testing::{
    NOWHERE, SavedLibrary, core, find, frames, has_text, key, nodes, press, signed_in_core,
    signed_in_core_with_library, start, typed, wait_until,
};
use serde_json::json;

use super::*;

fn heading(text: &str) -> bool {
    find(Role::Heading, text).is_some() || has_text(text)
}

#[test]
fn signed_out_it_asks_to_sign_in_without_the_rail() {
    let (_core, hiphile) = core("");
    let _ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));

    assert!(heading("Sign In"));
    assert!(find(Role::Navigation, "Destinations").is_none());
}

#[test]
fn signed_in_the_rail_goes_to_each_destination() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    assert!(find(Role::Navigation, "Destinations").is_some());
    assert_eq!(find(Role::Button, "Home").unwrap().selected, Some(true));

    for destination in Destination::ALL {
        press(&mut ui, Role::Button, destination.label());
        assert_eq!(
            find(Role::Button, destination.label()).unwrap().selected,
            Some(true),
            "{destination:?}"
        );
        assert!(has_text(destination.label()));
    }
}

#[test]
fn back_and_forward_retrace_the_destinations() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    assert!(find(Role::Button, "Back").unwrap().disabled);

    press(&mut ui, Role::Button, "Albums");
    press(&mut ui, Role::Button, "Songs");
    press(&mut ui, Role::Button, "Back");
    assert_eq!(find(Role::Button, "Albums").unwrap().selected, Some(true));

    press(&mut ui, Role::Button, "Forward");
    assert_eq!(find(Role::Button, "Songs").unwrap().selected, Some(true));
    assert!(find(Role::Button, "Forward").unwrap().disabled);
}

#[test]
fn settings_open_over_the_window_and_escape_closes_them() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));

    press(&mut ui, Role::Button, "Settings");
    assert_eq!(nodes(Role::Dialog).len(), 1);

    key(&mut ui, NamedKey::Escape);
    next_ui::testing::frames(&mut ui, 600);
    assert!(nodes(Role::Dialog).is_empty());
}

#[test]
fn the_theme_chosen_in_the_settings_shows_behind_them_at_once() {
    use anti_webui_md3e::color_scheme;
    use next_ui::theme::{DEFAULT_SEED, scheme};

    let (_core, hiphile) = signed_in_core(NOWHERE);
    let appearance = hiphile.settings().appearance().clone();
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    let surface = |dark| scheme(DEFAULT_SEED, dark).surface;
    assert_eq!(color_scheme().surface, surface(false));

    press(&mut ui, Role::Button, "Settings");
    press(&mut ui, Role::Tab, "Appearance");
    press(&mut ui, Role::Radio, "Dark");
    wait_until(&mut ui, || appearance.snapshot().appearance().is_some());
    next_ui::testing::frames(&mut ui, 1000);
    assert_eq!(color_scheme().surface, surface(true));
    assert_eq!(nodes(Role::Dialog).len(), 1);

    press(&mut ui, Role::Radio, "System");
    wait_until(&mut ui, || appearance.snapshot().appearance().is_none());
    next_ui::testing::frames(&mut ui, 1000);
    assert_eq!(color_scheme().surface, surface(false));
}

#[test]
fn signing_out_from_the_servers_goes_back_to_signing_in() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start({
        let hiphile = hiphile.clone();
        move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window)
    });

    press(&mut ui, Role::Button, "Servers");
    press(&mut ui, Role::MenuItem, "Sign Out");
    wait_until(&mut ui, || heading("Sign In"));

    assert!(hiphile.snapshot().workspace().is_none());
    assert!(find(Role::Navigation, "Destinations").is_none());
}

#[test]
fn in_the_smallest_window_the_rail_fits_and_the_screens_actions_keep_clear_of_the_settings() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = next_ui::testing::start_at(
        move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window),
        crate::SMALLEST_WINDOW,
    );
    press(&mut ui, Role::Button, "Songs");
    let sort = || {
        nodes(Role::Button).into_iter().find(|node| {
            node.label
                .as_deref()
                .is_some_and(|label| label.starts_with("Sort"))
        })
    };
    wait_until(&mut ui, || sort().is_some());
    let (width, height) = crate::SMALLEST_WINDOW;

    let favorites = find(Role::Button, "Favorites").expect("the last destination");
    assert!(favorites.bounds.bottom() <= height as f32);
    let settings = find(Role::Button, "Settings").expect("the settings");
    assert!(sort().expect("the sort").bounds.right() <= settings.bounds.x);
    let servers = find(Role::Button, "Servers").expect("the servers");
    assert!(settings.bounds.right() <= servers.bounds.x);
    assert!(servers.bounds.right() <= width as f32);
}

#[test]
fn the_rail_closes_in_a_narrow_window() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let _ui = next_ui::testing::start_at(
        move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window),
        (900, 600),
    );

    assert!(find(Role::Button, "Open Navigation").is_some());
}

#[test]
fn adding_a_server_opens_signing_in_over_the_workspace() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));

    press(&mut ui, Role::Button, "Servers");
    press(&mut ui, Role::MenuItem, "Add Server…");

    assert!(find(Role::Heading, "Add Server").is_some());
    assert!(find(Role::Navigation, "Destinations").is_some());
}

#[test]
fn signed_in_the_player_bar_is_at_the_bottom() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let _ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));

    let bar = find(Role::Toolbar, "Player").expect("the player bar");
    assert_eq!(bar.bounds.y + bar.bounds.height, crate::WINDOW.1 as f32);
}

#[test]
fn the_colors_follow_the_cover_of_what_plays() {
    let server = CoverServer::new(png(16, |_, _| [200, 40, 40, 255]));
    let (_core, hiphile) = signed_in_core(server.url());
    let player = hiphile.workspace().unwrap().player().clone();
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    let before = color_scheme().primary;

    let song = json!({ "id": "a", "title": "a", "coverArt": "al-a" });
    player.fill_queue(&[serde_json::from_value(song).unwrap()], 0);
    // The scheme cross-fades, which takes frames
    for _ in 0..500 {
        if color_scheme().primary != before {
            break;
        }
        frames(&mut ui, 16);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_ne!(color_scheme().primary, before);
}

/// The search's field is the one text input without a name; the screens name theirs.
fn search_is_open() -> bool {
    nodes(Role::TextInput)
        .iter()
        .any(|node| node.label.is_none())
}

fn focus_role() -> Option<Role> {
    let tree = anti_webui::core::access::snapshot();
    tree.get(tree.focus?).map(|node| node.role)
}

#[test]
fn the_rail_opens_the_search_over_the_screens_with_the_focus_in_it() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    press(&mut ui, Role::Button, "Albums");

    press(&mut ui, Role::Button, "Search");
    assert!(search_is_open());
    assert_eq!(focus_role(), Some(Role::TextInput));

    key(&mut ui, NamedKey::Escape);
    frames(&mut ui, 1000);
    assert!(!search_is_open());
    assert_eq!(find(Role::Button, "Albums").unwrap().selected, Some(true));
}

#[test]
fn choosing_an_album_in_the_search_goes_to_it() {
    let library = SavedLibrary {
        albums: serde_json::from_value(json!([
            { "id": "blue", "name": "Kind of Blue", "artist": "Miles Davis", "artistId": "miles" },
        ]))
        .unwrap(),
        ..SavedLibrary::default()
    };
    let (_core, hiphile, _catalog) = signed_in_core_with_library(NOWHERE, &library);
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    press(&mut ui, Role::Button, "Search");

    typed(&mut ui, "kind of blue");
    wait_until(&mut ui, || find(Role::ListOption, "Kind of Blue").is_some());
    key(&mut ui, NamedKey::Enter);

    assert_eq!(find(Role::Button, "Albums").unwrap().selected, Some(true));
    frames(&mut ui, 1000);
    assert!(!search_is_open());
}

#[test]
fn the_queue_opens_from_the_bar_beside_the_screens() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));

    press(&mut ui, Role::ToggleButton, "Queue");

    let title = find(Role::Heading, "Home").unwrap().bounds;
    let queue_title = find(Role::Heading, "Queue").unwrap().bounds;
    assert_eq!(queue_title.y, title.y);
    let handle = find(Role::Separator, "Queue Width").unwrap().bounds;
    let bar = find(Role::Toolbar, "Player").unwrap().bounds;
    assert_eq!(handle.y + handle.height, bar.y);
    let list = find(Role::ListBox, "Queue").unwrap().bounds;
    assert_eq!(list.x + list.width + GAP, crate::WINDOW.0 as f32);

    press(&mut ui, Role::ToggleButton, "Queue");
    assert!(find(Role::ListBox, "Queue").is_none());
}

fn song_rows() -> Vec<anti_webui::core::access::AccessNode> {
    let mut rows = nodes(Role::ListOption);
    rows.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));
    rows
}

/// The title of the song selected, without what its row says of it playing.
fn selected_song() -> Option<String> {
    let label = song_rows()
        .into_iter()
        .find(|row| row.selected == Some(true))?
        .label?;
    Some(label.split(", ").next().unwrap_or_default().to_owned())
}

#[test]
fn once_a_rows_menu_has_closed_the_keys_go_to_the_list_again() {
    let song = |id: &str, title: &str| json!({ "id": id, "title": title, "duration": 100 });
    let server = hiphile_core::test_support::SongsServer::new(vec![
        song("a", "Airegin"),
        song("b", "Blues"),
        song("c", "Cherokee"),
    ]);
    let (_core, hiphile) = signed_in_core(server.url());
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    press(&mut ui, Role::Button, "Songs");
    wait_until(&mut ui, || {
        song_rows()
            .iter()
            .filter_map(|row| row.label.as_deref())
            .eq(["Airegin", "Blues", "Cherokee"])
    });

    let blues = song_rows().remove(1).bounds;
    next_ui::testing::right_click(
        &mut ui,
        blues.x + blues.width / 2.0,
        blues.y + blues.height / 2.0,
    );
    let item = nodes(Role::MenuItem)
        .into_iter()
        .filter_map(|item| item.label)
        .find(|label| label.starts_with("Add to Queue"))
        .expect("the row's menu");
    press(&mut ui, Role::MenuItem, &item);
    frames(&mut ui, 400);
    assert_eq!(selected_song().as_deref(), Some("Blues"));

    key(&mut ui, NamedKey::ArrowDown);
    wait_until(&mut ui, || selected_song().as_deref() == Some("Cherokee"));
}

fn two_albums() -> SavedLibrary {
    SavedLibrary {
        albums: serde_json::from_value(json!([
            { "id": "a", "name": "Alpha", "artist": "Artist", "artistId": "ar" },
            { "id": "b", "name": "Beta", "artist": "Artist", "artistId": "ar" },
        ]))
        .unwrap(),
        ..SavedLibrary::default()
    }
}

fn list_width() -> f64 {
    let handle = find(Role::Separator, "List Width").expect("the handle");
    handle.value.expect("a width").now
}

fn is_selected(album: &str) -> bool {
    find(Role::ListOption, album).is_some_and(|row| row.selected == Some(true))
}

#[test]
fn the_rail_comes_back_to_the_album_left() {
    let (_core, hiphile, _catalog) = signed_in_core_with_library(NOWHERE, &two_albums());
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    press(&mut ui, Role::Button, "Albums");
    wait_until(&mut ui, || is_selected("Alpha"));
    press(&mut ui, Role::ListOption, "Beta");
    press(&mut ui, Role::Button, "Home");

    press(&mut ui, Role::Button, "Albums");

    wait_until(&mut ui, || find(Role::ListOption, "Beta").is_some());
    assert!(is_selected("Beta"));
    press(&mut ui, Role::Button, "Back");
    assert_eq!(find(Role::Button, "Home").unwrap().selected, Some(true));
}

#[test]
fn a_list_keeps_the_width_given_it_and_saves_it() {
    let (_core, hiphile, _catalog) = signed_in_core_with_library(NOWHERE, &two_albums());
    let widths = hiphile.settings().pane_widths().clone();
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));
    press(&mut ui, Role::Button, "Albums");
    wait_until(&mut ui, || find(Role::ListOption, "Alpha").is_some());
    let first = list_width();
    next_ui::testing::focus(&mut ui, Role::Separator, "List Width");

    key(&mut ui, NamedKey::ArrowRight);
    let widened = list_width();
    press(&mut ui, Role::Button, "Artists");
    let artists = list_width();
    press(&mut ui, Role::Button, "Albums");

    assert!(widened > first, "{widened} from {first}");
    assert_eq!(list_width(), widened);
    assert_eq!(artists, first);
    let saved = widths.snapshot();
    assert_eq!(saved.width(Pane::AlbumList), Some(widened as f32));
    assert_eq!(saved.width(Pane::ArtistList), None);
}

#[test]
fn a_list_starts_as_wide_as_it_was_left() {
    let (_core, hiphile, _catalog) = signed_in_core_with_library(NOWHERE, &two_albums());
    hiphile
        .settings()
        .pane_widths()
        .set_width(Pane::AlbumList, 400.0);
    let mut ui = start(move || app(hiphile, System::fixed(false), MenuBar::new(), Reach::Window));

    press(&mut ui, Role::Button, "Albums");

    assert_eq!(list_width(), 400.0);
}
