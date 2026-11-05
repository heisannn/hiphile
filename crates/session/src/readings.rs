use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::future::{Either, select};
use fuzzy::Reader;
use fuzzy::bundle::{self, BUNDLE};
use gpui_kit::{
    App, AppContext as _, AsyncApp, Context, Entity, EventEmitter, Global, SharedString, Task,
    WeakEntity,
};
use url::Url;

/// How often the progress of a download shows.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Where the dictionaries that read kanji have got to.
#[derive(Clone, Debug, PartialEq)]
pub enum ReadingsState {
    /// The user turned the search by romaji off.
    Off,
    Checking,
    /// As a fraction from 0 to 1.
    Downloading(f32),
    Ready(PathBuf),
    /// What went wrong, for Settings to show with a way to try again.
    Failed(SharedString),
}

/// Emitted when the dictionaries become available or stop being so: what was
/// read with them, or for lack of them, is out of date.
pub struct ReadingsChanged;

/// Emitted when getting the dictionaries failed, with what went wrong.
pub struct ReadingsFailed(pub SharedString);

/// The dictionaries that read kanji: fetched in the background the first time
/// the app runs, unless the user turned them off.
///
/// Lists work without them, finding text as written and by its kana. They read
/// their text again on [`ReadingsChanged`].
pub struct Readings {
    net: net::Client,
    /// `None` without a home directory: a choice then lasts until quitting.
    store: Option<config::Store>,
    /// `None` where the system has no cache directory.
    root: Option<PathBuf>,
    state: ReadingsState,
    /// Why the choice to turn them on or off could not be read or carried out.
    choice_error: Option<SharedString>,
    _work: Task<()>,
    _save: Task<()>,
}

impl EventEmitter<ReadingsChanged> for Readings {}

impl EventEmitter<ReadingsFailed> for Readings {}

struct GlobalReadings(Entity<Readings>);

impl Global for GlobalReadings {}

/// The dictionaries that read kanji, once [`crate::init`] has run.
pub fn readings(cx: &App) -> Option<Entity<Readings>> {
    cx.try_global::<GlobalReadings>()
        .map(|readings| readings.0.clone())
}

/// Where the dictionaries that read kanji are, when they are there.
pub(crate) fn readings_dir(cx: &App) -> Option<PathBuf> {
    match readings(cx)?.read(cx).state() {
        ReadingsState::Ready(dir) => Some(dir.clone()),
        _ => None,
    }
}

/// What to add when a search finds nothing but might have found kanji with
/// dictionaries that are not there, or romaji would look like it does not work.
pub fn missing_readings_note(cx: &App) -> Option<&'static str> {
    match readings(cx)?.read(cx).state() {
        ReadingsState::Checking | ReadingsState::Downloading(_) => {
            Some("Finding kanji by romaji needs a dictionary that has yet to download.")
        }
        ReadingsState::Failed(_) => Some(
            "Finding kanji by romaji needs a dictionary that failed to download. \
             Try again in Settings.",
        ),
        ReadingsState::Off | ReadingsState::Ready(_) => None,
    }
}

pub(crate) fn init(net: net::Client, cx: &mut App) {
    let readings = cx.new(|cx| Readings::new(net, cx));
    cx.set_global(GlobalReadings(readings));
}

impl Readings {
    fn new(net: net::Client, cx: &mut Context<Self>) -> Self {
        let (store, is_on, choice_error) = read_choice();
        let mut this = Self {
            net,
            store,
            root: bundle::root().ok(),
            state: ReadingsState::Off,
            choice_error,
            _work: Task::ready(()),
            _save: Task::ready(()),
        };
        if is_on {
            this.fetch(cx);
        } else {
            // Off may have been chosen in the file, or as a download finished.
            this.remove(cx);
        }
        this
    }

    pub fn state(&self) -> &ReadingsState {
        &self.state
    }

    pub fn is_on(&self) -> bool {
        self.state != ReadingsState::Off
    }

    pub fn choice_error(&self) -> Option<&SharedString> {
        self.choice_error.as_ref()
    }

    /// Turns the search by romaji on, fetching the dictionaries, or off,
    /// removing them.
    pub fn set_on(&mut self, is_on: bool, cx: &mut Context<Self>) {
        if self.is_on() == is_on {
            return;
        }
        self.save_choice(is_on, cx);
        if is_on {
            self.fetch(cx);
        } else {
            self.set_state(ReadingsState::Off, cx);
            self.remove(cx);
        }
    }

    /// Tries again after a failure.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, ReadingsState::Failed(_)) {
            self.fetch(cx);
        }
    }

    fn set_state(&mut self, state: ReadingsState, cx: &mut Context<Self>) {
        let was_ready = matches!(self.state, ReadingsState::Ready(_));
        let is_ready = matches!(state, ReadingsState::Ready(_));
        if let ReadingsState::Failed(error) = &state {
            cx.emit(ReadingsFailed(error.clone()));
        }
        self.state = state;
        if was_ready != is_ready {
            cx.emit(ReadingsChanged);
        }
        cx.notify();
    }

    /// Uses the dictionaries on disk, or downloads them if there are none.
    fn fetch(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            let error = format!(
                "Couldn't download the dictionaries: {}.",
                bundle::Error::NoCacheDir
            );
            self.set_state(ReadingsState::Failed(error.into()), cx);
            return;
        };
        self.set_state(ReadingsState::Checking, cx);
        let net = self.net.clone();
        self._work = cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn({
                    let root = root.clone();
                    async move { usable_bundle(&root) }
                })
                .await;
            let result = match saved {
                Some(dir) => Ok(dir),
                None => download(&this, net, root, cx).await,
            };
            this.update(cx, |this, cx| {
                let state = match result {
                    Ok(dir) => ReadingsState::Ready(dir),
                    Err(error) => ReadingsState::Failed(error.into()),
                };
                this.set_state(state, cx);
            })
            .ok();
        });
    }

    /// Removes the dictionaries, stopping a download under way.
    fn remove(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self._work = cx.spawn(async move |this, cx| {
            let removed = cx
                .background_spawn(async move { bundle::remove(&root) })
                .await;
            if let Err(error) = removed {
                this.update(cx, |this, cx| {
                    let error = format!("Couldn't remove the dictionaries: {error}.");
                    this.choice_error = Some(error.into());
                    cx.notify();
                })
                .ok();
            }
        });
    }

    fn save_choice(&mut self, is_on: bool, cx: &mut Context<Self>) {
        self.choice_error = None;
        let Some(store) = self.store.clone() else {
            return;
        };
        self._save = cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn(async move { store.save_romaji_search(is_on) })
                .await;
            if let Err(error) = saved {
                this.update(cx, |this, cx| {
                    let error = format!("Couldn't save the search setting: {error}.");
                    this.choice_error = Some(error.into());
                    cx.notify();
                })
                .ok();
            }
        });
    }
}

/// The saved choice, which is on when it cannot be read, and why it could not be
/// read or will not be saved.
fn read_choice() -> (Option<config::Store>, bool, Option<SharedString>) {
    match config::Store::user() {
        Ok(store) => match store.romaji_search() {
            Ok(is_on) => (Some(store), is_on, None),
            Err(error) => {
                let error = format!("Couldn't read the search setting: {error}.");
                (Some(store), true, Some(error.into()))
            }
        },
        Err(error) => {
            let error = format!("The search setting won't be saved: {error}.");
            (None, true, Some(error.into()))
        }
    }
}

/// The bundle under `root`, unless it cannot be opened, as when the system cleared
/// part of its cache. Then it goes, to be downloaded again.
fn usable_bundle(root: &Path) -> Option<PathBuf> {
    let dir = bundle::installed(root, &BUNDLE)?;
    if Reader::open(&dir).is_ok() {
        return Some(dir);
    }
    bundle::remove(root).ok();
    None
}

/// Stops a download when the work waiting on it goes away, which its blocking
/// reads would not notice.
struct CancelOnDrop(net::DownloadHandle);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Downloads the dictionaries and installs them as they arrive, showing how far
/// along that is.
async fn download(
    this: &WeakEntity<Readings>,
    net: net::Client,
    root: PathBuf,
    cx: &mut AsyncApp,
) -> Result<PathBuf, String> {
    let describe = |error: net::Error| format!("Couldn't download the dictionaries: {error}.");
    let url = Url::parse(BUNDLE.url)
        .map_err(|error| format!("Couldn't download the dictionaries: {error}."))?;
    let download = net.download(url).await.map_err(describe)?;
    let total = download.content_length().unwrap_or(BUNDLE.download_bytes);
    let _cancel = CancelOnDrop(download.handle());
    let read = Arc::new(AtomicU64::new(0));
    let mut install = cx.background_spawn({
        let read = read.clone();
        async move {
            bundle::install(download, &root, &BUNDLE, move |bytes| {
                read.store(bytes, Ordering::Relaxed);
            })
        }
    });
    loop {
        let timer = cx.background_executor().timer(PROGRESS_INTERVAL);
        if let Either::Left((result, _)) = select(&mut install, std::pin::pin!(timer)).await {
            return result.map_err(|error| format!("Couldn't install the dictionaries: {error}."));
        }
        let fraction = fraction_of(read.load(Ordering::Relaxed), total);
        this.update(cx, |this, cx| {
            this.set_state(ReadingsState::Downloading(fraction), cx);
        })
        .ok();
    }
}

#[allow(clippy::cast_precision_loss, reason = "a fraction for a progress bar")]
fn fraction_of(read: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.;
    }
    (read as f32 / total as f32).clamp(0., 1.)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn without_a_cache_dir(cx: &mut gpui_kit::TestAppContext) -> Entity<Readings> {
        cx.new(|_| Readings {
            net: net::Client::new("hiphile-test").unwrap(),
            store: None,
            root: None,
            state: ReadingsState::Off,
            choice_error: None,
            _work: Task::ready(()),
            _save: Task::ready(()),
        })
    }

    #[gpui_kit::test]
    fn every_failure_says_what_went_wrong(cx: &mut gpui_kit::TestAppContext) {
        let readings = without_a_cache_dir(cx);
        let failures = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let failures = failures.clone();
            cx.subscribe(&readings, move |_, ReadingsFailed(error), _| {
                failures.borrow_mut().push(error.clone());
            })
            .detach();
        });

        readings.update(cx, |it, cx| it.fetch(cx));
        readings.update(cx, |it, cx| it.retry(cx));
        cx.run_until_parked();

        let failures = failures.borrow();
        assert_eq!(failures.len(), 2);
        assert!(failures[0].starts_with("Couldn't download the dictionaries: "));
        assert!(failures[0].ends_with('.'));
    }

    #[gpui_kit::test]
    fn only_a_failure_is_told_as_one(cx: &mut gpui_kit::TestAppContext) {
        let readings = without_a_cache_dir(cx);
        let told = std::rc::Rc::new(std::cell::Cell::new(false));
        cx.update(|cx| {
            let told = told.clone();
            cx.subscribe(&readings, move |_, _: &ReadingsFailed, _| told.set(true))
                .detach();
        });

        readings.update(cx, |it, cx| {
            it.set_state(ReadingsState::Downloading(0.5), cx);
            it.set_state(ReadingsState::Ready("dir".into()), cx);
            it.set_state(ReadingsState::Off, cx);
        });
        cx.run_until_parked();

        assert!(!told.get());
    }

    #[test]
    fn a_bundle_that_cannot_be_opened_goes() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(usable_bundle(root.path()), None);

        let gutted = root.path().join(BUNDLE.name);
        std::fs::create_dir(&gutted).unwrap();
        assert_eq!(usable_bundle(root.path()), None);
        assert!(!gutted.exists());
    }

    #[test]
    fn progress_stays_a_fraction() {
        assert!((fraction_of(50, 100) - 0.5).abs() < f32::EPSILON);
        assert!((fraction_of(200, 100) - 1.).abs() < f32::EPSILON);
        assert!(fraction_of(1, 0).abs() < f32::EPSILON);
    }
}
