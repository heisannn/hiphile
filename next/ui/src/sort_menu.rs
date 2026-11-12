//! The order of a list, chosen from a menu: a button for the bar of the screen that names the
//! order and which way it runs.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    ButtonStyle, MenuItem, MenuStyle, menu_item, sized_icon, styled_menu, styled_toggle_button,
};
use anti_webui_md3e::icons;

/// One way a list can be ordered, or run.
#[derive(Clone)]
pub struct SortOption {
    label: String,
    is_checked: Rc<dyn Fn() -> bool>,
    on_select: Rc<dyn Fn()>,
}

impl SortOption {
    /// `is_checked` says whether the list is in this order now, and may read signals.
    pub fn new(
        label: impl Into<String>,
        is_checked: impl Fn() -> bool + 'static,
        on_select: impl Fn() + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            is_checked: Rc::new(is_checked),
            on_select: Rc::new(on_select),
        }
    }
}

/// A button that names the checked option of the first of `groups`, with an arrow for which way
/// the list runs, and opens a menu of the groups. It looks pressed while that is open.
/// `is_descending` may read signals.
pub fn sort_menu(is_descending: impl Fn() -> bool + 'static, groups: Vec<Vec<SortOption>>) -> Div {
    let open = signal(false);
    let anchor = node_ref();
    let orders = groups.first().cloned().unwrap_or_default();
    let order = Rc::new(move || {
        orders
            .iter()
            .find(|order| (order.is_checked)())
            .map(|order| order.label.clone())
    });
    let is_descending = Rc::new(is_descending);
    let shown = order.clone();
    let label = move || shown().unwrap_or_else(|| "Sort".to_owned());
    let (told, way) = (order, is_descending.clone());
    let name = move || match told() {
        Some(order) if way() => format!("Sort by {order}, Descending"),
        Some(order) => format!("Sort by {order}, Ascending"),
        None => "Sort".to_owned(),
    };
    let items = groups
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .map(|option| {
                    let SortOption {
                        label,
                        is_checked,
                        on_select,
                    } = option;
                    menu_item(label)
                        .selected_when(move || is_checked())
                        .on_click(move || on_select())
                })
                .collect::<Vec<MenuItem>>()
        })
        .collect();
    let arrow = sized_icon(icons::ARROW_UPWARD, 20.0)
        .dyn_style(move |s| s.rotate(if is_descending() { 180.0_f32 } else { 0.0 }));
    div()
        .flex()
        .flex_none()
        .node_ref(anchor)
        .child(
            styled_toggle_button(label, open, ButtonStyle::text())
                .access(move |a| a.role(Role::Button).label(name()).expanded(open.get()))
                .child(arrow),
        )
        .child(styled_menu(
            move || open.get(),
            move || open.set(false),
            items,
            MenuStyle::new()
                .anchor(anchor)
                .placement(Placement::BelowEnd),
        ))
}

#[cfg(test)]
mod tests;
