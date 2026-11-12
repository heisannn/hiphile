use std::rc::Rc;

use hiphile_core::navigation::Location;

/// What a screen asks of the shell, which goes elsewhere or opens what is the window's.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Show(Location),
    GoBack,
    GoForward,
    OpenSearch,
    OpenSettings,
}

/// Where a screen sends its [`Command`]s. Cheap to clone.
#[derive(Clone)]
pub struct Commands(Rc<dyn Fn(Command)>);

impl Commands {
    pub fn new(handle: impl Fn(Command) + 'static) -> Self {
        Self(Rc::new(handle))
    }

    pub fn send(&self, command: Command) {
        (self.0)(command);
    }
}
