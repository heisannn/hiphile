use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable, Size};
use gpui_kit::{
    App, IntoElement, ObjectFit, ParentElement as _, RenderOnce, SharedString, Styled as _,
    StyledImage as _, Window, div, img,
};

/// Album artwork, or a neutral placeholder while there is none.
///
/// The URL must stay the same while the same image is shown: Subsonic URLs
/// carry a per-request salt, so build it once per album, not per frame.
#[derive(IntoElement)]
pub struct CoverArt {
    url: Option<SharedString>,
    size: Size,
    fill: bool,
}

impl CoverArt {
    pub fn new(url: Option<SharedString>) -> Self {
        Self {
            url,
            size: Size::Medium,
            fill: false,
        }
    }

    /// Takes the width of its parent, and as much height, instead of a size.
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }
}

impl Sizable for CoverArt {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl RenderOnce for CoverArt {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let large = self.fill || matches!(self.size, Size::Large);
        let radius = if matches!(self.size, Size::Large) {
            cx.theme().radius_lg
        } else {
            cx.theme().radius
        };
        let muted = cx.theme().muted;
        let muted_foreground = cx.theme().muted_foreground;
        let placeholder = move || {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(muted)
                .text_color(muted_foreground)
                .child(Icon::new(IconName::Disc3).with_size(if large {
                    Size::Large
                } else {
                    Size::Small
                }))
                .into_any_element()
        };
        let frame = div().flex_none().overflow_hidden().rounded(radius);
        let frame = match self.size {
            _ if self.fill => frame.w_full().aspect_square(),
            Size::XSmall | Size::Small => frame.size_10(),
            Size::Medium | Size::Size(_) => frame.size_16(),
            Size::Large => frame.size_48(),
        };
        match self.url {
            Some(url) => frame.child(
                img(url)
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(placeholder)
                    .with_loading(placeholder),
            ),
            None => frame.child(placeholder()),
        }
    }
}
