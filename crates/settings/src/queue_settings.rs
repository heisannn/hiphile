use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Task};

pub struct SkipInstrumentalsChanged;

pub struct QueueSettings {
    /// `None` without a home directory: the choice then lasts until quitting.
    store: Option<config::Store>,
    is_skip_instrumentals_on: bool,
    error: Option<SharedString>,
    /// Whether the user has changed it since it was last saved. Only then is it
    /// written: it may have come from the environment, or from a file that could
    /// not be read.
    unsaved: bool,
    saving: bool,
    _save: Task<()>,
}

impl EventEmitter<SkipInstrumentalsChanged> for QueueSettings {}

impl QueueSettings {
    pub(crate) fn new(_: &mut Context<Self>) -> Self {
        let (store, is_on, error) = read_saved();
        Self::with(store, is_on, error)
    }

    fn with(store: Option<config::Store>, is_on: bool, error: Option<SharedString>) -> Self {
        Self {
            store,
            is_skip_instrumentals_on: is_on,
            error,
            unsaved: false,
            saving: false,
            _save: Task::ready(()),
        }
    }

    pub fn is_skip_instrumentals_on(&self) -> bool {
        self.is_skip_instrumentals_on
    }

    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(crate) fn set_skip_instrumentals(&mut self, is_on: bool, cx: &mut Context<Self>) {
        if self.is_skip_instrumentals_on == is_on {
            return;
        }
        self.is_skip_instrumentals_on = is_on;
        self.unsaved = true;
        self.save(cx);
        cx.emit(SkipInstrumentalsChanged);
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
            while let Ok(Some(is_on)) = this.update(cx, |this, _| {
                this.saving = std::mem::take(&mut this.unsaved);
                this.saving.then_some(this.is_skip_instrumentals_on)
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move { store.save_skip_instrumentals(is_on) })
                    .await;
                this.update(cx, |this, cx| {
                    this.error = saved
                        .err()
                        .map(|error| format!("Couldn't save this setting: {error}.").into());
                    cx.notify();
                })
                .ok();
            }
        });
    }
}

/// The saved choice, and why it could not be read or will not be saved.
fn read_saved() -> (Option<config::Store>, bool, Option<SharedString>) {
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            let error = format!("This setting won't be saved: {error}.").into();
            return (None, false, Some(error));
        }
    };
    match store.skip_instrumentals() {
        Ok(is_on) => (Some(store), is_on, None),
        Err(error) => {
            let error = format!("Couldn't read this setting: {error}.").into();
            (Some(store), false, Some(error))
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
        let settings = cx.new(|_| QueueSettings::with(Some(store.clone()), false, None));
        let told = Rc::new(Cell::new(0));
        let _subscription = cx.update(|cx| {
            let told = told.clone();
            cx.subscribe(&settings, move |_, _: &SkipInstrumentalsChanged, _| {
                told.set(told.get() + 1);
            })
        });

        settings.update(cx, |settings, cx| {
            settings.set_skip_instrumentals(false, cx)
        });
        cx.run_until_parked();
        assert_eq!(told.get(), 0);
        assert!(!store.config_path().exists());

        settings.update(cx, |settings, cx| settings.set_skip_instrumentals(true, cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 1);
        assert!(store.skip_instrumentals().unwrap());
        settings.read_with(cx, |settings, _| {
            assert!(settings.is_skip_instrumentals_on());
            assert!(settings.error().is_none());
        });
    }
}
