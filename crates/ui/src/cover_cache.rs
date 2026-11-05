use std::sync::Arc;

use gpui_kit::{
    App, AppContext as _, Entity, ImageCache, ImageCacheError, ImageCacheItem, RenderImage,
    Resource, WeakEntity, Window,
};

/// How many decoded covers stay in memory once out of sight. A 256-pixel cover
/// takes about 256 KiB.
const CAPACITY: usize = 64;

struct CachedCover {
    resource: Resource,
    item: ImageCacheItem,
    /// Whether a frame asked for it since the last trim.
    is_in_use: bool,
}

/// Decoded cover art, keeping the images used most recently.
///
/// Covers on the screen stay however many there are: a large window can show
/// more than the capacity, and letting them push each other out would load them
/// over and over without ever showing them. So the excess goes between frames,
/// taking only covers the last frame did not ask for.
///
/// Install it with `image_cache` above the views that show covers. Dropping it
/// frees every image it holds, along with the URLs they were loaded from.
pub struct CoverCache {
    /// Least recently used first.
    items: Vec<CachedCover>,
    is_trim_scheduled: bool,
    this: WeakEntity<Self>,
}

impl CoverCache {
    pub fn new(cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|cx| Self {
            items: Vec::new(),
            is_trim_scheduled: false,
            this: cx.weak_entity(),
        });
        cx.observe_release(&cache, |cache, cx| {
            for cover in cache.items.drain(..) {
                if let Some(Ok(image)) = cover.item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        cache
    }

    /// Frees the covers past the capacity that no frame asked for since the last
    /// trim, oldest first.
    fn trim(&mut self, window: &mut Window, cx: &mut App) {
        self.is_trim_scheduled = false;
        let mut excess = self.items.len().saturating_sub(CAPACITY);
        self.items.retain_mut(|cover| {
            let keep = excess == 0 || cover.is_in_use;
            cover.is_in_use = false;
            if !keep {
                excess -= 1;
                if let Some(Ok(image)) = cover.item.get() {
                    cx.drop_image(image, Some(window));
                }
            }
            keep
        });
    }

    fn schedule_trim(&mut self, window: &Window) {
        if self.items.len() <= CAPACITY || self.is_trim_scheduled {
            return;
        }
        self.is_trim_scheduled = true;
        let this = self.this.clone();
        window.on_next_frame(move |window, cx| {
            this.update(cx, |cache, cx| cache.trim(window, cx)).ok();
        });
    }
}

impl ImageCache for CoverCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let cached = self
            .items
            .iter()
            .position(|cover| cover.resource == *resource);
        let mut cover = match cached {
            Some(ix) => self.items.remove(ix),
            None => CachedCover {
                resource: resource.clone(),
                item: ImageCacheItem::new(resource, cx),
                is_in_use: false,
            },
        };
        cover.is_in_use = true;
        let image = cover.item.use_image(window);
        self.items.push(cover);
        self.schedule_trim(window);
        image
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use gpui_kit::{
        Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext,
        VisualTestContext, div, img,
    };

    use super::*;

    fn uri(ix: usize) -> String {
        format!("https://example.test/{ix}")
    }

    struct Covers {
        cache: Entity<CoverCache>,
        shown: Range<usize>,
    }

    impl Render for Covers {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .image_cache(self.cache.clone())
                .children(self.shown.clone().map(|ix| img(uri(ix)).size_4()))
        }
    }

    /// Draws a frame that shows the covers in `shown`, up to the start of the next.
    fn frame(covers: &Entity<Covers>, shown: Range<usize>, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            covers.update(cx, |covers, _| covers.shown = shown);
            window.refresh();
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }

    fn covers(cx: &mut TestAppContext) -> (Entity<Covers>, &mut VisualTestContext) {
        cx.add_window_view(|_, cx| Covers {
            cache: CoverCache::new(cx),
            shown: 0..0,
        })
    }

    fn holds(covers: &Entity<Covers>, ix: usize, cx: &mut VisualTestContext) -> bool {
        let resource = Resource::Uri(uri(ix).into());
        cx.update(|_, cx| {
            let items = &covers.read(cx).cache.read(cx).items;
            items.iter().any(|cover| cover.resource == resource)
        })
    }

    fn held(covers: &Entity<Covers>, cx: &mut VisualTestContext) -> usize {
        cx.update(|_, cx| covers.read(cx).cache.read(cx).items.len())
    }

    #[gpui_kit::test]
    fn a_frame_keeps_all_its_covers_however_many_there_are(cx: &mut TestAppContext) {
        let (covers, cx) = covers(cx);
        let shown = CAPACITY * 3;
        for _ in 0..3 {
            frame(&covers, 0..shown, cx);
            assert!(holds(&covers, 0, cx), "the first cover went");
            assert_eq!(held(&covers, cx), shown);
        }
    }

    #[gpui_kit::test]
    fn covers_out_of_sight_go_past_the_capacity_oldest_first(cx: &mut TestAppContext) {
        let (covers, cx) = covers(cx);
        let shown = CAPACITY * 2;
        frame(&covers, 0..shown, cx);

        // The next covers on show are still more than the capacity holds.
        let next = shown..shown * 2;
        frame(&covers, next.clone(), cx);
        frame(&covers, next.clone(), cx);

        assert_eq!(held(&covers, cx), shown);
        assert!(!holds(&covers, 0, cx));
        assert!(holds(&covers, next.start, cx));

        // Back to a few: what is left over goes, least recently used first.
        frame(&covers, 0..1, cx);
        frame(&covers, 0..1, cx);

        assert_eq!(held(&covers, cx), CAPACITY);
        assert!(holds(&covers, 0, cx));
        assert!(!holds(&covers, next.start, cx));
        assert!(holds(&covers, next.end - 1, cx));
    }

    #[gpui_kit::test]
    fn a_full_cache_is_left_alone_until_it_overflows(cx: &mut TestAppContext) {
        let (covers, cx) = covers(cx);
        frame(&covers, 0..CAPACITY, cx);
        frame(&covers, 0..1, cx);

        assert_eq!(held(&covers, cx), CAPACITY);
        assert!(holds(&covers, CAPACITY - 1, cx));
    }
}
