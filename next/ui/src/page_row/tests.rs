use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::row_text::row_text;
use crate::testing::{click_text, find, nodes, shows_keys, start};

fn content(title: &str) -> Div {
    row_text(title.to_owned(), None)
}

#[test]
fn a_row_is_an_option_that_says_whether_it_is_selected() {
    let selected = signal(false);
    let mut ui = start(move || {
        div().flex().flex_col().child(page_row(
            PageRow::new(content("Blue"))
                .label(|| "Blue, playing".to_owned())
                .selected_when(move || selected.get()),
        ))
    });

    let row = find(Role::ListOption, "Blue, playing").expect("the row");
    assert_eq!(row.selected, Some(false));
    selected.set(true);
    ui.update();
    let row = find(Role::ListOption, "Blue, playing").unwrap();
    assert_eq!(row.selected, Some(true));
}

#[test]
fn the_row_the_keys_are_on_shows_it() {
    let keyed = signal(false);
    let mut ui = start(move || {
        div()
            .flex()
            .flex_col()
            .child(page_row(
                PageRow::new(content("Blue")).keyed_when(move || keyed.get()),
            ))
            .child(page_row(PageRow::new(content("Red"))))
    });
    assert!(!shows_keys(Role::ListOption, "Blue"));

    keyed.set(true);
    ui.update();

    assert!(shows_keys(Role::ListOption, "Blue"));
    assert!(!shows_keys(Role::ListOption, "Red"));
}

#[test]
fn a_row_is_as_high_as_a_row_lists_and_on_the_line_of_the_pane() {
    let _ui = start(|| {
        div().w(600.0_f32).flex().flex_col().child(page_row(
            PageRow::new(content("Blue")).label(|| "Blue".to_owned()),
        ))
    });

    let row = find(Role::ListOption, "Blue").unwrap().bounds;
    assert_eq!(row.height, ROW_HEIGHT);
    assert_eq!(row.x + ROW_PADDING, crate::pane::INSET);
    assert_eq!(row.width, 600.0 - INSET * 2.0);
}

#[test]
fn a_row_takes_no_focus_of_its_own() {
    let _ui = start(|| page_row(PageRow::new(content("Blue"))));

    assert!(nodes(Role::ListOption).iter().all(|node| !node.focusable));
}

#[test]
fn a_rows_clicks_are_the_callers() {
    let clicks = Rc::new(Cell::new(0));
    let counted = clicks.clone();
    let mut ui = start(move || {
        page_row(PageRow::new(content("Blue"))).on_click(move |_| counted.set(counted.get() + 1))
    });

    click_text(&mut ui, "Blue");

    assert_eq!(clicks.get(), 1);
}
