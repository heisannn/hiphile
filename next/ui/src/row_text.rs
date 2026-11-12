//! The words of a row: its title, with what it is by or on below. AntiWebUI has no ellipsis, so
//! what does not fit is cut and its end fades, to show that there is more.

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::typography::TextStyle;

/// How far from the end what is cut fades, in logical pixels.
const FADE: f32 = 24.0;

#[derive(ShaderParams, Clone, Copy)]
struct FadeEnd {
    width: f32,
}

static FADE_END: Shader<FadeEnd> = Shader::new("fade_end", include_str!("row_text/fade_end.wgsl"));

/// The title in the color of the row, and the details below in the color for what matters less.
pub fn row_text(title: impl IntoTextSource, details: Option<String>) -> Div {
    div()
        .flex()
        .flex_col()
        .justify_center()
        .min_w(0)
        .child(faded_end(
            TextStyle::TITLE_SMALL.apply(div()).child(text(title)),
        ))
        .children(details.map(|details| {
            faded_end(
                TextStyle::BODY_MEDIUM
                    .apply(div())
                    .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
                    .child(text(details)),
            )
        }))
}

/// `line` on one line as wide as it is given: cut where it does not fit, with its end fading.
pub fn faded_end(line: Div) -> Div {
    let room = signal(0.0f32);
    let needed = signal(0.0f32);
    div()
        .flex()
        .flex_row()
        .min_w(0)
        .overflow_hidden()
        .on_resized(move |width, _| room.set_if_changed(width))
        .filter(move || {
            (needed.get() > room.get() + 0.5)
                .then(|| Image::content().through(&FADE_END, FadeEnd { width: FADE }))
        })
        .child(
            line.flex_none()
                .whitespace_nowrap()
                .on_resized(move |width, _| needed.set_if_changed(width)),
        )
}

#[cfg(test)]
mod tests;
