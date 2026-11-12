//! Text in the styles of the type scale, in the colors of the scheme.

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::typography::TextStyle;

/// The default for text in a window: the body style, on the surface.
pub fn body<S: Styled>(styled: S) -> S {
    TextStyle::BODY_MEDIUM.apply(styled)
}

/// `content` in `style`, in the color that text on the surface has.
pub fn styled_text(style: TextStyle, content: impl IntoTextSource) -> Div {
    style
        .apply(div())
        .dyn_style(|s| s.text_color(color_scheme().on_surface))
        .child(text(content))
}

/// `content` in `style`, in the color for what is less important than the text around it.
pub fn muted_text(style: TextStyle, content: impl IntoTextSource) -> Div {
    style
        .apply(div())
        .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
        .child(text(content))
}

/// An error, in the color for errors.
pub fn error_text(content: impl IntoTextSource) -> Div {
    TextStyle::BODY_SMALL
        .apply(div())
        .dyn_style(|s| s.text_color(color_scheme().error))
        .child(text(content))
}
