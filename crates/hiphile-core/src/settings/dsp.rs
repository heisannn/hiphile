use std::mem;
use std::sync::{Arc, Weak};

use config::{
    BALANCE_DBS, CROSSFEED_CUTOFFS_HZ, CROSSFEED_SEPARATIONS_DB, Crossfeed, CrossfeedPreset,
};
use executor::{Executor, Publish, Published, Saver};
use tokio::sync::watch;

use super::choices::{number_value, numbers};
use super::{Choice, NumberRange};

pub const CROSSFEED_PRESETS: [&str; 4] = ["bs2b", "Chu Moy", "Jan Meier", "Custom"];
const CUSTOM_IX: usize = 3;
const BALANCE_RANGE: NumberRange =
    NumberRange::new(*BALANCE_DBS.start(), *BALANCE_DBS.end(), 0.5, Some(1));
const SEPARATION_STEP_DB: f64 = 0.5;
const CUTOFF_STEP_HZ: u32 = 50;

/// Crossfeed and balance, with what is needed to choose them.
#[derive(Clone, Debug, PartialEq)]
pub struct DspSnapshot {
    crossfeed: Crossfeed,
    /// See [`BALANCE_DBS`].
    balance_db: f64,
    read_errors: Vec<String>,
    save_error: Option<String>,
}

impl DspSnapshot {
    /// What the engine is to play with.
    pub fn settings(&self) -> audio::DspSettings {
        audio::dsp_from_config(self.crossfeed, self.balance_db)
    }

    pub fn is_crossfeed_on(&self) -> bool {
        self.crossfeed.is_on
    }

    /// Into [`CROSSFEED_PRESETS`].
    pub fn crossfeed_preset_ix(&self) -> usize {
        match self.crossfeed.preset {
            CrossfeedPreset::Bs2b => 0,
            CrossfeedPreset::ChuMoy => 1,
            CrossfeedPreset::JanMeier => 2,
            CrossfeedPreset::Custom(_) => CUSTOM_IX,
        }
    }

    pub fn separation_options(&self) -> Vec<Choice> {
        let steps = steps(
            *CROSSFEED_SEPARATIONS_DB.start(),
            *CROSSFEED_SEPARATIONS_DB.end(),
            SEPARATION_STEP_DB,
        );
        numbers(&steps, self.filter().separation_db, |db| {
            format!("{db:.1} dB")
        })
    }

    pub fn separation_value(&self) -> String {
        number_value(self.filter().separation_db)
    }

    pub fn cutoff_options(&self) -> Vec<Choice> {
        let steps: Vec<f64> = CROSSFEED_CUTOFFS_HZ
            .step_by(CUTOFF_STEP_HZ as usize)
            .map(f64::from)
            .collect();
        numbers(&steps, f64::from(self.filter().cutoff_hz), |hz| {
            format!("{hz} Hz")
        })
    }

    pub fn cutoff_value(&self) -> String {
        number_value(f64::from(self.filter().cutoff_hz))
    }

    /// See [`BALANCE_DBS`].
    pub fn balance_db(&self) -> f64 {
        self.balance_db
    }

    /// Names the channel that is lowered.
    pub fn balance_text(&self) -> String {
        let db = self.balance_db;
        if db < 0.0 {
            format!("Right {db:.1} dB")
        } else if db > 0.0 {
            format!("Left {:.1} dB", -db)
        } else {
            "Center".to_owned()
        }
    }

    pub fn errors(&self) -> impl Iterator<Item = &str> {
        self.read_errors
            .iter()
            .chain(&self.save_error)
            .map(String::as_str)
    }

    fn filter(&self) -> config::CrossfeedFilter {
        self.crossfeed.preset.filter()
    }
}

/// Crossfeed and balance. Each change is saved, and only what was changed is written: the rest
/// may have come from the environment, or from a file that could not be read.
#[derive(Clone)]
pub struct DspSettings(Arc<Inner>);

struct Inner {
    /// `None` without a home directory: choices then last until quitting.
    store: Option<config::Store>,
    saver: Option<Saver<Changes>>,
    state: Published<State>,
}

struct State {
    snapshot: DspSnapshot,
    /// What the crossfeed comes to on disk once the saver has written what it was given, to
    /// write only what has changed since.
    written_crossfeed: Crossfeed,
}

impl Publish for State {
    type Snapshot = DspSnapshot;

    fn snapshot(&self) -> &DspSnapshot {
        &self.snapshot
    }
}

/// What a write is to do: the crossfeed as it was on disk and as it is now, and the balance.
struct Changes {
    crossfeed: Option<(Crossfeed, Crossfeed)>,
    balance_db: Option<f64>,
}

impl DspSettings {
    pub fn new(executor: Executor, store: Result<config::Store, config::Error>) -> Self {
        let (store, crossfeed, balance_db, read_errors) = read_saved(store);
        Self::with(executor, store, crossfeed, balance_db, read_errors)
    }

    fn with(
        executor: Executor,
        store: Option<config::Store>,
        crossfeed: Crossfeed,
        balance_db: f64,
        read_errors: Vec<String>,
    ) -> Self {
        Self(Arc::new_cyclic(|this: &Weak<Inner>| Inner {
            saver: store.clone().map(|store| {
                let this = this.clone();
                Saver::new(executor, move |changes: Changes| {
                    let saved = write(&store, &changes);
                    if let Some(this) = this.upgrade() {
                        this.state.update(|state| {
                            state.snapshot.save_error = saved
                                .err()
                                .map(|error| format!("Couldn't save the DSP settings: {error}."));
                        });
                    }
                })
            }),
            store,
            state: Published::new(State {
                snapshot: DspSnapshot {
                    crossfeed,
                    balance_db,
                    read_errors,
                    save_error: None,
                },
                written_crossfeed: crossfeed,
            }),
        }))
    }

    pub fn snapshot(&self) -> DspSnapshot {
        self.0.state.snapshot()
    }

    /// Sees each new snapshot. A borrow of it must not be held while calling these settings,
    /// which publish under their own lock.
    pub fn subscribe(&self) -> watch::Receiver<DspSnapshot> {
        self.0.state.subscribe()
    }

    /// What [`Self::set_balance`] holds a balance to, in dB.
    pub fn balance_range() -> NumberRange {
        BALANCE_RANGE
    }

    /// Rounded to a step of [`Self::balance_range`] and held to it. NaN is the center.
    pub fn set_balance(&self, db: f64) {
        let step = BALANCE_RANGE.step();
        let db = BALANCE_RANGE
            .clamp((db / step).round() * step)
            .unwrap_or(0.0);
        self.0.state.update(|state| {
            if state.snapshot.balance_db == db {
                return;
            }
            state.snapshot.balance_db = db;
            self.save(state, None, Some(db));
        });
    }

    pub fn set_crossfeed(&self, is_on: bool) {
        self.change_crossfeed(|crossfeed| crossfeed.is_on = is_on);
    }

    /// `ix` is into [`CROSSFEED_PRESETS`]. Custom starts from the values in use, so choosing it
    /// changes nothing until one of them is changed.
    pub fn select_crossfeed_preset(&self, ix: usize) {
        self.change_crossfeed(|crossfeed| {
            crossfeed.preset = match ix {
                0 => CrossfeedPreset::Bs2b,
                1 => CrossfeedPreset::ChuMoy,
                2 => CrossfeedPreset::JanMeier,
                _ => CrossfeedPreset::Custom(crossfeed.preset.filter()),
            };
        });
    }

    /// `value` is one of [`DspSnapshot::separation_options`]. Another value than the one in use
    /// makes the preset custom.
    pub fn set_crossfeed_separation(&self, value: &str) {
        let Some(db) = value
            .parse()
            .ok()
            .filter(|db| CROSSFEED_SEPARATIONS_DB.contains(db))
        else {
            return;
        };
        self.change_filter(|filter| filter.separation_db = db);
    }

    /// `value` is one of [`DspSnapshot::cutoff_options`]. Another value than the one in use makes
    /// the preset custom.
    pub fn set_crossfeed_cutoff(&self, value: &str) {
        let Some(hz) = value
            .parse::<f64>()
            .ok()
            .map(|hz| hz as u32)
            .filter(|hz| CROSSFEED_CUTOFFS_HZ.contains(hz))
        else {
            return;
        };
        self.change_filter(|filter| filter.cutoff_hz = hz);
    }

    /// Writes what is still waiting for its turn, which nothing would write once the app has
    /// quit. Blocks on the disk.
    pub fn save_before_quitting(&self) {
        let (Some(store), Some(saver)) = (&self.0.store, &self.0.saver) else {
            return;
        };
        if let Some(changes) = saver.take_waiting() {
            write(store, &changes).ok();
        }
    }

    fn change_filter(&self, change: impl FnOnce(&mut config::CrossfeedFilter)) {
        self.change_crossfeed(|crossfeed| {
            let mut filter = crossfeed.preset.filter();
            change(&mut filter);
            if filter != crossfeed.preset.filter() {
                crossfeed.preset = CrossfeedPreset::Custom(filter);
            }
        });
    }

    fn change_crossfeed(&self, change: impl FnOnce(&mut Crossfeed)) {
        self.0.state.update(|state| {
            let before = state.snapshot.crossfeed;
            change(&mut state.snapshot.crossfeed);
            if state.snapshot.crossfeed != before {
                let written = mem::replace(&mut state.written_crossfeed, state.snapshot.crossfeed);
                self.save(state, Some(written), None);
            }
        });
    }

    /// Under the lock, so that the changes reach the disk in the order they were made.
    /// `written_crossfeed` is what the crossfeed was on disk before, if it changed.
    fn save(&self, state: &State, written_crossfeed: Option<Crossfeed>, balance_db: Option<f64>) {
        let Some(saver) = &self.0.saver else {
            return;
        };
        let crossfeed = state.snapshot.crossfeed;
        saver.save_with(|waiting| {
            let waiting = waiting.unwrap_or(Changes {
                crossfeed: None,
                balance_db: None,
            });
            Changes {
                crossfeed: waiting
                    .crossfeed
                    .map(|(before, _)| before)
                    .or(written_crossfeed)
                    .map(|before| (before, crossfeed)),
                balance_db: balance_db.or(waiting.balance_db),
            }
        });
    }
}

/// From `start` to `end` by `step`, with `end` among them.
fn steps(start: f64, end: f64, step: f64) -> Vec<f64> {
    let count = ((end - start) / step).round() as usize;
    (0..=count).map(|ix| start + ix as f64 * step).collect()
}

/// Blocks on the disk.
fn write(store: &config::Store, changes: &Changes) -> Result<(), Box<config::Error>> {
    if let Some((before, after)) = changes.crossfeed {
        store.save_crossfeed_changes(before, after)?;
    }
    if let Some(db) = changes.balance_db {
        store.save_balance_db(db)?;
    }
    Ok(())
}

/// The saved settings, and why they could not be read or will not be saved.
fn read_saved(
    store: Result<config::Store, config::Error>,
) -> (Option<config::Store>, Crossfeed, f64, Vec<String>) {
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            let error = format!("DSP settings won't be saved: {error}.");
            return (None, Crossfeed::default(), 0.0, vec![error]);
        }
    };
    let mut errors = Vec::new();
    let mut read = |error: config::Error| {
        errors.push(format!("Couldn't read the DSP settings: {error}."));
    };
    let crossfeed = store.crossfeed().unwrap_or_else(|error| {
        read(error);
        Crossfeed::default()
    });
    let balance_db = store.balance_db().unwrap_or_else(|error| {
        read(error);
        0.0
    });
    (Some(store), crossfeed, balance_db, errors)
}

#[cfg(test)]
mod tests;
