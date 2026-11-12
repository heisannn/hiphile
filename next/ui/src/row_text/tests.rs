use anti_webui::core::tree::{NodeKind, with_doc};

use super::*;
use crate::testing::{has_text, start};

/// Whether what shows `shown` fades at its end.
fn fades(shown: &str) -> bool {
    with_doc(|d| {
        let text = d
            .nodes
            .iter()
            .find_map(|(id, node)| match &node.kind {
                NodeKind::Text(text) if text.text == shown => Some(id),
                _ => None,
            })
            .expect("the text shown");
        d.ancestors(text)
            .into_iter()
            .any(|id| d.nodes[id].filter.is_some())
    })
}

fn in_width(width: i32, view: Div) -> Div {
    div().w(width).flex().flex_col().child(view)
}

#[test]
fn the_fade_compiles() {
    anti_webui::wgpu::check_shader(&FADE_END).unwrap();
}

#[test]
fn only_what_does_not_fit_fades() {
    let title = signal("A Short Title".to_owned());
    let _ui = start(move || {
        div()
            .child(in_width(
                120,
                row_text(
                    "A title much too long for the row it is in",
                    Some("Short".into()),
                ),
            ))
            .child(in_width(600, row_text(move || title.get(), None)))
    });

    assert!(fades("A title much too long for the row it is in"));
    assert!(!fades("Short"));
    assert!(!fades("A Short Title"));
}

#[test]
fn a_title_that_grows_too_long_starts_to_fade() {
    let title = signal("Short".to_owned());
    let mut ui = start(move || in_width(200, row_text(move || title.get(), None)));
    assert!(!fades("Short"));

    title.set("Longer than the two hundred pixels it has".into());
    ui.update();
    ui.update();

    assert!(has_text("Longer than the two hundred pixels it has"));
    assert!(fades("Longer than the two hundred pixels it has"));
}
