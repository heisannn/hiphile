//! The colors the window can take, as Material You offers them: for each, a swatch of the scheme
//! it gives, the cover's first.

use anti_webui::prelude::*;
use anti_webui_md3e::components::{Ink, sized_icon, tooltip};
use anti_webui_md3e::{ColorScheme, color_scheme, icons};
use config::ThemeColor;
use hiphile_core::settings::THEME_COLORS;
use next_ui::focus_ring::focus_ring;
use next_ui::theme::Schemes;

const SWATCH: f32 = 40.0;
/// The chosen swatch squares off, as a selected toggle of Material 3 Expressive does.
const CHOSEN_CORNER: f32 = 12.0;
/// The disc in the middle that marks the chosen swatch, and the cover's.
const MARK: f32 = 24.0;
const MARK_ICON: f32 = 18.0;

/// A radio button for each of [`THEME_COLORS`], the one that `chosen` reads checked. Choosing
/// another asks `choose` for it. The group is one stop for Tab, and the arrow keys choose.
pub(crate) fn swatches(
    chosen: impl Fn() -> ThemeColor + 'static,
    choose: impl Fn(ThemeColor) + 'static,
    schemes: Schemes,
) -> Div {
    let chosen = memo(chosen);
    let values = THEME_COLORS.iter().map(|(color, _)| *color).collect();
    radio_group_with(
        Binding::new(move || chosen.get(), choose),
        values,
        move |group| {
            div().flex().flex_row().flex_wrap().gap(8.0_f32).children(
                THEME_COLORS
                    .iter()
                    .enumerate()
                    .map(|(ix, (color, name))| swatch(group, ix, *color, name, schemes))
                    .collect::<Vec<_>>(),
            )
        },
    )
    .access(|a| a.label("Color"))
}

/// The primary role over the secondary and the tertiary, as Android draws a scheme to pick.
fn swatch(
    group: RadioGroupState<ThemeColor>,
    ix: usize,
    color: ThemeColor,
    name: &'static str,
    schemes: Schemes,
) -> Div {
    let colors = memo(move || schemes.of(color));
    let chosen = memo(move || group.is_selected(ix));
    let corner = move || {
        if chosen.get() {
            CHOSEN_CORNER
        } else {
            SWATCH / 2.0
        }
    };
    let part = move |w: f32, role: fn(&ColorScheme) -> Color| {
        div()
            .w(w)
            .h(SWATCH / 2.0)
            .dyn_style(move |s| s.bg(role(&colors.get())))
    };
    let marked = move || chosen.get() || color == ThemeColor::Cover;
    let mark = div()
        .absolute()
        .inset(0)
        .flex()
        .items_center()
        .justify_center()
        .pointer_events_none()
        .child(show(marked, move || {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(MARK)
                .h(MARK)
                .rounded(MARK / 2.0)
                .dyn_style(move |s| {
                    let c = colors.get();
                    s.bg(c.primary_container).text_color(c.on_primary_container)
                })
                .child(dyn_view(move || {
                    let icon = if chosen.get() {
                        icons::CHECK
                    } else {
                        icons::ALBUM
                    };
                    untrack(|| sized_icon(icon, MARK_ICON))
                }))
        }));
    let ink = Ink::new(|c| c.on_surface, || false);
    // Over the colors, which would hide a state layer drawn under the children
    let layer = ink.paint(div().absolute().inset(0).pointer_events_none());
    let swatch = div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(SWATCH)
        .h(SWATCH)
        // The corners of a box are no rounder than half its shorter side, so the parts are cut
        // out of the round swatch rather than rounded themselves
        .overflow_hidden()
        .cursor(CursorIcon::Pointer)
        .access(move |a| a.label(name))
        .dyn_style(move |s| {
            let keyed = group.focus_visible() && chosen.get();
            focus_ring(s.rounded(corner()), keyed, &color_scheme())
        })
        .child(part(SWATCH, |c| c.primary))
        .child(
            div()
                .flex()
                .flex_row()
                .child(part(SWATCH / 2.0, |c| c.secondary))
                .child(part(SWATCH / 2.0, |c| c.tertiary)),
        )
        .child(mark)
        .child(layer);
    let swatch = ink.listen(group.item(ix, swatch));
    match color {
        // The only one whose look does not say what it is
        ThemeColor::Cover => tooltip(swatch, name),
        _ => swatch,
    }
}

#[cfg(test)]
mod tests;
