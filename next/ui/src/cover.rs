//! Covers, fetched once however many views show them.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::{image, sized_icon};
use anti_webui_md3e::icons::{self, Symbol};
use hiphile_core::{CoverSource, Hiphile};
use next_host::on_ui;

/// How many covers stay known once no view shows them. Each is a path or an image still held.
const KEPT: usize = 512;

/// What a cover stands for while there is no picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverKind {
    Album,
    Artist,
    Playlist,
}

#[derive(Clone, Debug, PartialEq)]
enum Loaded {
    Loading,
    Found(CoverSource),
    Missing,
}

struct Entry {
    loaded: Signal<Loaded>,
    /// Holds `loaded`, apart from any view.
    owner: Owner,
    /// Holds the fetch while it lasts. What waits on the UI thread is looked into at every update,
    /// so it goes once the fetch ends.
    fetching: Option<Owner>,
    views: usize,
}

impl Entry {
    /// Kept once no view shows it: what was found. What was not may have been for a moment, as a
    /// server that did not answer looks the same, and is asked for again when next shown.
    fn is_kept(&self) -> bool {
        self.fetching.is_none()
            && self
                .loaded
                .with_untracked(|loaded| matches!(loaded, Loaded::Found(_)))
    }

    /// Calls off the fetch, if it lasts.
    fn dispose(self) {
        if let Some(fetching) = self.fetching {
            fetching.dispose();
        }
        self.owner.dispose();
    }
}

struct Covers {
    hiphile: Hiphile,
    entries: HashMap<Arc<str>, Entry>,
    /// Shown by no view, the least recently first.
    unseen: VecDeque<Arc<str>>,
}

thread_local! {
    static COVERS: RefCell<Option<Covers>> = const { RefCell::new(None) };
}

/// Where the covers on this UI thread come from. Call before any cover is shown; signing in to
/// another server needs no new call, as a cover's URL names its server.
pub fn install_covers(hiphile: Hiphile) {
    COVERS.with(|covers| {
        let old = covers.borrow_mut().take();
        for entry in old.into_iter().flat_map(|old| old.entries.into_values()) {
            entry.dispose();
        }
        *covers.borrow_mut() = Some(Covers {
            hiphile,
            entries: HashMap::new(),
            unseen: VecDeque::new(),
        });
    });
}

/// The cover at `url`, a session's `cover_art_url`, filling its box and cut to it. Until it comes,
/// and when there is none, the box shows what `kind` stands for.
pub fn cover(url: Option<Arc<str>>, kind: CoverKind) -> Div {
    let frame = div()
        .relative()
        .overflow_hidden()
        .flex_none()
        .dyn_style(|s| s.bg(color_scheme().surface_container_highest));
    let Some(url) = url else {
        return frame.child(placeholder(kind));
    };
    let loaded = watch(url);
    frame.child(dyn_view(move || match loaded.get() {
        Loaded::Found(source) => {
            untrack(|| image(image_of(source)).absolute().inset(0).into_view())
        }
        Loaded::Loading | Loaded::Missing => untrack(|| placeholder(kind).into_view()),
    }))
}

/// The cover at `url` filling its box once it has come, and nothing before then or when there is
/// none: for laying over a smaller one that is already there.
pub fn cover_over(url: Arc<str>) -> Div {
    let loaded = watch(url);
    div().child(dyn_view(move || match loaded.get() {
        Loaded::Found(source) => {
            untrack(|| picture_at_once(image_of(source)).size_full().into_view())
        }
        Loaded::Loading | Loaded::Missing => untrack(|| div().into_view()),
    }))
}

/// `source` filling its box as soon as it is decoded, with no surface under it and no fade, unlike
/// md3e's `image`: for a picture already in view, such as one moving from where it was.
pub fn picture_at_once(source: impl IntoImageSource) -> Img {
    img(source).object_fit(ObjectFit::Cover).select_none()
}

/// The cover as `image` takes it. A kept cover is named by its file, by which AntiWebUI keeps what
/// it decoded.
pub fn image_of(source: CoverSource) -> ImageSource {
    match source {
        CoverSource::File(path) => ImageSource::Path(path.into()),
        CoverSource::Bytes(bytes) => ImageSource::Bytes(bytes),
    }
}

fn placeholder(kind: CoverKind) -> Div {
    let symbol: &'static Symbol = match kind {
        CoverKind::Album => icons::ALBUM,
        CoverKind::Artist => icons::ARTIST,
        CoverKind::Playlist => icons::QUEUE_MUSIC,
    };
    div()
        .absolute()
        .inset(0)
        .flex()
        .items_center()
        .justify_center()
        .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
        .child(sized_icon(symbol, 24.0))
}

/// The cover at `url` as it loads, for as long as the current owner lives.
fn watch(url: Arc<str>) -> Signal<Loaded> {
    let loaded = COVERS.with(|covers| {
        let mut covers = covers.borrow_mut();
        let covers = covers
            .as_mut()
            .expect("next_ui::cover::install_covers on this thread first");
        covers.unseen.retain(|unseen| *unseen != url);
        if let Some(entry) = covers.entries.get_mut(&url) {
            entry.views += 1;
            return entry.loaded;
        }
        let (loaded, owner) = create_root(|| signal(Loaded::Loading));
        let ((), fetching) = create_root(|| {
            let (fetch, url) = (covers.hiphile.cover_file(&url), url.clone());
            let _ = on_ui(fetch, move |found| {
                loaded.set(found.map_or(Loaded::Missing, Loaded::Found));
                let done = COVERS.with(|covers| {
                    let mut covers = covers.borrow_mut();
                    covers.as_mut()?.entries.get_mut(&url)?.fetching.take()
                });
                // Outside the borrow, and last: it is the owner this runs in
                if let Some(done) = done {
                    done.dispose();
                }
            });
        });
        covers.entries.insert(
            url.clone(),
            Entry {
                loaded,
                owner,
                fetching: Some(fetching),
                views: 1,
            },
        );
        loaded
    });
    on_cleanup(move || COVERS.with(|covers| unwatch(covers, url)));
    loaded
}

/// One view less shows the cover at `url`. A fetch no view waits for is called off, and what was
/// found stays known for a while.
fn unwatch(covers: &RefCell<Option<Covers>>, url: Arc<str>) {
    let gone = {
        let mut covers = covers.borrow_mut();
        let Some(covers) = covers.as_mut() else {
            return;
        };
        let Some(entry) = covers.entries.get_mut(&url) else {
            return;
        };
        entry.views -= 1;
        if entry.views > 0 {
            return;
        }
        if !entry.is_kept() {
            covers.entries.remove(&url).into_iter().collect::<Vec<_>>()
        } else {
            covers.unseen.push_back(url);
            let excess = covers.unseen.len().saturating_sub(KEPT);
            covers
                .unseen
                .drain(..excess)
                .filter_map(|old| covers.entries.remove(&old))
                .collect()
        }
    };
    // Outside the borrow: disposing runs cleanups, which may reach the covers again
    for entry in gone {
        entry.dispose();
    }
}

#[cfg(test)]
fn known() -> Vec<(Arc<str>, usize, bool)> {
    COVERS.with(|covers| {
        let covers = covers.borrow();
        let mut known: Vec<_> = covers
            .as_ref()
            .map(|covers| {
                covers
                    .entries
                    .iter()
                    .map(|(url, entry)| (url.clone(), entry.views, entry.fetching.is_some()))
                    .collect()
            })
            .unwrap_or_default();
        known.sort();
        known
    })
}

#[cfg(test)]
mod tests;
