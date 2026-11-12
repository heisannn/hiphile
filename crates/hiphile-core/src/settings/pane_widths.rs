use std::collections::HashMap;
use std::sync::Arc;

use config::Pane;
use executor::{Executor, Publish, Published, Saver};

/// The widths the user gave the panes, in logical pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneWidthsSnapshot {
    widths: HashMap<Pane, f32>,
}

impl PaneWidthsSnapshot {
    /// `None` until the user gives `pane` a width, which leaves it at the host's own.
    pub fn width(&self, pane: Pane) -> Option<f32> {
        self.widths.get(&pane).copied()
    }
}

impl Publish for PaneWidthsSnapshot {
    type Snapshot = Self;

    fn snapshot(&self) -> &Self {
        self
    }
}

/// The widths the user gave the panes they make wider or narrower, which last from one launch to
/// the next. Fitting them to the window is the host's. A width written so that it can't be read is
/// left at the host's own, and one that can't be saved lasts until quitting: nothing tells of
/// either.
#[derive(Clone)]
pub struct PaneWidths(Arc<Inner>);

struct Inner {
    /// `None` without a home directory.
    saver: Option<Saver<HashMap<Pane, f32>>>,
    state: Published<PaneWidthsSnapshot>,
}

impl PaneWidths {
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let store = store.ok();
        let widths = store.as_ref().map(read_saved).unwrap_or_default();
        Self(Arc::new(Inner {
            saver: store.map(|store| {
                Saver::new(executor, move |widths: HashMap<Pane, f32>| {
                    for (pane, width) in widths {
                        let _ = store.save_pane_width(pane, width);
                    }
                })
            }),
            state: Published::new(PaneWidthsSnapshot { widths }),
        }))
    }

    pub fn snapshot(&self) -> PaneWidthsSnapshot {
        self.0.state.snapshot()
    }

    pub fn set_width(&self, pane: Pane, width: f32) {
        self.0.state.update(|state| {
            if state.widths.get(&pane) == Some(&width) {
                return;
            }
            state.widths.insert(pane, width);
            // Under the lock, so that the widths reach the saver in the order they were given
            if let Some(saver) = &self.0.saver {
                saver.save_with(|waiting| {
                    let mut waiting = waiting.unwrap_or_default();
                    waiting.insert(pane, width);
                    waiting
                });
            }
        });
    }
}

fn read_saved(store: &config::Store) -> HashMap<Pane, f32> {
    Pane::ALL
        .into_iter()
        .filter_map(|pane| Some((pane, store.pane_width(pane).ok()??)))
        .collect()
}

#[cfg(test)]
mod tests {
    use executor::testing::TestExecutor;

    use super::*;

    #[test]
    fn a_width_given_is_saved_and_read_at_the_next_launch() {
        let cx = &TestExecutor::new();
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let widths = PaneWidths::new(cx.executor(), Ok(store.clone()));
        assert_eq!(widths.snapshot().width(Pane::AlbumList), None);

        widths.set_width(Pane::AlbumList, 352.0);
        widths.set_width(Pane::PlaylistList, 300.0);
        cx.run_until_parked();

        assert_eq!(widths.snapshot().width(Pane::AlbumList), Some(352.0));
        let next = PaneWidths::new(cx.executor(), Ok(store));
        assert_eq!(next.snapshot().width(Pane::AlbumList), Some(352.0));
        assert_eq!(next.snapshot().width(Pane::PlaylistList), Some(300.0));
        assert_eq!(next.snapshot().width(Pane::ArtistList), None);
    }

    #[test]
    fn a_width_that_cant_be_read_is_left_out() {
        let cx = &TestExecutor::new();
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        std::fs::write(
            store.config_path(),
            "album_list_width = \"wide\"\nartist_list_width = 400\n",
        )
        .unwrap();

        let widths = PaneWidths::new(cx.executor(), Ok(store)).snapshot();

        assert_eq!(widths.width(Pane::AlbumList), None);
        assert_eq!(widths.width(Pane::ArtistList), Some(400.0));
    }
}
