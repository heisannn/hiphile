use std::sync::{Arc, Weak};

use config::LyricsHighlight;
use executor::{Executor, Saver};
use tokio::sync::watch;

use executor::{Publish, Published};

pub const LYRICS_HIGHLIGHTS: [(LyricsHighlight, &str); 2] = [
    (LyricsHighlight::ByWord, "By word"),
    (LyricsHighlight::ByLine, "By line"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricsSnapshot {
    highlight: LyricsHighlight,
    error: Option<String>,
}

impl LyricsSnapshot {
    pub fn highlight(&self) -> LyricsHighlight {
        self.highlight
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Publish for LyricsSnapshot {
    type Snapshot = Self;

    fn snapshot(&self) -> &Self {
        self
    }
}

#[derive(Clone)]
pub struct LyricsSettings(Arc<Inner>);

struct Inner {
    /// `None` without a home directory: the choice then lasts until quitting.
    saver: Option<Saver<LyricsHighlight>>,
    state: Published<LyricsSnapshot>,
}

impl LyricsSettings {
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let (store, highlight, error) = read_saved_highlight(store);
        Self::with(executor, store, highlight, error)
    }

    fn with(
        executor: Executor,
        store: Option<config::Store>,
        highlight: LyricsHighlight,
        error: Option<String>,
    ) -> Self {
        Self(Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            saver: store.map(|store| {
                let this = this.clone();
                Saver::new(executor, move |highlight| {
                    let saved = store.save_lyrics_highlight(highlight);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.error = saved.err().map(|error| {
                                format!("Couldn't save the lyrics highlight: {error}.")
                            });
                        });
                    }
                })
            }),
            state: Published::new(LyricsSnapshot { highlight, error }),
        }))
    }

    pub fn snapshot(&self) -> LyricsSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling these settings,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<LyricsSnapshot> {
        self.0.state.subscribe()
    }

    pub fn set_highlight(&self, highlight: LyricsHighlight) {
        self.0.state.update(|state| {
            if state.highlight == highlight {
                return;
            }
            state.highlight = highlight;
            // Under the lock, so that the choices reach the saver in the order they were made
            if let Some(saver) = &self.0.saver {
                saver.save(highlight);
            }
        });
    }
}

/// The saved highlight, and why it could not be read or will not be saved.
fn read_saved_highlight(
    store: Result<config::Store, config::Error>,
) -> (Option<config::Store>, LyricsHighlight, Option<String>) {
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            let error = format!("The lyrics highlight won't be saved: {error}.");
            return (None, LyricsHighlight::default(), Some(error));
        }
    };
    match store.lyrics_highlight() {
        Ok(highlight) => (Some(store), highlight, None),
        Err(error) => {
            let error = format!("Couldn't read the lyrics highlight: {error}.");
            (Some(store), LyricsHighlight::default(), Some(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn a_new_choice_is_told_and_saved_and_the_same_one_is_neither(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let executor = cx.update(|cx| gpui_host::executor(cx));
        let settings =
            LyricsSettings::with(executor, Some(store.clone()), LyricsHighlight::ByWord, None);
        let told = settings.subscribe();

        settings.set_highlight(LyricsHighlight::ByWord);
        cx.run_until_parked();
        assert!(!told.has_changed().unwrap());
        assert!(!store.config_path().exists());

        settings.set_highlight(LyricsHighlight::ByLine);
        cx.run_until_parked();
        assert!(told.has_changed().unwrap());
        assert_eq!(store.lyrics_highlight().unwrap(), LyricsHighlight::ByLine);
        let settings = settings.snapshot();
        assert_eq!(settings.highlight(), LyricsHighlight::ByLine);
        assert!(settings.error().is_none());
    }
}
