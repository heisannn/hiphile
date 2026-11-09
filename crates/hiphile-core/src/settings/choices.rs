/// One of the values a setting can take, and how a UI calls it.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    value: String,
    label: String,
}

impl Choice {
    pub(crate) fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }

    /// What the setting is given back when this is chosen.
    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

pub(super) fn same_labels(values: impl Iterator<Item = String>) -> Vec<Choice> {
    values
        .map(|value| Choice::new(value.clone(), value))
        .collect()
}

pub(in crate::settings) fn number_value(number: f64) -> String {
    number.to_string()
}

/// The values, and `current` too if the file holds one that is not among them, so that the
/// choice stays visible.
pub(in crate::settings) fn numbers(
    values: &[f64],
    current: f64,
    label: impl Fn(f64) -> String,
) -> Vec<Choice> {
    let mut choices: Vec<Choice> = values
        .iter()
        .map(|number| Choice::new(number_value(*number), label(*number)))
        .collect();
    if !values.contains(&current) {
        choices.push(Choice::new(number_value(current), label(current)));
    }
    choices
}

#[cfg(test)]
mod tests;
