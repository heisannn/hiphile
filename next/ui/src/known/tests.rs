use std::cell::Cell;
use std::rc::Rc;

use anti_webui::prelude::div;
use serde_json::json;

use super::*;
use crate::testing::{NOWHERE, signed_in_core, start, wait_until};

fn song(title: &str) -> Song {
    serde_json::from_value(json!({ "id": title, "title": title })).unwrap()
}

#[test]
fn instrumentals_are_skipped_only_while_the_player_skips_them() {
    let (_core, hiphile) = signed_in_core(NOWHERE);
    let kept = Rc::new(Cell::new(None));
    let mut ui = start({
        let (hiphile, kept) = (hiphile.clone(), kept.clone());
        move || {
            let workspace = hiphile.workspace().expect("signed in");
            kept.set(Some(Known::follow(&hiphile, &workspace)));
            div()
        }
    });
    let known = kept.get().expect("made with the view");
    let instrumental = song("One (Instrumental)");
    assert!(!known.is_skipped(&instrumental));

    hiphile.settings().queue().set_skip_instrumentals(true);
    wait_until(&mut ui, || known.is_skipped(&instrumental));

    assert!(!known.is_skipped(&song("One")));
}
