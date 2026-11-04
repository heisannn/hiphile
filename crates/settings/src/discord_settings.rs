use std::time::Duration;

use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Subscription, Task};

/// How long the key must stay unchanged before it applies. The field reports
/// every keystroke, and each key on the way would be tried on ImgBB.
const SETTLE: Duration = Duration::from_millis(500);

/// The key changed; read it from [`DiscordSettings::imgbb_key`].
pub struct ImgbbKeyChanged;

/// The ImgBB API key that lets covers go on the Discord status. Saving happens
/// off the main thread.
pub struct DiscordSettings {
    /// `None` without a home directory: the key then lasts until quitting.
    store: Option<config::Store>,
    /// What the field last held. `None` when it is empty.
    typed: Option<String>,
    /// The key in effect, which `typed` becomes once it settles.
    applied: Option<String>,
    /// Why the key could not be read or saved.
    error: Option<SharedString>,
    /// Why the last cover could not be published.
    upload_error: Option<SharedString>,
    _settle: Task<()>,
    unsaved: bool,
    saving: bool,
    _save: Task<()>,
    _quit: Subscription,
}

impl EventEmitter<ImgbbKeyChanged> for DiscordSettings {}

impl DiscordSettings {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let (store, key, error) = read_saved_key();
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
            _quit: cx.on_app_quit(|this, _| {
                this.save_before_quitting();
                async {}
            }),
        }
    }

    pub fn imgbb_key(&self) -> Option<&str> {
        self.applied.as_deref()
    }

    /// Says why the last cover could not be published, if it could not.
    pub fn set_upload_error(&mut self, error: Option<SharedString>, cx: &mut Context<Self>) {
        self.upload_error = error;
        cx.notify();
    }

    pub(crate) fn errors(&self) -> impl Iterator<Item = &SharedString> {
        self.error.iter().chain(&self.upload_error)
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

    /// Writes a key still settling or waiting for its turn, which no task would
    /// write once the app has quit.
    fn save_before_quitting(&self) {
        if (self.typed != self.applied || self.unsaved)
            && let Some(store) = &self.store
        {
            store.save_imgbb_api_key(self.typed.as_deref()).ok();
        }
    }
}

/// The key `text` holds. Pasting often brings spaces along.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_what_is_left_of_the_text() {
        assert_eq!(chosen_key("  abc123\n").as_deref(), Some("abc123"));
        assert_eq!(chosen_key(""), None);
        assert_eq!(chosen_key("   "), None);
    }
}
