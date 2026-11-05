use config::LyricsHighlight;
use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Task};

pub(crate) const CHOICES: [(LyricsHighlight, &str); 2] = [
    (LyricsHighlight::ByWord, "By word"),
    (LyricsHighlight::ByLine, "By line"),
];

pub struct LyricsHighlightChanged;

pub struct LyricsSettings {
    /// `None` without a home directory: the choice then lasts until quitting.
    store: Option<config::Store>,
    highlight: LyricsHighlight,
    error: Option<SharedString>,
    /// Whether the user has changed `highlight` since it was last saved. Only
    /// then is it written: it may have come from the environment, or from a
    /// file that could not be read.
    unsaved: bool,
    saving: bool,
    _save: Task<()>,
}

impl EventEmitter<LyricsHighlightChanged> for LyricsSettings {}

impl LyricsSettings {
    pub(crate) fn new(_: &mut Context<Self>) -> Self {
        let (store, highlight, error) = read_saved_highlight();
        Self::with(store, highlight, error)
    }

    fn with(
        store: Option<config::Store>,
        highlight: LyricsHighlight,
        error: Option<SharedString>,
    ) -> Self {
        Self {
            store,
            highlight,
            error,
            unsaved: false,
            saving: false,
            _save: Task::ready(()),
        }
    }

    pub fn highlight(&self) -> LyricsHighlight {
        self.highlight
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(crate) fn set_highlight(&mut self, highlight: LyricsHighlight, cx: &mut Context<Self>) {
        if self.highlight == highlight {
            return;
        }
        self.highlight = highlight;
        self.unsaved = true;
        self.save(cx);
        cx.emit(LyricsHighlightChanged);
        cx.notify();
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
            while let Ok(Some(highlight)) = this.update(cx, |this, _| {
                this.saving = std::mem::take(&mut this.unsaved);
                this.saving.then_some(this.highlight)
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move { store.save_lyrics_highlight(highlight) })
                    .await;
                this.update(cx, |this, cx| {
                    this.error = saved.err().map(|error| {
                        format!("Couldn't save the lyrics highlight: {error}.").into()
                    });
                    cx.notify();
                })
                .ok();
            }
        });
    }
}

/// The saved highlight, and why it could not be read or will not be saved.
fn read_saved_highlight() -> (Option<config::Store>, LyricsHighlight, Option<SharedString>) {
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            let error = format!("The lyrics highlight won't be saved: {error}.").into();
            return (None, LyricsHighlight::default(), Some(error));
        }
    };
    match store.lyrics_highlight() {
        Ok(highlight) => (Some(store), highlight, None),
        Err(error) => {
            let error = format!("Couldn't read the lyrics highlight: {error}.").into();
            (Some(store), LyricsHighlight::default(), Some(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn a_new_choice_is_told_and_saved_and_the_same_one_is_neither(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings =
            cx.new(|_| LyricsSettings::with(Some(store.clone()), LyricsHighlight::ByWord, None));
        let told = Rc::new(Cell::new(0));
        let _subscription = cx.update(|cx| {
            let told = told.clone();
            cx.subscribe(&settings, move |_, _: &LyricsHighlightChanged, _| {
                told.set(told.get() + 1);
            })
        });

        settings.update(cx, |settings, cx| {
            settings.set_highlight(LyricsHighlight::ByWord, cx);
        });
        cx.run_until_parked();
        assert_eq!(told.get(), 0);
        assert!(!store.config_path().exists());

        settings.update(cx, |settings, cx| {
            settings.set_highlight(LyricsHighlight::ByLine, cx);
        });
        cx.run_until_parked();
        assert_eq!(told.get(), 1);
        assert_eq!(store.lyrics_highlight().unwrap(), LyricsHighlight::ByLine);
        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.highlight(), LyricsHighlight::ByLine);
            assert!(settings.error().is_none());
        });
    }
}
