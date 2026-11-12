use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anti_webui::prelude::*;
use executor::Task;
use hiphile_core::{Hiphile, Workspace};
use next_host::{Pending, Scope, follow};
use subsonic::Song;

/// What the cover is asked for at, which the bar shows at 56 points and the colors are taken from
/// at about this many pixels.
pub const COVER_PIXELS: u32 = 128;

/// The cover of a song, as `img` takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cover {
    url: Arc<str>,
    image: ImageSource,
}

impl Cover {
    /// The same for as long as the cover is: `img` keeps what it decoded by it.
    pub fn image(&self) -> ImageSource {
        self.image.clone()
    }
}

/// The cover of the song `workspace` plays, once it has come. It stays as it was while the next
/// one comes, and goes when the song has none or it cannot be had.
pub fn now_playing_cover(hiphile: &Hiphile, workspace: &Workspace) -> Signal<Option<Cover>> {
    let session = workspace.session().clone();
    let shown = follow(workspace.player().subscribe());
    let url = memo(move || {
        shown.with(|shown| {
            shown
                .current_song()
                .and_then(Song::cover_art)
                .map(|cover| session.cover_art_url(cover, COVER_PIXELS))
        })
    });
    let cover = signal(None::<Cover>);
    let hiphile = hiphile.clone();
    let scope = Scope::here();
    let fetching: Rc<RefCell<Option<Pending>>> = Rc::default();
    effect(move || {
        let wanted = url.get();
        untrack(|| {
            if let Some(fetching) = fetching.borrow_mut().take() {
                fetching.cancel();
            }
            let Some(url) = wanted else {
                cover.set(None);
                return;
            };
            if cover.with(|cover| cover.as_ref().is_some_and(|cover| cover.url == url)) {
                return;
            }
            let fetched = scope.on_ui(fetch(&hiphile, &url), move |image| {
                cover.set(image.map(|image| Cover { url, image }));
            });
            *fetching.borrow_mut() = Some(fetched);
        });
    });
    cover
}

fn fetch(hiphile: &Hiphile, url: &str) -> Task<Option<ImageSource>> {
    let image = hiphile.cover_image(url);
    next_host::executor().spawn(async move { image.await.map(ImageSource::from) })
}

#[cfg(test)]
mod tests;
