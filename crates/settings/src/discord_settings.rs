use std::time::Duration;

use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Subscription, Task};

/// The field reports every keystroke, and each key on the way would be tried on ImgBB.
const SETTLE: Duration = Duration::from_millis(500);

pub struct ImgbbKeyChanged;

pub struct PresenceChanged;

/// The ImgBB API key is what lets covers go on the Discord status.
pub struct DiscordSettings {
    /// `None` without a home directory: the key then lasts until quitting.
    store: Option<config::Store>,
    /// What the field last held. `None` when it is empty.
    typed: Option<String>,
    /// The key in effect, which `typed` becomes once it settles.
    applied: Option<String>,
    /// Why the key could not be read or saved.
    error: Option<SharedString>,
    upload_error: Option<SharedString>,
    _settle: Task<()>,
    unsaved: bool,
    saving: bool,
    _save: Task<()>,
    is_presence_on: bool,
    presence_error: Option<SharedString>,
    /// Whether the user has changed `is_presence_on` since it was last saved.
    /// Only then is it written: it may have come from the environment, or from
    /// a file that could not be read.
    presence_unsaved: bool,
    saving_presence: bool,
    _save_presence: Task<()>,
    _quit: Subscription,
}

impl EventEmitter<ImgbbKeyChanged> for DiscordSettings {}
impl EventEmitter<PresenceChanged> for DiscordSettings {}

impl DiscordSettings {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let (store, key, error) = read_saved_key();
        let (is_presence_on, presence_error) = read_saved_presence();
        Self::with(store, key, error, is_presence_on, presence_error, cx)
    }

    fn with(
        store: Option<config::Store>,
        key: Option<String>,
        error: Option<SharedString>,
        is_presence_on: bool,
        presence_error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            store,
            typed: key.clone(),
            applied: key,
            error,
            upload_error: None,
            _settle: Task::ready(()),
            unsaved: false,
            saving: false,
            _save: Task::ready(()),
            is_presence_on,
            presence_error,
            presence_unsaved: false,
            saving_presence: false,
            _save_presence: Task::ready(()),
            _quit: cx.on_app_quit(|this, _| {
                this.save_before_quitting();
                async {}
            }),
        }
    }

    pub fn imgbb_key(&self) -> Option<&str> {
        self.applied.as_deref()
    }

    pub fn is_presence_on(&self) -> bool {
        self.is_presence_on
    }

    /// An upload that was under way when the status was turned off no longer matters.
    pub fn set_upload_error(&mut self, error: Option<SharedString>, cx: &mut Context<Self>) {
        self.upload_error = error.filter(|_| self.is_presence_on);
        cx.notify();
    }

    pub(crate) fn errors(&self) -> impl Iterator<Item = &SharedString> {
        self.error
            .iter()
            .chain(&self.presence_error)
            .chain(&self.upload_error)
    }

    pub(crate) fn set_presence_on(&mut self, is_on: bool, cx: &mut Context<Self>) {
        if self.is_presence_on == is_on {
            return;
        }
        self.is_presence_on = is_on;
        if !is_on {
            self.upload_error = None;
        }
        self.presence_unsaved = true;
        self.save_presence(cx);
        cx.emit(PresenceChanged);
        cx.notify();
    }

    /// One write at a time, like [`Self::save`].
    fn save_presence(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.saving_presence {
            return;
        }
        self.saving_presence = true;
        self._save_presence = cx.spawn(async move |this, cx| {
            while let Ok(Some(is_on)) = this.update(cx, |this, _| {
                this.saving_presence = std::mem::take(&mut this.presence_unsaved);
                this.saving_presence.then_some(this.is_presence_on)
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move { store.save_discord_presence(is_on) })
                    .await;
                this.update(cx, |this, cx| {
                    this.presence_error = saved
                        .err()
                        .map(|error| format!("Couldn't save the Discord status: {error}.").into());
                    cx.notify();
                })
                .ok();
            }
        });
    }

    pub(crate) fn set_typed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.typed = chosen_key(text);
        self._settle = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            this.update(cx, |this, cx| this.apply(cx)).ok();
        });
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.typed == self.applied {
            return;
        }
        self.applied = self.typed.clone();
        if self.store.is_some() {
            self.error = None;
        }
        self.upload_error = None;
        cx.emit(ImgbbKeyChanged);
        cx.notify();
        self.unsaved = true;
        self.save(cx);
    }

    /// One write at a time, so a slow one cannot leave an older key on disk;
    /// the latest is written when the current write finishes.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.saving {
            return;
        }
        self.saving = true;
        self._save = cx.spawn(async move |this, cx| {
            while let Ok(Some(key)) = this.update(cx, |this, _| {
                this.saving = std::mem::take(&mut this.unsaved);
                this.saving.then(|| this.applied.clone())
            }) {
                let store = store.clone();
                let saved = cx
                    .background_spawn(async move { store.save_imgbb_api_key(key.as_deref()) })
                    .await;
                this.update(cx, |this, cx| {
                    this.error = saved
                        .err()
                        .map(|error| format!("Couldn't save the API key: {error}.").into());
                    cx.notify();
                })
                .ok();
            }
        });
    }

    /// Writes what is still settling or waiting for its turn, which no task
    /// would write once the app has quit.
    fn save_before_quitting(&self) {
        let Some(store) = &self.store else {
            return;
        };
        if self.typed != self.applied || self.unsaved {
            store.save_imgbb_api_key(self.typed.as_deref()).ok();
        }
        if self.presence_unsaved {
            store.save_discord_presence(self.is_presence_on).ok();
        }
    }
}

/// Pasting often brings spaces along.
fn chosen_key(text: &str) -> Option<String> {
    let key = text.trim();
    (!key.is_empty()).then(|| key.to_owned())
}

/// The saved key, and why it could not be read or will not be saved.
fn read_saved_key() -> (Option<config::Store>, Option<String>, Option<SharedString>) {
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            let error = format!("The API key won't be saved: {error}.");
            return (None, None, Some(error.into()));
        }
    };
    match store.imgbb_api_key() {
        Ok(key) => (Some(store), key, None),
        Err(error) => {
            let error = format!("Couldn't read the API key: {error}.");
            (Some(store), None, Some(error.into()))
        }
    }
}

/// Whether the status is on, which it is not when that cannot be read, as it
/// may have been turned off; and why it could not be read or will not be saved.
fn read_saved_presence() -> (bool, Option<SharedString>) {
    let store = match config::Store::user() {
        Ok(store) => store,
        Err(error) => {
            let error = format!("The Discord status won't be saved: {error}.");
            return (true, Some(error.into()));
        }
    };
    match store.discord_presence() {
        Ok(is_on) => (is_on, None),
        Err(error) => {
            let error = format!("Couldn't read the Discord status: {error}.");
            (false, Some(error.into()))
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
    fn turning_the_status_off_is_told_and_saved_and_doing_so_twice_is_not(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let store = config::Store::at(dir.path());
        let settings =
            cx.new(|cx| DiscordSettings::with(Some(store.clone()), None, None, true, None, cx));
        let told = Rc::new(Cell::new(0));
        let _subscription = cx.update(|cx| {
            let told = told.clone();
            cx.subscribe(&settings, move |_, _: &PresenceChanged, _| {
                told.set(told.get() + 1);
            })
        });

        settings.update(cx, |settings, cx| settings.set_presence_on(true, cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 0);
        assert!(!store.config_path().exists());

        settings.update(cx, |settings, cx| settings.set_presence_on(false, cx));
        cx.run_until_parked();
        assert_eq!(told.get(), 1);
        assert!(!store.discord_presence().unwrap());
    }

    #[gpui_kit::test]
    fn with_the_status_off_a_late_upload_error_is_not_shown(cx: &mut TestAppContext) {
        let settings = cx.new(|cx| DiscordSettings::with(None, None, None, false, None, cx));

        settings.update(cx, |settings, cx| {
            settings.set_upload_error(Some("gone".into()), cx);
        });

        settings.read_with(cx, |settings, _| {
            assert_eq!(settings.errors().count(), 0);
        });
    }

    #[test]
    fn a_key_is_what_is_left_of_the_text() {
        assert_eq!(chosen_key("  abc123\n").as_deref(), Some("abc123"));
        assert_eq!(chosen_key(""), None);
        assert_eq!(chosen_key("   "), None);
    }
}
