use actions::ShowAlbum;
use gpui_kit::component::{ActiveTheme as _, Sizable, Size};
use gpui_kit::{App, ElementId, IntoElement, RenderOnce, SharedString, Window};
use subsonic::{AlbumId, Song};

use crate::name_link::{name_link, plain_name};

/// An album's name, which shows the album when clicked. A name the server gives
/// no ID for is plain text: there is nowhere to go.
#[derive(IntoElement)]
pub struct AlbumLink {
    name: SharedString,
    album: Option<AlbumId>,
    size: Size,
}

impl AlbumLink {
    pub fn new(name: impl Into<SharedString>, album: Option<AlbumId>) -> Self {
        Self {
            name: name.into(),
            album,
            size: Size::XSmall,
        }
    }

    /// The link to the album of `song`, when it names one.
    pub fn of_song(song: &Song) -> Option<Self> {
        let name = song.album()?;
        Some(Self::new(name.to_owned(), song.album_id().cloned()))
    }
}

impl Sizable for AlbumLink {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl RenderOnce for AlbumLink {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { name, album, size } = self;
        let color = cx.theme().muted_foreground;
        let Some(album) = album else {
            return plain_name(name, color);
        };
        name_link(
            ElementId::Name(format!("album-{album}").into()),
            name,
            size,
            color,
            move |window, cx| {
                window.dispatch_action(Box::new(ShowAlbum::new(album.clone())), cx);
            },
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::{
        ClickEvent, Context, InteractiveElement as _, ParentElement as _, Render,
        StatefulInteractiveElement as _, Styled as _, TestAppContext, VisualTestContext, div,
        point, px, size,
    };

    use super::*;

    /// A row around a link, which notes clicks that reach it.
    struct Row {
        album: Option<AlbumId>,
        clicked: usize,
    }

    impl Render for Row {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("row")
                .size_full()
                .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.clicked += 1))
                .child(AlbumLink::new("Kind of Blue", self.album.clone()))
        }
    }

    /// What clicking the link results in: the albums shown, and the clicks the
    /// row got.
    fn click(album: Option<AlbumId>, cx: &mut TestAppContext) -> (Vec<AlbumId>, usize) {
        let shown = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            let shown = shown.clone();
            cx.on_action(move |action: &ShowAlbum, _| {
                shown.borrow_mut().push(action.album().clone());
            });
        });
        let (row, cx) = cx.add_window_view(|_, _| Row { album, clicked: 0 });
        let cx: &mut VisualTestContext = cx;
        cx.simulate_resize(size(px(400.), px(100.)));
        cx.run_until_parked();
        cx.simulate_click(point(px(10.), px(6.)), Default::default());
        cx.run_until_parked();
        let shown = shown.take();
        (shown, cx.update(|_, cx| row.read(cx).clicked))
    }

    #[gpui_kit::test]
    fn a_click_shows_the_album_and_stays_out_of_the_row(cx: &mut TestAppContext) {
        let (shown, row_clicks) = click(Some(AlbumId::new("al1")), cx);
        assert_eq!(shown, [AlbumId::new("al1")]);
        assert_eq!(row_clicks, 0);
    }

    #[gpui_kit::test]
    fn a_name_with_no_album_to_show_leaves_the_click_to_the_row(cx: &mut TestAppContext) {
        let (shown, row_clicks) = click(None, cx);
        assert!(shown.is_empty());
        assert_eq!(row_clicks, 1);
    }
}
