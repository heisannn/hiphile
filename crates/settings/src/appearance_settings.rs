use config::Appearance;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Window, WindowAppearance};
use hiphile_core::settings::AppearanceSettings;

/// Puts the theme on the chosen appearance.
pub(crate) fn apply(appearance: &AppearanceSettings, cx: &mut App) {
    let mode = theme_mode(appearance.snapshot().appearance(), cx.window_appearance());
    Theme::change(mode, None, cx);
}

/// `None` follows the system.
pub(crate) fn set_appearance(
    appearance: &AppearanceSettings,
    choice: Option<Appearance>,
    cx: &mut App,
) {
    if appearance.snapshot().appearance() == choice {
        return;
    }
    appearance.set_appearance(choice);
    apply(appearance, cx);
}

pub(crate) fn on_system_appearance_changed(
    appearance: &AppearanceSettings,
    window: &Window,
    cx: &mut App,
) {
    if appearance.snapshot().appearance().is_none() {
        Theme::change(theme_mode(None, window.appearance()), None, cx);
    }
}

fn theme_mode(appearance: Option<Appearance>, system: WindowAppearance) -> ThemeMode {
    match appearance {
        Some(Appearance::Light) => ThemeMode::Light,
        Some(Appearance::Dark) => ThemeMode::Dark,
        None => system.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_appearance_ignores_the_system() {
        for system in [WindowAppearance::Light, WindowAppearance::Dark] {
            assert_eq!(
                theme_mode(Some(Appearance::Light), system),
                ThemeMode::Light
            );
            assert_eq!(theme_mode(Some(Appearance::Dark), system), ThemeMode::Dark);
        }
    }

    #[test]
    fn no_choice_follows_the_system() {
        assert_eq!(theme_mode(None, WindowAppearance::Light), ThemeMode::Light);
        assert_eq!(theme_mode(None, WindowAppearance::Dark), ThemeMode::Dark);
        assert_eq!(
            theme_mode(None, WindowAppearance::VibrantDark),
            ThemeMode::Dark
        );
    }
}
