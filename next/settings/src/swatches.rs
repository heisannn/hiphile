//! The colors the window can take, as Material You offers them: for each, a swatch of the scheme
//! it gives, the cover's first, and last a wheel for a color of the user's own.

use std::f64::consts::TAU;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{Ink, sized_icon, tooltip};
use anti_webui_md3e::dynamic::Hct;
use anti_webui_md3e::icons::{self, Symbol};
use config::ThemeColor;
use hiphile_core::settings::THEME_COLORS;
use next_ui::focus_ring::focus_ring;
use next_ui::theme::{Schemes, Swatch, rgb_of};

pub(crate) const CUSTOM: &str = "Custom";

const SWATCH: f32 = 40.0;
/// The chosen swatch squares off, as a selected toggle of Material 3 Expressive does.
const CHOSEN_CORNER: f32 = 12.0;
/// The disc in the middle that marks the chosen swatch, and the cover's.
const MARK: f32 = 24.0;
const MARK_ICON: f32 = 18.0;
/// The room between the custom color's disc and the wheel around it.
const GAP: f32 = 2.0;
const WEDGES: usize = 36;

/// What a swatch stands for: the radio group's values are fixed when it is made, and a custom
/// color can be any.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    Named(ThemeColor),
    Custom,
}

impl Slot {
    fn of(color: ThemeColor) -> Self {
        match color {
            ThemeColor::Custom(_) => Self::Custom,
            named => Self::Named(named),
        }
    }
}

/// A radio button for each of [`THEME_COLORS`], and one for [`ThemeColor::Custom`], the one that
/// `chosen` reads checked. Choosing another asks `choose` for it. The custom one gives the color
/// that `custom` reads, or until there is one, the color of the scheme chosen before. The group
/// is one stop for Tab, and the arrow keys choose.
pub(crate) fn swatches(
    chosen: impl Fn() -> ThemeColor + 'static,
    custom: impl Fn() -> Option<u32> + 'static,
    choose: impl Fn(ThemeColor) + 'static,
    schemes: Schemes,
) -> Div {
    let chosen = memo(chosen);
    let custom = memo(move || {
        custom().unwrap_or_else(|| match chosen.get() {
            ThemeColor::Custom(rgb) => rgb,
            other => rgb_of(schemes.seed(other)),
        })
    });
    let values = THEME_COLORS
        .iter()
        .map(|(color, _)| Slot::Named(*color))
        .chain([Slot::Custom])
        .collect();
    radio_group_with(
        Binding::new(
            move || Slot::of(chosen.get()),
            move |slot| {
                choose(match slot {
                    Slot::Named(color) => color,
                    Slot::Custom => ThemeColor::Custom(untrack(|| custom.get())),
                })
            },
        ),
        values,
        move |group| {
            let named = THEME_COLORS
                .iter()
                .enumerate()
                .map(|(ix, (color, name))| named_swatch(group, ix, *color, name, schemes));
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(8.0_f32)
                .children(named.collect::<Vec<_>>())
                .child(custom_swatch(group, THEME_COLORS.len(), custom))
        },
    )
    .access(|a| a.label("Color"))
}

fn named_swatch(
    group: RadioGroupState<Slot>,
    ix: usize,
    color: ThemeColor,
    name: &'static str,
    schemes: Schemes,
) -> Div {
    let colors = memo(move || schemes.swatch(color));
    let chosen = memo(move || group.is_selected(ix));
    let part = move |w: f32, role: fn(&Swatch) -> Color| {
        div()
            .w(w)
            .h(SWATCH / 2.0)
            .dyn_style(move |s| s.bg(role(&colors.get())))
    };
    let parts = div()
        .flex()
        .flex_col()
        .child(part(SWATCH, |c| c.seed))
        .child(
            div()
                .flex()
                .flex_row()
                .child(part(SWATCH / 2.0, |c| c.tertiary))
                .child(part(SWATCH / 2.0, |c| c.primary)),
        );
    let marked = move || chosen.get() || color == ThemeColor::Cover;
    let mark = show(marked, move || {
        mark(
            MARK,
            move || {
                let c = colors.get();
                (c.mark, c.on_mark)
            },
            move || {
                Some(if chosen.get() {
                    icons::CHECK
                } else {
                    icons::ALBUM
                })
            },
        )
    });
    let swatch = frame(group, ix, name, chosen, parts, mark);
    match color {
        // The only one whose look does not say what it is
        ThemeColor::Cover => tooltip(swatch, name),
        _ => swatch,
    }
}

/// A wheel of hues around the custom color, which `rgb` reads.
fn custom_swatch(group: RadioGroupState<Slot>, ix: usize, rgb: Memo<u32>) -> Div {
    let chosen = memo(move || group.is_selected(ix));
    let disc = memo(move || {
        let color = Hct::from_argb(0xff00_0000 | rgb.get());
        let on = if color.tone() > 60.0 { 10.0 } else { 95.0 };
        let on = Hct::new(color.hue(), color.chroma().min(16.0), on).to_color();
        (Color::from_rgb(rgb.get()), on)
    });
    let ring = div()
        .flex()
        .items_center()
        .justify_center()
        .w(MARK + GAP * 2.0)
        .h(MARK + GAP * 2.0)
        .rounded(MARK / 2.0 + GAP)
        .dyn_style(|s| s.bg(color_scheme().surface_bright))
        .child(mark(
            MARK,
            move || disc.get(),
            move || chosen.get().then_some(icons::CHECK),
        ));
    frame(group, ix, CUSTOM, chosen, wheel(), ring)
}

/// The hues of HCT around the middle, as light and as colorful as the swatches' seeds.
fn wheel() -> Svg {
    let middle = f64::from(SWATCH) / 2.0;
    // Past the corners, so that the wheel fills the square the chosen swatch becomes
    let far = middle * 2.0;
    let at = |turn: f64| {
        let angle = turn * TAU - TAU / 4.0;
        (middle + far * angle.cos(), middle + far * angle.sin())
    };
    let wedges = (0..WEDGES).map(|i| {
        let from = i as f64 / WEDGES as f64;
        // A little over the next wedge, so that no seam shows between them
        let to = (i as f64 + 1.5) / WEDGES as f64;
        let hue = (i as f64 + 0.5) * 360.0 / WEDGES as f64;
        svg::polygon([(middle, middle), at(from), at(to)])
            .fill(Hct::new(hue, 48.0, 70.0).to_color())
    });
    svg()
        .w(SWATCH)
        .h(SWATCH)
        .children(wedges.collect::<Vec<_>>())
}

/// A disc of `size` in the colors `colors` reads, with the icon `icon` reads on it.
fn mark(
    size: f32,
    colors: impl Fn() -> (Color, Color) + 'static,
    icon: impl Fn() -> Option<&'static Symbol> + 'static,
) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .w(size)
        .h(size)
        .rounded(size / 2.0)
        .dyn_style(move |s| {
            let (bg, on) = colors();
            s.bg(bg).text_color(on)
        })
        .child(dyn_view(move || {
            let icon = icon();
            untrack(|| {
                icon.map(|icon| sized_icon(icon, MARK_ICON))
                    .unwrap_or_else(div)
            })
        }))
}

/// A swatch that shows `look` with `mark` over it, the radio button `ix` of `group`.
fn frame(
    group: RadioGroupState<Slot>,
    ix: usize,
    name: &'static str,
    chosen: Memo<bool>,
    look: impl IntoView,
    mark: impl IntoView,
) -> Div {
    let corner = move || {
        if chosen.get() {
            CHOSEN_CORNER
        } else {
            SWATCH / 2.0
        }
    };
    let mark = div()
        .absolute()
        .inset(0)
        .flex()
        .items_center()
        .justify_center()
        .pointer_events_none()
        .child(mark);
    let ink = Ink::new(|c| c.on_surface, || false);
    // Over the colors, which would hide a state layer drawn under the children
    let layer = ink.paint(div().absolute().inset(0).pointer_events_none());
    let swatch = div()
        .relative()
        .flex_none()
        .w(SWATCH)
        .h(SWATCH)
        // The corners of a box are no rounder than half its shorter side, so the look is cut out
        // of the round swatch rather than rounded itself
        .overflow_hidden()
        .cursor(CursorIcon::Pointer)
        .access(move |a| a.label(name))
        .dyn_style(move |s| {
            let keyed = group.focus_visible() && chosen.get();
            focus_ring(s.rounded(corner()), keyed, &color_scheme())
        })
        .child(look)
        .child(mark)
        .child(layer);
    ink.listen(group.item(ix, swatch))
}

#[cfg(test)]
mod tests;
