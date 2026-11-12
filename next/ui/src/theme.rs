use std::cell::Cell;

use anti_webui::prelude::{Color, effect, rgb, untrack};
use anti_webui_md3e::{
    ColorScheme, SchemeOptions, SchemeVariant, motion, set_color_scheme, set_color_scheme_animated,
};
use config::Appearance;

/// The seed while nothing playing gives one.
pub const DEFAULT_SEED: Color = rgb(0x4f5bd5);

/// The scheme the app is drawn with: the content variant, which keeps a seed taken from a cover
/// recognizable.
pub fn scheme(seed: Color, dark: bool) -> ColorScheme {
    ColorScheme::from_seed_with(
        seed,
        SchemeOptions {
            variant: SchemeVariant::Content,
            dark,
            contrast: 0.0,
        },
    )
}

/// `None` follows the system.
pub fn is_dark(appearance: Option<Appearance>, system_is_dark: bool) -> bool {
    match appearance {
        Some(Appearance::Dark) => true,
        Some(Appearance::Light) => false,
        None => system_is_dark,
    }
}

/// Keeps the thread's scheme to `dark` and `seed`: at once the first time and while
/// `reduces_motion`, and else by a cross-fade. A seed of `None` is [`DEFAULT_SEED`].
pub fn follow_theme(
    dark: impl Fn() -> bool + 'static,
    seed: impl Fn() -> Option<Color> + 'static,
    reduces_motion: impl Fn() -> bool + 'static,
) {
    let first = Cell::new(true);
    effect(move || {
        let colors = scheme(seed().unwrap_or(DEFAULT_SEED), dark());
        if first.replace(false) || untrack(&reduces_motion) {
            set_color_scheme(colors);
        } else {
            set_color_scheme_animated(colors, motion::slow_effects());
        }
    });
}

#[cfg(test)]
mod tests;
