//! Rows among the other parts of a page rather than in a row list, such as the songs Home shows
//! between its shelves of cards, or an artist's top songs above their albums.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::Ink;

use crate::row_list::{ROW_HEIGHT, ROW_PADDING};
use crate::selection::{CORNER, When, focus_ring, selection_fill, shows_keys_on};

/// From the edge of the page to the row, which puts what the row shows on the line of the rest of
/// the pane, as in a row list.
const INSET: f32 = crate::pane::INSET - ROW_PADDING;

/// What [`page_row`] shows.
pub struct PageRow {
    content: Div,
    label: Option<Rc<dyn Fn() -> String>>,
    selected: When,
    focused: When,
    keyed: When,
}

impl PageRow {
    /// `content` fills the row, such as [`crate::song_row::song_with_cover`].
    pub fn new(content: Div) -> Self {
        Self {
            content,
            label: None,
            selected: Rc::new(|| false),
            focused: Rc::new(|| false),
            keyed: Rc::new(|| false),
        }
    }

    /// What the row is called, for assistive technology. It is what the row shows otherwise. It
    /// may read signals, such as whether the row's song plays, and follows them.
    pub fn label(mut self, label: impl Fn() -> String + 'static) -> Self {
        self.label = Some(Rc::new(label));
        self
    }

    pub fn selected_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.selected = Rc::new(f);
        self
    }

    /// Whether what the page is in has the focus, which makes the selection stronger.
    pub fn focused_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.focused = Rc::new(f);
        self
    }

    /// Whether the keys are on the row while the focus shows, as a row list says of its rows.
    pub fn keyed_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.keyed = Rc::new(f);
        self
    }
}

/// A row as high as those of a row list and that looks like one, but takes no focus of its own:
/// what the page is in has it and moves the selection with the keys. Its clicks are the caller's.
pub fn page_row(row: PageRow) -> Div {
    let PageRow {
        content,
        label,
        selected,
        focused,
        keyed,
    } = row;
    let selected = memo(move || selected());
    let focused = memo(move || focused());
    let keyed = memo(move || keyed());
    let ink = Ink::new(|c| c.on_surface, || false);
    shows_keys_on(&ink, keyed);
    let frame = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .h(ROW_HEIGHT)
        .mx(INSET)
        .min_w(0)
        .rounded(CORNER)
        .select_none()
        .access(move |a| {
            let a = a.role(Role::ListOption).selected(selected.get());
            match &label {
                Some(label) => a.label(label()),
                None => a,
            }
        })
        .dyn_style(move |s| {
            let c = color_scheme();
            let s = focus_ring(s.text_color(c.on_surface), keyed.get(), &c);
            match selection_fill(&c, selected.get(), focused.get()) {
                Some(fill) => s.bg(fill),
                None => s,
            }
        })
        .child(content.flex_1().min_w(0).h_full());
    ink.paint(ink.listen(frame))
}

#[cfg(test)]
mod tests;
