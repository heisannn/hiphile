//! Where the song is, which the core does not tell of, read again and again.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use anti_webui::prelude::*;

/// Keeps `position` to where the song of `player` is, reading it every `every` while `wanted`
/// and the current owner lives.
pub(crate) fn keep_reading(
    player: playback::Player,
    position: Signal<Duration>,
    every: Duration,
    wanted: impl Fn() -> bool + 'static,
) {
    let next: Rc<Cell<Option<Timeout>>> = Rc::default();
    let called_off = next.clone();
    on_cleanup(move || {
        if let Some(next) = called_off.take() {
            next.cancel();
        }
    });
    effect(move || {
        let wanted = wanted();
        untrack(|| {
            if let Some(next) = next.take() {
                next.cancel();
            }
            if wanted {
                read(&player, position);
                read_after(player.clone(), position, every, next.clone());
            }
        });
    });
}

pub(crate) fn read(player: &playback::Player, position: Signal<Duration>) {
    let now = player.position();
    if position.get_untracked() != now {
        position.set(now);
    }
}

/// Timers run outside any owner, which keeps no cleanup for them: each one after the first is
/// called off through `next` alone.
fn read_after(
    player: playback::Player,
    position: Signal<Duration>,
    every: Duration,
    next: Rc<Cell<Option<Timeout>>>,
) {
    let again = next.clone();
    next.set(Some(set_timeout(every, move || {
        if !position.is_alive() {
            return;
        }
        read(&player, position);
        read_after(player, position, every, again);
    })));
}
