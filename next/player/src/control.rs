use std::cell::Cell;

use anti_webui::prelude::*;
use anti_webui_md3e::color::ColorScheme;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{Ink, sized_icon};
use anti_webui_md3e::icons::Symbol;
use anti_webui_md3e::motion::{default_effects, fast_spatial};
use next_ui::focus_ring::focus_ring;

/// How an [`icon_control`] looks, after the icon buttons of `anti_webui_md3e`, whose icon is chosen
/// once and for all.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    /// Background and icon, given whether it is on.
    colors: fn(&ColorScheme, bool) -> (Color, Color),
    height: f32,
    padding: f32,
    icon: f32,
    /// The radii of its corners at rest, pressed and on.
    rest: f32,
    pressed: f32,
    on: f32,
    spring: fn() -> Spring,
    fill: Fill,
}

/// When the icon is filled in.
#[derive(Clone, Copy, PartialEq)]
enum Fill {
    Always,
    WhenOn,
    Never,
}

fn filled(c: &ColorScheme, _: bool) -> (Color, Color) {
    (c.primary, c.on_primary)
}

fn standard(c: &ColorScheme, _: bool) -> (Color, Color) {
    (Color::TRANSPARENT, c.on_surface_variant)
}

fn standard_toggle(c: &ColorScheme, on: bool) -> (Color, Color) {
    let icon = if on { c.primary } else { c.on_surface_variant };
    (Color::TRANSPARENT, icon)
}

/// The filled icon button of the medium size, wide, which is round while it is off and a rounded
/// square while it is on. It bounces into shape as the toggle buttons do.
pub(crate) const PLAY: Look = Look {
    colors: filled,
    height: 56.0,
    padding: 24.0,
    icon: 24.0,
    rest: 28.0,
    pressed: 12.0,
    on: 16.0,
    spring: fast_spatial,
    fill: Fill::Always,
};

/// The standard icon button of the small size.
pub(crate) const STANDARD: Look = Look {
    colors: standard,
    height: 40.0,
    padding: 8.0,
    icon: 24.0,
    rest: 20.0,
    pressed: 8.0,
    on: 20.0,
    spring: default_effects,
    fill: Fill::Never,
};

/// The standard icon toggle button of the small size.
pub(crate) const STANDARD_TOGGLE: Look = Look {
    colors: standard_toggle,
    on: 12.0,
    fill: Fill::WhenOn,
    ..STANDARD
};

/// A button whose icon is what `symbol` returns, which can change while it keeps the focus. `on`
/// is the state a toggle shows, or a button that changes its shape with what it does.
pub(crate) fn icon_control(
    symbol: impl Fn() -> &'static Symbol + 'static,
    look: Look,
    on: impl Fn() -> bool + Copy + 'static,
    disabled: impl Fn() -> bool + Copy + 'static,
) -> Div {
    let Look {
        colors,
        height,
        padding,
        icon,
        rest,
        pressed,
        on: on_radius,
        spring,
        fill,
    } = look;
    let ink = Ink::new(move |c| colors(c, on()).1, disabled);
    let Ink {
        pressed: is_pressed,
        focused,
        ..
    } = ink;
    let first = if untrack(on) { on_radius } else { rest };
    let corner = animated(first);
    // What stays as it is asks for no frames
    let wanted = Cell::new(first);
    effect(move || {
        let target = if is_pressed.get() {
            pressed
        } else if on() {
            on_radius
        } else {
            rest
        };
        if wanted.replace(target) != target {
            untrack(|| corner.spring_to(target, spring()));
        }
    });
    let glyph = dyn_view(move || {
        let symbol = symbol();
        untrack(|| {
            sized_icon(symbol, icon)
                .transition_of([Prop::FontAxes], Transition::ms(200).ease_out())
                .dyn_style(move |s| {
                    let is_filled = match fill {
                        Fill::Always => true,
                        Fill::WhenOn => on(),
                        Fill::Never => false,
                    };
                    s.font_axis(b"FILL", if is_filled { 1.0 } else { 0.0 })
                })
        })
    });
    let root = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .justify_center()
        .select_none()
        .h(height)
        .px(padding)
        .cursor(CursorIcon::Pointer)
        .focusable()
        .access(|a| a.role(Role::Button))
        .dyn_style(move |s| {
            let c = color_scheme();
            let (background, content) = colors(&c, on());
            let s = s.rounded(corner.get());
            let s = if disabled() {
                let s = s
                    .text_color(c.on_surface.with_alpha(0.38))
                    .cursor(CursorIcon::Default);
                if background.is_transparent() {
                    s
                } else {
                    s.bg(c.on_surface.with_alpha(0.1))
                }
            } else {
                s.bg(background).text_color(content)
            };
            focus_ring(s, focused.get() && !disabled(), &c)
        })
        .child(glyph);
    ink.paint(ink.listen(root)).disabled(disabled)
}
