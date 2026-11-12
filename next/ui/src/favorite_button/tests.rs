use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::core::Ui;
use anti_webui::core::access::Checked;
use hiphile_core::test_support::FavoritesServer;
use serde_json::json;
use subsonic::Song;

use super::*;
use crate::row_list::{RowList, Selection, row_list};
use crate::testing::{
    NOWHERE, TestCore, click, find, recording_commands, screen_context, signed_in_core, start,
    wait_until,
};

fn song() -> Song {
    serde_json::from_value(json!({ "id": "s1", "title": "Song" })).unwrap()
}

struct Shown {
    ui: Ui,
    sent: Rc<RefCell<Vec<Command>>>,
    selection: Selection<String>,
    _core: TestCore,
}

/// The button of the song in a row of a list, on the workspace on `server`.
fn in_a_row(server: &str) -> Shown {
    let (core, hiphile) = signed_in_core(server);
    let (commands, sent) = recording_commands();
    let selection = Selection::new();
    let ui = start(move || {
        let cx = screen_context(&hiphile, commands);
        let songs: Arc<[Song]> = Arc::from([song()]);
        row_list(
            RowList::new(
                "Songs",
                move || songs.clone(),
                |song: &Song| song.id().to_string(),
                move |song, _| {
                    div().flex().flex_row().child(favorite_button(
                        Favorite::Song(song.clone()),
                        &cx.commands,
                        cx.known,
                    ))
                },
            )
            .selection(selection),
        )
    });
    Shown {
        ui,
        sent,
        selection,
        _core: core,
    }
}

fn button() -> anti_webui::core::access::AccessNode {
    find(Role::ToggleButton, "Favorite").expect("the button")
}

fn wait_until_known(shown: &mut Shown) {
    wait_until(&mut shown.ui, || !button().disabled);
}

#[test]
fn it_is_on_for_a_favorite() {
    let server = FavoritesServer::new(&["s1"]);
    let mut shown = in_a_row(server.url());

    wait_until_known(&mut shown);

    assert_eq!(button().checked, Some(Checked::True));
}

#[test]
fn it_is_off_for_what_is_not_one() {
    let server = FavoritesServer::new(&[]);
    let mut shown = in_a_row(server.url());

    wait_until_known(&mut shown);

    assert_eq!(button().checked, Some(Checked::False));
}

#[test]
fn pressing_it_toggles_the_favorite_and_leaves_the_row_alone() {
    let server = FavoritesServer::new(&[]);
    let mut shown = in_a_row(server.url());
    wait_until_known(&mut shown);

    let bounds = button().bounds;
    click(
        &mut shown.ui,
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );

    assert_eq!(
        *shown.sent.borrow(),
        [Command::ToggleFavorite(Favorite::Song(song()))]
    );
    assert!(!shown.selection.contains(&"s1".to_owned()));
}

#[test]
fn it_waits_until_the_favorites_are_known() {
    let _shown = in_a_row(NOWHERE);

    assert!(button().disabled);
}

#[test]
fn a_menu_names_the_way_it_goes() {
    assert_eq!(favorite_label(Some(true)), "Remove from Favorites");
    assert_eq!(favorite_label(Some(false)), "Add to Favorites");
    assert_eq!(favorite_label(None), "Add to Favorites");
}
