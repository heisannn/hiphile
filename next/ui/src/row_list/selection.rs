use std::borrow::Borrow;
use std::collections::HashSet;
use std::hash::Hash;

/// How a key moves through the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    By(isize),
    Pages(isize),
    First,
    Last,
}

impl Motion {
    /// The position to go to in `len` rows with `page` rows in view, from `current`. With nothing
    /// current, a move starts at the first row, or at the last for [`Motion::Last`].
    pub fn apply(self, current: Option<usize>, len: usize, page: usize) -> Option<usize> {
        let last = len.checked_sub(1)?;
        let Some(current) = current.filter(|current| *current <= last) else {
            return Some(if matches!(self, Self::Last) { last } else { 0 });
        };
        Some(match self {
            Self::By(delta) => current.saturating_add_signed(delta).min(last),
            Self::Pages(pages) => current
                .saturating_add_signed(pages.saturating_mul(page as isize))
                .min(last),
            Self::First => 0,
            Self::Last => last,
        })
    }
}

/// Rows in their order, by id.
pub(super) trait Order<K> {
    fn len(&self) -> usize;
    fn id_at(&self, position: usize) -> Option<K>;
    fn position_of(&self, id: &K) -> Option<usize>;
}

/// The rows selected, by id, so that it stays with them however they move. The cursor is the row
/// that keys move from, and the anchor where Shift selects from. What is selected stays so while a
/// filter hides it, so it is counted only among the rows on show.
#[derive(Clone, Debug)]
pub struct Selected<K> {
    ids: HashSet<K>,
    anchor: Option<K>,
    cursor: Option<K>,
}

impl<K> Default for Selected<K> {
    fn default() -> Self {
        Self {
            ids: HashSet::new(),
            anchor: None,
            cursor: None,
        }
    }
}

impl<K: Clone + Eq + Hash> Selected<K> {
    pub fn contains(&self, id: &K) -> bool {
        self.ids.contains(id)
    }

    pub fn cursor(&self) -> Option<&K> {
        self.cursor.as_ref()
    }

    /// How many of `shown`, the ids of the rows on show, are selected.
    pub fn count_in(&self, shown: impl IntoIterator<Item = impl Borrow<K>>) -> usize {
        shown
            .into_iter()
            .filter(|id| self.ids.contains(id.borrow()))
            .count()
    }

    /// Whether `id` alone is selected, and the keys and Shift go from it.
    pub(super) fn is_only(&self, id: &K) -> bool {
        self.ids.len() == 1
            && self.ids.contains(id)
            && self.anchor.as_ref() == Some(id)
            && self.cursor.as_ref() == Some(id)
    }

    pub(super) fn is_cleared(&self) -> bool {
        self.ids.is_empty() && self.anchor.is_none() && self.cursor.is_none()
    }

    #[cfg(test)]
    pub(super) fn ids(&self) -> impl Iterator<Item = &K> {
        self.ids.iter()
    }

    pub(super) fn select_only(&mut self, id: K) {
        self.ids.clear();
        self.ids.insert(id.clone());
        self.anchor = Some(id.clone());
        self.cursor = Some(id);
    }

    pub(super) fn toggle(&mut self, id: K) {
        if !self.ids.remove(&id) {
            self.ids.insert(id.clone());
        }
        self.anchor = Some(id.clone());
        self.cursor = Some(id);
    }

    /// Selects the rows from the anchor to `id` and nothing else. An anchor that is not shown
    /// gives way to the cursor, and that to `id`.
    pub(super) fn extend_to(&mut self, id: K, order: &impl Order<K>) {
        let Some(to) = order.position_of(&id) else {
            return;
        };
        let from = [&self.anchor, &self.cursor]
            .into_iter()
            .flatten()
            .find_map(|from| order.position_of(from))
            .unwrap_or(to);
        self.ids = (from.min(to)..=from.max(to))
            .filter_map(|position| order.id_at(position))
            .collect();
        if self.anchor.is_none() {
            self.anchor = Some(id.clone());
        }
        self.cursor = Some(id);
    }

    pub(super) fn select_all(&mut self, order: &impl Order<K>) {
        self.ids = (0..order.len())
            .filter_map(|position| order.id_at(position))
            .collect();
        if self.cursor.is_none() {
            self.cursor = order.id_at(0);
            self.anchor = self.cursor.clone();
        }
    }

    /// Moves the cursor from `current`, the position it is at, and selects it, or, with `extend`,
    /// what is from the anchor to it. Where the cursor went.
    pub(super) fn move_by(
        &mut self,
        motion: Motion,
        extend: bool,
        current: Option<usize>,
        order: &impl Order<K>,
        page: usize,
    ) -> Option<usize> {
        let to = motion.apply(current, order.len(), page)?;
        let id = order.id_at(to)?;
        if extend {
            self.extend_to(id, order);
        } else {
            self.select_only(id);
        }
        Some(to)
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}
