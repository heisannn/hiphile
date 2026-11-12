use anti_webui::core::event::NamedKey;
use anti_webui::prelude::Role;
use next_player::testing::{CoverServer, png};
use next_ui::testing::{
    NOWHERE, core, find, frames, has_text, key, nodes, press, signed_in_core, start, wait_until,
};
use serde_json::json;

use super::*;

fn heading(text: &str) -> bool {
    find(Role::Heading, text).is_some() || has_text(text)
}

#[test]
fn signed_out_it_asks_to_sign_in_without_the_rail() {
    let (_core, hiphile) = core("");
    let _ui = start(move || app(hiphile, signal(false), MenuBar::new(), Reach::Window));

    assert!(heading("Sign In"));
    assert!(find(Role::Navigation, "Destinations").is_none());
}

#[test]
fn signed_in_the_rail_goes_to_each_destination() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start(move || app(hiphile, signal(false), MenuBar::new(), Reach::Window));
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
    let mut ui = start(move || app(hiphile, signal(false), MenuBar::new(), Reach::Window));
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
    let mut ui = start(move || app(hiphile, signal(false), MenuBar::new(), Reach::Window));

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
    let mut ui = start(move || app(hiphile, signal(false), MenuBar::new(), Reach::Window));
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
        move || app(hiphile, signal(false), MenuBar::new(), Reach::Window)
    });

    press(&mut ui, Role::Button, "Servers");
    press(&mut ui, Role::MenuItem, "Sign Out");
    wait_until(&mut ui, || heading("Sign In"));

    assert!(hiphile.snapshot().workspace().is_none());
    assert!(find(Role::Navigation, "Destinations").is_none());
}

#[test]
fn the_rail_closes_in_a_narrow_window() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let _ui = next_ui::testing::start_at(
        move || app(hiphile, signal(false), MenuBar::new(), Reach::Window),
        (900, 600),
    );

    assert!(find(Role::Button, "Open Navigation").is_some());
}

#[test]
fn adding_a_server_opens_signing_in_over_the_workspace() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let mut ui = start(move || app(hiphile, signal(false), MenuBar::new(), Reach::Window));

    press(&mut ui, Role::Button, "Servers");
    press(&mut ui, Role::MenuItem, "Add Server…");

    assert!(find(Role::Heading, "Add Server").is_some());
    assert!(find(Role::Navigation, "Destinations").is_some());
}

#[test]
fn signed_in_the_player_bar_is_at_the_bottom() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let _ui = start(move || app(hiphile, signal(false), MenuBar::new(), Reach::Window));

    let bar = find(Role::Toolbar, "Player").expect("the player bar");
    assert_eq!(bar.bounds.y + bar.bounds.height, crate::WINDOW.1 as f32);
}

#[test]
fn the_colors_follow_the_cover_of_what_plays() {
    let server = CoverServer::new(png(16, |_, _| [200, 40, 40, 255]));
    let (_core, hiphile) = signed_in_core(server.url());
    let player = hiphile.workspace().unwrap().player().clone();
    let mut ui = start(move || app(hiphile, signal(false), MenuBar::new(), Reach::Window));
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
