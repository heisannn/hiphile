//! What a list selects after it changes. The selection itself stays with each view.

/// The position that takes the place of the one taken out at `ix`, in a list now `len` long:
/// the same, or the last when it was the last. `None` once the list is empty.
pub fn after_removal(ix: usize, len: usize) -> Option<usize> {
    len.checked_sub(1).map(|last| ix.min(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_takes_the_place_of_one_removed() {
        assert_eq!(after_removal(1, 3), Some(1));
    }

    #[test]
    fn the_one_before_takes_the_place_of_the_last() {
        assert_eq!(after_removal(3, 3), Some(2));
    }

    #[test]
    fn nothing_takes_the_place_of_the_only_one() {
        assert_eq!(after_removal(0, 0), None);
    }
}
