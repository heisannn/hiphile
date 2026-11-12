use std::sync::Arc;

use anti_webui::prelude::*;
use hiphile_core::navigation::Destination;
use next_platform::menu::{Item, ItemState, Key, Menu, Shortcut, Standard};
use next_ui::{Command, Commands};
use playback::RepeatMode;

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
    /// Whether a window takes what is chosen, whether someone is signed in or not.
    pub(crate) attached: bool,
    /// Where the window is, while someone is signed in.
    pub(crate) destination: Option<Destination>,
    pub(crate) can_go_back: bool,
    pub(crate) can_go_forward: bool,
    /// Something is open over the screens, such as the settings.
    pub(crate) covered: bool,
    /// There is a song to play, pause, skip or seek.
    pub(crate) has_song: bool,
    pub(crate) has_queue: bool,
    pub(crate) shuffled: bool,
    /// What repeats, while someone is signed in.
    pub(crate) repeat: Option<RepeatMode>,
    pub(crate) queue_fits: bool,
    pub(crate) queue_shown: bool,
    pub(crate) lyrics_open: bool,
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
        Menu::new(
            "Library",
            vec![
                Item::command("Refresh Library", Command::Refresh, Shortcut::command('r')),
                Item::Separator,
                Item::command(
                    "New Playlist…",
                    Command::NewPlaylist(Arc::from([])),
                    Shortcut::command('n'),
                ),
                Item::command("Save Queue as Playlist…", Command::SaveQueue, None),
            ],
        ),
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
            .chain([
                Item::Separator,
                Item::command(
                    "Queue",
                    Command::ToggleQueue,
                    Shortcut::command('u').option(),
                ),
                Item::command(
                    "Lyrics",
                    Command::ToggleLyrics,
                    Shortcut::command('l').option(),
                ),
            ])
            .collect(),
        ),
        Menu::new("Controls", controls()),
    ]
}

fn controls() -> Vec<Item<Command>> {
    let repeat = [
        ("Off", RepeatMode::Off),
        ("All", RepeatMode::All),
        ("One", RepeatMode::One),
    ]
    .into_iter()
    .map(|(title, mode)| Item::command(title, Command::SetRepeat(mode), None))
    .collect();
    vec![
        Item::command(
            "Play/Pause",
            Command::TogglePlayback,
            Shortcut::command('p').option(),
        ),
        Item::command("Next", Command::Next, Shortcut::command(Key::Right)),
        Item::command("Previous", Command::Previous, Shortcut::command(Key::Left)),
        Item::Separator,
        Item::command(
            "Shuffle",
            Command::ToggleShuffle,
            Shortcut::command('s').option(),
        ),
        Item::Submenu(Menu::new("Repeat", repeat)),
        Item::Separator,
        Item::command(
            "Seek Forward",
            Command::SeekForward,
            Shortcut::command(Key::Right).shift(),
        ),
        Item::command(
            "Seek Backward",
            Command::SeekBackward,
            Shortcut::command(Key::Left).shift(),
        ),
        Item::Separator,
        Item::command("Volume Up", Command::VolumeUp, Shortcut::command(Key::Up)),
        Item::command(
            "Volume Down",
            Command::VolumeDown,
            Shortcut::command(Key::Down),
        ),
        Item::command(
            "Mute",
            Command::ToggleMute,
            Shortcut::command(Key::Down).option(),
        ),
    ]
}

/// Nothing goes while something covers the screens, and only the settings while no one is signed
/// in: the menu bar's keys come before the window's, so a disabled item is what keeps them from
/// what is open.
fn state_of(shown: &Shown, command: &Command) -> ItemState {
    let free = shown.destination.is_some() && !shown.covered;
    match command {
        Command::OpenSettings => ItemState::enabled(shown.attached && !shown.covered),
        Command::Show(location) => {
            ItemState::enabled(free).checked(shown.destination == Some(location.destination()))
        }
        // Back closes the lyrics first
        Command::GoBack => ItemState::enabled(free && (shown.can_go_back || shown.lyrics_open)),
        Command::GoForward => {
            ItemState::enabled(free && shown.can_go_forward && !shown.lyrics_open)
        }
        Command::TogglePlayback
        | Command::Next
        | Command::Previous
        | Command::SeekForward
        | Command::SeekBackward => ItemState::enabled(free && shown.has_song),
        Command::SaveQueue => ItemState::enabled(free && shown.has_queue),
        Command::ToggleQueue => {
            ItemState::enabled(free && shown.queue_fits).checked(shown.queue_shown)
        }
        Command::ToggleLyrics => {
            ItemState::enabled(free && shown.has_song).checked(shown.lyrics_open)
        }
        Command::ToggleShuffle => ItemState::enabled(free).checked(shown.shuffled),
        Command::SetRepeat(repeat) => {
            ItemState::enabled(free).checked(shown.repeat == Some(*repeat))
        }
        _ => ItemState::enabled(free),
    }
}

#[cfg(test)]
mod tests;
