use actions::ShowArtist;
use gpui_kit::component::{ActiveTheme as _, Sizable, Size, h_flex};
use gpui_kit::{
    AnyElement, App, ElementId, IntoElement, ParentElement as _, RenderOnce, SharedString,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use subsonic::{Album, ArtistId, Song};

use crate::album_link::AlbumLink;
use crate::name_link::{name_link, plain_name};

/// From the keyboard, the view with the selection handles [`actions::GoToArtist`] instead.
///
/// A name the server gives no ID for is plain text: there is nowhere to go.
#[derive(IntoElement)]
pub struct ArtistLink {
    name: SharedString,
    artist: Option<ArtistId>,
    size: Size,
    muted: bool,
}

impl ArtistLink {
    pub fn new(name: impl Into<SharedString>, artist: Option<ArtistId>) -> Self {
        Self {
            name: name.into(),
            artist,
            size: Size::XSmall,
            muted: true,
        }
    }

    /// In the size and color of the text around it, for a line of its own.
    pub fn prominent(mut self) -> Self {
        self.size = Size::Medium;
        self.muted = false;
        self
    }
}

impl Sizable for ArtistLink {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl RenderOnce for ArtistLink {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            name,
            artist,
            size,
            muted,
        } = self;
        let color = if muted {
            cx.theme().muted_foreground
        } else {
            cx.theme().foreground
        };
        let Some(artist) = artist else {
            return plain_name(name, color);
        };
        name_link(
            ElementId::Name(format!("artist-{artist}").into()),
            name,
            size,
            color,
            move |window, cx| {
                window.dispatch_action(Box::new(ShowArtist::new(artist.clone())), cx);
            },
            cx,
        )
    }
}

#[derive(IntoElement)]
pub struct ArtistLine {
    artist: Option<ArtistLink>,
    rest: Option<AnyElement>,
}

impl ArtistLine {
    pub fn new(artist: Option<ArtistLink>, rest: Option<SharedString>) -> Self {
        let rest = rest.map(|rest| div().min_w_0().truncate().child(rest).into_any_element());
        Self { artist, rest }
    }

    pub fn of_song(song: &Song) -> Self {
        Self::of_song_by(song_artist_link(song), song)
    }

    /// `artist`, in place of the one `song` names, and the song's album.
    pub fn of_song_by(artist: Option<ArtistLink>, song: &Song) -> Self {
        Self {
            artist,
            rest: AlbumLink::of_song(song).map(IntoElement::into_any_element),
        }
    }

    pub fn of_album(album: &Album) -> Self {
        Self::new(
            album_artist_link(album),
            album.year().map(|year| year.to_string().into()),
        )
    }
}

impl RenderOnce for ArtistLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let separated = self.artist.is_some();
        h_flex()
            .min_w_0()
            .gap_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .children(self.artist)
            .when_some(self.rest, |this, rest| {
                this.when(separated, |this| this.child(div().flex_none().child("·")))
                    .child(rest)
            })
    }
}

pub fn song_artist_link(song: &Song) -> Option<ArtistLink> {
    let name = song.artist()?;
    Some(ArtistLink::new(name.to_owned(), song.artist_id().cloned()))
}

pub fn album_artist_link(album: &Album) -> Option<ArtistLink> {
    let name = album.artist()?;
    Some(ArtistLink::new(name.to_owned(), album.artist_id().cloned()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::{
        ClickEvent, Context, InteractiveElement as _, Render, StatefulInteractiveElement as _,
        TestAppContext, VisualTestContext, point, px, size,
    };

    use super::*;

    struct Row {
        artist: Option<ArtistId>,
        clicked: usize,
    }

    impl Render for Row {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("row")
                .size_full()
                .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.clicked += 1))
                .child(ArtistLink::new("Miles Davis", self.artist.clone()))
        }
    }

    /// What clicking the link results in: the artists shown, and the clicks
    /// the row got.
    fn click(artist: Option<ArtistId>, cx: &mut TestAppContext) -> (Vec<ArtistId>, usize) {
        let shown = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            gpui_kit::init(cx);
            let shown = shown.clone();
            cx.on_action(move |action: &ShowArtist, _| {
                shown.borrow_mut().push(action.artist().clone());
            });
        });
        let (row, cx) = cx.add_window_view(|_, _| Row { artist, clicked: 0 });
        let cx: &mut VisualTestContext = cx;
        cx.simulate_resize(size(px(400.), px(100.)));
        cx.run_until_parked();
        cx.simulate_click(point(px(10.), px(6.)), Default::default());
        cx.run_until_parked();
        let shown = shown.take();
        (shown, cx.update(|_, cx| row.read(cx).clicked))
    }

    #[gpui_kit::test]
    fn a_click_shows_the_artist_and_stays_out_of_the_row(cx: &mut TestAppContext) {
        let (shown, row_clicks) = click(Some(ArtistId::new("ar1")), cx);
        assert_eq!(shown, [ArtistId::new("ar1")]);
        assert_eq!(row_clicks, 0);
    }

    #[gpui_kit::test]
    fn a_name_without_an_id_goes_nowhere(cx: &mut TestAppContext) {
        let (shown, row_clicks) = click(None, cx);
        assert!(shown.is_empty());
        assert_eq!(row_clicks, 1);
    }
}
