use actions::Favorite;
use gpui_kit::App;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable as _,
    button::{Button, ButtonVariants as _},
};

/// Disabled until the favorites are known.
pub fn favorite_button(id: &'static str, favorite: Favorite, cx: &App) -> Button {
    let target = favorite.target();
    let is_favorite = actions::is_favorite(&target, cx);
    let icon = if is_favorite == Some(true) {
        IconName::StarFill
    } else {
        IconName::Star
    };
    Button::new(id)
        .ghost()
        .icon(icon)
        .accessibility_label(actions::favorite_command_label(&target, cx))
        .disabled(is_favorite.is_none())
        .on_click(move |_, window, cx| actions::toggle_favorite(favorite.clone(), window, cx))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    use actions::SetFavorite;
    use gpui_kit::{
        Context, IntoElement, Modifiers, ParentElement as _, Render, Styled as _, TestAppContext,
        VisualTestContext, Window, div, point, px,
    };
    use subsonic::{Artist, ArtistId};

    use super::*;

    struct Star;

    impl Render for Star {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(favorite_button("star", Favorite::Artist(artist()), cx))
        }
    }

    fn artist() -> Artist {
        Artist::new(ArtistId::new("x"), "X".to_owned(), None, None)
    }

    fn clicked(
        known: Option<HashSet<subsonic::StarTarget>>,
        cx: &mut TestAppContext,
    ) -> Vec<SetFavorite> {
        cx.update(|cx| {
            gpui_kit::init(cx);
            actions::set_known_favorites(known, cx);
        });
        let (_, cx): (_, &mut VisualTestContext) = cx.add_window_view(|_, _| Star);
        let asked = Rc::new(RefCell::new(Vec::new()));
        cx.update(|_, cx| {
            let asked = asked.clone();
            cx.on_action(move |action: &SetFavorite, _| asked.borrow_mut().push(action.clone()));
        });
        cx.run_until_parked();

        cx.simulate_click(point(px(8.), px(8.)), Modifiers::default());
        cx.run_until_parked();
        asked.take()
    }

    #[gpui_kit::test]
    fn the_star_adds_what_is_no_favorite(cx: &mut TestAppContext) {
        assert_eq!(
            clicked(Some(HashSet::new()), cx),
            [SetFavorite::new(Favorite::Artist(artist()), true)]
        );
    }

    #[gpui_kit::test]
    fn the_star_takes_out_a_favorite(cx: &mut TestAppContext) {
        let known = HashSet::from([Favorite::Artist(artist()).target()]);
        assert_eq!(
            clicked(Some(known), cx),
            [SetFavorite::new(Favorite::Artist(artist()), false)]
        );
    }

    #[gpui_kit::test]
    fn the_star_does_nothing_until_the_favorites_are_known(cx: &mut TestAppContext) {
        assert_eq!(clicked(None, cx), []);
    }
}
