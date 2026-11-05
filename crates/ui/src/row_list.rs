use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::{
    Div, InteractiveElement as _, IntoElement, ParentElement as _, Role, Stateful,
    StatefulInteractiveElement as _, Styled as _, UniformListScrollHandle, div, uniform_list,
};

/// The caller says how much room it takes.
///
/// `label` names the list for a screen reader. `render_row` is given a row's
/// position, and may run whenever the list measures or redraws.
pub fn row_list<R: IntoElement>(
    id: &'static str,
    label: &'static str,
    len: usize,
    scroll: &UniformListScrollHandle,
    render_row: impl Fn(usize) -> Option<R> + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .role(Role::List)
        .aria_label(label)
        .child(
            uniform_list("rows", len, move |range, _, _| {
                range.filter_map(&render_row).collect()
            })
            .size_full()
            .track_scroll(scroll),
        )
        .child(Scrollbar::vertical(scroll))
}
