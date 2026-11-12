//! A color of the user's own, picked by its hue, chroma and tone, the terms Material You makes its
//! schemes in, or typed in hex.

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{TextFieldStyle, outlined_text_field};
use anti_webui_md3e::dynamic::Hct;
use anti_webui_md3e::typography::TextStyle;
use next_ui::focus_ring::focus_ring;
use next_ui::theme::rgb_of;

/// Past the chroma of any color of sRGB.
const MAX_CHROMA: f32 = 120.0;
const TRACK: f32 = 16.0;
const THUMB: f32 = 24.0;
const NAMES: f32 = 64.0;
const HEX: f32 = 160.0;
/// The colors a track shows, evenly along it.
const STOPS: usize = 13;
const NOT_HEX: &str = "Use six digits, 0–9 and a–f.";

/// Where the color is in HCT. The sliders keep what they were set to, though HCT gives no more
/// chroma than the color's hue and tone have in sRGB.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    hue: f32,
    chroma: f32,
    tone: f32,
}

impl Place {
    fn of(rgb: u32) -> Self {
        let color = Hct::from_argb(0xff00_0000 | rgb);
        Self {
            hue: color.hue() as f32,
            chroma: color.chroma() as f32,
            tone: color.tone() as f32,
        }
    }

    fn color(self) -> Color {
        Hct::new(
            f64::from(self.hue),
            f64::from(self.chroma),
            f64::from(self.tone),
        )
        .to_color()
    }
}

/// One slider of [`Place`]: what it is called, and how far it goes.
#[derive(Clone, Copy)]
struct Axis {
    name: &'static str,
    max: f32,
    get: fn(&Place) -> f32,
    set: fn(&mut Place, f32),
}

const AXES: [Axis; 3] = [
    Axis {
        name: "Hue",
        max: 360.0,
        get: |p| p.hue,
        set: |p, v| p.hue = v,
    },
    Axis {
        name: "Chroma",
        max: MAX_CHROMA,
        get: |p| p.chroma,
        set: |p, v| p.chroma = v,
    },
    Axis {
        name: "Tone",
        max: 100.0,
        get: |p| p.tone,
        set: |p, v| p.tone = v,
    },
];

/// A slider for each of hue, chroma and tone, and a field for the color in hex, starting at
/// `start`, as `0xRRGGBB`. Each change gives `choose` the color at once.
pub(crate) fn color_picker(start: u32, choose: impl Fn(u32) + 'static) -> Div {
    let rgb = signal(start);
    let place = signal(Place::of(start));
    let hex = signal(format!("{start:06x}"));
    effect(move || {
        let rgb = rgb.get();
        choose(rgb);
        // What is typed stays as it is while it says this color
        if untrack(|| parse(&hex.get())) != Some(rgb) {
            hex.set(format!("{rgb:06x}"));
        }
    });
    effect(move || {
        if let Some(typed) = parse(&hex.get())
            && typed != rgb.get_untracked()
        {
            rgb.set(typed);
            place.set(Place::of(typed));
        }
    });
    // Fewer digits are on their way to a color
    let wrong = move || hex.with(|text| !could_be_hex(text));
    let field = outlined_text_field(
        hex,
        TextFieldStyle::new()
            .label("Hex")
            .prefix("#")
            .error_when(wrong)
            .supporting(move || {
                if wrong() {
                    NOT_HEX.to_owned()
                } else {
                    String::new()
                }
            }),
    )
    .w(HEX);
    div()
        .flex()
        .flex_col()
        .gap(12.0_f32)
        .w_full()
        .access(|a| a.role(Role::Group).label("Custom color"))
        .children(
            AXES.iter()
                .map(|axis| slider_line(*axis, place, rgb))
                .collect::<Vec<_>>(),
        )
        .child(field)
}

/// `0xRRGGBB` from six hex digits.
fn parse(text: &str) -> Option<u32> {
    let digits = digits(text);
    (digits.len() == 6 && could_be_hex(digits))
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

fn could_be_hex(text: &str) -> bool {
    let digits = digits(text);
    digits.len() <= 6 && digits.chars().all(|c| c.is_ascii_hexdigit())
}

/// What is typed, without the `#` that a color copied from elsewhere often has.
fn digits(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('#').unwrap_or(text)
}

fn slider_line(axis: Axis, place: Signal<Place>, rgb: Signal<u32>) -> Div {
    let value = Binding::new(
        move || place.with(axis.get),
        move |to: f32| {
            let mut moved = place.get_untracked();
            (axis.set)(&mut moved, to);
            place.set(moved);
            rgb.set(rgb_of(moved.color()));
        },
    );
    let name = TextStyle::BODY_MEDIUM
        .apply(div().w(NAMES).flex_none().child(text(axis.name)))
        .dyn_style(|s| s.text_color(color_scheme().on_surface_variant));
    let slider = slider_with(
        value,
        0.0,
        axis.max,
        SliderOptions::new().inset(THUMB / 2.0),
        move |s| look(s, axis, place, rgb),
    )
    .flex_grow(1.0_f32);
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(12.0_f32)
        .child(name)
        .child(slider)
}

/// A track of the colors the slider goes through from where the others are, and a thumb in the
/// color picked.
fn look(s: SliderState, axis: Axis, place: Signal<Place>, rgb: Signal<u32>) -> Div {
    let gradient = move || {
        let place = place.get();
        (0..STOPS).fold(Gradient::linear(0.0, 0.0, 1.0, 0.0), |gradient, i| {
            let along = i as f32 / (STOPS - 1) as f32;
            let mut stop = place;
            (axis.set)(&mut stop, along * axis.max);
            gradient.stop(along, stop.color())
        })
    };
    let colors = svg()
        .w_full()
        .h_full()
        .view_box(0.0, 0.0, 1.0, 1.0)
        .aspect(svg::SvgAspect {
            align: None,
            slice: false,
        })
        .child(svg::rect(0.0, 0.0, 1.0, 1.0).dyn_style(move |st| st.fill(gradient())));
    // From the middle of the thumb at one end to the middle of it at the other, where the pointer
    // puts the value
    let track = div()
        .absolute()
        .top((THUMB - TRACK) / 2.0)
        .left((THUMB - TRACK) / 2.0)
        .right((THUMB - TRACK) / 2.0)
        .h(TRACK)
        .rounded(TRACK / 2.0)
        .overflow_hidden()
        .child(colors)
        // The track's shape where its colors are as light or as dark as the row
        .child(
            div()
                .absolute()
                .inset(0)
                .rounded(TRACK / 2.0)
                .border(1.0_f32)
                .dyn_style(|st| st.border_color(color_scheme().outline_variant)),
        );
    let thumb = s
        .thumb(
            0,
            div()
                .flex_none()
                .w(THUMB)
                .h(THUMB)
                .rounded(THUMB / 2.0)
                .border(2.0_f32)
                .shadow_sm()
                .access(move |a| a.label(axis.name)),
        )
        .dyn_style(move |st| {
            let c = color_scheme();
            focus_ring(
                st.bg(Color::from_rgb(rgb.get()))
                    .border_color(c.surface_bright),
                s.focus_visible(),
                &c,
            )
        });
    let spacer =
        move |grow: fn(SliderState) -> f32| div().dyn_style(move |st| st.flex_grow(grow(s)));
    let rail = div()
        .absolute()
        .inset(0)
        .flex()
        .flex_row()
        .items_center()
        .child(spacer(|s| s.fraction()))
        .child(thumb)
        .child(spacer(|s| 1.0 - s.fraction()));
    div()
        .relative()
        .h(THUMB)
        .min_w(120.0_f32)
        .flex()
        .items_center()
        .cursor(CursorIcon::Pointer)
        .child(track)
        .child(rail)
}

#[cfg(test)]
mod tests;
