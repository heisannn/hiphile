use std::cell::RefCell;
use std::sync::Arc;

use anti_webui::core::Ui;
use anti_webui::core::access::Checked;
use anti_webui::core::event::NamedKey;
use anti_webui::core::tree::with_doc;
use anti_webui_md3e::components::menu_item;
use serde_json::json;
use subsonic::Song;

use super::*;
use crate::Command;
use crate::link::song_line;
use crate::row_list::{RowList, Selection, row_list};
use crate::row_text::row_text;
use crate::testing::{
    click, find, frames, key, middle_of, nodes, press, recording_commands, right_click, start,
};

fn song() -> Song {
    serde_json::from_value(json!({
        "id": "s1",
        "title": "Blue in Green",
        "artist": "Miles Davis",
        "artistId": "ar1",
    }))
    .unwrap()
}

/// What a row's menu holds: an item that says which row it was opened on.
fn items(id: &str, chosen: &Rc<RefCell<Vec<String>>>) -> Vec<Vec<MenuItem>> {
    let (id, chosen) = (id.to_owned(), chosen.clone());
    vec![vec![
        menu_item("Play").on_click(move || chosen.borrow_mut().push(id.clone())),
    ]]
}

struct Shown {
    ui: Ui,
    menu: ContextMenu,
    covers: Covers,
    selection: Selection<String>,
    chosen: Rc<RefCell<Vec<String>>>,
    sent: Rc<RefCell<Vec<Command>>>,
}

/// A row of a song, whose menu opens where it is pressed with the secondary button and from its
/// More button.
fn a_row() -> Shown {
    let (commands, sent) = recording_commands();
    let covers = Covers::new();
    let menu = ContextMenu::new(covers);
    let selection = Selection::new();
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let ui = start({
        let chosen = chosen.clone();
        move || {
            let songs: Arc<[Song]> = Arc::from([song()]);
            let row = {
                let (chosen, commands) = (chosen.clone(), commands.clone());
                move |song: &Song, _| {
                    let id = song.id().to_string();
                    let chosen = chosen.clone();
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(row_text(song.title().to_owned(), None))
                                .child(song_line(song, &commands)),
                        )
                        .child(more_button(menu, move || items(&id, &chosen)))
                }
            };
            div()
                .w(600)
                .h(400)
                .child(row_list(
                    RowList::new(
                        "Songs",
                        move || songs.clone(),
                        |song: &Song| song.id().to_string(),
                        row,
                    )
                    .row_height(56.0)
                    .selection(selection)
                    .on_context_menu(move |id, at| menu.open_at(at, items(&id, &chosen))),
                ))
                .child(menu.view())
        }
    });
    Shown {
        ui,
        menu,
        covers,
        selection,
        chosen,
        sent,
    }
}

/// Where the menu hangs from, in the window.
fn hung_from(menu: ContextMenu) -> (i32, i32) {
    let rect = with_doc(|doc| doc.window_rect(menu.point.get().unwrap())).unwrap();
    (rect.x, rect.y)
}

#[test]
fn the_secondary_button_opens_the_rows_menu_where_it_was_pressed() {
    let mut shown = a_row();

    let (x, y) = middle_of("Blue in Green");
    right_click(&mut shown.ui, x, y);

    assert!(shown.menu.is_open());
    assert!(find(Role::MenuItem, "Play").is_some());
    assert_eq!(hung_from(shown.menu), (x.round() as i32, y.round() as i32));
    assert!(shown.selection.contains(&"s1".to_owned()));

    press(&mut shown.ui, Role::MenuItem, "Play");
    assert_eq!(*shown.chosen.borrow(), ["s1"]);
}

#[test]
fn the_secondary_button_on_a_name_opens_the_rows_menu_too() {
    let mut shown = a_row();

    let (x, y) = middle_of("Miles Davis");
    right_click(&mut shown.ui, x, y);

    assert!(find(Role::MenuItem, "Play").is_some());
    assert!(shown.sent.borrow().is_empty());
}

#[test]
fn more_opens_the_same_menu_below_itself_and_leaves_the_row_alone() {
    let mut shown = a_row();
    let more = find(Role::Button, "More").unwrap();
    assert_eq!(more.expanded, Some(false));

    let bounds = more.bounds;
    click(
        &mut shown.ui,
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );

    assert!(find(Role::MenuItem, "Play").is_some());
    assert_eq!(find(Role::Button, "More").unwrap().expanded, Some(true));
    assert_eq!(
        hung_from(shown.menu),
        (bounds.x.round() as i32, bounds.y.round() as i32)
    );
    assert!(!shown.selection.contains(&"s1".to_owned()));
}

fn more_looks_pressed() -> bool {
    find(Role::Button, "More").unwrap().checked == Some(Checked::True)
}

#[test]
fn more_looks_pressed_while_the_menu_it_opened_is_open() {
    let mut shown = a_row();
    assert!(!more_looks_pressed());

    press(&mut shown.ui, Role::Button, "More");
    assert!(more_looks_pressed());

    key(&mut shown.ui, NamedKey::Escape);
    frames(&mut shown.ui, 400);
    assert!(!more_looks_pressed());

    let (x, y) = middle_of("Blue in Green");
    right_click(&mut shown.ui, x, y);
    assert!(shown.menu.is_open());
    assert!(!more_looks_pressed());
}

#[test]
fn the_screens_are_covered_while_it_is_open() {
    let mut shown = a_row();
    assert!(!shown.covers.any());

    let (x, y) = middle_of("Blue in Green");
    right_click(&mut shown.ui, x, y);
    assert!(shown.covers.any());

    key(&mut shown.ui, NamedKey::Escape);
    frames(&mut shown.ui, 400);
    assert!(!shown.covers.any());
}

#[test]
fn escape_closes_it() {
    let mut shown = a_row();
    press(&mut shown.ui, Role::Button, "More");

    key(&mut shown.ui, NamedKey::Escape);
    frames(&mut shown.ui, 400);

    assert!(!shown.menu.is_open());
    assert!(nodes(Role::MenuItem).is_empty());
    assert_eq!(find(Role::Button, "More").unwrap().expanded, Some(false));
}

#[test]
fn it_closes_when_what_it_was_opened_from_has_gone() {
    let mut ui = start(|| {
        let menu = ContextMenu::new(Covers::new());
        let shown = signal(true);
        let remove = move || vec![vec![menu_item("Remove").on_click(move || shown.set(false))]];
        div()
            .w(400)
            .h(300)
            .child(show(move || shown.get(), move || more_button(menu, remove)))
            .child(menu.view())
    });
    press(&mut ui, Role::Button, "More");

    press(&mut ui, Role::MenuItem, "Remove");
    frames(&mut ui, 400);

    assert!(find(Role::Button, "More").is_none());
    assert!(nodes(Role::MenuItem).is_empty());
}

#[test]
fn each_opening_has_the_items_given_then() {
    let mut shown = a_row();
    press(&mut shown.ui, Role::Button, "More");
    key(&mut shown.ui, NamedKey::Escape);
    frames(&mut shown.ui, 400);

    shown.menu.open_at(
        PointF::new(10.0, 10.0),
        vec![vec![menu_item("Go to Artist")]],
    );
    shown.ui.update();

    assert!(find(Role::MenuItem, "Go to Artist").is_some());
    assert!(find(Role::MenuItem, "Play").is_none());
}
