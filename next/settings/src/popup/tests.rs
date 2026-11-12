use anti_webui::core::Ui;
use anti_webui::core::access::{AccessNode, snapshot};
use anti_webui::core::event::NamedKey;
use anti_webui_md3e::components::button;
use next_ui::testing::{
    TestCore, WINDOW, click, core, find, focus, frames, has_text, key, nodes, press, start,
    wait_until,
};

use super::*;
use crate::testing::{replace, value};

struct Fixture {
    open: Signal<bool>,
    ui: Ui,
    _core: TestCore,
}

/// The settings open over a window with a button of its own.
fn fixture(config: &str) -> Fixture {
    let (core, hiphile) = core(config);
    let opened = std::rc::Rc::new(std::cell::Cell::new(None));
    let ui = start({
        let opened = opened.clone();
        move || {
            let open = signal(true);
            opened.set(Some(open));
            div()
                .size_full()
                .child(button("Behind").on_click(move |_| open.set(true)))
                .child(settings_popup(hiphile, open))
        }
    });
    let mut it = Fixture {
        open: opened.get().expect("the view made"),
        ui,
        _core: core,
    };
    frames(&mut it.ui, 400);
    it
}

fn selected_tab() -> String {
    nodes(Role::Tab)
        .into_iter()
        .find(|tab| tab.selected == Some(true))
        .and_then(|tab| tab.label)
        .expect("a page chosen")
}

fn focused() -> AccessNode {
    let tree = snapshot();
    let id = tree.focus.expect("the focus somewhere");
    tree.nodes
        .into_iter()
        .find(|node| node.id == id)
        .expect("the focused node")
}

#[test]
fn the_pages_are_tabs_in_their_order_and_the_one_chosen_shows() {
    let mut it = fixture("");
    let tabs: Vec<String> = nodes(Role::Tab)
        .into_iter()
        .filter_map(|tab| tab.label)
        .collect();
    assert_eq!(
        tabs,
        [
            "Audio",
            "DSP",
            "Queue",
            "Lyrics",
            "Storage",
            "Discord",
            "Appearance"
        ]
    );
    assert_eq!(selected_tab(), "Audio");
    assert!(find(Role::TabPanel, "Audio").is_some());

    press(&mut it.ui, Role::Tab, "Storage");
    assert_eq!(selected_tab(), "Storage");
    assert!(find(Role::TabPanel, "Audio").is_none());
    assert!(find(Role::TabPanel, "Storage").is_some());
    assert!(find(Role::Heading, "Storage").is_some());
}

#[test]
fn up_and_down_go_between_the_pages() {
    let mut it = fixture("");
    focus(&mut it.ui, Role::Tab, "Audio");

    key(&mut it.ui, NamedKey::ArrowDown);
    assert_eq!(selected_tab(), "DSP");
    assert_eq!(focused().label.as_deref(), Some("DSP"));

    key(&mut it.ui, NamedKey::ArrowUp);
    key(&mut it.ui, NamedKey::ArrowUp);
    assert_eq!(selected_tab(), "Appearance");
    assert!(find(Role::TabPanel, "Appearance").is_some());
}

#[test]
fn escape_and_the_scrim_close_it_and_the_page_stays_chosen() {
    let mut it = fixture("");
    press(&mut it.ui, Role::Tab, "Discord");
    key(&mut it.ui, NamedKey::Escape);
    assert!(!it.open.get_untracked());
    frames(&mut it.ui, 600);
    assert!(nodes(Role::Dialog).is_empty());

    it.open.set(true);
    frames(&mut it.ui, 400);
    assert_eq!(selected_tab(), "Discord");
    click(&mut it.ui, 10.0, 10.0);
    assert!(!it.open.get_untracked());
}

#[test]
fn the_close_button_closes_it() {
    let mut it = fixture("");
    press(&mut it.ui, Role::Button, "Close");
    assert!(!it.open.get_untracked());
}

const ATTENUATION: &str = "r8brain attenuation";

/// The settings with r8brain, `999` typed into the attenuation, beyond what it takes.
fn typed_beyond_the_attenuation() -> Fixture {
    let mut it = fixture("resampling = \"r8brain\"\n");
    replace(&mut it.ui, Role::TextInput, ATTENUATION, "999");
    assert!(has_text("Enter 49 to 218."));
    it
}

/// Closing counts what was typed as leaving the field does, at once, and opening again shows it.
fn assert_counted_once_closed(it: &mut Fixture) {
    assert!(!it.open.get_untracked());
    wait_until(&mut it.ui, || value(Role::TextInput, ATTENUATION) == "218");
    assert!(!has_text("Enter 49 to 218."));
    frames(&mut it.ui, 600);
    assert!(nodes(Role::Dialog).is_empty());

    it.open.set(true);
    frames(&mut it.ui, 400);
    assert_eq!(value(Role::TextInput, ATTENUATION), "218");
    assert!(!has_text("Enter 49 to 218."));
}

#[test]
fn escape_counts_what_was_typed() {
    let mut it = typed_beyond_the_attenuation();
    key(&mut it.ui, NamedKey::Escape);
    assert_counted_once_closed(&mut it);
}

#[test]
fn the_scrim_counts_what_was_typed() {
    let mut it = typed_beyond_the_attenuation();
    click(&mut it.ui, 10.0, 10.0);
    assert_counted_once_closed(&mut it);
}

#[test]
fn the_close_button_counts_what_was_typed() {
    let mut it = typed_beyond_the_attenuation();
    press(&mut it.ui, Role::Button, "Close");
    assert_counted_once_closed(&mut it);
}

#[test]
fn choosing_another_page_counts_what_was_typed() {
    let mut it = typed_beyond_the_attenuation();
    press(&mut it.ui, Role::Tab, "Storage");
    press(&mut it.ui, Role::Tab, "Audio");
    assert_eq!(value(Role::TextInput, ATTENUATION), "218");
    assert!(!has_text("Enter 49 to 218."));
}

#[test]
fn escape_in_a_menu_closes_only_the_menu() {
    let mut it = fixture("");
    focus(&mut it.ui, Role::ComboBox, "Sample rate");
    key(&mut it.ui, NamedKey::Enter);
    assert_eq!(
        find(Role::ComboBox, "Sample rate").unwrap().expanded,
        Some(true)
    );

    key(&mut it.ui, NamedKey::Escape);
    assert_eq!(
        find(Role::ComboBox, "Sample rate").unwrap().expanded,
        Some(false)
    );
    assert!(it.open.get_untracked());
}

#[test]
fn tab_stays_inside_and_brings_the_focus_into_view() {
    // r8brain makes the Audio page longer than the window
    let mut it = fixture("resampling = \"r8brain\"\n");
    assert!(find(Role::TextInput, "r8brain filter bank cache").is_some());
    let behind = find(Role::Button, "Behind").expect("the button behind").id;
    let (top, bottom) = (0.0, WINDOW.1 as f32);
    let mut reached = false;
    for _ in 0..120 {
        key(&mut it.ui, NamedKey::Tab);
        frames(&mut it.ui, 16);
        let node = focused();
        assert_ne!(node.id, behind);
        let (y, height) = (node.bounds.y, node.bounds.height);
        assert!(
            y >= top && y + height <= bottom,
            "{:?} {:?} is out of view at {y}",
            node.role,
            node.label
        );
        if node.label.as_deref() == Some("r8brain filter bank cache") {
            reached = true;
        }
    }
    assert!(reached, "Tab never reached the last field");
}
