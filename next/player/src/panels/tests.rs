use super::*;

#[test]
fn the_queue_keeps_its_width_where_there_is_room() {
    assert_eq!(queue_width_in(QUEUE_WIDTH, 1000.0), QUEUE_WIDTH);
    assert_eq!(queue_width_in(QUEUE_WIDTH, 290.0), 290.0);
    assert_eq!(queue_width_in(QUEUE_WIDTH, 100.0), QUEUE_MIN_WIDTH);
}

#[test]
fn without_room_for_its_least_the_queue_neither_shows_nor_opens() {
    let panels = Panels::new();
    panels.queue.set(true);
    assert!(panels.is_queue_shown());

    panels.set_queue_room(QUEUE_MIN_WIDTH - 1.0);
    assert!(!panels.queue_fits());
    assert!(!panels.is_queue_shown());
    panels.toggle_queue();
    assert!(panels.queue.get_untracked());

    panels.set_queue_room(QUEUE_MIN_WIDTH);
    assert!(panels.is_queue_shown());
    panels.toggle_queue();
    assert!(!panels.queue.get_untracked());
}

#[test]
fn the_queue_cannot_be_made_wider_than_its_room_or_its_most() {
    let panels = Panels::new();
    assert_eq!(panels.queue_most(), QUEUE_MAX_WIDTH);

    panels.set_queue_room(300.0);
    assert_eq!(panels.queue_most(), 300.0);
    panels.set_queue_width(QUEUE_MAX_WIDTH);
    assert_eq!(panels.queue_width(), 300.0);
}
