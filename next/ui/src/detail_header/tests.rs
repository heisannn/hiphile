use std::cell::Cell;

use super::*;
use crate::testing::{find, has_text, press, start, start_at};

fn picture() -> Div {
    div().access(|a| a.role(Role::Image).label("Cover"))
}

fn header() -> DetailHeader {
    DetailHeader::new(picture(), "Blue", "2020 · 10 songs · 40:00")
        .subtitle(div().child("Someone"))
        .comment(Some("Recorded live.".to_owned()))
}

#[test]
fn the_title_is_the_panes_heading() {
    let _ui = start(|| detail_header(header()));

    let heading = find(Role::Heading, "Blue").expect("the heading");
    assert_eq!(heading.level, Some(2));
    assert!(has_text("Someone"));
    assert!(has_text("Recorded live."));
    assert!(has_text("2020 · 10 songs · 40:00"));
}

#[test]
fn a_blank_comment_is_none() {
    let _ui = start(|| detail_header(header().comment(Some("  ".to_owned()))));

    assert!(!has_text("  "));
    assert!(!has_text("Recorded live."));
}

#[test]
fn the_cover_is_smaller_in_a_narrow_pane() {
    let _ui = start_at(|| detail_header(header()).w_full(), (WIDE as i32, 600));
    assert_eq!(find(Role::Image, "Cover").unwrap().bounds.width, COVER);

    let _ui = start_at(|| detail_header(header()).w_full(), (WIDE as i32 - 1, 600));
    assert_eq!(
        find(Role::Image, "Cover").unwrap().bounds.width,
        NARROW_COVER
    );
}

#[test]
fn an_artists_picture_is_round() {
    let _ui = start(|| detail_header(header().artist_picture()));

    let cover = find(Role::Image, "Cover").unwrap().bounds;
    assert_eq!(cover.width, cover.height);
}

#[test]
fn play_and_shuffle_run_what_they_are_given() {
    let played = Rc::new(Cell::new(0));
    let shuffled = Rc::new(Cell::new(0));
    let (p, s) = (played.clone(), shuffled.clone());
    let mut ui = start(move || {
        play_and_shuffle(
            move || p.set(p.get() + 1),
            move || s.set(s.get() + 1),
            || false,
        )
    });

    press(&mut ui, Role::Button, "Play");
    press(&mut ui, Role::Button, "Shuffle");

    assert_eq!((played.get(), shuffled.get()), (1, 1));
}

#[test]
fn play_and_shuffle_are_disabled_with_nothing_to_play() {
    let empty = signal(true);
    let mut ui = start(move || play_and_shuffle(|| {}, || {}, move || empty.get()));

    assert!(find(Role::Button, "Play").unwrap().disabled);
    assert!(find(Role::Button, "Shuffle").unwrap().disabled);
    empty.set(false);
    ui.update();
    assert!(!find(Role::Button, "Play").unwrap().disabled);
    assert!(!find(Role::Button, "Shuffle").unwrap().disabled);
}
