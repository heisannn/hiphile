use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::core::Ui;
use serde_json::{Value, json};

use super::*;
use crate::row_list::{RowList, Selection, row_list};
use crate::row_text::row_text;
use crate::testing::{
    click, double_click, find, has_text, middle_of, press, recording_commands, start,
};

fn song(fields: Value) -> Song {
    let mut song = json!({ "id": "s1", "title": "Blue in Green" });
    song.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    serde_json::from_value(song).unwrap()
}

fn by_miles() -> Song {
    song(json!({
        "artist": "Miles Davis",
        "artistId": "ar1",
        "album": "Kind of Blue",
        "albumId": "al1",
    }))
}

struct Row {
    ui: Ui,
    sent: Rc<RefCell<Vec<Command>>>,
    selection: Selection<String>,
    opened: Rc<Cell<usize>>,
}

/// `song` in a row of a list, with its line of links below its title.
fn in_a_row(song: Song) -> Row {
    let (commands, sent) = recording_commands();
    let selection = Selection::new();
    let opened = Rc::new(Cell::new(0));
    let ui = start({
        let opened = opened.clone();
        move || {
            let songs: Arc<[Song]> = Arc::from([song]);
            row_list(
                RowList::new(
                    "Songs",
                    move || songs.clone(),
                    |song: &Song| song.id().to_string(),
                    move |song, _| {
                        div()
                            .flex()
                            .flex_col()
                            .child(row_text(song.title().to_owned(), None))
                            .child(song_line(song, &commands))
                    },
                )
                .row_height(56.0)
                .selection(selection)
                .on_open(move |_| opened.set(opened.get() + 1)),
            )
        }
    });
    Row {
        ui,
        sent,
        selection,
        opened,
    }
}

#[test]
fn a_name_shows_what_it_names_and_leaves_the_row_alone() {
    let mut row = in_a_row(by_miles());

    let (x, y) = middle_of("Miles Davis");
    click(&mut row.ui, x, y);
    let (x, y) = middle_of("Kind of Blue");
    double_click(&mut row.ui, x, y);

    assert_eq!(
        *row.sent.borrow(),
        [
            Command::Show(Location::Artists(Some(ArtistId::new("ar1")))),
            Command::Show(Location::Albums(Some(AlbumId::new("al1")))),
            Command::Show(Location::Albums(Some(AlbumId::new("al1")))),
        ]
    );
    assert!(!row.selection.contains(&"s1".to_owned()));
    assert_eq!(row.opened.get(), 0);
}

#[test]
fn the_rest_of_the_row_still_selects_it() {
    let mut row = in_a_row(by_miles());

    let (x, y) = middle_of("Blue in Green");
    click(&mut row.ui, x, y);

    assert!(row.selection.contains(&"s1".to_owned()));
    assert!(row.sent.borrow().is_empty());
}

#[test]
fn a_name_is_a_link_to_assistive_technology() {
    let mut row = in_a_row(by_miles());

    press(&mut row.ui, Role::Link, "Miles Davis");

    assert_eq!(
        *row.sent.borrow(),
        [Command::Show(Location::Artists(Some(ArtistId::new("ar1"))))]
    );
}

#[test]
fn a_name_the_server_gives_no_id_for_is_plain_text() {
    let mut row = in_a_row(song(
        json!({ "artist": "Miles Davis", "album": "Kind of Blue" }),
    ));

    assert!(find(Role::Link, "Miles Davis").is_none());
    let (x, y) = middle_of("Miles Davis");
    click(&mut row.ui, x, y);

    assert!(row.sent.borrow().is_empty());
    assert!(row.selection.contains(&"s1".to_owned()));
}

#[test]
fn the_dot_is_only_between_two_things() {
    let _row = in_a_row(song(json!({ "artist": "Miles Davis", "artistId": "ar1" })));

    assert!(has_text("Miles Davis"));
    assert!(!has_text("·"));
}
