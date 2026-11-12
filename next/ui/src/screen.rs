use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::typography::TextStyle;

use crate::window::{kept_still, moves_window};

/// The height of the small top app bar, which a band beside it keeps to stay in line.
pub const APP_BAR_HEIGHT: f32 = 64.0;

/// What a screen shows: its title and what it does, in the bar above it, and itself below.
pub struct Screen {
    pub title: String,
    /// Icon buttons and menus at the end of the bar, such as sorting.
    pub actions: Vec<Div>,
    /// Made of panes, such as [`crate::pane::pane`] or [`crate::list_detail::list_detail`]: the
    /// shell gives it only its margins.
    pub body: Div,
    /// Where the keys go while nothing has the focus and nothing on the screen has had it, as
    /// after a menu opened from where the focus was not: what the screen is mostly about, such
    /// as its list.
    pub home: Option<NodeRef>,
}

impl Screen {
    pub fn new(title: impl Into<String>, body: Div) -> Self {
        Self {
            title: title.into(),
            actions: Vec::new(),
            body,
            home: None,
        }
    }

    pub fn home(mut self, node: NodeRef) -> Self {
        self.home = Some(node);
        self
    }

    pub fn action(mut self, action: Div) -> Self {
        self.actions.push(action);
        self
    }
}

/// The small top app bar of Material 3 with room for going back and forward, which the one of
/// `anti_webui_md3e` has not. Dragging it moves the window.
pub fn app_bar(navigation: Vec<Div>, title: String, actions: Vec<Div>) -> Div {
    moves_window(
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(APP_BAR_HEIGHT)
            .px(4.0_f32)
            .select_none()
            .child(kept_still(div().flex().flex_row().children(navigation)))
            .child(
                TextStyle::TITLE_LARGE
                    .apply(div())
                    .ml(8)
                    .flex_1()
                    .min_w(0)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .dyn_style(|s| s.text_color(color_scheme().on_surface))
                    .access(|a| a.role(Role::Heading).level(1))
                    .child(text(title)),
            )
            .child(kept_still(
                div().flex().flex_row().items_center().children(actions),
            )),
    )
}
