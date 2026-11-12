use std::rc::Rc;

use anti_webui::prelude::Role;
use next_ui::testing::{core, find, focus, has_text, press, start, typed, wait_until};

use super::*;
use crate::testing::{replace, value};

const KEY: &str = "ImgBB API key";

#[test]
fn the_key_is_saved_as_it_is_typed_and_stays_when_shown() {
    let (_core, hiphile) = core("");
    let discord = hiphile.settings().discord().clone();
    let mut ui = start({
        let discord = discord.clone();
        move || discord_page(&discord)
    });
    assert!(has_text(
        "Shows covers on your Discord status by uploading them to ImgBB, where anyone with the \
         link can see them for 180 days. Get a key at api.imgbb.com."
    ));

    focus(&mut ui, Role::TextInput, KEY);
    typed(&mut ui, "abc");
    wait_until(&mut ui, || discord.snapshot().imgbb_key() == Some("abc"));

    press(&mut ui, Role::ToggleButton, "Show ImgBB API Key");
    assert_eq!(value(Role::TextInput, KEY), "abc");
}

#[test]
fn the_key_waits_while_the_status_is_off() {
    let (_core, hiphile) = core("");
    let discord = hiphile.settings().discord().clone();
    let mut ui = start({
        let discord = discord.clone();
        move || discord_page(&discord)
    });
    assert!(discord.snapshot().is_presence_on());
    assert!(!find(Role::TextInput, KEY).unwrap().disabled);

    press(&mut ui, Role::Switch, "Show current song as status");
    assert!(!discord.snapshot().is_presence_on());
    wait_until(&mut ui, || find(Role::TextInput, KEY).unwrap().disabled);
}

#[test]
fn shown_again_before_the_key_settles_the_field_has_what_was_typed() {
    let (_core, hiphile) = core("imgbb_api_key = \"saved\"\n");
    let discord = hiphile.settings().discord().clone();
    let made = Rc::new(Cell::new(None));
    let mut ui = start({
        let made = made.clone();
        move || {
            let open = signal(true);
            made.set(Some(open));
            div().child(show(move || open.get(), move || discord_page(&discord)))
        }
    });
    let open = made.get().expect("the view made");
    replace(&mut ui, Role::TextInput, KEY, "typed");

    open.set(false);
    ui.update();
    open.set(true);
    ui.update();

    press(&mut ui, Role::ToggleButton, "Show ImgBB API Key");
    assert_eq!(value(Role::TextInput, KEY), "typed");
}

#[test]
fn a_key_from_the_file_is_in_the_field() {
    let (_core, hiphile) = core("imgbb_api_key = \"saved\"\n");
    let discord = hiphile.settings().discord().clone();
    assert_eq!(discord.snapshot().imgbb_key(), Some("saved"));
    let mut ui = start(move || discord_page(&discord));

    press(&mut ui, Role::ToggleButton, "Show ImgBB API Key");
    assert_eq!(value(Role::TextInput, KEY), "saved");
}
