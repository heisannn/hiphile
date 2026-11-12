use std::cell::Cell;

use anti_webui::prelude::NamedKey;

use super::*;
use crate::testing::{click_text, find, nodes, start};

fn rows() -> Vec<Vec<&'static str>> {
    vec![vec!["s1"], vec!["s2"], vec!["a1", "a2", "a3"], vec!["a4"]]
}

#[test]
fn a_move_with_nothing_selected_starts_at_an_end() {
    for motion in [
        GridMotion::Up,
        GridMotion::Down,
        GridMotion::Left,
        GridMotion::Right,
    ] {
        assert_eq!(step(&rows(), None, motion), Some("s1"));
    }
    assert_eq!(step(&rows(), None, GridMotion::Last), Some("a4"));
}

#[test]
fn a_selection_no_longer_on_the_screen_starts_over() {
    assert_eq!(step(&rows(), Some(&"gone"), GridMotion::Down), Some("s1"));
}

#[test]
fn nothing_to_select_stays_nothing() {
    assert_eq!(step::<&str>(&[], None, GridMotion::Down), None);
    assert_eq!(step(&[], Some(&"s1"), GridMotion::Last), None);
}

#[test]
fn up_and_down_keep_the_column_where_the_row_has_it() {
    let rows = rows();
    assert_eq!(step(&rows, Some(&"s2"), GridMotion::Down), Some("a1"));
    assert_eq!(step(&rows, Some(&"a1"), GridMotion::Down), Some("a4"));
    assert_eq!(step(&rows, Some(&"a3"), GridMotion::Down), Some("a4"));
    assert_eq!(step(&rows, Some(&"a3"), GridMotion::Up), Some("s2"));
}

#[test]
fn the_ends_hold() {
    let rows = rows();
    assert_eq!(step(&rows, Some(&"s1"), GridMotion::Up), Some("s1"));
    assert_eq!(step(&rows, Some(&"a4"), GridMotion::Down), Some("a4"));
    assert_eq!(step(&rows, Some(&"a1"), GridMotion::Left), Some("a1"));
    assert_eq!(step(&rows, Some(&"a3"), GridMotion::Right), Some("a3"));
}

#[test]
fn left_and_right_stay_in_the_row() {
    let rows = rows();
    assert_eq!(step(&rows, Some(&"a1"), GridMotion::Right), Some("a2"));
    assert_eq!(step(&rows, Some(&"a2"), GridMotion::Left), Some("a1"));
    assert_eq!(step(&rows, Some(&"s1"), GridMotion::Right), Some("s1"));
}

#[test]
fn pages_move_several_rows_and_stop_at_the_ends() {
    let rows: Vec<_> = (0..10).map(|ix| vec![ix]).collect();
    let page = |from: i32, motion| step(&rows, Some(&from), motion);
    assert_eq!(page(1, GridMotion::PageDown), Some(5));
    assert_eq!(page(8, GridMotion::PageDown), Some(9));
    assert_eq!(page(5, GridMotion::PageUp), Some(1));
    assert_eq!(page(2, GridMotion::PageUp), Some(0));
}

#[test]
fn first_and_last_reach_the_ends() {
    let rows = rows();
    assert_eq!(step(&rows, Some(&"a2"), GridMotion::First), Some("s1"));
    assert_eq!(step(&rows, Some(&"s2"), GridMotion::Last), Some("a4"));
}

#[test]
fn columns_follow_the_width_and_never_reach_zero() {
    assert_eq!(columns_for(0.0), 1);
    assert_eq!(columns_for(100.0), 1);
    let four = INSET * 2.0 + CARD_MIN_WIDTH * 4.0 + CARD_GAP * 3.0;
    assert_eq!(columns_for(four - 1.0), 3);
    assert_eq!(columns_for(four), 4);
    assert_eq!(columns_for(four + CARD_MIN_WIDTH), 4);
    assert_eq!(columns_for(four + CARD_MIN_WIDTH + CARD_GAP), 5);
}

#[test]
fn keys_with_modifiers_are_not_moves() {
    assert_eq!(
        GridMotion::of(&Keystroke::named(NamedKey::ArrowDown)),
        Some(GridMotion::Down)
    );
    assert_eq!(
        GridMotion::of(&Keystroke::named(NamedKey::End)),
        Some(GridMotion::Last)
    );
    assert_eq!(
        GridMotion::of(&Keystroke::named(NamedKey::ArrowDown).with_shift()),
        None
    );
    assert_eq!(GridMotion::of(&Keystroke::character('j')), None);
}

#[test]
fn a_card_is_an_option_that_says_whether_it_is_selected() {
    let selected = signal(false);
    let mut ui = start(move || {
        card_row(
            4,
            vec![
                card(Card::new(div(), "Blue").selected_when(move || selected.get())),
                card(Card::new(div(), "Red").details("Someone")),
            ],
        )
    });

    assert_eq!(
        find(Role::ListOption, "Blue").unwrap().selected,
        Some(false)
    );
    selected.set(true);
    ui.update();
    assert_eq!(find(Role::ListOption, "Blue").unwrap().selected, Some(true));
}

#[test]
fn a_row_of_fewer_cards_leaves_the_rest_of_it_empty() {
    let _ui = start(|| {
        div().w(800.0_f32).child(card_row(
            4,
            vec![
                card(Card::new(div(), "Blue")),
                card(Card::new(div(), "Red")),
            ],
        ))
    });

    let blue = find(Role::ListOption, "Blue").unwrap().bounds;
    let red = find(Role::ListOption, "Red").unwrap().bounds;
    let each = (800.0 - INSET * 2.0 - CARD_GAP * 3.0) / 4.0;
    assert_eq!(blue.width, each);
    assert_eq!(red.x - blue.x, each + CARD_GAP);
}

#[test]
fn a_card_takes_no_focus_of_its_own() {
    let _ui = start(|| card(Card::new(div(), "Blue")));

    assert!(nodes(Role::ListOption).iter().all(|node| !node.focusable));
}

#[test]
fn a_cards_clicks_are_the_callers() {
    let clicks = Rc::new(Cell::new(0));
    let counted = clicks.clone();
    let mut ui = start(move || {
        card(Card::new(div(), "Blue")).on_click(move |_| counted.set(counted.get() + 1))
    });

    click_text(&mut ui, "Blue");

    assert_eq!(clicks.get(), 1);
}
