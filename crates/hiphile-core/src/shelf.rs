use std::sync::Arc;

/// Items that load, which stay on show while a reload is on its way or fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shelf<T> {
    Loading,
    Loaded(Arc<[T]>),
    /// They never showed, for this reason.
    Failed(String),
}

impl<T> Shelf<T> {
    pub fn items(&self) -> &[T] {
        match self {
            Self::Loaded(items) => items,
            Self::Loading | Self::Failed(_) => &[],
        }
    }

    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded(_))
    }

    /// Whether it loaded and holds nothing, which says nothing worth a place on the screen.
    pub(crate) fn has_loaded_nothing(&self) -> bool {
        matches!(self, Self::Loaded(items) if items.is_empty())
    }

    pub(crate) fn begin(&mut self) {
        if !self.is_loaded() {
            *self = Self::Loading;
        }
    }

    /// A failure leaves items on show in place.
    pub(crate) fn finish(&mut self, result: Result<Vec<T>, String>) {
        match result {
            Ok(items) => *self = Self::Loaded(items.into()),
            Err(_) if self.is_loaded() => {}
            Err(error) => *self = Self::Failed(error),
        }
    }
}

#[cfg(test)]
mod tests;
