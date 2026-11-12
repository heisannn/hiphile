use std::cell::Cell;
use std::rc::Rc;

use anti_webui_md3e::components::icon_button;
use anti_webui_md3e::icons;

use super::*;
use crate::testing::{find, has_text, press, start};

#[test]
fn a_heading_is_one_for_assistive_technology() {
    let _ui = start(|| section_heading("Recently Played"));

    let heading = find(Role::Heading, "Recently Played").expect("the heading");
    assert_eq!(heading.level, Some(2));
}

#[test]
fn a_heading_can_end_in_a_button() {
    let _ui = start(|| {
        section_heading_with(
            "Random",
            icon_button(icons::REFRESH).access(|a| a.label("Reload Random")),
        )
    });

    let heading = find(Role::Heading, "Random").expect("the heading");
    let button = find(Role::Button, "Reload Random").expect("the button");
    assert!(button.bounds.x > heading.bounds.x + heading.bounds.width);
}

#[test]
fn a_heading_with_a_button_is_as_high_as_one_without() {
    let _ui = start(|| {
        div()
            .flex()
            .flex_col()
            .child(section_heading("Plain"))
            .child(section_heading_with(
                "Reloads",
                icon_button(icons::REFRESH).access(|a| a.label("Reload")),
            ))
    });

    let plain = find(Role::Heading, "Plain").unwrap().bounds;
    let reloads = find(Role::Heading, "Reloads").unwrap().bounds;
    assert_eq!(reloads.y - plain.y, LINE + ABOVE);
}

#[test]
fn a_loading_section_says_what_it_waits_for() {
    let _ui = start(|| section_loading("Loading Random"));

    assert!(find(Role::ProgressBar, "Loading Random").is_some());
}

#[test]
fn a_failed_section_says_why_and_tries_again() {
    let tries = Rc::new(Cell::new(0));
    let counted = tries.clone();
    let mut ui = start(move || {
        section_failure("The server didn't answer.", move || {
            counted.set(counted.get() + 1)
        })
    });

    assert!(has_text("The server didn't answer."));
    press(&mut ui, Role::Button, "Retry");
    assert_eq!(tries.get(), 1);
}
