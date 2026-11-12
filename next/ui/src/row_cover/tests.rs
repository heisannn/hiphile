use super::*;
use crate::testing::{signed_in_core, start};

fn bounds_of_cover(kind: CoverKind) -> (f32, f32) {
    let (_core, hiphile) = signed_in_core("http://127.0.0.1:9");
    let workspace = hiphile.workspace().expect("signed in");
    let _ui = start(move || {
        div().child(
            row_cover(&workspace, None, kind)
                .access(|a| a.role(Role::Image).label("Cover"))
                .flex_none(),
        )
    });
    let cover = crate::testing::find(Role::Image, "Cover").expect("the cover");
    (cover.bounds.width, cover.bounds.height)
}

#[test]
fn a_rows_cover_is_square_at_its_side() {
    assert_eq!(bounds_of_cover(CoverKind::Album), (ROW_COVER, ROW_COVER));
    assert_eq!(bounds_of_cover(CoverKind::Playlist), (ROW_COVER, ROW_COVER));
}
