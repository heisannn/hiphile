use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::components::{SearchStyle, search_bar};
use hiphile_core::navigation::Location;
use hiphile_core::search::{Hit, Results};
use next_host::follow;
use next_ui::{Command, Commands, ScreenContext};
use subsonic::Song;

use crate::results::{Found, HitKey, results};

const WIDTH: f32 = 640.0;
/// Puts the bar in the middle of the screens' app bar, which is 64 high to its 56.
const TOP: f32 = 4.0;

/// The search over the screens, open while `open` says so: the bar at their top, with what it
/// finds below it. Choosing what it found goes there or plays it, and closes the search; Escape
/// and a click outside of it close it too. It goes where it is a child of, which must be relative
/// and as wide as the screens.
///
/// Typing goes to the bar, and the keys move what is highlighted: ↑ and ↓ move it, and Enter
/// chooses it.
pub fn search(cx: &ScreenContext, open: Signal<bool>) -> Div {
    let index = cx.workspace.search().clone();
    let status = follow(index.subscribe());
    let query = signal(String::new());
    // From opening until the bar has closed, which goes on after `open` turns false
    let shown = signal(false);
    {
        let index = index.clone();
        let was_open = Cell::new(false);
        effect(move || {
            let now = open.get();
            if was_open.replace(now) == now || !now {
                return;
            }
            untrack(|| {
                index.open();
                // At once, so that it does not open on how it was when last open
                status.set(index.status());
                query.set_if_changed(String::new());
                shown.set(true);
            });
        });
    }
    effect(move || {
        let query = query.get();
        untrack(|| index.search(&query));
    });
    let found = memo(move || status.with(|status| query.with(|query| Found::new(status, query))));
    // Where the keys moved the highlight to, in which results: new ones start it at the first
    let moved = signal(None::<(Arc<Results>, usize)>);
    let highlighted = memo(move || {
        let Found::Hits(now) = found.get() else {
            return None;
        };
        let ix = match moved.get() {
            Some((then, ix)) if Arc::ptr_eq(&then, &now) => ix,
            _ => 0,
        };
        hits(&now).nth(ix).map(|hit| (ix, hit.clone()))
    });
    let choose = chooser(open, cx.commands.clone());
    let step = move |by: isize| {
        let Found::Hits(now) = untrack(|| found.get()) else {
            return false;
        };
        let last = hits(&now).count().saturating_sub(1);
        let from = untrack(|| highlighted.with(|hit| hit.as_ref().map_or(0, |(ix, _)| *ix)));
        moved.set(Some((now, from.saturating_add_signed(by).min(last))));
        true
    };
    let key_of = memo(move || highlighted.with(|hit| hit.as_ref().map(|(_, hit)| HitKey::of(hit))));
    let cx = cx.clone();
    let bar = move || {
        let (cx, choose) = (cx.clone(), choose.clone());
        let enter = choose.clone();
        search_bar(
            query,
            move || open.get(),
            move |now| open.set(now),
            move || {
                on_cleanup(move || {
                    if shown.is_alive() {
                        shown.set(false);
                    }
                });
                results(found, key_of, choose.clone(), &cx)
            },
            SearchStyle::new()
                .placeholder("Search artists, albums and songs")
                .width(WIDTH),
        )
        .on_key_down(move |e| {
            let m = e.modifiers;
            if e.is_default_prevented() || m.shift || m.ctrl || m.alt || m.logo {
                return;
            }
            let took = match e.key {
                Key::Named(NamedKey::ArrowDown) => step(1),
                Key::Named(NamedKey::ArrowUp) => step(-1),
                Key::Named(NamedKey::Enter) => match untrack(|| highlighted.get()) {
                    Some((_, hit)) => {
                        enter(&hit);
                        true
                    }
                    None => false,
                },
                _ => false,
            };
            if took {
                e.stop_propagation();
                e.prevent_default();
            }
        })
    };
    div()
        .absolute()
        .top(TOP)
        .left(0)
        .right(0)
        .flex()
        .flex_row()
        .justify_center()
        .child(show(move || shown.get(), bar))
}

pub(crate) type Choose = Rc<dyn Fn(&Hit)>;

/// What choosing `hit` does: it closes the search, and goes to it or plays it.
fn chooser(open: Signal<bool>, commands: Commands) -> Choose {
    Rc::new(move |hit| {
        open.set(false);
        commands.send(match hit {
            Hit::Artist(artist) => Command::Show(Location::Artists(Some(artist.id.clone()))),
            Hit::Album(album) => Command::Show(Location::Albums(Some(album.id().clone()))),
            Hit::Song(song) => {
                let songs: Arc<[Song]> = Arc::new([song.clone()]);
                Command::Play {
                    songs,
                    start: Some(0),
                }
            }
        });
    })
}

/// Every hit, kind by kind, in the order they show.
pub(crate) fn hits(results: &Results) -> impl Iterator<Item = &Hit> {
    results.sections.iter().flat_map(|(_, hits)| hits)
}

#[cfg(test)]
mod tests;
