use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::{App, IntoElement, SharedString, Window};

type SelectHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// One way a list can be ordered, for [`sort_menu`].
#[derive(Clone)]
pub struct SortOption {
    label: SharedString,
    is_checked: bool,
    on_select: SelectHandler,
}

impl SortOption {
    pub fn new(
        label: impl Into<SharedString>,
        is_checked: bool,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            is_checked,
            on_select: Rc::new(on_select),
        }
    }
}

/// `None` in `options` draws a separator. `is_descending` is the way the list runs now, which
/// the icon shows.
pub fn sort_menu(
    id: &'static str,
    is_descending: bool,
    options: Vec<Option<SortOption>>,
) -> impl IntoElement {
    let options: Rc<[Option<SortOption>]> = options.into();
    let icon = if is_descending {
        IconName::SortDescending
    } else {
        IconName::SortAscending
    };
    Button::new(id)
        .ghost()
        .small()
        .icon(icon)
        .accessibility_label("Sort")
        .dropdown_menu(move |menu, _, _| {
            options.iter().fold(menu, |menu, option| match option {
                Some(option) => {
                    let on_select = option.on_select.clone();
                    menu.item(
                        PopupMenuItem::new(option.label.clone())
                            .checked(option.is_checked)
                            .on_click(move |_, window, cx| on_select(window, cx)),
                    )
                }
                None => menu.separator(),
            })
        })
}
