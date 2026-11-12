use std::cell::Cell;
use std::rc::Rc;

use anti_webui_md3e::components::icon_button;

use super::*;
use crate::testing::{find, focus, has_text, key, press, start, typed};

#[test]
fn the_header_says_how_many_once_that_is_known() {
    let count = signal(None::<String>);
    let mut ui = start(move || list_header(ListHeader::new(move || count.get())));

    assert!(!has_text("12 albums"));
    count.set(Some("12 albums".to_owned()));
    ui.update();
    assert!(has_text("12 albums"));
}

#[test]
fn the_header_shows_an_update_only_while_there_is_one() {
    let updating = signal(false);
    let mut ui = start(move || {
        list_header(ListHeader::new(|| None).updating("Updating Albums", move || updating.get()))
    });

    assert!(find(Role::ProgressBar, "Updating Albums").is_none());
    updating.set(true);
    ui.update();
    assert!(find(Role::ProgressBar, "Updating Albums").is_some());
}

#[test]
fn the_headers_buttons_are_at_its_end() {
    let _ui = start(|| {
        div().w(320.0_f32).child(
            list_header(
                ListHeader::new(|| Some("3 playlists".to_owned()))
                    .action(icon_button(icons::ADD).access(|a| a.label("New Playlist"))),
            )
            .w_full(),
        )
    });

    let button = find(Role::Button, "New Playlist").expect("the button");
    assert_eq!(
        button.bounds.x + button.bounds.width,
        320.0 - (INSET - BUTTON_PADDING)
    );
}

fn filter() -> (Signal<String>, impl FnOnce() -> Div) {
    let value = signal(String::new());
    (value, move || list_filter(value, "Filter Albums"))
}

#[test]
fn what_is_typed_in_the_filter_is_its_value() {
    let (value, view) = filter();
    let mut ui = start(view);

    focus(&mut ui, Role::TextInput, "Filter Albums");
    typed(&mut ui, "blue");

    assert_eq!(value.get_untracked(), "blue");
}

#[test]
fn the_filter_has_a_clear_button_only_with_something_in_it() {
    let (value, view) = filter();
    let mut ui = start(view);

    assert!(find(Role::Button, "Clear Filter").is_none());
    value.set("blue".to_owned());
    ui.update();
    press(&mut ui, Role::Button, "Clear Filter");

    assert_eq!(value.get_untracked(), "");
    assert!(find(Role::Button, "Clear Filter").is_none());
}

#[test]
fn escape_empties_the_filter() {
    let (value, view) = filter();
    let mut ui = start(view);
    focus(&mut ui, Role::TextInput, "Filter Albums");
    typed(&mut ui, "blue");

    key(&mut ui, NamedKey::Escape);

    assert_eq!(value.get_untracked(), "");
}

#[test]
fn escape_in_an_empty_filter_goes_on_to_what_holds_it() {
    let reached = Rc::new(Cell::new(false));
    let told = reached.clone();
    let (_, view) = filter();
    let mut ui = start(move || {
        div()
            .on_key_down(move |e| {
                if matches!(e.key, Key::Named(NamedKey::Escape)) {
                    told.set(true);
                }
            })
            .child(view())
    });
    focus(&mut ui, Role::TextInput, "Filter Albums");

    key(&mut ui, NamedKey::Escape);

    assert!(reached.get());
}

#[test]
fn a_failed_refresh_says_why_and_tries_again() {
    let tries = Rc::new(Cell::new(0));
    let counted = tries.clone();
    let mut ui = start(move || {
        refresh_failure(
            "Albums didn't update",
            "The server didn't answer.",
            move || counted.set(counted.get() + 1),
        )
    });

    assert!(has_text("Albums didn't update"));
    assert!(has_text("The server didn't answer."));
    press(&mut ui, Role::Button, "Retry");
    assert_eq!(tries.get(), 1);
}
