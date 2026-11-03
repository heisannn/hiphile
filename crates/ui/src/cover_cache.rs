use std::sync::Arc;

use gpui_kit::{
    App, AppContext as _, Entity, ImageCache, ImageCacheError, ImageCacheItem, RenderImage,
    Resource, Window,
};

/// How many decoded covers stay in memory. A 512-pixel cover takes about
/// 1 MiB.
const CAPACITY: usize = 64;

/// Decoded cover art, keeping the images used most recently.
///
/// Install it with `image_cache` above the views that show covers. Dropping it
/// frees every image it holds, along with the URLs they were loaded from.
pub struct CoverCache {
    /// Least recently used first.
    items: Vec<(Resource, ImageCacheItem)>,
}

impl CoverCache {
    pub fn new(cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self { items: Vec::new() });
        cx.observe_release(&cache, |cache, cx| {
            for (_, item) in cache.items.drain(..) {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        cache
    }
}

impl ImageCache for CoverCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let entry = match self.items.iter().position(|(cached, _)| cached == resource) {
            Some(ix) => self.items.remove(ix),
            None => {
                if self.items.len() >= CAPACITY {
                    let (_, oldest) = self.items.remove(0);
                    if let Some(Ok(image)) = oldest.get() {
                        cx.drop_image(image, Some(window));
                    }
                }
                (resource.clone(), ImageCacheItem::new(resource, cx))
            }
        };
        self.items.push(entry);
        self.items.last()?.1.use_image(window)
    }
}
