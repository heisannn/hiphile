use super::*;

fn shown(items: &[&'static str], visible: &[usize]) -> Shown<&'static str> {
    Shown::new(items.into(), visible.into())
}

#[test]
fn positions_count_among_what_shows() {
    let shown = shown(&["a", "b", "c"], &[2, 0]);

    assert_eq!(shown.get(0), Some(&"c"));
    assert_eq!(shown.get(2), None);
    assert_eq!(shown.position(|item| *item == "a"), Some(1));
    assert_eq!(shown.position(|item| *item == "b"), None);
}

#[test]
fn the_queue_holds_what_shows_in_its_order() {
    let shown = shown(&["a", "b", "c"], &[2, 0]);

    let (queue, start) = shown.queue_from(1).unwrap();

    assert_eq!(*queue, ["c", "a"]);
    assert_eq!(start, 1);
    assert_eq!(shown.queue_from(2), None);
}

#[test]
fn every_item_in_its_own_order_queues_the_items_themselves() {
    let shown = shown(&["a", "b"], &[0, 1]);

    let (queue, _) = shown.queue_from(0).unwrap();

    assert!(Arc::ptr_eq(&queue, shown.items()));
}

#[test]
fn the_same_items_read_again_keep_their_arc() {
    let mut items = Shelf::Loaded(Arc::from(["a"]));
    let before = items.clone();
    let mut failure = None;

    assert!(!take_reading(&mut items, &mut failure, Ok(vec!["a"])));
    assert!(matches!((&items, &before), (Shelf::Loaded(a), Shelf::Loaded(b)) if Arc::ptr_eq(a, b)));
    assert!(take_reading(&mut items, &mut failure, Ok(vec!["b"])));
    assert_eq!(items.items(), ["b"]);
}

#[test]
fn a_failed_reading_keeps_the_items_on_show_and_says_why() {
    let mut items = Shelf::Loaded(Arc::from(["a"]));
    let mut failure = None;

    assert!(!take_reading(
        &mut items,
        &mut failure,
        Err("offline".into())
    ));
    assert_eq!(items.items(), ["a"]);
    assert_eq!(failure.as_deref(), Some("offline"));

    let mut never_shown = Shelf::<&str>::Loading;
    let mut failure = None;
    take_reading(&mut never_shown, &mut failure, Err("offline".into()));
    assert_eq!(never_shown, Shelf::Failed("offline".into()));
    assert_eq!(failure, None);
}

#[test]
fn items_read_wait_until_they_show() {
    let items: Arc<[&str]> = Arc::from(["a"]);
    let read = Shelf::Loaded(items.clone());

    assert!(is_waiting(&read, None));
    assert!(is_waiting(&read, Some(&shown(&["a"], &[0]))));
    assert!(!is_waiting(&read, Some(&Shown::new(items, [0].into()))));
    assert!(!is_waiting(&Shelf::<&str>::Loading, None));
}
