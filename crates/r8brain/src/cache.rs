use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// Most recently used first. Past `limit` it drops the last item not in use,
/// and grows if all are.
pub(crate) struct Cache<T> {
    items: Mutex<Vec<Arc<T>>>,
    limit: AtomicUsize,
}

impl<T> Cache<T> {
    pub(crate) const fn new(limit: usize) -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            limit: AtomicUsize::new(limit),
        }
    }

    pub(crate) fn set_limit(&self, limit: usize) {
        self.limit.store(limit, Ordering::Relaxed);
        let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        drop_unused_past(&mut items, limit);
    }

    /// Other threads wait while this one builds, so that two never build the
    /// same item.
    pub(crate) fn get_or_build(
        &self,
        matches: impl Fn(&T) -> bool,
        build: impl FnOnce() -> T,
    ) -> Arc<T> {
        let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        let item = match items.iter().position(|item| matches(item)) {
            Some(index) => items.remove(index),
            None => {
                let room_for_one = self.limit.load(Ordering::Relaxed).saturating_sub(1);
                drop_unused_past(&mut items, room_for_one);
                Arc::new(build())
            }
        };
        items.insert(0, Arc::clone(&item));
        item
    }
}

/// Shares an item only while something holds it, so that what goes out of use is freed at once.
pub(crate) struct WhileInUse<T> {
    items: Mutex<Vec<Weak<T>>>,
}

impl<T> WhileInUse<T> {
    pub(crate) const fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
        }
    }

    /// Other threads wait while this one builds, so that two never build the
    /// same item.
    pub(crate) fn get_or_build(
        &self,
        matches: impl Fn(&T) -> bool,
        build: impl FnOnce() -> T,
    ) -> Arc<T> {
        let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        items.retain(|item| item.strong_count() > 0);
        if let Some(item) = items
            .iter()
            .filter_map(Weak::upgrade)
            .find(|item| matches(item))
        {
            return item;
        }
        let item = Arc::new(build());
        items.push(Arc::downgrade(&item));
        item
    }
}

fn drop_unused_past<T>(items: &mut Vec<Arc<T>>, limit: usize) {
    while items.len() > limit
        && let Some(unused) = items.iter().rposition(|item| Arc::strong_count(item) == 1)
    {
        items.remove(unused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_is_built_once_and_the_least_recent_unused_one_goes_first() {
        let cache = Cache::new(2);
        let get = |key: u32| cache.get_or_build(|item| *item == key, || key);

        let kept = get(1);
        get(2);
        assert!(Arc::ptr_eq(&kept, &get(1)));
        // 1 is in use and more recent, and 2 is neither
        get(3);
        assert_eq!(*cache.items.lock().unwrap(), [Arc::new(3), Arc::new(1)],);
    }

    #[test]
    fn a_lower_limit_drops_what_is_not_in_use() {
        let cache = Cache::new(4);
        let get = |key: u32| cache.get_or_build(|item| *item == key, || key);
        let kept = get(1);
        get(2);
        get(3);
        cache.set_limit(2);
        assert_eq!(*cache.items.lock().unwrap(), [Arc::new(3), kept]);
    }

    #[test]
    fn an_item_is_shared_while_in_use_and_freed_once_out_of_use() {
        let items = WhileInUse::new();
        let get = |key: u32| items.get_or_build(|item| *item == key, || key);
        let first = get(1);
        assert!(Arc::ptr_eq(&first, &get(1)));
        let gone = Arc::downgrade(&first);
        drop(first);
        assert!(gone.upgrade().is_none());
        assert_eq!(*get(1), 1);
    }

    #[test]
    fn items_in_use_stay_past_the_limit() {
        let cache = Cache::new(1);
        let get = |key: u32| cache.get_or_build(|item| *item == key, || key);
        let _first = get(1);
        let _second = get(2);
        assert_eq!(cache.items.lock().unwrap().len(), 2);
    }
}
