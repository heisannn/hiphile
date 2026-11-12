use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use executor::{Executor, Task};
use futures::executor::block_on;
use session::CoverSource;

/// The image at `url`, from `covers` while they keep it and else from the server, which then
/// keeps it, or nothing when neither has it. Runs as blocking work, as the covers are on the disk.
pub(crate) fn cover_image(
    executor: &Executor,
    net: net::Client,
    covers: Option<catalog::CoverStore>,
    url: Arc<str>,
) -> Task<Option<Arc<[u8]>>> {
    executor.spawn_blocking(move || {
        block_on(session::cover_image(&net, covers.as_ref(), &url)).map(Arc::from)
    })
}

/// As [`cover_image`], giving the file a kept image is in. An image not kept is the one
/// `fetched` still holds for `url` while it is the same, so that a UI caching images by their
/// address decodes it once.
pub(crate) fn cover_file(
    executor: &Executor,
    net: net::Client,
    covers: Option<catalog::CoverStore>,
    fetched: Arc<FetchedCovers>,
    url: Arc<str>,
) -> Task<Option<CoverSource>> {
    executor.spawn_blocking(move || {
        match block_on(session::cover_file(&net, covers.as_ref(), &url))? {
            CoverSource::Bytes(image) => Some(CoverSource::Bytes(fetched.share(url, image))),
            file => Some(file),
        }
    })
}

/// The images fetched and not kept, by URL, while someone holds them.
#[derive(Default)]
pub(crate) struct FetchedCovers(Mutex<HashMap<Arc<str>, Weak<[u8]>>>);

impl FetchedCovers {
    fn share(&self, url: Arc<str>, image: Arc<[u8]>) -> Arc<[u8]> {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        // Its `Weak` keeps the bytes of an image no one holds any more
        held.retain(|_, image| image.strong_count() > 0);
        if let Some(same) = held
            .get(&url)
            .and_then(Weak::upgrade)
            .filter(|held| *held == image)
        {
            return same;
        }
        held.insert(url, Arc::downgrade(&image));
        image
    }
}

#[cfg(test)]
mod tests;
