use std::sync::{Arc, Weak};

use config::{Appearance, ThemeColor};
use executor::{Executor, Saver};
use tokio::sync::watch;

use executor::{Publish, Published};

/// `None` follows the system.
pub const APPEARANCES: [(Option<Appearance>, &str); 3] = [
    (None, "System"),
    (Some(Appearance::Light), "Light"),
    (Some(Appearance::Dark), "Dark"),
];

pub const THEME_COLORS: [(ThemeColor, &str); 9] = [
    (ThemeColor::Cover, "From the cover"),
    (ThemeColor::Red, "Red"),
    (ThemeColor::Orange, "Orange"),
    (ThemeColor::Yellow, "Yellow"),
    (ThemeColor::Green, "Green"),
    (ThemeColor::Teal, "Teal"),
    (ThemeColor::Blue, "Blue"),
    (ThemeColor::Purple, "Purple"),
    (ThemeColor::Pink, "Pink"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearanceSnapshot {
    appearance: Option<Appearance>,
    theme_color: ThemeColor,
    /// What [`ThemeColor::Custom`] was given last, while it is chosen or since it was here.
    custom_color: Option<u32>,
    /// Why the appearance could not be read or saved, or why nothing here will be saved.
    appearance_error: Option<String>,
    color_error: Option<String>,
}

impl AppearanceSnapshot {
    /// `None` follows the system.
    pub fn appearance(&self) -> Option<Appearance> {
        self.appearance
    }

    pub fn theme_color(&self) -> ThemeColor {
        self.theme_color
    }

    /// The color of [`ThemeColor::Custom`] chosen last, as `0xRRGGBB`, which stays when another
    /// is chosen. `None` until one is.
    pub fn custom_color(&self) -> Option<u32> {
        self.custom_color
    }

    pub fn errors(&self) -> impl Iterator<Item = &str> {
        self.appearance_error
            .iter()
            .chain(&self.color_error)
            .map(String::as_str)
    }
}

impl Publish for AppearanceSnapshot {
    type Snapshot = Self;

    fn snapshot(&self) -> &Self {
        self
    }
}

/// The chosen appearance and theme color. Applying them to the UI is the host's.
#[derive(Clone)]
pub struct AppearanceSettings(Arc<Inner>);

struct Inner {
    /// `None` without a home directory: the choices then last until quitting.
    appearance_saver: Option<Saver<Option<Appearance>>>,
    color_saver: Option<Saver<ThemeColor>>,
    state: Published<AppearanceSnapshot>,
}

impl AppearanceSettings {
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let (store, snapshot) = read_saved(store);
        Self::with(executor, store, snapshot)
    }

    fn with(
        executor: Executor,
        store: Option<config::Store>,
        snapshot: AppearanceSnapshot,
    ) -> Self {
        Self(Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            appearance_saver: store.clone().map(|store| {
                let this = this.clone();
                Saver::new(executor.clone(), move |appearance| {
                    let saved = store.save_appearance(appearance);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.appearance_error = saved
                                .err()
                                .map(|error| format!("Couldn't save the appearance: {error}."));
                        });
                    }
                })
            }),
            color_saver: store.map(|store| {
                let this = this.clone();
                Saver::new(executor, move |color| {
                    let saved = store.save_theme_color(color);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.color_error = saved
                                .err()
                                .map(|error| format!("Couldn't save the color: {error}."));
                        });
                    }
                })
            }),
            state: Published::new(snapshot),
        }))
    }

    pub fn snapshot(&self) -> AppearanceSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling these settings,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<AppearanceSnapshot> {
        self.0.state.subscribe()
    }

    /// `None` follows the system.
    pub fn set_appearance(&self, appearance: Option<Appearance>) {
        self.0.state.update(|state| {
            if state.appearance == appearance {
                return;
            }
            state.appearance = appearance;
            // Under the lock, so that the choices reach the saver in the order they were made
            if let Some(saver) = &self.0.appearance_saver {
                saver.save(appearance);
            }
        });
    }

    pub fn set_theme_color(&self, color: ThemeColor) {
        self.0.state.update(|state| {
            if state.theme_color == color {
                return;
            }
            state.theme_color = color;
            if let ThemeColor::Custom(rgb) = color {
                state.custom_color = Some(rgb);
            }
            // Under the lock, so that the choices reach the saver in the order they were made
            if let Some(saver) = &self.0.color_saver {
                saver.save(color);
            }
        });
    }
}

/// The saved choices, and why they could not be read or will not be saved.
fn read_saved(
    store: Result<config::Store, config::Error>,
) -> (Option<config::Store>, AppearanceSnapshot) {
    let mut snapshot = AppearanceSnapshot {
        appearance: None,
        theme_color: ThemeColor::default(),
        custom_color: None,
        appearance_error: None,
        color_error: None,
    };
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            snapshot.appearance_error = Some(format!("The appearance won't be saved: {error}."));
            return (None, snapshot);
        }
    };
    match store.appearance() {
        Ok(appearance) => snapshot.appearance = appearance,
        Err(error) => {
            snapshot.appearance_error = Some(format!("Couldn't read the appearance: {error}."));
        }
    }
    match store.theme_color() {
        Ok(color) => {
            snapshot.theme_color = color;
            if let ThemeColor::Custom(rgb) = color {
                snapshot.custom_color = Some(rgb);
            }
        }
        Err(error) => snapshot.color_error = Some(format!("Couldn't read the color: {error}.")),
    }
    (Some(store), snapshot)
}

#[cfg(test)]
mod tests;
