use anti_webui::core::tree::NodeKind;
use anti_webui_md3e::components::text_button;

use super::*;
use crate::testing::start;

/// The color the text `label` is drawn in.
fn color_of(label: &str) -> Color {
    with_doc(|doc| {
        doc.nodes.iter().find_map(|(_, node)| match &node.kind {
            NodeKind::Text(shown) if shown.text == label => Some(node.computed.color),
            _ => None,
        })
    })
    .unwrap_or_else(|| panic!("no text {label:?}"))
}

#[test]
fn what_cant_be_undone_is_said_in_the_color_for_errors() {
    let _ui = start(|| {
        dialog_actions()
            .child(text_button("Cancel"))
            .child(irreversible_button("Delete"))
    });

    assert_eq!(color_of("Delete"), color_scheme().error);
    assert_eq!(color_of("Cancel"), color_scheme().primary);
}
