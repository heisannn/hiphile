use std::cell::Cell;

use anti_webui::prelude::{Color, Memo, Signal, effect, memo, rgb, untrack};
use anti_webui_md3e::dynamic::Hct;
use anti_webui_md3e::{
    ColorScheme, SchemeOptions, SchemeVariant, motion, set_color_scheme, set_color_scheme_animated,
};
use config::{Appearance, ThemeColor};

/// The seed of the cover's colors while nothing playing gives one.
pub const DEFAULT_SEED: Color = rgb(0x4f5bd5);

/// The scheme the app is drawn with in `color`. A picked color gives tonal spot, the scheme
/// Material You makes of a color the user picks, and the cover gives [`cover_scheme`].
/// `cover_seed` is the seed of the cover of what plays, `None` while there is none.
pub fn scheme(color: ThemeColor, cover_seed: Option<Color>, dark: bool) -> ColorScheme {
    let seed = seed(color, cover_seed);
    match color {
        ThemeColor::Cover => cover_scheme(seed, dark),
        _ => ColorScheme::from_seed(seed, dark),
    }
}

/// The colors that show `color` where it can be picked: its seed over its tertiary and a light
/// primary. Like Material's fixed roles, they are the same in light and dark, at tones that keep
/// them clear of gray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swatch {
    pub seed: Color,
    pub tertiary: Color,
    pub primary: Color,
    /// What marks the swatch, and the icon on it.
    pub mark: Color,
    pub on_mark: Color,
}

pub fn swatch(color: ThemeColor, cover_seed: Option<Color>) -> Swatch {
    let colors = scheme(color, cover_seed, false);
    let at = |color: Color, tone: f64| {
        let color = Hct::from_color(color);
        Hct::new(color.hue(), color.chroma(), tone).to_color()
    };
    Swatch {
        seed: at(seed(color, cover_seed), 70.0),
        tertiary: at(colors.tertiary, 80.0),
        primary: at(colors.primary, 90.0),
        mark: at(colors.primary, 30.0),
        on_mark: at(colors.primary, 90.0),
    }
}

/// A picked color's seed is its hue at a chroma that shows clearly in a swatch: tonal spot takes
/// nothing but the hue.
fn seed(color: ThemeColor, cover_seed: Option<Color>) -> Color {
    match picked_hue(color) {
        Some(hue) => Hct::new(hue, 48.0, 50.0).to_color(),
        None => cover_seed.unwrap_or(DEFAULT_SEED),
    }
}

/// The scheme of the colors of a cover: the content variant, which keeps a seed taken from a cover
/// recognizable.
pub fn cover_scheme(seed: Color, dark: bool) -> ColorScheme {
    ColorScheme::from_seed_with(
        seed,
        SchemeOptions {
            variant: SchemeVariant::Content,
            dark,
            contrast: 0.0,
        },
    )
}

/// Where each color that can be picked is on HCT's circle of hues.
fn picked_hue(color: ThemeColor) -> Option<f64> {
    let hue = match color {
        ThemeColor::Cover => return None,
        ThemeColor::Red => 25.0,
        ThemeColor::Orange => 50.0,
        ThemeColor::Yellow => 90.0,
        ThemeColor::Green => 145.0,
        ThemeColor::Teal => 185.0,
        ThemeColor::Blue => 255.0,
        ThemeColor::Purple => 300.0,
        ThemeColor::Pink => 350.0,
    };
    Some(hue)
}

/// What the window's scheme is made of besides the color chosen. Cheap to copy.
#[derive(Clone, Copy)]
pub struct Schemes {
    is_dark: Memo<bool>,
    cover_seed: Signal<Option<Color>>,
}

impl Schemes {
    /// `cover_seed` is the seed of the cover of what plays, `None` while there is none.
    pub fn new(is_dark: impl Fn() -> bool + 'static, cover_seed: Signal<Option<Color>>) -> Self {
        Self {
            is_dark: memo(is_dark),
            cover_seed,
        }
    }

    /// The scheme that `color` gives the window now. Reading it tracks it.
    pub fn of(&self, color: ThemeColor) -> ColorScheme {
        scheme(color, self.cover_seed_of(color), self.is_dark.get())
    }

    /// The [`Swatch`] of `color` now. Reading it tracks it.
    pub fn swatch(&self, color: ThemeColor) -> Swatch {
        swatch(color, self.cover_seed_of(color))
    }

    /// Only the cover's colors read it, and change with what plays.
    fn cover_seed_of(&self, color: ThemeColor) -> Option<Color> {
        match color {
            ThemeColor::Cover => self.cover_seed.get(),
            _ => None,
        }
    }
}

/// `None` follows the system.
pub fn is_dark(appearance: Option<Appearance>, system_is_dark: bool) -> bool {
    match appearance {
        Some(Appearance::Dark) => true,
        Some(Appearance::Light) => false,
        None => system_is_dark,
    }
}

/// Keeps the thread's scheme to what `scheme` gives: at once the first time and while
/// `reduces_motion`, and else by a cross-fade.
pub fn follow_theme(
    scheme: impl Fn() -> ColorScheme + 'static,
    reduces_motion: impl Fn() -> bool + 'static,
) {
    let first = Cell::new(true);
    effect(move || {
        let colors = scheme();
        if first.replace(false) || untrack(&reduces_motion) {
            set_color_scheme(colors);
        } else {
            set_color_scheme_animated(colors, motion::slow_effects());
        }
    });
}

#[cfg(test)]
mod tests;
