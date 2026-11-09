/// What a number that is typed may be, and how far a stepper moves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumberRange {
    min: f64,
    max: f64,
    step: f64,
    fraction: Option<usize>,
}

impl NumberRange {
    pub(crate) const fn new(min: f64, max: f64, step: f64, fraction: Option<usize>) -> Self {
        Self {
            min,
            max,
            step,
            fraction,
        }
    }

    pub fn min(self) -> f64 {
        self.min
    }

    pub fn max(self) -> f64 {
        self.max
    }

    pub fn step(self) -> f64 {
        self.step
    }

    /// How many digits may follow the point, with `None` for any.
    pub fn fraction(self) -> Option<usize> {
        self.fraction
    }

    /// `number` held to the range, or `None` for NaN.
    pub(crate) fn clamp(self, number: f64) -> Option<f64> {
        // Adding zero turns `-0` into `0`, which is how it should read later
        (!number.is_nan()).then(|| number.clamp(self.min, self.max) + 0.0)
    }

    /// The number `text` stands for, held to the range; `None` while it is not one yet, as for
    /// `-` on the way to a number.
    pub(crate) fn parse(self, text: &str) -> Option<f64> {
        self.clamp(text.parse().ok()?)
    }

    /// Whether `text` is a number that the range does not hold, as `1` on the way to `100` can be.
    pub(crate) fn is_beyond(self, text: &str) -> bool {
        text.parse::<f64>()
            .is_ok_and(|number| !(self.min..=self.max).contains(&number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RANGE: NumberRange = NumberRange::new(0., 10., 1., None);

    #[test]
    fn typed_text_is_held_to_the_range() {
        assert_eq!(RANGE.parse("0.5"), Some(0.5));
        assert_eq!(RANGE.parse("-1"), Some(0.));
        assert_eq!(RANGE.parse("99999999"), Some(10.));
        assert_eq!(RANGE.parse("inf"), Some(10.));
    }

    #[test]
    fn negative_zero_is_zero() {
        assert!(RANGE.parse("-0").unwrap().is_sign_positive());
    }

    #[test]
    fn text_on_its_way_to_a_number_is_none_yet() {
        for text in ["", "-", ".", "NaN"] {
            assert_eq!(RANGE.parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn only_numbers_can_be_beyond() {
        assert!(RANGE.is_beyond("11"));
        assert!(RANGE.is_beyond("-1"));
        assert!(!RANGE.is_beyond("10"));
        assert!(!RANGE.is_beyond("-"));
    }
}
