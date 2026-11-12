use anti_webui::prelude::*;
use hiphile_core::navigation::Destination;
use next_platform::menu::{Item, ItemState, Menu, Shortcut, Standard};
use next_ui::{Command, Commands};

use crate::navigation::location_of;

/// The menu bar's side of the shell: where what is chosen in it goes, and what it shows of the
/// screens. Cheap to copy.
#[derive(Clone, Copy)]
pub struct MenuBar {
    shown: Signal<Shown>,
    commands: Signal<Option<Commands>>,
}

/// What the menu bar shows of the screens.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Shown {
    /// Where the window is, while someone is signed in.
    pub(crate) destination: Option<Destination>,
    pub(crate) can_go_back: bool,
    pub(crate) can_go_forward: bool,
    /// Something is open over the screens, such as the settings.
    pub(crate) covered: bool,
}

impl MenuBar {
    pub fn new() -> Self {
        Self {
            shown: signal(Shown::default()),
            commands: signal(None),
        }
    }

    /// Sends `command` to the shell of whoever is signed in, if it can be chosen now.
    pub fn choose(&self, command: Command) {
        if !untrack(|| self.state_of(&command)).is_enabled() {
            return;
        }
        if let Some(commands) = self.commands.get_untracked() {
            commands.send(command);
        }
    }

    /// Whether the item of `command` can be chosen now, and whether it is checked.
    pub fn state_of(&self, command: &Command) -> ItemState {
        self.shown.with(|shown| state_of(shown, command))
    }

    /// Lets the menu bar send `commands` and show `shown` until the owner this is called in goes.
    pub(crate) fn attach(&self, commands: Commands, shown: impl Fn() -> Shown + 'static) {
        let Self {
            shown: showing,
            commands: sending,
        } = *self;
        sending.set(Some(commands));
        effect(move || showing.set_if_changed(shown()));
        on_cleanup(move || {
            // When the window goes, its owner removes the menu bar's signals before any cleanup
            if !sending.is_alive() {
                return;
            }
            sending.set(None);
            showing.set_if_changed(Shown::default());
        });
    }
}

impl Default for MenuBar {
    fn default() -> Self {
        Self::new()
    }
}

/// What the menu bar holds: hiphile, Library, View and Controls.
pub fn menus() -> Vec<Menu<Command>> {
    let destinations = Destination::ALL
        .iter()
        .zip('1'..)
        .map(|(destination, key)| {
            Item::command(
                destination.label(),
                Command::Show(location_of(*destination)),
                Shortcut::command(key),
            )
        });
    vec![
        Menu::new(
            "hiphile",
            vec![
                Item::command("Settings…", Command::OpenSettings, Shortcut::command(',')),
                Item::Separator,
                Item::standard("Hide hiphile", Standard::Hide),
                Item::standard("Hide Others", Standard::HideOthers),
                Item::standard("Show All", Standard::ShowAll),
                Item::Separator,
                Item::standard("Quit hiphile", Standard::Quit),
            ],
        ),
        Menu::new("Library", Vec::new()),
        Menu::new(
            "View",
            [
                Item::command("Search…", Command::OpenSearch, Shortcut::command('f')),
                Item::Separator,
                Item::command("Back", Command::GoBack, Shortcut::command('[')),
                Item::command("Forward", Command::GoForward, Shortcut::command(']')),
                Item::Separator,
            ]
            .into_iter()
            .chain(destinations)
            .collect(),
        ),
        Menu::new("Controls", Vec::new()),
    ]
}

/// Nothing goes while no one is signed in or something covers the screens: the menu bar's keys
/// come before the window's, so a disabled item is what keeps them from what is open.
fn state_of(shown: &Shown, command: &Command) -> ItemState {
    let free = shown.destination.is_some() && !shown.covered;
    match command {
        Command::Show(location) => {
            ItemState::enabled(free).checked(shown.destination == Some(location.destination()))
        }
        Command::GoBack => ItemState::enabled(free && shown.can_go_back),
        Command::GoForward => ItemState::enabled(free && shown.can_go_forward),
        _ => ItemState::enabled(free),
    }
}

#[cfg(test)]
mod tests;
