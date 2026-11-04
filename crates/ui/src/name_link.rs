use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{
    AnyElement, App, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, Window, div,
};

/// A name that is plain text, for one with nowhere to go.
pub(crate) fn plain_name(name: SharedString, color: Hsla) -> AnyElement {
    div()
        .min_w_0()
        .truncate()
        .text_color(color)
        .child(name)
        .into_any_element()
}

/// A name in a row's text that goes somewhere when clicked. It is not a Tab
/// stop: from the keyboard, the view with the selection handles going there.
///
/// Quiet until the pointer is on it: a row's details are read far more often
/// than they are clicked. The hover is the name's own, as the button keeps its
/// own to itself.
pub(crate) fn name_link(
    id: ElementId,
    name: SharedString,
    size: Size,
    color: Hsla,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let foreground = cx.theme().foreground;
    // Over a row that is itself hovered, since the pointer is on both.
    let hover_background = foreground.opacity(0.1);
    Button::new(id)
        .text()
        .with_size(size)
        .tab_stop(false)
        .flex_shrink(1.)
        .min_w_0()
        // Out as far as the name is padded, so the name itself stays in line
        // with the text around it.
        .mx_neg_1()
        .my_neg_0p5()
        .text_color(color)
        .accessibility_label(name.clone())
        .child(
            div()
                .min_w_0()
                .px_1()
                .py_0p5()
                .truncate()
                .rounded(cx.theme().radius)
                .hover(move |this| this.text_color(foreground).bg(hover_background))
                .child(name),
        )
        // Inside a row or a card, whose own click means something else.
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .into_any_element()
}
