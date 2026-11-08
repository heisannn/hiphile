use super::*;

#[test]
fn a_value_from_the_file_stays_an_option() {
    let label = |number: f64| format!("{number:.2}");
    assert_eq!(numbers(&[1.3, 1.4], 1.33, label).len(), 3);
    assert_eq!(numbers(&[1.3, 1.4], 1.4, label).len(), 2);
}
