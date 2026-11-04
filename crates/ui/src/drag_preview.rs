use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    Context, IntoElement, ParentElement as _, Pixels, Point, Render, SharedString, Styled as _,
    Window, div,
};

/// What follows the pointer while a list row is dragged to a new place.
pub struct DragPreview {
    title: SharedString,
    /// The pointer's offset in the row the drag started on.
    offset: Point<Pixels>,
}

impl DragPreview {
    pub fn new(title: SharedString, offset: Point<Pixels>) -> Self {
        Self { title, offset }
    }
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The drag draws this from the row's corner, where it was relative to
        // the pointer; the measured offset brings the chip to the pointer.
        div().pl(self.offset.x).pt(self.offset.y).child(
            div()
                .max_w_64()
                .px_3()
                .py_1()
                .truncate()
                .text_sm()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .text_color(cx.theme().popover_foreground)
                .shadow_md()
                .child(self.title.clone()),
        )
    }
}
