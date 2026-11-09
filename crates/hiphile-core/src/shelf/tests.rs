use super::*;

#[test]
fn a_failed_reload_keeps_what_is_on_show() {
    let mut shelf = Shelf::Loaded(Arc::from(["s1"]));
    shelf.begin();
    shelf.finish(Err("offline".into()));
    assert_eq!(shelf.items(), ["s1"]);
}

#[test]
fn a_failure_with_nothing_on_show_says_so() {
    let mut shelf = Shelf::<&str>::Failed("before".into());
    shelf.begin();
    assert_eq!(shelf, Shelf::Loading);
    shelf.finish(Err("offline".into()));
    assert_eq!(shelf, Shelf::Failed("offline".into()));
}
