use std::mem;
use std::sync::Arc;

use gpui_kit::{App, Context, Global, Task};
use session::{Readings, ReadingsStatus};

use crate::http::NetHttpClient;

struct GlobalCovers(catalog::CoverStore);

impl Global for GlobalCovers {}

struct GlobalReadings(Readings);

impl Global for GlobalReadings {}

/// Installs `net` as GPUI's HTTP client, so `img(url)` cover art shares the
/// API's transport and is kept on disk when the system has a cache directory,
/// and starts fetching what reads kanji. Call once at startup, after
/// `gpui_kit::init`.
pub fn init_network(net: net::Client, cx: &mut App) {
    let covers = session::open_covers(config::Store::user());
    if let Some(covers) = &covers {
        cx.set_global(GlobalCovers(covers.clone()));
    }
    let readings = Readings::new(gpui_host::executor(cx), net.clone(), config::Store::user());
    cx.set_global(GlobalReadings(readings));
    cx.set_http_client(Arc::new(NetHttpClient::new(net, covers)));
}

/// Keeps at most about `capacity` bytes of covers from the next one on, or the
/// default amount for `None`.
pub fn set_cover_capacity(capacity: Option<u64>, cx: &App) {
    if let Some(GlobalCovers(covers)) = cx.try_global() {
        session::set_cover_capacity(covers, capacity);
    }
}

/// The dictionaries that read kanji, once [`init_network`] has run.
pub fn readings(cx: &App) -> Option<Readings> {
    cx.try_global::<GlobalReadings>()
        .map(|readings| readings.0.clone())
}

/// Calls `on_change` each time the dictionaries come or go, for as long as the task is kept:
/// what was read with them, or for lack of them, is out of date.
pub fn on_readings_changed<V: 'static>(
    readings: &Readings,
    cx: &mut Context<V>,
    mut on_change: impl FnMut(&mut V, &mut Context<V>) + 'static,
) -> Task<()> {
    let changes = readings.subscribe();
    let mut had_dictionaries = changes.borrow().dictionaries().is_some();
    gpui_host::observe(changes, cx, move |this, status: ReadingsStatus, cx| {
        let has_dictionaries = status.dictionaries().is_some();
        if mem::replace(&mut had_dictionaries, has_dictionaries) != has_dictionaries {
            on_change(this, cx);
        }
    })
}
