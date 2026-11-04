use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
    button::Button,
    h_flex,
    input::{Input, InputState},
    resizable::{ResizableState, h_resizable, resizable_panel},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, ElementId, Entity, InteractiveElement as _, IntoElement, IsZero as _,
    ParentElement as _, Pixels, RenderOnce, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

use crate::CoverArt;
use crate::list_navigation::CONTROL_CONTEXT;

/// The list's width when first shown, in rem.
const LIST_WIDTH: f32 = 20.;
const LIST_MIN_WIDTH: f32 = 14.;
const LIST_MAX_WIDTH: f32 = 32.;
/// The narrowest the detail goes, in rem.
const DETAIL_MIN_WIDTH: f32 = 26.;
const DETAIL_MAX_WIDTH: f32 = 200.;

/// A list beside the detail of its selection, with the split between them
/// kept in `panes`, which the feature owns.
#[derive(IntoElement)]
pub struct MasterDetail {
    id: ElementId,
    panes: Entity<ResizableState>,
    list: AnyElement,
    detail: AnyElement,
}

impl MasterDetail {
    pub fn new(
        id: impl Into<ElementId>,
        panes: &Entity<ResizableState>,
        list: impl IntoElement,
        detail: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            panes: panes.clone(),
            list: list.into_any_element(),
            detail: detail.into_any_element(),
        }
    }

    /// The narrowest the two go, with the list as wide as the user left it.
    pub fn min_width(panes: &Entity<ResizableState>, window: &Window, cx: &App) -> Pixels {
        let rem = window.rem_size();
        let list = panes
            .read(cx)
            .sizes()
            .first()
            .copied()
            .filter(|width| !width.is_zero())
            .unwrap_or(rem * LIST_WIDTH);
        list + rem * DETAIL_MIN_WIDTH
    }
}

impl RenderOnce for MasterDetail {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let rem = window.rem_size();
        h_resizable(self.id)
            .with_state(&self.panes)
            .child(
                resizable_panel()
                    .size(rem * LIST_WIDTH)
                    .size_range(rem * LIST_MIN_WIDTH..rem * LIST_MAX_WIDTH)
                    .fixed()
                    .child(self.list),
            )
            .child(
                resizable_panel()
                    .size_range(rem * DETAIL_MIN_WIDTH..rem * DETAIL_MAX_WIDTH)
                    .child(self.detail),
            )
    }
}

/// What a list pane is titled, with how many it holds once that is known.
#[derive(IntoElement)]
pub struct ListPaneHeader {
    title: &'static str,
    count: Option<usize>,
    status: Option<AnyElement>,
    action: Option<AnyElement>,
}

impl ListPaneHeader {
    pub fn new(title: &'static str, count: Option<usize>) -> Self {
        Self {
            title,
            count,
            status: None,
            action: None,
        }
    }

    /// Shown after the count, for work under way.
    pub fn status(mut self, status: impl IntoElement) -> Self {
        self.status = Some(status.into_any_element());
        self
    }

    /// Set against the trailing edge.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for ListPaneHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // A button brings its own padding, so it comes closer to the edge than
        // text would to line up.
        let trailing_action = self.action.is_some();
        h_flex()
            .h_10()
            .flex_none()
            .pl_4()
            .map(|this| {
                if trailing_action {
                    this.pr_2()
                } else {
                    this.pr_4()
                }
            })
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_medium().child(self.title))
            .when_some(self.count, |this, count| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(count.to_string()),
                )
            })
            .children(self.status)
            .when_some(self.action, |this, action| {
                this.child(div().flex_1()).child(action)
            })
    }
}

/// The field that narrows down a list pane.
#[derive(IntoElement)]
pub struct ListFilter(Entity<InputState>);

impl ListFilter {
    pub fn new(input: &Entity<InputState>) -> Self {
        Self(input.clone())
    }
}

impl RenderOnce for ListFilter {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().flex_none().px_3().py_2().child(
            Input::new(&self.0)
                .small()
                .prefix(Icon::new(IconName::Search).small())
                .cleanable(true),
        )
    }
}

/// A failed update of a list still on show below it.
#[derive(IntoElement)]
pub struct RefreshFailure {
    title: &'static str,
    reason: SharedString,
    retry: Button,
}

impl RefreshFailure {
    pub fn new(title: &'static str, reason: SharedString, retry: Button) -> Self {
        Self {
            title,
            reason,
            retry,
        }
    }
}

impl RenderOnce for RefreshFailure {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .flex_none()
            .px_4()
            .pb_2()
            .gap_1()
            .child(div().text_sm().font_medium().child(self.title))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.reason),
            )
            .child(h_flex().pt_1().child(self.retry))
    }
}

/// The top of a detail: the cover beside what is on show and what can be done
/// with it.
#[derive(IntoElement)]
pub struct DetailHeader {
    cover_url: Option<SharedString>,
    title: SharedString,
    subtitle: Option<SharedString>,
    comment: Option<SharedString>,
    summary: SharedString,
    actions: Vec<AnyElement>,
}

impl DetailHeader {
    pub fn new(
        cover_url: Option<SharedString>,
        title: SharedString,
        summary: SharedString,
    ) -> Self {
        Self {
            cover_url,
            title,
            subtitle: None,
            comment: None,
            summary,
            actions: Vec::new(),
        }
    }

    pub fn subtitle(mut self, subtitle: Option<SharedString>) -> Self {
        self.subtitle = subtitle;
        self
    }

    pub fn comment(mut self, comment: Option<SharedString>) -> Self {
        self.comment = comment;
        self
    }

    /// A button below the summary, which keeps Enter and Space for itself.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for DetailHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .flex_none()
            .items_end()
            .gap_6()
            .p_6()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(CoverArt::new(self.cover_url).large())
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_2xl().font_semibold().child(self.title))
                    .when_some(self.subtitle, |this, subtitle| {
                        this.child(div().child(subtitle))
                    })
                    // The cover sets the header's height, which a long
                    // comment would take from the songs.
                    .when_some(self.comment, |this, comment| {
                        this.child(div().text_ellipsis().line_clamp(2).child(comment))
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.summary),
                    )
                    .child(
                        h_flex()
                            .key_context(CONTROL_CONTEXT)
                            .pt_3()
                            .gap_2()
                            .children(self.actions),
                    ),
            )
    }
}
