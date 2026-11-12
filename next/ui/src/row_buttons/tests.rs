use std::cell::RefCell;
use std::sync::Arc;

use anti_webui::core::Ui;
use anti_webui_md3e::components::menu_item;
use hiphile_core::test_support::FavoritesServer;
use serde_json::json;
use subsonic::Song;

use super::*;
use crate::row_list::{RowList, Selection, row_list};
use crate::row_text::row_text;
use crate::testing::{
    TestCore, click, find, recording_commands, screen_context, signed_in_core, start, wait_until,
};
use crate::{Command, Overlays};

fn song() -> Song {
    serde_json::from_value(json!({ "id": "s1", "title": "Song" })).unwrap()
}

struct Shown {
    ui: Ui,
    sent: Rc<RefCell<Vec<Command>>>,
    selection: Selection<String>,
    menu: ContextMenu,
    _core: TestCore,
}

/// A row of the song with its buttons, in a list, on the favorites `server` has.
fn a_row(server: &FavoritesServer) -> Shown {
    let (core, hiphile) = signed_in_core(server.url());
    let (commands, sent) = recording_commands();
    let (selection, menu) = (Selection::new(), ContextMenu::new(Overlays::new()));
    let ui = start(move || {
        let cx = screen_context(&hiphile, commands);
        let songs: Arc<[Song]> = Arc::from([song()]);
        let rows = RowList::new(
            "Songs",
            move || songs.clone(),
            |song: &Song| song.id().to_string(),
            move |song, row| {
                let buttons = RowButtons::new(
                    Favorite::Song(song.clone()),
                    &cx.commands,
                    cx.known,
                    menu,
                    || vec![vec![menu_item("Play Next")]],
                )
                .selected_when(move || row.is_selected());
                row_with_buttons(row_text(song.title().to_owned(), None), Some(buttons))
            },
        )
        .selection(selection);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(row_list(rows).flex_1())
            .child(menu.view())
    });
    let mut shown = Shown {
        ui,
        sent,
        selection,
        menu,
        _core: core,
    };
    wait_until(&mut shown.ui, || {
        find(Role::ToggleButton, "Favorite").is_some_and(|button| !button.disabled)
    });
    shown
}

fn press_button(shown: &mut Shown, role: Role, label: &str) {
    let bounds = find(role, label).expect("the button").bounds;
    click(
        &mut shown.ui,
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );
}

#[test]
fn the_heart_toggles_the_favorite_and_leaves_the_row_alone() {
    let server = FavoritesServer::new(&[]);
    let mut shown = a_row(&server);

    press_button(&mut shown, Role::ToggleButton, "Favorite");

    assert_eq!(
        *shown.sent.borrow(),
        [Command::ToggleFavorite(Favorite::Song(song()))]
    );
    assert!(!shown.selection.contains(&"s1".to_owned()));
}

#[test]
fn more_opens_the_rows_menu_and_leaves_the_row_alone() {
    let server = FavoritesServer::new(&[]);
    let mut shown = a_row(&server);

    press_button(&mut shown, Role::Button, "More");
    shown.ui.update();

    assert!(untrack(|| shown.menu.is_open()));
    assert!(find(Role::MenuItem, "Play Next").is_some());
    assert!(!shown.selection.contains(&"s1".to_owned()));
}
