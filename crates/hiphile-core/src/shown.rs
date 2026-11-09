use std::sync::Arc;

use executor::{Executor, Task};
use fuzzy::{Index, Speller};
use session::Session;

use crate::Shelf;

/// Items and the order they show in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown<T> {
    items: Arc<[T]>,
    visible: Arc<[usize]>,
}

impl<T> Shown<T> {
    pub(crate) fn new(items: Arc<[T]>, visible: Arc<[usize]>) -> Self {
        Self { items, visible }
    }

    pub fn items(&self) -> &Arc<[T]> {
        &self.items
    }

    /// Positions in [`Shown::items`] of those that match the query, in the order they show.
    pub fn visible(&self) -> &Arc<[usize]> {
        &self.visible
    }

    /// The item at `position` among those that show.
    pub fn get(&self, position: usize) -> Option<&T> {
        self.items.get(*self.visible.get(position)?)
    }

    /// Where the first item that `is` holds for shows.
    pub fn position(&self, mut is: impl FnMut(&T) -> bool) -> Option<usize> {
        self.visible
            .iter()
            .position(|ix| self.items.get(*ix).is_some_and(&mut is))
    }
}

impl<T: Clone> Shown<T> {
    /// What plays from the item at `position`: those that show, in their order, and where that
    /// item is among them.
    pub fn queue_from(&self, position: usize) -> Option<(Arc<[T]>, usize)> {
        if position >= self.visible.len() {
            return None;
        }
        let is_every_item_in_order = self.visible.len() == self.items.len()
            && self.visible.iter().enumerate().all(|(at, ix)| at == *ix);
        let queue = if is_every_item_in_order {
            self.items.clone()
        } else {
            self.visible
                .iter()
                .map(|ix| self.items[*ix].clone())
                .collect()
        };
        Some((queue, position))
    }
}

/// Whether `items` were read but do not show yet, waiting for their index and their order.
pub(crate) fn is_waiting<T>(items: &Shelf<T>, shown: Option<&Shown<T>>) -> bool {
    match items {
        Shelf::Loaded(items) => shown.is_none_or(|shown| !Arc::ptr_eq(shown.items(), items)),
        Shelf::Loading | Shelf::Failed(_) => false,
    }
}

/// Takes in a reading of a list's items. The same items keep their `Arc`, and with it their index;
/// a failure leaves the items on show there, saying why in `failure`. Whether other items came,
/// which need an index.
pub(crate) fn take_reading<T: PartialEq>(
    items: &mut Shelf<T>,
    failure: &mut Option<String>,
    result: Result<Vec<T>, String>,
) -> bool {
    match result {
        Ok(read) if items.is_loaded() && *items.items() == *read => false,
        Ok(read) => {
            *items = Shelf::Loaded(read.into());
            true
        }
        Err(error) if items.is_loaded() => {
            *failure = Some(error);
            false
        }
        Err(error) => {
            *items = Shelf::Failed(error);
            false
        }
    }
}

/// A list's state whose items are searched and ordered off the host's thread.
pub(crate) trait Arranged: Send + 'static {
    type Item: Send + Sync + 'static;
    /// What orders the items besides their index, as it was when the arrangement started.
    type Order: Send + 'static;

    fn arranging(&mut self) -> &mut Arranging<Self::Item>;
    fn shown(&mut self) -> &mut Option<Shown<Self::Item>>;
    fn order(&self) -> Self::Order;
    fn index(items: &[Self::Item], speller: Option<&dyn Speller>) -> Index;
    fn arrange(items: &[Self::Item], index: &Index, order: Self::Order) -> Vec<usize>;
}

/// How far a list's index and order have got.
pub(crate) struct Arranging<T> {
    /// The items the last index was built for, with it.
    indexed: Option<(Arc<[T]>, Arc<Index>)>,
    /// Count the indexes and the arrangements, so that one replaced while it was finishing on
    /// another thread cannot replace its successor's result.
    index_id: u64,
    arrange_id: u64,
    _index: Task<()>,
    _arrange: Task<()>,
}

impl<T> Default for Arranging<T> {
    fn default() -> Self {
        Self {
            indexed: None,
            index_id: 0,
            arrange_id: 0,
            _index: Task::ready(()),
            _arrange: Task::ready(()),
        }
    }
}

/// Gets back into a list's state under its lock, from work the list started, unless the list is
/// gone.
type Reentry<S> = Arc<dyn Fn(Box<dyn FnOnce(&mut S) + Send>) + Send + Sync>;

/// Indexes and orders a list's items off the host's thread, and shows them.
pub(crate) struct Arranger<S> {
    executor: Executor,
    session: Session,
    reentry: Reentry<S>,
}

impl<S> Clone for Arranger<S> {
    fn clone(&self) -> Self {
        Self {
            executor: self.executor.clone(),
            session: self.session.clone(),
            reentry: self.reentry.clone(),
        }
    }
}

impl<S: Arranged> Arranger<S> {
    pub(crate) fn new(
        executor: Executor,
        session: Session,
        reentry: impl Fn(Box<dyn FnOnce(&mut S) + Send>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            executor,
            session,
            reentry: Arc::new(reentry),
        }
    }

    /// Indexes `items`, then shows them in order; the items on show stay until then. The index
    /// is built at once as far as the items read without the dictionaries, then with them when
    /// they are there: reading with them waits for any other list's reading, which takes a while
    /// on the first start.
    pub(crate) fn index(&self, state: &mut S, items: Arc<[S::Item]>) {
        let arranging = state.arranging();
        arranging.index_id += 1;
        let index_id = arranging.index_id;
        let book = self.session.reading_book();
        let this = self.clone();
        arranging._index = self.executor.spawn(async move {
            let quick = this
                .executor
                .spawn_blocking({
                    let items = items.clone();
                    move || S::index(&items, None)
                })
                .await;
            this.take_index(index_id, items.clone(), quick);
            let Some(book) = book else {
                return;
            };
            let read = this
                .executor
                .spawn_blocking({
                    let items = items.clone();
                    move || book.read(|speller| S::index(&items, Some(speller)))
                })
                .await;
            this.take_index(index_id, items, read);
        });
    }

    fn take_index(&self, index_id: u64, items: Arc<[S::Item]>, index: Index) {
        let this = self.clone();
        (self.reentry)(Box::new(move |state| {
            if state.arranging().index_id == index_id {
                state.arranging().indexed = Some((items, Arc::new(index)));
                this.arrange(state);
            }
        }));
    }

    /// Orders the items indexed last as the state says, and shows them. Before any index, does
    /// nothing: the index orders them once it comes.
    pub(crate) fn arrange(&self, state: &mut S) {
        let order = state.order();
        let arranging = state.arranging();
        arranging.arrange_id += 1;
        let Some((items, index)) = arranging.indexed.clone() else {
            return;
        };
        let arrange_id = arranging.arrange_id;
        let this = self.clone();
        arranging._arrange = self.executor.spawn(async move {
            let visible = this
                .executor
                .spawn_blocking({
                    let items = items.clone();
                    move || S::arrange(&items, &index, order)
                })
                .await;
            (this.reentry)(Box::new(move |state| {
                if state.arranging().arrange_id == arrange_id {
                    show(state.shown(), items, visible);
                }
            }));
        });
    }
}

/// The same order of the same items keeps its `Arc`, so that a host sees nothing new.
fn show<T>(shown: &mut Option<Shown<T>>, items: Arc<[T]>, visible: Vec<usize>) {
    let is_same = shown
        .as_ref()
        .is_some_and(|old| Arc::ptr_eq(&old.items, &items) && *old.visible == *visible);
    if !is_same {
        *shown = Some(Shown::new(items, visible.into()));
    }
}

#[cfg(test)]
mod tests;
