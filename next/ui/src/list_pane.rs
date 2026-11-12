//! What goes above a list on its pane: the filter, how many there are and what to do, and a
//! refresh that failed. The title is the screen's, in the bar.

use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    TextFieldStyle, icon_button, indeterminate_circular_progress, outlined_text_field, sized_icon,
};
use anti_webui_md3e::icons;
use anti_webui_md3e::typography::TextStyle;

use crate::load_state::retry_button;
use crate::pane::INSET;
use crate::type_scale::{error_text, muted_text, styled_text};

/// As high as a small icon button.
const LINE: f32 = 40.0;
/// What a small icon button has between its edge and its icon, which brings it closer to the edge
/// than text for its icon to line up.
const BUTTON_PADDING: f32 = 8.0;
const UPDATING: f32 = 20.0;

/// What [`list_header`] shows.
pub struct ListHeader {
    count: Rc<dyn Fn() -> Option<String>>,
    updating: Option<(String, Rc<dyn Fn() -> bool>)>,
    actions: Vec<Div>,
}

impl ListHeader {
    /// `count` says how many there are, such as "12 albums", or `None` before that is known. It
    /// may read signals.
    pub fn new(count: impl Fn() -> Option<String> + 'static) -> Self {
        Self {
            count: Rc::new(count),
            updating: None,
            actions: Vec::new(),
        }
    }

    /// Shows that the list is being brought up to date while `when` says so. `label` names that
    /// for assistive technology.
    pub fn updating(mut self, label: impl Into<String>, when: impl Fn() -> bool + 'static) -> Self {
        self.updating = Some((label.into(), Rc::new(when)));
        self
    }

    /// A button at the end of the line, such as the one that makes a new playlist.
    pub fn action(mut self, action: Div) -> Self {
        self.actions.push(action);
        self
    }
}

/// How many there are, whether they are being updated, and buttons at the end.
pub fn list_header(header: ListHeader) -> Div {
    let ListHeader {
        count,
        updating,
        actions,
    } = header;
    let has_actions = !actions.is_empty();
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(8.0_f32)
        .h(LINE)
        .pl(INSET)
        .pr(if has_actions {
            INSET - BUTTON_PADDING
        } else {
            INSET
        })
        .child(muted_text(TextStyle::BODY_MEDIUM, move || {
            count().unwrap_or_default()
        }))
        .children(updating.map(|(label, when)| {
            show(
                move || when(),
                move || {
                    let label = label.clone();
                    indeterminate_circular_progress()
                        .size(UPDATING)
                        .access(move |a| a.role(Role::ProgressBar).label(label.clone()))
                },
            )
        }))
        .child(div().flex_1())
        .children(actions)
}

/// The field that filters a list, bound to `value`. `label` says what it filters, such as "Filter
/// Albums". Escape empties it, and Enter goes on to `list`.
pub fn list_filter(value: Signal<String>, label: impl Into<String>, list: NodeRef) -> Div {
    let is_empty = move || value.with(String::is_empty);
    let clear = move || value.set(String::new());
    let field = outlined_text_field(
        value,
        TextFieldStyle::new()
            .label(label)
            .leading(sized_icon(icons::SEARCH, 24.0))
            .trailing(show(
                move || !is_empty(),
                move || {
                    icon_button(icons::CLOSE)
                        .access(|a| a.label("Clear Filter"))
                        .on_click(move |_| clear())
                },
            )),
    )
    .w_full()
    .on_key_down(move |e| {
        let m = e.modifiers;
        let plain = !(m.shift || m.ctrl || m.alt || m.logo);
        match e.key {
            Key::Named(NamedKey::Escape) if !untrack(is_empty) => clear(),
            Key::Named(NamedKey::Enter) if plain => list.focus(),
            _ => return,
        }
        e.stop_propagation();
        e.prevent_default();
    });
    div()
        .flex()
        .flex_none()
        .px(INSET)
        .pt(INSET)
        .pb(8.0_f32)
        .child(field)
}

/// Above a list that is still shown: that bringing it up to date failed, `title` saying what and
/// `reason` why, and Retry, which runs `retry`.
pub fn refresh_failure(
    title: impl Into<String>,
    reason: impl Into<String>,
    retry: impl Fn() + 'static,
) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .items_start()
        .gap(4.0_f32)
        .px(INSET)
        .pb(8.0_f32)
        .child(styled_text(TextStyle::TITLE_SMALL, title.into()))
        .child(error_text(reason.into()))
        .child(retry_button(retry).mt(4))
}

#[cfg(test)]
mod tests;
