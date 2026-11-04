use config::Appearance;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{AppContext as _, Context, SharedString, Task, Window, WindowAppearance};

/// The choices in the order they show. `None` follows the system.
pub(crate) const CHOICES: [(Option<Appearance>, &str); 3] = [
    (None, "System"),
    (Some(Appearance::Light), "Light"),
    (Some(Appearance::Dark), "Dark"),
];

/// Whether the app is light or dark, which it keeps the theme on. Saving
/// happens off the main thread.
pub(crate) struct AppearanceSettings {
    /// `None` without a home directory: the choice then lasts until quitting.
    store: Option<config::Store>,
    /// `None` follows the system.
    appearance: Option<Appearance>,
    error: Option<SharedString>,
    /// Whether the user has changed `appearance` since it was last saved. Only
    /// then is it written: it may have come from the environment, or from a
    /// file that could not be read.
    unsaved: bool,
    saving: bool,
    _save: Task<()>,
}

impl AppearanceSettings {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let (store, appearance, error) = read_saved_appearance();
        Theme::change(theme_mode(appearance, cx.window_appearance()), None, cx);
        Self {
            store,
            appearance,
            error,
            unsaved: false,
            saving: false,
            _save: Task::ready(()),
        }
    }

    pub(crate) fn appearance(&self) -> Option<Appearance> {
        self.appearance
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(crate) fn set_appearance(
        &mut self,
        appearance: Option<Appearance>,
        cx: &mut Context<Self>,
    ) {
        if self.appearance == appearance {
            return;
        }
        self.appearance = appearance;
        self.unsaved = true;
        Theme::change(theme_mode(appearance, cx.window_appearance()), None, cx);
        self.save(cx);
        cx.notify();
    }

    /// Moves the theme to the system's new appearance, if it is being followed.
    pub(crate) fn on_system_appearance_changed(&self, window: &Window, cx: &mut Context<Self>) {
        if self.appearance.is_none() {
            Theme::change(theme_mode(None, window.appearance()), None, cx);
        }
    }

    /// One write at a time, so a slow one cannot leave an older choice on
    /// disk; the latest is written when the current write finishes.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.saving {
            return;
        }
        self.saving = true;
        self._save = cx.spawn(async move |this, cx| {
            while let Ok(Some(appearance)) = this.update(cx, |this, _| {
                this.saving = std::mem::take(&mut this.unsaved);
                this.saving.then_some(this.appearance)
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move { store.save_appearance(appearance) })
                    .await;
                this.update(cx, |this, cx| {
                    this.error = saved
                        .err()
                        .map(|error| format!("Couldn't save the appearance: {error}.").into());
                    cx.notify();
                })
                .ok();
            }
        });
    }
}

fn theme_mode(appearance: Option<Appearance>, system: WindowAppearance) -> ThemeMode {
    match appearance {
        Some(Appearance::Light) => ThemeMode::Light,
        Some(Appearance::Dark) => ThemeMode::Dark,
        None => system.into(),
    }
}

/// The saved appearance, and why it could not be read or will not be saved.
fn read_saved_appearance() -> (
    Option<config::Store>,
    Option<Appearance>,
    Option<SharedString>,
) {
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            let error = format!("The appearance won't be saved: {error}.").into();
            return (None, None, Some(error));
        }
    };
    match store.appearance() {
        Ok(appearance) => (Some(store), appearance, None),
        Err(error) => {
            let error = format!("Couldn't read the appearance: {error}.").into();
            (Some(store), None, Some(error))
        }
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
