use std::rc::Rc;

use anti_webui::prelude::*;

type IsOpen = Rc<dyn Fn() -> bool>;

/// What a screen opens over the screens, such as its own dialogs, which the menu bar's commands
/// wait for as they do for the shell's. Cheap to copy.
#[derive(Clone, Copy)]
pub struct Overlays {
    open: Signal<Vec<(usize, IsOpen)>>,
    added: Signal<usize>,
}

impl Overlays {
    pub fn new() -> Self {
        Self {
            open: signal(Vec::new()),
            added: signal(0),
        }
    }

    /// Counts what `open` says is open, until the owner this is called in goes.
    pub fn add(&self, open: impl Fn() -> bool + 'static) {
        let id = self.added.get_untracked();
        self.added.set(id + 1);
        self.open.update(|all| all.push((id, Rc::new(open))));
        let all = self.open;
        on_cleanup(move || {
            if all.is_alive() {
                all.update(|all| all.retain(|(added, _)| *added != id));
            }
        });
    }

    /// Whether any of them is open, following each.
    pub fn any(&self) -> bool {
        let all = self.open.with(Clone::clone);
        all.iter().any(|(_, open)| open())
    }
}

impl Default for Overlays {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
